use super::*;

impl GameLevel<'_> {
    /// On an authoritative server, the block at `pos`, in state `from`, is about to grow into
    /// `to` by natural growth (sculk growth above a charged sculk block). Set `canceled` to
    /// leave `pos` as it is.
    #[inline(always)]
    pub(in crate::game) fn native_block_grow(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        from: BlockStateId,
        to: BlockStateId,
        canceled: &mut bool,
    ) {
        self.fire_block_grow(dimension, pos, from, to, canceled);
    }
}
