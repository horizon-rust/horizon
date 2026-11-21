//! The `hopper_pickup_item_detail` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0353

pub trait Hooks {
    /// A hopper at `pos` is about to pick up `count` of `item`, the dropped item entity `entity`,
    /// so a mod can find the drop it is about (a locked death drop, for one). Heard after
    /// `hopper_pickup_item`, from the same place. Set `cancel` to leave the dropped stack in the
    /// world. A separate hook, because `hopper_pickup_item` keeps its published signature.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no direct equivalent; Bukkit's InventoryPickupItemEvent)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_hopper_pickup_item_detail"
    )]
    fn hopper_pickup_item_detail();
}
