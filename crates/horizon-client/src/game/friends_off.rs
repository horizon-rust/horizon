//! Play with Friends left out of the build (the `friends` feature off, as in the server build):
//! no world is shared, the pause menu has no Friends button (`friends_shown` is false, as on
//! another server) and `horizon-ctl friends` says it isn't there. game/friends.rs is the real one.
use super::Game;
use horizon_client::Result;

pub const HELP: &[&str] = &[];

/// The pause menu's Friends icon is vanilla's, drawn inactive: its name for the widget tree.
pub const TITLE: &str = "Play with Friends";
pub fn title() -> String {
    horizon_client::lang::get_or("horizon.friends.title", TITLE)
}

impl Game<'_> {
    /// `horizon-ctl friends ...`.
    pub(super) fn friends_control(&mut self, words: &[&str]) -> Result<Option<serde_json::Value>> {
        if words[0] != "friends" {
            return Ok(None);
        }
        Err("this build has no Play with Friends (the `friends` feature)".into())
    }
}
