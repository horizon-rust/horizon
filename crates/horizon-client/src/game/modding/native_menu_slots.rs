//! `player_open_menu_slots`: the open menu of a guest, refusals of `player_container_menu` lifted.

use super::*;
use mod_api::{MenuRef, PlayerRef};

impl Game<'_> {
    /// `HumanEntity.getOpenInventory` for a guest: the menu it has open. A client's own menu is not lent here.
    pub(super) fn player_open_menu_slots(&self, player: PlayerRef) -> Option<MenuRef> {
        if self.remote.is_some() {
            return None;
        }
        let super::native_server::PlayerRead::Guest(guest) = self.player_handle_read(player)? else {
            return None;
        };
        Some(mod_api::__private::game::menu_ref(
            player,
            guest.menu.as_ref().map_or(0, |m| m.window),
            guest.menu_generation,
        ))
    }
}
