//! `ServerPlayer.teleportTo`'s wake-up: a sleeping player gets up before the
//! teleport moves them (`stopSleepInBed(true, true)`), and a relative
//! destination then moves from where they got up.

use super::*;

#[cfg(test)]
#[path = "tp_motion_tests.rs"]
mod tp_motion_tests;

impl Game<'_> {
    /// `ServerPlayer.teleportTo`: a sleeping player is woken before the move
    /// (`stopSleepInBed(true, true)`), which stands them beside the bed and
    /// tells their client where they are before the teleport does. Answers
    /// `destination`, moved from where the wake left them on the axes
    /// `relative` names: `Entity.teleportSetPosition` reads
    /// `PositionMoveRotation.of(this)` after the wake, so a relative axis adds
    /// the distance the wake took them.
    pub(super) fn tp_wake(&mut self, destination: Vec3, relative: [bool; 3]) -> Vec3 {
        let was = self.player.as_ref().map(|p| p.position);
        if self.sleeping.is_none() {
            return destination;
        }
        // The wake's fade is the client's own `sleepCounter`: `stopSleepInBed`'s
        // forceful wake-up leaves it at 0, and the client fades from the
        // `ClientboundAnimatePacket`'s 0 (`ClientPacketListener.handleAnimate` ->
        // `stopSleepInBed(false, false)`), as for every other wake.
        self.wake_up(true, true);
        let stood = self.player.as_ref().map(|p| p.position);
        woken(destination, relative, stood, was)
    }
}

/// The destination of a teleport that woke a sleeper first: a relative axis
/// moves from the position the wake left them in, because
/// `Entity.teleportSetPosition` reads `PositionMoveRotation.of(this)` after
/// `ServerPlayer.teleportTo`'s `stopSleepInBed`; an absolute axis is the
/// command's own.
fn woken(destination: Vec3, relative: [bool; 3], stood: Option<Vec3>, was: Option<Vec3>) -> Vec3 {
    let (Some(stood), Some(was)) = (stood, was) else {
        return destination;
    };
    let axis = |keep: bool, to: f64, stood: f64, was: f64| {
        if keep {
            to + (stood - was)
        } else {
            to
        }
    };
    Vec3::new(
        axis(relative[0], destination.x, stood.x, was.x),
        axis(relative[1], destination.y, stood.y, was.y),
        axis(relative[2], destination.z, stood.z, was.z),
    )
}
