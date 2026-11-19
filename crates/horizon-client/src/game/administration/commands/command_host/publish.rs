use crate::game::Game;
use crate::game::lan::LanSettings;
use horizon_gameplay::commands::publish::PublishHost;

impl PublishHost for Game<'_> {
    fn lan_port(&self) -> Option<u16> {
        if self.serving() {
            self.published_port()
        } else {
            self.lan.as_ref().map(|l| l.settings.port)
        }
    }
    fn open_to_lan(&mut self, port: u16, guest_commands: bool) -> bool {
        let settings = LanSettings {
            port,
            guest_commands,
            // `PublishCommand.publishServer` only opens the LAN: the world's
            // force-game-mode setting stays what it was.
            force_game_mode: self
                .lan
                .as_ref()
                .is_none_or(|host| host.settings.force_game_mode),
        };
        self.serving() && self.publish(settings).is_ok()
    }
    fn close_to_lan(&mut self) -> bool {
        self.unpublish()
    }
}
