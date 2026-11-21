//! `player_resend_command_tree` (`ServerHost`): one online player gets their command tree again,
//! with the mods' say on it, for a mod whose answers about that player have changed.
use super::Game;
use mod_api::{PlayerRef, PlayerWorldRefusal as Refusal};

impl Game<'_> {
    /// `Commands.sendCommands` to one player: the tree their permission level may use, with the
    /// mods' say on it (as `resend_commands_filtered` gives every player). Only that player is
    /// sent a packet. A stale or unknown player is `Stale`, a fake one `NoConnection`, a client
    /// `Client`; nothing is sent to anyone then. Called from `player_commands_sent` it does
    /// nothing: the pass that asked the hook sends the trees when the hook returns, so one call
    /// cannot start the hook again.
    pub(super) fn player_resend_command_tree(&mut self, player: PlayerRef) -> Result<(), Refusal> {
        let conn = self.player_connection(player)?;
        let placed = self
            .lan
            .as_ref()
            .and_then(|lan| lan.hub.guest(conn))
            .is_some_and(|guest| guest.placed);
        if !placed {
            return Err(Refusal::Stale);
        }
        // Called from `player_commands_sent`, while the server asks what a tree keeps: that pass
        // sends the tree when the hook returns, so this call does nothing and is not an error.
        if self.native_commands.filtering {
            return Ok(());
        }
        self.native_filter_commands(Some(conn));
        let host = self.lan.as_ref().ok_or(Refusal::Stale)?;
        let index = host
            .hub
            .guests
            .connection_index(conn)
            .ok_or(Refusal::Stale)?;
        host.hub.send_commands(index);
        Ok(())
    }
}
