//! `/clear`: `Inventory.clearOrCountMatchingItems` over a player's slots
//! (`ClearInventoryCommands.clearInventory`), and the menu broadcasts the
//! command ends with.
use super::{Game, Target};
use crate::game::lan::LanHost;
use horizon_client::{
    command::args::ItemInput,
    items::ItemStack,
    menu::{Kind, Menu},
};
use horizon_protocol::packets::play::{self, Clientbound as C};

impl Game<'_> {
    /// `ClearInventoryCommands.clearInventory` for one player: the matching
    /// items go (or are only counted), then the player's menu hears what
    /// changed (`containerMenu.broadcastChanges`, `inventoryMenu.slotsChanged`).
    pub(in crate::game) fn clear_player(
        &mut self,
        t: Target,
        predicate: &ItemInput,
        max: i32,
        counting: bool,
    ) -> i32 {
        // The inventory menu's 2x2 grid is a guest's own (`Guest::craft_grid`):
        // the body's grid is empty while a guest's turn is swapped in.
        let conn = match t {
            Target::Player => self.lan_acting,
            Target::Guest(conn) => Some(conn),
            _ => None,
        };
        let gi = conn.and_then(|c| {
            let host = self.lan.as_ref()?;
            host.hub.guests.iter().position(|g| g.conn == c)
        });
        let mut grid = gi.map(|i| self.lan.as_ref().unwrap().hub.guests[i].craft_grid.clone());
        let cleared = self.for_player(t, |body| {
            // The menu's grid, in the body's while it clears.
            if let Some(grid) = grid.as_mut() {
                for k in 0..4 {
                    std::mem::swap(&mut body.craft_grid[k], &mut grid[k]);
                }
            }
            let was = body.carried.clone();
            let n = body.clear_matching(predicate, max);
            let now = body.carried.clone();
            if let Some(grid) = grid.as_mut() {
                for k in 0..4 {
                    std::mem::swap(&mut body.craft_grid[k], &mut grid[k]);
                }
            }
            (n, was, now)
        });
        let Some((n, was, now)) = cleared else {
            return 0;
        };
        let (Some(gi), Some(mut host)) = (gi, self.lan.take()) else {
            return n;
        };
        if let Some(grid) = grid {
            host.hub.guests[gi].craft_grid = grid;
        }
        if !counting {
            self.clear_broadcast(&mut host, gi, was, now);
        }
        self.lan = Some(host);
        n
    }
    /// `AbstractContainerMenu.broadcastChanges` for a guest's inventory menu:
    /// the cursor it no longer holds, then the result slot
    /// (`CraftingMenu.slotChangedCraftingGrid`, which the next broadcast sends
    /// as slot 0) of the grid the clear left.
    fn clear_broadcast(
        &mut self,
        host: &mut LanHost,
        gi: usize,
        was: Option<ItemStack>,
        now: Option<ItemStack>,
    ) {
        let grid = host.hub.guests[gi].craft_grid.clone();
        let mut menu = Menu::new(Kind::Inventory);
        self.guest_menu_op(host, gi, &mut menu, |menu, env| {
            for (k, s) in grid.into_iter().enumerate() {
                menu.set_slot(env, 1 + k, s);
            }
        });
        host.hub.guests[gi].craft_result = menu.slot(0).cloned();
        // `synchronizeCarriedToRemote`: only what the player's copy differs on.
        if now != was {
            let net = host.hub.stack(&now);
            host.hub.guests[gi].send(&C::SetCursorItem(play::SetCursorItem { item: net }));
        }
    }
    /// Inventory.clearOrCountMatchingItems for the player in place: `max`
    /// of the matching items go (all when negative, none and only counted
    /// when 0). Returns how many matched.
    fn clear_matching(&mut self, predicate: &ItemInput, max: i32) -> i32 {
        let counting = max == 0;
        // Inventory slots 0..40: main, armor feet to head, off hand; then
        // clearOrCountMatchingItems visits `inventoryMenu.getCraftSlots()` (the
        // 2x2, never an open crafting table's 3x3) and the carried stack.
        let mut removed = 0i32;
        let mut budget = if max < 0 { i32::MAX } else { max };
        let all = |g: &Self| -> Vec<Option<ItemStack>> {
            let mut v: Vec<Option<ItemStack>> = g.inventory.clone();
            v.extend(g.armor.iter().rev().cloned());
            v.push(g.offhand.clone());
            v.extend(g.craft_grid[..4].iter().cloned());
            v.push(g.carried.clone());
            v
        };
        let matching: Vec<bool> = all(self)
            .iter()
            .map(|s| s.as_ref().is_some_and(|s| self.stack_matches(predicate, s)))
            .collect();
        let mut slots: Vec<&mut Option<ItemStack>> = Vec::new();
        slots.extend(self.inventory.iter_mut());
        slots.extend(self.armor.iter_mut().rev());
        slots.push(&mut self.offhand);
        slots.extend(self.craft_grid[..4].iter_mut());
        slots.push(&mut self.carried);
        for (slot, matches) in slots.into_iter().zip(matching) {
            if !matches {
                continue;
            }
            let Some(stack) = slot.as_mut() else { continue };
            let take = if counting {
                stack.count as i32
            } else {
                (stack.count as i32).min(budget)
            };
            removed += take;
            if !counting {
                budget -= take;
                stack.count -= take as u32;
                if stack.count == 0 {
                    *slot = None;
                }
                if budget == 0 {
                    break;
                }
            }
        }
        removed
    }
}
