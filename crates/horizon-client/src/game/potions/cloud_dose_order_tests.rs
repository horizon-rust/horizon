//! A cloud doses each mob at its own place in the level's join order
//! (`ServerLevel.tick`'s `entityTickList`, #12183). The expected values are
//! the official 26.3 server's, stepped one tick at a time (the issue's table):
//! a pig that joined before the cloud has counted its effects and hurt timer
//! down when the dose lands, one that joined after counts the new effect down
//! in the same tick.
use crate::game::*;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::chunk::Chunk;
use horizon_client::motion::PlayerMotion;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let (Some(root), Some(home)) = (
        std::env::var_os("HORIZON_TEST_RESOURCES"),
        std::env::var_os("HORIZON_HOME"),
    ) else {
        return;
    };
    let root = std::path::PathBuf::from(root);
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
    options.game_dir = Some(
        std::path::PathBuf::from(home).join(format!("h12183-{:?}", std::thread::current().id())),
    );
    options.player_name = Some("H12183".into());
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
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
    // Entities tick in a chunk with the chunks two around it in too.
    for (x, z) in (-2..=2).flat_map(|x| (-2..=2).map(move |z| (x, z))) {
        game.world
            .chunks
            .insert((x, z), Arc::new(Chunk::empty(x, z)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(x, z))
            .unwrap();
    }
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
    game.creative = true;
    test(&mut game);
}

const CLOUD: &str = "{Radius:1.5f,WaitTime:0,Duration:200,potion_contents:{custom_effects:[\
    {id:\"minecraft:speed\",duration:100},{id:\"minecraft:instant_damage\"}]}}";

/// Pig `n`: its Speed ticks left, hurt timer and health.
fn pig(game: &Game<'_>, n: i32) -> Option<(i32, i32, f32)> {
    let m = game
        .mobs
        .iter()
        .find(|m| game.persist.uuid_of(m.id) == Some([0, 12183, 0, n]))?;
    let speed = m.extra.effects.iter().find(|e| e.0 == "minecraft:speed");
    Some((speed.map_or(0, |e| e.1), m.hurt_time, m.health))
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
fn a_cloud_doses_each_mob_in_the_order_they_joined() {
    with_game(|game| {
        let summon_pig = |game: &mut Game<'_>, n: i32, (x, z): (f64, f64)| {
            let command = format!(
                "summon minecraft:pig {x} 100 {z} {{UUID:[I;0,12183,0,{n}],NoAI:1b,Silent:1b}}"
            );
            assert!(game.run_command(&command), "{command}");
        };
        let summon_cloud = |game: &mut Game<'_>, (x, z): (f64, f64)| {
            let command = format!("summon minecraft:area_effect_cloud {x} 100 {z} {CLOUD}");
            assert!(game.run_command(&command), "{command}");
        };
        // Pig 1, then its cloud.
        summon_pig(game, 1, (2.5, 8.5));
        summon_cloud(game, (2.5, 8.5));
        // A cloud, then pig 2.
        summon_cloud(game, (7.5, 8.5));
        summon_pig(game, 2, (7.5, 8.5));
        // Pig 3, a cloud, then pig 4: each pig on its own side of it.
        summon_pig(game, 3, (12.5, 8.5));
        summon_cloud(game, (12.5, 8.5));
        summon_pig(game, 4, (12.5, 8.5));
        // Pig 5, a cloud, then pig 6, with pig 6 first in the mob list, as a
        // mob put back out of join order would be: the walk is still by join.
        summon_pig(game, 5, (7.5, 3.5));
        summon_cloud(game, (7.5, 3.5));
        summon_pig(game, 6, (7.5, 3.5));
        let six = game
            .mobs
            .iter()
            .position(|m| game.persist.uuid_of(m.id) == Some([0, 12183, 0, 6]))
            .unwrap();
        let six = game.mobs.remove(six);
        game.mobs.insert(0, six);
        for _ in 0..5 {
            game.tick_world(true).unwrap();
        }
        let got: Vec<_> = (1..=6).map(|n| pig(game, n)).collect();
        // Before the cloud: Speed 100 and hurt timer 10; after it: 99 and 9.
        let want = [
            Some((100, 10, 7.0)),
            Some((99, 9, 7.0)),
            Some((100, 10, 7.0)),
            Some((99, 9, 7.0)),
            Some((100, 10, 7.0)),
            Some((99, 9, 7.0)),
        ];
        assert_eq!(got, want, "pigs 1-6 after the dose tick (the fifth)");
    });
}
