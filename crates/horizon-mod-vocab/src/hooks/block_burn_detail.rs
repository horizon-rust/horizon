//! The `block_burn_detail` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0402

pub trait Hooks {
    /// `block_burn` with the fire: `igniting` is the fire block that burns the block at `pos`
    /// (Bukkit's `BlockBurnEvent.getIgnitingBlock`), `None` when no fire names it. Heard right
    /// after `block_burn`, from the same place; set `cancel` to keep the block. A separate hook,
    /// because `block_burn` keeps its published signature (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's BlockBurnEvent)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_burn_detail"
    )]
    fn block_burn_detail();
}
