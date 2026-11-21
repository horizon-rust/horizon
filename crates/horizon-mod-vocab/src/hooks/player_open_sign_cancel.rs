//! The `player_open_sign_cancel` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0411

pub trait Hooks {
    /// A server player is about to open a sign for editing, before the editor state is made, and
    /// before `player_open_sign`. `front` selects the front or back side. Set `cancel` to keep the
    /// editor closed; `player_open_sign` is then not heard. A separate hook, because
    /// `player_open_sign` only listens, after the editor opens, and keeps its published signature
    /// (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_open_sign_cancel();
}
