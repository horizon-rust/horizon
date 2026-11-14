//! The two ends of a name tag: `NameTagItem.interactLivingEntity` shrinks the
//! stack it names with -- and `Player.interactOn` puts a creative player's
//! count straight back -- and `EntityAttachment.NAME_TAG` says where the tag
//! hangs, which `EntityTypes.SNIFFER` moves to 2.05 above the feet.
use super::super::*;
use super::name_tag_at;
use horizon_client::entities::ArmorStand;
use horizon_client::mobs::{Anim, Brain, Mob, MobType, Part, Species};
use horizon_core::geometry::Vec3;
use horizon_world::nbt::Tag;
use std::sync::Arc;

/// A mob of `species` with the box of its vanilla type, built without the
/// prepared resources a test machine may not have.
fn mob(id: u32, entity: &str, species: Species, width: f64, height: f64) -> Mob {
    let part = Part {
        name: "root".into(),
        pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        polygons: Vec::new(),
        children: Vec::new(),
    };
    let kind = MobType {
        id: entity.into(),
        species,
        width,
        height,
        eye_height: height * 0.6,
        category: "creature".into(),
        max_health: 10.0,
        speed: 0.2,
        flying_speed: 0.02,
        attack_damage: 1.0,
        attack_knockback: 0.0,
        follow_range: 16.0,
        step_height: 0.6,
        armor: 0.0,
        armor_toughness: 0.0,
        knockback_resistance: 0.0,
        jump_strength: 0.42,
        attributes: Default::default(),
        item_stack_limits: Default::default(),
        gravity: 0.08,
        safe_fall_distance: 3.0,
        fall_damage_multiplier: 1.0,
        water_efficiency: 0.0,
        water_slowdown: 0.8,
        air_speed: 0.02,
        baby_scale: 0.5,
        head_rot_speed: 10.0,
        max_head_x_rot: 40.0,
        max_head_y_rot: 75.0,
        ambient_interval: 80,
        flags: Default::default(),
        model: Arc::new(part),
        layers: Default::default(),
        texture: String::new(),
        brain: Brain::Passive,
        anim: Anim::Humanoid { zombie_arms: false },
        sounds: ["entity.cow.hurt", "entity.cow.hurt", "entity.cow.death"],
        step_sound: None,
        food: Default::default(),
        tempt: Default::default(),
    };
    Mob::new(id, Arc::new(kind), Vec3::new(0.0, 64.0, 0.0), 0.0)
}

/// `EntityTypes.SNIFFER`: `.sized(1.9F, 1.75F).nameTagOffset(2.05F)`. The tag
/// hangs 2.05 above the feet where every other type hangs it at its box
/// height, and `SubmitNodeCollector.submitNameTag` lifts both 0.5 more.
#[test]
fn a_sniffers_tag_hangs_from_its_name_tag_attachment() {
    let tag_height = |m: &Mob| name_tag_at(m, 1.0).y - m.position.y;
    let sniffer = mob(1, "minecraft:sniffer", Species::Sniffer, 1.9, 1.75);
    assert!(
        (tag_height(&sniffer) - 2.55).abs() < 1e-6,
        "2.05 + 0.5, got {}",
        tag_height(&sniffer)
    );
    // A cow attaches nothing of its own: `Fallback.AT_HEIGHT`, 1.4 + 0.5.
    let cow = mob(2, "minecraft:cow", Species::Cow, 0.9, 1.4);
    assert!(
        (tag_height(&cow) - 1.9).abs() < 1e-6,
        "got {}",
        tag_height(&cow)
    );
    // `EntityDimensions.scale`: a baby's attachment shrinks with its box.
    let mut baby = mob(3, "minecraft:sniffer", Species::Sniffer, 1.9, 1.75);
    baby.extra.age = -1;
    assert!(
        (tag_height(&baby) - 1.525).abs() < 1e-6,
        "2.05 * 0.5 + 0.5, got {}",
        tag_height(&baby)
    );
    // `Sniffer.DIGGING_DIMENSIONS` is built without the attachment, so a
    // digging sniffer's tag falls back to its lower box: 1.35 + 0.5.
    let mut digging = mob(4, "minecraft:sniffer", Species::Sniffer, 1.9, 1.75);
    digging.extra.variant = horizon_client::mobs::sniffer::DIGGING;
    assert!(
        (tag_height(&digging) - 1.85).abs() < 1e-6,
        "got {}",
        tag_height(&digging)
    );
}

fn game() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
        .unwrap();
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap(),
    );
    game.screen = Screen::Playing;
    game.selected = 0;
    game
}

/// `Player.interactOn`: `NameTagItem` shrinks the stack it named with, and a
/// creative player's count comes straight back -- one tag names as many stands
/// as they click.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_creative_player_keeps_the_tag_it_names_a_stand_with() {
    let mut game = game();
    game.armor_stands
        .push(ArmorStand::new(9, Vec3::new(0.5, 64.0, 0.5), 0.0));
    let mut tag = ItemStack::new("minecraft:name_tag", 2);
    tag.data_mut().custom_name = Some(Tag::compound([("text", Tag::String("Bob".into()))]));
    game.inventory[0] = Some(tag);
    let target = || {
        Some(InteractTarget {
            id: 9,
            location: [0.0, 0.0, 0.0],
        })
    };
    game.set_mode_id(1);
    assert!(game.interact_armor_stand(target()));
    assert_eq!(
        game.inventory[0].as_ref().map(|s| s.count),
        Some(2),
        "a creative player keeps the tag"
    );
    let named = game.armor_stands[0]
        .custom_name
        .as_ref()
        .map(horizon_client::text::Text::plain);
    assert_eq!(named.as_deref(), Some("Bob"));
    // Survival spends one.
    game.set_mode_id(0);
    assert!(game.interact_armor_stand(target()));
    assert_eq!(
        game.inventory[0].as_ref().map(|s| s.count),
        Some(1),
        "a survival player spends the tag"
    );
}
