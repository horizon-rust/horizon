use super::*;

impl Game<'_> {
    /// `EndCrystal.hurtServer`: remove it, blast only for non-explosion damage,
    /// then notify the dragon fight even when no blast was created.
    pub(super) fn destroy_crystal(
        &mut self,
        index: usize,
        by: Option<FightWho>,
        damage: CrystalDamage,
        cause: Option<crate::game::explosion::ExplosionCause>,
    ) {
        if index >= self.crystals.len() || self.crystals[index].invulnerable {
            return;
        }
        let c = self.crystals.remove(index);
        // Read before the blast, which can take other respawn crystals.
        let respawn_crystal = self.fight.respawn.is_some()
            && self
                .fight
                .respawn_crystals
                .iter()
                .any(|p| p.distance_squared(c.position) < 0.01);
        if damage == CrystalDamage::Direct {
            let at = [c.position.x, c.position.y, c.position.z];
            // The blast's damage source carries the player, so the dragon
            // remembers them before the blast can kill it.
            self.fight.blast_by = by;
            // EndCrystal.hurtServer: ExplosionInteraction.BLOCK.
            let interaction = self.destroy_type("block_explosion_drop_decay");
            // EndCrystal is 2 by 2 blocks: its box decides `isInWater`.
            let (x, y, z) = (c.position.x, c.position.y, c.position.z);
            self.combat.explosion_in_water = self.bounds_in_water(Aabb::new(
                Vec3::new(x - 1.0, y, z - 1.0),
                Vec3::new(x + 1.0, y + 2.0, z + 1.0),
            ));
            let source = (
                c.id,
                self.explosion_entity_name(c.id, "minecraft:end_crystal"),
            );
            let silent = self.entity_silent(c.id);
            let _ = self.with_explosion_cause(cause, source, |g| {
                g.server_explode_from(
                    at,
                    6.0,
                    interaction,
                    false,
                    crate::game::explosion::ExplosionSource::default()
                        .with_silence(silent)
                        .with_entity(c.id, "minecraft:end_crystal"),
                )
            });
            self.combat.explosion_in_water = false;
            self.fight.blast_by = None;
        }
        self.crystal_destroyed(index, &c, by, respawn_crystal);
    }
}
