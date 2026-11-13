use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use horizon_core::{
    geometry::Vec3,
    random::{EntityRandom, LegacyRandom, LevelRandom, RandomSource},
};
use std::sync::Arc;

/// This regression deliberately uses only the pre-fix API, so exactly the same
/// production call and assertion run against unchanged main and the fix.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn anvil_wear_leaves_the_level_random_stream_untouched() {
    let mut game = crate::game::native_machines::tests::server_game();
    game.options.dedicated = None;
    game.options.headless = true;
    game.options.game_dir = Some(std::env::current_dir().unwrap());
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
    game.world.random = LevelRandom::for_level(LegacyRandom::new(98765));
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
            game.world.random = LevelRandom::for_level(LegacyRandom::new(98765));
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
    // Keep the neighbouring bottle-impact fix when composing the sound arms.
    // These are production event calls with source-derived stream controls,
    // not a native client rendering comparison.
    for id in [1029, 1030, 1053, 1054, 1042, 1043, 1044] {
        game.world.random = LevelRandom::for_level(LegacyRandom::new(98765));
        *game.particles.level_random() = LegacyRandom::new(54321);
        game.player.as_mut().unwrap().random = EntityRandom::own(LegacyRandom::new(12345));
        game.level_event_at(id, [3, 64, 3], 0);
        let mut world = LegacyRandom::new(98765);
        let mut client = LegacyRandom::new(54321);
        if matches!(id, 1029 | 1030 | 1053 | 1054) {
            client.next_float();
        } else {
            world.next_float();
        }
        assert_eq!(game.world.random.next_int(), world.next_int(), "event {id}");
        assert_eq!(
            game.particles.level_random().next_int(),
            client.next_int(),
            "event {id}"
        );
        assert_eq!(
            game.player.as_mut().unwrap().random.next_int(),
            LegacyRandom::new(12345).next_int(),
            "event {id}: sounds must not draw from the player"
        );
        println!("sound RNG composition control event={id}");
    }
}
