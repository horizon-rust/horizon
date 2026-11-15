//! Panic after damage and the water a burning mob runs to: `Fox.FoxPanicGoal`
//! never starts while the fox defends, `PanicGoal.shouldPanic` only reads the
//! last damage source (fire only picks the water), and `AnimalPanic` looks
//! for water before it picks a random spot.
use super::*;
use crate::mobs::world::with_built_test_world;
use crate::mobs::{Damage, DamageKind, animal_brain, farm, panda, polar_bear};
use std::sync::Arc;

/// A stone floor at y = 63 under chunks -1..=1, so a random retreat spot
/// exists.
fn floor(world: &mut crate::world::World, registry: &horizon_world::registry::BlockRegistry) {
    for (cx, cz) in [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 0), (0, 1)] {
        world
            .chunks
            .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
    }
    let stone = registry.default_state("minecraft:stone").unwrap();
    for x in -16..16 {
        for z in -16..16 {
            world.set(x, 63, z, stone).unwrap();
        }
    }
}

/// `Fox.FoxPanicGoal.shouldPanic`: `!isDefending() && super.shouldPanic()`.
#[test]
fn a_defending_fox_does_not_panic() {
    let ran = with_built_test_world(&[], floor, |w, types| {
        let kind = Arc::new(types.types["minecraft:fox"].clone());
        let mut fox = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        fox.on_ground = true;
        register(&mut fox);
        set_flag(&mut fox, DEFENDING, true);
        // A hit that would panic a fox that was not defending.
        fox.last_damage = Some((Damage::of(DamageKind::MobAttack), i64::from(fox.age)));
        let mut lr = crate::mobs::world::level_random(0);
        let mut events = Vec::new();
        let mut cx = Cx {
            w,
            lr: &mut lr,
            events: &mut events,
        };
        let mut goals = std::mem::take(&mut fox.ai.goals);
        goals.tick(&mut fox, &mut cx);
        assert!(
            !goals.is_running("Panic"),
            "a defending fox keeps attacking"
        );
        assert!(!fox.ai.panicking);
        fox.ai.goals = goals;
    });
    let _ = ran;
}

/// `PanicGoal.shouldPanic` reads only `getLastDamageSource`; a mob that is
/// merely on fire does not panic (`isOnFire` only picks the water).
#[test]
fn a_burning_cow_without_damage_does_not_panic() {
    let ran = with_built_test_world(&[], floor, |w, types| {
        let kind = Arc::new(types.types["minecraft:cow"].clone());
        let mut cow = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        cow.on_ground = true;
        farm::register(&mut cow);
        cow.fire_ticks = 400;
        let mut lr = crate::mobs::world::level_random(0);
        let mut events = Vec::new();
        let mut cx = Cx {
            w,
            lr: &mut lr,
            events: &mut events,
        };
        let mut goals = std::mem::take(&mut cow.ai.goals);
        goals.tick(&mut cow, &mut cx);
        assert!(
            !goals.is_running("Panic"),
            "fire alone must not start a panic"
        );
        assert!(!cow.ai.panicking);
        cow.ai.goals = goals;
    });
    let _ = ran;
}

/// `Panda.PandaPanicGoal` and the polar bear's `PanicGoal` panic at
/// `#panic_environmental_causes` only: `isOnFire` picks the water, it does not
/// start the panic.
#[test]
fn a_burning_panda_and_polar_bear_hit_by_a_mob_do_not_panic() {
    let ran = with_built_test_world(&[], floor, |w, types| {
        for (name, register) in [
            ("minecraft:panda", panda::register as fn(&mut Mob)),
            ("minecraft:polar_bear", polar_bear::register),
        ] {
            let kind = Arc::new(types.types[name].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.on_ground = true;
            register(&mut m);
            m.fire_ticks = 20;
            // `mob_attack` is in `#panic_causes`, not in
            // `#panic_environmental_causes`.
            m.last_damage = Some((Damage::of(DamageKind::MobAttack), i64::from(m.age)));
            let mut lr = crate::mobs::world::level_random(0);
            let mut events = Vec::new();
            let mut cx = Cx {
                w,
                lr: &mut lr,
                events: &mut events,
            };
            let mut goals = std::mem::take(&mut m.ai.goals);
            goals.tick(&mut m, &mut cx);
            assert!(
                !m.ai.panicking,
                "{name} must not panic at a mob hit while burning"
            );
            m.ai.goals = goals;
        }
    });
    let _ = ran;
}

/// `AnimalPanic.lookForWater`: a hurt burning mob walks to water within 5
/// blocks before it picks a random spot. The two pools are both two blocks
/// away; vanilla's depth order visits the positive-z one first.
#[test]
fn a_burning_panicking_mob_walks_to_water_first() {
    let ran = with_built_test_world(
        &[],
        |world, registry| {
            floor(world, registry);
            let water = registry.default_state("minecraft:water").unwrap();
            world.set(0, 64, 2, water).unwrap();
            world.set(2, 64, 0, water).unwrap();
        },
        |w, types| {
            let kind = Arc::new(types.types["minecraft:copper_golem"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.on_ground = true;
            m.fire_ticks = 20;
            m.last_damage = Some((Damage::of(DamageKind::MobAttack), i64::from(m.age)));
            let mut lr = crate::mobs::world::level_random(0);
            let mut events = Vec::new();
            let mut target = None;
            let mut panicked = false;
            for _ in 0..40 {
                animal_brain::tick(&mut m, w, &mut lr, &mut events);
                panicked |= animal_brain::is_panicking(&m);
                if panicked
                    && let Some(brain) = &m.extra.animal_brain
                    && let Some(walk) = brain.mem.walk_target.value()
                {
                    target = Some(walk.target);
                    break;
                }
            }
            assert!(panicked, "the hurt copper golem panics");
            assert_eq!(
                target,
                Some(animal_brain::Tracker::Block([0, 64, 2])),
                "the burning mob heads for the water, not a random spot"
            );
        },
    );
    let _ = ran;
}
