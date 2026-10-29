//! Dropped items as a mob's AI sees them: the item entities within reach of
//! a goal's `getEntitiesOfClass(ItemEntity.class, ..)` (the fox's and the
//! dolphin's search goals).
use super::MobWorld;
use crate::entities::ItemEntity;
use horizon_core::geometry::Aabb;

impl MobWorld<'_> {
    /// `Level.getEntitiesOfClass(ItemEntity.class, area)`: the item entities
    /// whose box meets `area`, in the order the level lists them.
    pub fn items_in(&self, area: Aabb) -> impl Iterator<Item = &ItemEntity> {
        self.dropped
            .iter()
            .filter(move |e| e.stack.count > 0 && area.intersects(e.bounds()))
    }
    /// `Entity.isInWater` of a dropped item: its box touches water.
    pub fn item_in_water(&self, item: &ItemEntity) -> bool {
        self.fluids(item.bounds(), item.position.y).in_water
    }
}
