//! The `ender_pearl_teleport_damage` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0407

pub trait Hooks {
    /// A server player's or mob's ender pearl is about to teleport its `owner`, heard right after
    /// `ender_pearl_teleport`, from the same place, with the fall `damage` as the one that hook
    /// left it. `damage` is `None` to skip the hurt call altogether: no damage, hurt sound,
    /// animation or invulnerability check; the teleport still happens. A mob owner takes none, and
    /// its `damage` starts at `Some(0.0)`. `cancel` is the cancel of `ender_pearl_teleport`: set
    /// it to spend the pearl without a teleport. A separate hook, because `ender_pearl_teleport`
    /// keeps its published signature (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "EntityTeleportEvent.EnderPearl",
        cancellable = true
    )]
    fn ender_pearl_teleport_damage(
        owner: EntityRef,
        pearl: u32,
        damage: &mut Option<f32>,
        cancel: &mut bool,
    );
}
