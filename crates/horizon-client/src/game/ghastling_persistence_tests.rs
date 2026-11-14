//! `DriedGhastBlock.spawnGhastling` creates the ghastling and never calls
//! `setPersistenceRequired`, unlike `FrogspawnBlock.hatchFrogspawn`, which
//! sets it on every tadpole. The hatched ghastling is therefore not
//! persistent, and its saved `PersistenceRequired` is `0b`.
use super::super::*;
use horizon_client::entity_nbt::mob_to_nbt;
use horizon_client::mobs::Species;
use horizon_world::level::LevelEvent;
use horizon_world::nbt::Tag;

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut g = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    g.options.dedicated = Some(Vec::new());
    g.world = World::new(g.res.facts().air, 1);
    test(&mut g);
}

/// The hatch events the block layer emits, run through the game's own
/// handlers: a dried ghast block tick, and a frogspawn tick.
fn hatch(g: &mut Game<'_>, kind: &'static str, block: &str) {
    let state = g
        .pack
        .assets
        .registry
        .default_state(block)
        .unwrap_or_else(|| panic!("{block} has a default state"));
    g.level_events.push(LevelEvent::BlockEvent {
        pos: BlockPos::new(0, 64, 0),
        state,
        kind,
    });
    g.process_level_events().unwrap();
}

/// `spawnGhastling`: `EntityType.create(level, BREEDING)` and `addFreshEntity`
/// with no `setPersistenceRequired`, so the ghastling is a plain mob.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_hatched_ghastling_is_not_persistent() {
    with_game(|g| {
        hatch(g, "spawn_ghastling", "minecraft:dried_ghast");
        let ghastlings: Vec<_> = g
            .mobs
            .iter()
            .filter(|m| m.species() == Species::HappyGhast)
            .collect();
        assert_eq!(ghastlings.len(), 1, "one ghastling from the hatch");
        let ghastling = ghastlings[0];
        assert!(
            !ghastling.persistent,
            "spawnGhastling never calls setPersistenceRequired"
        );
        let nbt = mob_to_nbt(ghastling, [0; 4], None, None, |_| None);
        assert_eq!(
            nbt.get("PersistenceRequired"),
            Some(&Tag::Byte(0)),
            "Mob.addAdditionalSaveData writes persistenceRequired"
        );
    });
}

/// `FrogspawnBlock.hatchFrogspawn` calls `setPersistenceRequired` on each
/// tadpole, so the same hatch path still marks them persistent.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn hatched_tadpoles_are_still_persistent() {
    with_game(|g| {
        hatch(g, "hatch", "minecraft:frogspawn");
        let tadpoles: Vec<_> = g
            .mobs
            .iter()
            .filter(|m| m.species() == Species::Tadpole)
            .collect();
        assert!(
            (2..=5).contains(&tadpoles.len()),
            "hatchFrogspawn spawns 2-5 tadpoles, got {}",
            tadpoles.len()
        );
        for tadpole in tadpoles {
            assert!(tadpole.persistent, "every tadpole is persistent");
            let nbt = mob_to_nbt(tadpole, [0; 4], None, None, |_| None);
            assert_eq!(nbt.get("PersistenceRequired"), Some(&Tag::Byte(1)));
        }
    });
}
