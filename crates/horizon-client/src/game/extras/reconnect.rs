//! Auto-reconnect (`autoReconnect`, docs/extras.md) runs on the engine's connection hooks
//! (`horizon_client::reconnect::HOOKS`, registered in `entry.rs`). Its setting still lives in
//! the engine's extras, so it is handed over here; it leaves with the extras page (plan M5).
use crate::game::Game;

impl Game<'_> {
    /// Once per frame: the `autoReconnect` setting, for the hook to read when a connection ends.
    pub(in crate::game) fn sync_reconnect_setting(&self) {
        horizon_client::reconnect::HOOKS.set_enabled(self.settings.extras.auto_reconnect);
    }
}
