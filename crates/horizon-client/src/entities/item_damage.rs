//! `ItemEntity.hurtServer`'s hit for the damage the environment tick applies:
//! the health off, the contents of an item destroyed by it spilled, and the
//! `entity_damage` game event every hit posts (`ItemEntity.java:300-306`).
use super::ItemEntity;
use crate::items::ItemStack;

/// What the hits of one environment tick spilled, and the `entity_damage`
/// events they post.
#[derive(Default)]
pub struct ItemHits {
    pub spilled: Vec<ItemStack>,
    /// The block effects' hits, posted at the item's own position: they are
    /// applied after its move (`Entity.applyEffectsFromBlocks`).
    pub events: u32,
    /// `Entity.baseTick`'s burning, posted where the tick started from: it
    /// runs before `Entity.move`.
    pub burning_events: u32,
}

impl ItemHits {
    /// One `hurtServer`: `amount` off the item's health, and the stacks it
    /// leaves when that empties it (`ItemStack.onDestroyed`). A hit that
    /// destroyed the item ends the tick's hits: `Entity.checkInsideBlocks`
    /// stops at `!Entity.isAlive()` and `InsideBlockEffectApplier.applyAndClear`
    /// skips the effects still queued.
    fn strike(&mut self, item: &mut ItemEntity, amount: i32) -> bool {
        if item.health <= 0 {
            return false;
        }
        self.spilled.extend(item.hurt(amount).unwrap_or_default());
        true
    }

    /// A hit of the block pass, at the item's position.
    pub fn hit(&mut self, item: &mut ItemEntity, amount: i32) {
        if self.strike(item, amount) {
            self.events += 1;
        }
    }

    /// `Entity.baseTick`'s burning hit, at the position the tick started from.
    pub fn burning(&mut self, item: &mut ItemEntity, amount: i32) {
        if self.strike(item, amount) {
            self.burning_events += 1;
        }
    }
}
