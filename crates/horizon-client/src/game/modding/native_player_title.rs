//! `player_show_title`: a title and a subtitle on one player's screen.
use super::Game;
use horizon_client::{lan::data::component_nbt, text::Text};
use horizon_protocol::packets::play::{self, Clientbound as C};
use mod_api::PlayerRef;

/// The text packet a line is sent as.
fn text(line: &Text) -> play::Text {
    play::Text {
        text: component_nbt(&line.component()),
    }
}

impl Game<'_> {
    /// The timings, then the subtitle, then the title (what `/title` sends one by one); a line
    /// that is None sends nothing. False when the player isn't on.
    pub(super) fn player_show_title(
        &mut self,
        player: PlayerRef,
        title: Option<Text>,
        subtitle: Option<Text>,
        fade_in: i32,
        stay: i32,
        fade_out: i32,
    ) -> bool {
        let on = self.native_acting_player() == Some(player)
            || self.online_player_record(player).is_some();
        if !on {
            return false;
        }
        let uuid = player.uuid();
        let mut sent = self.native_send(
            uuid,
            C::SetTitlesAnimation(play::TitlesAnimation {
                fade_in,
                stay,
                fade_out,
            }),
        );
        if let Some(subtitle) = &subtitle {
            sent &= self.native_send(uuid, C::SetSubtitleText(text(subtitle)));
        }
        if let Some(title) = &title {
            sent &= self.native_send(uuid, C::SetTitleText(text(title)));
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use super::*;
    use horizon_client::lan::{Out, PlayerSender};
    use std::sync::{Arc, mpsc};

    /// The play packets a guest was sent since the last call, decoded.
    fn packets(rx: &mpsc::Receiver<Out>) -> Vec<play::Clientbound> {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(_) => Vec::new(),
            })
            .filter_map(|body| {
                let mut reader = horizon_protocol::buf::Reader::new(&body);
                let id = reader.varint().ok()?;
                play::Clientbound::decode(id, reader.rest()).ok()
            })
            .filter(|p| {
                matches!(
                    p,
                    C::SetTitlesAnimation(_) | C::SetSubtitleText(_) | C::SetTitleText(_)
                )
            })
            .collect()
    }

    /// The timings come first, then the subtitle, then the title; a None line sends nothing;
    /// only the named player is sent them.
    #[test]
    #[ignore = "daily"]
    fn a_title_goes_out_as_timings_subtitle_title() {
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
                packets(rx);
            }
            let both = game.player_show_title(
                a,
                Some(Text::literal("Big")),
                Some(Text::literal("small")),
                10,
                70,
                20,
            );
            assert!(both);
            let got = packets(&receivers[0]);
            assert_eq!(got.len(), 3);
            let C::SetTitlesAnimation(times) = &got[0] else {
                panic!("timings first: {got:?}");
            };
            assert_eq!((times.fade_in, times.stay, times.fade_out), (10, 70, 20));
            assert!(matches!(got[1], C::SetSubtitleText(_)), "then the subtitle");
            assert!(matches!(got[2], C::SetTitleText(_)), "then the title");
            assert!(packets(&receivers[1]).is_empty(), "nobody else");
            assert!(game.player_show_title(b, None, Some(Text::literal("only")), -1, -1, -1));
            let got = packets(&receivers[1]);
            assert_eq!(got.len(), 2);
            let C::SetTitlesAnimation(times) = &got[0] else {
                panic!("timings first: {got:?}");
            };
            assert_eq!((times.fade_in, times.stay, times.fade_out), (-1, -1, -1));
            assert!(matches!(got[1], C::SetSubtitleText(_)));
            let gone = mod_api::__private::game::player_ref(a.entity(), 999);
            assert!(!game.player_show_title(gone, None, None, 0, 0, 0));
        });
    }
}
