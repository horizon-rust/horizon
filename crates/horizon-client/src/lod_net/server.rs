//! The server side of `horizon:lod`: what each request gets
//! ([`Service::request`]), the background threads that sample the tiles
//! admitted, every player in turn (one, or more on a big machine:
//! [`auto_threads`]), and the replies they hand back to the game thread
//! ([`Service::deliver`]), which sends them with its own packets. A tile
//! sampled once answers every player who asks for it next (`cache`), and one
//! being sampled every player who asks for it meanwhile.
use super::{
    Bucket, IN_FLIGHT, MAX_DISTANCE, Offer, PER_SECOND, Request, SIDE,
    cache::{self, TileCache},
    encode_reply, refusal,
};
use crate::streaming::{LodSampler, LodSource};
use std::{
    collections::{HashMap, VecDeque},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread,
    time::{Duration, Instant},
};

/// Tiles the server lets a player ask for at once, past [`PER_SECOND`]: the
/// network bunches requests the client spaced out.
const SERVER_BURST: f64 = 4.0 * PER_SECOND as f64;
/// Requests a Horizon client never sends (unreadable, or past the offer's
/// rate) a player may make before they are disconnected.
pub const STRIKES: u32 = 64;
/// How long a tile may wait before the threads past the first sample too:
/// half the time a client's tiles in flight last ([`IN_FLIGHT`] waiting,
/// [`PER_SECOND`] asked for a second), the other half being its sampling, the
/// tick that sends it and the network. A player whose tiles wait longer gets
/// fewer than its client asks for. The same tiles cost each thread more when
/// several sample at once (1.4x a tile on 4 threads, measured), so the others
/// sleep while the first keeps up.
const BEHIND: Duration = Duration::from_millis(IN_FLIGHT as u64 * 1000 / PER_SECOND as u64 / 2);

/// One player's limits at the server.
struct Player {
    bucket: Bucket,
    /// Requests admitted whose reply the player's connection hasn't taken.
    in_flight: usize,
    strikes: u32,
}

/// What a request gets.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Sampled: [`Service::push`] it.
    Serve(Request),
    /// Answered at once with this refusal: out of reach, in a level with no
    /// far terrain, or every slot taken.
    Refuse(Vec<u8>),
    /// Not answered: unreadable, or past the rate. A strike.
    Ignore,
    /// Too many strikes: disconnect the player.
    Kick,
}

/// How [`Service::deliver`]'s connection took a reply.
pub enum Delivery {
    Sent,
    /// Its backlog is long: the reply waits for the next delivery.
    Busy,
    /// The player left.
    Gone,
}

/// The server's far terrain: what each request gets ([`Self::request`]), and
/// the thread that samples the tiles admitted.
pub struct Service {
    pub distance: u32,
    players: HashMap<u32, Player>,
    biome_names: Arc<Vec<String>>,
    /// Sampling threads, started with the first tile.
    threads: usize,
    worker: Option<Worker>,
    /// Replies the worker finished, waiting for their connection.
    ready: VecDeque<(u32, Vec<u8>)>,
}
/// A tile admitted, waiting for the worker.
pub struct Job {
    pub conn: u32,
    /// The level's id: its sampler is kept.
    pub level: String,
    pub source: Arc<dyn LodSource>,
    /// The level's save, whose changes drop the tiles made from it.
    pub save: Option<Arc<crate::save::SaveDir>>,
    pub request: Request,
}
impl Service {
    /// `threads` sample the tiles, 0 for [`auto_threads`] of this machine.
    pub fn new(distance: u32, threads: u32, biome_names: Vec<String>) -> Self {
        let cores = thread::available_parallelism().map_or(1, |n| n.get());
        Self {
            distance: distance.min(MAX_DISTANCE),
            players: HashMap::new(),
            biome_names: Arc::new(biome_names),
            threads: match threads {
                0 => auto_threads(cores),
                n => n as usize,
            },
            worker: None,
            ready: VecDeque::new(),
        }
    }
    /// The far terrain a server with `net` serves, None where it serves none.
    pub fn serving(net: &crate::lan::NetConfig, biome_names: &[String]) -> Option<Self> {
        (net.lod_distance > 0)
            .then(|| Self::new(net.lod_distance, net.lod_threads, biome_names.to_vec()))
    }
    /// The offer Horizon clients hear.
    pub fn offer(&self) -> Offer {
        Offer::new(self.distance)
    }
    /// What `conn`'s request `data` gets from a player at `position`, in a
    /// level with far terrain when `open`. Every
    /// request takes a token, refused or not; one with no token left, or
    /// unreadable, is a strike. A served one holds a slot until its reply
    /// leaves ([`Self::deliver`]).
    pub fn request(
        &mut self,
        conn: u32,
        data: &[u8],
        position: [f64; 3],
        open: bool,
        now: Instant,
    ) -> Verdict {
        let player = self.players.entry(conn).or_insert_with(|| Player {
            bucket: Bucket::new(f64::from(PER_SECOND), SERVER_BURST, now),
            in_flight: 0,
            strikes: 0,
        });
        // The token goes first, so an unreadable request is charged too.
        let token = player.bucket.take(now);
        let request = match Request::decode(data) {
            Ok(request) if token => request,
            _ => {
                player.strikes += 1;
                return if player.strikes >= STRIKES {
                    Verdict::Kick
                } else {
                    Verdict::Ignore
                };
            }
        };
        if !open
            || !request.allowed(self.distance, position)
            || player.in_flight >= IN_FLIGHT as usize
        {
            return Verdict::Refuse(refusal(request.id));
        }
        player.in_flight += 1;
        Verdict::Serve(request)
    }
    /// Forget the players for whom `keep` is false (they left).
    pub fn retain(&mut self, keep: impl Fn(u32) -> bool) {
        self.players.retain(|&conn, _| keep(conn));
    }
    pub fn players(&self) -> usize {
        self.players.len()
    }
    /// Queue a served tile: the worker starts with the first.
    pub fn push(&mut self, job: Job) {
        let (names, threads) = (self.biome_names.clone(), self.threads);
        let worker = self
            .worker
            .get_or_insert_with(|| Worker::start(names, threads));
        let mut queue = lock(&worker.shared.queue);
        let waiting = (Instant::now(), job);
        match queue
            .players
            .iter_mut()
            .find(|(conn, _)| *conn == waiting.1.conn)
        {
            Some((_, jobs)) => jobs.push_back(waiting),
            None => (queue.players).push_back((waiting.1.conn, VecDeque::from([waiting]))),
        }
        drop(queue);
        // Every thread looks: the first takes it, the others once it waited
        // `BEHIND`.
        worker.shared.wake.notify_all();
    }
    /// Hands the finished replies to `send`, on the game thread, so they
    /// keep their place among the game's packets. A reply its connection
    /// is too busy for waits; one taken frees its player's slot.
    pub fn deliver(&mut self, mut send: impl FnMut(u32, &mut Vec<u8>) -> Delivery) {
        if let Some(worker) = &self.worker {
            self.ready.extend(lock(&worker.shared.done).drain(..));
        }
        let players = &mut self.players;
        self.ready
            .retain_mut(|(conn, data)| match send(*conn, data) {
                Delivery::Sent => {
                    if let Some(p) = players.get_mut(conn) {
                        p.in_flight = p.in_flight.saturating_sub(1);
                    }
                    false
                }
                Delivery::Busy => true,
                Delivery::Gone => false,
            });
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Worker {
    shared: Arc<WorkerShared>,
    threads: Vec<thread::JoinHandle<()>>,
}
struct WorkerShared {
    queue: Mutex<Queue>,
    wake: Condvar,
    /// Replies sampled, for [`Service::deliver`].
    done: Mutex<Vec<(u32, Vec<u8>)>>,
    tiles: Mutex<TileCache>,
    /// The tiles being sampled, with the players who asked for one of them
    /// meanwhile (connection and request id): they get its reply too.
    sampling: Mutex<HashMap<Sampling, Vec<(u32, u32)>>>,
    biome_names: Arc<Vec<String>>,
}
/// A tile being sampled: its level, its source's address and the request's
/// shape.
type Sampling = (String, usize, i32, i32, i32, bool);
#[derive(Default)]
struct Queue {
    /// Each player's tiles, since when they wait, players in turn.
    players: VecDeque<(u32, VecDeque<(Instant, Job)>)>,
    stop: bool,
}
impl Worker {
    fn start(biome_names: Arc<Vec<String>>, threads: usize) -> Self {
        let shared = Arc::new(WorkerShared {
            queue: Mutex::default(),
            wake: Condvar::new(),
            done: Mutex::default(),
            tiles: Mutex::new(TileCache::new(cache::MAX_BYTES)),
            sampling: Mutex::default(),
            biome_names,
        });
        let threads = (0..threads.max(1))
            .map_while(|index| {
                let worker = shared.clone();
                // A thread the system refuses leaves the work to the others.
                thread::Builder::new()
                    .name("Far terrain server".into())
                    .spawn(move || worker.work(index > 0))
                    .ok()
            })
            .collect();
        Self { shared, threads }
    }
}
/// The thread count of [`THREADS_PROPERTY`](super::THREADS_PROPERTY)'s
/// auto (0) on a machine of `cores`: 1 up to 8 cores, then one for each 8,
/// at most 4.
pub fn auto_threads(cores: usize) -> usize {
    if cores <= 8 { 1 } else { (cores / 8).min(4) }
}
impl WorkerShared {
    /// A sampling thread: every player's tiles in turn (a `helper`, past the
    /// first thread, only once one waited [`BEHIND`]), until the service
    /// stops.
    fn work(&self, helper: bool) {
        // The ticks, and everything else the machine runs, come first.
        horizon_threads::idle_current();
        // A sampler per level, kept while its level's source is the same.
        let mut samplers: HashMap<String, (Arc<dyn LodSource>, Box<dyn LodSampler>)> =
            HashMap::new();
        while let Some(job) = self.next(helper) {
            let r = job.request;
            let kept = {
                let mut tiles = lock(&self.tiles);
                if let Some(save) = &job.save {
                    tiles.follow(&job.level, &job.source, save);
                }
                tiles.get(&job.level, &job.source, &r)
            };
            if let Some(reply) = kept {
                lock(&self.done).push((job.conn, reply));
                continue;
            }
            let key: Sampling = (
                job.level.clone(),
                Arc::as_ptr(&job.source) as *const () as usize,
                r.x,
                r.z,
                r.stride,
                r.approximate,
            );
            match lock(&self.sampling).entry(key.clone()) {
                // Another thread samples it: its reply answers this one too.
                std::collections::hash_map::Entry::Occupied(mut waiting) => {
                    waiting.get_mut().push((job.conn, r.id));
                    continue;
                }
                std::collections::hash_map::Entry::Vacant(free) => {
                    free.insert(Vec::new());
                }
            }
            // The save's changes from here: one over the tile while it is
            // sampled may have been read before it, and taken off the
            // kept tiles (`follow`) by another thread before this one keeps
            // it.
            let start = job.save.as_ref().map(|save| save.far_changes().end());
            // A sampler that panics is dropped, and the tile refused
            // (not kept): every job gets a reply, which frees its slot.
            let mut sampled = false;
            let reply = catch_unwind(AssertUnwindSafe(|| {
                let same = samplers
                    .get(&job.level)
                    .is_some_and(|(source, _)| same_source(source, &job.source));
                if !same {
                    let main = job.source.clone().sampler();
                    // A fork leaves out the client's chunk decoration threads.
                    let sampler = main.fork().unwrap_or(main);
                    samplers.insert(job.level.clone(), (job.source.clone(), sampler));
                }
                let (_, sampler) = samplers.get_mut(&job.level).expect("inserted above");
                sampler.set_approximate(r.approximate);
                let columns = sampler.grid(r.x, r.z, r.stride, SIDE);
                // Not kept when part of it is still on its way.
                sampled = !sampler.waited();
                encode_reply(r.id, Some(&columns), &self.biome_names)
            }))
            .unwrap_or_else(|_| {
                log::warn!("Far terrain: sampling {} failed; tile refused", job.level);
                samplers.remove(&job.level);
                refusal(r.id)
            });
            let changed = (job.save.as_ref().zip(start))
                .is_some_and(|(save, start)| cache::changed_under(save, start, &r));
            if sampled && !changed {
                lock(&self.tiles).put(&job.level, &job.source, &r, &reply);
            }
            let waiting = lock(&self.sampling).remove(&key).unwrap_or_default();
            let mut done = lock(&self.done);
            for (conn, id) in waiting {
                done.push((conn, cache::with_id(&reply, r.id, id)));
            }
            done.push((job.conn, reply));
        }
    }
    /// The next player's next tile (for a `helper`, once a tile waited
    /// [`BEHIND`]), None once the service stops.
    fn next(&self, helper: bool) -> Option<Job> {
        let mut queue = lock(&self.queue);
        loop {
            if queue.stop {
                return None;
            }
            let oldest = (queue.players.iter())
                .filter_map(|(_, jobs)| jobs.front().map(|(since, _)| *since))
                .min();
            let early = oldest
                .filter(|_| helper)
                .and_then(|since| BEHIND.checked_sub(since.elapsed()))
                .filter(|left| !left.is_zero());
            if let Some(left) = early {
                queue = (self.wake.wait_timeout(queue, left))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
                continue;
            }
            if let Some((conn, mut jobs)) = queue.players.pop_front() {
                let job = jobs.pop_front();
                if !jobs.is_empty() {
                    queue.players.push_back((conn, jobs));
                }
                if let Some((_, job)) = job {
                    return Some(job);
                }
                continue;
            }
            queue = self
                .wake
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        lock(&self.shared.queue).stop = true;
        self.shared.wake.notify_all();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
fn same_source(a: &Arc<dyn LodSource>, b: &Arc<dyn LodSource>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: u32, x: i32) -> Vec<u8> {
        Request {
            id,
            x,
            z: 0,
            stride: 1,
            approximate: false,
        }
        .encode()
    }

    #[test]
    fn players_are_held_to_their_rate_and_slots() {
        let now = Instant::now();
        let mut service = Service::new(64, 1, Vec::new());
        let at = [0.0; 3];
        let served = |v: Verdict| matches!(v, Verdict::Serve(_));
        for _ in 0..IN_FLIGHT {
            assert!(served(service.request(1, &request(0, 0), at, true, now)));
        }
        // Every slot taken: refused until a reply leaves.
        assert_eq!(
            service.request(1, &request(5, 0), at, true, now),
            Verdict::Refuse(refusal(5))
        );
        assert!(served(service.request(2, &request(0, 0), at, true, now)));
        // Out of reach: refused, and charged like any request.
        assert!(matches!(
            service.request(2, &request(0, 1 << 20), at, true, now),
            Verdict::Refuse(_)
        ));
        let mut spent = IN_FLIGHT as usize + 1;
        while matches!(
            service.request(1, &request(0, 1 << 20), at, true, now),
            Verdict::Refuse(_)
        ) {
            spent += 1;
        }
        // The burst spent, every request is a strike, up to a kick.
        assert_eq!(spent, SERVER_BURST as usize);
        for _ in 1..STRIKES - 1 {
            assert_eq!(
                service.request(1, &request(0, 0), at, true, now),
                Verdict::Ignore
            );
        }
        assert_eq!(service.request(1, b"junk", at, true, now), Verdict::Kick);
        // A second later there are tokens again; slots come back as replies leave.
        let later = now + Duration::from_secs(1);
        assert!(matches!(
            service.request(1, &request(0, 0), at, true, later),
            Verdict::Refuse(_)
        ));
        service.ready.push_back((1, Vec::new()));
        service.ready.push_back((2, Vec::new()));
        service.deliver(|conn, _| {
            if conn == 1 {
                Delivery::Sent
            } else {
                Delivery::Busy
            }
        });
        assert_eq!(service.ready.len(), 1);
        assert!(served(service.request(1, &request(0, 0), at, true, later)));
        // A level with no far terrain refuses.
        assert!(matches!(
            service.request(1, &request(9, 0), at, false, later),
            Verdict::Refuse(_)
        ));
        service.retain(|conn| conn == 2);
        assert_eq!(service.players(), 1);
    }

    /// Far terrain that fails west of x = 0.
    struct Brittle;
    impl LodSampler for Brittle {
        fn sample(&mut self, x: i32, _z: i32) -> Option<crate::lod::Surface> {
            assert!(x >= 0, "a broken chunk");
            None
        }
    }
    impl LodSource for Brittle {
        fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
            Box::new(Brittle)
        }
    }

    /// Far terrain that counts the columns it samples.
    struct Counted(Arc<std::sync::atomic::AtomicUsize>);
    impl LodSampler for Counted {
        fn sample(&mut self, x: i32, z: i32) -> Option<crate::lod::Surface> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(crate::lod::Surface {
                height: 60 + (x ^ z) % 9,
                state: horizon_world::registry::BlockStateId(1 + (x & 3) as u32),
                water: (z % 5 == 0).then_some((62, horizon_world::registry::BlockStateId(86))),
                biome: (x & 1) as u16,
            })
        }
    }
    impl LodSource for Counted {
        fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
            Box::new(Counted(self.0.clone()))
        }
    }

    /// Every reply in `service` for `count` requests.
    fn replies(service: &mut Service, count: usize) -> Vec<(u32, Vec<u8>)> {
        let mut replies = Vec::new();
        let start = Instant::now();
        while replies.len() < count && start.elapsed() < Duration::from_secs(10) {
            service.deliver(|conn, data| {
                replies.push((conn, std::mem::take(data)));
                Delivery::Sent
            });
            thread::sleep(Duration::from_millis(2));
        }
        replies
    }

    #[test]
    fn players_asking_for_the_same_tile_cost_one_sample_and_get_the_same_bytes() {
        let now = Instant::now();
        let names: Vec<String> = ["minecraft:plains", "minecraft:desert"]
            .map(String::from)
            .into();
        let mut service = Service::new(64, 1, names.clone());
        let columns = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let source: Arc<dyn LodSource> = Arc::new(Counted(columns.clone()));
        let at = [0.0; 3];
        // Four players, the same tile, each its own id; one asks again later.
        for (conn, id) in [(1, 5), (2, 300), (3, 7), (4, 1 << 20), (1, 6)] {
            let Verdict::Serve(request) = service.request(conn, &request(id, -16), at, true, now)
            else {
                panic!("served");
            };
            service.push(Job {
                conn,
                level: "minecraft:overworld".into(),
                source: source.clone(),
                save: None,
                request,
            });
        }
        let got = replies(&mut service, 5);
        assert_eq!(got.len(), 5);
        assert_eq!(
            columns.load(std::sync::atomic::Ordering::Relaxed),
            SIDE * SIDE
        );
        // Each the bytes a fresh sample makes for its id.
        let mut fresh = Counted(Arc::default());
        let fresh = fresh.grid(-16, 0, 1, SIDE);
        for (_, reply) in &got {
            let id = super::super::reply_id(reply).unwrap();
            assert_eq!(*reply, encode_reply(id, Some(&fresh), &names));
        }
        // Another level, or the same one under another source, is sampled anew.
        let reopened: Arc<dyn LodSource> = Arc::new(Counted(columns.clone()));
        for (level, source) in [
            ("minecraft:the_nether", &source),
            ("minecraft:overworld", &reopened),
        ] {
            let Verdict::Serve(request) = service.request(2, &request(8, -16), at, true, now)
            else {
                panic!("served");
            };
            service.push(Job {
                conn: 2,
                level: level.into(),
                source: source.clone(),
                save: None,
                request,
            });
        }
        assert_eq!(replies(&mut service, 2).len(), 2);
        assert_eq!(
            columns.load(std::sync::atomic::Ordering::Relaxed),
            3 * SIDE * SIDE
        );
    }

    /// `Counted`, slow enough that every thread is busy before it is done.
    struct Slow(Arc<std::sync::atomic::AtomicUsize>);
    impl LodSampler for Slow {
        fn sample(&mut self, x: i32, z: i32) -> Option<crate::lod::Surface> {
            if (x, z) == (0, 0) {
                thread::sleep(Duration::from_millis(200));
            }
            Counted(self.0.clone()).sample(x, z)
        }
    }
    impl LodSource for Slow {
        fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
            Box::new(Slow(self.0.clone()))
        }
    }

    #[test]
    fn threads_sampling_together_sample_a_tile_asked_for_at_once_once() {
        assert_eq!(
            [4, 8, 9, 16, 24, 64, 256].map(auto_threads),
            [1, 1, 1, 2, 3, 4, 4]
        );
        let now = Instant::now();
        let mut service = Service::new(64, 4, Vec::new());
        let columns = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let source: Arc<dyn LodSource> = Arc::new(Slow(columns.clone()));
        // Four players, the same tile, then two of them another one: the
        // tile being sampled answers the three who ask for it meanwhile.
        for (conn, id, x) in [
            (1, 1, 0),
            (2, 2, 0),
            (3, 3, 0),
            (4, 4, 0),
            (1, 5, 16),
            (2, 6, 32),
        ] {
            let Verdict::Serve(request) =
                service.request(conn, &request(id, x), [0.0; 3], true, now)
            else {
                panic!("served");
            };
            service.push(Job {
                conn,
                level: "minecraft:overworld".into(),
                source: source.clone(),
                save: None,
                request,
            });
        }
        let got = replies(&mut service, 6);
        assert_eq!(got.len(), 6);
        assert_eq!(
            columns.load(std::sync::atomic::Ordering::Relaxed),
            3 * SIDE * SIDE
        );
        let fresh = Slow(Arc::default()).grid(0, 0, 1, SIDE);
        for (conn, reply) in &got {
            let id = super::super::reply_id(reply).unwrap();
            if id <= 4 {
                assert_eq!(id, *conn);
                assert_eq!(*reply, encode_reply(id, Some(&fresh), &[]));
            }
        }
    }

    /// Samples nothing until `open`, counting the threads sampling.
    #[derive(Default)]
    struct Gate {
        state: Mutex<(bool, usize)>,
        changed: Condvar,
    }
    struct Gated(Arc<Gate>);
    impl LodSampler for Gated {
        fn sample(&mut self, _: i32, _: i32) -> Option<crate::lod::Surface> {
            None
        }
        fn grid(&mut self, _: i32, _: i32, _: i32, side: usize) -> Vec<crate::lod::Column> {
            let gate = &self.0;
            let mut state = lock(&gate.state);
            state.1 += 1;
            gate.changed.notify_all();
            while !state.0 {
                state = gate.changed.wait(state).unwrap();
            }
            state.1 -= 1;
            vec![crate::lod::Column::default(); side * side]
        }
    }
    impl LodSource for Gated {
        fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
            Box::new(Gated(self.0.clone()))
        }
    }

    #[test]
    fn a_second_thread_samples_only_once_a_tile_waited_too_long() {
        let now = Instant::now();
        let mut service = Service::new(64, 2, Vec::new());
        let gate = Arc::new(Gate::default());
        let source: Arc<dyn LodSource> = Arc::new(Gated(gate.clone()));
        let push = |service: &mut Service, id: u32| {
            let Verdict::Serve(request) =
                service.request(1, &request(id, id as i32 * 16), [0.0; 3], true, now)
            else {
                panic!("served");
            };
            service.push(Job {
                conn: 1,
                level: "minecraft:overworld".into(),
                source: source.clone(),
                save: None,
                request,
            });
        };
        let sampling = |at_least: usize| {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut state = lock(&gate.state);
            while state.1 < at_least && Instant::now() < deadline {
                state = gate
                    .changed
                    .wait_timeout(state, Duration::from_millis(5))
                    .unwrap()
                    .0;
            }
            state.1
        };
        // The first thread samples one tile; the next one waits for it.
        push(&mut service, 0);
        assert_eq!(sampling(1), 1);
        let pushed = Instant::now();
        push(&mut service, 1);
        // The second thread takes it, but only once it has waited too long.
        assert_eq!(sampling(2), 2);
        assert!(pushed.elapsed() >= BEHIND);
        lock(&gate.state).0 = true;
        gate.changed.notify_all();
        assert_eq!(replies(&mut service, 2).len(), 2);
    }

    #[test]
    fn a_sampler_that_panics_refuses_the_tile_and_frees_the_slot() {
        let now = Instant::now();
        let mut service = Service::new(64, 1, Vec::new());
        let source: Arc<dyn LodSource> = Arc::new(Brittle);
        let mut replies = Vec::new();
        for (id, x) in [(1, -16), (2, 0)] {
            let Verdict::Serve(request) = service.request(1, &request(id, x), [0.0; 3], true, now)
            else {
                panic!("served");
            };
            service.push(Job {
                conn: 1,
                level: "minecraft:overworld".into(),
                source: source.clone(),
                save: None,
                request,
            });
        }
        let start = Instant::now();
        while replies.len() < 2 && start.elapsed() < Duration::from_secs(10) {
            service.deliver(|_, data| {
                replies.push(std::mem::take(data));
                Delivery::Sent
            });
            thread::sleep(Duration::from_millis(5));
        }
        let refused = |data: &Vec<u8>| data[1] == 1;
        assert_eq!(
            replies.iter().map(refused).collect::<Vec<_>>(),
            [true, false]
        );
        assert_eq!(service.players[&1].in_flight, 0);
    }
}
