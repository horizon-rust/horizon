//! `Entity.push`: the shove a mob applies to whatever it walks into.
use super::{hurt, on_climbable, root_vehicle};
use crate::mobs::{Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who};
use horizon_core::geometry::Vec3;
use horizon_core::random::RandomSource;

/// LivingEntity.pushEntities: every pushable entity overlapping the box is
/// pushed apart from this one (`doPush(e)` = `e.push(this)`); pushes of other
/// mobs are `Push` events their tick picks up.
pub(super) fn push_entities(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    m.on_climbable = on_climbable(m, w);
    if m.species() == Species::Bat {
        return;
    }
    let bounds = m.bounds();
    // The pusher's own isPushable, checked by `e.push(this)` before moving it.
    let self_pushable = m.alive()
        && m.species() != Species::Shulker
        && !m.on_climbable
        && (m.species() != Species::Creaking || crate::mobs::creaking::can_move(m))
        && (m.species() != Species::Warden || !crate::mobs::warden::digging_or_emerging(m))
        && !crate::mobs::equine::is_vehicle(m);
    // Each with whether it rides a vehicle (`Entity.isPassenger`) and
    // whether it shares this one's root vehicle (`isPassengerOfSameVehicle`).
    let mut targets: Vec<(Who, Vec3, bool, bool, bool)> = Vec::new();
    if let Some(p) = &w.player
        // Entity.push: passengers of the same vehicle never push each other.
        && m.rider.is_none()
        && p.alive
        && !p.spectator
        && p.bounds.intersects(bounds)
    {
        targets.push((Who::Player, p.position, p.passenger, false, false));
    }
    let root = root_vehicle(w, m.id, m.extra.mount);
    for o in w.others_near(bounds) {
        if o.id != m.id && o.alive && o.pushable && o.bounds.intersects(bounds) {
            let same = root_vehicle(w, o.id, o.vehicle) == root;
            targets.push((Who::Mob(o.id), o.position, o.passenger, same, o.is_vehicle));
        }
    }
    // EntitySelector.pushableBy(this): the teams' collision rules.
    let own = w.teams.of(Who::Mob(m.id));
    targets.retain(|(who, ..)| crate::scoreboard::pushable_by(own, w.teams.of(*who), true));
    // Cramming: a quarter of the ticks spent with `maxEntityCramming` or more
    // others around that are not riding anything hurts it by 6.
    let limit = w.max_entity_cramming;
    if limit > 0
        && targets.len() as i64 > i64::from(limit) - 1
        && m.random.next_int_bound(4).unwrap_or(1) == 0
        && targets.iter().filter(|t| !t.2).count() as i64 > i64::from(limit) - 1
    {
        hurt(m, w, events, 6.0, Damage::of(DamageKind::Cramming));
    }
    for (who, at, _, same, vehicle) in targets {
        // Parrot.doPush: a player is counted for cramming but never pushed.
        if matches!(who, Who::Player) && m.species() == Species::Parrot {
            continue;
        }
        // Warden.doPush: touching it angers it, then the push itself.
        if m.species() == Species::Warden {
            crate::mobs::warden::do_push(m, w, who, events);
        }
        // IronGolem.doPush: a monster that is not a creeper becomes its target
        // one time in 20, and the draw is made for those alone, then the push.
        if m.species() == Species::IronGolem
            && let Who::Mob(id) = who
            && w.seen(id).is_some_and(crate::mobs::golem::golem_prey)
            && m.random.next_int_bound(20).unwrap_or(1) == 0
        {
            m.ai.target = Some(who);
        }
        // doPush calls the other living entity's push; sleeping vetoes both impulses.
        let sleeping = match who {
            Who::Player => w.player.as_ref().is_some_and(|p| p.sleeping),
            Who::Mob(id) => w.seen(id).is_some_and(|o| o.sleeping),
        };
        if sleeping {
            continue;
        }
        // Entity.push: passengers of the same vehicle (a rider and its mount
        // among them) never push each other.
        if same {
            continue;
        }
        // Entity.push(Entity) with `this` the other entity.
        let (mut xa, mut za) = (m.position.x - at.x, m.position.z - at.z);
        let mut dd = xa.abs().max(za.abs());
        if dd < f64::from(0.01_f32) {
            continue;
        }
        dd = dd.sqrt();
        xa /= dd;
        za /= dd;
        let pow = (1.0 / dd).min(1.0);
        xa *= pow;
        za *= pow;
        xa *= f64::from(0.05_f32);
        za *= f64::from(0.05_f32);
        // A guest's server-side push never reaches its client (ServerPlayer
        // motion is synced only when hurt): its client pushes it itself.
        if !vehicle && (who != Who::Player || w.player.as_ref().is_none_or(|p| !p.remote)) {
            events.push(MobEvent::Push {
                target: who,
                velocity: [-xa, 0.0, -za],
            });
        }
        if self_pushable {
            m.velocity = Vec3::new(m.velocity.x + xa, m.velocity.y, m.velocity.z + za);
        }
    }
}
