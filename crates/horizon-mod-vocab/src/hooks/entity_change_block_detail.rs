//! The `entity_change_block_detail` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0359

pub trait Hooks {
    /// A mob changed a block at `pos` of the level `dimension` (a `&str`, `minecraft:overworld` and
    /// so on), after the new state was applied; heard after `entity_change_block`, from the same
    /// place. `entity` and `kind` are the mob; `from` and `to` are the block names. A separate
    /// hook, because `entity_change_block` keeps its published signature (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_change_block_detail(
        dimension: &str,
        entity: u32,
        kind: &str,
        pos: [i32; 3],
        from: &str,
        to: &str,
    );
}
