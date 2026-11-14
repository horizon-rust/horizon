//! A wolf's body armour: a hit the armour does not take still wears it
//! (`Wolf.hurtArmor`), and a piece that breaks goes out as the body slot's
//! break event, which each client answers with `LivingEntity.breakItem`.
use super::*;
use crate::mobs::Who;
use crate::mobs::sensor_scan_tests::kind;
use horizon_core::geometry::Vec3;
use std::sync::Arc;

fn wolf() -> Mob {
    let mut kind = kind();
    let k = Arc::make_mut(&mut kind);
    k.id = "minecraft:wolf".into();
    k.species = Species::Wolf;
    let mut m = Mob::new(7, kind, Vec3::ZERO, 0.0);
    m.extra.body = Some("minecraft:wolf_armor".into());
    m
}

fn thorns() -> Damage {
    Damage {
        kind: DamageKind::Thorns,
        attacker: Some(Who::Player),
        from: None,
        attacker_species: None,
    }
}

/// The wear `LivingEntity.doHurtEquipment` owes, before it divides it.
fn owed(m: &Mob) -> Option<f32> {
    m.outbox.iter().find_map(|e| match e {
        MobEvent::ArmorHurt { amount, .. } => Some(*amount),
        _ => None,
    })
}

/// `Wolf.actuallyHurt` takes a `#bypasses_wolf_armor` hit itself, but
/// `LivingEntity.actuallyHurt` still asks `getDamageAfterArmorAbsorb`, and
/// `Wolf.hurtArmor` hands `doHurtEquipment` the body slot: 24 thorns wear the
/// piece by `max(1, 24 / 4) = 6`, and the wolf keeps the hit on its health.
/// A hit the armour does take skips that: `Wolf.actuallyHurt` wears it by
/// `ceil(damage)` itself.
#[test]
fn a_thorns_hit_wears_the_body_piece_it_does_not_absorb() {
    let mut m = wolf();
    assert!(m.damage(24.0, thorns()), "the hit landed");
    assert_eq!(owed(&m), Some(24.0), "the wear doHurtEquipment owes");
    assert_eq!(m.extra.body_damage, 0, "the armour absorbed nothing");
    assert!(m.health < 40.0, "the wolf took the hit: {}", m.health);

    let mut m = wolf();
    assert!(m.damage(24.0, Damage::of(DamageKind::MobAttack)));
    assert_eq!(m.extra.body_damage, 24, "ceil(24) on the armour itself");
    assert_eq!(
        owed(&m),
        None,
        "the armour took it, so hurtArmor is not asked"
    );
}

/// `ItemStack.hurtAndBreak` at the piece's maximum calls
/// `LivingEntity.onEquippedItemBroken`, which broadcasts
/// `entityEventForEquipmentBreak(BODY)` — 65 — instead of a raw break sound:
/// each client's `breakItem` plays it at 0.8 volume and 0.8–1.2 pitch, gated
/// on silence, and throws five crumbs.
#[test]
fn a_broken_body_piece_sends_the_body_break_event() {
    let mut m = wolf();
    m.extra.body_damage = 63;
    assert!(
        m.damage(1.0, Damage::of(DamageKind::MobAttack)),
        "the hit landed"
    );
    assert_eq!(m.extra.body, None, "the piece broke");
    assert_eq!(m.extra.body_damage, 0);
    let events: Vec<_> = m
        .outbox
        .iter()
        .filter(|e| {
            matches!(
                e,
                MobEvent::EntityEvent {
                    mob: 7,
                    event: 65,
                    at: [0.0, 0.0, 0.0],
                }
            )
        })
        .collect();
    assert_eq!(
        events.len(),
        1,
        "the body slot's break event: {:?}",
        m.outbox
    );
    assert!(
        !m.outbox
            .iter()
            .any(|e| matches!(e, MobEvent::Sound("item.wolf_armor.break", ..))),
        "no raw break sound: {:?}",
        m.outbox
    );
}
