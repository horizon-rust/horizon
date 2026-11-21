//! The `prepare_hopper_transfer_cancel` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0413

pub trait Hooks {
    /// A hopper at `pos` began an item transfer attempt, after its cooldown and enabled checks,
    /// before pushing or pulling any items, and before `prepare_hopper_transfer`. Set `cancel` to
    /// skip this attempt: nothing is pushed or pulled and `prepare_hopper_transfer` is not heard.
    /// A separate hook, because `prepare_hopper_transfer` only listens and keeps its published
    /// signature (#19020). Server-side, including the single-player server.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (NeoForge has no hopper event)"
    )]
    fn prepare_hopper_transfer_cancel();
}
