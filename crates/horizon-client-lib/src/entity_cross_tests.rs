//! Tests of entity code that need the library above `horizon-entities`.
use crate::assets::Assets;
use crate::motion::{MotionResources, PlayerMotion};
use horizon_core::geometry::Vec3;
use horizon_world::{
    attribute_map::ValueKind,
    attributes::{Modifier, Operation},
};
use std::path::Path;

#[test]
fn attribute_bases_survive_a_save_and_reload() {
    // #4425: LivingEntity.addAdditionalSaveData writes AttributeMap.pack
    // and readAdditionalSaveData applies it (before the health).
    let root = std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client")
                .join(horizon_core::VANILLA_VERSION)
        });
    if !root.join("movement-properties.nbt").exists() {
        eprintln!("skipped attribute save: prepared resources unavailable");
        return;
    }
    let assets = Assets::load(&root).unwrap();
    let resources = MotionResources::load(&root, &assets).unwrap();
    let mut p = PlayerMotion::new(&resources, Vec3::ZERO).unwrap();
    let set_base = |p: &mut PlayerMotion, name: &str, v: f64| {
        let key = p.attributes.key(name).unwrap();
        let h = p.attributes.get_instance(Some(key)).unwrap().unwrap();
        p.attributes
            .edit(&h, |a, o| a.set_base_value(v, o))
            .unwrap();
    };
    set_base(&mut p, "minecraft:scale", 2.0);
    set_base(&mut p, "minecraft:max_health", 30.0);
    let key = p.attributes.key("minecraft:max_health").unwrap();
    let h = p.attributes.get_instance(Some(key)).unwrap().unwrap();
    let bonus = Modifier::new("test:bonus", 0.5, Operation::AddMultipliedBase).unwrap();
    p.attributes
        .edit(&h, |a, o| a.add_permanent(bonus, o))
        .unwrap();
    let mut level = crate::save::Level {
        health: 40.0,
        food: 20,
        saturation: 5.0,
        player_extra: Some(horizon_world::nbt::Tag::compound([(
            "attributes",
            p.saved_attributes(),
        )])),
        ..Default::default()
    };
    // Through the player compound as the file holds it.
    let tag = level.player_tag();
    level = crate::save::Level::default();
    level.read_player(&tag);
    level.player_extra = Some(tag);
    let mut back = PlayerMotion::new(&resources, Vec3::ZERO).unwrap();
    let mut pending = true;
    back.load_saved_state_once(&mut pending, &level.saved_player_state())
        .unwrap();
    let base = |p: &mut PlayerMotion, name: &str| {
        let key = p.attributes.key(name).unwrap();
        p.attributes.value(Some(key), ValueKind::Base).unwrap()
    };
    assert_eq!(base(&mut back, "minecraft:scale"), 2.0);
    assert_eq!(base(&mut back, "minecraft:max_health"), 30.0);
    assert_eq!(back.attribute("minecraft:max_health").unwrap(), 45.0);
    assert_eq!(back.health.health(), 40.0);
}

/// A mod's thrown item takes its template's tracking settings, not the builder defaults.
#[test]
fn mod_thrown_types_take_their_templates_tracking_settings() {
    crate::projectiles::register_test_types();
    let settings = crate::lan::tracking::settings;
    assert_eq!(settings("ruby:bolt"), settings("minecraft:snowball"));
    assert_eq!(settings("ruby:late"), (5, 3, true));
}
