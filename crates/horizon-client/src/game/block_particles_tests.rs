//! Terrain particles vanilla never shows: the barrier and the structure void
//! are the two blocks `Blocks` builds with `noTerrainParticles`
//! (`BlockState.shouldSpawnTerrainParticles`), so `addDestroyBlockEffect`
//! sprays nothing for them, and `addBreakingBlockEffects` skips the crumb of
//! every `INVISIBLE` render shape as well.
//!
//! They load the client's block models and sprites from the prepared resources,
//! so they are ignored: without `HORIZON_TEST_RESOURCES` they cannot run, and
//! `load_resources` installs the prepared language process-wide, which the
//! untranslated-key tests of this binary (`lan_screen`) would then read as English.
use super::*;
use horizon_client::particles::ParticleLevel;
use horizon_world::registry::BlockStateId;

/// A game on the prepared resources (`HORIZON_TEST_RESOURCES`). It loads the
/// block models and sprites a client has, since a block particle without its
/// sprite is dropped.
fn server() -> Game<'static> {
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
        false,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
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
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
    game
}

fn state(game: &Game<'_>, name: &str) -> BlockStateId {
    game.pack
        .assets
        .registry
        .default_state(name)
        .unwrap_or_else(|| panic!("no {name}"))
}

/// The particles the level events just queued, once a tick has drawn them.
fn drawn(game: &mut Game<'_>) -> usize {
    let level = ParticleLevel {
        world: &game.world,
        facts: game.res.facts(),
        collision: game.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    game.particles.tick(&level);
    game.particles.len()
}

fn heard(game: &Game<'_>, event: &str) -> u32 {
    game.audio.heard.get(event).copied().unwrap_or(0)
}

/// `Blocks.BARRIER` and `Blocks.STRUCTURE_VOID` call `.noTerrainParticles()`;
/// every other block keeps `Properties`' default of true.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn only_the_barrier_and_the_structure_void_spawn_no_terrain_particles() {
    let game = server();
    let facts = game.res.facts();
    for name in ["minecraft:barrier", "minecraft:structure_void"] {
        assert!(
            !facts.get(state(&game, name)).spawn_terrain_particles,
            "{name} spawns no terrain particles"
        );
    }
    for name in ["minecraft:stone", "minecraft:light"] {
        assert!(
            facts.get(state(&game, name)).spawn_terrain_particles,
            "{name} spawns terrain particles"
        );
    }
}

/// `LevelEventHandler` 2001: the break sound plays for every block, the burst
/// (`ClientLevel.addDestroyBlockEffect`) only where
/// `shouldSpawnTerrainParticles` holds — a stone cube leaves its 64 crumbs,
/// the two blocks leave none.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_2001_event_sprays_nothing_for_a_barrier_or_a_structure_void() {
    let mut game = server();
    let pos = [4, 64, 4];
    for (name, crumbs) in [
        ("minecraft:barrier", 0),
        ("minecraft:structure_void", 0),
        ("minecraft:stone", 64),
    ] {
        let block = state(&game, name);
        game.world.set(pos[0], pos[1], pos[2], block).unwrap();
        game.particles.clear();
        let sounds = heard(&game, "minecraft:block.stone.break");
        game.level_event_at(2001, pos, block.0 as i32);
        assert_eq!(drawn(&mut game), crumbs, "the {name} burst");
        assert_eq!(
            heard(&game, "minecraft:block.stone.break") - sounds,
            1,
            "the {name} break sound still plays"
        );
    }
}

/// `ClientLevel.addBreakingBlockEffects`: the hit sound always, the crumb only
/// for a visible render shape that spawns terrain particles. Both blocks are
/// `INVISIBLE`, so mining them shows nothing.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn mining_a_barrier_or_a_structure_void_shows_no_crumb() {
    let mut game = server();
    let pos = [4, 64, 4];
    for (name, crumbs) in [
        ("minecraft:barrier", 0),
        ("minecraft:structure_void", 0),
        ("minecraft:stone", 1),
    ] {
        let block = state(&game, name);
        game.world.set(pos[0], pos[1], pos[2], block).unwrap();
        game.particles.clear();
        let sounds = heard(&game, "minecraft:block.stone.hit");
        game.breaking_block_effects(pos, [0, 1, 0], true);
        assert_eq!(drawn(&mut game), crumbs, "the {name} crumb");
        assert_eq!(
            heard(&game, "minecraft:block.stone.hit") - sounds,
            1,
            "the {name} hit sound still plays"
        );
    }
}
