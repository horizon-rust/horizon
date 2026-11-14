//! The synched dash flags of the mounts that have one: `Camel.DASH` and
//! `LAST_POSE_CHANGE_TICK`, and `AbstractNautilus.DASH`, written by `lan.rs`'s
//! `mob_meta`.
use horizon_client::mobs::{Mob, camel, nautilus};
use horizon_protocol::entity_data::{Value, serializer};

/// The entries after `AbstractHorse`'s flags: `Camel.DASH` (19) and
/// `LAST_POSE_CHANGE_TICK` (20), or `AbstractNautilus.DASH` (20, after
/// `TamableAnimal`'s flags and owner), which starts its clients' 40-tick dash
/// clock (`onSyncedDataUpdated`) and greys their jump bar (`getJumpCooldown`).
pub(super) fn write(
    m: &Mob,
    game_time: i64,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
    if camel::is_camel(m.species()) {
        if m.extra.flag2 {
            meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
        }
        defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
        let tick = camel::synced_pose_tick(m, game_time);
        if tick != 0 {
            meta.push((20, serializer::LONG, Value::Long(tick)));
        }
        defaults.push((20, serializer::LONG, Value::Long(0)));
    }
    if nautilus::is_nautilus(m.species()) {
        if m.extra.flag2 {
            meta.push((20, serializer::BOOLEAN, Value::Bool(true)));
        }
        defaults.push((20, serializer::BOOLEAN, Value::Bool(false)));
    }
}

#[cfg(test)]
mod tests {
    use super::super::mob_meta;
    use horizon_client::mobs::Mob;
    use horizon_protocol::entity_data::Value;

    /// `AbstractNautilus.DASH` (entry 20, after `TamableAnimal`'s flags and
    /// owner): the flag a client's 40-tick dash clock and jump bar follow goes
    /// out while the nautilus dashes, and its default is listed so the end of
    /// the dash is sent too.
    #[test]
    fn a_dashing_nautilus_sends_its_dash_flag() {
        use horizon_protocol::entity_data::serializer;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        let Some(types) = horizon_client::mobs::MobTypes::load(&root).ok() else {
            return;
        };
        for kind in ["minecraft:nautilus", "minecraft:zombie_nautilus"] {
            let Some(t) = types.types.get(kind) else {
                return;
            };
            let at = horizon_core::geometry::Vec3::new(0.0, 64.0, 0.0);
            let mut m = Mob::new(1, std::sync::Arc::new(t.clone()), at, 0.0);
            let meta_of = |m: &Mob| {
                let (mut meta, mut defaults) = (Vec::new(), Vec::new());
                mob_meta(
                    m,
                    &horizon_client::lan::data::SyncData::empty(),
                    None,
                    0,
                    &mut meta,
                    &mut defaults,
                );
                (meta, defaults)
            };
            let (meta, defaults) = meta_of(&m);
            assert!(
                defaults.contains(&(20, serializer::BOOLEAN, Value::Bool(false))),
                "{kind}: {defaults:?}"
            );
            assert!(meta.iter().all(|e| e.0 != 20), "{kind}: {meta:?}");
            m.extra.flag2 = true;
            let (meta, _) = meta_of(&m);
            assert!(
                meta.contains(&(20, serializer::BOOLEAN, Value::Bool(true))),
                "{kind}: {meta:?}"
            );
        }
    }
}
