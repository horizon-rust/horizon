//! `player_set_display_score`: a score under a player's name in one viewer's client only.
use super::Game;
use mod_api::PlayerRef;

impl Game<'_> {
    /// Shows `score` for `target` in `slot` (0 Tab list, 2 below the name) on `viewer`'s client only;
    /// None takes it out. False when either player is not online, or the slot is neither 0 nor 2.
    pub(super) fn player_set_display_score(
        &mut self,
        viewer: PlayerRef,
        slot: u8,
        target: PlayerRef,
        score: Option<i32>,
    ) -> bool {
        let Some(host) = &mut self.lan else {
            return false;
        };
        host.hub
            .set_display_score(viewer.uuid(), slot, target.uuid(), score)
    }
}
