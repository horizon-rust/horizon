//! A server player's inventory by vanilla's slot numbers (`Inventory.getItem`/`setItem`), for
//! mods that copy or edit it whole: player data sync, `/invsee`.
use super::{Game, commands::items::Slot, native_server::PlayerRead};
use horizon_client::items::ItemStack;
use horizon_items::equipment::{BODY, Equipment, SADDLE};
use mod_api::{PlayerDataRefusal, PlayerRef};

/// `Inventory`'s slot numbers past the 36 main slots (`EQUIPMENT_SLOT_MAPPING`).
const SLOTS: u32 = 43;

/// 0..35 main (0..8 hotbar), 36..39 feet to head, 40 offhand, 41 body, 42 saddle.
/// ItemPlace uses 100..103 for armour and -106 for offhand; InventoryMenu has no 41/42.
/// The slot the game keeps `Inventory` slot `slot` in.
fn inventory_slot(slot: u32) -> Option<Slot> {
    Some(match slot {
        0..36 => Slot::Inv(slot as usize),
        // `armor` is head first: 36 feet is its last.
        36..40 => Slot::Armor((39 - slot) as usize),
        40 => Slot::Offhand,
        41 => Slot::Equipment(BODY),
        42 => Slot::Equipment(SADDLE),
        _ => return None,
    })
}

/// The non-empty slots, by number, of an inventory laid out as the game keeps it.
fn numbered(
    inventory: &[Option<ItemStack>],
    armor: &[Option<ItemStack>; 4],
    offhand: &Option<ItemStack>,
    equipment: &Equipment,
) -> Vec<(u32, ItemStack)> {
    (0..SLOTS)
        .filter_map(|slot| {
            let stack = match inventory_slot(slot)? {
                Slot::Inv(i) => inventory.get(i).cloned().flatten(),
                Slot::Armor(i) => armor[i].clone(),
                Slot::Offhand => offhand.clone(),
                Slot::Equipment(i) => equipment.get(i).cloned(),
                _ => None,
            };
            Some((slot, stack.filter(|s| s.count > 0)?))
        })
        .collect()
}

impl Game<'_> {
    /// `Inventory.getItem` for every slot that holds something.
    pub(super) fn player_inventory_slots(
        &self,
        player: PlayerRef,
    ) -> Option<Vec<(u32, ItemStack)>> {
        if self.remote.is_some() {
            return None;
        }
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => {
                numbered(&self.inventory, &self.armor, &self.offhand, &self.equipment)
            }
            PlayerRead::Guest(g) => numbered(&g.inventory, &g.armor, &g.offhand, &g.equipment),
        })
    }

    /// Why a write to this player's inventory, ender chest, hands or saved data is refused
    /// before it starts: a client, a handle with no online player, a dead player.
    pub(super) fn player_write_gate(&self, player: PlayerRef) -> Result<(), PlayerDataRefusal> {
        if self.remote.is_some() {
            return Err(PlayerDataRefusal::Client);
        }
        match self.player_handle_read(player) {
            None => Err(PlayerDataRefusal::Stale),
            Some(PlayerRead::Guest(g)) if g.dead => Err(PlayerDataRefusal::Dead),
            Some(_) => Ok(()),
        }
    }

    /// `Inventory.setItem`, in the player's turn: the tick's inventory sync tells the client.
    pub(super) fn player_set_inventory_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<(), PlayerDataRefusal> {
        self.player_write_gate(player)?;
        let target = inventory_slot(slot).ok_or(PlayerDataRefusal::SlotOutOfRange {
            slot,
            last: SLOTS - 1,
        })?;
        let mut set = false;
        let ran = self.native_as_player_handle(player, |game| {
            set = game.slot_set(&target, stack.filter(|s| s.count > 0));
            if set && game.player.is_some() {
                game.after_commit(super::transactions::After::InventoryTrigger(player.uuid()));
            }
        });
        if ran && set {
            Ok(())
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }

    /// `PlayerRef::swap_hands`: the main and off hand stacks trade places in the player's turn,
    /// as `own_swap_hands` does without the hook. Refused, with nothing changed, for a client,
    /// a stale or dead handle and a spectator.
    pub(super) fn player_swap_hands(&mut self, player: PlayerRef) -> bool {
        if self.player_write_gate(player).is_err() {
            return false;
        }
        let mut swapped = false;
        let ran = self.native_as_player_handle(player, |game| {
            if game.spectator() {
                return;
            }
            let selected = game.selected;
            let (old_main, old_off) = (game.inventory[selected].clone(), game.offhand.clone());
            std::mem::swap(&mut game.offhand, &mut game.inventory[selected]);
            game.hand_stack_replaced(selected);
            game.hand_stack_replaced(crate::game::shield::OFFHAND_SLOT);
            let (new_main, new_off) = (game.inventory[selected].clone(), game.offhand.clone());
            game.player_on_equip("offhand", old_off.as_ref(), new_off.as_ref());
            game.player_on_equip("mainhand", old_main.as_ref(), new_main.as_ref());
            swapped = true;
        });
        ran && swapped
    }

    /// `Player#dropItem(boolean)` for a mod: see `drop_selected_stack`. Refused, with nothing
    /// thrown, for a client, a stale or dead handle and a spectator.
    pub(super) fn player_drop_selected(&mut self, player: PlayerRef, whole_stack: bool) -> bool {
        if self.player_write_gate(player).is_err() {
            return false;
        }
        let mut dropped = false;
        let ran = self.native_as_player_handle(player, |game| {
            dropped = game.drop_selected_stack(whole_stack);
        });
        ran && dropped
    }
}

#[cfg(test)]
mod swap_hands_call_tests {
    use super::super::native_guest_turn_tests::with_guests;
    use horizon_client::items::ItemStack;

    /// Sets guest 13's selected slot and off hand, as a parked guest keeps them.
    fn set_hands(game: &mut super::super::Game<'_>, main: Option<ItemStack>, off: Option<ItemStack>) {
        let host = game.lan.as_mut().unwrap();
        let index = host.hub.guests.iter().position(|g| g.conn == 13).unwrap();
        host.hub.guests.update(index, |g| {
            let selected = g.selected;
            g.inventory[selected] = main;
            g.offhand = off;
        });
    }

    /// The item ids in guest 13's selected slot and off hand.
    fn hands(game: &super::super::Game<'_>) -> (Option<String>, Option<String>) {
        let guest = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
        (
            guest.inventory[guest.selected].as_ref().map(|s| s.item.to_string()),
            guest.offhand.as_ref().map(|s| s.item.to_string()),
        )
    }

    /// A sword and a torch trade places, and a second call trades them back; an empty off hand
    /// takes the sword, and a spectator's call changes nothing.
    #[test]
    #[ignore = "daily"]
    fn a_guests_hands_trade_places_per_call() {
        with_guests(|game| {
            set_hands(
                game,
                Some(ItemStack::new("minecraft:diamond_sword", 1)),
                Some(ItemStack::new("minecraft:torch", 1)),
            );
            let player = game.player(13).unwrap();
            assert!(player.swap_hands(game));
            assert_eq!(
                hands(game),
                (Some("minecraft:torch".into()), Some("minecraft:diamond_sword".into()))
            );
            assert!(player.swap_hands(game));
            assert_eq!(
                hands(game),
                (Some("minecraft:diamond_sword".into()), Some("minecraft:torch".into()))
            );
            set_hands(game, Some(ItemStack::new("minecraft:diamond_sword", 1)), None);
            assert!(player.swap_hands(game));
            assert_eq!(hands(game), (None, Some("minecraft:diamond_sword".into())));
        });
    }
}

#[cfg(test)]
mod drop_selected_tests {
    use super::super::native_guest_turn_tests::with_guests;
    use horizon_client::items::ItemStack;

    /// Puts `stack` in the selected slot of guest 13's record, as a parked guest keeps it.
    fn give(game: &mut super::super::Game<'_>, stack: Option<ItemStack>) {
        let host = game.lan.as_mut().unwrap();
        let index = host.hub.guests.iter().position(|g| g.conn == 13).unwrap();
        host.hub.guests.update(index, |g| {
            let selected = g.selected;
            g.inventory[selected] = stack;
        });
    }

    /// The stack in guest 13's selected slot.
    fn held(game: &super::super::Game<'_>) -> Option<ItemStack> {
        let guest = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
        guest.inventory[guest.selected].clone()
    }

    /// One item goes at a time, then the rest with `whole_stack`; an empty hand throws nothing.
    #[test]
    #[ignore = "daily"]
    fn a_guest_drops_one_then_the_rest() {
        with_guests(|game| {
            give(game, Some(ItemStack::new("minecraft:stone", 3)));
            let player = game.player(13).unwrap();
            let before = game.dropped.len();
            assert!(player.drop_selected(game, false));
            assert_eq!(held(game).map(|s| s.count), Some(2));
            assert_eq!(game.dropped.len(), before + 1);
            assert!(player.drop_selected(game, true));
            assert!(held(game).is_none());
            assert_eq!(game.dropped.len(), before + 2);
            assert!(!player.drop_selected(game, true), "an empty hand throws nothing");
            assert_eq!(game.dropped.len(), before + 2);
        });
    }
}
