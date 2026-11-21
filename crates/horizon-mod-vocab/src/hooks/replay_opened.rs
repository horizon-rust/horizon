//! The `replay_opened` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A replay is open and its first frame is set up: `recording` is its id (as
    /// `replay_open` takes it; empty for a recording no id names), `duration_ms` its length and
    /// `standalone` whether it owns its session. Heard for every opening, whether a mod's
    /// `replay_open`, the replays list or `/studio replay open` asked for it, and only once the
    /// recording has been read: one that can't be played opens nothing and is not heard of
    /// here (a mod's request hears `replay_open_failed`). `host.replay_state()` reads the
    /// replay from here on. A replay opened over another one is heard after `replay_closed`
    /// of that one.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon's replays; NeoForge has no replay playback)"
    )]
    fn replay_opened();
}
