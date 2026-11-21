//! The `player_fish_detail` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0406

pub trait Hooks {
    /// `player_fish` with what the bobber holds: `caught` is the entity id the bobber holds on a
    /// state-4 reel-in (a mob, an armour stand, an item), `None` in every other state. Heard
    /// right after `player_fish`, from the same place, with the same `state` and `cancel`. A
    /// separate hook, because `player_fish` keeps its published signature (#19020). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge's ItemFishedEvent fires after the loot roll, in FishingHook.retrieve; Bukkit's PlayerFishEvent)"
    )]
    fn player_fish_detail();
}
