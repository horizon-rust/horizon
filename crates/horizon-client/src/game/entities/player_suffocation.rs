//! `Entity.isInWall` for a player, and the suffocation predicate the nudge
//! reads. Both ask `BlockState.isSuffocating(level, pos)` and the block's
//! collision shape, not a full render cube: a closed shulker box and farmland,
//! dirt path, soul sand and mud suffocate a player standing in them, glass does
//! not.
use super::{Game, lan_actions::eye_columns};
use crate::collisions::SceneCollisions;
use horizon_client::blocks::BlockFacts;
use horizon_client::pose::Dimensions;
use horizon_client::world::World;
use horizon_core::geometry::{Aabb, ShapeOp, Vec3, VoxelShape};
use horizon_world::block_collision::BlockCollisionRegistry;
use horizon_world::registry::BlockStateId;
use horizon_world::suffocation::SuffocationRegistry;

/// The pieces of the game a suffocation query reads, so a caller that already
/// holds another field of `Game` can borrow only these (the movement replay).
pub(super) struct SuffocationWorld<'w, 'r> {
    pub world: &'w World,
    pub collision: &'w BlockCollisionRegistry<'r>,
    pub facts: &'w BlockFacts,
    pub registry: &'w SuffocationRegistry<'static>,
}

impl<'w, 'r> SuffocationWorld<'w, 'r> {
    pub(super) fn of(g: &'w Game<'r>) -> Self {
        Self {
            world: &g.world,
            collision: g.collision,
            facts: g.res.facts(),
            registry: g.mob_suffocation(),
        }
    }
}

/// `BlockState.isSuffocating(level, pos)`: the predicates `Blocks` installs,
/// bound by the mobs' `SuffocationRegistry`: `NOT_CLOSED_SHULKER` asks the
/// box's block entity, and farmland, dirt path, soul sand and mud are
/// `Blocks::always` although their collision shapes are not full cubes.
/// (`Player.freeAt` asks this and nothing else.)
pub(super) fn is_suffocating(g: &Game<'_>, state: BlockStateId, pos: [i32; 3]) -> bool {
    is_suffocating_in(&SuffocationWorld::of(g), state, pos)
}

fn is_suffocating_in(
    s: &SuffocationWorld<'_, '_>,
    state: BlockStateId,
    pos: [i32; 3],
) -> bool {
    let mut scene = SceneCollisions {
        world: s.world,
        binding: s.collision,
        subject: None,
    };
    s.registry
        .is_suffocating(&mut scene, state, pos)
        .unwrap_or(false)
}

/// Whether the block at `pos` suffocates and its collision shape, moved to
/// `pos`, meets `area` (level space): `Shapes.joinIsNotEmpty(shape, area, AND)`.
/// The nudge's full-cube shortcut applies only to cells its column overlaps.
pub(super) fn suffocates(
    s: &SuffocationWorld<'_, '_>,
    state: BlockStateId,
    pos: [i32; 3],
    area: Aabb,
    nudge: bool,
) -> bool {
    if !is_suffocating_in(s, state, pos) {
        return false;
    }
    if nudge && s.facts.get(state).full_collision {
        return true;
    }
    let mut scene = SceneCollisions {
        world: s.world,
        binding: s.collision,
        subject: None,
    };
    s.collision
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
#[cfg(test)]
pub(super) fn in_wall(g: &Game<'_>, at: [f64; 3], body: Dimensions) -> bool {
    in_wall_excluding(g, at, body, |_| false)
}

/// `in_wall` with a block the caller leaves out: a guest's collision exclusions
/// (arismoko/horizon#3717) are no wall.
pub(super) fn in_wall_excluding(
    g: &Game<'_>,
    at: [f64; 3],
    body: Dimensions,
    spared: impl Fn([i32; 3]) -> bool,
) -> bool {
    let s = SuffocationWorld::of(g);
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
            (y0..=y1).any(|y| {
                !spared([x, y, z])
                    && suffocates(&s, s.world.block(x, y, z), [x, y, z], area, false)
            })
        })
    })
}
