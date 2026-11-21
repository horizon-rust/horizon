//! The `left_click_block_player` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0408

pub trait Hooks {
    /// `left_click_block` with the clicking player: `player` is the guest's uuid
    /// (`PlayerInteractEvent.getEntity`); a click by the game's own player is not asked. Heard
    /// right after `left_click_block`, from the same place, with the same `canceled`. A separate
    /// hook, because `left_click_block` keeps its published signature (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "PlayerInteractEvent.LeftClickBlock"
    )]
    fn left_click_block_player();
}
