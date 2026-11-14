//! The End's exit portal: `ServerPlayer.findRespawnPositionAndUseSpawnBlock`
//! hands the transition the respawn point's angles, so a player out of the
//! End arrives facing the bed it slept in, level; and a respawn point whose
//! block is gone sends NO_RESPAWN_BLOCK_AVAILABLE, which the client says as
//! "You have no home bed or charged anchor".
use super::Trip;
use crate::game::{Dim, Game, lan::LanHost, lan::LanSettings};
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig},
};
use horizon_core::geometry::Vec3;
use horizon_core::position::ChunkPos;
use horizon_protocol::{ids, packets::configuration::ClientInformation};
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

/// A server in the Overworld with a stone floor at y 99 and one guest
/// standing on it: the level the End's exit arrives in.
fn server() -> (Game<'static>, LanHost, mpsc::Receiver<Out>) {
    let mut game = crate::game::native_machines::tests::server_game();
    light(&mut game, 0);
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command("forceload add 0 0"));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
    // A bed whose head is at (6, 100, 3), facing north.
    assert!(game.run_command("setblock 6 100 4 red_bed[facing=north,part=foot]"));
    assert!(game.run_command("setblock 6 100 3 red_bed[facing=north,part=head]"));
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
            uuid: 91,
            name: "Exit".into(),
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
        [8.5, 100.0, 8.5],
        2,
    );
    guest.dimension = "minecraft:overworld".into();
    guest.placed = true;
    guest.loaded = true;
    guest.load_timeout = 0;
    guest.health = 20.0;
    game.lan.as_mut().unwrap().hub.guests.push(guest);
    let host = game.lan.take().unwrap();
    (game, host, rx)
}

/// The trip an End portal out of the End starts (`start_server_trip` with no
/// plan): into the Overworld, whose respawn point is found on arrival.
fn exit_trip() -> Trip {
    Trip {
        conn: CONN,
        dim: Dim::Overworld,
        source: Dim::End,
        target: Vec3::new(0.5, 100.0, 0.5),
        end: true,
        plan: None,
        vehicle: None,
        gateway: None,
        respawn: None,
        // An End portal leaves the respawn block as it is.
        consume: false,
        world_spawn: false,
        finder: None,
        placed: false,
        hold: false,
        left: Some(Dim::End),
    }
}

/// The events of the GameEvent packets sent to the guest
/// (`ClientboundGameEventPacket`: the event's byte, then a float).
fn game_events(rx: &mpsc::Receiver<Out>) -> Vec<u8> {
    let mut events = Vec::new();
    while let Ok(out) = rx.try_recv() {
        let bodies = match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            _ => Vec::new(),
        };
        for body in bodies {
            let mut r = horizon_protocol::buf::Reader::new(&body);
            if r.varint().ok() == Some(ids::play::clientbound::GAME_EVENT)
                && let Ok(event) = r.u8()
            {
                events.push(event);
            }
        }
    }
    events
}

/// `RespawnPosAngle.of(standUpPosition, pos, 0.0F)`: a bed's or an anchor's
/// respawn point is level, and the player looks at the bed. The player's own
/// pitch (looking straight up) is not carried through the exit.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn the_end_exit_arrives_level_at_the_bed_it_slept_in() {
    let (mut game, mut host, rx) = server();
    // The bed's head, as `setRespawnPosition` keeps it (a bed sets 0 pitch).
    host.hub.guests[0].respawn_elsewhere =
        Some(("minecraft:overworld".into(), [6, 100, 3], 0.0, 0.0, false));
    host.hub.guests[0].pitch = 90.0;
    host.hub.guests[0].yaw = 12.0;
    game.server_trips.push(exit_trip());
    game.arrive_server_trips(&mut host);
    let guest = &host.hub.guests[0];
    assert_eq!(
        guest.position,
        [7.5, 100.0, 3.5],
        "standing east of the bed, its stand-up spot"
    );
    assert!(
        (guest.yaw - 90.0).abs() < 1.0e-4,
        "facing the bed, not the End's view: {}",
        guest.yaw
    );
    assert_eq!(
        guest.pitch, 0.0,
        "the bed's pitch, not the 90 the player looked up with"
    );
    assert!(
        !game_events(&rx).contains(&0),
        "the bed is still there: no NO_RESPAWN_BLOCK_AVAILABLE"
    );
}

/// `TeleportTransition.missingRespawnBlock`: the Overworld's respawn data
/// angles, and the player is told its spawn block is gone.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_gone_bed_at_the_end_exit_sends_no_respawn_block_available() {
    let (mut game, mut host, rx) = server();
    // The bed is broken: the point is still remembered, its block is not there.
    host.hub.guests[0].respawn_elsewhere =
        Some(("minecraft:overworld".into(), [6, 100, 3], 0.0, 0.0, false));
    let air = game
        .pack
        .assets
        .registry
        .default_state("minecraft:air")
        .unwrap();
    game.world.set(6, 100, 3, air).unwrap();
    game.world.set(6, 100, 4, air).unwrap();
    host.hub.guests[0].pitch = 90.0;
    host.hub.guests[0].yaw = 12.0;
    // The Overworld's respawn data, which createDefault and missingRespawnBlock use.
    game.level.world_spawn_angle = [30.0, -25.0];
    game.server_trips.push(exit_trip());
    game.arrive_server_trips(&mut host);
    let guest = &host.hub.guests[0];
    assert!(
        game_events(&rx).contains(&0),
        "NO_RESPAWN_BLOCK_AVAILABLE says the bed is gone"
    );
    assert_eq!(guest.yaw, 30.0, "the level's spawn yaw");
    assert_eq!(guest.pitch, -25.0, "the level's spawn pitch");
}

/// `TeleportTransition.createDefault`: no respawn point at all, so the shared
/// spawn with the level's respawn data angles, and no message (nothing is
/// missing).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_player_with_no_respawn_point_leaves_the_end_facing_the_spawn_angle() {
    let (mut game, mut host, rx) = server();
    host.hub.guests[0].pitch = 90.0;
    host.hub.guests[0].yaw = 12.0;
    game.level.world_spawn_angle = [30.0, -25.0];
    game.server_trips.push(exit_trip());
    game.arrive_server_trips(&mut host);
    let guest = &host.hub.guests[0];
    assert_eq!(guest.yaw, 30.0, "the level's spawn yaw");
    assert_eq!(guest.pitch, -25.0, "the level's spawn pitch");
    assert!(
        !game_events(&rx).contains(&0),
        "no respawn point is not a missing one: no NO_RESPAWN_BLOCK_AVAILABLE"
    );
}
