/// The sounds a player's own hurt posted, in order (`LivingEntity.makeSound`
/// through `Entity.playSound`, which leaves the player itself out).
fn posted_sounds(player: &mut crate::motion::PlayerMotion) -> Vec<&'static str> {
    std::mem::take(&mut player.events)
        .into_iter()
        .filter_map(|e| match e {
            crate::motion::MotionEvent::HurtSound(sound) => Some(sound),
            _ => None,
        })
        .collect()
}

/// `LivingEntity.hurtServer`: `playHurtSound(source)` runs whatever the hit's
/// `applyItemBlocking` stopped, so a shield that stops every point still
/// leaves the hurt sound, besides `BlocksAttacks.onBlocked`'s block sound.
#[test]
fn a_blocked_hit_still_plays_the_hurt_sound() {
    use horizon_world::blocking_rules::{BlockingRules, DamageReduction, ItemDamageFunction};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("movement-properties.nbt").exists() {
        return;
    }
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let mut player =
        crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO).unwrap();
    let (health, rules) = (&resources.health, super::HealthRules::default());
    let shield = super::Blocking {
        rules: BlockingRules {
            delay_seconds: 0.25,
            disable_scale: 1.0,
            reductions: vec![DamageReduction {
                horizontal_angle: 90.0,
                types: None,
                base: 0.0,
                factor: 1.0,
            }],
            item_damage: ItemDamageFunction {
                threshold: 3.0,
                base: 1.0,
                factor: 1.0,
            },
        },
        bypassed: false,
        pierce: false,
        angle: 0.0,
    };
    let id = health.damage.id("minecraft:mob_attack").unwrap();
    let o = player
        .hurt_caused_by(health, rules, id, 4.0, Some(shield), true, 0)
        .unwrap();
    assert!(o.fully_blocked && o.sound, "the shield's own block sound");
    assert_eq!(
        posted_sounds(&mut player),
        ["minecraft:entity.player.hurt"],
        "a fully blocked hit still plays the hurt sound"
    );
}

/// `LivingEntity.hurtServer` on the killing blow: `makeSound(getDeathSound())`
/// (no hurt sound), so the players around a dying player hear its death.
#[test]
fn a_dying_player_posts_its_death_sound_for_the_players_around() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("movement-properties.nbt").exists() {
        return;
    }
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let mut player =
        crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO).unwrap();
    let (health, rules) = (&resources.health, super::HealthRules::default());
    let id = health.damage.id("minecraft:mob_attack").unwrap();
    assert!(player.hurt(health, rules, id, 100.0).unwrap());
    assert!(player.health.dead_or_dying(), "the hit must kill");
    assert_eq!(
        posted_sounds(&mut player),
        ["minecraft:entity.player.death"],
        "Player.getDeathSound, heard by everyone but the dying player"
    );
}

/// `LivingEntity.handleDamageEvent` and `handleEntityEvent` 3: the hurt and
/// death sounds' pitch is `(nextFloat() - nextFloat()) * 0.2F + 1.0F`, drawn
/// twice from the entity's own random, so twenty hits do not sound alike.
#[test]
fn the_hurt_and_death_sound_pitch_comes_from_the_entity_random() {
    use horizon_core::random::{EntityRandom, LegacyRandom, RandomSource};
    let mut random = EntityRandom::own(LegacyRandom::new(7));
    let mut fresh = LegacyRandom::new(7);
    let (a, b) = (fresh.next_float(), fresh.next_float());
    assert_eq!(super::sound_pitch(&mut random), (a - b) * 0.2 + 1.0);
    let pitches: std::collections::HashSet<u32> = (0..20)
        .map(|_| {
            let pitch = super::sound_pitch(&mut random);
            assert!((0.8..=1.2).contains(&pitch), "{pitch}");
            pitch.to_bits()
        })
        .collect();
    assert!(pitches.len() > 1, "twenty hits must not sound alike");
}
