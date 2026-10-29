//! Wooden shelves: `ShelfBlock.useItemOn` (the held stack swapped with the slot
//! under the crosshair, or, on a powered chain, the hotbar swapped with the
//! whole chain), `ShelfBlockEntity.setChanged` and `ShelfRenderer`. The slots
//! are `containers::Shelf`; a server's clients get them through the block
//! entity's update tag (`lan::chunks::entity_tag`), which they draw from.
use super::{Game, item_draw::owner_at};
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    block_entity_render::vertex_light,
    containers::{BlockEntity, Shelf},
    item_render::ItemRenderState,
    mesher::Vertex,
};
use horizon_core::position::BlockPos;

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
    /// `ShelfRenderer.extractRenderState`: the item models of a shelf's slots
    /// (`updateForTopItem` for `ON_SHELF`, owned by the block entity and seeded
    /// by fastutil's long-to-int hash of the packed position plus the slot), its
    /// alignment, its facing and the light it stands in.
    fn shelf_extract(&self, key: (i32, i32, i32), shelf: &Shelf) -> ShelfRenderState {
        let state = self.world.block(key.0, key.1, key.2);
        // Direction.get2DDataValue: south 0, west 1, north 2, east 3.
        let facing: u8 = match self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("facing"))
            .map(String::as_str)
        {
            Some("west") => 1,
            Some("north") => 2,
            Some("east") => 3,
            _ => 0,
        };
        // `getVisualRotationYInDegrees` is the opposite facing's `toYRot`.
        let owner = owner_at(
            [
                f64::from(key.0) + 0.5,
                f64::from(key.1) + 0.5,
                f64::from(key.2) + 0.5,
            ],
            ((facing + 2) % 4) as f32 * 90.0,
            "",
        );
        let packed = BlockPos::new(key.0, key.1, key.2).pack();
        let seed = (packed ^ ((packed as u64) >> 32) as i64) as i32;
        let mut items: [Option<ItemRenderState>; 3] = Default::default();
        for (slot, stack) in shelf.items.iter().enumerate() {
            if let Some(stack) = stack {
                items[slot] = Some(self.resolve_item(
                    stack,
                    DisplayContext::OnShelf,
                    Some(&owner),
                    seed.wrapping_add(slot as i32),
                ));
            }
        }
        ShelfRenderState {
            items,
            align_to_bottom: shelf.align_items_to_bottom,
            facing,
            light: vertex_light(self.world.light(key.0, key.1, key.2)),
        }
    }
    /// `ShelfRenderer.submit`: every item of every shelf in range, turned to
    /// the shelf's front.
    pub(super) fn shelf_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        for (key, entity) in &self.world.block_entities {
            let BlockEntity::Shelf(shelf) = entity else {
                continue;
            };
            if shelf.items.iter().all(Option::is_none) {
                continue;
            }
            let at = glam::DVec3::new(f64::from(key.0), f64::from(key.1), f64::from(key.2));
            // `BlockEntityRenderer.shouldRender`: the block's centre within 64.
            if (at + glam::DVec3::splat(0.5)).distance_squared(camera) >= 64.0 * 64.0 {
                continue;
            }
            let state = self.shelf_extract(*key, shelf);
            // `-state.facing.toYRot()`.
            let y_rot = -(f32::from(state.facing) * 90.0);
            let origin = (at - camera).as_vec3();
            for (slot, item) in state.items.iter().enumerate() {
                if let Some(item) = item {
                    self.shelf_submit_item(&state, item, origin, slot, y_rot, out);
                }
            }
        }
    }
    /// `ShelfRenderer.submitItem`: the item in slot `slot` a quarter of its
    /// size, `(slot - 1) * 0.3125` along the shelf and a quarter block in
    /// front of its centre, centred on that point or, when aligned to the
    /// bottom, standing on the shelf board (`origin` is the block's corner
    /// relative to the camera).
    fn shelf_submit_item(
        &self,
        state: &ShelfRenderState,
        item: &ItemRenderState,
        origin: glam::Vec3,
        slot: usize,
        y_rot: f32,
        out: &mut Vec<Vertex>,
    ) {
        let slot_x = (slot as f32 - 1.0) * 0.3125;
        let slot_y = if state.align_to_bottom { -0.25 } else { 0.0 };
        let (lo, hi) = item.bounding_box();
        let mut offset_y = -lo.y;
        if !state.align_to_bottom {
            offset_y += -(hi.y - lo.y) / 2.0;
        }
        let pose = glam::Mat4::from_translation(origin + glam::Vec3::splat(0.5))
            * glam::Mat4::from_rotation_y(y_rot.to_radians())
            * glam::Mat4::from_translation(glam::Vec3::new(slot_x, slot_y, -0.25))
            * glam::Mat4::from_scale(glam::Vec3::splat(0.25))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, offset_y, 0.0));
        self.submit_item_world(item, pose, state.light, out);
    }
}

/// `ShelfRenderState`: what is drawn of one shelf.
struct ShelfRenderState {
    items: [Option<ItemRenderState>; 3],
    align_to_bottom: bool,
    /// The facing's 2D data value (south 0, west 1, north 2, east 3).
    facing: u8,
    light: u32,
}
