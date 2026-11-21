//! `server_online_mode`: the `online-mode` the dedicated server runs with.
use super::Game;

impl Game<'_> {
    /// The dedicated server's `online-mode`, None off a dedicated server and before it listens.
    pub(super) fn server_online_mode(&self) -> Option<bool> {
        self.native_online_mode()
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;

    /// A game that is not a dedicated server listens on no port, so it reports no online mode.
    #[test]
    #[ignore = "daily"]
    fn server_online_mode_is_none_off_a_dedicated_server() {
        with_guests(|game| {
            assert_eq!(game.server_online_mode(), None);
        });
    }
}
