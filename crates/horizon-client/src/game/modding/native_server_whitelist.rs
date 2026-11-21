//! `server_is_whitelisted`: whether a UUID has an entry on the dedicated server's whitelist.
use super::Game;

impl Game<'_> {
    /// Bukkit's `OfflinePlayer.isWhitelisted`: the entry in the whitelist file, read live. The entry
    /// is what counts, not the gate (`PlayerList.isWhiteListed` also lets ops and bypass names in).
    pub(super) fn server_is_whitelisted(&self, uuid: u128) -> bool {
        let Some(lists) = self.server_lists.as_ref() else {
            return false;
        };
        let lists = lists.lock().unwrap();
        lists.white.entries().iter().any(|entry| entry.user.id == uuid)
    }
}
