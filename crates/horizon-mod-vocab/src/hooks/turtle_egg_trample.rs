//! The `turtle_egg_trample` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Turtle eggs at `pos` in `dimension` are about to be trampled by `entity`: vanilla
    /// `TurtleEggBlock.canDestroyEgg` answered yes for a mob stepping or falling on them. Set
    /// `canceled` (or return `Flow::Cancel`) to keep the eggs: no egg is destroyed, cracked or
    /// counted down. A player breaking an egg is not this hook (it is the break hooks).
    /// Server-side. Beyond NeoForge: none.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn turtle_egg_trample(dimension: &str, pos: [i32; 3], entity: EntityRef, canceled: &mut bool);
}
