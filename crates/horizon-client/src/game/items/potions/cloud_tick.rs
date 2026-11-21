//! PURPOSE: `AreaEffectCloud.tick` for potion clouds, and the doses a cloud
//! gives the mobs at its own place in the level's join order.
//! WHY: `ServerLevel.tick` ticks every entity in the order it joined
//! (`entityTickList.forEach`), so a mob that joined before a cloud has
//! counted its effects and hurt timer down when the cloud doses it, and one
//! that joined after counts the new effect down in the same tick. The clouds
//! tick before the mobs here; a cloud that a mob joined before gives its mobs
//! their dose in the mob loop instead, just before the first mob that joined
//! after it ticks (#12183).
//! REFS: vanilla `AreaEffectCloud.serverTick`, `ServerLevel.tick`;
//! `tick_creature_entities`; `ordered_tick.rs`.
//! LOGS: none. READS-BY: `Game::tick_effects`, `tick_creature_entities`.
//! ENDS-WHEN: every kind ticks from the join-order walk.

use super::super::dragon::{Breath, FightPlayer};
use super::{PotionOwner, Victim, brewing, mob_can_be_affected, mob_effect_id};
use crate::game::Game;
use crate::game::combat::PlayerCredit;
use crate::game::remote_projectiles::cloud_client_tick;
use horizon_client::effects::EffectInstance;
use horizon_client::health::DamageLevel;
use horizon_client::join_seq::{JoinList, JoinSeq, ResidentList, RestoreOrder};
use horizon_client::lan::playerdata::ints_uuid;
use horizon_client::mobs::{Damage, Mob, Seen, Who, seen_grid::SeenGrid};
use horizon_client::text::Text;
use horizon_core::geometry::{Aabb, Vec3};
use std::collections::{HashMap, HashSet};

/// One of a cloud's effects: (id, duration, amplifier).
type Effect = (&'static str, i32, i32);
/// The entity type tags (`inverted_healing_and_harm`, the effect immunities).
pub(in crate::game) type EntityTags = HashMap<String, HashSet<String>>;

/// The mob half of this tick's doses of the clouds a mob joined before: they
/// run in the mob loop, at each cloud's place among the mobs.
#[derive(Default)]
pub(in crate::game) struct CloudDoses {
    /// Each such cloud's join number and the box it doses, in join order.
    due: Vec<(JoinSeq, Aabb)>,
    /// The clouds, out of `Game` while the mobs tick (`held`).
    clouds: ResidentList<Breath>,
    held: bool,
    /// The effects the doses in the loop added (`onEffectAdded`), told after it:
    /// no mod can change the mob list while it ticks.
    added: Vec<(&'static str, u32, i32, bool)>,
}

/// `AreaEffectCloud.serverTick` for an entity in its box: one already a
/// victim or out of the radius is passed over; any other becomes a victim.
fn admit(c: &mut Breath, victim: Victim, x: f64, z: f64) -> bool {
    let (dx, dz) = (x - c.at[0], z - c.at[2]);
    let Some(potion) = c.potion.as_mut() else {
        return false;
    };
    if potion.victims.iter().any(|(v, _)| *v == victim)
        || dx * dx + dz * dz > f64::from(c.radius * c.radius)
    {
        return false;
    }
    let until = c.age.wrapping_add(potion.settings.reapplication_delay);
    potion.victims.push((victim, until));
    true
}

/// After a dose: `radiusOnUse` and `durationOnUse`. False once the cloud is gone.
fn used(c: &mut Breath) -> bool {
    let Some(potion) = c.potion.as_ref() else {
        return true;
    };
    if potion.settings.radius_on_use != 0.0 {
        c.radius += potion.settings.radius_on_use;
        if c.radius < 0.5 {
            return false;
        }
        c.radius = c.radius.clamp(0.0, 32.0);
    }
    if potion.settings.duration_on_use != 0 && c.duration != -1 {
        c.duration = c.duration.wrapping_add(potion.settings.duration_on_use);
        if c.duration <= 0 {
            return false;
        }
    }
    true
}

/// Whether one of the cloud's effects can affect this mob
/// (`allEffects.stream().anyMatch(entity::canBeAffected)`).
fn takes(tags: &EntityTags, effects: &[Effect], m: &Mob) -> bool {
    effects
        .iter()
        .any(|(id, ..)| mob_can_be_affected(tags, &m.kind.id, id))
}

fn inverted(tags: &EntityTags, m: &Mob) -> bool {
    tags.get("minecraft:inverted_healing_and_harm")
        .is_some_and(|t| t.contains(&m.kind.id))
}

/// One of a cloud's effects on a mob: an instant one at half strength
/// (`HealOrHarmMobEffect.applyInstantaneousEffect`, `indirectMagic(cloud,
/// owner)`), any other at the cloud's duration scale (`LivingEntity.addEffect`).
/// Returns whether the cloud hurt it, and the effect it added (and whether it
/// was new) for the notice told after the loop.
#[allow(clippy::too_many_arguments)]
fn affect_mob(
    m: &mut Mob,
    level: &mut DamageLevel<'_>,
    context: &horizon_world::native_load_context::LoopContext,
    tags: &EntityTags,
    &(id, duration, amp): &Effect,
    (hidden, ambient, duration_scale): (&[&'static str], &[&'static str], f32),
    (at, owner, inverted): ([f64; 3], Option<Who>, bool),
) -> (bool, Option<(&'static str, bool)>) {
    match id {
        "instant_damage" | "instant_health" => {
            let amount =
                |base: i32| (0.5 * f64::from(base.wrapping_shl(amp as u32)) + 0.5) as i32 as f32;
            if (id == "instant_damage") != inverted {
                let source = Damage::indirect_magic(owner, at);
                let hurt = m.damage_by(level, amount(6), source, "minecraft:area_effect_cloud");
                (hurt, None)
            } else {
                m.heal(context, amount(4));
                (false, None)
            }
        }
        // `LivingEntity.addEffect` refuses what the mob cannot be affected by.
        _ => match mob_effect_id(id) {
            Some(name) if mob_can_be_affected(tags, &m.kind.id, id) => {
                let new = !m.has_effect(name);
                let scaled = horizon_client::effects::scaled_duration(duration, duration_scale);
                m.add_effect_with(
                    name,
                    scaled,
                    amp,
                    ambient.contains(&id),
                    !hidden.contains(&id),
                );
                (false, Some((name, new)))
            }
            _ => (false, None),
        },
    }
}

impl CloudDoses {
    /// Whether a dose is due before a mob's turn this tick, not only after
    /// them all: a cloud due joined before one of `mobs`.
    #[cfg(feature = "upstream-parallel-mobs")]
    pub(in crate::game) fn between_turns(&self, mobs: &[Mob]) -> bool {
        self.due
            .first()
            .is_some_and(|&(join, _)| mobs.iter().any(|m| join < m.join))
    }
}

impl Game<'_> {
    /// What a cloud puts on one entity: instant effects at half strength
    /// (`applyInstantaneousEffect` with 0.5), the others at the cloud's duration
    /// scale (`PotionContents.forEachEffect`). This is the path outside the mob
    /// loop, where the mod hooks can be asked; the loop's own doses use the
    /// vanilla tag checks (`affect_mob`) and tell their hooks after it.
    pub(in crate::game) fn cloud_affect(
        &mut self,
        victim: Victim,
        effects: &[Effect],
        (hidden, ambient, duration_scale): (&[&'static str], &[&'static str], f32),
        at: [f64; 3],
        source: &Text,
        (owner, guest): (Option<Who>, Option<u32>),
    ) {
        // `indirectMagic(cloud, owner)` names the cloud in the death message
        // (it gives no kill credit).
        let cloud_owner = PotionOwner {
            name: Some(source.clone()),
            ..Default::default()
        };
        // MobEffectInstance.withScaledDuration: floor(duration * scale), at least 1; an infinite
        // effect stays infinite.
        let scaled = |duration| horizon_client::effects::scaled_duration(duration, duration_scale);
        match victim {
            Victim::Player(who) => self.as_fight_player(who, |g| {
                for (id, duration, amp) in effects {
                    let Some(e) = brewing::effect(id) else {
                        continue;
                    };
                    if e.instant {
                        if let Some(a) = horizon_client::effects::instant(id, *amp, 0.5) {
                            g.apply_indirect_effect_action(a, &cloud_owner);
                        }
                    } else {
                        let mut instance = EffectInstance::new(e.id, scaled(*duration), *amp);
                        instance.visible = !hidden.contains(id);
                        instance.ambient = ambient.contains(id);
                        g.add_player_effect(instance);
                    }
                }
            }),
            Victim::Mob(mob) => {
                let Some(i) = self.mobs.iter().position(|m| m.id == mob) else {
                    return;
                };
                // Undead invert healing and harming.
                let tags = &world_stations!(self).enchantments.entity_tags;
                let inverted = tags
                    .get("minecraft:inverted_healing_and_harm")
                    .is_some_and(|t| t.contains(&self.mobs[i].kind.id));
                for (id, duration, amp) in effects {
                    // A mod's `onEffectAdded` may have changed the mobs.
                    let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) else {
                        return;
                    };
                    match *id {
                        "instant_damage" | "instant_health" => {
                            let amount = |base: i32| {
                                (0.5 * f64::from(base.wrapping_shl(*amp as u32)) + 0.5) as i32
                                    as f32
                            };
                            if (*id == "instant_damage") != inverted {
                                // HealOrHarmMobEffect.applyInstantaneousEffect: indirectMagic(cloud, owner).
                                let source = Damage::indirect_magic(owner, at);
                                if m.damage_by(
                                    crate::game::entity::damage_level_of!(self, scoped),
                                    amount(6),
                                    source,
                                    "minecraft:area_effect_cloud",
                                ) && owner == Some(Who::Player)
                                    && let Some(uuid) = self.player_uuid_of(guest)
                                {
                                    self.last_hurt_by_player.insert(
                                        mob,
                                        PlayerCredit(ints_uuid(uuid)),
                                    );
                                }
                            } else {
                                m.heal(&self.loop_context, amount(4));
                            }
                        }
                        _ => {
                            // `LivingEntity.addEffect` refuses what the mob
                            // cannot be affected by: the borrow ends before
                            // the hook asks, and the mob is found again.
                            let kind = m.kind.id.clone();
                            if self.can_be_affected(Some(mob as i32), &kind, id)
                                && let Some(name) = mob_effect_id(id)
                            {
                                let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) else {
                                    return;
                                };
                                let new = !m.has_effect(name);
                                m.add_effect_with(
                                    name,
                                    scaled(*duration),
                                    *amp,
                                    ambient.contains(id),
                                    !hidden.contains(id),
                                );
                                self.mod_effect_added(name, Some(mob as i32), *amp, new);
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Game<'_> {
    /// `AreaEffectCloud.tick` for the lingering potions' clouds: the client
    /// tick's particles, then the server tick.
    pub(in crate::game) fn tick_potion_clouds(&mut self) {
        // What a tick that stopped before its mobs left is not dosed late.
        self.potions.cloud_doses.due.clear();
        if self.breath_clouds.iter().all(|c| c.potion.is_none()) {
            return;
        }
        let players: Vec<FightPlayer> = self
            .fight_players()
            .into_iter()
            .filter(|p| p.alive)
            .collect();
        let first_mob = self.mobs.iter().map(|m| m.join).min();
        let mut clouds = self.breath_clouds.lift();
        #[cfg(test)]
        crate::game::entity::taken_pass_tests::after_take(self);
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut expired: Vec<u32> = Vec::new();
        clouds.retain_mut(|c| {
            let keep = c.potion.is_none()
                || !ticking(c.at[0].floor() as i32, c.at[2].floor() as i32)
                || self.tick_potion_cloud(c, &players, first_mob);
            if !keep {
                expired.push(c.id);
            }
            keep
        });
        self.breath_clouds.restore(clouds, RestoreOrder::SpawnsFirst);
        // A cloud that expired on its own takes its `noPhysics` write with it.
        for id in expired {
            self.forget_entity(id);
        }
    }
    /// `AreaEffectCloud.serverTick`: configured lifetime/radius changes and
    /// five-tick doses, with each victim's configured reapplication delay.
    /// The mobs' doses of a cloud a mob joined before (`first_mob`) wait for
    /// its place in the mob loop ([`CloudDoses`]). False once the cloud is gone.
    fn tick_potion_cloud(
        &mut self,
        c: &mut Breath,
        players: &[FightPlayer],
        first_mob: Option<JoinSeq>,
    ) -> bool {
        // `Entity.baseTick`: an `AreaEffectCloud` is fire-immune, so a positive
        // loaded `Fire` is cleared on its first tick (a load keeps it as given).
        if let Some(raw) = self.persist.raw.get_mut(&c.id) {
            horizon_client::entity_nbt::clear_positive_fire(raw);
        }
        // `Entity.baseTick` -> `updateFluidInteraction`: a cloud whose box (`radius * 2`
        // wide, 0.5 high) is in water resets the fall distance it was loaded with.
        if c.potion.is_some() {
            let r = f64::from(c.radius);
            let bounds = Aabb::new(
                Vec3::new(c.at[0] - r, c.at[1], c.at[2] - r),
                Vec3::new(c.at[0] + r, c.at[1] + 0.5, c.at[2] + r),
            );
            if self.bounds_in_water(bounds)
                && let Some(horizon_core::nbt::Tag::Compound(raw)) =
                    self.persist.raw.get_mut(&c.id)
                && raw.contains_key(&"fall_distance".into())
            {
                raw.insert("fall_distance".into(), horizon_core::nbt::Tag::Double(0.0_f64.to_bits()));
            }
        }
        c.age = c.age.wrapping_add(1);
        let Some(potion) = c.potion.as_mut() else {
            return true;
        };
        if c.duration != -1 && c.age.wrapping_sub(potion.settings.wait_time) >= c.duration {
            return false;
        }
        let waiting = c.age < potion.settings.wait_time;
        cloud_client_tick(
            &mut self.particles,
            c.at,
            c.radius,
            waiting,
            &potion.particle(),
        );
        if waiting {
            return true;
        }
        if potion.radius_per_tick != 0.0 {
            c.radius += potion.radius_per_tick;
            if c.radius < 0.5 {
                return false;
            }
            c.radius = c.radius.clamp(0.0, 32.0);
        }
        if c.age % 5 != 0 {
            return true;
        }
        potion.victims.retain(|(_, until)| c.age < *until);
        if potion.effects.is_empty() {
            potion.victims.clear();
            return true;
        }
        let r = f64::from(c.radius);
        let area = Aabb::new(
            Vec3::new(c.at[0] - r, c.at[1], c.at[2] - r),
            Vec3::new(c.at[0] + r, c.at[1] + 0.5, c.at[2] + r),
        );
        let mut inside = Vec::new();
        for p in players.iter().filter(|p| p.bounds.intersects(area)) {
            inside.push((Victim::Player(p.who), p.position.x, p.position.z));
        }
        if first_mob.is_some_and(|first| first < c.join) {
            // A mob joined before this cloud: its mob doses wait for their place in the
            // mob loop. The players' are given here, as `ServerPlayer.doTick` runs
            // before the level's entities.
            self.potions.cloud_doses.due.push((c.join, area));
        } else {
            for (mob, x, z) in self.cloud_mob_candidates(&potion.effects, area) {
                inside.push((Victim::Mob(mob), x, z));
            }
        }
        for (victim, x, z) in inside {
            if !admit(c, victim, x, z) {
                continue;
            }
            let Some(p) = c.potion.as_ref() else {
                break;
            };
            let looks = (&p.hidden[..], &p.ambient[..], p.settings.duration_scale);
            self.cloud_affect(victim, &p.effects, looks, c.at, &p.source, (p.owner, p.guest));
            if !used(c) {
                return false;
            }
        }
        true
    }
    /// The mobs a cloud doses in `area`: the living ones at least one of its
    /// effects can affect (`allEffects.stream().anyMatch(entity::canBeAffected)`),
    /// so the others neither become victims nor use up its radius.
    fn cloud_mob_candidates(&mut self, effects: &[Effect], area: Aabb) -> Vec<(u32, f64, f64)> {
        let effects = effects.to_vec();
        let candidates: Vec<(u32, String, f64, f64)> = self
            .mobs
            .iter()
            .filter(|m| m.alive() && m.bounds().intersects(area))
            .map(|m| (m.id, m.kind.id.clone(), m.position.x, m.position.z))
            .collect();
        candidates
            .into_iter()
            .filter(|(id, kind, ..)| {
                effects
                    .iter()
                    .any(|(effect, ..)| self.can_be_affected(Some(*id as i32), kind, effect))
            })
            .map(|(id, _, x, z)| (id, x, z))
            .collect()
    }
    /// Before the mob loop, when it doses a cloud's mobs: the mobs in join
    /// order, as `ServerLevel.tick` walks them (`entityTickList`), since a
    /// cloud's doses go just before the first mob that joined after it. The
    /// list is in that order unless something put a mob back out of place.
    pub(in crate::game) fn order_mobs_for_cloud_doses(&mut self) {
        if self.potions.cloud_doses.due.is_empty() || self.mobs.is_sorted_by_key(|m| m.join) {
            return;
        }
        // An insertion sort in place: swaps keep every mob's identity and join number.
        for i in 1..self.mobs.len() {
            let mut j = i;
            while j > 0 && self.mobs[j].join < self.mobs[j - 1].join {
                self.mobs.swap(j - 1, j);
                j -= 1;
            }
        }
    }
    /// Before the mob loop: the doses due in it, with the clouds out of the
    /// level for the loop (nothing reaches them while the mobs tick).
    pub(in crate::game) fn take_cloud_doses(&mut self) -> CloudDoses {
        let mut doses = std::mem::take(&mut self.potions.cloud_doses);
        if !doses.due.is_empty() {
            doses.due.sort_unstable_by_key(|d| d.0);
            doses.clouds = self.breath_clouds.lift();
            doses.held = true;
        }
        doses
    }
    /// Before mob `slot`'s turn in the mob loop: the doses of the clouds that
    /// joined before it. The loop holds `Game`'s mobs, so the dose works on the
    /// loop's own list: it reads the mobs as they are at that point, as
    /// `getEntitiesOfClass` does, and redoes the dosed mobs' snapshot slots so
    /// the mobs after them see the new state. What reaches outside the mobs
    /// (`onEffectAdded`) is told after the loop; a mob that died after its own
    /// turn dies now (`dead`).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn run_cloud_doses(
        &self,
        doses: &mut CloudDoses,
        level: &mut DamageLevel<'_>,
        slot: usize,
        mobs: &mut JoinList<Mob>,
        keep: &[bool],
        (snapshot, grid): (&mut [Seen], &mut SeenGrid),
        credit: &mut HashMap<u32, PlayerCredit>,
        dead: &mut Vec<Mob>,
        died: &mut horizon_client::fast_hash::FxHashSet<u32>,
    ) {
        let before = mobs.get(slot).map_or(JoinSeq::MAX, |m| m.join);
        let tags = &world_stations!(self).enchantments.entity_tags;
        let items = world_items!(self);
        let context = &self.loop_context;
        let uuid = self.level.player_uuid;
        let CloudDoses {
            due,
            clouds,
            added,
            ..
        } = doses;
        while let Some(&(join, area)) = due.first()
            && join < before
        {
            due.remove(0);
            let Some(k) = clouds.locate(join) else {
                continue;
            };
            let mut remove = false;
            clouds.update(k, |c| {
                let Some(effects) = c.potion.as_ref().map(|p| p.effects.clone()) else {
                    return;
                };
                let inside: Vec<usize> = (0..mobs.len())
                    .filter(|&j| {
                        let m = &mobs[j];
                        keep.get(j).copied().unwrap_or(true)
                            && m.alive()
                            && m.bounds().intersects(area)
                            && takes(tags, &effects, m)
                    })
                    .collect();
                for j in inside {
                    let (id, x, z) = {
                        let m = &mobs[j];
                        (m.id, m.position.x, m.position.z)
                    };
                    if !admit(c, Victim::Mob(id), x, z) {
                        continue;
                    }
                    let Some(p) = c.potion.as_ref() else {
                        break;
                    };
                    let looks = (&p.hidden[..], &p.ambient[..], p.settings.duration_scale);
                    let (owner, guest, at) = (p.owner, p.guest, c.at);
                    let flip = inverted(tags, &mobs[j]);
                    for effect in &effects {
                        let m = &mut mobs[j];
                        let (hurt, told) =
                            affect_mob(m, level, context, tags, effect, looks, (at, owner, flip));
                        if hurt
                            && owner == Some(Who::Player)
                            && let Some(uuid) = self.player_uuid_of(guest)
                        {
                            credit.insert(id, PlayerCredit(ints_uuid(uuid)));
                        }
                        if let Some((name, new)) = told {
                            added.push((name, id, effect.2, new));
                        }
                    }
                    let m = &mut mobs[j];
                    // A mob whose turn is over has already left its death to the
                    // next tick: it dies now (`LivingEntity.die` runs at the blow).
                    if !m.alive() && !m.removed && j < slot && died.insert(m.id) {
                        dead.push(m.clone());
                    }
                    let alive = keep.get(j).copied().unwrap_or(true);
                    if let Some(s) = snapshot.get_mut(j) {
                        *s = m.seen(uuid, items);
                        s.alive &= alive;
                        grid.update(j, s.bounds);
                    }
                    if !used(c) {
                        remove = true;
                        return;
                    }
                }
            });
            if remove {
                clouds.remove(k);
            }
        }
    }
    /// After the mob loop, with the mobs and the deaths set back in `Game`:
    /// the doses of the clouds that joined after every mob (all have ticked;
    /// the ones gone have left the list), the clouds back in the level, then
    /// what the doses did outside the mobs.
    pub(in crate::game) fn finish_cloud_doses(&mut self, mut doses: CloudDoses) {
        for (join, area) in std::mem::take(&mut doses.due) {
            let Some(k) = doses.clouds.locate(join) else {
                continue;
            };
            let mut gone = false;
            doses.clouds.update(k, |c| {
                let Some(effects) = c.potion.as_ref().map(|p| p.effects.clone()) else {
                    return;
                };
                for (id, x, z) in self.cloud_mob_candidates(&effects, area) {
                    if !admit(c, Victim::Mob(id), x, z) {
                        continue;
                    }
                    let Some(p) = c.potion.as_ref() else {
                        break;
                    };
                    let looks = (&p.hidden[..], &p.ambient[..], p.settings.duration_scale);
                    self.cloud_affect(
                        Victim::Mob(id),
                        &p.effects,
                        looks,
                        c.at,
                        &p.source,
                        (p.owner, p.guest),
                    );
                    if !used(c) {
                        gone = true;
                        return;
                    }
                }
            });
            if gone {
                doses.clouds.remove(k);
            }
        }
        if std::mem::take(&mut doses.held) {
            let clouds = std::mem::take(&mut doses.clouds);
            self.breath_clouds.restore(clouds, RestoreOrder::SurvivorsFirst);
        }
        for (name, mob, amp, new) in std::mem::take(&mut doses.added) {
            self.mod_effect_added(name, Some(mob as i32), amp, new);
        }
        self.potions.cloud_doses = doses;
    }
}
