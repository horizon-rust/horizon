//! The `raid_omen_trigger` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player with a raid omen is about to trigger a raid at `pos` in `dimension` (vanilla
    /// `Raids.createOrExtendRaid(ServerPlayer, BlockPos)`, reached from a bad omen in a village).
    /// `player` is the player who triggers it. Set `canceled` (or return `Flow::Cancel`) to
    /// create or extend no raid: no raid is made, no raid bar shows and the omen is kept.
    /// Server-side. Paper: `RaidTriggerEvent`, which is cancellable and names the player.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn raid_omen_trigger(dimension: &str, pos: [i32; 3], player: PlayerRef, canceled: &mut bool);
}
