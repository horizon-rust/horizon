//! `player_locale`: the language the player's client announced.
use super::Game;
use mod_api::PlayerRef;

impl Game<'_> {
    /// The language of the player's latest client information (`ServerPlayer.language`),
    /// `en_us` before one arrives. None for a stale or unavailable player and a fake one.
    pub(super) fn player_locale(&self, player: PlayerRef) -> Option<String> {
        Some(self.online_player_record(player)?.info.language.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;

    /// Each player reads their own client's language, and a change of it reads back.
    #[test]
    #[ignore = "daily"]
    fn player_locale_reads_the_language_the_client_announced() {
        with_guests(|game| {
            let a = game.player(12).unwrap();
            let b = game.player(13).unwrap();
            assert_eq!(game.player_locale(a), Some("en_us".to_owned()));
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update_connection(13, |g| g.info.language = "de_de".into());
            assert_eq!(game.player_locale(b), Some("de_de".to_owned()));
            assert_eq!(game.player_locale(a), Some("en_us".to_owned()));
            let gone = mod_api::__private::game::player_ref(a.entity(), 999);
            assert_eq!(game.player_locale(gone), None);
        });
    }
}
