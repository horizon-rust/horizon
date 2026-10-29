//! Wooden shelves: `ShelfBlock.useItemOn` (the held stack swapped with the slot
//! under the crosshair, or, on a powered chain, the hotbar swapped with the
//! whole chain), `ShelfBlockEntity.setChanged` and `ShelfRenderer`. The slots
//! are `containers::Shelf`; a server's clients get them through the block
//! entity's update tag (`lan::chunks::entity_tag`), which they draw from.
use super::{Game, Hit, item_draw::owner_at, machines::dir_from_name};
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    block_entity_render::vertex_light,
    containers::{BlockEntity, Shelf},
    item_render::ItemRenderState,
    items::ItemStack,
    mesher::Vertex,
};
use horizon_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
};
use horizon_world::level::hit_slot;

impl Game<'_> {
    /// `SelectableSlotContainer.getHitSlot` for the click on a shelf (one row
    /// of three slots on its front face), and whether the shelf is powered.
    /// None when the block is no shelf or the click is off the front face.
    fn shelf_hit(&self, hit: &Hit) -> Option<(usize, bool)> {
        let pos = hit.pos;
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let info = self
            .pack
            .assets
            .registry
            .state(state)
            .filter(|s| s.name.ends_with("_shelf"))?;
        let facing = info
            .properties
            .get("facing")
            .and_then(|f| dir_from_name(f))?;
        let powered = info.properties.get("powered").map(String::as_str) == Some("true");
        let face = Direction::ALL
            .into_iter()
            .find(|d| d.normal() == hit.face)?;
        let at = Vec3::new(hit.point[0], hit.point[1], hit.point[2]);
        let slot = hit_slot(
            facing,
            face,
            BlockPos::new(pos[0], pos[1], pos[2]),
            at,
            1,
            3,
        )?;
        Some((slot, powered))
    }

    /// `ShelfBlock.useItemOn` on a server's level with `held` in the acting
    /// hand: None when the block passes the click, else whether the use
    /// swings the arm (`SUCCESS_SERVER`; a powered chain with nothing to swap
    /// only consumes the click).
    pub(super) fn use_shelf(&mut self, hit: &Hit, held: Option<&ItemStack>) -> Option<bool> {
        // `!hand.equals(InteractionHand.OFF_HAND)`.
        if self.hand != horizon_client::first_person::Hand::Main {
            return None;
        }
        let (slot, powered) = self.shelf_hit(hit)?;
        let pos = hit.pos;
        let centre = pos.map(|v| f64::from(v) + 0.5);
        let sound = if powered {
            if !self.shelf_swap_hotbar(pos) {
                // `InteractionResult.CONSUME`.
                self.shelf_used_on(pos, held);
                return Some(false);
            }
            "minecraft:block.shelf.multi_swap"
        } else {
            let held_empty = held.is_none();
            let removed = self.shelf_swap_single_item(pos, slot)?;
            if removed {
                if held_empty {
                    "minecraft:block.shelf.take_item"
                } else {
                    "minecraft:block.shelf.single_swap"
                }
            } else if held_empty {
                return None;
            } else {
                "minecraft:block.shelf.place_item"
            }
        };
        self.audio.play(sound, Some(centre), 1.0, 1.0);
        self.shelf_used_on(pos, held);
        Some(true)
    }

    /// `ServerPlayerGameMode.useItemOn` after a block that consumed the click:
    /// `ITEM_USED_ON_BLOCK` with the stack as it was before the use (none for
    /// an empty hand).
    fn shelf_used_on(&mut self, pos: [i32; 3], held: Option<&ItemStack>) {
        if let Some(used) = held {
            self.trigger_block("minecraft:item_used_on_block", pos, used);
        }
    }

    /// `ShelfBlock.useItemOn` on a client of a server (`isClientSide`): a click
    /// on a slot with something in the selected slot is consumed, and the
    /// server does the swap. Whether this click is one.
    pub(super) fn shelf_consumes_click(&self, hit: &Hit) -> bool {
        let key = (hit.pos[0], hit.pos[1], hit.pos[2]);
        matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Shelf(_))
        ) && self.shelf_hit(hit).is_some()
            && self.stack_in(self.selected).is_some()
    }

    /// The shelf's block entity (made if the block has none yet).
    fn shelf_entity(&mut self, pos: [i32; 3]) -> Option<&mut Shelf> {
        let entity = self
            .world
            .block_entities
            .get_or_insert_with((pos[0], pos[1], pos[2]), || {
                BlockEntity::Shelf(Box::default())
            });
        match entity {
            BlockEntity::Shelf(shelf) => Some(&mut **shelf),
            _ => None,
        }
    }

    /// `ShelfBlock.swapSingleItem`: the selected stack goes into slot `slot`
    /// and what was there comes back into the selected slot (a player with
    /// infinite materials keeps the stack when the slot was empty), then the
    /// shelf's `setChanged`. Whether an item was taken out.
    fn shelf_swap_single_item(&mut self, pos: [i32; 3], slot: usize) -> Option<bool> {
        let selected = self.selected;
        let held = self.stack_in(selected).cloned();
        let limit = held
            .as_ref()
            .map_or(1, |stack| self.max_stack(stack).clamp(1, 99));
        let (removed, stored) = self
            .shelf_entity(pos)?
            .swap_item_no_update(slot, held, limit);
        let now = if self.creative && removed.is_none() {
            stored
        } else {
            removed.clone()
        };
        // An item whose `use_effects` has no `interact_vibrations` (a spear)
        // makes no event.
        let silent = now.as_ref().is_some_and(|stack| {
            self.res
                .items()
                .get(&stack.item)
                .is_some_and(|item| !item.use_interact_vibrations)
        });
        *self.stack_in_mut(selected) = now;
        self.shelf_set_changed(pos, (!silent).then_some("minecraft:item_interact_finish"));
        Some(removed.is_some())
    }

    /// `ShelfBlock.swapHotbar`: the hotbar slots swap with the slots of the
    /// powered chain the shelf at `pos` is part of, its left end first, the
    /// chain's last slot against hotbar slot 8. Whether anything moved.
    fn shelf_swap_hotbar(&mut self, pos: [i32; 3]) -> bool {
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let connected =
            self.with_level(|b, level, ticks| b.shelf_connected_blocks(level, ticks, at));
        if connected.is_empty() {
            return false;
        }
        let mut any_swapped = false;
        for (index, part) in connected.iter().enumerate() {
            let part = [part.x, part.y, part.z];
            if self.shelf_entity(part).is_none() {
                continue;
            }
            for slot in 0..3 {
                let inventory_slot = 9 - (connected.len() as i32 - index as i32) * 3 + slot as i32;
                let Ok(inventory_slot) = usize::try_from(inventory_slot) else {
                    continue;
                };
                if inventory_slot >= self.inventory.len() {
                    continue;
                }
                let placed = self.inventory[inventory_slot].take();
                let placed_any = placed.is_some();
                let limit = placed
                    .as_ref()
                    .map_or(1, |stack| self.max_stack(stack).clamp(1, 99));
                let Some(shelf) = self.shelf_entity(part) else {
                    self.inventory[inventory_slot] = placed;
                    continue;
                };
                let (removed, _) = shelf.swap_item_no_update(slot, placed, limit);
                if placed_any || removed.is_some() {
                    self.inventory[inventory_slot] = removed;
                    any_swapped = true;
                }
            }
            self.shelf_set_changed(part, Some("minecraft:entity_interact"));
        }
        any_swapped
    }

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
