use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server player took `stack` from a result slot of a mod menu type's block menu.
    pub(in crate::game) fn native_menu_result_take(
        &mut self,
        player: u128,
        player_entity_id: i32,
        menu: &str,
        pos: [i32; 3],
        slot: u32,
        stack: &ItemStack,
    ) {
        self.fire_menu_result_take(player, player_entity_id, menu, pos, slot, stack);
    }
}
