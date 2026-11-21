//! The `replay_advanced` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The replay's clock moved this frame, by playing or by a seek: `position_ms` is where
    /// it is now, in ms from the recording's start, after the recording's packets up to there
    /// were applied. At most once a frame, and not on a frame the clock stood still (paused,
    /// at the end, or the frame it opened on, which `replay_opened` tells of). For a timeline
    /// or a camera path that follows playback.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon's replays; NeoForge has no replay playback)"
    )]
    fn replay_advanced();
}
