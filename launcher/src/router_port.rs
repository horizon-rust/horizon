//! A hosted server's port on the player's router, when they turn it on (`Settings::router_port`,
//! off by default).
//!
//! While a server the launcher shows runs, its TCP port is held open on the router with
//! `horizon_friends::portmap` (PCP, NAT-PMP, then UPnP IGD; the same code Play with Friends uses),
//! renewed when the lease is due, and removed when the server stops, the switch goes off or the
//! launcher closes. The launcher removes the mapping when it closes normally, a server that keeps
//! running as a background service included: [`begin_stop`] tells every job at once, without
//! waiting, and the process is kept alive while [`removals_pending`] says so, for at most
//! [`QUIT_WAIT`], long enough for a call already blocked on a router and then the removal. After a crash nothing removes it: the router's lease
//! ends it (an hour), except on a router that only gives permanent UPnP leases, where `portmap`
//! asks for none and the mapping stays until it is removed.
//!
//! [`Hold`] is the mapping and its rules with no thread and no router behind it (a [`Gateway`]
//! stands in for the router in tests), [`Job`] runs one on a thread, [`Jobs`] keeps one job per
//! server that should have one, and [`sync`], [`state`], [`begin_stop`] and [`removals_pending`] are
//! the launcher's one set of them.
use horizon_friends::portmap::{self, Targets};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

/// How long a failed opening or renewal waits before the router is asked again.
const RETRY: Duration = Duration::from_secs(60);
/// How often a job looks at its clocks.
const STEP: Duration = Duration::from_millis(200);
/// How often a job asks whether its server still runs.
const ALIVE_EVERY: Duration = Duration::from_secs(15);
/// How many of those answers in a row must say "no" before the job ends: one busy moment of the
/// server's control API must not take the port away.
const ALIVE_MISSES: u32 = 2;
/// The longest one router call takes in `portmap`: a UPnP request connects within 3 s and waits
/// 5 s for the answer (`portmap.rs`, `http`).
const ROUTER_CALL: Duration = Duration::from_secs(8);
/// How long closing the launcher keeps the process alive for the mappings to be removed: a renewal
/// that is already blocked on the router when the launcher closes, then the removal, each one
/// router call. A job looks at its stop request only between calls, so a shorter wait could let
/// the process end before the removal was even sent. The window's event loop keeps running while
/// it lasts (`router_panel::quit`).
pub const QUIT_WAIT: Duration = Duration::from_secs(ROUTER_CALL.as_secs() * 2 + 4);

/// Why the router did not open the port. The page turns [`Failure::NoPrivateAddress`] into its
/// own translated sentence; the other two carry text that is not the launcher's: `portmap`'s
/// diagnostic (each method's failure, in English by design, as in Play with Friends) and the
/// system's own error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// This computer has no private IPv4 address, so there is no router the launcher can ask.
    NoPrivateAddress,
    /// What `portmap` said.
    Router(String),
    /// The thread could not be started (the system's error).
    Thread(String),
}

/// What the hosting page says about the mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// The router is being asked.
    Asking,
    /// The port is open on the router: by which method, and as which address and port outside.
    /// The address is `None` when the router didn't say, or said one that isn't public (a
    /// carrier's NAT, a second router), which friends can't reach.
    Open {
        method: &'static str,
        wan: Option<IpAddr>,
        external_port: u16,
    },
    /// Opening or renewing failed; it is asked again after a minute.
    Failed(Failure),
}

/// What a router offers: the part of `portmap` this module uses, so that the rules around it can
/// be tested without one.
pub trait Gateway: Send + 'static {
    type Mapping: Send;
    /// Ask the router to open `port`.
    fn map(&mut self, port: u16) -> Result<Self::Mapping, Failure>;
    /// Whether the lease is past the point where it is renewed.
    fn due(&self, mapping: &Self::Mapping) -> bool;
    fn renew(&mut self, mapping: &mut Self::Mapping) -> Result<(), Failure>;
    /// Best effort.
    fn remove(&mut self, mapping: &Self::Mapping);
    /// How the mapping shows: method, WAN address as the router said it, external port.
    fn describe(&self, mapping: &Self::Mapping) -> (&'static str, IpAddr, u16);
}

/// The address the router is asked to forward to: this computer's, when it is a private IPv4
/// one (a shared or public address has no router of the player's in front of it).
fn private_client(lan: Option<IpAddr>) -> Result<Ipv4Addr, Failure> {
    match lan {
        Some(IpAddr::V4(v4)) if v4.is_private() => Ok(v4),
        _ => Err(Failure::NoPrivateAddress),
    }
}

/// The real router: `horizon_friends::portmap`, for this computer's LAN address.
pub struct Router;

impl Gateway for Router {
    type Mapping = portmap::Mapping;

    fn map(&mut self, port: u16) -> Result<portmap::Mapping, Failure> {
        let client = private_client(crate::servers::lan_address())?;
        let targets = Targets::discover(Duration::from_millis(1500));
        portmap::map(&targets, client, port, portmap::LEASE).map_err(Failure::Router)
    }
    fn due(&self, mapping: &portmap::Mapping) -> bool {
        mapping.due()
    }
    fn renew(&mut self, mapping: &mut portmap::Mapping) -> Result<(), Failure> {
        mapping.renew().map_err(|e| Failure::Router(e.to_string()))
    }
    fn remove(&mut self, mapping: &portmap::Mapping) {
        mapping.remove();
    }
    fn describe(&self, mapping: &portmap::Mapping) -> (&'static str, IpAddr, u16) {
        (
            mapping.method.name(),
            mapping.external.ip(),
            mapping.external.port(),
        )
    }
}

/// Whether friends on the internet can reach `ip`: not unspecified, loopback, link-local or
/// private (IPv4 private ranges; IPv6 unique-local fc00::/7), nor in 100.64.0.0/10
/// (carrier-grade NAT). An IPv4-mapped IPv6 address counts as its IPv4 address.
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_unspecified()
                || v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || (o[0] == 100 && (64..128).contains(&o[1])))
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            // An IPv4 address written as IPv6 (::ffff:a.b.c.d) is judged as the IPv4 address.
            Some(v4) => public(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                !(v6.is_unspecified()
                    || v6.is_loopback()
                    || first & 0xffc0 == 0xfe80
                    || first & 0xfe00 == 0xfc00)
            }
        },
    }
}

/// One server's mapping and the rules for it, without a thread.
pub struct Hold<G: Gateway> {
    gateway: G,
    port: u16,
    mapping: Option<G::Mapping>,
    state: State,
    /// Nothing is asked of the router before this, after a failed opening or renewal: a minute
    /// from when the failed call came back, not from when it began (a UPnP call can wait 5 s).
    retry_at: Option<Instant>,
    /// The time a failed call is over; the real clock but in tests.
    clock: Box<dyn Fn() -> Instant + Send>,
}

impl<G: Gateway> Hold<G> {
    pub fn new(gateway: G, port: u16) -> Self {
        Self::with_clock(gateway, port, Instant::now)
    }

    /// A hold that reads the time a failed call is over from `clock`.
    pub fn with_clock(gateway: G, port: u16, clock: impl Fn() -> Instant + Send + 'static) -> Self {
        Self {
            gateway,
            port,
            mapping: None,
            state: State::Asking,
            retry_at: None,
            clock: Box::new(clock),
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Ask the router to open the port (blocks for as long as the router takes).
    pub fn open(&mut self) {
        match self.gateway.map(self.port) {
            Ok(mapping) => {
                self.state = self.shown(&mapping);
                self.mapping = Some(mapping);
                self.retry_at = None;
            }
            Err(why) => {
                self.state = State::Failed(why);
                self.retry_at = Some((self.clock)() + RETRY);
            }
        }
    }

    fn shown(&self, mapping: &G::Mapping) -> State {
        let (method, wan, external_port) = self.gateway.describe(mapping);
        State::Open {
            method,
            wan: public(wan).then_some(wan),
            external_port,
        }
    }

    /// Ask again after a failed opening, renew the lease when it is due, each at most once a
    /// minute after a failure. Whether the state changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.retry_at.is_some_and(|at| now < at) {
            return false;
        }
        let before = self.state.clone();
        let Some(mapping) = self.mapping.as_mut() else {
            // Nothing was opened, or the opening failed: only a failure is asked again.
            if self.retry_at.is_some() {
                self.open();
            }
            return self.state != before;
        };
        if !self.gateway.due(mapping) {
            return false;
        }
        match self.gateway.renew(mapping) {
            Ok(()) => {
                self.retry_at = None;
                let mapping = self.mapping.as_ref().expect("held above");
                self.state = self.shown(mapping);
            }
            Err(why) => {
                self.retry_at = Some((self.clock)() + RETRY);
                self.state = State::Failed(why);
            }
        }
        self.state != before
    }

    /// Remove the mapping from the router, if there is one.
    pub fn close(&mut self) {
        if let Some(mapping) = self.mapping.take() {
            self.gateway.remove(&mapping);
        }
    }
}

struct Shared {
    stop: AtomicBool,
    /// Set when the thread is done, however it ended.
    finished: AtomicBool,
    state: Mutex<State>,
}

/// Held by the thread: dropping it (the end, or a panic) marks the job done and wakes the waiters.
struct Done(Arc<Shared>, mpsc::Sender<()>);

impl Drop for Done {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::SeqCst);
        let _ = self.1.send(());
    }
}

/// An earlier job for the same server, which a new one waits for before it asks the router: its
/// removal must not land after the new opening (UPnP may give the same external port again).
/// A new job waits for every such job, not only the latest: one stopped while it was itself
/// waiting ends at once, and an older one may still be removing.
pub struct Previous(Arc<Shared>);

/// A [`Hold`] on its own thread, from asking the router to removing the mapping.
pub struct Job {
    port: u16,
    shared: Arc<Shared>,
    done: mpsc::Receiver<()>,
}

impl Job {
    /// Open `port` and keep it open until [`Job::stop`], or until `alive` has said [`ALIVE_MISSES`]
    /// times in a row (asked every `alive_every`) that the server no longer runs; then remove it.
    /// `step` is how often the thread looks at its clocks. It waits for `before` to be done first.
    pub fn start<G: Gateway>(
        gateway: G,
        port: u16,
        alive: impl Fn() -> bool + Send + 'static,
        step: Duration,
        alive_every: Duration,
        before: Vec<Previous>,
    ) -> Self {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            state: Mutex::new(State::Asking),
        });
        let (done_tx, done) = mpsc::channel();
        let thread_shared = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("router-port".into())
            .spawn(move || {
                let _done = Done(thread_shared.clone(), done_tx);
                let stopped = || thread_shared.stop.load(Ordering::Relaxed);
                let publish = |state: &State| {
                    *thread_shared
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = state.clone();
                };
                for Previous(before) in &before {
                    while !before.finished.load(Ordering::SeqCst) && !stopped() {
                        std::thread::sleep(step);
                    }
                }
                let mut hold = Hold::new(gateway, port);
                if !stopped() {
                    hold.open();
                    publish(hold.state());
                }
                let mut asked = Instant::now();
                let mut misses = 0;
                while !stopped() {
                    std::thread::sleep(step);
                    if hold.tick(Instant::now()) {
                        publish(hold.state());
                    }
                    if asked.elapsed() >= alive_every {
                        asked = Instant::now();
                        misses = if alive() { 0 } else { misses + 1 };
                        if misses >= ALIVE_MISSES {
                            break;
                        }
                    }
                }
                hold.close();
            });
        if let Err(e) = spawned {
            shared.finished.store(true, Ordering::SeqCst);
            *shared.state.lock().unwrap_or_else(|e| e.into_inner()) =
                State::Failed(Failure::Thread(e.to_string()));
        }
        Self { port, shared, done }
    }

    /// The real router, with the real clocks.
    pub fn open_on_router(
        port: u16,
        alive: impl Fn() -> bool + Send + 'static,
        before: Vec<Previous>,
    ) -> Self {
        Self::start(Router, port, alive, STEP, ALIVE_EVERY, before)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn state(&self) -> State {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Whether the thread is done (the server stopped under it, or the mapping was removed).
    /// A job whose thread never started counts as done.
    pub fn finished(&self) -> bool {
        self.shared.finished.load(Ordering::SeqCst)
    }

    /// Ask the thread to remove the mapping and end, without waiting for the router.
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }

    /// Wait up to `wait` for the removal; whether it finished.
    pub fn wait(&self, wait: Duration) -> bool {
        matches!(
            self.done.recv_timeout(wait),
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected)
        ) || self.finished()
    }

    /// For a job that takes this one's place.
    pub fn previous(&self) -> Previous {
        Previous(self.shared.clone())
    }
}

/// The jobs, one per server, the jobs still removing a mapping, and the port each server listens
/// on.
#[derive(Default)]
pub struct Jobs {
    jobs: HashMap<PathBuf, Job>,
    /// Stopped jobs whose threads are still telling the router. Closing the launcher waits for
    /// them too, and a new job for the same server waits for its predecessor.
    stopping: Vec<Job>,
    /// The launcher is closing ([`Jobs::begin_stop`]): no new job is started.
    closing: bool,
    ports: HashMap<PathBuf, u16>,
    /// Servers that started again: the next port read for each is the one they started with.
    restarted: std::collections::HashSet<PathBuf>,
}

impl Jobs {
    /// The server started again (a new engine took `server.toml`, as `horizon-server restart` does
    /// with no stopped phase in between): the next port read for it is the one it now listens on,
    /// even though it is running. Until then the port it had is kept.
    pub fn server_restarted(&mut self, dir: &Path) {
        self.restarted.insert(dir.to_path_buf());
    }

    /// What a server's settings say its port is; `None` when they could not be read. A running
    /// server keeps the port it started with: one saved meanwhile is its next start's, and says
    /// nothing about the port the router needs. A stopped server's unreadable settings leave it
    /// with no known port, so nothing is guessed for it.
    pub fn note_port(&mut self, dir: &Path, port: Option<u16>, running: bool) {
        match port {
            Some(port) => {
                let restarted = self.restarted.remove(dir);
                if restarted || !running || !self.ports.contains_key(dir) {
                    self.ports.insert(dir.to_path_buf(), port);
                }
            }
            None if !running => {
                self.ports.remove(dir);
            }
            None => {}
        }
    }

    /// The servers that should have their port open: the running ones whose port is known, while
    /// the switch is on.
    pub fn wanted(&self, on: bool, running: &[PathBuf]) -> Vec<(PathBuf, u16)> {
        if !on {
            return Vec::new();
        }
        running
            .iter()
            .filter_map(|dir| Some((dir.clone(), *self.ports.get(dir)?)))
            .collect()
    }

    /// Start a job (with `start`, given the stopped jobs of that server still working) for
    /// each wanted server that has none, and stop the jobs of the servers no longer wanted, or
    /// whose port changed. A job that ended by itself is dropped, and started again if wanted.
    pub fn sync(
        &mut self,
        wanted: &[(PathBuf, u16)],
        start: impl Fn(&Path, u16, Vec<Previous>) -> Job,
    ) {
        // The page goes on polling while the launcher waits to close: a job started now would
        // have nobody to remove its mapping.
        if self.closing {
            return;
        }
        for (dir, job) in std::mem::take(&mut self.jobs) {
            let keep = !job.finished()
                && wanted
                    .iter()
                    .any(|(d, port)| *d == dir && *port == job.port());
            if keep {
                self.jobs.insert(dir, job);
            } else if !job.finished() {
                job.stop();
                self.stopping.push(job);
            }
        }
        self.stopping.retain(|job| !job.finished());
        for (dir, port) in wanted {
            if !self.jobs.contains_key(dir) {
                // Every stopped job still removing a mapping of this port, whichever server it
                // was for: two servers on one port share the router's mapping, and a removal
                // landing after the new opening would take it away.
                let before = self
                    .stopping
                    .iter()
                    .filter(|job| job.port() == *port)
                    .map(Job::previous)
                    .collect();
                self.jobs.insert(dir.clone(), start(dir, *port, before));
            }
        }
    }

    pub fn state(&self, dir: &Path) -> Option<State> {
        self.jobs.get(dir).map(Job::state)
    }

    /// Tell every job to remove its mapping and end, without waiting for any router: the jobs of
    /// servers still wanted and those stopped earlier alike. From now on no job is started.
    pub fn begin_stop(&mut self) {
        self.closing = true;
        let mut jobs: Vec<Job> = self.jobs.drain().map(|(_, job)| job).collect();
        self.stopping.append(&mut jobs);
        for job in &self.stopping {
            job.stop();
        }
    }

    /// Whether a router has still to be told something: some stopped job has not finished.
    pub fn removals_pending(&mut self) -> bool {
        self.stopping.retain(|job| !job.finished());
        !self.stopping.is_empty()
    }

    /// [`Jobs::begin_stop`], then wait up to `wait` in all for the routers to be told.
    pub fn stop_all(&mut self, wait: Duration) {
        self.begin_stop();
        let end = Instant::now() + wait;
        for job in &self.stopping {
            job.wait(end.saturating_duration_since(Instant::now()));
        }
    }
}

static JOBS: Mutex<Option<Jobs>> = Mutex::new(None);

fn jobs() -> std::sync::MutexGuard<'static, Option<Jobs>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The server started again (see [`Jobs::server_restarted`]).
pub fn server_restarted(dir: &Path) {
    jobs()
        .get_or_insert_with(Jobs::default)
        .server_restarted(dir);
}

/// What a server's settings say its port is (see [`Jobs::note_port`]).
pub fn note_port(dir: &Path, port: Option<u16>, running: bool) {
    jobs()
        .get_or_insert_with(Jobs::default)
        .note_port(dir, port, running);
}

/// Keep the real router's mappings as the switch (`on`) and the servers that run say: one for
/// each, removed for the rest. `alive(dir)` asks whether that server still runs.
pub fn sync(on: bool, running: &[PathBuf], alive: impl Fn(&Path) -> bool + Clone + Send + 'static) {
    let mut guard = jobs();
    let jobs = guard.get_or_insert_with(Jobs::default);
    let wanted = jobs.wanted(on, running);
    jobs.sync(&wanted, |dir, port, before| {
        let dir = dir.to_path_buf();
        let alive = alive.clone();
        Job::open_on_router(port, move || alive(&dir), before)
    });
}

/// What the hosting page says for `dir`: none when no mapping is held for it.
pub fn state(dir: &Path) -> Option<State> {
    jobs().as_ref().and_then(|j| j.state(dir))
}

/// The launcher is closing: every job is told to remove its mapping, and no job is started from
/// now on. Returns at once; the removals go on in the jobs' threads, and [`removals_pending`] says
/// when they are over.
pub fn begin_stop() {
    jobs().get_or_insert_with(Jobs::default).begin_stop();
}

/// Whether a router has still to be told something since [`begin_stop`].
pub fn removals_pending() -> bool {
    jobs().as_mut().is_some_and(Jobs::removals_pending)
}

/// Whether closing still waits: removals are pending, and `started` (when closing began) is less
/// than [`QUIT_WAIT`] before `now`.
pub fn still_waiting(pending: bool, started: Instant, now: Instant) -> bool {
    pending && now.saturating_duration_since(started) < QUIT_WAIT
}

#[cfg(test)]
#[path = "router_port_tests.rs"]
mod tests;
