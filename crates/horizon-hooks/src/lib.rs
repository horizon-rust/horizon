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

pub mod containers;
pub mod hud;
pub mod look;
pub mod pages;
mod registry;
pub mod settings;

use containers::{Container, SlotId};
use registry::Registry;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

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

    /// This window starts connecting to a multiplayer server (the server list, a direct
    /// connection, a Realm, a LAN world). Not single player's own server.
    fn connecting(&self, _server: &Server<'_>) {}

    /// The server let the player in.
    fn joined(&self) {}

    /// The player left the server on purpose: nothing to retry.
    fn left(&self) {}

    /// The connection ended without the player leaving it (lost, closed, kicked, refused). A hook
    /// may plan another attempt: the engine shows its countdown on the Disconnected screen, with
    /// Cancel, and connects again when it is due. Every hook hears the end; the first plan wins.
    fn ended(&self, _end: &Ended<'_>) -> Option<Reconnect> {
        None
    }

    // An open container screen (`containers`): each input hook returns whether it took the
    // input, and then vanilla's handling of it doesn't run. The first hook that takes it wins.

    /// A mouse button went down over the panel, on `slot` or none.
    fn container_press(&self, _c: &mut dyn Container, _slot: Option<SlotId>, _right: bool) -> bool {
        false
    }
    /// The cursor moved with a button held, now over `slot` or none.
    fn container_drag(&self, _c: &mut dyn Container, _slot: Option<SlotId>) -> bool {
        false
    }
    /// The button went up. Every hook hears it.
    fn container_release(&self, _c: &mut dyn Container) {}
    /// The wheel turned `steps` notches over `slot` or none (up is positive; a trackpad turns it
    /// in fractions).
    fn container_wheel(&self, _c: &mut dyn Container, _slot: Option<SlotId>, _steps: f32) -> bool {
        false
    }
    /// A key or a mouse button the screen got, by its name (`key.keyboard.r`, `key.mouse.middle`),
    /// with the cursor over `slot` or none.
    fn container_key(&self, _c: &mut dyn Container, _slot: Option<SlotId>, _key: &str) -> bool {
        false
    }
    /// Draws over the panel, from its top left corner, after the slots and before the tooltip.
    fn container_draw(&self, _c: &dyn Container, _canvas: &mut dyn hud::Canvas) {}
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

/// What [`Hooks::connecting`] gets.
#[non_exhaustive]
pub struct Server<'a> {
    /// As the player gave it (`host[:port]`).
    pub address: &'a str,
    /// The player name the connection uses.
    pub name: &'a str,
    /// Found on the local network.
    pub lan: bool,
}

impl<'a> Server<'a> {
    pub fn new(address: &'a str, name: &'a str, lan: bool) -> Self {
        Self { address, name, lan }
    }
}

/// What [`Hooks::ended`] gets.
#[non_exhaustive]
pub struct Ended<'a> {
    /// The reason shown on the Disconnected screen.
    pub reason: &'a str,
    /// Its translation key, when it has one (`disconnect.timeout`, `multiplayer.disconnect.kicked`).
    pub key: Option<&'a str>,
    /// The server sent it (a disconnect packet), rather than the connection failing.
    pub from_server: bool,
}

impl<'a> Ended<'a> {
    pub fn new(reason: &'a str, key: Option<&'a str>, from_server: bool) -> Self {
        Self {
            reason,
            key,
            from_server,
        }
    }
}

/// Another attempt at a server, planned by [`Hooks::ended`].
#[derive(Clone, Debug, PartialEq)]
pub struct Reconnect {
    pub address: String,
    pub name: String,
    pub lan: bool,
    /// When to connect.
    pub at: Instant,
    /// This attempt, 1-based, of `attempts`.
    pub attempt: usize,
    pub attempts: usize,
}

impl Reconnect {
    /// Whole seconds left, rounded up.
    pub fn seconds_left(&self, now: Instant) -> u64 {
        let left = self.at.saturating_duration_since(now);
        left.as_secs() + u64::from(left.subsec_nanos() > 0)
    }
    pub fn due(&self, now: Instant) -> bool {
        now >= self.at
    }
    /// The Disconnected screen's countdown line.
    pub fn text(&self, now: Instant) -> String {
        format!(
            "Reconnecting in {} s (attempt {} of {})",
            self.seconds_left(now),
            self.attempt,
            self.attempts
        )
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

thread_local! {
    static SEAT: Cell<usize> = const { Cell::new(0) };
}

/// The splitscreen player whose turn it is: 0 for P1 (and without splitscreen), 1 for P2...
/// Splitscreen players are one process, each `Game` in turn on the game's thread, so a crate's
/// statics are shared by them: a crate that keeps per-player state (settings, pages, HUD) indexes
/// it by this. A player keeps its seat while it plays. Other threads always see 0.
pub fn seat() -> usize {
    SEAT.get()
}

/// The engine's side: where it calls the hooks. Only the engine calls these.
pub mod engine {
    use super::*;

    /// The player whose turn starts on this thread ([`seat`]).
    pub fn set_seat(seat: usize) {
        SEAT.set(seat);
    }

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

    /// [`Hooks::connecting`].
    pub fn connecting(server: &Server<'_>) {
        for hooks in all() {
            hooks.connecting(server);
        }
    }

    /// [`Hooks::joined`].
    pub fn joined() {
        for hooks in all() {
            hooks.joined();
        }
    }

    /// [`Hooks::left`].
    pub fn left() {
        for hooks in all() {
            hooks.left();
        }
    }

    /// Whether a container screen's input reaches a hook: false while none is registered, and
    /// the engine builds no `Container` then.
    pub fn containers() -> bool {
        !all().is_empty()
    }

    /// [`Hooks::container_press`]: true when a hook took it.
    pub fn container_press(c: &mut dyn Container, slot: Option<SlotId>, right: bool) -> bool {
        all().iter().any(|h| h.container_press(c, slot, right))
    }

    /// [`Hooks::container_drag`].
    pub fn container_drag(c: &mut dyn Container, slot: Option<SlotId>) -> bool {
        all().iter().any(|h| h.container_drag(c, slot))
    }

    /// [`Hooks::container_release`], to every hook.
    pub fn container_release(c: &mut dyn Container) {
        for hooks in all() {
            hooks.container_release(c);
        }
    }

    /// [`Hooks::container_wheel`].
    pub fn container_wheel(c: &mut dyn Container, slot: Option<SlotId>, steps: f32) -> bool {
        all().iter().any(|h| h.container_wheel(c, slot, steps))
    }

    /// [`Hooks::container_key`].
    pub fn container_key(c: &mut dyn Container, slot: Option<SlotId>, key: &str) -> bool {
        all().iter().any(|h| h.container_key(c, slot, key))
    }

    /// [`Hooks::container_draw`].
    pub fn container_draw(c: &dyn Container, canvas: &mut dyn crate::hud::Canvas) {
        for hooks in all() {
            hooks.container_draw(c, canvas);
        }
    }

    /// [`Hooks::ended`]: every hook hears it, and the first plan is the one kept.
    pub fn ended(end: &Ended<'_>) -> Option<Reconnect> {
        let mut plan = None;
        for hooks in all() {
            let mine = hooks.ended(end);
            if plan.is_none() {
                plan = mine;
            }
        }
        plan
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
        fn ended(&self, end: &Ended<'_>) -> Option<Reconnect> {
            self.0.lock().unwrap().push(format!("ended {}", end.reason));
            None
        }
    }

    /// Plans an attempt numbered by its field.
    struct Planner(usize);
    impl Hooks for Planner {
        fn ended(&self, end: &Ended<'_>) -> Option<Reconnect> {
            Some(Reconnect {
                address: end.reason.to_owned(),
                name: String::new(),
                lan: false,
                at: Instant::now(),
                attempt: self.0,
                attempts: 5,
            })
        }
    }
    static FIRST: Planner = Planner(1);
    static SECOND: Planner = Planner(2);

    /// Implements nothing: the defaults do nothing.
    struct Silent;
    impl Hooks for Silent {}

    static RECORDER: Recorder = Recorder(StdMutex::new(Vec::new()));

    /// Takes its key on a container screen; notes what reached it.
    struct Keys(&'static str, StdMutex<Vec<String>>);
    impl Hooks for Keys {
        fn container_key(&self, _: &mut dyn Container, _: Option<SlotId>, key: &str) -> bool {
            self.1.lock().unwrap().push(format!("key {key}"));
            key == self.0
        }
        fn container_release(&self, _: &mut dyn Container) {
            self.1.lock().unwrap().push("release".into());
        }
    }
    static SORT: Keys = Keys("key.keyboard.r", StdMutex::new(Vec::new()));
    static ALSO_R: Keys = Keys("key.keyboard.r", StdMutex::new(Vec::new()));

    /// A screen with nothing in it.
    struct Empty;
    impl Container for Empty {
        fn menu(&self) -> &str {
            "inventory"
        }
        fn slots(&self) -> Vec<containers::Slot> {
            Vec::new()
        }
        fn stack(&mut self, _: SlotId) -> Option<containers::Stack> {
            None
        }
        fn carried(&self) -> Option<containers::Stack> {
            None
        }
        fn takes_one(&mut self, _: SlotId, _: &containers::Stack) -> bool {
            false
        }
        fn landing(&mut self, _: SlotId) -> Vec<SlotId> {
            Vec::new()
        }
        fn click(&mut self, _: SlotId, _: containers::Click) -> bool {
            false
        }
        fn spread(&self) -> usize {
            0
        }
        fn cursor(&self) -> (f32, f32) {
            (0.0, 0.0)
        }
        fn shift(&self) -> bool {
            false
        }
        fn order(&mut self, _: SlotId) -> (u32, String) {
            (u32::MAX, String::new())
        }
    }

    #[test]
    fn registered_hooks_hear_the_game_and_late_ones_are_refused() {
        register(&RECORDER).unwrap();
        register(&Silent).unwrap();
        register(&FIRST).unwrap();
        register(&SECOND).unwrap();
        register(&SORT).unwrap();
        register(&ALSO_R).unwrap();
        engine::started(&Started::new("0.2.31"));
        // Changes only: the same state every frame is said once.
        for open in [false, true, true, true, false, false] {
            engine::published(open);
        }
        // Every hook hears the end; the first plan registered is the one kept.
        let plan = engine::ended(&Ended::new("lost", None, false)).unwrap();
        assert_eq!(plan.attempt, 1);
        // A container screen's key goes to the first hook that takes it, its release to all.
        assert!(engine::containers());
        assert!(!engine::container_key(&mut Empty, None, "key.keyboard.e"));
        assert!(engine::container_key(&mut Empty, None, "key.keyboard.r"));
        engine::container_release(&mut Empty);
        assert_eq!(
            *SORT.1.lock().unwrap(),
            ["key key.keyboard.e", "key key.keyboard.r", "release"]
        );
        assert_eq!(
            *ALSO_R.1.lock().unwrap(),
            ["key key.keyboard.e", "release"],
            "taken before it"
        );
        engine::shutdown();
        assert_eq!(
            *RECORDER.0.lock().unwrap(),
            [
                "started 0.2.31",
                "published true",
                "published false",
                "ended lost"
            ]
        );
        assert_eq!(register(&Silent), Err(TooLate));
    }
}
