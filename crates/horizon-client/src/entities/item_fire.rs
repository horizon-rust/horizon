//! A dropped item's fire: `Entity.baseTick`'s burn and the part of
//! `Entity.applyEffectsFromBlocks` that puts it out — water (`WaterFluid.entityInside`)
//! and rain — with the hiss `playEntityOnFireExtinguishedSound` plays.
use super::{Fluid, ItemEntity};
use crate::items::{ItemStack, Items};

impl ItemEntity {
    /// `Entity.baseTick`'s fire ticks: a fire-immune stack never burns, else
    /// every 20th tick of fire hurts it and the countdown runs down.
    ///
    /// Returns the stacks a lethal burn left (`ItemStack.onDestroyed`) and
    /// whether it is on fire still: the `wasOnFire` that
    /// `Entity.applyEffectsFromBlocks` snapshots before the block effects.
    pub fn base_tick_fire(
        &mut self,
        fluid: Fluid,
        fire_immune: bool,
        items: &Items,
    ) -> (Vec<ItemStack>, bool) {
        let mut spilled = Vec::new();
        if fire_immune {
            self.fire_ticks = 0;
        }
        if self.fire_ticks > 0 {
            if self.fire_ticks % 20 == 0
                && fluid != Fluid::Lava
                && !items.resists_fire(&self.stack, "minecraft:on_fire")
            {
                spilled.extend(self.hurt(1).unwrap_or_default());
            }
            self.fire_ticks -= 1;
        }
        (spilled, self.fire_ticks > 0)
    }

    /// `Entity.applyEffectsFromBlocks`: water (`WaterFluid.entityInside`) and
    /// rain clear a burning item's fire, after the block contacts that may have
    /// ignited it. Returns whether the hiss plays:
    /// `playEntityOnFireExtinguishedSound` for an item that burned before the
    /// block effects and does not now.
    pub fn extinguish_if_wet(&mut self, in_water: bool, in_rain: bool, was_on_fire: bool) -> bool {
        if in_water || in_rain {
            // Entity.clearFire keeps only a fire-immune countdown.
            self.fire_ticks = self.fire_ticks.min(0);
        }
        was_on_fire && self.fire_ticks <= 0
    }
}
