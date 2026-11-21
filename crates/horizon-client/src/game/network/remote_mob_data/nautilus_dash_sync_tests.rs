//! `AbstractNautilus.DASH` on a client of a server: `onSyncedDataUpdated`
//! starts the 40-tick dash clock its `tick` counts down.
use horizon_client::mobs::Mob;
use horizon_protocol::entity_data::{Value, serializer as s};
use horizon_protocol::registry::Builtin;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION)
}

fn apply(m: &mut Mob, values: &[(u8, i32, Value)]) {
    let builtin = Builtin::load(&root().join("registries.json")).unwrap_or_default();
    super::apply(m, values, &builtin, 99);
}

/// A mob as a client spawns it (None without the prepared resources).
fn mob(kind: &str) -> Option<Mob> {
    let types = horizon_client::mobs::MobTypes::load(&root()).ok()?;
    let t = std::sync::Arc::new(types.types.get(kind)?.clone());
    let mut m = Mob::new(1, t, horizon_core::geometry::Vec3::ZERO, 0.0);
    super::client_defaults(&mut m);
    Some(m)
}

/// AbstractNautilus.DASH: the synced flag starts the 40-tick dash clock
/// (`onSyncedDataUpdated`), which a client counts down and the jump bar
/// reads as `getJumpCooldown`.
#[test]
fn a_nautilus_takes_its_dash_from_the_synced_flag() {
    for kind in ["minecraft:nautilus", "minecraft:zombie_nautilus"] {
        let Some(mut m) = mob(kind) else {
            return;
        };
        m.age = 1;
        apply(&mut m, &[(20, s::BOOLEAN, Value::Bool(true))]);
        assert!(m.extra.flag2, "{kind}: DASH");
        assert_eq!(m.extra.anim_ticks, 40, "{kind}: the dash cooldown");
        horizon_client::mobs::nautilus::client_tick(&mut m, &mut Vec::new());
        assert_eq!(m.extra.anim_ticks, 39, "{kind}: the client counts it down");
    }
}
