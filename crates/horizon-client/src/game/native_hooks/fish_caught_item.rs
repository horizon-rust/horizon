use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server player `player` (UUID) reeled in a catch, after `item_fished` did not cancel; `caught` is the
    /// first stack of the final drops, None when there are none. Server-side.
    pub(in crate::game) fn native_fish_caught_item(
        &mut self,
        player: u128,
        name: &str,
        caught: Option<&ItemStack>,
    ) {
        self.fire_fish_caught_item(player, name, caught);
    }
}
