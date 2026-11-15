//! `Entity.isInWall` and `LocalPlayer.moveTowardsClosestSpace` for a player.
//! Both ask `BlockState.isSuffocating(level, pos)` and the block's collision
//! shape, not a full render cube: a closed shulker box and farmland, dirt path,
//! soul sand and mud suffocate a player standing in them, glass does not.
use super::{Game, PlayerMotion, lan_actions::eye_columns};
use crate::collisions::SceneCollisions;
use horizon_client::pose::Dimensions;
use horizon_core::geometry::{Aabb, Axis, ShapeOp, Vec3, VoxelShape};
use horizon_world::registry::BlockStateId;

/// `BlockState.isSuffocating(level, pos)`: the predicates `Blocks` installs,
/// bound by the mobs' `SuffocationRegistry`: `NOT_CLOSED_SHULKER` asks the
/// box's block entity, and farmland, dirt path, soul sand and mud are
/// `Blocks::always` although their collision shapes are not full cubes.
/// (`Player.freeAt` asks this and nothing else.)
pub(super) fn is_suffocating(g: &Game<'_>, state: BlockStateId, pos: [i32; 3]) -> bool {
    let mut scene = SceneCollisions {
        world: &g.world,
        binding: g.collision,
        subject: None,
    };
    g.mob_suffocation()
        .is_suffocating(&mut scene, state, pos)
        .unwrap_or(false)
}

/// Whether the block at `pos` suffocates and its collision shape, moved to
/// `pos`, meets `area` (level space): `Shapes.joinIsNotEmpty(shape, area, AND)`.
/// The nudge's full-cube shortcut applies only to cells its column overlaps.
fn suffocates(g: &Game<'_>, state: BlockStateId, pos: [i32; 3], area: Aabb, nudge: bool) -> bool {
    if !is_suffocating(g, state, pos) {
        return false;
    }
    if nudge && g.res.facts().get(state).full_collision {
        return true;
    }
    let mut scene = SceneCollisions {
        world: &g.world,
        binding: g.collision,
        subject: None,
    };
    g.collision
        .collision_shape(&mut scene, state, pos)
        .is_ok_and(|shape| {
            let shape = shape.moved(Vec3::new(
                f64::from(pos[0]),
                f64::from(pos[1]),
                f64::from(pos[2]),
            ));
            VoxelShape::from_box(area).is_ok_and(|area| {
                shape
                    .join_is_not_empty(&area, ShapeOp::AND)
                    .unwrap_or(false)
            })
        })
}

/// `Entity.isInWall`: a suffocating block whose collision shape meets the box
/// at the eyes, 0.8 of the body wide and 1e-6 tall (`LivingEntity.isInWall`
/// leaves out a sleeping entity at the call site).
pub(super) fn in_wall(g: &Game<'_>, at: [f64; 3], body: Dimensions) -> bool {
    let eye = [at[0], at[1] + f64::from(body.eye_height), at[2]];
    let half = f64::from(body.width * 0.8) / 2.0;
    let area = Aabb::new(
        Vec3::new(eye[0] - half, eye[1] - 5.0e-7, eye[2] - half),
        Vec3::new(eye[0] + half, eye[1] + 5.0e-7, eye[2] + half),
    );
    // `BlockPos.betweenClosedStream` of the eye box: the cell below the eye
    // joins when the eye is within 5e-7 above its top.
    let (y0, y1) = (
        (eye[1] - 5.0e-7).floor() as i32,
        (eye[1] + 5.0e-7).floor() as i32,
    );
    let [xs, zs] = eye_columns(at, body);
    xs.into_iter().any(|x| {
        zs.clone().any(|z| {
            (y0..=y1).any(|y| suffocates(g, g.world.block(x, y, z), [x, y, z], area, false))
        })
    })
}

/// `LocalPlayer.moveTowardsClosestSpace` at the body's four corners (aiStep,
/// unless a spectator or a passenger): a corner inside a suffocating block
/// sets a 0.1 speed towards the nearest side that is free, on that side's
/// axis; a later corner overwrites an earlier one.
pub(super) fn closest_space_nudge(p: &PlayerMotion, g: &Game<'_>) -> [Option<f64>; 2] {
    let mut out = [None; 2];
    let body = p.bounds();
    let (min_y, max_y) = (body.min(Axis::Y) + 1.0e-7, body.max(Axis::Y) - 1.0e-7);
    // `suffocatesAt`: `collidesWithSuffocatingBlock` of the cell's column,
    // deflated by 1e-7.
    let suffocates_at = |x: i32, z: i32| {
        ((min_y.floor() as i32 - 1)..=(max_y.ceil() as i32 + 1)).any(|y| {
            f64::from(y) < max_y && f64::from(y + 1) > min_y && {
                let area = Aabb::new(
                    Vec3::new(f64::from(x) + 1.0e-7, min_y, f64::from(z) + 1.0e-7),
                    Vec3::new(
                        f64::from(x) + 1.0 - 1.0e-7,
                        max_y,
                        f64::from(z) + 1.0 - 1.0e-7,
                    ),
                );
                suffocates(g, g.world.block(x, y, z), [x, y, z], area, true)
            }
        })
    };
    let side = f64::from(p.dimensions.width) * 0.35;
    let (px, pz) = (p.position.x, p.position.z);
    for (x, z) in [
        (px - side, pz + side),
        (px - side, pz - side),
        (px + side, pz - side),
        (px + side, pz + side),
    ] {
        let (bx, bz) = (x.floor() as i32, z.floor() as i32);
        if !suffocates_at(bx, bz) {
            continue;
        }
        let (xd, zd) = (x - f64::from(bx), z - f64::from(bz));
        // WEST, EAST, NORTH, SOUTH: (axis, step, distance to that edge).
        let mut closest = 1.797_693_134_862_315_7e308; // Double.MAX_VALUE
        let mut chosen = None;
        for (axis, step, distance) in [(0, -1, xd), (0, 1, 1.0 - xd), (1, -1, zd), (1, 1, 1.0 - zd)]
        {
            let (nx, nz) = if axis == 0 {
                (bx + step, bz)
            } else {
                (bx, bz + step)
            };
            if distance < closest && !suffocates_at(nx, nz) {
                closest = distance;
                chosen = Some((axis, step));
            }
        }
        if let Some((axis, step)) = chosen {
            out[axis] = Some(0.1 * f64::from(step));
        }
    }
    out
}
