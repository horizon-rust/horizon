//! The `natural_block_spread` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// On an authoritative server, a natural grower spreads from `from` into
    /// `to`: a vine grows onto the empty face beside it (`kind` `vine`,
    /// `VineBlock.randomTick`), or sculk spreads into a neighbour of a vein or
    /// cursor (`kind` `sculk`, `SculkSpreader`). Set `canceled` to leave the
    /// block at `to` as it is. Vines ask on every random tick: answer fast.
    #[notes = r#"## Natural block spread

`block_spread` covers fluids and fire. `natural_block_spread` covers the
blocks that grow on their own: vines and sculk so far. `kind` names the grower,
`from` is the block that spreads and `to` the position it spreads into. The
hook runs before the block at `to` is written, so a refusal leaves the world as
it was. Vines and sculk draw their random numbers before the hook is asked,
so a refusal does not change the rest of the tick.

Not covered yet: amethyst, chorus and bamboo spread, and the vertical vine face."#]
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        target = "horizon_client::game::native_hooks::natural_block_spread::GameLevel<'_>::native_natural_block_spread",
        neoforge = "none (NeoForge has no vine or sculk spread event)",
        paper = "BlockSpreadEvent"
    )]
    fn natural_block_spread();
}
