use crate::game::Game;
use horizon_client::text::Text;
use horizon_gameplay::commands::version::VersionHost;

impl VersionHost for Game<'_> {
    fn source_silent(&self) -> bool {
        self.command_source.is_some_and(|s| s.silent)
    }
    fn chat_system_message(&mut self, text: Text) {
        self.system_message(text);
    }
    fn protocol_version(&self) -> i32 {
        horizon_protocol::PROTOCOL_VERSION
    }
}
