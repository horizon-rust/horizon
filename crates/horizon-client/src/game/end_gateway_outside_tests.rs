//! `EndGatewayBlock.entityInside` has no dimension test: a gateway outside
//! the End still takes an entity in and starts its cooldown, but
//! `TheEndGatewayBlockEntity.getPortalPosition` only builds an exit in the
//! End. A move is swept, so a fast fall cannot pass through between ticks.

use crate::game::Dim;
use crate::game::end::GatewayJob;
use crate::game::native_machines::tests::server_game;
use horizon_client::containers::BlockEntity;
use horizon_client::entities::ItemEntity;
use horizon_client::items::ItemStack;
use horizon_core::geometry::Vec3;

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_item_in_a_gateway_outside_the_end_still_enters_it() {
    let mut game = server_game();
    game.dim = Dim::Overworld;
    game.settings.simulation_distance = 4;
    game.settings.render_distance = 12;
    let gateway = game
        .pack
        .assets
        .registry
        .default_state("minecraft:end_gateway")
        .unwrap();
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 65.0, 8.5)).unwrap(),
    );
    game.load_ticking_neighbourhood(0, 0);
    game.world.set(8, 64, 8, gateway).unwrap();
    game.world
        .block_entities
        .insert((8, 64, 8), BlockEntity::EndGateway(Default::default()));
    game.dropped.push(ItemEntity::new(
        1,
        ItemStack::new("minecraft:stone", 1),
        Vec3::new(8.5, 64.2, 8.5),
        Vec3::ZERO,
        0.0,
    ));
    game.update_chunk_tickets();
    assert!(game.entity_ticking_area()(8, 8));

    game.find_end_gateway_portals();

    assert!(
        !game.gateway_available([8, 64, 8]),
        "the item went into the gateway outside the End and started its cooldown"
    );
    // Entity.handlePortal next tick sets the item's cooldown without an exit.
    game.handle_end_gateway_portals();
    assert!(
        game.end.arrivals.is_empty(),
        "no destination outside the End"
    );
    assert!(
        game.end.gateways.get(&1).is_some_and(|t| t.cooldown() > 0),
        "the item's portal cooldown is set even without a destination"
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_data_less_gateway_in_the_outer_end_searches_outward() {
    let mut game = server_game();
    game.dim = Dim::End;
    let (job, target) = game.gateway_job([702, 100, 0]);
    assert_eq!(
        job,
        GatewayJob::Find {
            gateway: [702, 100, 0],
            exact: false
        },
        "a gateway without an exit searches, however far out it is"
    );
    assert!((target.x - 1024.0).abs() < 1e-9, "{target:?}");
    assert_eq!((target.y, target.z), (75.0, 0.0));
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_fast_fall_through_a_gateway_outside_the_end_is_caught() {
    let (mut game, mut host, scratch) = super::credits_respawn_tests::server_with_a_nether_anchor();
    let gateway = game
        .pack
        .assets
        .registry
        .default_state("minecraft:end_gateway")
        .unwrap();
    game.world.set(2, 64, 2, gateway).unwrap();
    game.world
        .block_entities
        .insert((2, 64, 2), BlockEntity::EndGateway(Default::default()));
    // A fall of 3.9 blocks in one tick, from above the gateway to below it.
    let g = &mut host.hub.guests[0];
    g.previous_position = [2.5, 66.0, 2.5];
    g.first_good = [2.5, 66.0, 2.5];
    g.position = [2.5, 62.1, 2.5];

    game.handle_server_portal(&mut host, 2);

    assert!(
        !game.gateway_available([2, 64, 2]),
        "the fall swept through the gateway"
    );
    assert_eq!(
        game.portals.player.cooldown, 10,
        "the gateway started the player's portal cooldown"
    );
    assert!(
        game.server_trips.is_empty(),
        "a gateway without an exit outside the End has no destination"
    );
    // Standing still below it, the next ticks sweep no move: the fall is not
    // swept again when the gateway's own cooldown runs out.
    for _ in 0..40 {
        game.tick_end_gateway((2, 64, 2));
        let g = &mut host.hub.guests[0];
        g.first_good = g.position;
        game.handle_server_portal(&mut host, 2);
    }
    assert_eq!(
        game.portals.player.cooldown, 0,
        "the fall through the gateway is swept once, not every tick"
    );
    drop(host);
    drop(game);
    std::fs::remove_dir_all(scratch).unwrap();
}
