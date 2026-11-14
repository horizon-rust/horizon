//! `LevelData.RespawnData` at a respawn: the world spawn keeps its angle and
//! its level (`findRespawnDimension`), and `Level.getWorldBorderAdjustedRespawnData`
//! pulls it inside the world border.
use crate::game::{Dim, Game, lan::LanHost, lan::LanSettings};
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig},
};
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const CONN: u32 = 9;

fn light(game: &mut Game<'_>, around: i32) {
    for x in around - 2..=around + 2 {
        for z in around - 2..=around + 2 {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, z));
        }
    }
}

/// A server with one dead guest in the Overworld, without a point of its own.
fn server() -> (Game<'static>, LanHost, mpsc::Receiver<Out>) {
    let mut game = crate::game::native_machines::tests::server_game();
    // The world spawn's level is opened for a respawn into it.
    let scratch = std::env::temp_dir().join(format!("horizon-respawn-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    game.world_root = scratch;
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
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: CONN,
            uuid: 99,
            name: "RespawnData".into(),
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
        [0.5, 100.0, 0.5],
        2,
    );
    guest.dimension = "minecraft:overworld".into();
    guest.placed = true;
    guest.loaded = true;
    guest.load_timeout = 0;
    guest.health = 0.0;
    guest.dead = true;
    game.lan.as_mut().unwrap().hub.guests.push(guest);
    let host = game.lan.take().unwrap();
    (game, host, rx)
}

/// `TeleportTransition.createDefault`: the respawn data's yaw and pitch.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_respawn_at_the_world_spawn_faces_the_saved_angle() {
    let (mut game, mut host, _rx) = server();
    game.level.world_spawn = Some([0, 64, 0]);
    game.level.world_spawn_angle = [180.0, 20.0];
    light(&mut game, 0);
    game.guest_respawn(&mut host, CONN);
    let guest = &host.hub.guests[0];
    assert_eq!([guest.yaw, guest.pitch], [180.0, 20.0], "faces it at once");
    game.tick_server_travel(&mut host);
    let guest = &host.hub.guests[0];
    assert_eq!([guest.yaw, guest.pitch], [180.0, 20.0], "and on arrival");
}

/// `findRespawnDimension`: the world spawn's own level.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_world_spawn_set_in_the_nether_respawns_the_guest_there() {
    let (mut game, mut host, _rx) = server();
    game.level.world_spawn = None;
    game.as_console(|game| {
        assert!(game.run_command("execute in minecraft:the_nether run setworldspawn 0 64 0"));
    });
    game.guest_respawn(&mut host, CONN);
    assert_eq!(host.hub.guests[0].dimension, "minecraft:the_nether");
    let trip = game.server_trips.iter().find(|t| t.is(CONN));
    assert_eq!(
        trip.map(|t| t.dim),
        Some(Dim::Nether),
        "the trip follows it"
    );
}

/// `Level.getWorldBorderAdjustedRespawnData`: a spawn outside the border
/// respawns at the border's centre, not at its own column.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_world_spawn_outside_the_border_respawns_inside_it() {
    let (mut game, mut host, _rx) = server();
    game.level.world_spawn = Some([0, 64, 0]);
    game.border.set_center([500.0, 500.0]);
    game.border.set_size(10.0);
    light(&mut game, 500);
    game.guest_respawn(&mut host, CONN);
    game.tick_server_travel(&mut host);
    let at = host.hub.guests[0].position;
    assert!(game.border.contains(at[0], at[2]), "inside: {at:?}");
    assert_eq!((at[0], at[2]), (500.5, 500.5), "the border's centre column");
}
