//! The `player_advancement_done_cancel` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0404

pub trait Hooks {
    /// A server player completed an advancement, after progress was recorded, heard right after
    /// `player_advancement_done`. Set `cancel` to withhold its toast, announcement and rewards; the
    /// advancement stays done in the progress. Not the message override (Paper's
    /// `message(Component)`): only the cancel. A separate hook, because `player_advancement_done`
    /// only listens and keeps its published signature (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "AdvancementEvent.AdvancementEarnEvent",
        cancellable = true
    )]
    fn player_advancement_done_cancel();
}
