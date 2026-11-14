//! `AbstractHorse.executeRidersJump` launches a ridden mount with
//! `LivingEntity.getJumpPower(amount)`: jump strength times the amount, times
//! `Entity.getBlockJumpFactor`, plus `getJumpBoostPower`. The camel's dash and
//! the nautilus's also carry `Entity.getBlockSpeedFactor`. Honey (jump factor
//! 0.5, speed factor 0.4) and Jump Boost therefore move what a rider feels,
//! and the horse's own forward push keeps its size on honey.
use super::*;
use crate::mobs::world::with_built_test_world;
use std::sync::Arc;

/// A stone floor over x, z in 0..8, with a honey block at (4, 0, 4).
fn build(world: &mut crate::world::World, registry: &horizon_world::registry::BlockRegistry) {
    world
        .chunks
        .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
    let stone = registry.default_state("minecraft:stone").unwrap();
    let honey = registry.default_state("minecraft:honey_block").unwrap();
    for x in 0..8 {
        for z in 0..8 {
            world.set(x, 0, z, stone).unwrap();
        }
    }
    world.set(4, 0, 4, honey).unwrap();
}

/// A saddled mount standing on the floor at `at`, with a rider whose released
/// jump bar is `amount` (the vanilla scale runs 0.4 to 1.0).
fn mounted(types: &crate::mobs::MobTypes, kind: &str, at: Vec3, amount: f32) -> Mob {
    let mut m = Mob::new(1, Arc::new(types.types[kind].clone()), at, 0.0);
    m.on_ground = true;
    m.extra.saddled = true;
    m.rider = Some(Rider {
        forward: 1.0,
        jump: amount,
        ..Default::default()
    });
    m
}

#[test]
fn a_ridden_jump_reads_the_blocks_under_it_and_jump_boost() {
    let ran = with_built_test_world(&[], build, |w, types| {
        let close = |a: f64, b: f64| (a - b).abs() < 1.0e-6;
        let stone = Vec3::new(0.5, 1.0, 0.5);
        let honey = Vec3::new(4.5, 1.0, 4.5);
        // The blocks really carry those coefficients (honey: speed 0.4,
        // jump 0.5), so the expectations below are the vanilla ones.
        let probe = mounted(types, "minecraft:horse", honey, 0.0);
        assert_eq!(probe.block_jump_factor(w), 0.5);
        assert_eq!(probe.block_speed_factor(w), 0.4);
        assert_eq!(
            mounted(types, "minecraft:horse", stone, 0.0).block_jump_factor(w),
            1.0
        );

        // A horse: getJumpPower(0.5) up, and 0.4 * amount forward along the
        // yaw (the rider looks at +z). Jump Boost 3 adds 0.1 * 3.
        let horse = |at: Vec3, boost: Option<i32>| {
            let mut m = mounted(types, "minecraft:horse", at, 0.5);
            m.attrs.jump_strength = 0.7;
            if let Some(amplifier) = boost {
                m.extra
                    .effects
                    .push(("minecraft:jump_boost", 200, amplifier));
            }
            ridden_step(&mut m, Some(w));
            m
        };
        let on_stone = horse(stone, None);
        assert!(
            close(on_stone.velocity.y, f64::from(0.7_f32 * 0.5)),
            "0.7 jump strength, half the bar: {}",
            on_stone.velocity.y
        );
        assert!(
            close(on_stone.velocity.z, f64::from(0.4_f32 * 0.5)),
            "the forward push: {}",
            on_stone.velocity.z
        );
        // Honey halves the launch, and leaves the forward push alone.
        let on_honey = horse(honey, None);
        assert!(
            close(on_honey.velocity.y, f64::from(0.7_f32 * 0.5 * 0.5)),
            "honey halves the jump: {}",
            on_honey.velocity.y
        );
        assert!(
            close(on_honey.velocity.z, f64::from(0.4_f32 * 0.5)),
            "honey does not slow the horse's push: {}",
            on_honey.velocity.z
        );
        let boosted = horse(stone, Some(2));
        assert!(
            close(boosted.velocity.y, f64::from(0.7_f32 * 0.5 + 0.3)),
            "Jump Boost 3 adds 0.3: {}",
            boosted.velocity.y
        );

        // A camel's dash: 22.2222 * amount * speed * the block's speed factor
        // forward, and 1.4285 * amount * getJumpPower() up (0.42 here, half on
        // honey, plus Jump Boost).
        let camel = |at: Vec3, boost: Option<i32>| {
            let mut m = mounted(types, "minecraft:camel", at, 1.0);
            m.attrs.speed = 0.1;
            m.attrs.jump_strength = 0.42;
            if let Some(amplifier) = boost {
                m.extra
                    .effects
                    .push(("minecraft:jump_boost", 200, amplifier));
            }
            ridden_step(&mut m, Some(w));
            m
        };
        let dash = f64::from(22.2222_f32) * 0.1;
        // 1.4285F * amount * jumpMomentum is a float product in vanilla.
        let lift = |jump_power: f32| f64::from(1.4285_f32 * jump_power);
        let on_stone = camel(stone, None);
        assert!(
            close(on_stone.velocity.z, dash) && on_stone.velocity.x == 0.0,
            "the dash along +z: {:?}",
            on_stone.velocity
        );
        assert!(
            close(on_stone.velocity.y, lift(0.42_f32)),
            "the dash's lift: {}",
            on_stone.velocity.y
        );
        let on_honey = camel(honey, None);
        assert!(
            close(on_honey.velocity.z, dash * 0.4),
            "honey slows the dash to 0.4: {}",
            on_honey.velocity.z
        );
        assert!(
            close(on_honey.velocity.y, lift(0.42_f32 * 0.5)),
            "honey halves the dash's lift: {}",
            on_honey.velocity.y
        );
        let boosted = camel(stone, Some(2));
        assert!(
            close(boosted.velocity.y, lift(0.42_f32 + 0.3)),
            "Jump Boost 3 lifts the dash: {}",
            boosted.velocity.y
        );

        // A nautilus's dash: 0.5 * amount * speed * the block's speed factor
        // along the rider's look (no lift, the look is level).
        let nautilus = |at: Vec3| {
            let mut m = mounted(types, "minecraft:nautilus", at, 1.0);
            m.attrs.speed = 0.1;
            m.rider = Some(Rider {
                yaw: 0.0,
                pitch: 0.0,
                jump: 1.0,
                ..Default::default()
            });
            ridden_step(&mut m, Some(w));
            m
        };
        let dash = f64::from(0.5_f32) * 0.1;
        let on_stone = nautilus(stone);
        assert!(
            close(on_stone.velocity.z, dash) && on_stone.velocity.y == 0.0,
            "the dash along +z: {:?}",
            on_stone.velocity
        );
        let on_honey = nautilus(honey);
        assert!(
            close(on_honey.velocity.z, dash * 0.4),
            "honey slows the nautilus to 0.4: {}",
            on_honey.velocity.z
        );
    });
    let _ = ran;
}

/// A saved horse's jump strength is an attribute base (`AttributeMap.load`),
/// so a modifier saved with it is part of the jump its rider feels. The base
/// also stays the one its rider's client syncs (`vector[0]`).
#[test]
fn a_loaded_horses_jump_strength_reaches_its_launch() {
    let ran = with_built_test_world(&[], build, |w, types| {
        let tag = horizon_world::nbt::parse_snbt(
            r#"{id:"minecraft:horse",Pos:[0.5d,1d,0.5d],attributes:[{id:"minecraft:jump_strength",base:0.5d,modifiers:[{id:"t:add",amount:0.4d,operation:"add_value"}]}]}"#,
        )
        .unwrap()
        .0;
        let kinds = std::collections::HashMap::from([(
            "minecraft:horse".to_owned(),
            Arc::new(types.types["minecraft:horse"].clone()),
        )]);
        let mut m = crate::entity_nbt::mob_from_nbt(&tag, &kinds, 7, None).expect("a horse");
        assert!((m.extra.vector[0] - 0.5).abs() < 1.0e-9, "the synced base");
        assert!(
            (m.attrs.jump_strength - 0.9).abs() < 1.0e-9,
            "the loaded attribute: {}",
            m.attrs.jump_strength
        );
        m.on_ground = true;
        m.extra.saddled = true;
        m.rider = Some(Rider {
            forward: 1.0,
            jump: 1.0,
            ..Default::default()
        });
        ridden_step(&mut m, Some(w));
        assert!(
            (m.velocity.y - 0.9).abs() < 1.0e-6,
            "the launch: {}",
            m.velocity.y
        );
    });
    let _ = ran;
}
