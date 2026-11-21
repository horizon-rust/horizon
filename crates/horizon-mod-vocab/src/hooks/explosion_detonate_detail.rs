//! The `explosion_detonate_detail` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0400

pub trait Hooks {
    /// `explosion_detonate` with the exploding entity: heard right after it, from the same place,
    /// on the same lists. `source` is the entity that explodes (`getDirectSourceEntity`, as
    /// `explosion_start` names it; Paper's `EntityExplodeEvent.getEntity()`), `None` when Horizon
    /// tracks none, and it is never in `entities`. A separate hook, because `explosion_detonate`
    /// keeps its published signature (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "blocks",
        neoforge = "ExplosionEvent.Detonate"
    )]
    fn explosion_detonate_detail();
}
