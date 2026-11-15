use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Map(u16),
    Renew(u16),
    Remove(u16),
}

struct Granted {
    internal: u16,
}

/// A router that records what it is asked and answers as the test says.
#[derive(Clone)]
struct Fake {
    calls: Arc<Mutex<Vec<Call>>>,
    /// What `map` answers when it works: the method, the WAN address, the external port.
    granted: (&'static str, IpAddr, u16),
    map_fails: Arc<AtomicBool>,
    due: Arc<AtomicBool>,
    renew_fails: Arc<AtomicBool>,
    /// Removing a mapping waits until this is set (it starts set): a test holds a removal under
    /// way as long as it needs to, with no sleep to race.
    remove_gate: Arc<AtomicBool>,
    /// `map` panics: the thread of the job dies.
    map_panics: Arc<AtomicBool>,
    /// Time that passes (on `clock`) while `map` and `renew` wait for the router.
    takes_millis: Arc<std::sync::atomic::AtomicU64>,
    clock: Option<FakeClock>,
    /// `renew` waits until this is set (it starts set), like `remove_gate`.
    renew_gate: Arc<AtomicBool>,
}

/// A clock the tests move by hand, so that a call that "takes five seconds" does not sleep.
#[derive(Clone)]
struct FakeClock {
    base: Instant,
    millis: Arc<std::sync::atomic::AtomicU64>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            base: Instant::now(),
            millis: Arc::default(),
        }
    }
    fn now(&self) -> Instant {
        self.base + Duration::from_millis(self.millis.load(Ordering::SeqCst))
    }
    fn advance(&self, by: Duration) {
        self.millis
            .fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }
    /// `secs` after the start.
    fn at(&self, secs: u64) -> Instant {
        self.base + Duration::from_secs(secs)
    }
    /// It is `secs` after the start.
    fn set(&self, secs: u64) {
        self.millis.store(secs * 1000, Ordering::SeqCst);
    }
}

/// The hold's thread looks at the time `secs` after the start.
fn tick_at<G: Gateway>(hold: &mut Hold<G>, clock: &FakeClock, secs: u64) -> bool {
    clock.set(secs);
    hold.tick(clock.at(secs))
}

impl Fake {
    fn new() -> Self {
        Self {
            calls: Arc::default(),
            granted: ("UPnP", ip(203, 0, 113, 5), 40123),
            map_fails: Arc::default(),
            due: Arc::default(),
            renew_fails: Arc::default(),
            remove_gate: Arc::new(AtomicBool::new(true)),
            map_panics: Arc::default(),
            takes_millis: Arc::default(),
            clock: None,
            renew_gate: Arc::new(AtomicBool::new(true)),
        }
    }
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
    fn log(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }
    /// From now on a call takes five seconds.
    fn calls_take_five_seconds_from_now_on(&self) {
        self.takes_millis.store(5000, Ordering::SeqCst);
    }
    /// A call that takes `takes` of the fake clock's time.
    fn wait_for_the_router(&self) {
        if let Some(clock) = &self.clock {
            clock.advance(Duration::from_millis(
                self.takes_millis.load(Ordering::SeqCst),
            ));
        }
    }
    /// Hold every removal until [`Fake::release`].
    fn hold_removals(&self) {
        self.remove_gate.store(false, Ordering::SeqCst);
    }
    fn release(&self) {
        self.remove_gate.store(true, Ordering::SeqCst);
    }
    /// Let the held removals go after `millis`, from another thread.
    fn release_after(&self, millis: u64) {
        let gate = self.remove_gate.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(millis));
            gate.store(true, Ordering::SeqCst);
        });
    }
}

fn refusal() -> Failure {
    Failure::Router("PCP: no answer; UPnP: no router answered".into())
}

impl Gateway for Fake {
    type Mapping = Granted;
    fn map(&mut self, port: u16) -> Result<Granted, Failure> {
        self.log(Call::Map(port));
        self.wait_for_the_router();
        assert!(
            !self.map_panics.load(Ordering::SeqCst),
            "the gateway fell over"
        );
        if self.map_fails.load(Ordering::SeqCst) {
            return Err(refusal());
        }
        Ok(Granted { internal: port })
    }
    fn due(&self, _: &Granted) -> bool {
        self.due.load(Ordering::SeqCst)
    }
    fn renew(&mut self, m: &mut Granted) -> Result<(), Failure> {
        self.log(Call::Renew(m.internal));
        self.wait_for_the_router();
        let end = Instant::now() + Duration::from_secs(10);
        while !self.renew_gate.load(Ordering::SeqCst) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(1));
        }
        if self.renew_fails.load(Ordering::SeqCst) {
            Err(Failure::Router("the router stopped answering".into()))
        } else {
            Ok(())
        }
    }
    fn remove(&mut self, m: &Granted) {
        let end = Instant::now() + Duration::from_secs(10);
        while !self.remove_gate.load(Ordering::SeqCst) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.log(Call::Remove(m.internal));
    }
    fn describe(&self, _: &Granted) -> (&'static str, IpAddr, u16) {
        self.granted
    }
}

fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn job(fake: &Fake, port: u16, before: Vec<Previous>) -> Job {
    Job::start(
        fake.clone(),
        port,
        || true,
        Duration::from_millis(5),
        Duration::from_secs(3600),
        before,
    )
}

fn open_as_granted() -> State {
    State::Open {
        method: "UPnP",
        wan: Some(ip(203, 0, 113, 5)),
        external_port: 40123,
    }
}

#[test]
fn the_server_port_is_what_the_router_is_asked_for() {
    let fake = Fake::new();
    let mut hold = Hold::new(fake.clone(), 25570);
    assert_eq!(*hold.state(), State::Asking);
    hold.open();
    assert_eq!(fake.calls(), [Call::Map(25570)]);
    // The router put it on another port outside: that is what friends need.
    assert_eq!(*hold.state(), open_as_granted());
}

/// What a WAN address is worth to friends on the internet, at the exact edges of each range.
#[test]
fn only_a_public_wan_address_is_shown_as_one_to_join() {
    let v6 = |s: &str| IpAddr::V6(s.parse().unwrap());
    let not_public = [
        ip(0, 0, 0, 0),
        ip(127, 0, 0, 1),
        // Link-local.
        ip(169, 254, 0, 1),
        // The private ranges, at their edges.
        ip(10, 0, 0, 0),
        ip(10, 255, 255, 255),
        ip(172, 16, 0, 0),
        ip(172, 31, 255, 255),
        ip(192, 168, 0, 0),
        ip(192, 168, 255, 255),
        // Carrier-grade NAT 100.64.0.0/10: first and last address.
        ip(100, 64, 0, 0),
        ip(100, 127, 255, 255),
        v6("::"),
        v6("::1"),
        // An IPv4 address written as IPv6 is judged as the IPv4 address.
        v6("::ffff:10.0.0.5"),
        v6("::ffff:100.64.0.1"),
        // IPv6 link-local fe80::/10 and unique-local fc00::/7, at their edges.
        v6("fe80::1"),
        v6("febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
        v6("fc00::1"),
        v6("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
    ];
    for wan in not_public {
        assert!(!public(wan), "{wan}");
        let mut fake = Fake::new();
        fake.granted = ("PCP", wan, 25565);
        let mut hold = Hold::new(fake, 25565);
        hold.open();
        assert_eq!(
            *hold.state(),
            State::Open {
                method: "PCP",
                wan: None,
                external_port: 25565
            },
            "{wan}"
        );
    }
    // The address just outside each range is public.
    for wan in [
        ip(100, 63, 255, 255),
        ip(100, 128, 0, 0),
        ip(9, 255, 255, 255),
        ip(11, 0, 0, 0),
        ip(172, 15, 255, 255),
        ip(172, 32, 0, 0),
        ip(192, 167, 255, 255),
        ip(192, 169, 0, 0),
        ip(169, 253, 255, 255),
        ip(169, 255, 0, 0),
        ip(203, 0, 113, 5),
        v6("2001:db8::1"),
        v6("::ffff:8.8.8.8"),
        v6("fe7f:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
        v6("fe00::1"),
        v6("fbff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
    ] {
        assert!(public(wan), "{wan}");
    }
}

#[test]
fn only_a_private_ipv4_address_has_a_router_to_ask() {
    let v6: IpAddr = "fd00::5".parse().unwrap();
    assert_eq!(
        private_client(Some(ip(192, 168, 1, 20))),
        Ok(Ipv4Addr::new(192, 168, 1, 20))
    );
    assert_eq!(
        private_client(Some(ip(10, 0, 0, 2))),
        Ok(Ipv4Addr::new(10, 0, 0, 2))
    );
    // No network, a public address, a carrier's, loopback, IPv6: no private router.
    for lan in [
        None,
        Some(ip(8, 8, 8, 8)),
        Some(ip(100, 64, 1, 1)),
        Some(ip(127, 0, 0, 1)),
        Some(v6),
    ] {
        assert_eq!(
            private_client(lan),
            Err(Failure::NoPrivateAddress),
            "{lan:?}"
        );
    }
}

#[test]
fn a_router_that_refuses_says_why_and_has_nothing_to_remove() {
    let fake = Fake::new();
    fake.map_fails.store(true, Ordering::SeqCst);
    let mut hold = Hold::new(fake.clone(), 25565);
    hold.open();
    assert_eq!(*hold.state(), State::Failed(refusal()));
    hold.close();
    assert_eq!(fake.calls(), [Call::Map(25565)]);
}

#[test]
fn a_failed_opening_is_shown_and_asked_again_after_a_minute() {
    let clock = FakeClock::new();
    let mut fake = Fake::new();
    fake.map_fails.store(true, Ordering::SeqCst);
    fake.clock = Some(clock.clone());
    let at = clock.clone();
    let mut hold = Hold::with_clock(fake.clone(), 25565, move || at.now());
    hold.open();
    assert_eq!(*hold.state(), State::Failed(refusal()));
    // Not again within the minute, however often the thread looks.
    for secs in [1, 20, 59] {
        assert!(!hold.tick(clock.at(secs)));
    }
    assert_eq!(fake.calls(), [Call::Map(25565)]);
    // After it, the router still says no: shown the same, and another minute is waited.
    assert!(!tick_at(&mut hold, &clock, 61));
    assert!(!tick_at(&mut hold, &clock, 100));
    assert_eq!(fake.calls(), [Call::Map(25565), Call::Map(25565)]);
    // Then it answers (the network came up): open, and kept.
    fake.map_fails.store(false, Ordering::SeqCst);
    assert!(tick_at(&mut hold, &clock, 122));
    assert_eq!(*hold.state(), open_as_granted());
    assert_eq!(
        fake.calls(),
        [Call::Map(25565), Call::Map(25565), Call::Map(25565)]
    );
    // An open mapping is not asked for again.
    assert!(!tick_at(&mut hold, &clock, 500));
    assert_eq!(fake.calls().len(), 3);
    hold.close();
    assert_eq!(fake.calls().last(), Some(&Call::Remove(25565)));
}

/// A router call that fails after waiting five seconds for an answer: the minute before the next
/// try runs from when the failure came back, so it is 65 seconds after the call began, not 60.
#[test]
fn the_minute_after_a_failure_starts_when_the_failed_call_is_over() {
    let clock = FakeClock::new();
    let mut fake = Fake::new();
    fake.map_fails.store(true, Ordering::SeqCst);
    fake.clock = Some(clock.clone());
    fake.takes_millis.store(5000, Ordering::SeqCst);
    let at = clock.clone();
    let mut hold = Hold::with_clock(fake.clone(), 25565, move || at.now());
    hold.open();
    assert_eq!(fake.calls(), [Call::Map(25565)]);
    // 64 s after the call began is 59 s after it failed: still inside the minute.
    assert!(!tick_at(&mut hold, &clock, 64));
    assert_eq!(fake.calls().len(), 1);
    // 65 s is the minute: asked again (and fails again, after another five seconds).
    assert!(!tick_at(&mut hold, &clock, 65));
    assert_eq!(fake.calls().len(), 2);
    // That failure came back at 70 s: the next try is at 130 s, not at 125 s.
    assert!(!tick_at(&mut hold, &clock, 129));
    assert_eq!(fake.calls().len(), 2);
    assert!(!tick_at(&mut hold, &clock, 130));
    assert_eq!(fake.calls().len(), 3);
}

/// The same for a renewal that waits five seconds and fails.
#[test]
fn the_minute_after_a_failed_renewal_starts_when_the_renewal_is_over() {
    let clock = FakeClock::new();
    let mut fake = Fake::new();
    fake.clock = Some(clock.clone());
    let at = clock.clone();
    let mut hold = Hold::with_clock(fake.clone(), 25565, move || at.now());
    hold.open();
    fake.due.store(true, Ordering::SeqCst);
    fake.renew_fails.store(true, Ordering::SeqCst);
    fake.calls_take_five_seconds_from_now_on();
    assert!(tick_at(&mut hold, &clock, 0));
    // The renewal began at 0 and came back at 5 s: the retry is at 65 s.
    assert!(!tick_at(&mut hold, &clock, 64));
    assert_eq!(fake.calls(), [Call::Map(25565), Call::Renew(25565)]);
    tick_at(&mut hold, &clock, 65);
    assert_eq!(
        fake.calls(),
        [Call::Map(25565), Call::Renew(25565), Call::Renew(25565)]
    );
}

#[test]
fn a_hold_never_opened_asks_nothing() {
    let fake = Fake::new();
    let mut hold = Hold::new(fake.clone(), 25565);
    assert!(!hold.tick(Instant::now() + Duration::from_secs(3600)));
    assert_eq!(fake.calls(), []);
}

#[test]
fn a_lease_is_renewed_only_when_due() {
    let fake = Fake::new();
    let mut hold = Hold::new(fake.clone(), 25565);
    hold.open();
    let now = Instant::now();
    assert!(!hold.tick(now));
    assert!(!hold.tick(now + Duration::from_secs(5)));
    assert_eq!(fake.calls(), [Call::Map(25565)]);
    fake.due.store(true, Ordering::SeqCst);
    // A renewal that works changes nothing the page shows.
    assert!(!hold.tick(now + Duration::from_secs(10)));
    assert_eq!(fake.calls(), [Call::Map(25565), Call::Renew(25565)]);
}

#[test]
fn a_failed_renewal_is_shown_and_tried_again_after_a_minute() {
    let fake = Fake::new();
    let mut hold = Hold::new(fake.clone(), 25565);
    hold.open();
    fake.due.store(true, Ordering::SeqCst);
    fake.renew_fails.store(true, Ordering::SeqCst);
    let now = Instant::now();
    assert!(hold.tick(now));
    assert_eq!(
        *hold.state(),
        State::Failed(Failure::Router("the router stopped answering".into()))
    );
    // Not again within the minute, however often the thread looks.
    for secs in [1, 20, 59] {
        assert!(!hold.tick(now + Duration::from_secs(secs)));
    }
    assert_eq!(fake.calls(), [Call::Map(25565), Call::Renew(25565)]);
    // After it, and the router is back: open again, as before.
    fake.renew_fails.store(false, Ordering::SeqCst);
    assert!(hold.tick(now + Duration::from_secs(61)));
    assert_eq!(*hold.state(), open_as_granted());
    assert_eq!(
        fake.calls(),
        [Call::Map(25565), Call::Renew(25565), Call::Renew(25565)]
    );
}

#[test]
fn closing_removes_the_mapping_once() {
    let fake = Fake::new();
    let mut hold = Hold::new(fake.clone(), 25577);
    hold.open();
    hold.close();
    hold.close();
    assert_eq!(fake.calls(), [Call::Map(25577), Call::Remove(25577)]);
}

#[test]
fn a_job_opens_the_port_and_removes_it_when_stopped() {
    let fake = Fake::new();
    let job = job(&fake, 25580, Vec::new());
    until("the mapping", || matches!(job.state(), State::Open { .. }));
    assert_eq!(fake.calls(), [Call::Map(25580)]);
    assert!(!job.finished());
    job.stop();
    assert!(job.wait(Duration::from_secs(10)));
    assert_eq!(fake.calls(), [Call::Map(25580), Call::Remove(25580)]);
    assert!(job.finished());
}

#[test]
fn a_job_renews_while_it_runs() {
    let fake = Fake::new();
    let job = job(&fake, 25581, Vec::new());
    until("the mapping", || matches!(job.state(), State::Open { .. }));
    fake.due.store(true, Ordering::SeqCst);
    until("a renewal", || fake.calls().contains(&Call::Renew(25581)));
    job.stop();
    assert!(job.wait(Duration::from_secs(10)));
    assert_eq!(fake.calls().last(), Some(&Call::Remove(25581)));
}

fn job_with_alive(fake: &Fake, port: u16, alive: impl Fn() -> bool + Send + 'static) -> Job {
    Job::start(
        fake.clone(),
        port,
        alive,
        Duration::from_millis(5),
        Duration::from_millis(10),
        Vec::new(),
    )
}

#[test]
fn a_job_ends_and_removes_the_port_when_its_server_stops_running() {
    let fake = Fake::new();
    let running = Arc::new(AtomicBool::new(true));
    let seen = running.clone();
    let job = job_with_alive(&fake, 25582, move || seen.load(Ordering::SeqCst));
    until("the mapping", || matches!(job.state(), State::Open { .. }));
    assert!(!job.finished());
    running.store(false, Ordering::SeqCst);
    until("the job to end", || job.finished());
    assert_eq!(fake.calls(), [Call::Map(25582), Call::Remove(25582)]);
}

#[test]
fn one_failed_check_of_the_server_does_not_end_the_job_but_two_in_a_row_do() {
    let fake = Fake::new();
    let asked = Arc::new(Mutex::new(Vec::<bool>::new()));
    // The control API answers "no" once, then "yes" for a long while, then "no" for good.
    let script = [false, true, true, true, true, false, false];
    let seen = asked.clone();
    let job = job_with_alive(&fake, 25584, move || {
        let mut asked = seen.lock().unwrap();
        let answer = script.get(asked.len()).copied().unwrap_or(false);
        asked.push(answer);
        answer
    });
    until("the job to end", || job.finished());
    // It lived through the single "no" at the start, and ended on the two at the end.
    assert_eq!(*asked.lock().unwrap(), script);
    assert_eq!(fake.calls(), [Call::Map(25584), Call::Remove(25584)]);
}

#[test]
fn a_job_stopped_before_the_router_answers_leaves_nothing_behind() {
    // `stop` before the thread's first look: nothing was opened, so nothing is removed.
    let fake = Fake::new();
    let job = job(&fake, 25583, Vec::new());
    job.stop();
    assert!(job.wait(Duration::from_secs(10)));
    let calls = fake.calls();
    assert!(
        calls.is_empty() || calls == [Call::Map(25583), Call::Remove(25583)],
        "{calls:?}"
    );
}

#[test]
fn a_job_shows_a_failed_opening_and_removes_nothing() {
    // The real minute is too long to wait for: the retry rule itself is tested on `Hold`, in
    // `a_failed_opening_is_shown_and_asked_again_after_a_minute`.
    let fake = Fake::new();
    fake.map_fails.store(true, Ordering::SeqCst);
    let job = job(&fake, 25585, Vec::new());
    until("the failure", || job.state() == State::Failed(refusal()));
    assert_eq!(fake.calls(), [Call::Map(25585)]);
    job.stop();
    assert!(job.wait(Duration::from_secs(10)));
    // Nothing was opened, so nothing is removed.
    assert_eq!(fake.calls(), [Call::Map(25585)]);
}

#[test]
fn only_running_servers_with_a_known_port_are_wanted_and_never_while_the_switch_is_off() {
    let (a, b, c) = (PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("c"));
    let mut jobs = Jobs::default();
    jobs.note_port(&a, Some(25565), false);
    jobs.note_port(&b, Some(25570), false);
    // c runs, but its settings were never read: no port to ask the router for.
    let running = [a.clone(), b.clone(), c];
    assert!(jobs.wanted(false, &running).is_empty());
    assert_eq!(
        jobs.wanted(true, &running),
        [(a.clone(), 25565), (b, 25570)]
    );
    assert_eq!(jobs.wanted(true, std::slice::from_ref(&a)), [(a, 25565)]);
}

#[test]
fn a_port_saved_while_the_server_runs_is_the_next_starts() {
    let dir = PathBuf::from("a");
    let mut jobs = Jobs::default();
    // Read before it runs (and while it is stopped): the latest wins.
    jobs.note_port(&dir, Some(25565), false);
    jobs.note_port(&dir, Some(25566), false);
    assert_eq!(
        jobs.wanted(true, std::slice::from_ref(&dir)),
        [(dir.clone(), 25566)]
    );
    // Running: a later read (the settings page saved another port) changes nothing yet.
    jobs.note_port(&dir, Some(25999), true);
    assert_eq!(
        jobs.wanted(true, std::slice::from_ref(&dir)),
        [(dir.clone(), 25566)]
    );
    // Stopped again, then read: the new port.
    jobs.note_port(&dir, Some(25999), false);
    assert_eq!(
        jobs.wanted(true, std::slice::from_ref(&dir)),
        [(dir, 25999)]
    );
}

#[test]
fn a_port_that_could_not_be_read_is_not_guessed() {
    let dir = PathBuf::from("a");
    let mut jobs = Jobs::default();
    // Never read: nothing to ask for.
    jobs.note_port(&dir, None, false);
    assert!(jobs.wanted(true, std::slice::from_ref(&dir)).is_empty());
    // Read, then the settings file could not be read while the server was stopped: forgotten.
    jobs.note_port(&dir, Some(25565), false);
    jobs.note_port(&dir, None, false);
    assert!(jobs.wanted(true, std::slice::from_ref(&dir)).is_empty());
    // A running server keeps the port it started with when a later read fails.
    jobs.note_port(&dir, Some(25570), false);
    jobs.note_port(&dir, None, true);
    assert_eq!(
        jobs.wanted(true, std::slice::from_ref(&dir)),
        [(dir, 25570)]
    );
}

#[test]
fn each_running_server_gets_its_own_port_and_loses_it_when_it_is_no_longer_wanted() {
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    let a = PathBuf::from("server-a");
    let b = PathBuf::from("server-b");

    jobs.sync(&[(a.clone(), 25565), (b.clone(), 25570)], start);
    until("both mappings", || {
        matches!(jobs.state(&a), Some(State::Open { .. }))
            && matches!(jobs.state(&b), Some(State::Open { .. }))
    });
    let mut calls = fake.calls();
    calls.sort_by_key(|c| format!("{c:?}"));
    assert_eq!(calls, [Call::Map(25565), Call::Map(25570)]);

    // The same list again changes nothing.
    jobs.sync(&[(a.clone(), 25565), (b.clone(), 25570)], start);
    assert_eq!(fake.calls().len(), 2);

    // b stops: only its port is removed.
    jobs.sync(&[(a.clone(), 25565)], start);
    until("b's removal", || {
        fake.calls().contains(&Call::Remove(25570))
    });
    assert!(!fake.calls().contains(&Call::Remove(25565)));
    assert_eq!(jobs.state(&b), None);

    // a's port was changed in its settings: the old one goes, the new one is asked for.
    jobs.sync(&[(a.clone(), 25999)], start);
    until("the new port", || fake.calls().contains(&Call::Map(25999)));
    until("the old port's removal", || {
        fake.calls().contains(&Call::Remove(25565))
    });

    // The switch goes off: the mapping is removed.
    jobs.sync(&[], start);
    assert_eq!(jobs.state(&a), None);
    until("the last removal", || {
        fake.calls().contains(&Call::Remove(25999))
    });
}

#[test]
fn closing_the_launcher_waits_for_every_removal() {
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    jobs.sync(
        &[(PathBuf::from("a"), 26001), (PathBuf::from("b"), 26002)],
        start,
    );
    until("both mappings", || {
        fake.calls().len() == 2
            && ["a", "b"]
                .iter()
                .all(|d| matches!(jobs.state(Path::new(d)), Some(State::Open { .. })))
    });
    // The routers are slow to answer: closing must wait, not return and let the process end.
    fake.hold_removals();
    fake.release_after(100);
    jobs.stop_all(Duration::from_secs(10));
    let calls = fake.calls();
    assert!(calls.contains(&Call::Remove(26001)), "{calls:?}");
    assert!(calls.contains(&Call::Remove(26002)), "{calls:?}");
}

#[test]
fn closing_the_launcher_also_waits_for_a_removal_that_was_already_under_way() {
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    let dir = PathBuf::from("a");
    jobs.sync(&[(dir.clone(), 26003)], start);
    until("the mapping", || {
        matches!(jobs.state(&dir), Some(State::Open { .. }))
    });
    // The switch goes off, and the launcher closes at once: the removal is held, so it is surely
    // still going.
    fake.hold_removals();
    jobs.sync(&[], start);
    assert!(!fake.calls().contains(&Call::Remove(26003)));
    fake.release_after(100);
    jobs.stop_all(Duration::from_secs(10));
    assert_eq!(fake.calls(), [Call::Map(26003), Call::Remove(26003)]);
}

#[test]
fn a_new_job_for_a_server_waits_for_the_removal_of_the_one_before_it() {
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    let dir = PathBuf::from("a");
    jobs.sync(&[(dir.clone(), 26004)], start);
    until("the mapping", || {
        matches!(jobs.state(&dir), Some(State::Open { .. }))
    });
    // Off and on again at once, with the removal held: the router must not be asked again while
    // it is going.
    fake.hold_removals();
    jobs.sync(&[], start);
    jobs.sync(&[(dir.clone(), 26004)], start);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(fake.calls(), [Call::Map(26004)]);
    fake.release();
    until("the second mapping", || {
        fake.calls()
            .iter()
            .filter(|c| **c == Call::Map(26004))
            .count()
            == 2
    });
    assert_eq!(
        fake.calls(),
        [Call::Map(26004), Call::Remove(26004), Call::Map(26004)]
    );
    jobs.stop_all(Duration::from_secs(10));
    assert_eq!(fake.calls().last(), Some(&Call::Remove(26004)));
}

/// A chain of off and on: the job started in the middle is stopped while it still waits, and ends
/// at once, so the last one must wait for the first one's removal too, not only for the middle one.
#[test]
fn a_chain_of_off_and_on_never_asks_the_router_while_an_older_removal_is_going() {
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    let dir = PathBuf::from("a");
    let on = [(dir.clone(), 26005)];
    jobs.sync(&on, start);
    until("the mapping", || {
        matches!(jobs.state(&dir), Some(State::Open { .. }))
    });
    fake.hold_removals();
    jobs.sync(&[], start);
    jobs.sync(&on, start);
    jobs.sync(&[], start);
    jobs.sync(&on, start);
    // The first removal is still held: nobody has asked the router again, however long we look.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(fake.calls(), [Call::Map(26005)]);
    fake.release();
    until("the last job's mapping", || {
        fake.calls()
            .iter()
            .filter(|c| **c == Call::Map(26005))
            .count()
            == 2
    });
    // The first mapping was removed, then the last job asked; the jobs in between never opened.
    assert_eq!(
        fake.calls(),
        [Call::Map(26005), Call::Remove(26005), Call::Map(26005)]
    );
    jobs.stop_all(Duration::from_secs(10));
    assert_eq!(
        fake.calls(),
        [
            Call::Map(26005),
            Call::Remove(26005),
            Call::Map(26005),
            Call::Remove(26005)
        ]
    );
}

#[test]
fn a_gateway_that_panics_ends_its_job_and_the_server_can_be_mapped_again() {
    let fake = Fake::new();
    fake.map_panics.store(true, Ordering::SeqCst);
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    let dir = PathBuf::from("a");
    let on = [(dir.clone(), 26006)];
    jobs.sync(&on, start);
    until("the job to end", || {
        jobs.jobs.get(&dir).is_some_and(Job::finished)
    });
    // Waiting for it does not hang.
    let waited = Instant::now();
    let dead = jobs.jobs.get(&dir).expect("kept until the next sync");
    assert!(dead.wait(Duration::from_secs(10)));
    assert!(waited.elapsed() < Duration::from_secs(5));
    // The next sync drops it, and the gateway is back: the port is opened.
    fake.map_panics.store(false, Ordering::SeqCst);
    jobs.sync(&on, start);
    until("the new mapping", || {
        matches!(jobs.state(&dir), Some(State::Open { .. }))
    });
    jobs.stop_all(Duration::from_secs(10));
    assert_eq!(
        fake.calls(),
        [Call::Map(26006), Call::Map(26006), Call::Remove(26006)]
    );
}

#[test]
fn a_job_that_ended_by_itself_is_started_again_while_its_server_is_wanted() {
    let fake = Fake::new();
    let dead = Arc::new(AtomicBool::new(false));
    let dir = PathBuf::from("server");
    let mut jobs = Jobs::default();
    let seen = dead.clone();
    jobs.sync(&[(dir.clone(), 26010)], |_, port, before| {
        let seen = seen.clone();
        Job::start(
            fake.clone(),
            port,
            move || !seen.load(Ordering::SeqCst),
            Duration::from_millis(5),
            Duration::from_millis(10),
            before,
        )
    });
    until("the mapping", || {
        matches!(jobs.state(&dir), Some(State::Open { .. }))
    });
    dead.store(true, Ordering::SeqCst);
    until("the removal", || {
        fake.calls().contains(&Call::Remove(26010))
    });
    // The server is back (a restart): the next sync opens the port again.
    dead.store(false, Ordering::SeqCst);
    until("the job to be done", || {
        jobs.jobs.get(&dir).is_some_and(Job::finished)
    });
    jobs.sync(&[(dir.clone(), 26010)], |_, port, before| {
        job(&fake, port, before)
    });
    until("the second mapping", || {
        fake.calls()
            .iter()
            .filter(|c| **c == Call::Map(26010))
            .count()
            == 2
    });
    jobs.stop_all(Duration::from_secs(10));
}

#[test]
fn a_restart_that_took_another_port_is_followed_but_a_port_saved_while_running_is_not() {
    let dir = PathBuf::from("a");
    let mut jobs = Jobs::default();
    let on = std::slice::from_ref(&dir);
    jobs.note_port(&dir, Some(25565), false);
    assert_eq!(jobs.wanted(true, on), [(dir.clone(), 25565)]);
    // Saved while it runs: its next start's, as before.
    jobs.note_port(&dir, Some(25570), true);
    assert_eq!(jobs.wanted(true, on), [(dir.clone(), 25565)]);
    // `horizon-server restart` took 25570 with no stopped phase: once the launcher says the server
    // started again, the next read of a running server's port is the one it listens on now...
    jobs.server_restarted(&dir);
    jobs.note_port(&dir, Some(25570), true);
    assert_eq!(jobs.wanted(true, on), [(dir.clone(), 25570)]);
    // ...and only that read: another one saved while it runs waits for the next start again.
    jobs.note_port(&dir, Some(25999), true);
    assert_eq!(jobs.wanted(true, on), [(dir.clone(), 25570)]);
    // A restart with the port unchanged changes nothing.
    jobs.server_restarted(&dir);
    jobs.note_port(&dir, Some(25570), true);
    assert_eq!(jobs.wanted(true, on), [(dir, 25570)]);
}

#[test]
fn a_server_taking_over_a_port_waits_for_the_removal_of_the_one_that_left_it() {
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    let (a, b) = (PathBuf::from("a"), PathBuf::from("b"));
    jobs.sync(&[(a.clone(), 26020)], start);
    until("the mapping", || {
        matches!(jobs.state(&a), Some(State::Open { .. }))
    });
    fake.hold_removals();
    jobs.sync(&[], start);
    // Another server directory, the same TCP port, while a's removal is still going: its
    // mapping is the same one on the router, so a late removal would take it away.
    jobs.sync(&[(b.clone(), 26020)], start);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(fake.calls(), [Call::Map(26020)]);
    fake.release();
    until("b's mapping", || {
        fake.calls()
            .iter()
            .filter(|c| **c == Call::Map(26020))
            .count()
            == 2
    });
    assert_eq!(
        fake.calls(),
        [Call::Map(26020), Call::Remove(26020), Call::Map(26020)]
    );
    jobs.stop_all(Duration::from_secs(10));
}

#[test]
fn a_server_on_another_port_does_not_wait_for_a_removal_it_does_not_share() {
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    let (a, b) = (PathBuf::from("a"), PathBuf::from("b"));
    jobs.sync(&[(a.clone(), 26021)], start);
    until("the mapping", || {
        matches!(jobs.state(&a), Some(State::Open { .. }))
    });
    fake.hold_removals();
    jobs.sync(&[], start);
    jobs.sync(&[(b.clone(), 26022)], start);
    until("b's mapping", || {
        matches!(jobs.state(&b), Some(State::Open { .. }))
    });
    assert_eq!(fake.calls(), [Call::Map(26021), Call::Map(26022)]);
    fake.release();
    jobs.stop_all(Duration::from_secs(10));
}

/// Closing while a job is blocked on a router call: the launcher waits for that call and then for
/// the removal, instead of letting the process end before the removal is sent.
#[test]
fn closing_the_launcher_waits_for_a_renewal_that_is_blocked_on_the_router_and_then_removes() {
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    let dir = PathBuf::from("a");
    jobs.sync(&[(dir.clone(), 26030)], start);
    until("the mapping", || {
        matches!(jobs.state(&dir), Some(State::Open { .. }))
    });
    // The lease is due and the router does not answer the renewal.
    fake.renew_gate.store(false, Ordering::SeqCst);
    fake.due.store(true, Ordering::SeqCst);
    until("the renewal to begin", || {
        fake.calls().contains(&Call::Renew(26030))
    });
    let gate = fake.renew_gate.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        gate.store(true, Ordering::SeqCst);
    });
    jobs.stop_all(Duration::from_secs(10));
    // Not returned early: the removal was sent after the blocked renewal came back.
    let calls = fake.calls();
    assert_eq!(calls.first(), Some(&Call::Map(26030)), "{calls:?}");
    assert_eq!(calls.last(), Some(&Call::Remove(26030)), "{calls:?}");
}

/// The wait at close covers one blocked router call and then the removal.
#[test]
fn the_wait_at_close_covers_a_blocked_call_and_a_removal() {
    assert!(QUIT_WAIT >= ROUTER_CALL * 2, "{QUIT_WAIT:?}");
}

/// What closing costs (`router_panel::stop_on_quit`, then `router_panel::quit`): nothing when no
/// mapping is held, and beginning never waits for a router.
#[test]
fn closing_with_nothing_mapped_returns_at_once_and_has_nothing_pending() {
    let start = Instant::now();
    let mut jobs = Jobs::default();
    jobs.begin_stop();
    assert!(!jobs.removals_pending());
    // The ones the launcher calls, on the process-wide set (which nothing else here uses).
    begin_stop();
    assert!(!removals_pending());
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
}

/// Beginning to close returns at once even while a router has not answered, leaves the removal
/// going, and starts nothing new.
#[test]
fn beginning_to_close_does_not_wait_for_a_router_and_starts_nothing_new() {
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    let dir = PathBuf::from("a");
    jobs.sync(&[(dir.clone(), 26050)], start);
    until("the mapping", || {
        matches!(jobs.state(&dir), Some(State::Open { .. }))
    });
    fake.hold_removals();
    let beginning = Instant::now();
    jobs.begin_stop();
    assert!(
        beginning.elapsed() < Duration::from_secs(1),
        "{:?}",
        beginning.elapsed()
    );
    // The router has not been told yet, and something is pending.
    assert!(!fake.calls().contains(&Call::Remove(26050)));
    assert!(jobs.removals_pending());
    // The page goes on polling while the launcher waits: nothing is started for any server.
    jobs.sync(&[(dir.clone(), 26050), (PathBuf::from("b"), 26051)], start);
    assert_eq!(jobs.state(&dir), None);
    assert_eq!(jobs.state(Path::new("b")), None);
    fake.release();
    until("the removal", || !jobs.removals_pending());
    assert_eq!(fake.calls(), [Call::Map(26050), Call::Remove(26050)]);
}

/// What `router_panel::quit` does after the event loop goes on: it asks `removals_pending` until
/// it says no, and every removal has been sent by then, for every server.
#[test]
fn waiting_after_beginning_to_close_still_sends_every_removal() {
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    jobs.sync(
        &[(PathBuf::from("a"), 26060), (PathBuf::from("b"), 26061)],
        start,
    );
    until("both mappings", || {
        fake.calls().len() == 2
            && ["a", "b"]
                .iter()
                .all(|d| matches!(jobs.state(Path::new(d)), Some(State::Open { .. })))
    });
    fake.hold_removals();
    fake.release_after(150);
    jobs.begin_stop();
    assert!(jobs.removals_pending());
    until("nothing pending", || !jobs.removals_pending());
    let calls = fake.calls();
    assert!(calls.contains(&Call::Remove(26060)), "{calls:?}");
    assert!(calls.contains(&Call::Remove(26061)), "{calls:?}");
}

/// A router call that does not come back keeps the removal pending, and closing stops waiting at
/// `QUIT_WAIT` all the same: `still_waiting` is what bounds `router_panel::quit`'s loop.
#[test]
fn closing_stops_waiting_at_the_quit_wait_whatever_a_router_does() {
    let started = Instant::now();
    let at = |after: Duration| started + after;
    // Nothing pending: no wait, at any moment.
    assert!(!still_waiting(false, started, at(Duration::ZERO)));
    // Pending: waits until the wait is over, and no longer.
    assert!(still_waiting(true, started, at(Duration::ZERO)));
    assert!(still_waiting(
        true,
        started,
        at(QUIT_WAIT - Duration::from_millis(1))
    ));
    assert!(!still_waiting(true, started, at(QUIT_WAIT)));
    assert!(!still_waiting(true, started, at(QUIT_WAIT * 2)));
    // And a removal that never comes back stays pending: it is the bound that ends the wait.
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    let dir = PathBuf::from("a");
    jobs.sync(&[(dir.clone(), 26070)], start);
    until("the mapping", || {
        matches!(jobs.state(&dir), Some(State::Open { .. }))
    });
    fake.hold_removals();
    jobs.begin_stop();
    std::thread::sleep(Duration::from_millis(300));
    assert!(jobs.removals_pending());
    // Let the held removal finish so its thread ends with the test.
    fake.release();
    until("the removal", || !jobs.removals_pending());
}

/// A router that answers the removal at once costs the time of that call, not the wait.
#[test]
fn closing_when_the_router_answers_at_once_does_not_use_up_the_wait() {
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    let dir = PathBuf::from("a");
    jobs.sync(&[(dir.clone(), 26040)], start);
    until("the mapping", || {
        matches!(jobs.state(&dir), Some(State::Open { .. }))
    });
    let closing = Instant::now();
    jobs.stop_all(QUIT_WAIT);
    assert!(
        closing.elapsed() < Duration::from_secs(5),
        "{:?}",
        closing.elapsed()
    );
    assert_eq!(fake.calls(), [Call::Map(26040), Call::Remove(26040)]);
}

/// A router call that does not come back holds closing for the wait and no longer: `stop_all`
/// returns when the wait is over, with the removal still not done.
#[test]
fn closing_stops_waiting_when_the_wait_is_over_though_a_router_call_has_not_come_back() {
    let fake = Fake::new();
    let start = |_: &Path, port: u16, before: Vec<Previous>| job(&fake, port, before);
    let mut jobs = Jobs::default();
    let dir = PathBuf::from("a");
    jobs.sync(&[(dir.clone(), 26041)], start);
    until("the mapping", || {
        matches!(jobs.state(&dir), Some(State::Open { .. }))
    });
    fake.hold_removals();
    let wait = Duration::from_millis(400);
    let closing = Instant::now();
    jobs.stop_all(wait);
    let took = closing.elapsed();
    assert!(took >= wait, "{took:?}");
    assert!(took < wait + Duration::from_secs(3), "{took:?}");
    assert!(!fake.calls().contains(&Call::Remove(26041)));
    // Let the held removal finish so its thread ends with the test.
    fake.release();
    until("the removal", || {
        fake.calls().contains(&Call::Remove(26041))
    });
}
