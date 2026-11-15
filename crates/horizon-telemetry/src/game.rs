//! The game's part in opt-in data collection (docs/telemetry.md), behind the `game` feature. When
//! the player allows it, the game keeps a scrubbed crash report, or a play session's performance
//! summary, in the outbox. It never sends those itself: the launcher does, at its next start (the
//! game's only sending is the anonymous "I'm playing" ping, `horizon-heartbeat`). Without the
//! player's yes for that kind nothing is kept, and crash-reports/ stays local.
//!
//! It hears the game only through the platform's hooks ([`HOOKS`], `horizon_platform::hooks`);
//! the official executable registers it (`horizon_official::PARTS`).
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use horizon_platform::hooks::{self, Hooks, Milestone, PlayedFrame, WorldLeft};

use crate::payload::{Crash, Display, Perf, Settings, System, Timings};
use crate::scrub::Scrubber;
use crate::{Item, Payload, Source, outbox};

/// A session shorter than this says too little to keep.
const MIN_SECONDS: f32 = 60.0;

/// The details of a crash report that say what the game was doing, and only those: not the
/// world, the server or the resource packs.
const CRASH_DETAILS: [&str; 3] = ["Horizon Version", "Graphics Adapter", "Render Settings"];

static CORE_READY_MS: AtomicU32 = AtomicU32::new(u32::MAX);
static REST_READY_MS: AtomicU32 = AtomicU32::new(u32::MAX);
/// The screen the window opened on.
static DISPLAY: Mutex<Option<Display>> = Mutex::new(None);

/// The game's hooks, for `horizon_platform::hooks::register`.
pub static HOOKS: Telemetry = Telemetry {
    session: LazyLock::new(|| Mutex::new(Session::default())),
};

/// The play session's performance, as the game's hooks tell it.
pub struct Telemetry {
    session: LazyLock<Mutex<Session>>,
}

impl Telemetry {
    fn session(&self) -> std::sync::MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Hooks for Telemetry {
    fn startup(&self, milestone: Milestone, since_start: Duration) {
        match milestone {
            Milestone::CoreReady => mark_ready(&CORE_READY_MS, since_start),
            Milestone::RestReady => mark_ready(&REST_READY_MS, since_start),
            Milestone::TitleShown => self.session().title_shown(since_start),
        }
    }

    /// A player who allows performance statistics also says what their slow frames spent their
    /// time in.
    fn wants_frame_steps(&self, home: &Path) -> bool {
        crate::Consent::load(home).allows(crate::Kind::Perf)
    }

    fn world_shown(&self, since_opening: Duration) {
        self.session().world_shown(since_opening);
    }

    fn played_frame(&self, frame: &PlayedFrame<'_>) {
        self.session().frame(frame.ms, || frame.slowest_step());
    }

    fn world_left(&self, left: &WorldLeft<'_>) {
        let settings = Settings {
            render_distance: left.render_distance.clamp(0, 255) as u8,
            simulation_distance: left.simulation_distance.clamp(0, 255) as u8,
            max_fps: left
                .max_fps
                .map_or(0, |f| f.min(u32::from(u16::MAX)) as u16),
            vsync: left.vsync,
            fullscreen: left.fullscreen,
            graphics: left.graphics.to_owned(),
        };
        self.session()
            .finish(left.home, left.version, left.gpu, settings);
    }

    fn display(&self, display: &hooks::Display) {
        *DISPLAY.lock().unwrap_or_else(|e| e.into_inner()) = Some(Display {
            width: display.width,
            height: display.height,
            refresh_hz: display.refresh_hz.unwrap_or(0),
            scale: display.scale as f32,
        });
    }

    fn crashed(&self, crash: &hooks::Crash<'_>) {
        keep_crash(crash);
    }

    fn benchmarked(&self, run: &hooks::Benchmarked<'_>) {
        keep_benchmark(run);
    }
}

fn mark_ready(mark: &AtomicU32, since_start: Duration) {
    let ms = since_start.as_millis().min(u128::from(u32::MAX - 1)) as u32;
    let _ = mark.compare_exchange(u32::MAX, ms, Ordering::Relaxed, Ordering::Relaxed);
}

fn readiness_ms(mark: &AtomicU32) -> Option<u32> {
    let ms = mark.load(Ordering::Relaxed);
    (ms != u32::MAX).then_some(ms)
}

/// This machine, with the GPU once the renderer has one and the screen. The drive is the one
/// under the game folder (the current folder).
pub fn system(gpu: Option<&hooks::Gpu>) -> System {
    let home = std::env::current_dir().ok();
    let mut system = System::detect_in(home.as_deref());
    if let Some(gpu) = gpu {
        system.vram_mb = match gpu.kind.as_str() {
            "discrete" => crate::system::vram_mb(&gpu.vendor).unwrap_or(0),
            _ => 0,
        };
        system.gpu = gpu.name.clone();
        system.gpu_vendor = gpu.vendor.clone();
        system.gpu_kind = gpu.kind.clone();
        system.driver = gpu.driver.clone();
        system.backend = gpu.backend.clone();
    }
    system.display = DISPLAY.lock().unwrap_or_else(|e| e.into_inner()).clone();
    system
}

/// What a crash report becomes when it may leave: the error and stack, the thread, and the
/// details in [`CRASH_DETAILS`], with the world's name and folder and the server's address
/// scrubbed wherever they appear. No log lines.
pub fn crash_item(report: &hooks::Crash<'_>) -> Item {
    let mut scrubber = Scrubber::new();
    for (key, value) in report.details {
        match key.as_str() {
            // `Name (saves/folder)`, as game.rs sets it.
            "World" => {
                let (name, folder) = value.rsplit_once(" (saves/").unwrap_or((value, ""));
                scrubber = scrubber.term(name).term(folder.trim_end_matches(')'));
            }
            "Server" => {
                let host = value.rsplit_once(':').map_or(value.as_str(), |(h, _)| h);
                scrubber = scrubber.term(value.as_str()).term(host);
            }
            _ => {}
        }
    }
    let crash = Crash {
        system: system(report.gpu),
        error: scrubber.scrub(report.error),
        stack: scrubber.scrub(report.stack),
        thread: report.thread.to_owned(),
        details: report
            .details
            .iter()
            .filter(|(k, _)| CRASH_DETAILS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), scrubber.scrub(v)))
            .collect(),
    };
    let mut item = Item::new(Source::Game, report.version, Payload::Crash(crash));
    // A huge stack keeps its top, where the error is.
    while item.validate().is_err() {
        let Payload::Crash(crash) = &mut item.payload else {
            break;
        };
        let keep = crash.stack.lines().count() * 3 / 4;
        if keep == 0 {
            crash.stack.clear();
            break;
        }
        crash.stack = crash
            .stack
            .lines()
            .take(keep)
            .map(|l| format!("{l}\n"))
            .collect();
    }
    item
}

/// Keeps the crash for the launcher to send, if the player allows crash reports.
pub fn keep_crash(report: &hooks::Crash<'_>) {
    if let Err(e) = outbox::push(report.home, &crash_item(report)) {
        log::warn!("Horizon: could not keep the crash report to send: {e}");
    }
}

/// Keeps a benchmark report (`--benchmark`) for the launcher to send, if the player allows
/// benchmark uploads: the report as docs/benchmark-report.md defines it, dated to the day.
pub fn keep_benchmark(run: &hooks::Benchmarked<'_>) {
    let data = match serde_json::from_str::<serde_json::Value>(run.report) {
        Ok(data) => data,
        Err(e) => {
            log::warn!("Horizon: the benchmark report to send is not JSON: {e}");
            return;
        }
    };
    let item = Item::new(Source::Game, run.version, Payload::Benchmark(data));
    if let Err(e) = outbox::push(run.home, &item) {
        log::warn!("Horizon: could not keep the benchmark report to send: {e}");
    }
}

/// One play session's performance, from entering a world to leaving it: frame and tick
/// times as [`Timings`] (fixed size whatever the length), never the frames themselves.
#[derive(Default)]
pub struct Session {
    frames: Timings,
    ticks: Timings,
    played_ms: f32,
    to_title_ms: Option<u32>,
    /// The start was measured: only the first session carries it.
    title_measured: bool,
    to_world_ms: Option<u32>,
    slow_frames: Vec<crate::payload::SlowFrames>,
}

impl Session {
    /// The title screen drew its first frame, `since_start` after the game started.
    pub fn title_shown(&mut self, since_start: Duration) {
        if !self.title_measured {
            self.title_measured = true;
            self.to_title_ms = Some(since_start.as_millis().min(u128::from(u32::MAX)) as u32);
        }
    }

    /// A world's first frame, `since_opening` after it started opening: the session's first
    /// world counts.
    pub fn world_shown(&mut self, since_opening: Duration) {
        if self.to_world_ms.is_none() {
            self.to_world_ms = Some(since_opening.as_millis().min(u128::from(u32::MAX)) as u32);
        }
    }

    /// A frame drawn while playing (not in menus or loading): a frame of `SLOW_FRAME_MS` or
    /// more counts under the step it spent most of its time in (`slowest_step`, the game's
    /// `spans::culprit`).
    pub fn frame(&mut self, ms: f32, slowest_step: impl FnOnce() -> Option<(String, f32)>) {
        use crate::payload::{SLOW_FRAME_MS, SlowFrames};
        if ms >= SLOW_FRAME_MS {
            let (mut path, span_ms) = slowest_step().unwrap_or_default();
            // A path too long for the item keeps its last steps.
            while path.len() > SlowFrames::MAX_NAME
                && let Some((_, rest)) = path.split_once('/')
            {
                path = rest.to_owned();
            }
            SlowFrames::record(&mut self.slow_frames, ms, &path, span_ms);
        }
        self.frames.record(ms);
        self.played_ms += ms;
    }

    /// One tick of the integrated server, in single player.
    pub fn tick(&mut self, ms: f32) {
        self.ticks.record(ms);
    }

    /// The session's summary, if it was long enough to say something.
    pub fn summary(&self, gpu: Option<&hooks::Gpu>, settings: Settings) -> Option<Perf> {
        let seconds = self.played_ms / 1000.0;
        (seconds >= MIN_SECONDS).then(|| Perf {
            system: system(gpu),
            settings,
            seconds: seconds as u32,
            frames: self.frames.summary(),
            ticks: (self.ticks.count() > 0).then(|| self.ticks.summary()),
            peak_ram_mb: peak_ram_mb(),
            to_title_ms: self.to_title_ms,
            to_world_ms: self.to_world_ms,
            to_core_ms: self.to_title_ms.and_then(|_| readiness_ms(&CORE_READY_MS)),
            to_rest_ms: self.to_title_ms.and_then(|_| readiness_ms(&REST_READY_MS)),
            slow_frames: self.slow_frames.clone(),
        })
    }

    /// Keeps the session's summary for the launcher to send, if the player allows performance
    /// statistics, and starts the next session (the time to the title goes with the first).
    pub fn finish(
        &mut self,
        home: &Path,
        version: &str,
        gpu: Option<&hooks::Gpu>,
        settings: Settings,
    ) {
        if crate::Consent::load(home).allows(crate::Kind::Perf)
            && let Some(perf) = self.summary(gpu, settings)
        {
            let item = Item::new(Source::Game, version, Payload::Perf(perf));
            if let Err(e) = outbox::push(home, &item) {
                log::warn!("Horizon: could not keep the session's performance summary: {e}");
            }
        }
        *self = Self {
            title_measured: self.title_measured,
            ..Self::default()
        };
    }
}

/// The game's resident memory: the peak on Linux, what it uses now elsewhere.
fn peak_ram_mb() -> u32 {
    let kib = if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|t| {
                t.lines()
                    .find(|l| l.starts_with("VmHWM:"))?
                    .split_whitespace()
                    .nth(1)?
                    .parse::<u64>()
                    .ok()
            })
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
    } else {
        None
    };
    kib.map_or(0, |k| (k / 1024) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Choices, Consent, Kind};

    fn home(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "horizon-client-telemetry-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn allow(home: &Path, choices: Choices) {
        let mut consent = Consent::load(home);
        consent.answer(choices);
        consent.save(home).unwrap();
    }

    /// A crash report's parts, as `crash.rs` gives them to the hooks.
    struct Report {
        error: String,
        stack: String,
        details: Vec<(String, String)>,
    }

    impl Report {
        fn crash<'a>(&'a self, home: &'a Path) -> hooks::Crash<'a> {
            hooks::Crash {
                home,
                version: "v2026.10.15",
                error: &self.error,
                stack: &self.stack,
                thread: "Render thread",
                details: &self.details,
                gpu: None,
            }
        }
    }

    fn report() -> Report {
        Report {
            error:
                "panicked at src/save.rs:10:5:\nno level.dat in /Users/ana/horizon/saves/Ana's Base"
                    .into(),
            stack: "horizon_client::save::load (crates/horizon-client/src/save.rs:10:5)\n".into(),
            details: vec![
                (
                    "Horizon Version".into(),
                    "v2026.10.15 (client 0.1.0)".into(),
                ),
                (
                    "Operating System".into(),
                    "macos (aarch64) version macOS 26.0".into(),
                ),
                ("World".into(), "Castle Ana (saves/Ana's Base)".into()),
                ("Server".into(), "play.example.net:25565".into()),
                ("Resource Packs".into(), "Ana's pack".into()),
            ],
        }
    }

    #[test]
    fn a_crash_keeps_only_what_says_what_happened() {
        let report = report();
        let item = crash_item(&report.crash(Path::new("home")));
        let Payload::Crash(crash) = &item.payload else {
            panic!("a crash item");
        };
        assert_eq!(
            crash.error,
            "panicked at src/save.rs:10:5:\nno level.dat in /Users/<user>/horizon/saves/<private>"
        );
        assert_eq!(
            crash.details,
            [(
                "Horizon Version".to_owned(),
                "v2026.10.15 (client 0.1.0)".to_owned()
            )]
        );
        let json = serde_json::to_string(&item).unwrap();
        for private in ["Castle Ana", "Ana's Base", "example.net", "Ana's pack"] {
            assert!(!json.contains(private), "{private} in {json}");
        }
        assert!(item.validate().is_ok());
    }

    #[test]
    fn a_huge_stack_is_cut_to_fit() {
        let mut report = report();
        report.stack =
            "horizon_client::game::deep (crates/horizon-client/src/game.rs:1:1)\n".repeat(2000);
        let item = crash_item(&report.crash(Path::new("home")));
        assert!(item.validate().is_ok());
        let Payload::Crash(crash) = item.payload else {
            panic!("a crash item");
        };
        assert!(crash.stack.lines().count() > 100);
    }

    #[test]
    fn crashes_stay_local_without_consent() {
        let home = home("crash");
        let report = report();
        keep_crash(&report.crash(&home));
        assert!(outbox::pending(&home).is_empty());
        allow(
            &home,
            Choices {
                performance: true,
                ..Choices::NONE
            },
        );
        keep_crash(&report.crash(&home));
        assert!(outbox::pending(&home).is_empty());
        allow(
            &home,
            Choices {
                crashes: true,
                ..Choices::NONE
            },
        );
        keep_crash(&report.crash(&home));
        let pending = outbox::pending(&home);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.kind(), Kind::Crash);
    }

    #[test]
    fn a_benchmark_is_kept_only_with_its_consent() {
        let home = home("benchmark");
        let run = hooks::Benchmarked {
            home: &home,
            version: "v2026.10.15",
            report: r#"{"kind":"benchmark"}"#,
        };
        keep_benchmark(&run);
        assert!(outbox::pending(&home).is_empty());
        allow(
            &home,
            Choices {
                benchmarks: true,
                ..Choices::NONE
            },
        );
        keep_benchmark(&hooks::Benchmarked {
            report: "not json",
            ..run
        });
        assert!(outbox::pending(&home).is_empty());
        keep_benchmark(&run);
        let pending = outbox::pending(&home);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.kind(), Kind::Benchmark);
    }

    #[test]
    fn the_time_to_a_world_is_the_sessions_first() {
        let mut session = Session::default();
        session.world_shown(Duration::from_millis(4200));
        session.world_shown(Duration::from_millis(900));
        assert_eq!(session.to_world_ms, Some(4200));
    }

    #[test]
    fn the_time_to_the_title_goes_with_the_first_session_only() {
        let home = home("title");
        let mut session = Session::default();
        session.title_shown(Duration::from_millis(3100));
        assert_eq!(session.to_title_ms, Some(3100));
        session.finish(&home, "dev", None, Settings::default());
        session.title_shown(Duration::from_millis(9000));
        assert_eq!(session.to_title_ms, None);
    }

    #[test]
    fn a_session_is_kept_only_with_consent_and_once_long_enough() {
        let home = home("session");
        let mut session = Session::default();
        session.world_shown(Duration::from_millis(1500));
        (0..2000).for_each(|_| session.frame(16.6, || None));
        session.finish(&home, "dev", None, Settings::default());
        assert!(outbox::pending(&home).is_empty(), "33 s, and no consent");
        allow(
            &home,
            Choices {
                performance: true,
                ..Choices::NONE
            },
        );
        (0..2000).for_each(|_| session.frame(16.6, || None));
        session.finish(&home, "dev", None, Settings::default());
        assert!(outbox::pending(&home).is_empty(), "33 s is too short");
        session.world_shown(Duration::from_millis(1500));
        (0..4000).for_each(|_| session.frame(16.6, || None));
        (0..1200).for_each(|_| session.tick(4.0));
        session.finish(
            &home,
            "dev",
            None,
            Settings {
                render_distance: 12,
                ..Settings::default()
            },
        );
        let pending = outbox::pending(&home);
        assert_eq!(pending.len(), 1);
        let Payload::Perf(perf) = &pending[0].1.payload else {
            panic!("a perf item");
        };
        assert_eq!(perf.seconds, 66);
        assert_eq!(perf.frames.count, 4000);
        assert_eq!(perf.ticks.as_ref().unwrap().p50_ms, 4.0);
        assert_eq!(perf.settings.render_distance, 12);
        assert!(perf.to_world_ms.is_some());
        assert!(serde_json::to_vec(&pending[0].1).unwrap().len() < 1024);
    }

    /// Frames of 250 ms or more count under the step that took most of them; a frame with no
    /// span counts under none. Faster frames leave no entry.
    #[test]
    fn slow_frames_are_put_down_to_their_step() {
        // What the game's `spans::culprit` names for each frame.
        let step = |path: &'static str, ms: f32| move || Some((path.to_owned(), ms));
        let mut session = Session::default();
        (0..4000).for_each(|_| session.frame(16.6, step("tick", 10.0)));
        session.frame(320.0, step("renderer/r_uploads", 280.0));
        session.frame(900.0, step("renderer/r_uploads", 280.0));
        session.frame(260.0, || None);
        session.frame(249.0, step("renderer/r_uploads", 280.0));
        let perf = session.summary(None, Settings::default()).unwrap();
        let seen: Vec<_> = perf
            .slow_frames
            .iter()
            .map(|s| (s.span.as_str(), s.frames, s.frame_max_ms, s.span_max_ms))
            .collect();
        assert_eq!(
            seen,
            [("renderer/r_uploads", 2, 900.0, 280.0), ("", 1, 260.0, 0.0)]
        );
    }
}
