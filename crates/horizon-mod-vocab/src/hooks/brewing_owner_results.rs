//! The `brewing_owner_results` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A brewing stand at `pos` finished a brew: `results` are its three bottles as they are now,
    /// empty slots left out, and `player` (UUID) is the guest who last opened the stand, None when no
    /// guest did (a stand nobody opened here, or the host's own). Fires right after `brewing_complete`;
    /// listen only, and the player's state is not swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)",
        paper = "BrewingCompleteEvent (the Paper event has no player; the opener is Horizon's)"
    )]
    fn brewing_owner_results();
}
