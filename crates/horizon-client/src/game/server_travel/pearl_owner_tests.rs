//! A pearl left in one level takes its owner from another (#16396).
use crate::game::{Game, lan::LanHost, lan::LanSettings};
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    mobs::Who,
    projectiles::{Hit, Projectile, ProjectileKind},
};
use horizon_core::geometry::Vec3;
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const CONN: u32 = 9;

/// A server in the Overworld with one guest, alive, standing in the Nether.
fn server_with_a_guest_in_the_nether() -> (Game<'static>, LanHost) {
    let mut game = crate::game::native_machines::tests::server_game();
    for x in -2..=2 {
        for z in -2..=2 {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, z));
        }
    }
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::loopback(0, String::new(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
        },
    )
    .unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: CONN,
            uuid: 99,
            name: "Stasis".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:9".parse().unwrap(),
            expected: false,
            configured: Default::default(),
        },
        CONN as i32,
        [40.5, 70.0, -12.5],
        2,
    );
    guest.dimension = "minecraft:the_nether".into();
    guest.placed = true;
    guest.loaded = true;
    guest.load_timeout = 0;
    guest.health = 20.0;
    game.lan.as_mut().unwrap().hub.guests.push(guest);
    let host = game.lan.take().unwrap();
    (game, host)
}

/// `ThrownEnderpearl.onHit` teleports its owner with a `TeleportTransition`
/// into the pearl's own level: a player in the Nether whose pearl lands in the
/// Overworld comes back to the Overworld, where the pearl was, not to those
/// coordinates in the Nether.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_pearl_in_the_overworld_takes_its_owner_out_of_the_nether() {
    let (mut game, host) = server_with_a_guest_in_the_nether();
    assert_eq!(game.dim, crate::game::Dim::Overworld);
    let landed = Vec3::new(8.5, 64.0, 8.5);
    let mut pearl = Projectile::new(ProjectileKind::EnderPearl, landed, Some(Who::Player));
    pearl.guest = Some(CONN);
    pearl.previous = landed;
    game.lan = Some(host);
    game.guest_projectile_hit(
        CONN,
        &pearl,
        Hit::Block {
            pos: [8, 63, 8],
            at: Vec3::new(8.5, 64.0, 8.5),
            face: [0, 1, 0],
        },
    )
    .unwrap();
    let mut host = game.lan.take().unwrap();
    game.run_level_teleports(&mut host);
    let guest = &host.hub.guests[0];
    assert_eq!(
        (guest.dimension.as_str(), guest.position),
        ("minecraft:overworld", [8.5, 64.0, 8.5]),
        "the owner goes to the pearl's level"
    );
}
