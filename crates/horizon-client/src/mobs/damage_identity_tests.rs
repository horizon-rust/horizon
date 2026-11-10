use super::*;

fn victim() -> Mob {
    Mob::new(91, sensor_scan_tests::kind(), Vec3::ZERO, 0.0)
}
#[test]
fn accepted_damage_retains_the_verified_causing_identity() {
    let a = [1, 2, 3, 4];
    let b = [5, 6, 7, 8];
    let mut m = victim();
    assert!(m.damage_with_player_uuid(1.0, Damage::player([0.0; 3]), a, None));
    assert_eq!(m.last_damage.unwrap().0.attacker_uuid, Some(a));
    println!(
        "identity player {}",
        serde_json::to_string(&m.last_damage.unwrap().0.attacker_uuid).unwrap()
    );
    assert!(!m.damage_with_player_uuid(1.0, Damage::player([0.0; 3]), b, None));
    assert_eq!(
        m.last_damage.unwrap().0.attacker_uuid,
        Some(a),
        "rejected hit retains the accepted source"
    );
    assert!(m.damage_with_player_uuid(2.0, Damage::player([0.0; 3]), b, None));
    assert_eq!(m.last_damage.unwrap().0.attacker_uuid, Some(b));
}
#[test]
fn deferred_identity_survives_without_fabricating_nonplayer_context() {
    let a = [1, 2, 3, 4];
    let b = [5, 6, 7, 8];
    let mut m = victim();
    let source = Damage::indirect_magic(Some(Who::Mob(19)), [0.0; 3]).with_attacker_uuid(Some(a));
    assert!(m.damage_with_player_uuid(1.0, source, b, None));
    assert_eq!(m.last_damage.unwrap().0.attacker_uuid, Some(a));
    assert_eq!(m.last_damage.unwrap().0.attacker, Some(Who::Mob(19)));
    println!(
        "identity npc {}",
        serde_json::to_string(&m.last_damage.unwrap().0.attacker_uuid).unwrap()
    );
    assert!(m.damage_with_player_uuid(2.0, Damage::of(DamageKind::Generic), b, None));
    assert_eq!(m.last_damage.unwrap().0.attacker_uuid, None);
    println!(
        "identity ownerless {}",
        serde_json::to_string(&m.last_damage.unwrap().0.attacker_uuid).unwrap()
    );
    let deferred = Damage::indirect_magic(Some(Who::Player), [0.0; 3]).with_attacker_uuid(Some(a));
    assert!(m.damage_by(3.0, deferred, "minecraft:splash_potion"));
    assert_eq!(m.last_damage.unwrap().0.attacker_uuid, Some(a));
    assert_eq!(m.last_damage.unwrap().0.attacker, Some(Who::Player));
}
