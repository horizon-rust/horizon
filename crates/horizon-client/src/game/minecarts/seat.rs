//! `AbstractMinecart`'s rider seat and the side it sets a rider down on:
//! `getPassengerAttachmentPoint` and `getDismountLocationForPassenger`.
use super::super::mounts::DismountWorld;
use super::*;

/// AbstractMinecart passenger attachment Y (EntityAttachments).
pub(super) const PASSENGER_Y: f64 = 0.1875;

/// `AbstractMinecart.getPassengerAttachmentPoint`: a villager and a wandering
/// trader get `LOWERED_PASSENGER_ATTACHMENT`, the cart's own y.
pub(super) fn cart_attachment(s: Species) -> f64 {
    match s {
        Species::Villager | Species::WanderingTrader => 0.0,
        _ => PASSENGER_Y,
    }
}

/// `Entity.positionRider`'s offset on an `AbstractMinecart`: the cart's
/// passenger attachment less the rider's own `vehicle_attachment` (the cart's
/// box is 0.98 wide).
pub(super) fn cart_seat_offset(s: Species, vehicle_attachment: f64) -> f64 {
    cart_attachment(s) - vehicle_attachment
}

/// `AbstractMinecart.getMotionDirection` as a `from2DDataValue` value (south,
/// west, north, east): `OldMinecartBehavior` turns the cart's facing clockwise,
/// and the opposite facing's clockwise when the cart is flipped; the
/// experiment keeps the facing itself (`MinecartBehavior`).
pub(super) fn cart_motion_face(yaw: f32, flipped: bool, improvements: bool) -> i32 {
    let facing = (f64::from(yaw) / 90.0 + 0.5).floor() as i32;
    if improvements {
        facing
    } else {
        facing + if flipped { 3 } else { 1 }
    }
}

/// `DismountHelper.offsetsForDirection` for a `from2DDataValue` face: right,
/// left, back right, back left, forward right, forward left, back, forward.
pub(super) fn dismount_offsets(face: i32) -> [(i32, i32); 8] {
    let (fx, fz) = [(0, 1), (-1, 0), (0, -1), (1, 0)][(face & 3) as usize];
    // `right` is `Direction.getClockWise`.
    let (rx, rz) = (-fz, fx);
    [
        (rx, rz),
        (-rx, -rz),
        (rx - fx, rz - fz),
        (-rx - fx, -rz - fz),
        (fx + rx, fz + rz),
        (fx - rx, fz - rz),
        (-fx, -fz),
        (fx, fz),
    ]
}

/// `AbstractMinecart.getDismountLocationForPassenger` for a mob `width` by
/// `height` in its one dismount pose (`LivingEntity.getDismountPoses`:
/// standing): the floor of the first block round the cart's, in
/// `DismountHelper.offsetsForDirection` order from the motion direction
/// `face`, at the cart's level, then one up, then one down
/// (`POSE_DISMOUNT_HEIGHTS`), where the mob's box fits (`canDismountTo`).
/// With no room, `Entity.getDismountLocationForPassenger`: on top of the
/// cart's box.
pub(in crate::game) fn mob_dismount_location(
    cart: Vec3,
    face: i32,
    width: f32,
    height: f32,
    world: &impl DismountWorld,
) -> Vec3 {
    let [bx, by, bz] = [cart.x, cart.y, cart.z].map(|v| v.floor() as i32);
    let half = f64::from(width.min(1.0) / 2.0);
    for dy in [0, 1, -1] {
        for (dx, dz) in dismount_offsets(face) {
            let p = [bx + dx, by + dy, bz + dz];
            let floor = world.floor_height(p);
            // DismountHelper.isBlockFloorValid.
            if floor.is_infinite() || floor >= 1.0 {
                continue;
            }
            // Vec3.upFromBottomCenterOf.
            let at = Vec3::new(
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + floor,
                f64::from(p[2]) + 0.5,
            );
            let bounds = Aabb::new(
                Vec3::new(at.x - half, at.y, at.z - half),
                Vec3::new(at.x + half, at.y + f64::from(height), at.z + half),
            );
            if world.can_dismount_to(bounds) {
                return at;
            }
        }
    }
    Vec3::new(cart.x, cart.y + HEIGHT, cart.z)
}

/// The level as a cart's dismount reads it: its floors come from
/// `DismountHelper.nonClimbableShape`, so a ladder, scaffolding, a vine or an
/// open trapdoor is no floor (a mount's own search reads the plain shapes).
struct CartDismountWorld<'a, 'g>(&'a Game<'g>);
impl DismountWorld for CartDismountWorld<'_, '_> {
    fn lava(&self, p: [i32; 3]) -> bool {
        super::super::mounts::GameDismountWorld(self.0).lava(p)
    }
    fn floor_height(&self, p: [i32; 3]) -> f64 {
        let level = self.0.stand_up_level();
        let shape = level.non_climbable_shape(p);
        let below = std::cell::OnceCell::new();
        horizon_core::geometry::block_floor_height(&shape, || {
            below.get_or_init(|| level.non_climbable_shape([p[0], p[1] - 1, p[2]]))
        })
    }
    fn can_dismount_to(&self, bounds: Aabb) -> bool {
        self.0.can_dismount_to(bounds)
    }
}

impl Game<'_> {
    /// Where cart `c` lets the mob at `rider` out
    /// (`AbstractMinecart.getDismountLocationForPassenger`).
    pub(in crate::game) fn cart_mob_dismount_point(&self, c: &Minecart, rider: usize) -> Vec3 {
        let m = &self.mobs[rider];
        let face = cart_motion_face(c.yaw, c.flipped, self.minecart_improvements());
        mob_dismount_location(
            c.position,
            face,
            m.width(),
            m.height(),
            &CartDismountWorld(self),
        )
    }
}
