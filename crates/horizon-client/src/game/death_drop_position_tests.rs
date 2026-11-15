//! `LivingEntity.dropAllDeathLoot`: the items `Entity.spawnAtLocation` makes
//! (`Entity.java:2283-2292`, `new ItemEntity(level, getX() + offset.x, ...)`
//! with the zero offset) and the orbs `LivingEntity.dropExperience` awards
//! (`LivingEntity.java:1535`, `ExperienceOrb.award(level, this.position(), ..)`)
//! start at the mob's own position, the bottom of its box, with no vertical
//! bias.
use super::*;
use horizon_client::chunk::Chunk;
use horizon_core::geometry::Vec3;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

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
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
    // The player whose hit kills the cow, so `lastHurtByPlayerMemoryTime` is
    // above zero and the loot table and experience both run.
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(
            game.motion,
            horizon_core::geometry::Vec3::new(14.5, 100.0, 14.5),
        )
        .unwrap(),
    );
    game
}

/// A player's kill of a cow standing on the floor: its beef and leather, then
/// its experience, leave from the cow's feet, not from half a block above.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_dead_mobs_drops_and_orbs_start_at_its_feet() {
    let mut game = game();
    // The stone's top face at y = 100 is where the summoned cow's box starts.
    let feet = Vec3::new(4.5, 100.0, 4.5);
    assert!(game.run_command("summon minecraft:cow 4.5 100 4.5 {Health:1f,NoAI:1b}"));
    assert!(
        game.run_command(
            "damage @e[type=minecraft:cow,limit=1] 1000 minecraft:player_attack by @p"
        )
    );
    // `LivingEntity.die` runs in the entity phase, when the tick finds the
    // mob at zero health; the fresh items and orbs joined after this tick's
    // `begin_ordered_ticks`, so they keep the position they spawned at.
    game.tick_creature_entities().unwrap();
    assert!(
        game.mobs.iter().all(|m| !m.alive()),
        "the cow died: {:?}",
        game.mobs
            .iter()
            .map(|m| (&*m.kind.id, m.health, m.position))
            .collect::<Vec<_>>()
    );
    assert!(
        !game.dropped.is_empty(),
        "a player's kill drops the cow's loot table"
    );
    assert!(!game.orbs.is_empty(), "and awards its experience");
    for item in &game.dropped {
        assert_eq!(
            item.position, feet,
            "`spawnAtLocation` puts the drop at the mob, with no offset"
        );
    }
    for orb in &game.orbs {
        assert_eq!(
            orb.position, feet,
            "`ExperienceOrb.award` puts the orb at the mob's position"
        );
    }
}
