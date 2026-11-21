//! The `living_damage_post_cancelled` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0355

pub trait Hooks {
    /// NeoForge's `LivingDamageEvent.Post` for a hit a `living_incoming_damage` listener canceled:
    /// entity `entity` of type `kind` was hit as `container` describes, and took no health. It
    /// fires at the cancel, once per canceled hit, with the container as the incoming check left
    /// it. `living_damage_post` hears only the hits that were not canceled, and keeps its
    /// published signature; this is the separate hook for the canceled ones. Armour and shields do
    /// not cancel. It only listens. It can't write the entity or the world's blocks: the level
    /// host gives the hit's random and level attachments.
    #[hook(
        side = "server",
        contract = "listen",
        host = "level",
        neoforge = "LivingDamageEvent.Post",
        target = "horizon_client::game::native_hooks::living_damage_post_cancelled::HitLevel<'_>::native_living_damage_post_cancelled"
    )]
    fn living_damage_post_cancelled();
}
