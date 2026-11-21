//! Shulker boxes (`ShulkerBoxBlockEntity`: 27 slots that refuse other boxes,
//! kept in the dropped item's `container` component) and ender chests (the
//! player's `PlayerEnderChestContainer`, saved as `EnderItems`).
use super::Game;
use horizon_client::{
    containers::{BlockEntity, ChestHalf},
    items::ItemStack,
    text::Text,
};
use horizon_core::random::RandomSource;
use horizon_world::{
    level::{Behaviours, classes::Class},
    registry::BlockStateId,
};

/// The blocks a spectator's click opens a menu for (`BlockState.getMenuProvider`
/// up to `BaseContainerBlockEntity.canOpen`): vanilla asks each block's class, so
/// a content block of a container's template answers for it, and a block whose id
/// only ends in a container's name does not.
fn spectator_container(behaviours: &Behaviours, state: BlockStateId) -> bool {
    if is_shulker_box(behaviours, state) || behaviours.is_chest_block(state) {
        return true;
    }
    let data = behaviours.data(state);
    [
        Class::Barrel,
        Class::Hopper,
        Class::Dispenser,
        Class::Dropper,
        Class::AbstractFurnace,
        Class::BrewingStand,
        Class::Crafter,
    ]
    .into_iter()
    .any(|class| data.is(class))
}

impl Game<'_> {
    /// `ServerPlayerGameMode.useItemOn` for a spectator: the block's menu
    /// (`BlockState.getMenuProvider`) opens, and nothing else happens. The
    /// lid, its sound and the `container_open` game event come from
    /// `ContainerOpenersCounter`, which spectators never join
    /// (`ChestBlockEntity.startOpen`); the stat, the piglins' anger and the
    /// shulker box's room check are `useWithoutItem`'s, which is not called.
    /// Only the block containers whose clicks the server takes back from a
    /// spectator (`handleContainerClick`) open here: storage, the furnaces,
    /// the brewing stand and the crafter.
    pub(super) fn spectator_open_menu(&mut self, pos: [i32; 3]) {
        if self.fake_blocks(mod_api::FakeEffect::Menu) {
            return;
        }
        let key = (pos[0], pos[1], pos[2]);
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let facts = self.res.facts();
        let name = facts.get(state).name.clone();
        let container = spectator_container(self.res.behaviours(), state);
        // ChestBlock.getMenuProvider: none while either half is blocked.
        if !container || self.chest_menu_blocked(key, &name) {
            return;
        }
        // BaseContainerBlockEntity.canOpen: a spectator opens no container that still
        // has a loot table to unpack (it clears while it rolls, on first open).
        // ServerPlayer.openMenu sends the refused spectator a red action bar.
        if self
            .world
            .chunks
            .get(&(key.0 >> 4, key.2 >> 4))
            .and_then(|c| c.saved.as_ref())
            .is_some_and(|s| s.pending_loot.contains_key(&key))
        {
            self.send_overlay_message(
                Text::translate("container.spectatorCantOpen", &[])
                    .with_color(horizon_client::text::RED),
            );
            return;
        }
        let Some(entity) = BlockEntity::for_block(&name) else {
            return;
        };
        self.world.block_entities.get_or_insert_with(key, || entity);
        self.container = Some(key);
        self.open_container_slots = Some((key, self.open_container_size()));
        self.set_screen(super::Screen::Inventory);
        self.capture(false);
        self.right = false;
    }
    /// The custom name of the open container, or its default title
    /// (a double chest is named by whichever half has a name).
    pub(super) fn container_title(&self, default: String) -> String {
        self.container_parts()
            .iter()
            .find_map(|(key, _)| self.block_name_plain(*key))
            .unwrap_or(default)
    }
    /// `BaseContainerBlockEntity.createMenu` (and `ChestBlock`'s provider for
    /// a double chest, which asks both halves' `canOpen`): a `lock` the opener
    /// cannot unlock refuses the menu with `sendChestLockedNotifications` -
    /// the action bar names the container and one chest-locked sound plays.
    /// True when the open is refused.
    pub(super) fn refuse_locked_container(&mut self, key: (i32, i32, i32), name: &str) -> bool {
        // A carrier transfer fence holds this cell: its block entity's
        // container does not open while the operation runs.
        if self.world.carrier_fences.locks_cell(&[key.0, key.1, key.2]) {
            return true;
        }
        let Some(default_key) = horizon_client::containers::default_name_key(name) else {
            return false;
        };
        // `LockCode.canUnlock` reads `Player.getMainHandItem`, whichever
        // hand the click came from.
        let hand = self.stack_in(self.selected).cloned();
        let spectator = self.spectator();
        // The halves in `DoubleBlockCombiner` order: the right half first.
        let halves = match self.chest_partner(key) {
            Some((other, true)) => [Some(key), Some(other)],
            Some((other, false)) => [Some(other), Some(key)],
            None => [Some(key), None],
        };
        if !halves
            .iter()
            .flatten()
            .any(|&half| self.container_locked_for(half, spectator, hand.as_ref()))
        {
            return false;
        }
        // The display name of the provider the player tried to open: a double
        // chest is named by its right half, else its left, else
        // `container.chestDouble`; any other container by its custom name or
        // its own default key.
        let title = match (halves[0], halves[1]) {
            (Some(first), Some(second)) => horizon_client::containers::double_chest_name(
                self.block_name_plain(first).as_deref(),
                self.block_name_plain(second).as_deref(),
            ),
            (Some(one), None) => horizon_client::containers::display_name(
                self.block_name_plain(one).as_deref(),
                default_key,
            ),
            _ => horizon_client::containers::display_name(None, default_key),
        };
        let title = horizon_client::component::name_text(&title);
        self.send_overlay_message(Text::translate("container.isLocked", &[title]));
        // `Level.playSound(null, pos, SoundEvents.CHEST_LOCKED, BLOCKS, 1, 1)`:
        // the block's centre, or between the halves of a double chest.
        let at = match (halves[0], halves[1]) {
            (Some(first), Some(second)) => [
                f64::from(first.0 + second.0) / 2.0 + 0.5,
                f64::from(first.1) + 0.5,
                f64::from(first.2 + second.2) / 2.0 + 0.5,
            ],
            (Some(one), None) => [
                f64::from(one.0) + 0.5,
                f64::from(one.1) + 0.5,
                f64::from(one.2) + 0.5,
            ],
            _ => [0.0; 3],
        };
        self.audio
            .play("minecraft:block.chest.locked", Some(at), 1.0, 1.0);
        true
    }
    /// ShulkerBoxSlot.mayPlace: no shulker box inside a shulker box
    /// (`ItemStack.canFitInsideContainerItems`, whose vanilla answer is
    /// `!(block instanceof ShulkerBoxBlock)`).
    pub(super) fn refuses(&self, slot: usize, stack: &ItemStack) -> bool {
        (300..400).contains(&slot)
            && !self.stack_fits_inside_container_items(stack)
            && self.container.is_some_and(|k| {
                let state = self.world.block(k.0, k.1, k.2);
                is_shulker_box(self.res.behaviours(), state)
            })
    }
    /// `ItemStack.canFitInsideContainerItems` for a stack whose item this client
    /// has: the block the item places decides (`ItemInfo.block`), which is the
    /// rule in `menu::container_items`.
    fn stack_fits_inside_container_items(&self, stack: &ItemStack) -> bool {
        horizon_client::menu::container_items::vanilla_fits_inside_container_items_for_item(
            self.res.items(),
            self.res.facts(),
            stack,
        )
    }
    /// Opening an ender chest shows the player's ender inventory in its place.
    pub(super) fn open_ender_chest(&mut self, key: (i32, i32, i32)) {
        self.world
            .block_entities
            .insert(key, BlockEntity::Chest(self.ender_items.clone()));
        self.container = Some(key);
        self.ender_open = Some(key);
    }
    /// The block above `key` is a redstone conductor.
    fn conductor_above(&self, key: (i32, i32, i32)) -> bool {
        let above = self.world.block(key.0, key.1 + 1, key.2);
        self.res.behaviours().facts(above).flags
            & horizon_world::level::data::state_flags::CONDUCTOR
            != 0
    }
    /// `ChestBlock.isChestBlockedAt`: a redstone conductor above the chest or
    /// a cat sitting on it.
    fn chest_blocked_at(&self, key: (i32, i32, i32)) -> bool {
        self.conductor_above(key) || super::machines::cat_sitting_above(&self.mobs, key)
    }
    /// `ChestBlock.getMenuProvider` (`combine(.., false)`) and
    /// `EnderChestBlock.useWithoutItem`: a blocked chest, or a double chest
    /// with either half blocked, opens no menu.
    pub(super) fn chest_menu_blocked(&self, key: (i32, i32, i32), name: &str) -> bool {
        if self.res.behaviours().is_chest_block(self.world.block(key.0, key.1, key.2)) {
            return self.chest_blocked_at(key)
                || self
                    .chest_partner(key)
                    .is_some_and(|(other, _)| self.chest_blocked_at(other));
        }
        // The ender chest only looks at the block above (no cat check).
        if name == "minecraft:ender_chest" {
            return self.conductor_above(key);
        }
        false
    }
    /// Closing it copies the items back and keeps the block entity, as vanilla's
    /// `EnderChestBlockEntity` persists across open/close (`close_ender_chest`
    /// must not remove it, or `/data get block` loses it after a use).
    pub(super) fn close_ender_chest(&mut self) {
        let Some(key) = self.ender_open.take() else {
            return;
        };
        // The items go back to the player: an `EnderChestBlockEntity` holds none, so
        // nothing of theirs stays in the chunk or in `/data get block`.
        if let Some(BlockEntity::Chest(items)) = self.world.block_entities.get_mut(&key) {
            self.ender_items = std::mem::replace(items, vec![None; 27]);
        }
    }
}

// ---- chest menus: double chests, titles and shift-click ----

/// A chest block state's half: its block, `type` and `facing`.
fn chest_half(state: &horizon_world::chunk::BlockState) -> Option<ChestHalf<'_>> {
    Some(ChestHalf {
        block: &state.name,
        kind: state.properties.get("type")?,
        facing: state.properties.get("facing")?,
    })
}
fn facing_step(facing: &str) -> (i32, i32) {
    match facing {
        "north" => (0, -1),
        "south" => (0, 1),
        "west" => (-1, 0),
        _ => (1, 0),
    }
}

/// Java's `state.getBlock() instanceof ShulkerBoxBlock`, for this client's
/// tables: the block's class, which a content block answers with its template's.
/// The client's own door to that answer. Its name and place are kept because the
/// mod point `horizon_client::game::storage::is_shulker_box` and its entry in the
/// generated `internals.json` may not disappear (`docs/layers.md#mod-points-stay-put`,
/// `tools/check_mod_points_unchanged.py`); the arguments the class answer needs
/// changed that point, and `tools/mod_points_removed.txt` lists it.
pub(super) fn is_shulker_box(behaviours: &Behaviours, state: BlockStateId) -> bool {
    behaviours.is_shulker_box(state)
}

/// `ChestBlock.getConnectedBlockPos` for a chest whose `type` is not single:
/// where its other half belongs, whatever stands there now. `state` is the
/// block there now and `id` its state, which `facts` answers for: vanilla asks
/// the block's class, never its name.
pub(super) fn chest_connected_pos(
    behaviours: &Behaviours,
    state: &horizon_world::chunk::BlockState,
    id: BlockStateId,
    pos: [i32; 3],
) -> Option<[i32; 3]> {
    if !behaviours.is_chest_block(id) {
        return None;
    }
    let (dx, dz) = chest_half(state)?.connected_step()?;
    Some([pos[0] + dx, pos[1], pos[2] + dz])
}

impl Game<'_> {
    /// The other half of the open double chest and whether the open one is
    /// the right half (`ChestBlock.getConnectedDirection`).
    pub(super) fn chest_partner(&self, key: (i32, i32, i32)) -> Option<((i32, i32, i32), bool)> {
        let registry = self.pack.assets.registry;
        let id = self.world.block(key.0, key.1, key.2);
        let state = registry.state(id)?;
        if !self.res.behaviours().is_chest_block(id) {
            return None;
        }
        let own = chest_half(state)?;
        let (dx, dz) = own.connected_step()?;
        let other = (key.0 + dx, key.1, key.2 + dz);
        let other_state = registry.state(self.world.block(other.0, other.1, other.2))?;
        let paired = own.pairs_with(&chest_half(other_state)?)
            && matches!(
                self.world.block_entities.get(&other),
                Some(BlockEntity::Chest(items)) if items.len() == 27
            );
        paired.then_some((other, own.kind == "right"))
    }
    /// The open or close sound of a container block (`ChestBlockEntity.playSound`,
    /// `BarrelBlockEntity.playSound`, `ShulkerBoxBlockEntity.startOpen`): volume
    /// 0.5 and pitch 0.9 to 1.0. A double chest sounds once from the middle of
    /// both halves, a barrel from the face it opens on.
    pub(super) fn container_sound(&mut self, sound: &str, key: (i32, i32, i32), name: &str) {
        let mut at = [
            f64::from(key.0) + 0.5,
            f64::from(key.1) + 0.5,
            f64::from(key.2) + 0.5,
        ];
        if let Some((other, _)) = self.chest_partner(key) {
            at[0] = f64::from(key.0 + other.0) / 2.0 + 0.5;
            at[2] = f64::from(key.2 + other.2) / 2.0 + 0.5;
        } else if name == "minecraft:barrel"
            && let Some(state) = self
                .pack
                .assets
                .registry
                .state(self.world.block(key.0, key.1, key.2))
            && let Some(facing) = state.properties.get("facing")
        {
            let (dx, dy, dz) = match facing.as_str() {
                "up" => (0.0, 0.5, 0.0),
                "down" => (0.0, -0.5, 0.0),
                f => {
                    let (x, z) = facing_step(f);
                    (f64::from(x) / 2.0, 0.0, f64::from(z) / 2.0)
                }
            };
            at = [at[0] + dx, at[1] + dy, at[2] + dz];
        }
        let pitch = self.world.random.next_float() * 0.1 + 0.9;
        // A server's `playSound(null, …)`: every player around, the opener too.
        // `Audio::play` captures the sound while the world is open to the
        // network and `lan_broadcast_effects` sends that tick to every guest,
        // so nothing is queued separately (a second queue played it twice).
        self.audio.play(sound, Some(at), 0.5, pitch);
    }
    /// The open or close sound of the container block at `key`, if it has one.
    /// Callers play it only when the opener count crosses zero
    /// (`ContainerOpenersCounter.onOpen` / `onClose`).
    pub(super) fn container_edge_sound(&mut self, key: (i32, i32, i32), open: bool) {
        let id = self.world.block(key.0, key.1, key.2);
        let facts = self.res.facts();
        let name = facts.get(id).name.clone();
        let sound = if is_shulker_box(self.res.behaviours(), id) {
            if open {
                "minecraft:block.shulker_box.open"
            } else {
                "minecraft:block.shulker_box.close"
            }
        } else if &*name == "minecraft:ender_chest" {
            if open {
                "minecraft:block.ender_chest.open"
            } else {
                "minecraft:block.ender_chest.close"
            }
        } else if let Some(sound) = horizon_client::mobs::transport_items::chest_hinge_sound(
            &name,
            self.res.behaviours().is_chest_block(id),
            open,
        ) {
            sound
        } else if &*name == "minecraft:barrel" {
            if open {
                "minecraft:block.barrel.open"
            } else {
                "minecraft:block.barrel.close"
            }
        } else {
            return;
        };
        self.container_sound(sound, key, &name);
    }
    /// The other half of a double chest at `key`, for neighbour updates.
    pub(super) fn chest_other_half(&self, key: (i32, i32, i32)) -> Option<(i32, i32, i32)> {
        self.chest_partner(key).map(|p| p.0)
    }
    /// The block entities of the open container in menu order, with their
    /// sizes: a double chest is its right half then its left half.
    pub(super) fn container_parts(&self) -> Vec<((i32, i32, i32), usize)> {
        let Some(key) = self.container else {
            return Vec::new();
        };
        let Some(entity) = self.world.block_entities.get(&key) else {
            return Vec::new();
        };
        let own = (key, entity.slots().len());
        match self.chest_partner(key) {
            Some((other, true)) => vec![own, (other, 27)],
            Some((other, false)) => vec![(other, 27), own],
            None => vec![own],
        }
    }
    /// `Container.stillValidBlockEntity`: the open block container closes once
    /// the player's eyes are out of reach of any of its blocks (a 4-block buffer).
    pub(super) fn tick_container_reach(&mut self) {
        // A server's menu ends when the server closes it.
        if self.remote.is_some() {
            return;
        }
        let Some(eye) = self.eye_position() else {
            return;
        };
        if self
            .container
            .is_none_or(|k| k.1 == super::minecarts::CART_KEY_Y)
        {
            return;
        }
        // `Container.stillValidBlockEntity`: the menu holds a block entity per
        // part, and a part that went away (`getBlockEntity(pos) != this`) ends
        // it. A double chest whose other half broke would otherwise show a
        // shorter container.
        if self.open_container_slots.is_some_and(|(open, slots)| {
            self.container == Some(open) && self.open_container_size() < slots
        }) {
            self.close_container();
            self.screen = super::Screen::Playing;
            self.capture(true);
            return;
        }
        let range = self.reach() + 4.0;
        let near = self.container_parts().iter().all(|&((x, y, z), _)| {
            horizon_client::lan::guest::within_block_range(eye, range, [x, y, z])
        });
        if !near {
            self.close_container();
            self.set_screen(super::Screen::Playing);
            self.capture(true);
        }
    }
    /// `CraftingMenu.stillValid`: the open crafting table closes once its
    /// block is no longer a crafting table or the player's eyes are out of
    /// reach (a 4-block buffer).
    pub(super) fn tick_craft_table(&mut self) {
        // A server's menu ends when the server closes it.
        if self.remote.is_some() || self.craft_size != 3 {
            return;
        }
        let (Some(pos), Some(eye)) = (self.craft_table, self.eye_position()) else {
            return;
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let table = &*self.res.facts().get(state).name == "minecraft:crafting_table";
        let range = self.reach() + 4.0;
        if !(table && horizon_client::lan::guest::within_block_range(eye, range, pos)) {
            self.close_container();
            self.set_screen(super::Screen::Playing);
            self.capture(true);
        }
    }
    /// The number of slots of the open container.
    pub(super) fn open_container_size(&self) -> usize {
        self.container_parts().iter().map(|p| p.1).sum()
    }
    pub(super) fn container_slot(&self, mut n: usize) -> &Option<ItemStack> {
        for (pos, size) in self.container_parts() {
            if n < size {
                return self
                    .world
                    .block_entities
                    .get(&pos)
                    .and_then(|e| e.slots().get(n))
                    .unwrap_or(&self.scratch_slot);
            }
            n -= size;
        }
        &self.scratch_slot
    }
    /// Container slot `n` (menu slot `300 + n`).
    pub(super) fn container_slot_mut(&mut self, mut n: usize) -> &mut Option<ItemStack> {
        let mut target = None;
        for (pos, size) in self.container_parts() {
            if n < size {
                target = Some((pos, n));
                break;
            }
            n -= size;
        }
        match target.and_then(|(pos, i)| {
            self.world
                .block_entities
                .get_mut(&pos)
                .and_then(|e| e.slots_mut().get_mut(i))
        }) {
            Some(slot) => slot,
            None => &mut self.scratch_slot,
        }
    }
    pub(super) fn container_block_is_shulker(&self) -> bool {
        self.container_state()
            .is_some_and(|id| is_shulker_box(self.res.behaviours(), id))
    }
    /// The state of the open container's block.
    fn container_state(&self) -> Option<horizon_world::registry::BlockStateId> {
        let k = self.container?;
        Some(self.world.block(k.0, k.1, k.2))
    }
    fn container_block(&self) -> Option<std::sync::Arc<str>> {
        let k = self.container?;
        Some(
            self.res
                .facts()
                .get(self.world.block(k.0, k.1, k.2))
                .name
                .clone(),
        )
    }
    /// Rows of the open generic 9-wide container (`ChestMenu`), or None for
    /// the other menus (hoppers, dispensers, shulker boxes, furnaces...).
    pub(super) fn chest_rows(&self) -> Option<usize> {
        if let Some((rows, _)) = self.remote_chest_menu() {
            return Some(rows);
        }
        if !matches!(self.container_kind()?, BlockEntity::Chest(_)) {
            return None;
        }
        let generic = match self.container_block().as_deref() {
            Some("minecraft:barrel" | "minecraft:ender_chest") => true,
            _ if self
                .container_state()
                .is_some_and(|id| self.res.behaviours().is_chest_block(id)) =>
            {
                true
            }
            _ => self.open_cart_kind() == Some(super::minecarts::CartKind::Chest),
        };
        generic.then(|| self.open_container_size() / 9)
    }
    /// The title of the open chest-like menu.
    fn chest_title(&self, rows: usize) -> Text {
        if let Some((_, title)) = self.remote_chest_menu() {
            return title;
        }
        let (key, fallback) = match self.container_block().as_deref() {
            Some("minecraft:barrel") => ("container.barrel", "Barrel"),
            Some("minecraft:ender_chest") => ("container.enderchest", "Ender Chest"),
            _ if self.open_cart_kind().is_some() => {
                ("entity.minecraft.chest_minecart", "Minecart with Chest")
            }
            _ if rows > 3 => ("container.chestDouble", "Large Chest"),
            _ => ("container.chest", "Chest"),
        };
        Text::literal(self.container_title(self.translate_with_fallback(key, fallback)))
            .resolve_legacy()
    }
    /// `ContainerScreen` / `ShulkerBoxScreen`: the background with its
    /// container rows, the title and the inventory label.
    pub(super) fn draw_chest_screen(
        &self,
        batch: &mut horizon_client::hud::HudBatch,
        ox: f32,
        oy: f32,
    ) {
        use horizon_client::hud::rgba;
        let atlas = &self.gui.atlas;
        let grey = rgba(64, 64, 64, 255);
        if self
            .container_state()
            .is_some_and(|id| is_shulker_box(self.res.behaviours(), id))
        {
            batch.sprite_region(
                atlas,
                "gui/container/shulker_box",
                [ox, oy, 176.0, 167.0],
                256.0,
                256.0,
            );
            self.draw_screen_phase(batch, horizon_client::screen_draw::RenderPhase::Background);
            // ShulkerBoxScreen draws the menu's title: the box's custom name
            // (BaseContainerBlockEntity.getDisplayName) or its default name.
            let title = self.remote_menu_title().unwrap_or_else(|| {
                Text::literal(self.container_title(
                    self.translate_with_fallback("container.shulkerBox", "Shulker Box"),
                ))
                .resolve_legacy()
            });
            super::screen_info::name_container(|| title.plain());
            batch.text_styled(atlas, &title, ox + 8.0, oy + 6.0, grey, false);
            let inventory = self.translate_with_fallback("container.inventory", "Inventory");
            batch.text(atlas, &inventory, ox + 8.0, oy + 73.0, grey, false);
            return;
        }
        let rows = self.chest_rows().unwrap_or(3);
        let top = rows as f32 * 18.0 + 17.0;
        batch.sprite_region(
            atlas,
            "gui/container/generic_54",
            [ox, oy, 176.0, top],
            256.0,
            256.0,
        );
        // The player half of the texture starts at v = 126.
        if let Some(mut r) = atlas.sprites.get("gui/container/generic_54").copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.v0 += 126.0 * dv;
            r.v1 = r.v0 + 96.0 * dv;
            r.u1 = r.u0 + 176.0 * du;
            batch.rect_uv(ox, oy + top, 176.0, 96.0, r, u32::MAX);
        }
        self.draw_screen_phase(batch, horizon_client::screen_draw::RenderPhase::Background);
        let title = self.chest_title(rows);
        super::screen_info::name_container(|| title.plain());
        batch.text_styled(atlas, &title, ox + 8.0, oy + 6.0, grey, false);
        let inventory = self.translate_with_fallback("container.inventory", "Inventory");
        batch.text(
            atlas,
            &inventory,
            ox + 8.0,
            oy + 114.0 + rows as f32 * 18.0 - 94.0,
            grey,
            false,
        );
    }
}

impl Game<'_> {
    /// `quickMoveStack` of the open chest-like, hopper, dispenser or furnace
    /// menu for a shift-click on `slot`, run by the verified menu engine
    /// (`horizon_client::menu`); false when the menu has none.
    pub(super) fn container_quick_move(&mut self, slot: usize) -> bool {
        self.click_open_inventory(Some(slot), 0, horizon_client::menu::Input::QuickMove)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `BlockState.getMenuProvider`: vanilla asks the block's class, so a copper
    /// chest (a `ChestBlock`) opens for a spectator, an ender chest's
    /// `EnderChestBlock` does not, and a block of another class whose id ends in
    /// `chest` does not either.
    #[test]
    #[ignore = "daily"]
    fn a_spectators_containers_are_asked_by_class() {
        let mut resources = None;
        let game = super::super::game_tests::tests::server_game(&mut resources);
        let behaviours = game.res.behaviours();
        let registry = game.pack.assets.registry;
        for (block, opens) in [
            ("minecraft:chest", true),
            ("minecraft:trapped_chest", true),
            ("minecraft:copper_chest", true),
            ("minecraft:waxed_exposed_copper_chest", true),
            ("minecraft:barrel", true),
            ("minecraft:hopper", true),
            ("minecraft:furnace", true),
            ("minecraft:blast_furnace", true),
            ("minecraft:smoker", true),
            ("minecraft:dispenser", true),
            ("minecraft:dropper", true),
            ("minecraft:brewing_stand", true),
            ("minecraft:crafter", true),
            ("minecraft:shulker_box", true),
            ("minecraft:ender_chest", false),
            ("minecraft:crafting_table", false),
        ] {
            let state = registry.default_state(block).unwrap();
            assert_eq!(spectator_container(behaviours, state), opens, "{block}");
        }
    }

    /// `ChestBlockEntity`'s opener counter plays the block's own hinge
    /// sound: a copper chest's, by weather state, not the wooden chest's.
    #[test]
    #[ignore = "daily"]
    fn a_player_opens_and_closes_a_chest_with_its_own_hinge_sound() {
        let mut resources = None;
        let mut game = super::super::game_tests::tests::server_game(&mut resources);
        for (block, sound) in [
            ("minecraft:chest", "minecraft:block.chest"),
            (
                "minecraft:exposed_copper_chest",
                "minecraft:block.copper_chest",
            ),
            (
                "minecraft:waxed_oxidized_copper_chest",
                "minecraft:block.copper_chest_oxidized",
            ),
        ] {
            let state = game.pack.assets.registry.default_state(block).unwrap();
            game.world.set(8, 64, 8, state).unwrap();
            game.audio.heard.clear();
            game.container_edge_sound((8, 64, 8), true);
            game.container_edge_sound((8, 64, 8), false);
            let heard: Vec<_> = game.audio.heard.keys().cloned().collect();
            assert_eq!(
                heard,
                [format!("{sound}.close"), format!("{sound}.open")],
                "{block}"
            );
        }
    }

    /// `ShulkerBoxBlock.canOpen`: a box whose lid has no room opens neither its
    /// menu nor anything else (`useWithoutItem` returns before the screen).
    #[test]
    #[ignore = "daily"]
    fn a_shulker_box_with_a_blocked_lid_opens_nothing() {
        let mut resources = None;
        let mut game = super::super::game_tests::tests::server_game(&mut resources);
        let registry = game.pack.assets.registry;
        let shulker = registry.default_state("minecraft:shulker_box").unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        let key = (8, 64, 8);
        game.world.set(key.0, key.1, key.2, shulker).unwrap();
        game.world.set(key.0, key.1 + 1, key.2, stone).unwrap();
        game.open_block_effect_menu([8, 64, 8]);
        assert_eq!(game.container, None, "a blocked shulker box opened its menu");
        assert!(
            !matches!(game.screen, crate::game::Screen::Inventory),
            "a blocked shulker box opened a screen"
        );
    }

    #[test]
    fn double_chest_halves_connect_around_the_facing() {
        let half = |kind, facing| ChestHalf {
            block: "minecraft:chest",
            kind,
            facing,
        };
        // A left half facing north connects east, a right half west.
        assert_eq!(half("left", "north").connected_step(), Some((1, 0)));
        assert_eq!(half("right", "north").connected_step(), Some((-1, 0)));
        assert_eq!(half("left", "south").connected_step(), Some((-1, 0)));
        assert_eq!(half("right", "east").connected_step(), Some((0, -1)));
        assert_eq!(half("single", "east").connected_step(), None);
    }
    #[test]
    fn chest_halves_join_only_with_the_same_block() {
        let half = |block, kind| ChestHalf {
            block,
            kind,
            facing: "north",
        };
        for block in [
            "minecraft:chest",
            "minecraft:trapped_chest",
            "minecraft:copper_chest",
            "minecraft:waxed_weathered_copper_chest",
        ] {
            assert!(half(block, "left").pairs_with(&half(block, "right")));
        }
        // ChestBlock.combine needs the same block on both halves.
        let copper = half("minecraft:copper_chest", "left");
        assert!(!copper.pairs_with(&half("minecraft:exposed_copper_chest", "right")));
        // A single half never joins.
        let single = half("minecraft:chest", "single");
        assert!(!single.pairs_with(&half("minecraft:chest", "left")));
    }
}

#[cfg(test)]
mod reach_tests {
    use super::super::game_tests::tests::server_game;
    use horizon_client::{containers::BlockEntity, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;

    /// `Container.stillValidBlockEntity` for a double chest (`CompoundContainer`
    /// asks both halves): breaking the half the player did not click ends the
    /// six-row menu instead of leaving a three-row one.
    #[test]
    #[ignore = "daily"]
    fn breaking_the_other_half_closes_the_open_double_chest() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.player = Some(
            PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5)).expect("a player position"),
        );
        let registry = game.pack.assets.registry;
        let half = |kind: &str| {
            let mut state = registry
                .state(registry.default_state("minecraft:chest").unwrap())
                .unwrap()
                .clone();
            state.properties.insert("type".into(), kind.into());
            state.properties.insert("facing".into(), "north".into());
            registry.id(&state).unwrap()
        };
        let left = (8, 64, 8);
        let right = (9, 64, 8);
        game.world
            .set(left.0, left.1, left.2, half("left"))
            .unwrap();
        game.world
            .set(right.0, right.1, right.2, half("right"))
            .unwrap();
        for key in [left, right] {
            game.world
                .block_entities
                .insert(key, BlockEntity::Chest(vec![None; 27]));
        }
        game.open_block_effect_menu([8, 64, 8]);
        assert_eq!(game.container, Some(left), "the double chest opened");

        game.destroy_block_dropping([9, 64, 8])
            .expect("the other half breaks");
        game.tick_container_reach();
        assert_eq!(
            game.container, None,
            "the open double chest kept a menu after its other half broke"
        );
    }
}

#[cfg(test)]
mod lock_tests {
    use super::super::game_tests::tests::server_game;
    use horizon_client::{containers::BlockEntity, items::ItemStack};
    use horizon_core::nbt::Tag;

    /// `BaseContainerBlockEntity.canOpen` (`LockCode`): a `lock` whose item
    /// predicate the opener's main hand does not match refuses the menu at
    /// `createMenu`, so the block is never set as the open container. A hand
    /// holding the lock's item opens it.
    #[test]
    #[ignore = "daily"]
    fn a_locked_container_refuses_the_local_players_open() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.options.dedicated = None;
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        let key = (8, 64, 8);
        game.world.set(key.0, key.1, key.2, state).unwrap();
        game.world
            .block_entities
            .insert(key, BlockEntity::Chest(vec![None; 27]));
        let lock = Tag::compound([(
            "lock",
            Tag::compound([("items", Tag::String("minecraft:gold_ingot".into()))]),
        )]);
        game.world.set_block_lock(key, &lock);

        // An empty hand does not match `lock:{items:"minecraft:gold_ingot"}`.
        game.open_block_effect_menu([8, 64, 8]);
        assert_eq!(game.container, None, "a locked chest opened");
        assert!(
            game.titles.overlay.is_some(),
            "no `container.isLocked` action bar"
        );

        // `LockCode.canUnlock` reads the main hand: the key in the off hand
        // does not open it.
        game.inventory[0] = None;
        game.offhand = Some(ItemStack::new("minecraft:gold_ingot", 1));
        game.titles.overlay = None;
        game.open_block_effect_menu([8, 64, 8]);
        assert_eq!(
            game.container, None,
            "an off-hand key opened a locked chest"
        );

        // The lock's own item in the main hand opens it.
        game.inventory[0] = Some(ItemStack::new("minecraft:gold_ingot", 1));
        game.offhand = None;
        game.selected = 0;
        game.titles.overlay = None;
        game.open_block_effect_menu([8, 64, 8]);
        assert_eq!(game.container, Some(key), "the lock's item did not open it");
        assert!(game.titles.overlay.is_none());
    }

    /// `ChestBlock.MENU_PROVIDER_COMBINER.createMenu`: a double chest asks
    /// `canOpen` of both halves, so a lock on the half the player did not
    /// click still refuses the six-row menu.
    #[test]
    #[ignore = "daily"]
    fn a_lock_on_either_half_refuses_a_double_chest() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.options.dedicated = None;
        let registry = game.pack.assets.registry;
        let half = |kind: &str| {
            let mut state = registry
                .state(registry.default_state("minecraft:chest").unwrap())
                .unwrap()
                .clone();
            state.properties.insert("type".into(), kind.into());
            state.properties.insert("facing".into(), "north".into());
            registry.id(&state).unwrap()
        };
        let left_key = (8, 64, 8);
        let right_key = (9, 64, 8);
        game.world
            .set(left_key.0, left_key.1, left_key.2, half("left"))
            .unwrap();
        game.world
            .set(right_key.0, right_key.1, right_key.2, half("right"))
            .unwrap();
        for key in [left_key, right_key] {
            game.world
                .block_entities
                .insert(key, BlockEntity::Chest(vec![None; 27]));
        }
        // The right half holds the lock; the player clicks the left one.
        let lock = Tag::compound([(
            "lock",
            Tag::compound([("items", Tag::String("minecraft:gold_ingot".into()))]),
        )]);
        game.world.set_block_lock(right_key, &lock);

        game.open_block_effect_menu([8, 64, 8]);
        assert_eq!(game.container, None, "a half-locked double chest opened");
        assert!(game.titles.overlay.is_some());
    }
}
