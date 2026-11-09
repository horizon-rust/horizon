use super::{CartKind, Minecart};
use crate::game::native_machines::tests::server_game;
use horizon_core::geometry::Vec3;

/// Ticks a moving TNT cart until it stops or blows up; true when it
/// blew up.
fn runs_into_the_wall(rails: bool, at: Vec3, velocity: Vec3) -> bool {
    let mut game = server_game();
    let registry = game.pack.assets.registry;
    let stone = registry.default_state("minecraft:stone").unwrap();
    let rail = registry.default_state("minecraft:rail").unwrap();
    let mut east_west = registry.state(rail).unwrap().clone();
    east_west
        .properties
        .insert("shape".into(), "east_west".into());
    let rail = registry.id(&east_west).unwrap();
    for x in 0..16 {
        for z in 0..16 {
            game.world.set(x, 63, z, stone).unwrap();
        }
    }
    for z in 0..16 {
        game.world.set(9, 64, z, stone).unwrap();
        game.world.set(9, 65, z, stone).unwrap();
    }
    if rails {
        for x in 1..9 {
            game.world.set(x, 64, 8, rail).unwrap();
        }
    }
    let mut cart = Minecart::new(1, CartKind::Tnt, at);
    cart.velocity = velocity;
    for _ in 0..40 {
        if !game.tick_cart(&mut cart, [0.0; 2], 0, &mut super::CartIndex::default()) {
            return true;
        }
    }
    false
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_tnt_cart_stopped_head_on_by_a_wall_does_not_blow_up() {
    // OldMinecartBehavior.moveAlongTrack: the move zeroes the blocked x
    // and the track's re-pointing scales what is left, so MinecartTNT
    // reads no speed at the wall.
    assert!(!runs_into_the_wall(
        true,
        Vec3::new(3.5, 64.0, 8.5),
        Vec3::new(0.4, 0.0, 0.0)
    ));
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_tnt_cart_off_its_rails_glancing_off_a_wall_blows_up() {
    // comeOffTrack keeps the delta the move resolved: the wall zeroes x,
    // z goes on at over a tenth of a block a tick.
    assert!(runs_into_the_wall(
        false,
        Vec3::new(8.3, 64.0, 4.5),
        Vec3::new(0.4, 0.0, 0.4)
    ));
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_cart_on_a_slope_presses_a_plate_at_its_temporary_position() {
    // OldMinecartBehavior.moveAlongTrack places the cart at the rail's top
    // height, moves it horizontally (Entity.move records the step) and settles
    // it on the slope; the tick's applyEffectsFromBlocks still sweeps the step.
    let mut game = server_game();
    let registry = game.pack.assets.registry;
    let stone = registry.default_state("minecraft:stone").unwrap();
    let plate = registry
        .default_state("minecraft:oak_pressure_plate")
        .unwrap();
    let rail = registry.default_state("minecraft:rail").unwrap();
    let mut slope = registry.state(rail).unwrap().clone();
    slope
        .properties
        .insert("shape".into(), "ascending_east".into());
    let rail = registry.id(&slope).unwrap();
    for x in 0..16 {
        for z in 0..16 {
            game.world.set(x, 63, z, stone).unwrap();
        }
    }
    game.world.set(5, 64, 8, rail).unwrap();
    game.world.set(6, 64, 8, stone).unwrap();
    game.world.set(6, 65, 8, plate).unwrap();
    let mut cart = Minecart::new(1, CartKind::Rideable, Vec3::new(5.5, 64.5, 8.5));
    cart.velocity = Vec3::new(0.4, 0.0, 0.0);
    game.carts.push(cart.clone());
    assert!(game.tick_cart(&mut cart, [0.0; 2], 0, &mut super::CartIndex::default()));
    let powered = registry
        .state(game.world.block(6, 65, 8))
        .unwrap()
        .properties
        .get("powered")
        .cloned();
    assert_eq!(
        powered.as_deref(),
        Some("true"),
        "{:?}",
        registry.state(game.world.block(6, 65, 8))
    );
}
