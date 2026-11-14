//! A player's blow on an End crystal blows it up (`EndCrystal.hurtServer`), and
//! `ServerExplosion.hurtEntities` hurts and pushes every player in reach, the
//! attacker and everyone else alike (`ServerPlayer.onExplosionHit`, then the
//! `ClientboundExplodePacket` carries each pushed player's knockback).
//! Two players here, the attacker and a bystander in reach.
use super::super::*;
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
};
use horizon_protocol::packets::{configuration::ClientInformation, play};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const ATTACKER: u32 = 2;
const BYSTANDER: u32 = 3;
const CRYSTAL: [f64; 3] = [22.5, 100.0, 10.5];

/// A dedicated-style host (no player of its own) on a stone floor, with an End
/// crystal and two connected, placed, loaded guests: the attacker (creative,
/// 3 blocks from the crystal, so the blast leaves them unhurt) and a
/// survival bystander 9 blocks from it (about 14 of their 20 health lost).
fn game() -> (Game<'static>, mpsc::Receiver<Out>, mpsc::Receiver<Out>, u32) {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
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
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
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
    for x in 0..2 {
        game.world
            .chunks
            .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
        game.block_ticks
            .add_empty_chunk(horizon_core::position::ChunkPos::new(x, 0))
            .unwrap();
    }
    assert!(game.run_command("fill 0 99 0 31 99 15 stone"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 6.5)).unwrap());
    let net = NetConfig {
        announce: false,
        ..NetConfig::lan(0, "crystal-blast-test".into(), 4)
    };
    game.lan_host(
        lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        net,
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 4,
            integrated: false,
            levels: Default::default(),
        },
    )
    .unwrap();
    let mut guest = |conn: u32, name: &str, at: [f64; 3], game_mode: i32| {
        let (tx, rx) = mpsc::channel();
        let mut g = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn),
                name: name.into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: format!("127.0.0.1:{}", 25565 + conn).parse().unwrap(),
                expected: false,
                configured: Default::default(),
            },
            conn as i32,
            at,
            2,
        );
        g.placed = true;
        g.loaded = true;
        // The client sent `ServerboundPlayerLoadedPacket`: until then damage is refused.
        g.load_timeout = 0;
        g.game_mode = game_mode;
        game.lan.as_mut().unwrap().hub.guests.push(g);
        rx
    };
    let attacker = guest(ATTACKER, "AttackerA", [19.5, 100.0, 10.5], 1);
    let bystander = guest(BYSTANDER, "BystanderB", [22.5, 100.0, 1.5], 0);
    // A dedicated server has no player of its own (`dedicated.rs` drops it
    // once the spawn area is ready).
    game.player = None;
    assert!(game.run_command(&format!(
        "summon end_crystal {} {} {}",
        CRYSTAL[0], CRYSTAL[1], CRYSTAL[2]
    )));
    let crystal = game.crystals.last().expect("the crystal is summoned").id;
    (game, attacker, bystander, crystal)
}

/// The clientbound play packets the guest was sent so far.
fn sent(rx: &mpsc::Receiver<Out>) -> Vec<play::Clientbound> {
    let mut bodies = Vec::new();
    for out in rx.try_iter() {
        match out {
            Out::Packet(body) => bodies.push(body),
            Out::Packets(many) => bodies.extend(many),
            Out::Disconnect(_) => {}
        }
    }
    bodies
        .iter()
        .filter_map(|body| {
            let mut r = horizon_protocol::buf::Reader::new(body);
            let id = r.varint().ok()?;
            play::Clientbound::decode(id, r.rest()).ok()
        })
        .collect()
}

/// The guest's health as the hub holds it between turns.
fn health(game: &Game<'_>, conn: u32) -> f32 {
    game.lan.as_ref().unwrap().hub.guest(conn).unwrap().health
}

/// The `ClientboundExplodePacket` the guest was sent for the crystal's blast.
fn explosion(packets: &[play::Clientbound]) -> Option<&play::Explode> {
    packets.iter().find_map(|p| match p {
        play::Clientbound::Explode(e) if e.center == CRYSTAL => Some(e),
        _ => None,
    })
}

/// A guest's blow on the crystal blows it up inside that guest's turn, where
/// the hub is out of `self.lan` (`tick_lan_packets`): `ServerExplosion.hurtEntities`
/// still hurts the bystander and the blast packet still carries their push, as
/// for a command (`/damage`). Expected values worked out from vanilla's
/// formulas: 9 blocks from a radius-6 blast, nothing in the way, no armor, so
/// `(1 - 9/12)` of the blast's reach, damage `(p*p + p) / 2 * 7 * 12 + 1` for
/// `p = 0.25` (14.125), knocked back `p` along centre to eye
/// (`(0, 1.62, -9)` normalised).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_guests_blow_on_a_crystal_hurts_and_pushes_the_bystander() {
    let (mut game, attacker, bystander, crystal) = game();
    let _ = (sent(&attacker), sent(&bystander));
    let mut host = game.lan.take().expect("the world is hosted");
    game.lan_guest_packet(
        &mut host,
        ATTACKER,
        play::Serverbound::Attack(play::Attack { id: crystal as i32 }),
    );
    game.lan = Some(host);
    // The packet is handled (`lan_process_events`): the blow it put off runs.
    game.run_guest_strikes();
    // The turn is over: what it put off reaches the players (`tick_lan`).
    game.flush_lan_explosions();
    assert!(game.crystals.iter().all(|c| c.id != crystal), "it blew up");
    let hurt = 20.0 - health(&game, BYSTANDER);
    assert!((hurt - 14.125).abs() < 0.01, "lost {hurt}");
    let push = explosion(&sent(&bystander))
        .expect("the bystander is sent the blast")
        .knockback
        .expect("the blast pushes the bystander");
    let length = (1.62f64 * 1.62 + 81.0).sqrt();
    for (got, want) in push
        .iter()
        .zip([0.0, 0.25 * 1.62 / length, -0.25 * 9.0 / length])
    {
        assert!((got - want).abs() < 1e-4, "pushed {push:?}");
    }
    assert!(
        explosion(&sent(&attacker)).is_some_and(|e| e.knockback.is_some()),
        "the attacker is pushed too"
    );
}
