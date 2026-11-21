//! The `player_death_message` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `ServerPlayer.die` builds a player's death message once the text is made, before it goes to
    /// the chat and the death screen (Paper `PlayerDeathEvent#deathMessage`). `message` holds the
    /// text: replace it, or set it to `None` and nothing is said to anyone (the dying player's
    /// death screen gets an empty message). Every mod is asked in order. Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "message",
        neoforge = "none (vanilla ServerPlayer.die message; NeoForge's LivingDeathEvent carries no message)"
    )]
    fn player_death_message();
}
