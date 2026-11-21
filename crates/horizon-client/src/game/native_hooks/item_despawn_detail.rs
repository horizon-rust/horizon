use super::*;

impl Game<'_> {
    /// A dropped item entity reached its 6000-tick lifetime and is about to be removed (Bukkit's
    /// `ItemDespawnEvent`). Set `cancel` to keep it for another 6000 ticks. Server-side.
    pub(in crate::game) fn native_item_despawn_detail(
        &mut self,
        entity: u32,
        item: &str,
        count: i32,
        pos: [f64; 3],
        cancel: &mut bool,
    ) {
        self.fire_item_despawn_detail(entity, item, count, pos, cancel);
    }
}
