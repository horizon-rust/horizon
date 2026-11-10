//! Play with Friends left out of the build (the `friends` feature off, as in the server build):
//! no world is shared, no page is registered for the pause menu's Friends icon (it stays
//! inactive, named by vanilla's tooltip) and `horizon-ctl friends` says it isn't there.
//! game/friends.rs is the real one.
use super::Game;
use horizon_client::Result;

pub const HELP: &[&str] = &[];

impl Game<'_> {
    /// `horizon-ctl friends ...`.
    pub(super) fn friends_control(&mut self, words: &[&str]) -> Result<Option<serde_json::Value>> {
        if words[0] != "friends" {
            return Ok(None);
        }
        Err("this build has no Play with Friends (the `friends` feature)".into())
    }
}
