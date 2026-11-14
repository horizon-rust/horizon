//! `ThrownEnderpearl.tick`: a pearl a player threw places a
//! `TicketType.ENDER_PEARL` ticket on its chunk
//! (`ServerPlayer.registerAndUpdateEnderPearlTicket` ->
//! `placeEnderPearlTicket`: `addTicketWithRadius(ENDER_PEARL, chunk, 2)`, so
//! level `FULL - 2` = 31, refreshed every 39 ticks). That ticket loads and
//! simulates the pearl's chunk and keeps its dimension running
//! (`TicketType.ENDER_PEARL` flags `LOADING | SIMULATION | KEEP_DIMENSION_ACTIVE`,
//! `ServerChunkCache.hasActiveTickets`), so a stasis chamber's furnace keeps
//! cooking with nobody in the level.
use super::native_machines::tests::server_game;
use super::*;
use horizon_client::mobs::Who;
use horizon_client::projectiles::{Projectile, ProjectileKind};
use horizon_core::geometry::Vec3;

/// A pearl in chunk (5, -3) of the Nether, with nobody in that level: the
/// level ticks anyway, and so does the pearl's chunk and its neighbours.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_pearl_keeps_its_chunk_and_level_ticking_with_nobody_in_it() {
    let mut game = server_game();
    game.dim = Dim::Nether;
    game.creatures.projectiles.push(Projectile::new(
        ProjectileKind::EnderPearl,
        Vec3::new(88.5, 100.0, -40.5),
        Some(Who::Player),
    ));
    // `owner instanceof ServerPlayer`: a pearl a mob threw holds no ticket.
    game.creatures.projectiles.push(Projectile::new(
        ProjectileKind::EnderPearl,
        Vec3::new(8.5, 100.0, 8.5),
        None,
    ));
    game.update_chunk_tickets();
    let levels = game.tickets.levels();
    // Level 31 on the pearl's chunk: entities, random ticks and block
    // entities; the ring one chunk out only block ticks, and the ring at 2
    // is `FULL` (`ChunkLevel.byStatus(FULL) - 2`).
    assert!(levels.entity_ticking(5, -3), "the pearl's chunk ticks");
    assert!(levels.random_ticking(5, -3), "and random ticks");
    assert!(
        levels.block_ticking(6, -3) && levels.block_ticking(4, -2),
        "its neighbours block tick"
    );
    assert!(
        !levels.block_ticking(7, -3) && !levels.block_ticking(3, -5),
        "two chunks out only loads"
    );
    assert!(
        !levels.entity_ticking(0, 0),
        "a pearl with no player owner holds no ticket"
    );
    // `TicketType.ENDER_PEARL.shouldKeepDimensionActive`: the level runs with
    // no player in it, as a forced chunk's level does.
    assert_eq!(game.levels_to_tick(), vec![Dim::Nether]);
}
