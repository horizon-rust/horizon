//! The `player_teleport_end_gateway_to` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0405

pub trait Hooks {
    /// A server player is about to travel through an End gateway at `gateway`; `to` is where they
    /// would land: the exit (block centre), or the search start when no exit is known yet. Heard
    /// right after `player_teleport_end_gateway`, from the same place. Set `cancel` to leave them
    /// at the entrance; the gateway is not consumed and its cooldown does not start. A separate
    /// hook, because `player_teleport_end_gateway` keeps its published signature (#19020).
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn player_teleport_end_gateway_to();
}
