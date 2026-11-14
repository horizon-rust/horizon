//! What a teleport does to the motion it carries and to a sleeping player:
//! `TeleportCommand.getRelatives`'s relative deltas, the
//! `Entity.teleportSetPosition` they feed, and `ServerPlayer.teleportTo`'s
//! wake-up.

use super::*;

#[cfg(test)]
#[path = "tp_motion_tests.rs"]
mod tp_motion_tests;

/// `TeleportCommand.getRelatives`'s `Relative.direction` half: a relative
/// destination coordinate names `Relative.DELTA_*`, so
/// `PositionMoveRotation.calculateAbsolute` keeps the entity's motion along
/// that axis (`dir` holds for a move inside the level and for one into
/// another; only `Relative.position` is dropped there). The position goes out
/// absolute and the rotation is resolved here, so neither of their bits is
/// set.
pub(super) fn delta_relatives(destination: Option<&Coordinates>) -> i32 {
    use play::relative::{DELTA_X, DELTA_Y, DELTA_Z};
    let relative = match destination {
        // `LocalCoordinates.isXRelative`: `^ ^ ^` is relative on every axis.
        Some(Coordinates::Local(_)) => [true; 3],
        Some(Coordinates::World(axes)) => axes.map(|axis| axis.relative),
        None => [false; 3],
    };
    let bits = [DELTA_X, DELTA_Y, DELTA_Z];
    let mut relatives = 0;
    for (bit, relative) in bits.into_iter().zip(relative) {
        if relative {
            relatives |= bit;
        }
    }
    relatives
}

/// `Entity.teleportSetPosition` for a transition that carries no motion
/// (`TeleportTransition` from `teleportTo`): an axis `relatives` names keeps
/// the entity's own, the others go to zero.
pub(super) fn keep_motion(velocity: &mut Vec3, relatives: i32) {
    use play::relative::{DELTA_X, DELTA_Y, DELTA_Z};
    let kept = *velocity;
    velocity.x = if relatives & DELTA_X != 0 {
        kept.x
    } else {
        0.0
    };
    velocity.y = if relatives & DELTA_Y != 0 {
        kept.y
    } else {
        0.0
    };
    velocity.z = if relatives & DELTA_Z != 0 {
        kept.z
    } else {
        0.0
    };
}

impl Game<'_> {
    /// `Entity.teleportSetPosition` for the player in place: the position, no
    /// fall distance, and the motion `relatives` keeps.
    pub(super) fn tp_place_player(&mut self, v: Vec3, relatives: i32) {
        if let Some(p) = self.player.as_mut() {
            p.position = v;
            keep_motion(&mut p.velocity, relatives);
            p.fall_distance = 0.0;
            p.ground.on_ground = false;
        }
    }

    /// The `Relative` bits of the move the hub has not sent the acting
    /// guest's client yet: `Hub::teleport` sends them with it.
    pub(super) fn tp_guest_relatives(&mut self, conn: u32, relatives: i32) {
        if let Some(host) = self.lan.as_mut()
            && let Some(g) = host.hub.guests.iter_mut().find(|g| g.conn == conn)
        {
            g.teleport_relatives = relatives;
        }
    }

    /// A server's player teleported within their level is placed there at
    /// once: the arrival they were waiting for elsewhere in it (whose chunks
    /// lose their tickets) is abandoned.
    pub(super) fn tp_hold_guest(&mut self, v: Vec3, relatives: i32) {
        let Some(conn) = self.lan_acting else {
            return;
        };
        horizon_client::lan::hub::teleport_cause(Some("command"));
        self.tp_guest_relatives(conn, relatives);
        self.hold_for_chunks(conn, v);
    }

    /// `ServerPlayer.teleportTo`: a sleeping player is woken before the move
    /// (`stopSleepInBed(true, true)`), which stands them beside the bed and
    /// tells their client where they are before the teleport does. Answers
    /// `destination` moved from where the wake left them on the axes
    /// `relatives` names (`Entity.teleportSetPosition` reads
    /// `PositionMoveRotation.of(this)` after the wake), and where they stood
    /// before it.
    pub(super) fn tp_wake(&mut self, destination: Vec3, relatives: i32) -> (Vec3, Option<Vec3>) {
        let was = self.player.as_ref().map(|p| p.position);
        if self.sleeping.is_none() {
            return (destination, was);
        }
        let Some(conn) = self.lan_acting else {
            // The wake's fade is the client's own `sleepCounter`: 100 from
            // `ClientboundAnimatePacket`'s 0 (`ClientPacketListener.handleAnimate`
            // -> `stopSleepInBed(false, false)`), as for every other wake.
            self.wake_up();
            let stood = self.player.as_ref().map(|p| p.position);
            return (woken(destination, relatives, stood, was), was);
        };
        let Some(mut host) = self.lan.take() else {
            return (destination, was);
        };
        let mut stood = None;
        if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
            self.wake_guest(&mut host, gi);
            // The guest's body is the one in the game's hands during their turn
            // (`Guest::motion` is empty then), so `stopSleeping`'s pose change
            // lands on it rather than through `wake_guest_from`.
            if let Some(p) = self.player.as_mut() {
                let _ = p.set_sleeping(false);
            }
            let at = host.hub.guests[gi].position;
            stood = Some(Vec3::new(at[0], at[1], at[2]));
        }
        self.lan = Some(host);
        // The bed and the sleep counter go back to the guest at `leave_guest`:
        // `stopSleepInBed(true, true)` leaves their own `sleepCounter` at 0 (the
        // fade their client shows comes from the animate packet `Hub::wake`
        // sends).
        self.sleeping = None;
        self.sleep_timer = 0;
        (woken(destination, relatives, stood, was), was)
    }
}

/// The destination of a teleport that woke a sleeper first: a relative axis
/// moves from the position the wake left them in, because
/// `Entity.teleportSetPosition` reads `PositionMoveRotation.of(this)` after
/// `ServerPlayer.teleportTo`'s `stopSleepInBed`; an absolute axis is the
/// command's own.
fn woken(destination: Vec3, relatives: i32, stood: Option<Vec3>, was: Option<Vec3>) -> Vec3 {
    use play::relative::{DELTA_X, DELTA_Y, DELTA_Z};
    let (Some(stood), Some(was)) = (stood, was) else {
        return destination;
    };
    let axis = |bit: i32, to: f64, stood: f64, was: f64| {
        if relatives & bit != 0 {
            to + (stood - was)
        } else {
            to
        }
    };
    Vec3::new(
        axis(DELTA_X, destination.x, stood.x, was.x),
        axis(DELTA_Y, destination.y, stood.y, was.y),
        axis(DELTA_Z, destination.z, stood.z, was.z),
    )
}
