use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A brewing stand at `pos` finished a brew with `results`; `player` is the guest who last opened
    /// it, None when none did. Server-side.
    pub(in crate::game) fn native_brewing_owner_results(
        &mut self,
        player: Option<u128>,
        pos: [i32; 3],
        results: &[ItemStack],
    ) {
        self.fire_brewing_owner_results(player, pos, results);
    }
}
