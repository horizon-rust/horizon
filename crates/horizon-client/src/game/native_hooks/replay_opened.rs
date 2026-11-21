use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A replay is open: its recording id, its length and whether it owns its world.
    pub(in crate::game) fn native_replay_opened(
        &mut self,
        recording: &str,
        duration_ms: u32,
        standalone: bool,
    ) {
        self.fire_replay_opened(recording, duration_ms, standalone);
    }
}
