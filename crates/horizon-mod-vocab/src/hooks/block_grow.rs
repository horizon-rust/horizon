//! The `block_grow` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// On an authoritative server, the block at `pos`, in state `from`, is about to grow into
    /// `to` by natural growth: sculk growth (a sculk shrieker or sensor) above a sculk block
    /// when a charge is spent. Set `canceled` to leave `pos` as it is; the charge is spent
    /// either way. Grass and mycelium spread and rock growth are not in the engine yet.
    #[notes = r#"## Natural block growth

`block_grow` is the in-place counterpart of `natural_block_spread`: the block at `pos` changes
into `to` by its own growth, and `from` is the state it had. Only sculk growth is wired so far.
A refusal leaves `pos` as it was and the sculk charge is still spent, as in vanilla when the
growth position is not a valid place. Grass and mycelium spreading onto dirt, and the growth of
pointed dripstone, are not in the engine and will need their own call sites."#]
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        target = "horizon_client::game::native_hooks::block_grow::GameLevel<'_>::native_block_grow",
        neoforge = "none (no NeoForge event for sculk growth was found)",
        paper = "BlockGrowEvent (crops and plants; not checked for sculk)"
    )]
    fn block_grow();
}
