use super::*;

impl GameLevel<'_> {
    /// On an authoritative server, a natural grower spreads from `from` into
    /// `to` (`kind` is `vine` or `sculk`): `VineBlock.randomTick` and
    /// `SculkSpreader`. Set `canceled` to leave the block at `to` as it is.
    #[inline(always)]
    pub(in crate::game) fn native_natural_block_spread(
        &self,
        dimension: LevelDimension,
        kind: &str,
        from: BlockPos,
        to: BlockPos,
        canceled: &mut bool,
    ) {
        self.fire_natural_block_spread(dimension, kind, from, to, canceled);
    }
}
