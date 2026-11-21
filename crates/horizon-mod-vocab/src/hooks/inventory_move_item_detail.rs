//! The `inventory_move_item_detail` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0354

pub trait Hooks {
    /// A hopper is about to move one item between containers, a composter's bone meal included.
    /// Heard after `inventory_move_item`, from the same place, with what that hook leaves out:
    /// `source_slot` is the slot the item leaves in the source container's numbering (a
    /// composter's bone meal is its slot 0; a capability's slot is the capability's), `None` for
    /// a source with no slot number, and `amount` is how many leave: a hopper moves one item per
    /// transfer. Set `cancel` to leave it in its source slot. A chest boat or minecart is reported
    /// at the block it is in. A separate hook, because `inventory_move_item` keeps its published
    /// signature.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        neoforge = "none (NeoForge has no such event; Bukkit's InventoryMoveItemEvent)",
        cancellable = true
    )]
    fn inventory_move_item_detail();
}
