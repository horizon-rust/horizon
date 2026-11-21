//! `PlayerRef::chat`: a player says a line in public chat.
use super::Game;
use horizon_client::{lan::data::component_nbt, text::Text};
use mod_api::{PlayerChatRefusal, PlayerRef};

/// The longest line a client may send (`ServerboundChatPacket`'s 256-character string).
const MAX_CHARACTERS: usize = 256;

/// `StringUtil.isAllowedChatCharacter`: no control character, no DEL, no `§`.
fn allowed(c: char) -> bool {
    c >= ' ' && c != '\u{7f}' && c != '§'
}

/// The refusal for `message`, in the order the variants are listed after `Stale`.
fn refusal(message: &str) -> Option<PlayerChatRefusal> {
    if message.is_empty() {
        Some(PlayerChatRefusal::Empty)
    } else if message.starts_with('/') {
        Some(PlayerChatRefusal::Command)
    } else if message.chars().count() > MAX_CHARACTERS {
        Some(PlayerChatRefusal::TooLong)
    } else if !message.chars().all(allowed) {
        Some(PlayerChatRefusal::IllegalCharacter)
    } else {
        None
    }
}

impl Game<'_> {
    /// An unsigned line from `player` to every placed player (`Hub::chat_as`).
    pub(super) fn player_chat(
        &mut self,
        player: PlayerRef,
        message: &str,
    ) -> Result<(), PlayerChatRefusal> {
        let on = self.native_acting_player() == Some(player)
            || self.online_player_record(player).is_some();
        let Some(host) = self.lan.as_ref().filter(|_| on) else {
            return Err(PlayerChatRefusal::Stale);
        };
        if let Some(refusal) = refusal(message) {
            return Err(refusal);
        }
        let line = component_nbt(&Text::literal(message).component());
        if host.hub.chat_as(player.uuid(), line) {
            Ok(())
        } else {
            Err(PlayerChatRefusal::Stale)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::native_guest_turn_tests::with_guests;
    use horizon_client::lan::{Out, PlayerSender};
    use horizon_protocol::packets::play;
    use std::sync::{Arc, mpsc};

    /// A line's refusal, one by one, in the order the enum lists them.
    #[test]
    fn a_line_is_refused_by_name_in_order() {
        assert_eq!(refusal(""), Some(PlayerChatRefusal::Empty));
        assert_eq!(refusal("/op me"), Some(PlayerChatRefusal::Command));
        assert_eq!(refusal(&"a".repeat(257)), Some(PlayerChatRefusal::TooLong));
        assert_eq!(refusal(&"a".repeat(256)), None);
        assert_eq!(
            refusal("hi\u{7}"),
            Some(PlayerChatRefusal::IllegalCharacter)
        );
        assert_eq!(
            refusal("a §c b"),
            Some(PlayerChatRefusal::IllegalCharacter)
        );
        assert_eq!(refusal("hello there"), None);
        // Over-long and illegal: the length is named first.
        assert_eq!(
            refusal(&format!("{}§", "a".repeat(300))),
            Some(PlayerChatRefusal::TooLong)
        );
    }

    /// The disguised chat lines a guest was sent since the last call.
    fn disguised(rx: &mpsc::Receiver<Out>) -> usize {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(_) => Vec::new(),
            })
            .filter(|body| {
                let mut reader = horizon_protocol::buf::Reader::new(body);
                let id = reader.varint().unwrap();
                matches!(
                    play::Clientbound::decode(id, reader.rest()),
                    Ok(play::Clientbound::DisguisedChat(_))
                )
            })
            .count()
    }

    /// A player's line reaches every guest once; a refused line reaches nobody.
    #[test]
    #[ignore = "daily"]
    fn a_players_line_reaches_every_guest_once() {
        with_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            let receivers: Vec<_> = [12, 13]
                .into_iter()
                .map(|conn| {
                    let (tx, rx) = mpsc::channel();
                    game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                        guest.sender = PlayerSender::new(tx, Arc::default());
                    });
                    rx
                })
                .collect();
            for rx in &receivers {
                disguised(rx);
            }
            assert_eq!(game.player_chat(a, "hello there"), Ok(()));
            assert_eq!(receivers.iter().map(disguised).collect::<Vec<_>>(), [1, 1]);
            assert_eq!(
                game.player_chat(b, "/op me"),
                Err(PlayerChatRefusal::Command)
            );
            assert_eq!(receivers.iter().map(disguised).collect::<Vec<_>>(), [0, 0]);
            let gone = mod_api::__private::game::player_ref(a.entity(), 999);
            assert_eq!(game.player_chat(gone, "hi"), Err(PlayerChatRefusal::Stale));
        });
    }
}
