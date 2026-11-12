use super::*;
use crate::game::{commands, dragon, explosion};

impl Game<'_> {
    pub(super) fn projectile_hit_inner(&mut self, p: &Projectile, hit: Hit) -> Result<()> {
        let _attacker = self.knockback_scope_for(p.owner);
        // Projectile.onHitBlock: BlockState.onProjectileHit, for every
        // projectile (a wind charge out of the world's top has no face).
        if let Hit::Block { pos, at, face } = hit
            && face != [0; 3]
        {
            self.bell_projectile_hit(&ProjectileHitter::of(p), pos, at, face);
            self.amethyst_projectile_hit(pos);
            if self.with_level(|b, level, ticks| {
                b.big_dripleaf_projectile_hit(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]))
            }) {
                self.process_level_events()?;
            }
            self.projectile_breaks_block(p, pos);
        }
        self.combat.projectile_owner = match p.owner {
            Some(Who::Mob(id)) => Some(id),
            _ => None,
        };
        self.combat.projectile_direct = match hit {
            Hit::Entity {
                who: Who::Player, ..
            } => Some(self.target_name(commands::Target::Projectile(p.id))),
            _ => None,
        };
        let at = match hit {
            Hit::Entity { at, .. } | Hit::Block { at, .. } => at,
            Hit::None => return Ok(()),
        };
        // Projectile.onHitBlock: BlockState.onProjectileHit, for every
        // projectile (a wind charge out of the world's top has no face).
        if let Hit::Block { pos, face, .. } = hit
            && face != [0; 3]
        {
            self.target_block_hit(&ProjectileHitter::of(p), pos, at, face);
        }
        if let Hit::Entity {
            who: Who::Mob(id),
            at,
        } = hit
            && self.lan_projectile_hit(p, id, at)
        {
            return Ok(());
        }
        if matches!(
            p.kind,
            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
        ) {
            return self.wind_charge_hit(p, hit);
        }
        // A wither skull's hit on a part or crystal goes through
        // `wither_skull_hit` below, as on any entity, and then explodes.
        if let Hit::Entity {
            who: Who::Mob(id), ..
        } = hit
            && id >= dragon::SPECIAL
            && !matches!(
                p.kind,
                ProjectileKind::WitherSkull { .. } | ProjectileKind::DragonFireball
            )
        {
            let by_player = p.owner == Some(Who::Player);
            if p.arrow.is_some() {
                // AbstractArrow.onHitEntity: spent if the part or crystal took
                // it, else turned back (a perched dragon sets it alight first).
                let mut arrow = p.clone();
                let amount = self.shot_damage(&mut arrow, None);
                let outcome = match self.hit_special(
                    id,
                    amount,
                    by_player,
                    horizon_client::mobs::DamageKind::Arrow,
                    Some(&p.kind),
                    self.projectile_explosion_cause(p),
                ) {
                    dragon::Hurt::Landed { .. } => EntityHit::Hurt,
                    dragon::Hurt::Refused => EntityHit::Refused,
                    dragon::Hurt::Shrugged => {
                        if let Some(a) = arrow.arrow.as_mut() {
                            a.ignite_for_seconds(1.0);
                        }
                        EntityHit::Refused
                    }
                };
                self.finish_arrow_hit(arrow, outcome);
                return Ok(());
            }
            let amount = match p.kind {
                ProjectileKind::SmallFireball => 5.0,
                ProjectileKind::LargeFireball { .. } => 6.0,
                ProjectileKind::Thrown(t) => t.damage,
                _ => 0.0,
            };
            let kind = match p.kind {
                ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                    horizon_client::mobs::DamageKind::Fireball
                }
                _ => horizon_client::mobs::DamageKind::Thrown,
            };
            self.hit_special(
                id,
                amount,
                by_player,
                kind,
                Some(&p.kind),
                self.projectile_explosion_cause(p),
            );
            // ThrowableProjectile.onHit goes on after onHitEntity: a pearl
            // still takes its owner to where it broke (into the crystal's
            // blast), an egg still hatches, a snowball still breaks. A large
            // fireball still explodes (LargeFireball.onHit), whatever the
            // part or crystal made of its hit (both refuse fire), and the
            // blast takes the crystal.
            if !matches!(
                p.kind,
                ProjectileKind::EnderPearl
                    | ProjectileKind::Egg
                    | ProjectileKind::Snowball
                    | ProjectileKind::Thrown(_)
                    | ProjectileKind::LargeFireball { .. }
            ) {
                return Ok(());
            }
        }
        if p.arrow.is_some()
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && self.item_frames.iter().any(|f| f.id == id)
        {
            // AbstractArrow.onHitEntity on an item frame: spent when the
            // frame took the hit, turned back when a fixed one refused it.
            let by_player = p.owner == Some(Who::Player);
            let hurt = self.projectile_hurts_frame(id, by_player, by_player && self.creative);
            let outcome = if hurt {
                EntityHit::Hurt
            } else {
                EntityHit::Refused
            };
            self.finish_arrow_hit(p.clone(), outcome);
            return Ok(());
        }
        if p.arrow.is_some()
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && let Some(i) = self.carts.iter().position(|c| c.id == id)
        {
            // AbstractArrow.onHitEntity on a minecart.
            let mut arrow = p.clone();
            let amount = self.shot_damage(&mut arrow, None);
            let burning = p.arrow.as_ref().is_some_and(|a| a.on_fire());
            let creative = p.owner == Some(Who::Player) && self.creative;
            self.arrow_hits_cart(i, amount, p.velocity, burning, creative);
            self.finish_arrow_hit(arrow, EntityHit::Hurt);
            return Ok(());
        }
        if p.arrow.is_some()
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && let Some(i) = self.boats.iter().position(|b| b.id == id)
        {
            // AbstractArrow.onHitEntity on a boat: VehicleEntity.hurtServer
            // takes the arrow's damage, and the arrow is spent.
            let mut arrow = p.clone();
            let amount = self.shot_damage(&mut arrow, None);
            let creative = p.owner == Some(Who::Player) && self.creative;
            self.hurt_boat(i, amount, creative);
            self.finish_arrow_hit(arrow, EntityHit::Hurt);
            return Ok(());
        }
        let atp = [at.x, at.y, at.z];
        // Projectile.onHit: the landing is heard.
        self.projectile_event("minecraft:projectile_land", p.owner, atp);
        let source_kind = match p.kind {
            ProjectileKind::Arrow { .. } | ProjectileKind::Trident => DamageKind::Arrow,
            ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                DamageKind::Fireball
            }
            _ => DamageKind::Thrown,
        };
        let attacker_species = match p.owner {
            Some(Who::Mob(id)) => self.mobs.iter().find(|m| m.id == id).map(|m| m.species()),
            _ => None,
        };
        let source = Damage {
            kind: source_kind,
            attacker: p.owner,
            from: Some([p.previous.x, p.previous.y, p.previous.z]),
            attacker_species,
        };
        let aim = self.attacker_aim(source.attacker);
        match (&p.kind, hit) {
            // ThrownTrident.onHitEntity: 8 damage (Impaling on aquatic mobs),
            // dealt whether or not the hurt lands, then it bounces off.
            (ProjectileKind::Trident, Hit::Entity { who, .. }) => {
                let mut trident = p.clone();
                let victim = match who {
                    Who::Mob(id) => self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| m.kind.id.clone()),
                    Who::Player => Some("minecraft:player".to_owned()),
                };
                let amount = self.shot_damage(&mut trident, victim.as_deref());
                let hurt = match who {
                    Who::Player => self.mob_hits_player(
                        amount,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:trident",
                    ),
                    Who::Mob(id) => self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                        m.damage_projectile(amount, source, aim, [p.velocity.x, p.velocity.z])
                    }),
                };
                let outcome = if hurt {
                    self.shot_post_attack(p, who, "minecraft:trident", "minecraft:trident");
                    EntityHit::Hurt
                } else {
                    EntityHit::Refused
                };
                self.burning_arrow_primes(p, who);
                self.finish_arrow_hit(trident, outcome);
            }
            // AbstractArrow.onHitBlock plays its sound itself (ArrowEvent).
            (ProjectileKind::Trident, Hit::Block { .. }) => {}
            // AbstractArrow.onHitEntity on a ghast's fireball or a wind charge:
            // `hurtOrSimulate` fails (`AbstractHurtingProjectile.hurtServer` is
            // false; the projectile turned in `Projectile.onHit`, which already
            // ran), so the arrow takes `deflect(REVERSE, 0.2)` instead of vanishing.
            (
                ProjectileKind::Arrow { .. },
                Hit::Entity {
                    who: Who::Mob(id), ..
                },
            ) if pickable_projectile_hit(id).is_some() => {
                let mut arrow = p.clone();
                arrow_damage(&mut arrow, &mut self.world.random);
                self.finish_arrow_hit(arrow, EntityHit::Refused);
            }
            (ProjectileKind::Arrow { .. }, Hit::Entity { who, .. }) => {
                let mut arrow = p.clone();
                let amount = arrow_damage(&mut arrow, &mut self.world.random) + p.bonus.damage;
                let player_fire = self.player.as_ref().map(|pl| pl.fire_ticks);
                // AbstractArrow.onHitEntity saves the fire, ignites, then puts
                // it back when hurtOrSimulate fails and the target still takes
                // a projectile's side effects.
                let mob_fire = match who {
                    Who::Mob(id) => self.mobs.iter().find(|m| m.id == id).map(|m| m.fire_ticks),
                    Who::Player => None,
                };
                let mut dodged = false;
                // AbstractArrow.onHitEntity: a burning arrow sets its target alight.
                if p.arrow.as_ref().is_some_and(|a| a.on_fire()) {
                    self.handle_mob_events(vec![MobEvent::Ignite(who, 5)])?;
                }
                let hurt = match who {
                    Who::Player => {
                        let hurt = self.arrow_hits_player(amount, p);
                        // AbstractArrow.onHitEntity: a hit that hurt leaves a
                        // non-piercing arrow stuck in the body.
                        if hurt && p.arrow.as_ref().is_some_and(|a| a.pierce == 0) {
                            self.combat.stuck.add_arrow();
                        }
                        hurt
                    }
                    Who::Mob(id) => {
                        let hurt = self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                            m.damage_projectile(amount, source, aim, [p.velocity.x, p.velocity.z])
                        });
                        dodged = !hurt
                            && self.mobs.iter().find(|m| m.id == id).is_some_and(|m| {
                                m.species() == Species::Enderman
                                    && horizon_client::mobs::enderman::dodges(m, &source)
                            });
                        if p.owner == Some(Who::Player) {
                            self.last_hurt_by_player.insert(id, self.lan_acting);
                            if hurt && let Some(m) = self.mobs.iter().find(|m| m.id == id) {
                                // LivingEntity.hurtServer: PLAYER_HURT_ENTITY.
                                let victim = self.mob_entity(m);
                                let spectral = p.arrow.as_ref().is_some_and(|a| {
                                    &*a.pickup_item.item == "minecraft:spectral_arrow"
                                });
                                let direct = horizon_client::advancements::EntityView {
                                    kind: if spectral {
                                        "minecraft:spectral_arrow"
                                    } else {
                                        "minecraft:arrow"
                                    }
                                    .into(),
                                    position: [p.position.x, p.position.y, p.position.z],
                                    ..Default::default()
                                };
                                self.player_hit(&victim, "minecraft:arrow", Some(direct), amount);
                            }
                            // AbstractArrow.onHitEntity: KILLED_BY_ARROW for a kill
                            // (piercing arrows are not modelled).
                            if hurt
                                && let Some(m) = self.mobs.iter().find(|m| m.id == id)
                                && !m.alive()
                            {
                                let victim = self.mob_entity(m);
                                let weapon =
                                    p.arrow.as_ref().and_then(|a| a.fired_from_weapon.as_ref());
                                self.trigger_killed_by_arrow(&[victim], weapon);
                            }
                        }
                        hurt
                    }
                };
                self.burning_arrow_primes(p, who);
                // doKnockback: Punch pushes along the flight, less the target's
                // knockback resistance.
                let flight = (p.velocity.x * p.velocity.x + p.velocity.z * p.velocity.z).sqrt();
                let resistance = match who {
                    Who::Player => self
                        .player
                        .as_mut()
                        .and_then(|pl| pl.attribute("minecraft:knockback_resistance").ok())
                        .unwrap_or(0.0),
                    Who::Mob(id) => self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map_or(0.0, |m| m.knockback_resistance()),
                };
                let knockback = if !hurt {
                    0.0
                } else if let Some(weapon) =
                    p.arrow.as_ref().and_then(|a| a.fired_from_weapon.as_ref())
                {
                    let victim = match who {
                        Who::Player => Some("minecraft:player"),
                        Who::Mob(id) => self
                            .mobs
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| m.kind.id.as_str()),
                    };
                    let owner = match p.owner {
                        Some(Who::Player) => Some("minecraft:player"),
                        Some(Who::Mob(id)) => self
                            .mobs
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| m.kind.id.as_str()),
                        None => None,
                    };
                    let arrow_type = if p
                        .arrow
                        .as_ref()
                        .is_some_and(|a| &*a.pickup_item.item == "minecraft:spectral_arrow")
                    {
                        "minecraft:spectral_arrow"
                    } else {
                        "minecraft:arrow"
                    };
                    let ctx = horizon_client::enchant::EffectContext {
                        this: victim,
                        attacker: owner,
                        direct_attacker: Some(arrow_type),
                        damage_type: Some("minecraft:arrow"),
                        items: Some(self.res.items()),
                        ..Default::default()
                    };
                    self.res.stations().enchantments.modify_value(
                        Some(weapon),
                        "minecraft:knockback",
                        &ctx,
                        &mut self.world.random,
                        0.0,
                    )
                } else {
                    p.bonus.punch as f32
                };
                let push = f64::from(knockback) * 0.6 * (1.0 - resistance).max(0.0);
                if hurt && push > 0.0 && flight > 1e-6 {
                    let k = push / flight;
                    self.handle_mob_events(vec![MobEvent::Push {
                        target: who,
                        velocity: [p.velocity.x * k, 0.1, p.velocity.z * k],
                    }])?;
                }
                if hurt {
                    let spectral = p
                        .arrow
                        .as_ref()
                        .is_some_and(|a| &*a.pickup_item.item == "minecraft:spectral_arrow");
                    let direct = if spectral {
                        "minecraft:spectral_arrow"
                    } else {
                        "minecraft:arrow"
                    };
                    self.shot_post_attack(p, who, direct, "minecraft:arrow");
                    self.arrow_post_hurt_effects(p, who);
                }
                match (hurt, who) {
                    (true, _) => self.finish_arrow_hit(arrow, EntityHit::Hurt),
                    // What the player took no damage from (a raised shield)
                    // keeps its fire as it was and turns the arrow back.
                    (false, Who::Player) => {
                        if let (Some(pl), Some(ticks)) = (self.player.as_mut(), player_fire) {
                            pl.set_remaining_fire_ticks(ticks);
                        }
                        self.finish_arrow_hit(arrow, EntityHit::Refused);
                    }
                    // EnderMan.hurtServer teleported away: the arrow flies on.
                    (false, Who::Mob(_)) if dodged => {
                        self.finish_arrow_hit(arrow, arrow_miss(true));
                    }
                    // hurtOrSimulate returned false and
                    // projectileReceivesSideEffectsOnHit(false) is the default
                    // (Shulker.hurtServer on a closed shell, invulnerability).
                    // The fire the arrow just set goes back, then
                    // deflect(REVERSE, entity, owner, false, 0.2). The hit
                    // sound plays only when the hurt lands.
                    (false, Who::Mob(id)) => {
                        if let (Some(m), Some(ticks)) =
                            (self.mobs.iter_mut().find(|m| m.id == id), mob_fire)
                        {
                            m.fire_ticks = ticks;
                        }
                        self.finish_arrow_hit(arrow, arrow_miss(false));
                    }
                }
            }
            // AbstractArrow.onHitBlock plays its sound itself (ArrowEvent).
            (ProjectileKind::Arrow { .. }, Hit::Block { .. }) => {}
            (ProjectileKind::Snowball, hit) => {
                if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    // Snowball.onHitEntity: 3 damage to blazes, 0 otherwise.
                    let amount = if m.species() == Species::Blaze {
                        3.0
                    } else {
                        0.0
                    };
                    if m.damage_projectile(amount, source, aim, [p.velocity.x, p.velocity.z]) {
                        self.thrown_hurt_mob(p, id, "minecraft:snowball", amount);
                    }
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                {
                    // Snowball.onHitEntity: 0 damage, which still shakes a
                    // boat or minecart (VehicleEntity.hurtServer).
                    self.hurt_vehicle(p, id, 0.0);
                }
                self.thrown_break_event(p.id, atp);
                self.item_burst(atp, "minecraft:snowball");
            }
            // a mod's copy of Snowball: its damage to anything it hits.
            (ProjectileKind::Thrown(t), hit) => {
                match hit {
                    Hit::Entity {
                        who: Who::Mob(id), ..
                    } => {
                        let push = [p.velocity.x, p.velocity.z];
                        let hurt = self
                            .mobs
                            .iter_mut()
                            .find(|m| m.id == id)
                            .is_some_and(|m| m.damage_projectile(t.damage, source, aim, push));
                        if hurt {
                            self.player_hurt_mob(p, id);
                        }
                        // LivingEntity.hurtServer: PLAYER_HURT_ENTITY.
                        if hurt
                            && p.owner == Some(Who::Player)
                            && let Some(m) = self.mobs.iter().find(|m| m.id == id)
                        {
                            let victim = self.mob_entity(m);
                            let direct = horizon_client::advancements::EntityView {
                                kind: t.id.into(),
                                position: [p.position.x, p.position.y, p.position.z],
                                ..Default::default()
                            };
                            self.player_hit(&victim, "minecraft:thrown", Some(direct), t.damage);
                        }
                    }
                    // Player.hurtServer takes no 0 damage.
                    Hit::Entity {
                        who: Who::Player, ..
                    } if t.damage > 0.0 => {
                        let from = [p.previous.x, p.previous.y, p.previous.z];
                        self.mob_hits_player(t.damage, from, "minecraft:thrown");
                    }
                    _ => {}
                }
                self.thrown_break_event(p.id, atp);
                self.item_burst(atp, t.item);
            }
            (ProjectileKind::Egg, hit) => {
                if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    if m.damage_projectile(0.0, source, aim, [p.velocity.x, p.velocity.z]) {
                        self.thrown_hurt_mob(p, id, "minecraft:egg", 0.0);
                    }
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                {
                    // ThrownEgg.onHitEntity: the same 0 damage.
                    self.hurt_vehicle(p, id, 0.0);
                }
                // ThrownEgg.onHit: 1/8 chance of a chick, 1/32 of four.
                if self.world.random.next_int_bound(8).unwrap_or(1) == 0 {
                    let n = if self.world.random.next_int_bound(32).unwrap_or(1) == 0 {
                        4
                    } else {
                        1
                    };
                    for _ in 0..n {
                        if let Some(id) = self.spawn_mob_raw("minecraft:chicken", atp, true)
                            && let Some(c) = self.mobs.iter_mut().find(|m| m.id == id)
                        {
                            (c.extra.age, c.extra.variant) = (-24000, p.egg_variant.unwrap_or(0));
                        }
                    }
                }
                self.thrown_break_event(p.id, atp);
                self.item_burst(atp, "minecraft:egg");
            }
            (ProjectileKind::EnderPearl, hit) => {
                if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    if m.damage_projectile(0.0, source, aim, [p.velocity.x, p.velocity.z]) {
                        self.thrown_hurt_mob(p, id, "minecraft:ender_pearl", 0.0);
                    }
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                {
                    // ThrownEnderpearl.onHitEntity: the same 0 damage.
                    self.hurt_vehicle(p, id, 0.0);
                }
                // To where the pearl started its last tick (oldPosition).
                let mut to = p.previous;
                let mut damage = 5.0;
                let mut cancel = false;
                // EntityTeleportEvent.EnderPearl: a server player's pearl.
                if p.owner == Some(Who::Player)
                    && let Some(from) = self.player.as_ref().map(|pl| pl.position)
                    && let (Some(uuid), Some(name)) =
                        (self.lan_acting_uuid, self.lan_acting_name.clone())
                {
                    let mut target = [to.x, to.y, to.z];
                    self.native_ender_pearl_teleport(
                        uuid,
                        &name,
                        p.id,
                        [from.x, from.y, from.z],
                        &mut target,
                        &mut damage,
                        &mut cancel,
                    );
                    // A target that is not a number keeps the pearl's.
                    if target.iter().all(|v| v.is_finite()) {
                        to = Vec3::new(target[0], target[1], target[2]);
                    }
                }
                // ThrownEnderpearl.isAllowedToTeleportOwner: an owner who is
                // dead or asleep cannot take the pearl, so it is discarded
                // where it lands (no teleport and no 5 damage).
                let taken = self.player.as_ref().is_some_and(|pl| {
                    horizon_client::projectiles::owner_can_take_pearl(
                        !pl.health.dead_or_dying(),
                        self.sleeping.is_some(),
                    )
                });
                if p.owner == Some(Who::Player) && taken && !cancel {
                    // ThrownEnderpearl: 5% endermite where the player was
                    // (when monsters spawn), teleport, 5 damage.
                    if self.world.random.next_float() < 0.05
                        && self.ui.rules.bool("spawn_monsters")
                        && self.ui.difficulty != 0
                        && let Some(pl) = self.player.as_ref()
                    {
                        let from = pl.position;
                        self.spawn_mob_raw("minecraft:endermite", [from.x, from.y, from.z], false);
                    }
                    // ServerPlayer.teleport: removeVehicle before the placement.
                    self.stop_riding();
                    if let Some(pl) = self.player.as_mut() {
                        pl.position = to;
                        pl.velocity = Vec3::ZERO;
                        pl.fall_distance = 0.0;
                        pl.impulse.reset();
                        pl.teleported = true;
                    }
                    if self.lan_acting.is_some() {
                        horizon_client::lan::hub::teleport_cause(Some("ender_pearl"));
                    }
                    self.previous_position = to;
                    if damage > 0.0 {
                        self.hurt_player_by("minecraft:ender_pearl", damage);
                    }
                    self.audio.play(
                        "minecraft:entity.player.teleport",
                        Some([to.x, to.y, to.z]),
                        1.0,
                        1.0,
                    );
                }
            }
            (ProjectileKind::SmallFireball, hit) => match hit {
                Hit::Entity { who, .. } => match who {
                    Who::Player => {
                        if !self.creative {
                            // SmallFireball.onHitEntity: remember the fire, ignite,
                            // and restore it when the hurt is refused (a raised shield).
                            let remaining = self.player.as_ref().map(|p| p.fire_ticks);
                            if let Some(p) = self.player.as_mut() {
                                p.ignite_for_ticks(100)?;
                            }
                            let hurt = self.mob_hits_player(
                                5.0,
                                [p.previous.x, p.previous.y, p.previous.z],
                                "minecraft:fireball",
                            );
                            if !hurt
                                && let Some(remaining) = remaining
                                && let Some(pl) = self.player.as_mut()
                            {
                                pl.set_remaining_fire_ticks(remaining);
                            }
                        }
                    }
                    Who::Mob(id) => {
                        let hurt = self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                            let remaining = m.fire_ticks;
                            if !m.kind.flags.fire_immune {
                                m.ignite_for_ticks(100);
                            }
                            let hurt =
                                m.damage_projectile(5.0, source, aim, [p.velocity.x, p.velocity.z]);
                            if !hurt {
                                m.fire_ticks = remaining;
                            }
                            hurt
                        });
                        if hurt {
                            self.player_hurt_mob(p, id);
                            self.shot_post_attack(
                                p,
                                who,
                                "minecraft:small_fireball",
                                "minecraft:fireball",
                            );
                        } else if !self.mobs.iter().any(|m| m.id == id) {
                            // SmallFireball.onHitEntity: 5 damage to a boat or minecart.
                            self.hurt_vehicle(p, id, 5.0);
                        }
                    }
                },
                Hit::Block { pos, face, .. } => {
                    // SmallFireball.onHitBlock: a mob's only with mobGriefing;
                    // BaseFireBlock.getState placed with updates (the fire
                    // ticks, ages and goes out where it can't stay).
                    let fire = [pos[0] + face[0], pos[1] + face[1], pos[2] + face[2]];
                    let mob = matches!(p.owner, Some(Who::Mob(_)));
                    if (!mob || self.ui.rules.bool("mob_griefing"))
                        && self
                            .res
                            .facts()
                            .is_air(self.world.block(fire[0], fire[1], fire[2]))
                    {
                        let at = BlockPos::new(fire[0], fire[1], fire[2]);
                        let state =
                            self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, at));
                        self.level_set(fire, state)?;
                    }
                }
                Hit::None => {}
            },
            (ProjectileKind::LargeFireball { power }, hit) => {
                // Fireball explosions are caused by the shooter. A direct hit
                // on the player takes `projectile_owner`, so read the owner
                // from the fireball.
                let owner = match p.owner {
                    Some(Who::Mob(id)) => Some(id),
                    _ => None,
                };
                if let Hit::Entity { who, .. } = hit {
                    match who {
                        Who::Player => {
                            self.mob_hits_player(
                                6.0,
                                [p.previous.x, p.previous.y, p.previous.z],
                                "minecraft:fireball",
                            );
                        }
                        Who::Mob(id) => {
                            if self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                                m.damage_projectile(6.0, source, aim, [p.velocity.x, p.velocity.z])
                            }) {
                                self.player_hurt_mob(p, id);
                            }
                            self.shot_post_attack(
                                p,
                                who,
                                "minecraft:fireball",
                                "minecraft:fireball",
                            );
                        }
                    }
                }
                self.combat.explosion_owner = owner;
                self.combat.explosion_in_water = p.touching_water;
                // The blast's source names the owner, so a dragon it kills
                // remembers the player who shot it.
                self.set_blast_by_player(p.owner == Some(Who::Player));
                // LargeFireball.onHit: both fire and MOB block destruction
                // follow mob_griefing; other mob explosions keep their default.
                let fire = self.ui.rules.bool("mob_griefing");
                let result = self.explode_mob(
                    atp,
                    *power as f32,
                    fire,
                    explosion::ExplosionSource::default()
                        .with_silence(self.entity_silent(p.id))
                        .with_entity(p.id, "minecraft:fireball"),
                );
                self.set_blast_by_player(false);
                self.combat.explosion_owner = None;
                self.combat.explosion_in_water = false;
                result?;
            }
            // AbstractThrownPotion.onHit, then onHitAsPotion from the stack it
            // flies as: the splash happens where the hit was, a lingering
            // potion's cloud sits at the mob it hit (ThrownLingeringPotion),
            // and the potion that lands names the hurt it deals.
            (kind @ (ProjectileKind::Potion(..) | ProjectileKind::ExperienceBottle), hit) => {
                // A bottle flies as the bottle item (`getDefaultItem`).
                let bottle;
                let stack = match kind {
                    ProjectileKind::Potion(stack, _) => stack,
                    _ => {
                        bottle = Box::new(ItemStack::new("minecraft:experience_bottle", 1));
                        &bottle
                    }
                };
                let block = match hit {
                    Hit::Block { pos, face, .. } => Some((
                        pos,
                        Direction::approximate_nearest(Vec3::new(
                            f64::from(face[0]),
                            f64::from(face[1]),
                            f64::from(face[2]),
                        )),
                    )),
                    _ => None,
                };
                let mob = match hit {
                    Hit::Entity {
                        who: Who::Player, ..
                    } => self.player.as_ref().map(|pv| pv.position),
                    Hit::Entity {
                        who: Who::Mob(id), ..
                    } => self.mobs.iter().find(|m| m.id == id).map(|m| m.position),
                    _ => None,
                };
                let owner = self.potion_owner(p.owner);
                let item = stack.item.to_string();
                let mut cancel = false;
                if stack.item.as_ref() != "minecraft:experience_bottle" {
                    self.native_potion_splash_v2(&item, [at.x, at.y, at.z], &mut cancel);
                }
                if !cancel {
                    self.splash(
                        stack,
                        at,
                        mob,
                        block,
                        p.age,
                        &owner,
                        self.entity_silent(p.id),
                        p.velocity,
                    );
                    if stack.item.as_ref() != "minecraft:experience_bottle" {
                        self.native_potion_splash(&item, [at.x, at.y, at.z]);
                    }
                }
            }
            // LlamaSpit.onHitEntity hurts only when the owner is a LivingEntity.
            (ProjectileKind::LlamaSpit, Hit::Entity { .. }) if p.owner.is_none() => {}
            (ProjectileKind::LlamaSpit, Hit::Entity { who, .. }) => match who {
                Who::Player => {
                    self.mob_hits_player(
                        1.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:spit",
                    );
                }
                Who::Mob(id) => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                        && m.damage_projectile(1.0, source, aim, [p.velocity.x, p.velocity.z])
                    {
                        self.shot_post_attack(p, who, "minecraft:llama_spit", "minecraft:spit");
                    }
                }
            },
            // ShulkerBullet.onHitEntity: 4 damage, then levitation for 10 s.
            (ProjectileKind::ShulkerBullet { .. }, Hit::Entity { who, .. }) => match who {
                Who::Player => {
                    // Only a hit that hurt (`wasHurt`) brings the levitation.
                    if !self.creative
                        && self.mob_hits_player(
                            4.0,
                            [p.previous.x, p.previous.y, p.previous.z],
                            "minecraft:mob_projectile",
                        )
                    {
                        self.give_player_effect("minecraft:levitation", 200, 0);
                    }
                }
                Who::Mob(id) => {
                    let hurt = self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                        let hurt =
                            m.damage_projectile(4.0, source, aim, [p.velocity.x, p.velocity.z]);
                        if hurt {
                            // Shulker.hurtServer: a shulker hit by a bullet.
                            horizon_client::mobs::shulker::hit_by_bullet(m);
                        }
                        hurt
                    });
                    if hurt {
                        // The hurt's teleport (`teleportSomewhere`,
                        // `hitByShulkerBullet`) comes before the levitation.
                        self.settle_hurt_teleports()?;
                        self.shot_post_attack(
                            p,
                            who,
                            "minecraft:shulker_bullet",
                            "minecraft:mob_projectile",
                        );
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            m.add_effect("minecraft:levitation", 200, 0);
                        }
                    }
                }
            },
            // DragonFireball.onHit (its own owner's hit never gets here): the
            // dragon's breath where it burst.
            (ProjectileKind::DragonFireball, _) => self.dragon_fireball_on_hit(at),
            (ProjectileKind::WitherSkull { .. }, hit) => {
                let who = match hit {
                    Hit::Entity { who, .. } => Some(who),
                    _ => None,
                };
                self.wither_skull_hit(p, who, atp)?;
            }
            (ProjectileKind::ShulkerBullet { .. }, Hit::Block { .. }) => {
                self.audio
                    .play("minecraft:entity.shulker_bullet.hit", Some(atp), 1.0, 1.0);
                if let Some(ty) = horizon_client::particles::ParticleType::from_name("explosion") {
                    let options = horizon_client::particles::ParticleOptions::simple(ty);
                    for _ in 0..2 {
                        let r = &mut self.world.random;
                        let at = [
                            atp[0] + r.next_gaussian() * 0.2,
                            atp[1] + r.next_gaussian() * 0.2,
                            atp[2] + r.next_gaussian() * 0.2,
                        ];
                        self.particles.add(&options, at, [0.0; 3]);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
