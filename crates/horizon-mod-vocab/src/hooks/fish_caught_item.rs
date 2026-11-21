//! The `fish_caught_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player `player` (UUID) reeled in a catch, after `item_fished` and its listeners
    /// are done and only when they did not cancel: `caught` is the first stack of the final drop
    /// list (`PlayerFishEvent.getCaught` on CAUGHT_FISH), None when the reel-in gave nothing.
    /// A listener here cannot change the catch; a catch of several stacks lists the rest in
    /// `item_fished`'s `drops`. `name` is the player's name, and the player's state is not swapped
    /// in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; ItemFishedEvent carries the list)",
        paper = "PlayerFishEvent (CAUGHT_FISH)"
    )]
    fn fish_caught_item();
}
