//! The `entity_toggle_glide_cancel` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0358

pub trait Hooks {
    /// A guest's fall flying state is about to change on the server, before it is applied (Paper
    /// `EntityToggleGlideEvent`). `gliding` is the new state. Set `cancel` to keep the guest's
    /// current state: a refused start leaves them not gliding, a refused stop leaves them gliding;
    /// `entity_toggle_glide` is then not heard. A separate hook, because `entity_toggle_glide`
    /// only listens, after the change, and keeps its published signature (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_toggle_glide_cancel();
}
