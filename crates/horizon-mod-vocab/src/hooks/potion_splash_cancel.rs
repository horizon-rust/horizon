//! The `potion_splash_cancel` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0353

pub trait Hooks {
    /// A thrown splash or lingering potion (`item`, its item id) is about to
    /// apply its hit effects at `pos` (NeoForge's `PotionSplashEvent`, asked
    /// before the effects; Paper's `PotionSplashEvent#setCancelled`). Set
    /// `cancel` to skip them: the potion breaks with no effect on anything it
    /// would have hit, and `potion_splash` does not fire for it. Experience
    /// bottles are never asked. Server-side, once per landed potion.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "PotionSplashEvent",
        cancellable = true
    )]
    fn potion_splash_cancel(item: &str, pos: [f64; 3], cancel: &mut bool);
}
