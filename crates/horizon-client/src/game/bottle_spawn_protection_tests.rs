//! `BottleItem.use` asks `Level.mayInteract` before it fills the bottle
//! (`BottleItem.java:49-56`), so on a dedicated server a guest who is not an
//! operator fills nothing inside the spawn protection radius, while an
//! operator does, and a non-operator outside it does.
use crate::game::lan::LanHost;
use horizon_client::chunk::Chunk;
use horizon_client::items::ItemStack;
use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
use horizon_core::position::ChunkPos;
use horizon_protocol::packets::{
    configuration::ClientInformation,
    play::{Serverbound, UseItem},
};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const CONN: u32 = 4;
const GUEST_UUID: u128 = 425;
/// The water source the bottle fills from: it sits at the world spawn, so it
/// is inside the protection radius.
const SOURCE: [i32; 3] = [10, 199, 10];
/// The water source 17 blocks east of the spawn: past the radius.
const OUTSIDE: [i32; 3] = [27, 199, 10];

/// `ServerboundUseItemPacket` from a player looking straight down.
fn use_item(sequence: i32) -> Serverbound {
    Serverbound::UseItem(UseItem {
        hand: 0,
        yaw: 0.0,
        pitch: 90.0,
        sequence,
    })
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_guest_without_build_rights_fills_no_bottle_under_spawn_protection() {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    // The second chunk holds the water outside the radius.
    game.world
        .chunks
        .insert((1, 0), Arc::new(Chunk::empty(1, 0)));
    assert!(game.run_command("fill 0 199 0 31 199 15 stone"));
    assert!(game.run_command("setblock 10 199 10 water"));
    assert!(game.run_command(&format!(
        "setblock {} {} {} water",
        OUTSIDE[0], OUTSIDE[1], OUTSIDE[2]
    )));
    // The world spawn sits inside what the guest aims at, and the server
    // protects it (`DedicatedServer.isUnderSpawnProtection`).
    game.level.world_spawn = Some(SOURCE);
    game.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "bottle-spawn-protection-test".into(), 2)
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
            uuid: GUEST_UUID,
            name: "BottleGuest".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
            configured: Default::default(),
        },
        CONN as i32,
        [10.5, 200.0, 10.5],
        2,
    );
    guest.dimension = "minecraft:overworld".into();
    guest.placed = true;
    guest.loaded = true;
    guest.selected = 0;
    let mut host = game.lan.take().unwrap();
    host.hub.guests.push(guest);
    // `spawn-protection` in server.properties, and someone else is an
    // operator: `PlayerList.getOps()` is not empty, so the radius holds.
    host.hub.spawn_protection = 16;
    host.hub.ops.insert(900, 4);

    let held = |host: &LanHost| {
        host.hub.guests[0].inventory[0]
            .as_ref()
            .map(|s| s.item.to_string())
    };
    let give = |host: &mut LanHost, item: &str| {
        host.hub.guests[0].inventory[0] = Some(ItemStack::new(item, 1));
    };
    let move_to = |host: &mut LanHost, at: [f64; 3]| {
        host.hub.guests[0].position = at;
        host.hub.guests[0].previous_position = at;
    };

    // Filling from the water source in the protected area: the non-operator
    // keeps the empty bottle.
    give(&mut host, "minecraft:glass_bottle");
    game.lan_guest_packet(&mut host, CONN, use_item(1));
    assert_eq!(
        held(&host).as_deref(),
        Some("minecraft:glass_bottle"),
        "a non-operator's fill is refused"
    );
    // The same non-operator 17 blocks east of the spawn, past the radius,
    // fills it: `isUnderSpawnProtection` measures the distance to the spawn.
    move_to(
        &mut host,
        [
            f64::from(OUTSIDE[0]) + 0.5,
            f64::from(OUTSIDE[1]) + 1.0,
            f64::from(OUTSIDE[2]) + 0.5,
        ],
    );
    game.lan_guest_packet(&mut host, CONN, use_item(2));
    assert_eq!(
        held(&host).as_deref(),
        Some("minecraft:potion"),
        "outside the radius a non-operator fills"
    );
    assert_eq!(
        host.hub.guests[0].inventory[0].as_ref().map(|s| s.count),
        Some(1),
        "one glass bottle makes one water bottle"
    );
    // The same click from an operator fills it inside the radius.
    move_to(&mut host, [10.5, 200.0, 10.5]);
    give(&mut host, "minecraft:glass_bottle");
    host.hub.ops.insert(GUEST_UUID, 4);
    game.lan_guest_packet(&mut host, CONN, use_item(3));
    assert_eq!(
        held(&host).as_deref(),
        Some("minecraft:potion"),
        "an operator still fills"
    );
    // The water source stays: a bottle does not take the block with it.
    let water = game
        .pack
        .assets
        .registry
        .default_state("minecraft:water")
        .unwrap()
        .0;
    assert_eq!(
        game.world.block(SOURCE[0], SOURCE[1], SOURCE[2]).0,
        water,
        "the source is still there"
    );
}
