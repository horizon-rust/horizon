//! A closed shulker box and farmland, dirt path, soul sand and mud suffocate a
//! player standing in them (`Blocks`' explicit `isSuffocating` predicates, and
//! the shulker box's `NOT_CLOSED_SHULKER`); glass does not. Both the server's
//! `Entity.isInWall` and the client's `moveTowardsClosestSpace` read that
//! predicate and the collision shape, not a full render cube.

use super::native_machines::tests::server_game;
use horizon_client::block_entity_anim::{Shulker, ShulkerStatus};
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;

/// A player standing at (8.5, `y`, 8.5): at 64.0 the eye cell is (8, 65, 8),
/// the body cells (8, 64, 8) and (8, 65, 8).
fn standing_player(
    game: &mut super::Game<'_>,
    y: f64,
) -> ([f64; 3], horizon_client::pose::Dimensions) {
    let player = PlayerMotion::new(game.motion, Vec3::new(8.5, y, 8.5)).unwrap();
    let at = [player.position.x, player.position.y, player.position.z];
    let dimensions = player.dimensions;
    game.player = Some(player);
    (at, dimensions)
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_closed_shulker_box_and_the_short_suffocating_blocks_suffocate_a_player() {
    let mut game = server_game();
    let registry = game.pack.assets.registry;
    let (at, body) = standing_player(&mut game, 64.0);
    let mut wrong = Vec::new();
    // (block, the server's isInWall at the eyes, the client's nudge)
    let cases = [
        ("minecraft:shulker_box", true, true),
        ("minecraft:farmland", true, true),
        ("minecraft:dirt_path", true, true),
        ("minecraft:soul_sand", true, true),
        ("minecraft:mud", true, true),
        ("minecraft:stone", true, true),
        ("minecraft:glass", false, false),
    ];
    for (name, wall, nudged) in cases {
        let state = registry.default_state(name).unwrap();
        for y in [64, 65] {
            game.world.set(8, y, 8, state).unwrap();
        }
        let got_wall = game.in_wall(at, body);
        let got_nudge = game.closest_space_nudge().iter().any(Option::is_some);
        if (got_wall, got_nudge) != (wall, nudged) {
            wrong.push(format!(
                "{name}: isInWall {got_wall} (want {wall}), nudge {got_nudge} (want {nudged})"
            ));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");

    // The eye box is 1e-6 tall: an eye within 5e-7 above a cell boundary also
    // asks the cell below (`BlockPos.betweenClosedStream`), whose full cube the
    // eye box still reaches. Feet at 64.38 put the eye at exactly 66.0.
    let air = registry.default_state("minecraft:air").unwrap();
    game.world.set(8, 64, 8, air).unwrap();
    game.world
        .set(8, 65, 8, registry.default_state("minecraft:stone").unwrap())
        .unwrap();
    let (at, body) = standing_player(&mut game, 64.38);
    assert!(game.in_wall(at, body), "an eye at 66.0 asks the cell below");

    // A full cube touching the eye box at its edge does not intersect it.
    game.world.set(8, 65, 8, air).unwrap();
    game.world
        .set(9, 65, 8, registry.default_state("minecraft:stone").unwrap())
        .unwrap();
    let half = f64::from(body.width * 0.8) / 2.0;
    assert!(!game.in_wall([9.0 - half, 64.0, 8.5], body));
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_open_shulker_box_does_not_suffocate_a_player() {
    let mut game = server_game();
    let registry = game.pack.assets.registry;
    let state = registry.default_state("minecraft:shulker_box").unwrap();
    let (at, body) = standing_player(&mut game, 64.0);
    for y in [64, 65] {
        game.world.set(8, y, 8, state).unwrap();
        // `ShulkerBoxBlockEntity.isClosed` is false while the lid is open.
        game.world.shulkers.insert(
            [8, y, 8],
            Shulker {
                status: ShulkerStatus::Opened,
                ..Default::default()
            },
        );
    }
    assert!(!game.in_wall(at, body), "an open lid");
    assert!(
        game.closest_space_nudge().iter().all(Option::is_none),
        "an open lid"
    );
}
