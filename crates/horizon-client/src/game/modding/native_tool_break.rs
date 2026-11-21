//! `tool_break_block`: the break as the acting player, with drops and tool damage.

use super::*;
use mod_api::PlayerRef;

impl Game<'_> {
    /// `Player#breakBlock` for the acting player. Other players are refused as `NotThisPlayer`.
    pub(super) fn tool_break_block(&mut self, player: PlayerRef, pos: [i32; 3]) -> BlockBreakResult {
        if self.local_player() != Some(player) {
            return BlockBreakResult::NotThisPlayer;
        }
        match self.player_break_block(pos) {
            Ok(true) => BlockBreakResult::Broken,
            Ok(false) | Err(_) => BlockBreakResult::Kept,
        }
    }
}
