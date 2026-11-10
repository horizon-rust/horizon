use super::*;
use horizon_client::mobs::{Damage, DamageKind, Who};

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
