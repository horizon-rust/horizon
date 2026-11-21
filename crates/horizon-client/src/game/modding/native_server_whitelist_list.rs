//! `server_whitelist`: the entries of the dedicated server's whitelist, read live.
use super::Game;
use mod_api::GameProfile;

impl Game<'_> {
    /// The whitelist's entries as its file holds them: the UUID and name of each, with no properties.
    pub(super) fn server_whitelist(&self) -> Vec<GameProfile> {
        let Some(shared) = self.server_lists.as_ref() else {
            return Vec::new();
        };
        let lists = shared.lock().unwrap();
        let out: Vec<GameProfile> = lists
            .white
            .entries()
            .into_iter()
            .map(|entry| GameProfile::new(entry.user.id, entry.user.name.clone(), Vec::new()))
            .collect();
        out
    }
}
