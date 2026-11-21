//! The `player_take_lectern_book_cancel` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0412

pub trait Hooks {
    /// A server player is about to take the book from a lectern, before the menu and block entity
    /// change, and before `player_take_lectern_book`. `book` is the item id of the book it takes.
    /// Set `cancel` to leave the book on the lectern; `player_take_lectern_book` is then not heard.
    /// A separate hook, because `player_take_lectern_book` only listens, after the take, and
    /// keeps its published signature (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_take_lectern_book_cancel();
}
