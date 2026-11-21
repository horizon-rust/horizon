//! The `break_block_player` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0409

pub trait Hooks {
    /// `break_block` with the breaking player: `player` is the guest's uuid
    /// (`BreakBlockEvent.getPlayer`), `None` when the game's own player breaks the block. Heard
    /// right after `break_block`, from the same place, with the same `canceled` and
    /// `exp_to_drop`. A separate hook, because `break_block` keeps its published signature
    /// (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "BreakBlockEvent"
    )]
    fn break_block_player();
}
