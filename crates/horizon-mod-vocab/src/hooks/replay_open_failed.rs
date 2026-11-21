//! The `replay_open_failed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A `replay_open` the game accepted opened nothing: `recording` is the id it was asked
    /// for, `error` why. `Unreadable` when the recording turned out damaged or of another
    /// version once read; `WorldBusy` when a world was joined, left or opened before the replay
    /// was ready, or another world entry took the replay's place while it waited; `Unavailable`
    /// when the game began closing, the replay's session couldn't start, or the person
    /// cancelled the wait for it (Escape while the world loads). Heard once for each accepted
    /// request that doesn't reach `replay_opened`, except one replaced by a later `replay_open`
    /// before it was ready. A single-player world already left for the replay stays left;
    /// anything else stays as it was.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon's replays; NeoForge has no replay playback)"
    )]
    fn replay_open_failed();
}
