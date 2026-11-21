//! `whitelist_remove`: a player removed from the dedicated server's whitelist, saved to whitelist.json at once.
use super::Game;
use horizon_server_files::user_lists::NameAndId;

impl Game<'_> {
    /// `UserWhiteList.remove` by UUID: the list keys its entries by UUID alone, so no name is needed.
    pub(super) fn whitelist_remove(&self, uuid: u128) -> bool {
        let Some(shared) = self.server_lists.as_ref() else {
            return false;
        };
        let user = NameAndId {
            id: uuid,
            name: String::new(),
        };
        shared.lock().unwrap().white.remove(&user)
    }
}
