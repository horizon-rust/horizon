//! The `menu_result_take` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player took `stack` from a result slot (`results`) of a mod menu type's
    /// block menu: `slot` is the container slot, `pos` the menu's block, `menu` its type,
    /// `player` the player's UUID and `player_entity_id` their network id. A click that picks
    /// the stack up and a shift-click that moves it both count, as vanilla's result slots call
    /// `Slot.onTake` (`checkTakeAchievements`): what a furnace-like machine hands out then (its
    /// recipes' experience, at the player) goes here. Asked after the click, once per take.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "menu",
        key = "menu",
        neoforge = "none (vanilla Slot.onTake / checkTakeAchievements, which a menu's result slot overrides)"
    )]
    fn menu_result_take();
}
