//! The `pressure_plate_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A pressure plate at `pos` (`state`) is about to change its signal from `from` to `to`
    /// (`BasePressurePlateBlock.checkPressed`: an entity moves on or off it, or the plate's timer
    /// ends; a weighted plate's `from` and `to` are its weights). Only a change is asked. Set
    /// `cancel` to keep the plate as it was: no new signal, no neighbour update, no sound or game
    /// event. Server-side, on authoritative levels only.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (vanilla BasePressurePlateBlock.checkPressed; NeoForge adds no event for it)",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_pressure_plate_change"
    )]
    fn pressure_plate_change();
}
