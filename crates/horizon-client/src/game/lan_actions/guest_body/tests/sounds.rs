//! `Player.playSound` over a guest's footsteps, swim and splash: the server
//! sends them to every player around but the one that made them.
use crate::game::Game;
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig},
};
use horizon_core::position::ChunkPos;
use horizon_protocol::packets::{
    Empty,
    configuration::ClientInformation,
    play,
    play::{Clientbound as C, Serverbound as S},
};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const NAME: &str = "GuestSoundTest";
/// The stone floor, and the first column of the pool above it.
const FLOOR: i32 = 99;
const WATER: i32 = 13;
const Z: f64 = 12.5;
/// `SoundSource.PLAYERS`, where `Player.getSoundSource` puts everything a
/// player's body makes, a footstep on stone included.
const PLAYERS: i32 = 7;

/// The window's own connection (`lan::conn::MEMORY_ADDRESS`).
const LOCAL: std::net::SocketAddr = horizon_client::lan::conn::MEMORY_ADDRESS;

/// A guest whose client's packets the test can read.
fn watched_guest(
    entity_id: i32,
    at: [f64; 3],
    address: std::net::SocketAddr,
) -> (Guest, mpsc::Receiver<Out>) {
    let (sender, rx) = mpsc::channel();
    let guest = Guest::new(
        Joined {
            conn: entity_id as u32,
            uuid: entity_id as u128,
            name: format!("{NAME}{entity_id}"),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(sender, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address,
            expected: false,
            configured: Default::default(),
        },
        entity_id,
        at,
        2,
    );
    (guest, rx)
}

/// A server with two guests on the floor: the walker at 10.5, the one that
/// listens at 11.5. East of x 13 the walker steps into water.
fn scene() -> (Game<'static>, mpsc::Receiver<Out>, mpsc::Receiver<Out>) {
    scene_of("127.0.0.1:25565".parse().unwrap())
}

/// `scene` with the walker's own connection: a remote one, or the window's
/// (`LOCAL`).
fn scene_of(
    walker_address: std::net::SocketAddr,
) -> (Game<'static>, mpsc::Receiver<Out>, mpsc::Receiver<Out>) {
    let mut g = crate::game::native_machines::tests::server_game();
    g.build_registries();
    g.set_allow_commands(true);
    let stone = g
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap()
        .0;
    let water = g
        .pack
        .assets
        .registry
        .default_state("minecraft:water")
        .unwrap()
        .0;
    let mut chunk = Chunk::empty(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set(x, FLOOR, z, stone);
            if x >= WATER {
                chunk.set(x, FLOOR + 1, z, water);
                chunk.set(x, FLOOR + 2, z, water);
            }
        }
    }
    g.world.chunks.insert((0, 0), Arc::new(chunk));
    g.block_ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    assert!(g.run_command("forceload add 0 0"));
    assert!(g.run_command("tick freeze"));
    g.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::loopback(0, "guest-sounds".into(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: false,
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
    let feet = f64::from(FLOOR + 1);
    let (mut walker, walker_rx) = watched_guest(1, [10.5, feet, Z], walker_address);
    let remote = "127.0.0.1:25565".parse().unwrap();
    let (mut listener, listener_rx) = watched_guest(2, [11.5, feet, Z], remote);
    for guest in [&mut walker, &mut listener] {
        guest.placed = true;
        guest.loaded = true;
        guest.dimension = "minecraft:overworld".into();
        guest.game_mode = 0;
        guest.on_ground = true;
        guest.load_timeout = 0;
    }
    let hub = &mut g.lan.as_mut().unwrap().hub;
    hub.guests.push(walker);
    hub.guests.push(listener);
    g.sync_tick_chunks();
    g.ticked_entities_through = g.next_entity;
    (g, walker_rx, listener_rx)
}

/// One server tick in which the walker says it moved to `x`, on the ground.
fn walk_to(g: &mut Game<'_>, x: f64) {
    let guest = &mut g.lan.as_mut().unwrap().hub.guests[0];
    guest.inbox.push_back((S::ClientTickEnd(Empty), 0));
    let to = play::MovePlayerPos {
        x,
        y: f64::from(FLOOR + 1),
        z: Z,
        flags: 1,
    };
    guest.inbox.push_back((S::MovePlayerPos(to), 0));
    guest.inbox.push_back((S::ClientTickEnd(Empty), 0));
    g.tick_lan_packets();
    let guest = &g.lan.as_ref().unwrap().hub.guests[0];
    assert_eq!(guest.moves.len(), 1, "the move is accepted");
    assert_eq!(guest.position[0], x);
    g.tick_lan(false);
}

/// A `ClientboundSoundPacket` a guest's client was sent.
#[derive(Debug, PartialEq)]
struct Heard {
    name: String,
    source: i32,
    volume: f32,
    pitch: f32,
    /// The position as the packet quantizes it.
    at: [i32; 3],
}

/// The sounds a guest's client was sent this tick.
fn sounds(g: &Game<'_>, rx: &mpsc::Receiver<Out>) -> Vec<Heard> {
    let hub = &g.lan.as_ref().unwrap().hub;
    let mut out = Vec::new();
    while let Ok(message) = rx.try_recv() {
        let bodies = match message {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(reason) => panic!("a player was disconnected: {reason:?}"),
        };
        for body in bodies {
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            let C::Sound(sound) = C::decode(id, reader.rest()).unwrap() else {
                continue;
            };
            let name = match &sound.sound.1 {
                Some((name, _)) => name.clone(),
                None => hub
                    .registry_name("minecraft:sound_event", sound.sound.0)
                    .unwrap_or_default()
                    .to_owned(),
            };
            out.push(Heard {
                name,
                source: sound.source,
                volume: sound.volume,
                pitch: sound.pitch,
                at: [sound.x, sound.y, sound.z],
            });
        }
    }
    out
}

/// The walker's feet at `x`, as `ClientboundSoundPacket` stores them.
fn at(x: f64) -> [i32; 3] {
    let y = f64::from(FLOOR + 1);
    [(x * 8.0) as i32, (y * 8.0) as i32, (Z * 8.0) as i32]
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_guests_footsteps_and_splash_are_heard_by_the_others_but_not_by_itself() {
    let (mut g, walker, listener) = scene();
    // The first tick settles the guest on the floor.
    walk_to(&mut g, 10.5);
    assert!(sounds(&g, &listener).is_empty(), "standing still is silent");

    // `nextStep` starts at 1.0: half a metre is no step, a metre is one.
    // `Player.playStepSound` plays the block's `SoundType` step at 0.15 of its
    // volume, from `Entity.applyMovementEmissionAndPlaySound`.
    walk_to(&mut g, 11.5);
    assert!(
        sounds(&g, &listener).is_empty(),
        "the first half metre is silent"
    );
    walk_to(&mut g, 12.5);
    assert_eq!(
        sounds(&g, &listener),
        vec![Heard {
            name: "minecraft:block.stone.step".to_owned(),
            source: PLAYERS,
            volume: 0.15,
            pitch: 1.0,
            at: at(12.5),
        }],
        "the other player hears the step at the walker's feet"
    );

    // Into the water at x 13: `Entity.doWaterSplashEffect`'s splash at the
    // speed it entered at.
    walk_to(&mut g, 13.0);
    let heard = sounds(&g, &listener);
    assert_eq!(heard.len(), 1, "{heard:?}");
    let splash = &heard[0];
    assert_eq!(splash.name, "minecraft:entity.player.splash");
    assert_eq!((splash.source, splash.at), (PLAYERS, at(13.0)));
    assert!(
        splash.volume > 0.0 && splash.volume < 0.25,
        "the speed it entered at: {splash:?}"
    );
    assert!((0.6..1.4).contains(&splash.pitch), "{splash:?}");

    // Walking on in water: `Entity.waterSwimSound` at the swimmer's speed,
    // then `playMuffledStepSound`'s quieter, lower step.
    walk_to(&mut g, 14.5);
    let heard = sounds(&g, &listener);
    assert_eq!(heard.len(), 2, "{heard:?}");
    let (swim, step) = (&heard[0], &heard[1]);
    assert_eq!(swim.name, "minecraft:entity.player.swim");
    assert_eq!((swim.source, swim.at), (PLAYERS, at(14.5)));
    assert!(swim.volume <= 0.35, "the swimmer's speed: {swim:?}");
    assert!((0.6..1.4).contains(&swim.pitch), "{swim:?}");
    assert_eq!(
        step,
        &Heard {
            name: "minecraft:block.stone.step".to_owned(),
            source: PLAYERS,
            volume: 0.05,
            pitch: 0.8,
            at: at(14.5),
        },
        "`playMuffledStepSound` under the swim sound"
    );

    // The walker's own client plays what it makes; the server leaves it out
    // (`Player.playSound` broadcasts with the player as `except`).
    assert!(
        sounds(&g, &walker).is_empty(),
        "the walker is sent none of its own"
    );
}

/// A player on the window's own machine (the single player's owner) plays what
/// its body makes on its own client, so the server's copy must not play it on
/// that window a second time.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_local_players_step_is_played_once() {
    let (mut g, _walker, listener) = scene_of(LOCAL);
    walk_to(&mut g, 10.5);
    walk_to(&mut g, 11.5);
    walk_to(&mut g, 12.5);
    // The players around still hear it, as the server's broadcast does.
    assert_eq!(
        sounds(&g, &listener),
        vec![Heard {
            name: "minecraft:block.stone.step".to_owned(),
            source: PLAYERS,
            volume: 0.15,
            pitch: 1.0,
            at: at(12.5),
        }],
        "a remote player hears the local player's step"
    );
    // The window does not: its own client played it (`LocalPlayer.move`).
    assert_eq!(
        g.audio
            .heard
            .get("minecraft:block.stone.step")
            .copied()
            .unwrap_or(0),
        0,
        "the window plays no second step for its own body"
    );
}

/// `Player.getMovementEmission`: a player sneaking on the ground emits
/// nothing, from the tick the sneak starts (`isShiftKeyDown` comes from a
/// packet the server handled before the tick).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_sneaking_players_step_is_silent_from_the_tick_it_starts() {
    let (mut g, _walker, listener) = scene();
    walk_to(&mut g, 10.5);
    walk_to(&mut g, 11.5);
    g.lan.as_mut().unwrap().hub.guests[0].sneaking = true;
    walk_to(&mut g, 12.5);
    assert!(
        sounds(&g, &listener).is_empty(),
        "a sneaking walker makes no sound"
    );
}
