//! `TheEndGatewayBlockEntity.findExitPortalXZPosTentative` asks the level for
//! every candidate chunk it tests (`getChunk`, which loads it), so where a new
//! End gateway leads cannot depend on what happened to be loaded when the
//! entity arrived.

use super::super::Dim;
use super::super::native_machines::tests::server_game;
use super::{Arriving, Game, GatewayArrival, GatewayJob};
use horizon_client::chunk::Chunk;
use horizon_core::geometry::Vec3;
use std::sync::Arc;

/// A gateway on the +X axis, in chunk (6, 0).
const GATEWAY: [i32; 3] = [96, 75, 0];
/// The first candidate, `findExitPortalXZPosTentative`'s direction times 1024:
/// its chunk is (64, 0), and the search walks forward through (65, 0), ...
const TARGET: Vec3 = Vec3::new(1024.0, 75.0, 0.0);
/// An end stone block in chunk (65, 0): the landing spot `findValidSpawnInChunk`
/// finds there.
const STONE: [i32; 3] = [1044, 64, 4];
/// The exit that stone leads to: `findTallestBlock` around it, then
/// `findOrCreateValidTeleportPos`'s `above(10)`.
const EXIT: [i32; 3] = [1044, 74, 4];

fn job() -> GatewayJob {
    GatewayJob::Find {
        gateway: GATEWAY,
        exact: false,
    }
}

/// An entity waiting at the gateway for the search to end.
fn wait_for_exit(game: &mut Game<'_>) {
    game.end.arrivals.push(GatewayArrival {
        traveller: Arriving::Live(1),
        job: job(),
        target: TARGET,
    });
}

/// Whether `chunk` is asked for as an area around the gateway's arrivals.
fn kept(game: &Game<'_>, chunk: (i32, i32)) -> bool {
    game.streaming
        .extra
        .iter()
        .any(|&(c, r)| c == chunk && r == 1)
}

/// The exit the gateway's block entity holds (`setExitPosition`), if any.
fn exit_of(game: &Game<'_>) -> Option<[i32; 3]> {
    game.known_gateway_exit(GATEWAY).map(|(exit, _)| exit)
}

/// Every end stone block in the loaded chunks.
fn end_stone(game: &Game<'_>, stone: u32) -> Vec<[i32; 3]> {
    let mut found = Vec::new();
    for &(cx, cz) in game.world.chunks.keys() {
        for x in cx * 16..cx * 16 + 16 {
            for z in cz * 16..cz * 16 + 16 {
                for y in 60..90 {
                    if game.world.block(x, y, z).0 == stone {
                        found.push([x, y, z]);
                    }
                }
            }
        }
    }
    found
}

/// The world the search sees: the gateway's chunk, the candidate chunks around
/// x = 1024 (and the ones `findTallestBlock` reads), and the end stone island
/// in chunk (65, 0). A clean slate each time: the previous search's gateway and
/// island are dropped with their chunks.
fn load_world(game: &mut Game<'_>, with_island: bool) {
    let gateway = game
        .pack
        .assets
        .registry
        .default_state("minecraft:end_gateway")
        .unwrap();
    for cx in 64..=66 {
        for cz in -1..=1 {
            game.world.chunks.remove(&(cx, cz));
            if !with_island && cx == 65 {
                continue;
            }
            game.world
                .chunks
                .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
        }
    }
    game.world.chunks.remove(&(6, 0));
    game.world.block_entities.clear();
    game.world
        .chunks
        .insert((6, 0), Arc::new(Chunk::empty(6, 0)));
    game.world
        .set(GATEWAY[0], GATEWAY[1], GATEWAY[2], gateway)
        .unwrap();
    if with_island {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:end_stone")
            .unwrap();
        game.world.set(STONE[0], STONE[1], STONE[2], stone).unwrap();
    }
}

/// The exit search loads every candidate it tests, as vanilla's `getChunk`
/// does: with the island's chunk not loaded yet, it waits for that chunk
/// instead of ending there and taking `findOrCreateValidTeleportPos`'s
/// new-island fallback, and finds the same exit once the chunk is there.
#[test]
fn the_exit_search_does_not_stop_at_an_unloaded_candidate() {
    if std::env::var_os("HORIZON_TEST_RESOURCES").is_none() {
        return;
    }
    let mut game = server_game();
    game.dim = Dim::End;
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:end_stone")
        .unwrap();
    // Every candidate loaded: the island's stone is the exit.
    load_world(&mut game, true);
    let _ = game.resolve_gateway(job(), TARGET);
    assert_eq!(
        exit_of(&game),
        Some(EXIT),
        "the preloaded search finds the island's stone"
    );
    // The same search with the island's chunk unloaded: it is one of the
    // candidates, so the search must wait for it rather than stop there.
    load_world(&mut game, false);
    let _ = game.resolve_gateway(job(), TARGET);
    assert_eq!(
        exit_of(&game),
        None,
        "an unloaded candidate chunk is not the end of the search"
    );
    assert_eq!(
        end_stone(&game, stone.0),
        Vec::<[i32; 3]>::new(),
        "no island was built where the search stopped"
    );
    // Loaded, the search ends where the preloaded one did.
    load_world(&mut game, true);
    let _ = game.resolve_gateway(job(), TARGET);
    assert_eq!(
        exit_of(&game),
        Some(EXIT),
        "the loading state did not change the exit"
    );
}

/// The candidate the search stops at is kept loaded until it is there: the
/// search waits for it and is tried again, instead of ending at a chunk
/// nothing asked for.
#[test]
fn the_exit_search_keeps_the_candidate_chunk_it_waits_for_loaded() {
    if std::env::var_os("HORIZON_TEST_RESOURCES").is_none() {
        return;
    }
    let mut game = server_game();
    game.dim = Dim::End;
    load_world(&mut game, false);
    wait_for_exit(&mut game);
    let _ = game.resolve_gateway(job(), TARGET);
    game.keep_gateway_destinations();
    assert!(
        kept(&game, (65, 0)),
        "chunk (65, 0) is loaded for the search: {:?}",
        game.streaming.extra
    );
}

/// A retry keeps every candidate an earlier try tested: with (65, 0) tested
/// empty and (66, 0) missing, then (65, 0) unloaded while (66, 0) loads, the
/// retry waits for (65, 0) and must not release (66, 0), or the two could take
/// turns being unloaded and the traveller would wait forever. Once nobody
/// waits for the gateway, its search's chunks are released.
#[test]
fn the_exit_search_keeps_every_candidate_it_tested_until_it_ends() {
    if std::env::var_os("HORIZON_TEST_RESOURCES").is_none() {
        return;
    }
    let mut game = server_game();
    game.dim = Dim::End;
    load_world(&mut game, true);
    // The candidates (64, 0) and (65, 0) are empty, (66, 0) is not loaded.
    for cz in -1..=1 {
        game.world.chunks.remove(&(66, cz));
        game.world
            .chunks
            .insert((65, cz), Arc::new(Chunk::empty(65, cz)));
    }
    wait_for_exit(&mut game);
    assert_eq!(game.resolve_gateway(job(), TARGET).ok(), Some(None));
    game.keep_gateway_destinations();
    assert!(
        kept(&game, (65, 0)) && kept(&game, (66, 0)),
        "{:?}",
        game.streaming.extra
    );
    // A sweep unloads (65, 0) as (66, 0) arrives: the retry waits for (65, 0).
    game.world.chunks.remove(&(65, 0));
    game.world
        .chunks
        .insert((66, 0), Arc::new(Chunk::empty(66, 0)));
    assert_eq!(game.resolve_gateway(job(), TARGET).ok(), Some(None));
    game.keep_gateway_destinations();
    assert!(
        kept(&game, (65, 0)) && kept(&game, (66, 0)),
        "the retry keeps the candidate it already tested: {:?}",
        game.streaming.extra
    );
    // The traveller is gone: nothing holds the search's chunks any more.
    game.end.arrivals.clear();
    game.keep_gateway_destinations();
    assert!(
        !kept(&game, (65, 0)) && !kept(&game, (66, 0)),
        "{:?}",
        game.streaming.extra
    );
}
