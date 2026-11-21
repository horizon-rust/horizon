//! The `item_despawn_detail` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0352

pub trait Hooks {
    /// A dropped item entity reached its 6000-tick lifetime and is about to be removed (Bukkit's
    /// `ItemDespawnEvent`), heard after `item_despawn`, from the same place. Set `cancel` to keep
    /// the item for 5 more minutes, as Paper's cancel does; a positive `extra_life` from the
    /// `item_expire` hook already keeps it and this hook does not run then. A separate hook,
    /// because `item_despawn` only listens and keeps its published signature. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's ItemDespawnEvent)",
        cancellable = true
    )]
    fn item_despawn_detail();
}
