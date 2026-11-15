//! `ThrownEnderpearl.onHit` for a pearl whose owner is in another level: the
//! owner goes to the pearl's level (`ServerPlayer.teleport` with the pearl's
//! `TeleportTransition(level, teleportPos, ..)`), not to the pearl's
//! coordinates in their own level. A stasis chamber's pearl, left in the
//! Overworld, takes its owner back from the Nether.

use std::cell::Cell;

use super::{Dim, Game, LevelTeleport};
use horizon_core::geometry::Vec3;

thread_local! {
    /// The level of the guest whose pearl is landing now, when known.
    static OWNER_LEVEL: Cell<Option<Dim>> = const { Cell::new(None) };
}

/// Runs `f`, a guest's pearl landing, with the guest's own level `level` known
/// to `pearl_takes_owner` (the hub, which holds it, is out of reach during the
/// guest's turn).
pub(in super::super) fn with_owner_level<R>(level: Option<Dim>, f: impl FnOnce() -> R) -> R {
    let outer = OWNER_LEVEL.with(|c| c.replace(level));
    let result = f();
    OWNER_LEVEL.with(|c| c.set(outer));
    result
}

impl Game<'_> {
    /// `ThrownEnderpearl.onHit` once the pearl may take its owner (the player
    /// in place): `player.teleport` to `to`, then `hurtServer(enderPearl, 5)`
    /// (`damage`, as the mods left it) and the teleport sound at `to`.
    pub(in super::super) fn pearl_takes_owner(&mut self, to: Vec3, damage: f32) {
        let away = OWNER_LEVEL
            .with(Cell::get)
            .filter(|level| *level != self.dim);
        if let (Some(conn), Some(_)) = (self.lan_acting, away) {
            // Into the pearl's level: a new body there, out of its vehicle and
            // with no fall distance or motion (`run_level_teleports`); the
            // rotation is the player's own (`Relative.ROTATION`).
            self.level_teleports.push(LevelTeleport {
                conn,
                dim: self.dim,
                at: [to.x, to.y, to.z],
                rotation: None,
                vehicle: None,
                cause: Some("ender_pearl"),
            });
        } else {
            // ServerPlayer.teleport: removeVehicle before the placement.
            self.stop_riding();
            if let Some(pl) = self.player.as_mut() {
                pl.position = to;
                pl.velocity = Vec3::ZERO;
                pl.fall_distance = 0.0;
                pl.impulse.reset();
                pl.teleported = true;
            }
            if self.lan_acting.is_some() {
                horizon_client::lan::hub::teleport_cause(Some("ender_pearl"));
            }
            self.previous_position = to;
        }
        if damage > 0.0 {
            self.hurt_player_by("minecraft:ender_pearl", damage);
        }
        self.audio.play(
            "minecraft:entity.player.teleport",
            Some([to.x, to.y, to.z]),
            1.0,
            1.0,
        );
    }
}
