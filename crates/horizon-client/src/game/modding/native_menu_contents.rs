//! `menu_contents`: every slot of a menu, read in order through `menu_slot`.

use super::*;
use mod_api::MenuRef;

impl Game<'_> {
    /// Every slot of `menu`, in slot order, through `menu_slot`. `None` when the menu has no count or a read is refused.
    pub(super) fn menu_contents(&self, menu: MenuRef) -> Option<Vec<Option<ItemStack>>> {
        let count = self.menu_slot_count(menu)?;
        (0..count).map(|slot| self.menu_slot(menu, slot)).collect()
    }
}
