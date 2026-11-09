//! The game's extension points for code above the engine (docs/layers.md, "How a Horizon crate
//! plugs in"). A platform or Horizon crate implements [`Hooks`] and [`register`]s it before the
//! game starts; the engine calls every registered hook at the points below. This crate names no
//! type from above, so the engine can offer it; `horizon-platform` re-exports it as
//! `horizon_platform::hooks`, which is where the Horizon layer uses it from.
//!
//! Registration happens once, before the game runs: the list is sealed the first time the engine
//! dispatches, and a hook registered after that is refused. With nothing registered a dispatch
//! is one load and an empty loop.
//!
//! A new hook is a new method with an empty default body, so existing implementations keep
//! compiling, and a dispatch function in [`engine`] that the engine calls at the place vanilla
//! does the matching thing.
//!
//! The platform registries (docs/layers.md, "Platform registries") have the same shape:
//! [`settings`] for the settings each crate keeps in its own lines, [`pages`] for its settings
//! pages, [`hud`] for what it draws on the HUD.
#![forbid(unsafe_code)]

pub mod hud;
pub mod pages;
mod registry;
pub mod settings;
pub mod tunnel;

use registry::Registry;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

/// What a crate above the engine hears from the game. Every method has an empty default:
/// implement only those you need. Hooks run on the game's thread (the render thread) unless a
/// method says otherwise; they must return quickly and must not panic.
pub trait Hooks: Sync {
    /// The game started for someone playing it: once, after the log and the crash handler, before
    /// the first frame. Not for a dedicated server, a benchmark, an offscreen run or a run that
    /// quits by itself or that horizon-ctl drives (those start in a source checkout).
    fn started(&self, _game: &Started<'_>) {}

    /// The world this window plays was opened to LAN or to friends (`true`), or no longer is
    /// (`false`). Called when it changes, not every frame.
    fn published(&self, _open: bool) {}

    /// The game is ending normally: its window closed and the world is saved. Not after a crash.
    fn shutdown(&self) {}

    /// A moment of the game's start, once each, with the time since the process started.
    fn startup(&self, _milestone: Milestone, _since_start: Duration) {}

    /// Whether this hook wants the render thread to time its steps in every frame, for
    /// [`Frame::slowest_step`]. Asked once as the game starts (not on a dedicated server);
    /// `home` is the game's home folder. Timing the steps costs microseconds a frame.
    fn wants_frame_steps(&self, _home: &Path) -> bool {
        false
    }

    /// A world's first frame was drawn: the time since it started opening (single player, a
    /// server, LAN). Every time a world is entered.
    fn world_shown(&self, _since_opening: Duration) {}

    /// A frame drawn while playing: not in the menus or while a world loads. Every frame.
    fn frame(&self, _frame: &Frame<'_>) {}

    /// The player left a world (to the title screen, or by quitting): the end of a play
    /// session, with the settings it was played with.
    fn world_left(&self, _left: &WorldLeft<'_>) {}

    /// The resolver of `friend:` addresses this crate offers ([`tunnel`]); the first hook that
    /// offers one is used. Asked on the network thread when joining and on the game thread when
    /// an address is shown, never per frame.
    fn tunnel(&self) -> Option<&dyn tunnel::Resolver> {
        None
    }
}

/// A moment of the game's start ([`Hooks::startup`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Milestone {
    /// The resources the menus need are loaded.
    CoreReady,
    /// The rest (what a world needs) too.
    RestReady,
    /// The title screen drew its first frame.
    TitleShown,
}

impl Milestone {
    fn bit(self) -> u8 {
        match self {
            Self::CoreReady => 1,
            Self::RestReady => 2,
            Self::TitleShown => 4,
        }
    }
}

/// What [`Hooks::frame`] gets.
pub struct Frame<'a> {
    /// How long the frame took.
    pub ms: f32,
    slowest_step: &'a dyn Fn() -> Option<(String, f32)>,
}

impl<'a> Frame<'a> {
    /// `slowest_step` gives [`Frame::slowest_step`], only when asked.
    pub fn new(ms: f32, slowest_step: &'a dyn Fn() -> Option<(String, f32)>) -> Self {
        Self { ms, slowest_step }
    }

    /// The step the frame spent the most time in (`render/terrain`) and its milliseconds, when
    /// the steps were timed ([`Hooks::wants_frame_steps`]). Worked out on each call: ask only
    /// for a slow frame.
    pub fn slowest_step(&self) -> Option<(String, f32)> {
        (self.slowest_step)()
    }
}

/// What [`Hooks::world_left`] gets: the game's home folder and the settings played with.
pub struct WorldLeft<'a> {
    pub home: &'a Path,
    pub render_distance: i32,
    pub simulation_distance: i32,
    /// `None`: unlimited.
    pub max_fps: Option<u32>,
    pub vsync: bool,
    pub fullscreen: bool,
    /// The graphics preset's name.
    pub graphics: &'a str,
}

/// What [`Hooks::started`] gets.
#[non_exhaustive]
pub struct Started<'a> {
    /// The build: the release tag (`0.2.31`), or `dev <crate version>` for a build from source.
    pub version: &'a str,
}

impl<'a> Started<'a> {
    pub fn new(version: &'a str) -> Self {
        Self { version }
    }
}

static HOOKS: Registry<dyn Hooks> = Registry::new();

/// Why [`register`] refused a hook.
#[derive(Debug, PartialEq, Eq)]
pub struct TooLate;

impl std::fmt::Display for TooLate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the game already started: hooks register before it runs")
    }
}

impl std::error::Error for TooLate {}

/// Adds `hooks` to those the game calls. Call it before the game starts (from the executable's
/// `main`, before it runs the game); once the engine has dispatched, the list is fixed and this
/// returns [`TooLate`]. Hooks are called in the order they registered.
pub fn register(hooks: &'static dyn Hooks) -> Result<(), TooLate> {
    HOOKS.register(hooks)
}

/// The registered hooks, sealing the list on the first call.
fn all() -> &'static [&'static dyn Hooks] {
    HOOKS.all()
}

/// The engine's side: where it calls the hooks. Only the engine calls these.
pub mod engine {
    use super::*;

    /// Seals every list (hooks and the registries); their first use does it anyway. Call where
    /// registration ends.
    pub fn seal() {
        all();
        crate::settings::all();
        crate::pages::engine::all();
        crate::hud::engine::all();
    }

    /// [`Hooks::started`].
    pub fn started(game: &Started<'_>) {
        for hooks in all() {
            hooks.started(game);
        }
    }

    /// Whether the world was open to LAN or friends at the last call.
    static PUBLISHED: AtomicBool = AtomicBool::new(false);

    /// [`Hooks::published`], when `open` differs from the last call's. Cheap enough to call
    /// every frame.
    pub fn published(open: bool) {
        let hooks = all();
        if !hooks.is_empty() && PUBLISHED.swap(open, Ordering::Relaxed) != open {
            for h in hooks {
                h.published(open);
            }
        }
    }

    /// [`Hooks::shutdown`].
    pub fn shutdown() {
        for hooks in all() {
            hooks.shutdown();
        }
    }

    /// The milestones already said.
    static MILESTONES: AtomicU8 = AtomicU8::new(0);

    /// [`Hooks::startup`], the first time `milestone` is reached; later calls do nothing.
    pub fn startup(milestone: Milestone, since_start: Duration) {
        if MILESTONES.fetch_or(milestone.bit(), Ordering::Relaxed) & milestone.bit() == 0 {
            for hooks in all() {
                hooks.startup(milestone, since_start);
            }
        }
    }

    /// [`Hooks::wants_frame_steps`]: whether any hook wants them.
    pub fn wants_frame_steps(home: &Path) -> bool {
        all().iter().any(|h| h.wants_frame_steps(home))
    }

    /// [`Hooks::world_shown`].
    pub fn world_shown(since_opening: Duration) {
        for hooks in all() {
            hooks.world_shown(since_opening);
        }
    }

    /// [`Hooks::frame`].
    pub fn frame(frame: &Frame<'_>) {
        for hooks in all() {
            hooks.frame(frame);
        }
    }

    /// [`Hooks::world_left`].
    pub fn world_left(left: &WorldLeft<'_>) {
        for hooks in all() {
            hooks.world_left(left);
        }
    }

    /// [`Hooks::tunnel`]: the first registered hook's resolver.
    pub fn tunnel() -> Option<&'static dyn tunnel::Resolver> {
        all().iter().find_map(|hooks| hooks.tunnel())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// Records what it heard. One process-wide list, as the game has: this test is the only
    /// one that registers or dispatches.
    struct Recorder(StdMutex<Vec<String>>);

    impl Hooks for Recorder {
        fn started(&self, game: &Started<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("started {}", game.version));
        }
        fn published(&self, open: bool) {
            self.0.lock().unwrap().push(format!("published {open}"));
        }
        fn startup(&self, milestone: Milestone, since_start: Duration) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{milestone:?} {}", since_start.as_millis()));
        }
        fn wants_frame_steps(&self, _home: &Path) -> bool {
            true
        }
        fn frame(&self, frame: &Frame<'_>) {
            let step = frame
                .slowest_step()
                .map(|(name, ms)| format!(" {name} {ms}"));
            self.0
                .lock()
                .unwrap()
                .push(format!("frame {}{}", frame.ms, step.unwrap_or_default()));
        }
    }

    /// Implements nothing: the defaults do nothing.
    struct Silent;
    impl Hooks for Silent {}

    static RECORDER: Recorder = Recorder(StdMutex::new(Vec::new()));

    #[test]
    fn registered_hooks_hear_the_game_and_late_ones_are_refused() {
        register(&RECORDER).unwrap();
        register(&Silent).unwrap();
        engine::started(&Started::new("0.2.31"));
        // Changes only: the same state every frame is said once.
        for open in [false, true, true, true, false, false] {
            engine::published(open);
        }
        // Each milestone once: the first time it is reached.
        engine::startup(Milestone::CoreReady, Duration::from_millis(900));
        engine::startup(Milestone::TitleShown, Duration::from_millis(1500));
        engine::startup(Milestone::CoreReady, Duration::from_millis(4000));
        // Any hook that wants the steps timed has them.
        assert!(engine::wants_frame_steps(Path::new("home")));
        engine::frame(&Frame::new(70.0, &|| Some(("render/terrain".into(), 50.0))));
        engine::shutdown();
        assert_eq!(
            *RECORDER.0.lock().unwrap(),
            [
                "started 0.2.31",
                "published true",
                "published false",
                "CoreReady 900",
                "TitleShown 1500",
                "frame 70 render/terrain 50",
            ]
        );
        assert_eq!(register(&Silent), Err(TooLate));
    }
}
