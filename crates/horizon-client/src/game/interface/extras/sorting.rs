//! Sorting (`sort`, `sortKey`; arismoko/horizon#332, `docs/extras.md`), off by default: a small button by a
//! menu's storage slots and one by the inventory's (an optional key sorts the part under the
//! cursor). The clicks come from `inv_sort::plan` and go through the Mouse Tweaks' ordinary slot
//! clicks (`mouse_tweaks.rs`), so they work wherever vanilla's do.
use crate::game::{Game, Screen};
use horizon_client::{
    containers::BlockEntity,
    hud::{HudBatch, rgba},
    inv_sort,
    items::ItemStack,
    keybinds,
    menu::Input,
    stations::hover_name,
};
use winit::{event::MouseButton, keyboard::KeyCode};

/// A sort button's side in GUI pixels.
const SORT_BUTTON: f32 = 9.0;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(in crate::game) enum ExtraButtonKind {
    Sort(Vec<usize>),
    DepositMatching,
    DepositAll,
    RestockAll,
}

impl Game<'_> {
    /// The extra buttons (sort, quick deposit, deposit all, restock all) of the open screen.
    pub(in crate::game) fn extra_buttons(&self) -> Vec<((f32, f32), ExtraButtonKind)> {
        if !self.settings.extras.sort
            || self.screen != Screen::Inventory
            || self.station.is_some()
            || self.mount_active()
        {
            return Vec::new();
        }
        let chest_like = if self.remote.is_some() {
            if !self.remote_storage_open() {
                return Vec::new();
            }
            true
        } else {
            match self.container_kind() {
                None => false,
                Some(BlockEntity::Chest(_)) => {
                    self.chest_rows().is_some() || self.container_block_is_shulker()
                }
                Some(_) => return Vec::new(),
            }
        };
        let mut buttons = Vec::new();
        let container: Vec<usize> = self
            .slots()
            .into_iter()
            .filter(|s| (300..500).contains(s))
            .collect();
        if chest_like && !container.is_empty() {
            buttons.push(((160.0, 5.0), ExtraButtonKind::Sort(container.clone())));
            buttons.push(((149.0, 5.0), ExtraButtonKind::DepositMatching));
            buttons.push(((138.0, 5.0), ExtraButtonKind::DepositAll));
            buttons.push(((127.0, 5.0), ExtraButtonKind::RestockAll));
        }
        let inv_y = self.screen_height() - 95.0;
        buttons.push(((160.0, inv_y), ExtraButtonKind::Sort((9..36).collect())));
        if chest_like && !container.is_empty() {
            buttons.push(((149.0, inv_y), ExtraButtonKind::DepositMatching));
        }
        buttons
    }

    /// The sort buttons of the open screen: their corner and the slots each sorts.
    pub(in crate::game) fn sort_buttons(&self) -> Vec<((f32, f32), Vec<usize>)> {
        self.extra_buttons()
            .into_iter()
            .filter_map(|(pos, kind)| match kind {
                ExtraButtonKind::Sort(slots) => Some((pos, slots)),
                _ => None,
            })
            .collect()
    }

    /// The extra buttons: bordered square with distinct icons, lighter under the cursor.
    pub(in crate::game) fn draw_sort_buttons(&self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let atlas = &self.gui.atlas;
        for ((x, y), kind) in self.extra_buttons() {
            let (x, y) = (ox + x, oy + y);
            let (cx, cy) = self.cursor;
            let hovered = cx >= x && cx < x + SORT_BUTTON && cy >= y && cy < y + SORT_BUTTON;
            let face = if hovered {
                rgba(220, 220, 220, 255)
            } else {
                rgba(160, 160, 160, 255)
            };
            batch.fill(atlas, x, y, SORT_BUTTON, SORT_BUTTON, rgba(55, 55, 55, 255));
            batch.fill(
                atlas,
                x + 1.0,
                y + 1.0,
                SORT_BUTTON - 2.0,
                SORT_BUTTON - 2.0,
                face,
            );
            let dark = rgba(55, 55, 55, 255);
            match kind {
                ExtraButtonKind::Sort(_) => {
                    for (i, w) in [5.0, 4.0, 3.0].into_iter().enumerate() {
                        let by = y + 2.0 + 2.0 * i as f32;
                        batch.fill(atlas, x + 2.0, by, w, 1.0, dark);
                    }
                }
                ExtraButtonKind::DepositMatching => {
                    batch.fill(atlas, x + 4.0, y + 2.0, 1.0, 5.0, dark);
                    batch.fill(atlas, x + 2.0, y + 4.0, 5.0, 1.0, dark);
                }
                ExtraButtonKind::DepositAll => {
                    batch.fill(atlas, x + 4.0, y + 2.0, 1.0, 4.0, dark);
                    batch.fill(atlas, x + 3.0, y + 5.0, 3.0, 1.0, dark);
                    batch.fill(atlas, x + 4.0, y + 6.0, 1.0, 1.0, dark);
                }
                ExtraButtonKind::RestockAll => {
                    batch.fill(atlas, x + 4.0, y + 2.0, 1.0, 1.0, dark);
                    batch.fill(atlas, x + 3.0, y + 3.0, 3.0, 1.0, dark);
                    batch.fill(atlas, x + 4.0, y + 4.0, 1.0, 3.0, dark);
                }
            }
        }
    }

    /// A left press on an inventory extra button (sort, deposit, restock).
    pub(in crate::game) fn sort_press(&mut self) -> bool {
        let Some(r) = self.renderer.as_ref() else {
            return false;
        };
        let batch = HudBatch::new(
            [r.config.width, r.config.height],
            self.settings.gui_scale_inputs(),
        );
        let (ox, oy) = self.inventory_origin(&batch);
        let (cx, cy) = self.cursor;
        let hit = self.extra_buttons().into_iter().find(|((x, y), _)| {
            let (x, y) = (ox + x, oy + y);
            cx >= x && cx < x + SORT_BUTTON && cy >= y && cy < y + SORT_BUTTON
        });
        let Some((_, kind)) = hit else {
            return false;
        };
        match kind {
            ExtraButtonKind::Sort(slots) => self.sort_slots(&slots),
            ExtraButtonKind::DepositMatching => {
                self.easy_deposit_matching();
            }
            ExtraButtonKind::DepositAll => {
                self.easy_deposit_all();
            }
            ExtraButtonKind::RestockAll => {
                self.easy_restock_all();
            }
        }
        true
    }

    /// Deposits all matching items from player inventory (9..36 and 0..9) into the container (300..500).
    pub(in crate::game) fn easy_deposit_matching(&mut self) -> bool {
        let container_slots: Vec<usize> = self
            .slots()
            .into_iter()
            .filter(|s| (300..500).contains(s))
            .collect();
        if container_slots.is_empty() {
            return false;
        }
        let container_items: Vec<ItemStack> = container_slots
            .iter()
            .filter_map(|&s| self.slot_view(s))
            .collect();
        if container_items.is_empty() {
            return false;
        }

        let mut inventory = Vec::new();
        for s in (9..36).chain(0..9) {
            if let Some(held) = self.slot_view(s) {
                inventory.push((s, held));
            }
        }
        let matches = plan_deposit_matching(&container_items, &inventory);
        let ops: Vec<_> = matches
            .into_iter()
            .map(|s| (Some(s), 0, Input::QuickMove))
            .collect();
        self.tweak_ops(&ops)
    }

    /// Deposits all non-hotbar items (9..36) from the player inventory into the container.
    pub(in crate::game) fn easy_deposit_all(&mut self) -> bool {
        let container_slots: Vec<usize> = self
            .slots()
            .into_iter()
            .filter(|s| (300..500).contains(s))
            .collect();
        if container_slots.is_empty() {
            return false;
        }
        let non_empty: Vec<usize> = (9..36).filter(|&s| self.slot_view(s).is_some()).collect();
        let ops: Vec<_> = plan_deposit_all(&non_empty)
            .into_iter()
            .map(|s| (Some(s), 0, Input::QuickMove))
            .collect();
        self.tweak_ops(&ops)
    }

    /// Takes all items from the container (300..500) into player inventory.
    pub(in crate::game) fn easy_restock_all(&mut self) -> bool {
        let container_slots: Vec<usize> = self
            .slots()
            .into_iter()
            .filter(|s| (300..500).contains(s))
            .collect();
        if container_slots.is_empty() {
            return false;
        }
        let non_empty: Vec<usize> = container_slots
            .into_iter()
            .filter(|&s| self.slot_view(s).is_some())
            .collect();
        let ops: Vec<_> = plan_restock_all(&non_empty)
            .into_iter()
            .map(|s| (Some(s), 0, Input::QuickMove))
            .collect();
        self.tweak_ops(&ops)
    }

    /// Quick-moves all stacks of the item in `slot` across between inventory and container.
    pub(in crate::game) fn quick_move_stack_all(&mut self, slot: usize) -> bool {
        let Some(item) = self.slot_view(slot) else {
            return false;
        };
        if !self.tweak_click(slot, 0, Input::QuickMove) {
            return false;
        }
        if self.remote.is_some() && self.screen != Screen::Creative {
            self.remote_quick_move_all(slot, &item);
        } else {
            self.menu_session(|menu, env, to_menu| {
                Some(menu.quick_move_all(env, to_menu(slot)?, &item).is_ok())
            });
            if self.remote.is_some() {
                self.send_creative_slots();
            }
        }
        if let Some(key) = self.container {
            self.container_changed(key);
        }
        true
    }

    /// The sort key: sorts the part under the cursor (the container when
    /// the cursor is over no slot). KeyX deposits matching items into container.
    pub(in crate::game) fn sort_key_press(&mut self, key: KeyCode) -> bool {
        if key == KeyCode::KeyX
            && !self.recipe_search_focused()
            && (self.settings.extras.sort || self.tweaks_on())
            && self.easy_deposit_matching()
        {
            return true;
        }
        keybinds::key_name(key).is_some_and(|name| self.sort_input(name))
    }

    pub(in crate::game) fn sort_mouse_press(&mut self, button: MouseButton) -> bool {
        self.sort_input(&keybinds::mouse_name(button))
    }

    fn sort_input(&mut self, name: &str) -> bool {
        let e = &self.settings.extras;
        if !e.sort || e.sort_key != name || self.recipe_search_focused() {
            return false;
        }
        let buttons = self.sort_buttons();
        let hovered = self.inventory_target().0;
        let slots = buttons
            .iter()
            .find(|(_, slots)| hovered.is_some_and(|h| slots.contains(&h)))
            .or(buttons.first())
            .map(|(_, slots)| slots.clone());
        let Some(slots) = slots else {
            return false;
        };
        self.sort_slots(&slots);
        true
    }

    /// Sorts `slots` with ordinary left clicks (needs an empty hand).
    fn sort_slots(&mut self, slots: &[usize]) {
        if self.carried.is_some() {
            return;
        }
        let views: Vec<Option<ItemStack>> = slots.iter().map(|&s| self.slot_view(s)).collect();
        let clicks = {
            let rank = |s: &ItemStack| {
                let order = self.creative_screen.order_of(s).unwrap_or(usize::MAX);
                (order, hover_name(&self.pack.names, s))
            };
            let max = |s: &ItemStack| self.max_stack(s);
            inv_sort::plan(&views, &rank, &max)
        };
        for i in clicks {
            self.tweak_click(slots[i], 0, Input::Pickup);
        }
    }
}

/// Finds which inventory slots match any item currently in the container.
pub(in crate::game) fn plan_deposit_matching(
    container_items: &[ItemStack],
    inventory: &[(usize, ItemStack)],
) -> Vec<usize> {
    if container_items.is_empty() {
        return Vec::new();
    }
    inventory
        .iter()
        .filter(|(_, held)| {
            container_items
                .iter()
                .any(|c| c.same_item_same_components(held))
        })
        .map(|&(slot, _)| slot)
        .collect()
}

/// Selects slots to deposit for 'Deposit All' (main inventory non-hotbar slots 9..36).
pub(in crate::game) fn plan_deposit_all(slots: &[usize]) -> Vec<usize> {
    slots
        .iter()
        .copied()
        .filter(|&s| (9..36).contains(&s))
        .collect()
}

/// Selects slots to restock for 'Restock All' (container slots 300..500).
pub(in crate::game) fn plan_restock_all(slots: &[usize]) -> Vec<usize> {
    slots
        .iter()
        .copied()
        .filter(|&s| (300..500).contains(&s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_button_kinds_and_slots() {
        let sort_slots = vec![9, 10, 11];
        let kind = ExtraButtonKind::Sort(sort_slots.clone());
        if let ExtraButtonKind::Sort(slots) = kind {
            assert_eq!(slots, sort_slots);
        } else {
            panic!("expected ExtraButtonKind::Sort");
        }

        let non_empty = vec![0, 5, 8, 9, 15, 35, 300, 305, 499];

        // Deposit all should only take main inventory non-hotbar slots (9..36)
        let deposit_slots = plan_deposit_all(&non_empty);
        assert_eq!(deposit_slots, vec![9, 15, 35]);

        // Restock all should only take container slots (300..500)
        let restock_slots = plan_restock_all(&non_empty);
        assert_eq!(restock_slots, vec![300, 305, 499]);
    }

    #[test]
    fn easy_deposit_matching_plan() {
        let container = vec![
            ItemStack::new("minecraft:cobblestone", 32),
            ItemStack::new("minecraft:iron_ingot", 10),
        ];

        let inv = vec![
            (9, ItemStack::new("minecraft:cobblestone", 64)), // main inv matching
            (10, ItemStack::new("minecraft:oak_planks", 16)), // main inv not matching
            (0, ItemStack::new("minecraft:iron_ingot", 5)),   // hotbar matching
            (1, ItemStack::new("minecraft:dirt", 64)),        // hotbar not matching
        ];

        let planned = plan_deposit_matching(&container, &inv);
        assert_eq!(planned, vec![9, 0]);

        // Empty container yields no operations
        assert!(plan_deposit_matching(&[], &inv).is_empty());

        // Empty inventory yields no operations
        assert!(plan_deposit_matching(&container, &[]).is_empty());
    }

    #[test]
    fn easy_deposit_and_restock_slot_filtering() {
        let non_empty = vec![0, 5, 8, 9, 15, 35, 300, 305, 499];

        // Deposit all should only take main inventory non-hotbar slots (9..36)
        let deposit_slots = plan_deposit_all(&non_empty);
        assert_eq!(deposit_slots, vec![9, 15, 35]);

        // Restock all should only take container slots (300..500)
        let restock_slots = plan_restock_all(&non_empty);
        assert_eq!(restock_slots, vec![300, 305, 499]);
    }
}
