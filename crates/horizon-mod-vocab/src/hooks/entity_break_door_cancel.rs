//! The `entity_break_door_cancel` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0357

pub trait Hooks {
    /// A mob is about to break the door at `pos` (Paper `EntityBreakDoorEvent`, as
    /// `BreakDoorGoal.tick` removes it), before the door is removed and before `entity_break_door`
    /// hears the break. Set `cancel` and the door stays as it is: no block is removed, no level
    /// event plays and `entity_break_door` is not heard. A separate hook, because
    /// `entity_break_door` only listens and keeps its published signature (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_break_door_cancel();
}
