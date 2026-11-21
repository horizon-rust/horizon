//! The `replay_closed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The replay of `recording` closed, whatever closed it: a mod's `replay_close`, Escape,
    /// the pause menu's Disconnect, or another replay opening over it. Heard once, after
    /// the replay is gone (`host.replay_state()` is none, or already the next replay's
    /// only after its `replay_opened`) and before a standalone replay's world is left.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon's replays; NeoForge has no replay playback)"
    )]
    fn replay_closed();
}
