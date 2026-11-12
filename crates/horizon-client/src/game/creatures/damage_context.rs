use super::*;

impl Game<'_> {
    /// Melee or ranged damage to another mob; whether the hit landed.
    pub(in crate::game) fn mob_hits_mob(
        &mut self,
        target: u32,
        amount: f32,
        mut source: Damage,
    ) -> bool {
        if source.attacker_uuid.is_none()
            && let Some(Who::Mob(id)) = source.attacker
        {
            source.attacker_uuid = self.damage_attacker_uuid(Some(Who::Mob(id)), None);
        }
        if source.attacker_species.is_none()
            && let Some(Who::Mob(id)) = source.attacker
        {
            source.attacker_species = self.mobs.iter().find(|m| m.id == id).map(|m| m.species());
        }
        let mut killed = false;
        let mut hurt = false;
        let mut infested = Vec::new();
        let catch_up = self.catch_up_countdowns(source.attacker, target);
        let aim = self.attacker_aim(source.attacker);
        // So `living_knockback_v2` sees who hit and what they hold.
        let _attacker = self.knockback_scope_for(source.attacker);
        if let Some(t) = self.mobs.iter_mut().find(|m| m.id == target) {
            let previous_invulnerable = t.invulnerable;
            let hit = t.damage_aimed(amount, source, aim);
            hurt = hit;
            if hit && catch_up && t.invulnerable > previous_invulnerable {
                // `LivingEntity.baseTick`'s countdowns, run after the hit in vanilla.
                t.hurt_time = (t.hurt_time - 1).max(0);
                t.invulnerable = (t.invulnerable - 1).max(0);
            }
            if hit {
                killed = !t.alive();
                // The hurt or death sound the mob raised.
                let center = t.center();
                for event in std::mem::take(&mut t.outbox) {
                    match event {
                        MobEvent::Sound(s, _, volume, pitch) => {
                            self.audio
                                .play(&format!("minecraft:{s}"), Some(center), volume, pitch);
                        }
                        MobEvent::Infested { at, velocities } => infested.push((at, velocities)),
                        other => {
                            // Left for the end of the tick.
                            t.outbox.push(other);
                        }
                    }
                }
            }
        }
        if hurt {
            self.resolve_player_responsible(target, source);
        }
        for (at, velocities) in infested {
            self.spawn_infested(at, velocities);
        }
        if hurt
            && let Some(Who::Mob(attacker)) = source.attacker
            && let Some(m) = self
                .mobs
                .iter()
                .find(|m| m.id == attacker && m.species() == Species::Axolotl)
        {
            self.audio.play(
                "minecraft:entity.axolotl.attack",
                Some([m.position.x, m.position.y, m.position.z]),
                1.0,
                1.0,
            );
        }
        if killed && let Some(Who::Mob(killer)) = source.attacker {
            self.zombie_killed(killer, target);
        }
        hurt
    }
    pub(in crate::game) fn projectile_hit(&mut self, p: &Projectile, hit: Hit) -> Result<()> {
        let result = self.projectile_hit_inner(p, hit);
        // LivingEntity.actuallyHurt credits the shooter (a ServerPlayer) with the absorbed damage.
        self.settle_dealt_absorbed(p.owner == Some(Who::Player));
        result?;
        // The hit's hurt teleports an enderman or a shulker now (`hurtServer`).
        self.settle_hurt_teleports()
    }
}
