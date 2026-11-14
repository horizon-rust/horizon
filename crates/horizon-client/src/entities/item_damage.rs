//! `ItemEntity.hurtServer`'s hit for the damage the environment tick applies:
//! the health off, the contents of an item destroyed by it spilled, and the
//! `entity_damage` game event every hit posts (`ItemEntity.java:300-306`).
use super::ItemEntity;
use crate::items::ItemStack;

/// What the hits of one environment tick spilled, and how many there were: the
/// game posts `entity_damage` at the item's position for each of them.
#[derive(Default)]
pub struct ItemHits {
    pub spilled: Vec<ItemStack>,
    pub events: u32,
}

impl ItemHits {
    /// One `hurtServer`: `amount` off the item's health, and the stacks it
    /// leaves when that empties it (`ItemStack.onDestroyed`).
    pub fn hit(&mut self, item: &mut ItemEntity, amount: i32) {
        self.spilled.extend(item.hurt(amount).unwrap_or_default());
        self.events += 1;
    }
}
