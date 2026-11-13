use super::*;
use horizon_client::mobs::{Damage, DamageKind, Who};

#[test]
#[ignore = "requires explicit isolated prepared resources"]
fn guest_melee_keeps_identity_without_changing_the_owner_filter() {
    let mut game = crate::game::kill_die_vibration_tests::game();
    game.mobs.clear();
    game.level.player_uuid = Some([0, 0, 0, 101]);
    game.lan_acting = Some(7);
    game.lan_acting_uuid = Some(19);
    assert_eq!(game.owner_damage_uuid(), None);
    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
    game.mobs
        .push(Mob::new(92, kind, Vec3::new(2.0, 100.0, 1.0), 0.0));
    let source = game.attack_source([0.0, 100.0, 0.0]);
    assert_eq!(source.attacker_uuid, Some([0, 0, 0, 19]));
    assert!(game.hurt_by_player(0, 2.0, source));
    let accepted = game.mobs[0].last_damage.unwrap().0;
    game.lan_acting = None;
    game.lan_acting_uuid = None;
    assert_eq!(accepted.attacker_uuid, Some([0, 0, 0, 19]));
}

#[test]
#[ignore = "requires explicit isolated prepared resources"]
fn specialized_projectiles_keep_the_live_owner_identity() {
    use horizon_client::projectiles::{Hit, Projectile, ProjectileKind};
    let mut game = crate::game::kill_die_vibration_tests::game();
    game.mobs.clear();
    let target = game.res.mob_types().get("minecraft:pig").unwrap().clone();
    let owner = game
        .res
        .mob_types()
        .get("minecraft:breeze")
        .unwrap()
        .clone();
    game.mobs
        .push(Mob::new(92, target, Vec3::new(2.0, 100.0, 1.0), 0.0));
    game.mobs
        .push(Mob::new(19, owner, Vec3::new(1.0, 100.0, 1.0), 0.0));
    game.persist.uuids.insert(19, [0, 0, 0, 19]);
    game.level.player_uuid = Some([0, 0, 0, 101]);
    game.lan_acting = Some(7);
    game.lan_acting_uuid = Some(23);
    for (who, expected) in [(Who::Mob(19), [0, 0, 0, 19]), (Who::Player, [0, 0, 0, 23])] {
        for skull in [false, true] {
            game.mobs[0].health = 20.0;
            game.mobs[0].invulnerable = 0;
            let kind = if skull {
                ProjectileKind::WitherSkull { dangerous: false }
            } else {
                ProjectileKind::WindCharge
            };
            let mut projectile = Projectile::new(kind, Vec3::new(0.0, 100.0, 0.0), Some(who));
            projectile.guest = (who == Who::Player).then_some(7);
            if skull {
                // Keep its later blast away so this observes the direct skull hit.
                game.wither_skull_hit(&projectile, Some(Who::Mob(92)), [20.0, 100.0, 20.0])
                    .unwrap();
            } else {
                game.wind_charge_hit(
                    &projectile,
                    Hit::Entity {
                        who: Who::Mob(92),
                        at: Vec3::new(2.0, 100.0, 1.0),
                    },
                )
                .unwrap();
            }
            let accepted = game.mobs[0].last_damage.unwrap().0;
            assert_eq!(accepted.attacker, Some(who));
            assert_eq!(accepted.attacker_uuid, Some(expected));
            assert_eq!(
                accepted.kind,
                if skull {
                    DamageKind::WitherSkull
                } else {
                    DamageKind::Thrown
                }
            );
        }
    }
}

#[test]
#[ignore = "requires explicit isolated prepared resources"]
fn damage_keeps_the_actual_owner_after_that_actor_leaves() {
    let mut game = crate::game::kill_die_vibration_tests::game();
    game.mobs.clear();
    let owner = [0, 0, 0, 101];
    let attacker = [0, 0, 0, 19];
    game.level.player_uuid = Some(owner);
    assert_eq!(
        game.damage_attacker_uuid(Some(Who::Player), None),
        Some(owner)
    );
    let zombie = game
        .res
        .mob_types()
        .get("minecraft:zombie")
        .unwrap()
        .clone();
    let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
    game.mobs
        .push(Mob::new(19, zombie, Vec3::new(1.0, 100.0, 1.0), 0.0));
    game.mobs
        .push(Mob::new(92, pig, Vec3::new(2.0, 100.0, 1.0), 0.0));
    game.persist.uuids.insert(19, attacker);
    let source = Damage {
        attacker: Some(Who::Mob(19)),
        ..Damage::of(DamageKind::MobAttack)
    };
    assert!(game.mob_hits_mob(92, 1.0, source));
    let captured = game
        .mobs
        .iter()
        .find(|m| m.id == 92)
        .unwrap()
        .last_damage
        .unwrap()
        .0;
    assert_eq!(captured.attacker_uuid, Some(attacker));
    game.mobs.retain(|m| m.id != 19);
    game.forget_entity(19);
    assert_eq!(game.damage_attacker_uuid(Some(Who::Mob(19)), None), None);
    assert_eq!(captured.attacker_uuid, Some(attacker));
    println!(
        "identity game_npc {}",
        serde_json::to_string(&captured.attacker_uuid).unwrap()
    );
    assert_eq!(game.damage_attacker_uuid(None, Some(7)), None);
}

#[test]
#[ignore = "requires explicit isolated prepared resources"]
fn a_mob_owned_water_potion_keeps_its_damage_ownerless() {
    let mut game = crate::game::kill_die_vibration_tests::game();
    game.mobs.clear();
    let kind = game.res.mob_types().get("minecraft:blaze").unwrap().clone();
    assert!(kind.flags.water_sensitive);
    game.mobs
        .push(Mob::new(92, kind, Vec3::new(2.0, 100.0, 1.0), 0.0));
    let before = game.mobs[0].health;
    let water = horizon_client::items::ItemStack::new("minecraft:splash_potion", 1)
        .with_potion("minecraft:water");
    assert!(game.potion_in_tag(&water, "minecraft:hurts_water_sensitive_entities"));
    game.affect_entities_around(
        &water,
        Vec3::new(2.0, 100.0, 1.0),
        false,
        Some([0, 0, 0, 19]),
    );
    let hit = game.mobs[0]
        .last_damage
        .expect("water damage must actually be accepted")
        .0;
    assert!(game.mobs[0].health < before);
    assert_eq!(hit.kind, DamageKind::IndirectMagic);
    assert_eq!(hit.attacker, None);
    assert_eq!(hit.attacker_uuid, None);
}
