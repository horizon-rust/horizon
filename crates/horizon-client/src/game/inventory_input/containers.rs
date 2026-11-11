//! The engine's side of the container screen API (`horizon_hooks::containers`, docs/layers.md,
//! "Container screens"): the open menu screen as a [`Container`] for the length of one hook call,
//! and the input and drawing that reach the hooks. With no hook registered nothing here is built.
use crate::game::{Game, NativeSlotRole, Screen};
use horizon_client::{
    extras_hud, hud::HudBatch, items::ItemStack, keybinds, menu::Input, stations::hover_name,
};
use horizon_hooks::{
    containers::{Click, Container, Role, Slot, SlotId, Stack},
    engine,
};
use winit::{event::MouseButton, keyboard::KeyCode};

/// The open screen's menu, over the game for one call. `origin`: the panel's top left corner in
/// GUI pixels.
struct Open<'g, 'a> {
    game: &'g mut Game<'a>,
    origin: (f32, f32),
}

/// The same, read only: what drawing gets.
struct View<'g, 'a> {
    game: &'g Game<'a>,
    origin: (f32, f32),
}

fn role(role: NativeSlotRole) -> Role {
    match role {
        NativeSlotRole::Hotbar => Role::Hotbar,
        NativeSlotRole::Inventory => Role::Inventory,
        NativeSlotRole::Storage => Role::Storage,
        NativeSlotRole::CraftingGrid | NativeSlotRole::Input => Role::Input,
        NativeSlotRole::Result => Role::Result,
        NativeSlotRole::Equipment => Role::Equipment,
    }
}

/// Stacks that merge (`same_item_same_components`) share a kind.
fn kind(stack: &ItemStack) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    stack.item.hash(&mut h);
    stack.damage.hash(&mut h);
    format!("{:?}", stack.extra).hash(&mut h);
    h.finish()
}

fn slot_id(slot: usize) -> SlotId {
    SlotId(u32::try_from(slot).unwrap_or(u32::MAX))
}

fn game_slot(slot: SlotId) -> usize {
    slot.0 as usize
}

impl<'a> Game<'a> {
    /// The open menu's type: the server's, else the menu this window opened itself.
    fn container_menu(&self) -> &str {
        if let Some(menu) = self.remote_menu_type() {
            return menu;
        }
        use horizon_client::containers::BlockEntity;
        if self.mount_active() {
            return "horizon:mount";
        }
        if self.station.is_some() {
            return "horizon:station";
        }
        match self.container_kind() {
            None if self.craft_size == 3 => "minecraft:crafting",
            None => "inventory",
            Some(BlockEntity::Chest(_)) => "minecraft:generic_9x3",
            Some(BlockEntity::Furnace(_)) => "minecraft:furnace",
            Some(_) => "horizon:block",
        }
    }
    fn container_slots(&self) -> Vec<Slot> {
        self.native_slot_roles()
            .into_iter()
            .map(|(s, r)| {
                let (x, y) = self.slot_position(s);
                Slot {
                    id: slot_id(s),
                    role: role(r),
                    x,
                    y,
                }
            })
            .collect()
    }
    fn container_stack(&self, stack: &ItemStack) -> Stack {
        Stack {
            item: stack.item.to_string(),
            count: stack.count,
            max: self.max_stack(stack),
            kind: kind(stack),
        }
    }
    /// The stack of `kind` the screen shows, on the cursor or in a slot.
    fn container_find(&mut self, of: &Stack) -> Option<ItemStack> {
        if let Some(c) = self.carried.as_ref().filter(|c| kind(c) == of.kind) {
            return Some(c.clone());
        }
        self.slots()
            .into_iter()
            .filter_map(|s| self.slot_view(s))
            .find(|s| kind(s) == of.kind)
    }
    /// The panel's top left corner, None without a window.
    fn container_origin(&self) -> Option<(f32, f32)> {
        let r = self.renderer.as_ref()?;
        Some(self.inventory_origin(&HudBatch::new([r.config.width, r.config.height])))
    }
    /// The open screen for the hooks' input, None when no hook may take it.
    fn container_open(&mut self) -> Option<Open<'_, 'a>> {
        if self.screen != Screen::Inventory || !engine::containers() {
            return None;
        }
        let origin = self.container_origin()?;
        Some(Open { game: self, origin })
    }
    fn hovered(&self) -> Option<SlotId> {
        self.inventory_target().0.map(slot_id)
    }
    /// A press over the panel: true when a hook took it, and vanilla's drag doesn't start.
    pub(in crate::game) fn container_press(&mut self, right: bool) -> bool {
        let slot = self.hovered();
        let taken = self
            .container_open()
            .is_some_and(|mut c| engine::container_press(&mut c, slot, right));
        if taken {
            self.inv_drag = None;
            self.inv_clicks.skip_release();
        }
        taken
    }
    /// The cursor moved with a button held: true when a hook owns the drag, which ends vanilla's.
    pub(in crate::game) fn container_drag(&mut self) -> bool {
        let slot = self.hovered();
        let taken = self
            .container_open()
            .is_some_and(|mut c| engine::container_drag(&mut c, slot));
        if taken {
            self.inv_drag = None;
            self.inv_clicks.skip_release();
        }
        taken
    }
    pub(in crate::game) fn container_release(&mut self) {
        if let Some(mut c) = self.container_open() {
            engine::container_release(&mut c);
        }
    }
    /// The wheel over an inventory screen: true when the Mouse Tweaks or a hook used it.
    pub(in crate::game) fn container_scroll(&mut self, steps: f32) -> bool {
        let slot = self.inventory_target().0;
        if self.tweak_scroll(steps, slot) {
            return true;
        }
        let slot = slot.map(slot_id);
        self.container_open()
            .is_some_and(|mut c| engine::container_wheel(&mut c, slot, steps))
    }
    /// A key the screen got: sorting's key, else a hook's.
    pub(in crate::game) fn container_key_press(&mut self, key: KeyCode) -> bool {
        self.sort_key_press(key)
            || keybinds::key_name(key).is_some_and(|name| self.container_key(name))
    }
    /// A mouse button the screen got, as a key.
    pub(in crate::game) fn container_mouse_press(&mut self, button: MouseButton) -> bool {
        self.sort_mouse_press(button) || self.container_key(&keybinds::mouse_name(button))
    }
    fn container_key(&mut self, name: &str) -> bool {
        if self.recipe_search_focused() {
            return false;
        }
        let slot = self.hovered();
        self.container_open()
            .is_some_and(|mut c| engine::container_key(&mut c, slot, name))
    }
    /// Draws the sort buttons and the hooks' drawing over the panel at (`ox`, `oy`).
    pub(in crate::game) fn draw_container_extras(&self, batch: &mut HudBatch, ox: f32, oy: f32) {
        self.draw_sort_buttons(batch, ox, oy);
        if !engine::containers() {
            return;
        }
        let view = View {
            game: self,
            origin: (ox, oy),
        };
        extras_hud::draw_on_panel(batch, &self.gui.atlas, (ox, oy), |canvas| {
            engine::container_draw(&view, canvas);
        });
    }
}

/// What both readings answer alike.
macro_rules! reads {
    () => {
        fn menu(&self) -> &str {
            self.game.container_menu()
        }
        fn slots(&self) -> Vec<Slot> {
            self.game.container_slots()
        }
        fn carried(&self) -> Option<Stack> {
            let c = self.game.carried.as_ref()?;
            Some(self.game.container_stack(c))
        }
        fn spread(&self) -> usize {
            self.game.inv_drag.as_ref().map_or(0, |(_, s)| s.len())
        }
        fn cursor(&self) -> (f32, f32) {
            let (x, y) = self.game.cursor;
            (x - self.origin.0, y - self.origin.1)
        }
        fn shift(&self) -> bool {
            self.game.has_shift_down()
        }
    };
}

impl Container for Open<'_, '_> {
    reads!();
    fn stack(&mut self, slot: SlotId) -> Option<Stack> {
        let s = self.game.slot_view(game_slot(slot))?;
        Some(self.game.container_stack(&s))
    }
    fn takes_one(&mut self, slot: SlotId, of: &Stack) -> bool {
        self.game
            .container_find(of)
            .is_some_and(|s| self.game.tweak_takes_one(game_slot(slot), &s))
    }
    fn landing(&mut self, slot: SlotId) -> Vec<SlotId> {
        let landing = self.game.tweak_landing(game_slot(slot));
        landing.into_iter().map(slot_id).collect()
    }
    fn click(&mut self, slot: SlotId, click: Click) -> bool {
        let slot = game_slot(slot);
        match click {
            Click::Pick { right } => self.game.tweak_click(slot, i32::from(right), Input::Pickup),
            Click::QuickMove => self.game.tweak_click(slot, 0, Input::QuickMove),
            Click::Put { one } => self.game.tweak_put(slot, one),
            _ => false,
        }
    }
    fn order(&mut self, slot: SlotId) -> (u32, String) {
        let Some(s) = self.game.slot_view(game_slot(slot)) else {
            return (u32::MAX, String::new());
        };
        let order = self.game.creative_screen.order_of(&s);
        let order = order.map_or(u32::MAX, |o| u32::try_from(o).unwrap_or(u32::MAX));
        (order, hover_name(&self.game.pack.names, &s))
    }
}

/// Drawing gets `&dyn Container`, which can't call the `&mut self` reads.
impl Container for View<'_, '_> {
    reads!();
    fn stack(&mut self, _: SlotId) -> Option<Stack> {
        None
    }
    fn takes_one(&mut self, _: SlotId, _: &Stack) -> bool {
        false
    }
    fn landing(&mut self, _: SlotId) -> Vec<SlotId> {
        Vec::new()
    }
    fn click(&mut self, _: SlotId, _: Click) -> bool {
        false
    }
    fn order(&mut self, _: SlotId) -> (u32, String) {
        (u32::MAX, String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::items::ItemStack;

    fn stack(item: &str, count: u32, damage: u32) -> ItemStack {
        ItemStack {
            item: item.into(),
            count,
            damage,
            extra: None,
        }
    }

    #[test]
    fn stacks_that_merge_share_a_kind() {
        let a = kind(&stack("minecraft:stone", 1, 0));
        assert_eq!(
            a,
            kind(&stack("minecraft:stone", 64, 0)),
            "the count is no part"
        );
        assert_ne!(a, kind(&stack("minecraft:dirt", 1, 0)));
        let worn = stack("minecraft:iron_sword", 1, 3);
        assert_ne!(kind(&worn), kind(&stack("minecraft:iron_sword", 1, 0)));
    }
}
