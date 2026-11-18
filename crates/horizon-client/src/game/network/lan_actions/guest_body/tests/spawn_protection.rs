//! `FarmlandBlock.fallOn` (`FarmlandBlock.java:104-107`) asks
//! `Server.isUnderSpawnProtection` for a player's landing: on a dedicated
//! server a guest who is not an operator tramples no farmland inside the spawn
//! protection radius, while outside it the same landing turns it to dirt.
use super::guest;
use crate::game::Game;
use horizon_client::lan::guest::InboxItem;
use horizon_client::{
    chunk::Chunk,
    lan::{NetConfig, hub::HubConfig},
};
use horizon_core::position::ChunkPos;
use horizon_protocol::packets::{Empty, play, play::Serverbound as S};
use std::sync::Arc;

/// The world spawn; both farmlands are measured from it.
const SPAWN: [i32; 3] = [4, 64, 4];
/// The farmland at the spawn, inside the radius.
const INSIDE: [i32; 3] = [4, 64, 4];
/// The farmland 17 blocks east: past the radius.
const OUTSIDE: [i32; 3] = [21, 64, 4];

/// A server with one survival guest in the air over farmland at the spawn and
/// past the radius, and the `spawn-protection` radius that holds a
/// non-operator back.
fn scene() -> Game<'static> {
    let mut g = crate::game::native_machines::tests::server_game();
    g.options.execution_role = crate::game::execution_role::ExecutionRole::DedicatedServer;
    g.build_registries();
    g.set_allow_commands(true);
    g.world.chunks.insert((1, 0), Arc::new(Chunk::empty(1, 0)));
    let farmland = g
        .pack
        .assets
        .registry
        .default_state("minecraft:farmland")
        .unwrap();
    for at in [INSIDE, OUTSIDE] {
        g.world.set(at[0], at[1], at[2], farmland).unwrap();
    }
    g.block_ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    g.block_ticks.add_empty_chunk(ChunkPos::new(1, 0)).unwrap();
    // The world spawn is what the protection radius measures from.
    g.level.world_spawn = Some(SPAWN);
    g.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "guest-farmland-spawn-protection".into(), 2)
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
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let mut guest = guest(1);
    guest.position = [4.5, 67.0, 4.5];
    guest.placed = true;
    guest.loaded = true;
    guest.dimension = "minecraft:overworld".into();
    guest.game_mode = 0;
    guest.on_ground = false;
    guest.load_timeout = 0;
    let host = g.lan.as_mut().unwrap();
    host.hub.guests.push(guest);
    // `spawn-protection` in server.properties, and someone else is an
    // operator: `PlayerList.getOps()` is not empty, so the radius holds.
    host.hub.spawn_protection = 16;
    host.hub.ops.insert(900, 4);
    g.sync_tick_chunks();
    g.ticked_entities_through = g.next_entity;
    g
}

/// The name of the block at `at`.
fn block(g: &Game<'_>, at: [i32; 3]) -> String {
    let state = g.world.block(at[0], at[1], at[2]);
    g.res.facts().get(state).name.to_string()
}

/// One server tick in which the guest fell two blocks onto the block at `at`,
/// its client's move ending on its feet on top of it.
fn fall_onto(g: &mut Game<'_>, at: [i32; 3]) {
    let (x, z) = (f64::from(at[0]) + 0.5, f64::from(at[2]) + 0.5);
    g.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
        // The fall starts two blocks above where it lands, so `fallOn` sees a
        // distance of 2 (`Entity.checkFallDamage`).
        guest.position = [x, f64::from(at[1]) + 3.0, z];
        guest.previous_position = guest.position;
        guest.on_ground = false;
        guest.moves.clear();
        guest
            .inbox
            .push_back(InboxItem::Packet(S::ClientTickEnd(Empty), None));
        let to = play::MovePlayerPos {
            x,
            y: f64::from(at[1]) + 1.0,
            z,
            flags: 1,
        };
        guest
            .inbox
            .push_back(InboxItem::Packet(S::MovePlayerPos(to), None));
        guest
            .inbox
            .push_back(InboxItem::Packet(S::ClientTickEnd(Empty), None));
    });
    g.tick_lan_packets();
    g.tick_lan(false);
}

#[test]
#[ignore = "daily"]
fn a_guest_without_build_rights_tramples_no_farmland_under_spawn_protection() {
    let mut g = scene();
    // Inside the radius the landing leaves the farmland alone.
    fall_onto(&mut g, INSIDE);
    assert_eq!(
        block(&g, INSIDE),
        "minecraft:farmland",
        "a non-operator's landing tramples no farmland"
    );
    // Outside it, the same landing tramples.
    fall_onto(&mut g, OUTSIDE);
    assert_eq!(
        block(&g, OUTSIDE),
        "minecraft:dirt",
        "outside the radius the landing tramples"
    );
}
