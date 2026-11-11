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
use crate::game::remote_projectiles::cloud_client_tick;
use horizon_client::effects::EffectInstance;
use horizon_client::join_seq::{JoinList, JoinSeq};
use horizon_client::mobs::{Damage, Mob, Seen, Who, seen_grid::SeenGrid};
use horizon_client::text::Text;
use horizon_core::geometry::{Aabb, Vec3};
use std::collections::{HashMap, HashSet};

horizon_client::impl_joined!(Breath);

impl Breath {
    /// `AreaEffectCloud.isWaiting`: before its wait time (20 ticks, 10 for a
    /// lingering potion's).
    pub(in crate::game) fn waiting(&self) -> bool {
        self.age < self.potion.as_ref().map_or(20, |p| p.settings.wait_time)
    }
}

/// The mob half of this tick's doses of the clouds a mob joined before: they
/// run in the mob loop, at each cloud's place among the mobs ([`Self::run`]).
#[derive(Default)]
pub(in crate::game) struct CloudDoses {
    /// Each such cloud's join number and the box it doses, in join order.
    due: Vec<(JoinSeq, Aabb)>,
    /// The clouds, out of `Game` while the mobs tick (`held`).
    clouds: JoinList<Breath>,
    held: bool,
    /// For after the loop: the kills a player's cloud is credited with, the
    /// effects added (`onEffectAdded`) and the mobs killed after their tick.
    hurt_by_player: Vec<(u32, Option<u32>)>,
    added: Vec<(&'static str, u32, i32, bool)>,
    killed: Vec<u32>,
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
fn takes(tags: &HashMap<String, HashSet<String>>, effects: &[Effect], m: &Mob) -> bool {
    effects
        .iter()
        .any(|(id, ..)| mob_can_be_affected(tags, &m.kind.id, id))
}

type Effect = (&'static str, i32, i32);

/// One of a cloud's effects on a mob: an instant one at half strength
/// (`HealOrHarmMobEffect.applyInstantaneousEffect`, `indirectMagic(cloud,
/// owner)`), any other at the cloud's duration scale (`LivingEntity.addEffect`).
/// Returns whether a player's cloud hurt it, and the effect added (if new).
fn affect_mob(
    m: &mut Mob,
    tags: &HashMap<String, HashSet<String>>,
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
                let hurt = m.damage_by(amount(6), source, "minecraft:area_effect_cloud");
                (hurt && owner == Some(Who::Player), None)
            } else {
                m.heal(amount(4));
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

/// `snapshot[i]` redone from the mob as it is now (`alive` false once it has
/// left the level), so the mobs that tick after it see that.
pub(in crate::game) fn refresh_seen(
    (snapshot, grid): (&mut [Seen], &mut SeenGrid),
    i: usize,
    m: &Mob,
    (uuid, alive): (Option<[i32; 4]>, bool),
) {
    if let Some(slot) = snapshot.get_mut(i) {
        *slot = m.seen(uuid);
        slot.alive &= alive;
        grid.update(i, slot.bounds);
    }
}

fn inverted(tags: &HashMap<String, HashSet<String>>, m: &Mob) -> bool {
    tags.get("minecraft:inverted_healing_and_harm")
        .is_some_and(|t| t.contains(&m.kind.id))
}

impl CloudDoses {
    /// In the mob loop, before mob `i` ticks (`mobs.len()`: after them all):
    /// the doses of the clouds that joined before it. `keep` holds the verdicts
    /// of the mobs that have ticked (the first `keep.len()`); the ones gone
    /// in their tick are not in the level any more. A dosed mob's snapshot
    /// (if it has one) is redone, so the mobs after it see it as it is now.
    pub(in crate::game) fn run(
        &mut self,
        game: &Game<'_>,
        i: usize,
        mobs: &mut [Mob],
        keep: &[bool],
        (snapshot, grid): (&mut [Seen], &mut SeenGrid),
    ) {
        let uuid = game.level.player_uuid;
        let before = mobs.get(i).map_or(JoinSeq::MAX, |m| m.join);
        while let Some(&(join, area)) = self.due.first()
            && join < before
        {
            self.due.remove(0);
            let Some(k) = self.clouds.iter().position(|c| c.join == join) else {
                continue;
            };
            let c = &mut self.clouds[k];
            let Some(effects) = c.potion.as_ref().map(|p| p.effects.clone()) else {
                continue;
            };
            let tags = &world_stations!(game).enchantments.entity_tags;
            let inside: Vec<usize> = (0..mobs.len())
                .filter(|&j| {
                    let m = &mobs[j];
                    keep.get(j).copied().unwrap_or(true)
                        && m.alive()
                        && m.bounds().intersects(area)
                        && takes(tags, &effects, m)
                })
                .collect();
            let mut gone = false;
            for j in inside {
                let m = &mut mobs[j];
                if !admit(c, Victim::Mob(m.id), m.position.x, m.position.z) {
                    continue;
                }
                let Some(p) = c.potion.as_ref() else {
                    break;
                };
                let flip = inverted(tags, m);
                let looks = (&p.hidden[..], &p.ambient[..], p.settings.duration_scale);
                for effect in &effects {
                    let (hurt, added) = affect_mob(m, tags, effect, looks, (c.at, p.owner, flip));
                    if hurt {
                        self.hurt_by_player.push((m.id, p.guest));
                    }
                    if let Some((name, new)) = added {
                        self.added.push((name, m.id, effect.2, new));
                    }
                }
                if j < keep.len() && !m.alive() {
                    self.killed.push(m.id);
                }
                let alive = keep.get(j).copied().unwrap_or(true);
                refresh_seen((&mut *snapshot, &mut *grid), j, m, (uuid, alive));
                if !used(c) {
                    gone = true;
                    break;
                }
            }
            if gone {
                self.clouds.remove(k);
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
        let first_mob = super::super::ordered_tick::first_join(&self.mobs);
        let mut clouds = std::mem::take(&mut self.breath_clouds);
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        clouds.retain_mut(|c| {
            c.potion.is_none()
                || !ticking(c.at[0].floor() as i32, c.at[2].floor() as i32)
                || self.tick_potion_cloud(c, &players, first_mob)
        });
        self.breath_clouds.append(&mut clouds);
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
            self.potions.cloud_doses.due.push((c.join, area));
        } else {
            let tags = &world_stations!(self).enchantments.entity_tags;
            // Only mobs that at least one of the cloud's effects can affect are
            // taken, so the others neither become victims nor use up its radius.
            for m in self.mobs.iter().filter(|m| {
                m.alive() && m.bounds().intersects(area) && takes(tags, &potion.effects, m)
            }) {
                inside.push((Victim::Mob(m.id), m.position.x, m.position.z));
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
            let (effects, source) = (&p.effects, &p.source);
            self.cloud_affect(victim, effects, looks, c.at, source, (p.owner, p.guest));
            if !used(c) {
                return false;
            }
        }
        true
    }
    /// What a cloud puts on one entity: instant effects at half strength
    /// (`applyInstantaneousEffect` with 0.5), the others at the cloud's duration
    /// scale (`PotionContents.forEachEffect`).
    pub(super) fn cloud_affect(
        &mut self,
        victim: Victim,
        effects: &[Effect],
        looks: (&[&'static str], &[&'static str], f32),
        at: [f64; 3],
        source: &Text,
        (owner, guest): (Option<Who>, Option<u32>),
    ) {
        match victim {
            Victim::Player(who) => {
                // `indirectMagic(cloud, owner)` names the cloud in the death
                // message (it gives no kill credit).
                let cloud_owner = PotionOwner {
                    name: Some(source.clone()),
                    ..Default::default()
                };
                self.as_fight_player(who, |g| {
                    for (id, duration, amp) in effects {
                        let Some(e) = brewing::effect(id) else {
                            continue;
                        };
                        if e.instant {
                            if let Some(a) = horizon_client::effects::instant(id, *amp, 0.5) {
                                g.apply_indirect_effect_action(a, &cloud_owner);
                            }
                        } else {
                            let scaled =
                                horizon_client::effects::scaled_duration(*duration, looks.2);
                            let mut instance = EffectInstance::new(e.id, scaled, *amp);
                            instance.visible = !looks.0.contains(id);
                            instance.ambient = looks.1.contains(id);
                            g.add_player_effect(instance);
                        }
                    }
                });
            }
            Victim::Mob(mob) => {
                let Some(m) = self.mobs.iter().find(|m| m.id == mob) else {
                    return;
                };
                // Undead invert healing and harming.
                let flip = inverted(&world_stations!(self).enchantments.entity_tags, m);
                for effect in effects {
                    let tags = &world_stations!(self).enchantments.entity_tags;
                    // A mod's `onEffectAdded` may have changed the mobs.
                    let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) else {
                        return;
                    };
                    let (hurt, added) = affect_mob(m, tags, effect, looks, (at, owner, flip));
                    if hurt {
                        self.last_hurt_by_player.insert(mob, guest);
                    }
                    if let Some((name, new)) = added {
                        self.mod_effect_added(name, Some(mob as i32), effect.2, new);
                    }
                }
            }
        }
    }

    /// Before the mob loop, when it doses a cloud's mobs: the mobs in join
    /// order, as `ServerLevel.tick` walks them (`entityTickList`), since a
    /// cloud's doses go just before the first mob that joined after it. The
    /// list is in that order unless something put a mob back out of place.
    pub(in crate::game) fn order_mobs_for_cloud_doses(&mut self) {
        if !self.potions.cloud_doses.due.is_empty() && !self.mobs.is_sorted_by_key(|m| m.join) {
            self.mobs.sort_by_key(|m| m.join);
        }
    }

    /// Before the mob loop: the doses due in it, with the clouds out of the
    /// level for the loop (nothing reaches them while the mobs tick).
    pub(in crate::game) fn take_cloud_doses(&mut self) -> CloudDoses {
        let mut doses = std::mem::take(&mut self.potions.cloud_doses);
        if !doses.due.is_empty() {
            doses.due.sort_unstable_by_key(|d| d.0);
            doses.clouds = std::mem::take(&mut self.breath_clouds);
            doses.held = true;
        }
        doses
    }

    /// After the mob loop, with the mobs and the deaths set back in `Game`:
    /// the doses of the clouds that joined after every mob (all have ticked;
    /// the ones gone have left the list), the clouds back in the level, then
    /// what the doses did outside the mobs.
    pub(in crate::game) fn finish_cloud_doses(&mut self, mut doses: CloudDoses) {
        if !doses.due.is_empty() {
            let mut mobs = std::mem::take(&mut self.mobs);
            let all = vec![true; mobs.len()];
            let mut grid = SeenGrid::new(std::iter::empty());
            doses.run(self, mobs.len(), &mut mobs, &all, (&mut [], &mut grid));
            mobs.append(&mut self.mobs);
            self.mobs = mobs;
        }
        doses.due.clear();
        if std::mem::take(&mut doses.held) {
            let mut joined = std::mem::take(&mut self.breath_clouds);
            self.breath_clouds = std::mem::take(&mut doses.clouds);
            self.breath_clouds.append(&mut joined);
        }
        for (mob, guest) in doses.hurt_by_player.drain(..) {
            self.last_hurt_by_player.insert(mob, guest);
        }
        for (name, mob, amp, new) in std::mem::take(&mut doses.added) {
            self.mod_effect_added(name, Some(mob as i32), amp, new);
        }
        // LivingEntity.die runs at the killing blow: a mob the cloud killed
        // after its own tick dies now, not in the next tick.
        for id in doses.killed.drain(..) {
            let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
                continue;
            };
            if !m.removed && self.died.insert(id) {
                let m = m.clone();
                self.mob_died(&m);
            }
        }
        self.potions.cloud_doses = doses;
    }
}
