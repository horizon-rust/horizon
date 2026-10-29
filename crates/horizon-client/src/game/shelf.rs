//! Wooden shelves: `ShelfBlock.useItemOn` (the held stack swapped with the slot
//! under the crosshair, or, on a powered chain, the hotbar swapped with the
//! whole chain), `ShelfBlockEntity.setChanged` and `ShelfRenderer`. The slots
//! are `containers::Shelf`; a server's clients get them through the block
//! entity's update tag (`lan::chunks::entity_tag`), which they draw from.
use super::Game;

impl Game<'_> {
    /// `ShelfBlockEntity.setChanged`: the chunk is saved and the comparators
    /// update (`BlockEntity.setChanged`), `event` goes out at the shelf with
    /// its state as context, and the clients get the new update tag
    /// (`sendBlockUpdated`).
    pub(super) fn shelf_set_changed(&mut self, pos: [i32; 3], event: Option<&'static str>) {
        let key = (pos[0], pos[1], pos[2]);
        self.block_entity_changed(key);
        if let Some(event) = event {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            self.post_block_game_event(event, pos, None, Some(state));
        }
        self.mark_block_entity_updated(key);
    }
}
