//! The `block_dispense` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A dispenser at `pos` is about to dispense `stack` (Paper
    /// `BlockDispenseEvent`), before the item leaves it and before any
    /// `dispense_item` or `dispense_method` answer is asked. Set `cancel` and
    /// nothing is dispensed: the stack stays in the dispenser. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_dispense();
}
