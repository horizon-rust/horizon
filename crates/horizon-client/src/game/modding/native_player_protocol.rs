//! `player_protocol_version`: the protocol version a player's handshake stated.
use super::Game;
use mod_api::PlayerRef;

impl Game<'_> {
    /// The protocol version the client's handshake stated. None for a stale or unavailable
    /// player, a fake one, and a connection that sent no handshake.
    pub(super) fn player_protocol_version(&self, player: PlayerRef) -> Option<i32> {
        let protocol = self.online_player_record(player)?.sender.protocol;
        (protocol != 0).then_some(protocol)
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;

    /// Each player reads the protocol their own handshake stated; one with none reads None.
    #[test]
    #[ignore = "daily"]
    fn player_protocol_version_reads_the_handshake_number() {
        with_guests(|game| {
            let a = game.player(12).unwrap();
            let b = game.player(13).unwrap();
            assert_eq!(game.player_protocol_version(b), None, "no handshake");
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update_connection(13, |g| g.sender.protocol = 767);
            assert_eq!(game.player_protocol_version(b), Some(767));
            assert_eq!(game.player_protocol_version(a), None, "the other player's own");
            let gone = mod_api::__private::game::player_ref(a.entity(), 999);
            assert_eq!(game.player_protocol_version(gone), None);
        });
    }
}
