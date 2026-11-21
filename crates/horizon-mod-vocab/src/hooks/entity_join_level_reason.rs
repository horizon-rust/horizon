//! The `entity_join_level_reason` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0410

pub trait Hooks {
    /// `entity_join_level` with the spawn reason, heard right after it, from the same place, with
    /// the same `canceled`. `reason` is the `EntitySpawnReason` a mob was spawned with (natural,
    /// spawner, command, conversion, ...), `None` for every other kind and for a mob coming back
    /// from disk: NeoForge's `EntityJoinLevelEvent` has no reason, so Horizon adds it here from the
    /// spawn site that made the mob. `entity` is its ID and `kind` its type, `loaded_from_disk` is
    /// true for an entity coming back with its chunk. Set `canceled` to keep it out. A separate
    /// hook, because `entity_join_level` keeps its published signature (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "EntityJoinLevelEvent"
    )]
    fn entity_join_level_reason(
        dimension: &str,
        entity: u32,
        kind: &str,
        pos: [f64; 3],
        loaded_from_disk: bool,
        reason: Option<SpawnReason>,
        canceled: &mut bool,
    );
}
