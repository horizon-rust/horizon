//! The overlays' per-entry lines (`DebugEntryLookingAt` and friends): each
//! entry reports its own lines, which the F3 overlay draws when the entry's
//! status lets it.
use super::Game;
use horizon_core::position::BlockPos;

impl Game<'_> {
    /// `DebugEntryLookingAt`: the crosshair's block, `Targeted Block: x, y, z`
    /// then the state's id and properties (`reach` 20 as the entry picks).
    pub(super) fn debug_looking_at_block(&self, tags: bool) -> Vec<String> {
        let mut out = Vec::new();
        let Some((origin, dir)) = self.aim() else {
            return Vec::new();
        };
        let Some(hit) = self.clip_ray(origin, dir, 20.0) else {
            return Vec::new();
        };
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let facts = self.res.facts().get(state);
        if tags {
            for (tag, members) in self.res.items().block_tags.iter() {
                if members.contains(&*facts.name) {
                    out.push(format!("#{tag}"));
                }
            }
        } else {
            out.push(format!(
                "§nTargeted Block: {}, {}, {}",
                hit.pos[0], hit.pos[1], hit.pos[2]
            ));
            out.push(facts.name.to_string());
            for (key, value) in &self.state_properties(state) {
                out.push(format!("{key}: {value}"));
            }
        }
        out
    }
    /// `DebugEntryLookingAt.FluidStateInfo`: the same for the fluid at the
    /// crosshair's block.
    pub(super) fn debug_looking_at_fluid(&self, tags: bool) -> Vec<String> {
        let mut out = Vec::new();
        let Some((origin, dir)) = self.aim() else {
            return Vec::new();
        };
        let Some(hit) = self.clip_ray(origin, dir, 20.0) else {
            return Vec::new();
        };
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let facts = self.res.facts().get(state);
        if facts.fluid == horizon_client::blocks::Fluid::None {
            return Vec::new();
        }
        if !tags {
            out.push(format!(
                "§nTargeted Fluid: {}, {}, {}",
                hit.pos[0], hit.pos[1], hit.pos[2]
            ));
        }
        out
    }
    /// The block state's properties, a boolean drawn green or red as
    /// `DebugEntryLookingAt.addStateProperties` does.
    fn state_properties(
        &self,
        state: horizon_world::registry::BlockStateId,
    ) -> Vec<(String, String)> {
        self.pack
            .assets
            .registry
            .state(state)
            .map(|s| {
                s.properties
                    .iter()
                    .map(|(k, v)| {
                        let shown = match v.as_str() {
                            "true" => "§atrue".to_owned(),
                            "false" => "§cfalse".to_owned(),
                            _ => v.clone(),
                        };
                        (k.clone(), shown)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    /// `DebugEntryLookingAtEntity`: `Targeted Entity` then the type's id, in
    /// the `looking_at_entity` group.
    pub(super) fn debug_looking_at_entity(&self) -> Vec<String> {
        let Some((index, _)) = self.mob_hit() else {
            return Vec::new();
        };
        let Some(mob) = self.mobs.get(index) else {
            return Vec::new();
        };
        vec!["§nTargeted Entity".to_owned(), mob.kind.id.clone()]
    }
    /// `DebugEntryLight`: `Client Light: raw (sky sky, block block)`.
    pub(super) fn debug_light_levels(&self) -> Vec<String> {
        let Some(p) = self.player.as_ref() else {
            return Vec::new();
        };
        let pos = BlockPos::new(
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        );
        let sky = i32::from(self.world.light(pos.x, pos.y, pos.z) >> 4);
        let block = i32::from(self.world.light(pos.x, pos.y, pos.z) & 15);
        vec![format!(
            "Client Light: {} ({sky} sky, {block} block)",
            sky.max(block)
        )]
    }
    /// `DebugEntryBiome`: `Biome: <id>` at the camera's feet.
    pub(super) fn debug_biome(&self) -> Vec<String> {
        let Some(p) = self.player.as_ref() else {
            return Vec::new();
        };
        self.biome_name_at(horizon_core::geometry::Vec3::new(
            p.position.x,
            p.position.y,
            p.position.z,
        ))
        .map(|name| vec![format!("Biome: {name}")])
        .unwrap_or_default()
    }
    /// `DebugEntryLocalDifficulty`: `Local Difficulty: %.2f // %.2f`, the
    /// difficulty then the special multiplier (`ClampedRegionalDifficulty`).
    pub(super) fn debug_local_difficulty(&self) -> Vec<String> {
        let Some(p) = self.player.as_ref() else {
            return Vec::new();
        };
        let at = [p.position.x, p.position.y, p.position.z];
        let local = self.local_difficulty(at);
        vec![format!(
            "Local Difficulty: {:.2} // {:.2}",
            local.effective,
            // `DifficultyInstance.getSpecialMultiplier`.
            local.special()
        )]
    }
    /// `DebugEntryDayCount`: `Day #<periods of 24000 game ticks>`.
    pub(super) fn debug_day_count(&self) -> Vec<String> {
        vec![format!("Day #{}", self.world.day_time.div_euclid(24000))]
    }
}
