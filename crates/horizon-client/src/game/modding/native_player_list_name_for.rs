//! `player_set_list_name_for`: a player's name in one viewer's Tab list only.
use super::Game;
use horizon_client::text::Text;
use mod_api::PlayerRef;

impl Game<'_> {
    /// Shows `name` for `target` in `viewer`'s Tab list only; None sends that viewer the target's usual
    /// entry again. False when either player is not online or the server list is unavailable.
    pub(super) fn player_set_list_name_for(
        &mut self,
        viewer: PlayerRef,
        target: PlayerRef,
        name: Option<Text>,
    ) -> bool {
        let Some(host) = &mut self.lan else {
            return false;
        };
        host.hub
            .set_list_name_for(viewer.uuid(), target.uuid(), name)
    }
}
