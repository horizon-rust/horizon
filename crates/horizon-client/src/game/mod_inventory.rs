//! Physical slots lent to synchronous server program callbacks.
use super::{Game, ItemStack, lan::LanHost};
use horizon_mods::{InventoryHand, InventoryStack, InventoryView};
use horizon_protocol::crypto::uuid_string;
use std::sync::Arc;

struct Slots {
    player: String,
    conn: Option<u32>,
    actor: bool,
    inventory: Vec<Option<ItemStack>>,
    offhand: Option<ItemStack>,
    selected: usize,
    changed: bool,
}
struct NativeInventory {
    resources: Arc<super::Resources>,
    slots: Vec<Slots>,
}
impl InventoryView for NativeInventory {
    fn custom_data_integer(
        &self,
        player: &str,
        hand: InventoryHand,
        field: &str,
    ) -> Result<Option<i64>, String> {
        if !matches!(field, "tm" | "Gen9Offset") {
            return Err("custom data integer field is outside the machine allowlist".into());
        }
        let row = self.slots.iter().find(|s| s.player == player)
            .ok_or("player inventory is unavailable in this callback context")?;
        let stack = match hand {
            InventoryHand::Main => row.inventory.get(row.selected).and_then(Option::as_ref),
            InventoryHand::Off => row.offhand.as_ref(),
        };
        let Some(stack) = stack.filter(|stack| stack.count > 0) else {
            return Ok(None);
        };
        if self.resources.items().get(&stack.item).is_none() {
            return Err("held item ID is not registered in this server".into());
        }
        self.resources.items().custom_data_integer(stack, field)
    }
    fn component_string(
        &self,
        player: &str,
        hand: InventoryHand,
        key: &str,
    ) -> Result<Option<String>, String> {
        let row = self.slots.iter().find(|s| s.player == player)
            .ok_or("player inventory is unavailable in this callback context")?;
        let stack = match hand {
            InventoryHand::Main => row.inventory.get(row.selected).and_then(Option::as_ref),
            InventoryHand::Off => row.offhand.as_ref(),
        };
        let Some(stack) = stack.filter(|stack| stack.count > 0) else {
            return Ok(None);
        };
        if self.resources.items().get(&stack.item).is_none() {
            return Err("held item ID is not registered in this server".into());
        }
        self.resources.items().component_string(stack, key)
    }
    fn held(&self, player: &str, hand: InventoryHand) -> Result<Option<InventoryStack>, String> {
        let row = self
            .slots
            .iter()
            .find(|s| s.player == player)
            .ok_or("player inventory is unavailable in this callback context")?;
        let stack = match hand {
            InventoryHand::Main => row.inventory.get(row.selected).and_then(Option::as_ref),
            InventoryHand::Off => row.offhand.as_ref(),
        };
        let Some(stack) = stack.filter(|s| s.count > 0) else {
            return Ok(None);
        };
        if self.resources.items().get(&stack.item).is_none() {
            return Err("held item ID is not registered in this server".into());
        }
        Ok(Some(InventoryStack {
            item_id: stack.item.to_string(),
            count: stack.count,
        }))
    }
    fn give(&mut self, player: &str, item: &str, count: u32) -> Result<u32, String> {
        let definition = self
            .resources
            .items()
            .get(item)
            .filter(|_| item != "minecraft:air")
            .ok_or("inventory item ID is not a registered non-air item")?;
        let max = definition.max_stack;
        if max == 0 {
            return Err("inventory item has no supported stack capacity".into());
        }
        let row = self
            .slots
            .iter_mut()
            .find(|s| s.player == player)
            .ok_or("player inventory is unavailable in this callback context")?;
        let rest = super::give_inventory(
            &mut row.inventory,
            row.selected,
            max,
            ItemStack::new(item, count),
        );
        let inserted = count - rest.map_or(0, |s| s.count);
        row.changed |= inserted > 0;
        Ok(inserted)
    }
}
impl Game<'_> {
    pub(super) fn lend_mod_inventory(&mut self, host: Option<&mut LanHost>) {
        if self.mods.mods.inventory_lent()
            || !self.mods.mods.wants_inventory()
            || self.remote.is_some()
            || self.world_root.as_os_str().is_empty()
        {
            return;
        }
        let mut slots = Vec::new();
        let actor = self.player.is_some()
            && (self.lan_acting_uuid.is_some() || self.options.execution_role.is_client());
        if actor && self.inventory.len() >= 36 && self.selected < 9 {
            slots.push(Slots {
                player: self
                    .lan_acting_uuid
                    .map(uuid_string)
                    .unwrap_or_else(|| self.player_uuid_string()),
                conn: self.lan_acting,
                actor: true,
                inventory: std::mem::take(&mut self.inventory),
                offhand: self.offhand.clone(),
                selected: self.selected,
                changed: false,
            });
        }
        if let Some(host) = host.or(self.lan.as_mut()) {
            for g in host.hub.guests.iter_mut().filter(|g| g.placed) {
                if self.lan_acting == Some(g.conn) || g.inventory.len() < 36 || g.selected >= 9 {
                    continue;
                }
                slots.push(Slots {
                    player: uuid_string(g.uuid),
                    conn: Some(g.conn),
                    actor: false,
                    inventory: std::mem::take(&mut g.inventory),
                    offhand: g.offhand.clone(),
                    selected: g.selected,
                    changed: false,
                });
            }
        }
        self.mods.mods.lend_inventory(Box::new(NativeInventory {
            resources: self.res.clone(),
            slots,
        }));
    }
    pub(super) fn return_mod_inventory(&mut self, host: Option<&mut LanHost>) {
        let Some(view) = self.mods.mods.take_inventory() else {
            return;
        };
        let view: Box<dyn std::any::Any> = view;
        let Ok(view) = view.downcast::<NativeInventory>() else {
            return;
        };
        let mut guests = Vec::new();
        for row in view.slots {
            if row.actor {
                self.inventory = row.inventory;
            } else {
                guests.push(row);
            }
        }
        if let Some(host) = host.or(self.lan.as_mut()) {
            for row in guests {
                if let Some(i) = host
                    .hub
                    .guests
                    .iter()
                    .position(|g| Some(g.conn) == row.conn && uuid_string(g.uuid) == row.player)
                {
                    host.hub.guests[i].inventory = row.inventory;
                    if row.changed {
                        host.hub.send_inventory(i);
                    }
                }
            }
        }
    }
}
