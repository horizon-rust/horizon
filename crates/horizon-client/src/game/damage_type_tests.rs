use super::*;

#[test]
fn named_damage_keeps_its_registry_id_with_or_without_an_attacker() {
    let named = horizon_client::mobs::NamedDamage::new(
        "example:fan_fire",
        horizon_client::mobs::NamedDamage::IS_FIRE,
    );
    for attacker in [None, Some(Who::Mob(7)), Some(Who::Player)] {
        assert_eq!(
            source_damage_type(Damage {
                kind: DamageKind::Named(named),
                attacker,
                from: None,
                attacker_species: None,
            }),
            "example:fan_fire"
        );
    }
}

/// `/damage <target> <amount> <type>`: the source's type is the one asked
/// for, so `DamageSource.getMsgId` gives the type's own message key.
#[test]
fn the_damage_command_keeps_the_types_registry_id() {
    for ty in [
        "minecraft:arrow",
        "minecraft:trident",
        "minecraft:magic",
        "minecraft:dragon_breath",
    ] {
        assert_eq!(
            source_damage_type(Damage {
                kind: Game::damage_kind(ty),
                attacker: None,
                from: None,
                attacker_species: None,
            }),
            ty
        );
    }
}

/// `DamageCommand.damage` uses `#no_knockback` for `dragon_breath`;
/// `LivingEntity.hurtServer` still takes health without changing motion.
#[test]
fn dragon_breath_from_the_damage_command_leaves_motion_unchanged() {
    use horizon_client::mobs::{Anim, Brain, MobType, Part, Species};
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    let kind = Arc::new(MobType {
        id: "minecraft:cow".into(),
        species: Species::Cow,
        width: 0.9,
        height: 1.4,
        eye_height: 1.3,
        category: "creature".into(),
        max_health: 10.0,
        speed: 0.2,
        flying_speed: 0.02,
        attack_damage: 0.0,
        attack_knockback: 0.0,
        follow_range: 16.0,
        step_height: 0.6,
        armor: 20.0,
        armor_toughness: 0.0,
        knockback_resistance: 0.0,
        jump_strength: 0.42,
        attributes: Default::default(),
        item_stack_limits: Default::default(),
        gravity: 0.08,
        safe_fall_distance: 3.0,
        fall_damage_multiplier: 1.0,
        water_efficiency: 0.0,
        water_slowdown: 0.8,
        air_speed: 0.02,
        baby_scale: 0.5,
        head_rot_speed: 10.0,
        max_head_x_rot: 40.0,
        max_head_y_rot: 75.0,
        ambient_interval: 80,
        flags: Default::default(),
        model: Arc::new(Part {
            name: "root".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: Vec::new(),
            children: Vec::new(),
        }),
        layers: Default::default(),
        texture: String::new(),
        brain: Brain::Passive,
        anim: Anim::Humanoid { zombie_arms: false },
        sounds: [""; 3],
        step_sound: None,
        food: Default::default(),
        tempt: Default::default(),
    });
    let velocity = Vec3::new(0.6, 0.2, -0.4);
    for ty in ["dragon_breath", "minecraft:dragon_breath"] {
        for from in [None, Some([2.0, 0.0, 1.0])] {
            let mut mob = Mob::new(1, kind.clone(), Vec3::ZERO, 0.0);
            mob.velocity = velocity;
            assert!(mob.damage(
                2.0,
                Damage {
                    kind: Game::damage_kind(ty),
                    attacker: None,
                    from,
                    attacker_species: None,
                },
            ));
            assert_eq!(mob.health, 8.0, "the source still bypasses armor");
            assert_eq!(mob.velocity, velocity, "{ty}, from={from:?}");
        }
    }
    let mut control = Mob::new(2, kind, Vec3::ZERO, 0.0);
    control.velocity = velocity;
    assert!(control.damage(2.0, Damage::of(Game::damage_kind("minecraft:trident"))));
    assert_ne!(control.velocity, velocity, "ordinary hits still knock back");
}

/// `DamageSources.explosion`: a blast with a causing entity is
/// `player_explosion` whoever caused it, so an exploding creeper's too.
#[test]
fn a_creeper_blast_is_a_player_explosion() {
    let blast = |attacker| Damage {
        kind: DamageKind::Explosion,
        attacker,
        from: Some([0.0; 3]),
        attacker_species: None,
    };
    let creeper = blast(Some(Who::Mob(7)));
    assert_eq!(source_damage_type(creeper), "minecraft:player_explosion");
    // No causing entity.
    assert_eq!(source_damage_type(blast(None)), "minecraft:explosion");
    // The other kinds do not ask.
    let bite = Damage {
        kind: DamageKind::MobAttack,
        ..creeper
    };
    assert_eq!(source_damage_type(bite), "minecraft:mob_attack");
}
