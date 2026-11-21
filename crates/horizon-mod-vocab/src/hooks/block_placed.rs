//! The `block_placed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A block a player placed is written at `pos` in `dimension`: `placed` is its final state and
    /// `replaced` the state it replaced (air, grass, water...). Heard after the write and after
    /// `on_block_placed_by`, for a block item a player used; not for dispensers, mobs or a refused
    /// placement (`entity_place` is the cancel hook, before the write). Listen-only. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge: BlockEvent.EntityPlaceEvent fires before the write)"
    )]
    fn block_placed(player: Option<PlayerRef>, dimension: &str, pos: [i32; 3], placed: BlockState, replaced: BlockState);
}
