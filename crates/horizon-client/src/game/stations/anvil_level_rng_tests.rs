use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use std::sync::Arc;

/// This regression deliberately uses only the pre-fix API, so exactly the same
/// production call and assertion run against unchanged main and the fix.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn anvil_wear_leaves_the_level_random_stream_untouched() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
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
    options.headless = true;
    options.game_dir = Some(std::env::current_dir().unwrap());
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
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
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    let oracle =
        std::fs::read_to_string(std::env::var_os("HORIZON_ANVIL_RNG_ORACLE").unwrap()).unwrap();
    let native_level_next: i32 = oracle
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .parse()
        .unwrap();
    game.world.random = LegacyRandom::new(98765);
    assert_eq!(
        game.world.random.next_int(),
        native_level_next,
        "no-event seeded stream control"
    );
    println!("level RNG control no-event: {native_level_next}");
    // Native onTake only queues a level event. Even a creative anvil use must
    // not spend server-level RNG on the subsequent client sound pitch.
    for creative in [true, false] {
        for name in [
            "minecraft:stone",
            "minecraft:anvil",
            "minecraft:chipped_anvil",
            "minecraft:damaged_anvil",
        ] {
            let pos = [3, 64, 3];
            let mut chunk = Chunk::empty(0, 0);
            chunk.set(
                pos[0],
                pos[1],
                pos[2],
                game.pack.assets.registry.default_state(name).unwrap().0,
            );
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            game.world.random = LegacyRandom::new(98765);
            *game.particles.level_random() = LegacyRandom::new(54321);
            game.anvil_used(pos, creative);
            let prefix = format!("0 {} {name} 0 ", u8::from(creative));
            let expected: i32 = oracle
                .lines()
                .find(|line| line.starts_with(&prefix))
                .unwrap()
                .split_whitespace()
                .last()
                .unwrap()
                .parse()
                .unwrap();
            let actual = game.world.random.next_int();
            assert_eq!(
                actual, expected,
                "creative={creative}, block={name}: native level RNG must remain unchanged"
            );
            // Source-derived control, not a native client rendering run:
            // LevelEventHandler bytecode draws once for 1029 and 1030.
            let mut client = LegacyRandom::new(54321);
            client.next_float();
            assert_eq!(
                game.particles.level_random().next_int(),
                client.next_int(),
                "one client-level pitch draw"
            );
            println!("level RNG control creative={creative}, block={name}: {actual}");
        }
    }
}
