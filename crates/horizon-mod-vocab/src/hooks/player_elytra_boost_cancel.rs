//! The `player_elytra_boost_cancel` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0403

pub trait Hooks {
    /// A server player is about to use a firework rocket to boost while gliding, before the rocket
    /// is launched or consumed (Paper `PlayerElytraBoostEvent`). Set `cancel` and nothing is
    /// launched: the rocket stays in the player's hand and no stat is awarded;
    /// `player_elytra_boost` is then not heard. A separate hook, because `player_elytra_boost`
    /// only listens, after the launch, and keeps its published signature (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_elytra_boost_cancel();
}
