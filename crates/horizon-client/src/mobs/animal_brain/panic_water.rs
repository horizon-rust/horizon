//! `AnimalPanic.getPanicPos` and `lookForWater`: a burning mob heads for the
//! nearest water within 5 blocks horizontally and 1 vertically before it
//! picks a random spot.

use super::{Mob, MobWorld};
use crate::blocks::Fluid;

pub(super) fn panic_pos(m: &mut Mob, w: &MobWorld, fly: bool) -> Option<[f64; 3]> {
    if m.fire_ticks > 0
        && let Some(p) = look_for_water(m, w)
    {
        return Some([
            f64::from(p[0]) + 0.5,
            f64::from(p[1]),
            f64::from(p[2]) + 0.5,
        ]);
    }
    if fly {
        let view = m.view_vector();
        m.air_and_water_random_pos(w, 5, 4, 0, view[0], view[2])
    } else {
        m.land_random_pos(w, 5, 4)
    }
}

/// `AnimalPanic.lookForWater`: `findBlocksInBoxByManhattanDistance(pos, 5, 1)`
/// in vanilla's depth order (positive z before its mirror), and a mob two
/// blocks wide needs the four cells of a 2x2 pool.
fn look_for_water(m: &Mob, w: &MobWorld) -> Option<[i32; 3]> {
    let (bx, by, bz) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    if w.collision_bounds([bx, by, bz]).is_some() {
        return None;
    }
    let wide = m.width().ceil() as i32 == 2;
    let water = |x: i32, y: i32, z: i32| w.fluid(x, y, z) == Fluid::Water;
    let fits = |x: i32, y: i32, z: i32| {
        water(x, y, z)
            && (!wide || (water(x, y, z + 1) && water(x + 1, y, z) && water(x + 1, y, z + 1)))
    };
    // reachX = reachZ = 5, reachY = 1, so the deepest cell is 11 away.
    for depth in 0i32..=11 {
        let max_x = depth.min(5);
        for x in -max_x..=max_x {
            let max_y = (depth - x.abs()).min(1);
            for y in -max_y..=max_y {
                let z = depth - x.abs() - y.abs();
                if z > 5 {
                    continue;
                }
                if fits(bx + x, by + y, bz + z) {
                    return Some([bx + x, by + y, bz + z]);
                }
                if z != 0 && fits(bx + x, by + y, bz - z) {
                    return Some([bx + x, by + y, bz - z]);
                }
            }
        }
    }
    None
}
