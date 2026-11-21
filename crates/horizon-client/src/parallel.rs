//! The helper threads of the server tick's parallel phases: one pool, made once, the first time a
//! phase shares its work, instead of threads started for every phase of every tick. A phase hands
//! it work that gives the same result in whatever order the threads take it, and each part's
//! results stay in order.
//!
//! Whether a tick's phases share their work is the Parallel Tick setting's
//! (`graphics::ParallelTickMode`: Off, Auto or On), decided between ticks for the next one
//! ([`after_tick`]) and never during one. `HORIZON_PARALLEL_TICK` wins over the setting, except
//! `auto`: it puts the setting itself in Auto and lets the machine decide, which is how a
//! dedicated server gets Auto (`server.toml`'s `parallel_tick = "auto"` or the variable).
use crate::graphics::ParallelTickMode;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

/// What `HORIZON_PARALLEL_TICK` asked for: the mode it forces, and whether it asks for the Auto
/// setting instead of forcing one.
///
/// `0` keeps every tick serial, `1` turns the server tick's parallel phases on, `verify` does too
/// and also checks the phases that can be checked against the serial code, and `auto` hands the
/// decision to the machine (the setting), as the window's settings screen would. Anything else,
/// and no variable at all, leaves the setting alone.
fn from_env(value: Option<&str>) -> (Option<u8>, bool) {
    match value {
        Some("0" | "off" | "false") => (Some(0), false),
        Some("1" | "on" | "true") => (Some(1), false),
        Some("verify") => (Some(2), false),
        Some("auto") => (None, true),
        _ => (None, false),
    }
}

/// What reading `HORIZON_PARALLEL_TICK` does: the mode it forces, and, for `auto`, the Auto
/// setting, which then decides between ticks itself.
fn init_forced(raw: Option<&str>) -> Option<u8> {
    let (mode, auto) = from_env(raw);
    if auto {
        apply_setting(ParallelTickMode::Auto);
    }
    mode
}

/// `HORIZON_PARALLEL_TICK`, whatever the setting says, except `auto`: that one applies the Auto
/// setting once and forces nothing, so the machine decides (a dedicated server's `"auto"`). It is
/// read before any setting is applied ([`set_setting`]), so on a client the window's own setting
/// (`apply_ui_settings`, a settings change or a pack reload) stands either way; a dedicated server
/// has no window, so its `auto` stays.
fn forced() -> Option<u8> {
    static FORCED: OnceLock<Option<u8>> = OnceLock::new();
    *FORCED.get_or_init(|| init_forced(std::env::var("HORIZON_PARALLEL_TICK").ok().as_deref()))
}

/// The Parallel Tick setting, as last applied: by the window's settings on a client, and on a
/// dedicated server by `HORIZON_PARALLEL_TICK=auto` or `server.toml`'s `parallel_tick = "auto"`,
/// which starts the game with that variable. Off until one of them does.
static SETTING: AtomicU8 = AtomicU8::new(0);
/// Whether the server's next tick shares its phases' work, by the setting ([`after_tick`]).
static SHARED: AtomicU8 = AtomicU8::new(0);
/// What Auto is doing, for the status ([`status`]).
static AUTO_STATE: AtomicU8 = AtomicU8::new(0);
static AUTO: Mutex<Auto> = Mutex::new(Auto::new());

/// Whether this tick's phases share their work: 0 serial, 1 shared, 2 shared and checked against
/// the serial code (`HORIZON_PARALLEL_TICK=verify`). Each phase gives the serial result either way.
pub fn parallel_tick() -> u8 {
    forced().unwrap_or_else(|| SHARED.load(Ordering::Relaxed))
}

/// The Parallel Tick setting, from the window's settings: it stands over the variable's `auto`.
/// The variable is read first, and its `auto` applies the Auto setting at that reading and never
/// after this call, so it cannot land on top of the setting.
pub fn set_setting(mode: ParallelTickMode) {
    let _ = forced();
    apply_setting(mode);
}

/// The setting applied: On and Off apply from the next tick, and Auto starts watching the ticks again.
fn apply_setting(mode: ParallelTickMode) {
    let mode = mode as u8;
    if SETTING.swap(mode, Ordering::Relaxed) != mode {
        *AUTO.lock().unwrap_or_else(PoisonError::into_inner) = Auto::new();
        AUTO_STATE.store(0, Ordering::Relaxed);
        SHARED.store(
            u8::from(mode == ParallelTickMode::On as u8),
            Ordering::Relaxed,
        );
    }
}

/// The server thread, after a tick that took `ms`, `cpu_ms` of them on the processor (where the
/// system tells): decides whether the next tick shares its phases' work. Only Auto has something
/// to decide. A dedicated server feeds it from its tick loop as the integrated one does, when the
/// host asks for Auto (`parallel_tick = "auto"`, `HORIZON_PARALLEL_TICK=auto`).
pub fn after_tick(ms: f32, cpu_ms: Option<f32>) {
    if forced().is_some() || SETTING.load(Ordering::Relaxed) != ParallelTickMode::Auto as u8 {
        return;
    }
    let mut auto = AUTO.lock().unwrap_or_else(PoisonError::into_inner);
    let was = auto.shared;
    let shared = auto.after_tick(ms, cpu_ms.unwrap_or(ms), horizon_threads::worker_count());
    AUTO_STATE.store(auto.state as u8, Ordering::Relaxed);
    if shared != was {
        let ms = auto.last_median;
        if shared {
            log::info!("Horizon: the tick takes {ms:.1} ms, sharing it among threads");
        } else {
            log::info!("Horizon: the tick takes {ms:.1} ms, back on one thread");
        }
        SHARED.store(u8::from(shared), Ordering::Relaxed);
    }
}

/// The setting, whether the next tick shares its work, and what Auto is doing (`watching`,
/// `trial` or `kept`), for the control status.
pub fn status() -> (&'static str, bool, &'static str) {
    let setting = match (forced(), SETTING.load(Ordering::Relaxed)) {
        (Some(_), _) => "forced",
        (None, 1) => "auto",
        (None, 2) => "on",
        _ => "off",
    };
    let auto = match AUTO_STATE.load(Ordering::Relaxed) {
        1 => "trial",
        2 => "kept",
        _ => "watching",
    };
    (setting, parallel_tick() > 0, auto)
}

/// Ticks in one of Auto's verdicts: five seconds of them.
const WINDOW: usize = 100;
/// The serial tick's median time on the processor, in milliseconds, from which Auto tries
/// sharing it: half of a tick's 50 ms. Below it the server keeps up easily, and helper threads
/// would only take cores from the frame and the chunk workers. Time on the processor, not on the
/// clock: ticks that are slow because the machine is busy or swapping have no work to share.
const HEAVY_MS: f32 = 25.0;
/// The shared tick's median under which Auto goes back to one thread: far enough under
/// `HEAVY_MS` that the serial tick it returns to doesn't start a trial again.
const LIGHT_MS: f32 = 10.0;
/// A trial is kept when the tick's median dropped under this share of what it was.
const KEEP_BELOW: f32 = 0.95;
/// Heavy windows in a row before a trial, and light ones before going back.
const STREAK: u8 = 2;
/// Windows left out after a switch, while the helper threads start and the caches settle.
const SETTLE: u8 = 1;
/// How many ticks after a trial that didn't help before the next one: 5 minutes after the
/// first, 20 after every later one.
const RETRY_AFTER: [u64; 2] = [5 * 60 * 20, 20 * 60 * 20];
/// Fewer hardware threads than this and Auto never shares the tick: the helpers would run on
/// the cores the frame and the chunk workers need.
const MIN_THREADS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// On one thread, looking for heavy ticks.
    Watching = 0,
    /// Shared for a trial: left out `SETTLE` windows, then measured against the ticks before.
    Trial = 1,
    /// The trial made the ticks faster: shared until they are light again.
    Kept = 2,
}

/// What Auto has decided and why, a window of ticks at a time. Nothing here starts a thread:
/// the caller applies what it answers.
struct Auto {
    window: Vec<f32>,
    /// The same ticks' time on the processor.
    cpu: Vec<f32>,
    shared: bool,
    state: State,
    /// Heavy windows in a row while watching, light ones while kept.
    streak: u8,
    /// The serial tick's median before the trial, in milliseconds.
    before: f32,
    /// Windows still left out after a switch.
    settling: u8,
    /// Ticks watched so far, and the tick from which the next trial may start.
    ticks: u64,
    retry_at: u64,
    /// Trials that didn't help.
    rejected: usize,
    last_median: f32,
}

impl Auto {
    const fn new() -> Self {
        Self {
            window: Vec::new(),
            cpu: Vec::new(),
            shared: false,
            state: State::Watching,
            streak: 0,
            before: 0.0,
            settling: 0,
            ticks: 0,
            retry_at: 0,
            rejected: 0,
            last_median: 0.0,
        }
    }

    /// A tick took `ms`, `cpu_ms` of them on the processor, on a machine with `threads`
    /// hardware threads: whether the next one shares its phases' work.
    fn after_tick(&mut self, ms: f32, cpu_ms: f32, threads: usize) -> bool {
        self.ticks += 1;
        self.window.push(ms);
        self.cpu.push(cpu_ms);
        if self.window.len() < WINDOW {
            return self.shared;
        }
        self.window.sort_by(f32::total_cmp);
        self.cpu.sort_by(f32::total_cmp);
        let (median, working) = (self.window[WINDOW / 2], self.cpu[WINDOW / 2]);
        self.window.clear();
        self.cpu.clear();
        self.last_median = median;
        if self.settling > 0 {
            self.settling -= 1;
            return self.shared;
        }
        match self.state {
            State::Watching => {
                let heavy =
                    working >= HEAVY_MS && threads >= MIN_THREADS && self.ticks >= self.retry_at;
                self.streak = if heavy { self.streak + 1 } else { 0 };
                if self.streak >= STREAK {
                    self.before = median;
                    self.switch(State::Trial, true);
                }
            }
            State::Trial => {
                if median < self.before * KEEP_BELOW {
                    self.state = State::Kept;
                    self.streak = 0;
                } else {
                    self.retry_at = self.ticks + RETRY_AFTER[self.rejected.min(1)];
                    self.rejected += 1;
                    self.switch(State::Watching, false);
                }
            }
            State::Kept => {
                self.streak = if median < LIGHT_MS {
                    self.streak + 1
                } else {
                    0
                };
                if self.streak >= STREAK {
                    self.switch(State::Watching, false);
                }
            }
        }
        self.shared
    }

    fn switch(&mut self, state: State, shared: bool) {
        self.state = state;
        self.shared = shared;
        self.streak = 0;
        self.settling = SETTLE;
    }
}

/// Whether a villager's brain draws from the villager's own random instead of the level's.
///
/// The level random is one stream, so a draw has to wait for every mob before it (`OrderedDraws`),
/// and a villager's brain draws on nearly every turn (behaviour durations, one-in-N rolls).
/// With the parallel tick on, those draws come from a stream seeded from the villager's own, so
/// no villager waits on another mob and the lanes run apart. That is the one place the parallel
/// tick departs from vanilla's draw order: the villagers' rolls have the same odds but not the
/// same values a vanilla level would give them. The seed is derived from the mob's id and the game
/// time, so the villager's entity random keeps vanilla's sequence. The serial loop does the same while the parallel
/// tick is on, so which path a tick takes never changes the result.
pub fn villagers_own_random() -> bool {
    OWN_RANDOM_TEST.load(Ordering::Relaxed) || parallel_tick() > 0
}

/// Whether a mob's turn draws the level-random rolls from a stream of its own instead of the level's.
///
/// A draw from the level's one stream waits until every mob before it in the list has had its turn
/// (`OrderedDraws`), in whatever lane those run. Some mobs draw on every tick (a fish that leads a
/// school rolls one in 200, `AbstractSchoolingFish.tick`; a parrot one in 400; brains draw their
/// durations), so on a busy level the lanes took turns instead of running side by side. With the
/// parallel tick on, a turn's level-random draws come from [`mob_turn_random`]: the same odds, not
/// the values a vanilla level would give, as for the villagers' brains above. No player can rely on
/// those values: the level's stream is shared with the spawns and the blocks of every chunk. The
/// mob's own entity random keeps vanilla's sequence, and the serial loop draws the same way while
/// the parallel tick is on, so which path a tick takes never changes the result.
pub fn mobs_own_random() -> bool {
    parallel_tick() > 0
}

/// The stream a mob's turn draws its level-random rolls from under [`mobs_own_random`]: seeded
/// from the mob's id and the game time (a splitmix64 mix), not from a draw.
pub fn mob_turn_random(id: u32, game_time: i64) -> horizon_core::random::LegacyRandom {
    let mut z = (u64::from(id) << 32) ^ (game_time as u64) ^ 0x6d6f_625f_7475_726e;
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    horizon_core::random::LegacyRandom::new((z ^ (z >> 31)) as i64)
}

/// A test's switch for [`villagers_own_random`], process-wide because the lanes are threads: any
/// gameplay test running at the same time in the same binary sees villagers on their own random, so
/// the tests that set it run alone (`--test-threads=1`).
#[doc(hidden)]
pub static OWN_RANDOM_TEST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// How many threads a phase may share its work among: `horizon-threads`' count (at most 8) while
/// the parallel tick is on, else 1.
pub fn lanes() -> usize {
    if parallel_tick() > 0 {
        horizon_threads::worker_count().clamp(1, 8)
    } else {
        1
    }
}

/// The pool: one thread fewer than a phase's lanes (the server thread takes parts too).
fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        log::info!("Horizon: starting the tick's helper threads");
        rayon::ThreadPoolBuilder::new()
            .num_threads(horizon_threads::sizing::for_pool(
                horizon_threads::sizing::Pool::TickHelper,
            ))
            .thread_name(|i| format!("tick-helper-{i}"))
            .build()
            .expect("the tick's helper threads start")
    })
}

/// `f` over `items` in parts on the pool, or all on this thread while the parallel tick is off or
/// for fewer than `min` items: every part's results, in the order of `items`, as one thread would
/// give them.
pub fn par_map<T: Sync, R: Send>(
    items: &[T],
    min: usize,
    f: impl Fn(&[T], &mut Vec<R>) + Sync,
) -> Vec<R> {
    par_map_on(items, min, lanes(), f)
}

/// `f` over `items` in chunks of `chunk`, in place, on the pool: the same sharing out as
/// `horizon_threads::par_chunks_mut`, without starting threads for every call. A chunk runs on
/// whatever thread takes it, so `f` must not depend on the order. Unlike [`par_map`] it does not
/// wait for the parallel tick's switch (the phases that use it ran shared out before the pool
/// existed, #8158): it shares out unless `HORIZON_SERIAL_TICK=all` or one thread is all there is.
/// The thread-locals the tick relies on do not follow a helper.
pub fn par_chunks_mut<T: Send>(items: &mut [T], chunk: usize, f: impl Fn(&mut [T]) + Send + Sync) {
    let lanes = if horizon_threads::serial_tick_all() {
        1
    } else {
        horizon_threads::worker_count().clamp(1, 8)
    };
    par_chunks_mut_on(items, chunk, lanes, f);
}

/// [`par_chunks_mut`] on `lanes` threads.
fn par_chunks_mut_on<T: Send>(
    items: &mut [T],
    chunk: usize,
    lanes: usize,
    f: impl Fn(&mut [T]) + Send + Sync,
) {
    let chunk = chunk.max(1);
    let count = items.len().div_ceil(chunk);
    if lanes <= 1 || count < 2 {
        for slice in items.chunks_mut(chunk) {
            f(slice);
        }
        return;
    }
    // A thread takes a chunk whole; the lock is only held while a chunk is taken, never while
    // it runs: the chunk is popped in a statement of its own, so the guard is gone before `f`
    // (a guard in a `while let` scrutinee would live through the loop body).
    let pending = Mutex::new(items.chunks_mut(chunk).collect::<Vec<_>>());
    let work = || {
        loop {
            let next = pending.lock().unwrap_or_else(PoisonError::into_inner).pop();
            let Some(slice) = next else { break };
            f(slice);
        }
    };
    pool().in_place_scope(|s| {
        for _ in 1..lanes.min(count) {
            s.spawn(|_| work());
        }
        work();
    });
}

/// [`par_map`] on `lanes` threads (a test's count, or a caller that caps them).
///
/// The parts are small (four a thread) and go to whichever thread is free, this one too: a helper
/// the machine is slow to run (the chunk workers compete for the same cores) holds the phase up by
/// one part at most instead of an eighth of the work, and the server thread works instead of only
/// waiting.
pub fn par_map_on<T: Sync, R: Send>(
    items: &[T],
    min: usize,
    lanes: usize,
    f: impl Fn(&[T], &mut Vec<R>) + Sync,
) -> Vec<R> {
    if lanes <= 1 || items.len() < min.max(2) {
        let mut out = Vec::with_capacity(items.len());
        f(items, &mut out);
        return out;
    }
    let size = items.len().div_ceil(lanes * 4);
    let count = items.len().div_ceil(size);
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Vec<R>>> = (0..count).map(|_| Mutex::new(Vec::new())).collect();
    let work = || {
        loop {
            let k = next.fetch_add(1, Ordering::Relaxed);
            if k >= count {
                break;
            }
            let part = &items[k * size..((k + 1) * size).min(items.len())];
            let mut done = Vec::with_capacity(part.len());
            f(part, &mut done);
            *slots[k].lock().unwrap_or_else(PoisonError::into_inner) = done;
        }
    };
    pool().in_place_scope(|s| {
        for _ in 1..lanes {
            s.spawn(|_| work());
        }
        work();
    });
    let mut out = Vec::with_capacity(items.len());
    for slot in slots {
        out.extend(slot.into_inner().unwrap_or_else(PoisonError::into_inner));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `HORIZON_PARALLEL_TICK` forces a mode for `0`, `1` and `verify`, and asks for Auto for
    /// `auto` instead of forcing one, so the machine still decides; anything else, and no variable,
    /// leaves the setting alone.
    #[test]
    fn the_variable_asks_for_auto_without_forcing_it() {
        assert_eq!(from_env(None), (None, false));
        assert_eq!(from_env(Some("auto")), (None, true), "the machine decides");
        assert_eq!(from_env(Some("0")), (Some(0), false));
        assert_eq!(from_env(Some("1")), (Some(1), false));
        assert_eq!(from_env(Some("verify")), (Some(2), false));
        for off in ["off", "false"] {
            assert_eq!(from_env(Some(off)), (Some(0), false), "{off}");
        }
        for on in ["on", "true"] {
            assert_eq!(from_env(Some(on)), (Some(1), false), "{on}");
        }
        for junk in ["", "2", "sometimes", "AUTO", " on"] {
            assert_eq!(from_env(Some(junk)), (None, false), "{junk:?}");
        }
    }

    /// Runs `body` in a process of its own with `HORIZON_PARALLEL_TICK=auto`: the variable is read
    /// once per process, so the case that needs it there needs its own process too, as
    /// `game::in_own_process` runs the cases that need their own process-wide state.
    fn with_auto_in_own_process(name: &str, body: impl FnOnce()) {
        const OWN_PROCESS: &str = "HORIZON_TEST_OWN_PROCESS";
        if std::env::var_os(OWN_PROCESS).is_some_and(|v| v == name) {
            return body();
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name, "--include-ignored", "--exact", "--test-threads=1"])
            .env(OWN_PROCESS, name)
            .env("HORIZON_PARALLEL_TICK", "auto")
            .output()
            .unwrap();
        let report = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success() && report.contains("test result: ok. 1 passed;"),
            "{name} failed in its own process:\n{report}"
        );
    }

    /// `HORIZON_PARALLEL_TICK=auto` must not land on top of a setting a window applied first: the
    /// window's settings run before the first tick, and the variable is read before a setting is
    /// applied, so the setting stands and the next tick stays on one thread.
    #[test]
    #[ignore = "daily"]
    fn the_variable_does_not_override_a_setting_applied_first() {
        with_auto_in_own_process(
            "parallel::tests::the_variable_does_not_override_a_setting_applied_first",
            || {
                set_setting(ParallelTickMode::Off);
                assert_eq!(status().0, "off", "the window's setting stands");
                assert!(!status().1, "the next tick stays on one thread");
            },
        );
    }

    /// The variable's `auto` applies the Auto setting when the variable is read, and a setting
    /// applied after that stands: `set_setting` reads the variable first, so Auto cannot land on
    /// top of a setting the window has applied.
    #[test]
    fn a_setting_applied_after_the_reading_stands() {
        assert_eq!(init_forced(Some("auto")), None, "auto forces no mode");
        assert_eq!(
            SETTING.load(Ordering::Relaxed),
            ParallelTickMode::Auto as u8,
            "auto puts the setting in Auto"
        );
        set_setting(ParallelTickMode::Off);
        assert_eq!(
            SETTING.load(Ordering::Relaxed),
            ParallelTickMode::Off as u8,
            "the setting stands"
        );
    }

    /// Auto keeps a quiet world on one thread, shares a heavy tick only after two heavy windows,
    /// keeps the trial when the ticks got faster, goes back once they are light, and after a
    /// trial that didn't help waits before the next.
    #[test]
    fn auto_shares_only_heavy_ticks_and_only_while_it_helps() {
        let run = |auto: &mut Auto, ms: f32, windows: usize| {
            let mut shared = auto.shared;
            for _ in 0..windows * WINDOW {
                shared = auto.after_tick(ms, ms, 8);
            }
            shared
        };
        let mut auto = Auto::new();
        assert!(!run(&mut auto, 4.0, 20), "a quiet world stays serial");
        assert!(!run(&mut auto, 36.0, 1), "one heavy window is not enough");
        assert!(run(&mut auto, 36.0, 1), "the second starts a trial");
        // The window after the switch is left out; the next one is faster: kept.
        assert!(run(&mut auto, 30.0, 2));
        assert_eq!(auto.state, State::Kept);
        // Light ticks for two windows: back on one thread, and it stays there.
        assert!(run(&mut auto, 6.0, 1));
        assert!(!run(&mut auto, 6.0, 1));
        assert!(!run(&mut auto, 9.0, 20));
        // A trial that doesn't help is undone, and not tried again for five minutes.
        assert!(
            run(&mut auto, 36.0, 3),
            "two heavy windows, then the one left out"
        );
        assert!(!run(&mut auto, 36.0, 2), "no faster: undone");
        assert!(!run(&mut auto, 36.0, 40), "waiting");
        assert!(run(&mut auto, 36.0, 20), "tried again after five minutes");
        // Too few hardware threads: never.
        let mut small = Auto::new();
        for _ in 0..20 * WINDOW {
            assert!(!small.after_tick(40.0, 40.0, 2));
        }
        // Ticks that are slow on the clock while the thread hardly works (a busy or swapping
        // machine) have nothing to share.
        let mut starved = Auto::new();
        for _ in 0..20 * WINDOW {
            assert!(!starved.after_tick(150.0, 4.0, 8));
        }
    }

    /// On several threads, every item's result comes back once, in the items' order, for item
    /// counts that don't divide into the parts evenly; a part may give several results or none.
    #[test]
    fn parts_come_back_in_order() {
        for n in [0usize, 1, 7, 33, 1000, 4097] {
            let items: Vec<u64> = (0..n as u64).collect();
            let f = |part: &[u64], out: &mut Vec<u64>| {
                for &x in part {
                    // Spin a little so the threads overlap.
                    let mut h = x;
                    for _ in 0..200 {
                        h = h.wrapping_mul(6364136223846793005).wrapping_add(1);
                    }
                    if x % 3 != 0 {
                        out.push(x * 10);
                    }
                    if x % 5 == 0 {
                        out.push(h);
                    }
                }
            };
            let mut serial = Vec::new();
            f(&items, &mut serial);
            assert_eq!(par_map_on(&items, 1, 4, f), serial, "{n} items");
        }
    }

    /// Every chunk is handled once, whole, and its items changed in place, for item counts that
    /// don't divide into chunks evenly.
    #[test]
    fn chunks_are_each_handled_once() {
        for n in [0usize, 1, 15, 16, 17, 79, 80, 1000] {
            let mut items: Vec<u64> = (0..n as u64).collect();
            par_chunks_mut_on(&mut items, 16, 4, |chunk| {
                assert!(chunk.len() <= 16);
                for x in chunk {
                    *x = *x * 3 + 1;
                }
            });
            assert_eq!(items, (0..n as u64).map(|x| x * 3 + 1).collect::<Vec<_>>());
        }
    }

    /// Chunks really run at the same time: one waits (bounded, so a serial run fails instead of
    /// hanging) until another has started, and each item is still handled exactly once.
    #[test]
    fn chunks_overlap_and_run_once() {
        use std::sync::atomic::AtomicBool;
        use std::time::{Duration, Instant};
        let active = AtomicUsize::new(0);
        let overlapped = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(10);
        let runs: Vec<AtomicUsize> = (0..64).map(|_| AtomicUsize::new(0)).collect();
        let mut items: Vec<usize> = (0..64).collect();
        par_chunks_mut_on(&mut items, 4, 4, |chunk| {
            for &i in chunk.iter() {
                runs[i].fetch_add(1, Ordering::SeqCst);
            }
            if active.fetch_add(1, Ordering::SeqCst) >= 1 {
                overlapped.store(true, Ordering::SeqCst);
            }
            while !overlapped.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            active.fetch_sub(1, Ordering::SeqCst);
        });
        assert!(
            overlapped.load(Ordering::SeqCst),
            "no two chunks ran at once"
        );
        assert!(runs.iter().all(|r| r.load(Ordering::SeqCst) == 1));
    }

    /// A panic in a part reaches the caller instead of being lost on a helper.
    #[test]
    fn a_panic_reaches_the_caller() {
        let items: Vec<u32> = (0..256).collect();
        let caught = std::panic::catch_unwind(|| {
            par_map_on(&items, 1, 4, |part, out: &mut Vec<u32>| {
                assert!(!part.contains(&200), "item 200");
                out.extend(part);
            })
        });
        assert!(caught.is_err());
    }
}
