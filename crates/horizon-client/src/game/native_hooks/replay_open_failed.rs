use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A mod's accepted `replay_open` of `recording` opened nothing, for `error`.
    pub(in crate::game) fn native_replay_open_failed(
        &mut self,
        recording: &str,
        error: mod_api::ReplayError,
    ) {
        self.fire_replay_open_failed(recording, error);
    }
}
