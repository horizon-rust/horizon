//! `whitelist_add`: a player added to the dedicated server's whitelist, saved to whitelist.json at once.
use super::Game;
use horizon_server_files::user_lists::{NameAndId, WhiteEntry};

impl Game<'_> {
    /// `UserWhiteList.add` for one player, refused for a UUID already listed (the list's own `add` would save
    /// it again and answer true), a name no player can have, and no server list.
    pub(super) fn whitelist_add(&self, uuid: u128, name: &str) -> bool {
        if !valid_player_name(name) {
            return false;
        }
        let Some(shared) = self.server_lists.as_ref() else {
            return false;
        };
        let mut lists = shared.lock().unwrap();
        let user = NameAndId {
            id: uuid,
            name: name.to_owned(),
        };
        if lists.white.contains(&user) {
            return false;
        }
        lists.white.add(WhiteEntry { user })
    }
}

/// A name a player can have: 1 to 16 ASCII letters, digits or underscores.
fn valid_player_name(name: &str) -> bool {
    (1..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
