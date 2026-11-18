//! `move_collision::check_move` builds the old box's shape when a block meets the new box, no longer
//! before the scan: the same answer for every move, over a scene of full blocks, slabs, stairs, fences,
//! snow layers, water and air.
use super::*;
use crate::collisions::SceneCollisions;
use horizon_core::geometry::{Aabb, ShapeOp, Vec3, VoxelShape};
use horizon_world::{
    block_collision_scan::{BlockCollisionScan, BlockCollisionWorld},
    collision_context::CollisionContext,
    entity_obstruction::EntityIdentity,
    world_movement::{
        CollisionCheckRequest, EntityMovementWorld, MovementRequest, no_world_collision,
        resolve_world_movement,
    },
};
use std::io;

/// `check_move` as it was: the old box's shape is built before the scan.
fn eager_check_move<W>(
    g: &Guest,
    to: [f64; 3],
    requested: Vec3,
    movement: Vec3,
    overridden: bool,
    world: &mut W,
) -> (bool, bool, bool)
where
    W: BlockCollisionWorld<Error = io::Error> + EntityMovementWorld<Error = io::Error>,
{
    // getBoundingBox: the box of the body's pose (swimming and crawling
    // are 0.6 tall), so a swimmer under a ceiling isn't sent back.
    let d = g.dimensions();
    let (half, height) = (f64::from(d.width) / 2.0, f64::from(d.height));
    let at = |p: [f64; 3]| {
        Aabb::new(
            Vec3::new(p[0] - half, p[1], p[2] - half),
            Vec3::new(p[0] + half, p[1] + height, p[2] + half),
        )
    };
    let (old, new) = (at(g.position), at(to));
    // player.move, then how far from the target it ended (vanilla drops
    // the vertical part: `yDist > -0.5 || yDist < 0.5` always holds).
    // The mover's own sneaking, as CollisionContext.of(player): a sneaking
    // player descends through scaffolding.
    let context = CollisionContext::position(g.position[1]).with_descending(g.sneaking);
    let done = resolve_world_movement(
        world,
        MovementRequest {
            entity: EntityIdentity(0),
            context,
            bounds: old,
            movement,
            max_step: 0.6,
            on_ground: g.on_ground,
            candidate_budget: 1_000_000,
        },
    )
    .ok();
    // The gate decides what actually moves: the requested delta, a listener's
    // replacement, or nothing (a cancel). «Moved wrongly» still measures the
    // **requested** move against what happened, as vanilla measures the
    // packet's target - a cancelled move that went nowhere is the full
    // discrepancy. A decision that is not the request fails whatever the
    // distance (arismoko/horizon#2414, ruling 5): a cancel or a replacement always sends the
    // guest a correction, so a walk's small step cannot slip under the
    // threshold and keep going. Vanilla's exemptions - a creative guest, and
    // the unobstructed-start check below - belong to the distance test alone
    // (review round 1): an overridden move is refused for a creative guest
    // and from an obstructed start too.
    let moved_wrongly = !g.creative()
        && done.is_some_and(|done| {
            let (dx, dz) = (requested.x - done.x, requested.z - done.z);
            dx * dx + dz * dz > 0.0625
        });
    let fail = overridden || moved_wrongly;
    // Entity.move: verticalCollisionBelow, the player "stands on something".
    let stands = done.is_some_and(|done| movement.y < 0.0 && done.y != movement.y);
    if fail {
        let unobstructed = no_world_collision(
            world,
            CollisionCheckRequest {
                entity: EntityIdentity(0),
                context: CollisionContext::position(old.min_position().y),
                bounds: old,
                candidate_budget: 1_000_000,
            },
        );
        // An overridden decision is refused from wherever it starts: the
        // unobstructed-old-box exemption is the distance test's, like
        // creative above (review round 1).
        if overridden || unobstructed.unwrap_or(true) {
            return (true, true, stands);
        }
    }
    // isEntityCollidingWithAnythingNew: a block the old box did not
    // already touch.
    let shrink = f64::from(1.0E-5_f32);
    let shrink = Vec3::new(shrink, shrink, shrink);
    let Ok(old_shape) = VoxelShape::from_box(old.deflate(shrink)) else {
        return (fail, false, stands);
    };
    let Ok(scan) = BlockCollisionScan::new(world, context, new.deflate(shrink), false, 1 << 20)
    else {
        return (fail, false, stands);
    };
    for hit in scan {
        let Ok(hit) = hit else { break };
        if !hit
            .shape
            .join_is_not_empty(&old_shape, ShapeOp::AND)
            .unwrap_or(true)
        {
            return (fail, true, stands);
        }
    }
    (fail, false, stands)
}

#[test]
#[ignore = "daily"]
fn the_old_boxs_shape_built_late_changes_no_answer() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let assets = &fixture.assets;
    let mut scene = World::test_scene(assets, &fixture.render, 2).unwrap();
    let names = [
        "minecraft:air",
        "minecraft:stone",
        "minecraft:oak_slab",
        "minecraft:oak_stairs",
        "minecraft:oak_fence",
        "minecraft:snow",
        "minecraft:water",
        "minecraft:cobblestone_wall",
        "minecraft:glass",
    ];
    let states: Vec<_> = names
        .iter()
        .map(|n| assets.registry.default_state(n).unwrap())
        .collect();
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    for x in 8..=40 {
        for z in 8..=40 {
            scene.set(x, 4, z, states[1]).unwrap();
            for y in 5..=9 {
                let pick = if y == 5 || next(6) == 0 { next(9) } else { 0 };
                scene.set(x, y, z, states[pick as usize]).unwrap();
            }
        }
    }
    let (mut hub, _rx) = hub_with_a_guest();
    let (mut moves, mut refused, mut touched) = (0, 0, 0);
    for round in 0..4000 {
        let at = [
            10.0 + next(2000) as f64 / 100.0 + 0.5,
            5.0 + next(300) as f64 / 100.0,
            10.0 + next(2000) as f64 / 100.0 + 0.5,
        ];
        let step = |n: &mut dyn FnMut(u64) -> u64| {
            let range = if round % 7 == 0 { 600 } else { 60 };
            (n(2 * range + 1) as f64 - range as f64) / 100.0
        };
        let to = [
            at[0] + step(&mut next),
            at[1] + step(&mut next) / 2.0,
            at[2] + step(&mut next),
        ];
        let on_ground = next(2) == 0;
        let sneaking = next(5) == 0;
        hub.guests.update(0, |g| {
            g.position = at;
            g.on_ground = on_ground;
            g.sneaking = sneaking;
        });
        let mut next_entity = 2;
        let border = crate::worldborder::WorldBorder::default();
        let ctx = TickCtx {
            sea_level: 63,
            world: &mut scene,
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [20.5, 5.0, 20.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: Some(&fixture.collision),
            border: &border,
            streaming: None,
        };
        let now = hub.collide(0, to, &ctx);
        let g = &hub.guests[0];
        let requested = Vec3::new(to[0] - at[0], to[1] - at[1], to[2] - at[2]);
        let mut world = SceneCollisions {
            world: &*ctx.world,
            binding: &fixture.collision,
            subject: None,
        };
        let before = eager_check_move(g, to, requested, requested, false, &mut world);
        assert_eq!(now, before, "round {round}: {at:?} -> {to:?}");
        moves += 1;
        refused += usize::from(before.1);
        touched += usize::from(before.0 || before.1);
    }
    assert!(
        moves == 4000 && refused > 100 && touched > 100,
        "{refused} {touched}"
    );
}
