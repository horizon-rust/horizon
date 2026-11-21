use super::*;

impl Game<'_> {
    /// Notify after a player places a block, with the replaced state
    pub(in crate::game) fn native_block_placed(&mut self, call: super::fire::BlockPlacedCall<'_>) {
        self.fire_block_placed(call);
    }
}
