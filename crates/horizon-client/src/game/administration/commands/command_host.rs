//! `Game` as the host of the commands that moved to `horizon_gameplay::commands`. Each command
//! family has its trait there and its impl in a file of `command_host/`; every method
//! calls the `Game` code it names and adds no logic.
mod gamerule;
mod help;
mod time;
mod weather;

use super::Game;
use horizon_client::text::Text;
use horizon_gameplay::commands::host::CommandHost;

impl CommandHost for Game<'_> {
    fn feedback(&mut self, message: Text) {
        Game::feedback(self, message);
    }
    fn feedback_unbroadcast(&mut self, message: Text) {
        Game::feedback_unbroadcast(self, message);
    }
}
