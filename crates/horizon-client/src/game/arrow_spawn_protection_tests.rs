//! `TntBlock.onProjectileHit` (`TntBlock.java:134-141`) and
//! `CampfireBlock.onProjectileHit` (`CampfireBlock.java:220-228`) ask
//! `Projectile.mayInteract` before they react, and for an arrow a player shot
//! that is `ServerLevel.mayInteract` (`Projectile.java:422-426`): on a
//! dedicated server a non-operator's arrow primes no TNT and lights no campfire
//! inside the spawn protection radius, while outside it both react.
use crate::game::Game;
use horizon_client::chunk::Chunk;
use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
use horizon_client::mobs::Who;
use horizon_client::projectiles::{Projectile, ProjectileKind};
use horizon_core::{geometry::Vec3, position::ChunkPos};
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const CONN: u32 = 5;
const GUEST_UUID: u128 = 426;
/// The world spawn, and the TNT and campfire inside the radius around it.
const SPAWN: [i32; 3] = [8, 200, 8];
const TNT_INSIDE: [i32; 3] = [8, 200, 8];
const CAMPFIRE_INSIDE: [i32; 3] = [12, 200, 8];
/// The TNT and campfire past the radius: 19 and 23 blocks east of the spawn.
const TNT_OUTSIDE: [i32; 3] = [27, 200, 8];
const CAMPFIRE_OUTSIDE: [i32; 3] = [31, 200, 8];

/// The name and `lit` of the block at `at`.
fn block(game: &Game<'_>, at: [i32; 3]) -> (String, Option<String>) {
    let state = game
        .pack
        .assets
        .registry
        .state(game.world.block(at[0], at[1], at[2]));
    match state {
        Some(state) => (state.name.clone(), state.properties.get("lit").cloned()),
        None => (String::new(), None),
    }
}

/// A burning arrow of the guest's shot straight down onto the block at `at`.
fn shoot_at(game: &mut Game<'_>, at: [i32; 3]) {
    let mut arrow = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        Vec3::new(
            f64::from(at[0]) + 0.5,
            f64::from(at[1]) + 1.6,
            f64::from(at[2]) + 0.5,
        ),
        Some(Who::Player),
    );
    // `Who::Player` is the guest who shot it, and the flame it was shot with.
    arrow.guest = Some(CONN);
    arrow.velocity = Vec3::new(0.0, -2.0, 0.0);
    arrow.arrow.as_mut().unwrap().fire_ticks = 100;
    game.add_projectile(arrow);
    game.ticked_entities_through = game.next_entity;
    game.tick_projectiles().unwrap();
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_guest_without_build_rights_light_nothing_with_a_flaming_arrow() {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    // The second chunk holds the blocks outside the radius.
    game.world
        .chunks
        .insert((1, 0), Arc::new(Chunk::empty(1, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(1, 0))
        .unwrap();
    for command in [
        "forceload add 0 0 31 15",
        "setblock 8 200 8 minecraft:tnt",
        "setblock 12 200 8 minecraft:campfire[lit=false]",
        "setblock 27 200 8 minecraft:tnt",
        "setblock 31 200 8 minecraft:campfire[lit=false]",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    // The world spawn is what the protection radius measures from, and the
    // server protects it (`DedicatedServer.isUnderSpawnProtection`).
    game.level.world_spawn = Some(SPAWN);
    game.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "arrow-spawn-protection-test".into(), 2)
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
            name: "ArrowGuest".into(),
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
        [8.5, 200.0, 24.5],
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
    // The arrows tick on the level, outside the guest's turn: the hub is the
    // game's again by then.
    game.lan = Some(host);

    // Inside the radius the burning arrow sticks without priming or lighting.
    shoot_at(&mut game, TNT_INSIDE);
    assert_eq!(
        block(&game, TNT_INSIDE),
        ("minecraft:tnt".into(), None),
        "a non-operator's arrow primes no TNT"
    );
    assert!(game.tnts.is_empty(), "no primed TNT was spawned");
    shoot_at(&mut game, CAMPFIRE_INSIDE);
    assert_eq!(
        block(&game, CAMPFIRE_INSIDE).1.as_deref(),
        Some("false"),
        "a non-operator's arrow lights no campfire"
    );
    // Outside it, the same arrows prime and light.
    shoot_at(&mut game, TNT_OUTSIDE);
    assert_eq!(
        block(&game, TNT_OUTSIDE).0,
        "minecraft:air",
        "outside the radius the TNT is primed and taken out"
    );
    assert_eq!(game.tnts.len(), 1, "one primed TNT");
    assert_eq!(
        [game.tnts[0].position.x, game.tnts[0].position.z],
        [
            f64::from(TNT_OUTSIDE[0]) + 0.5,
            f64::from(TNT_OUTSIDE[2]) + 0.5
        ],
        "primed where the arrow struck"
    );
    shoot_at(&mut game, CAMPFIRE_OUTSIDE);
    assert_eq!(
        block(&game, CAMPFIRE_OUTSIDE).1.as_deref(),
        Some("true"),
        "outside the radius the campfire is lit"
    );
}
