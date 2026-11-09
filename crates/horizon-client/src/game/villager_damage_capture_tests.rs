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
    assert_eq!(game.damage_attacker_uuid(None, Some(7)), None);
}
