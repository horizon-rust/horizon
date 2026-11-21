//! The `spawn_position_check` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A natural spawn is about to be position-checked at `pos` in `dimension` for `player`, the
    /// player the spawn is for (the nearest one in range, vanilla natural spawning). Set `canceled`
    /// (or return `Flow::Cancel`) to refuse the spawn at this spot before vanilla's position checks run:
    /// no mob is made there. A canceled check skips vanilla's draws for it, so the random order changes
    /// only then. Server-side. Beyond NeoForge: none. A listener should be a pure function of its inputs.
    #[hook(
        side = "server",
        contract = "cancel",
        host = "read",
        out = "canceled",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn spawn_position_check(dimension: &str, pos: [f64; 3], player: PlayerRef, canceled: &mut bool);
}
