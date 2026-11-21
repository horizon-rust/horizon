//! `scoreboard_score`: a holder's score on an objective, read from the world's scoreboard.
use super::Game;

impl Game<'_> {
    /// The value the holder has on the objective, None when it has none or this is a client.
    pub(super) fn scoreboard_score(&self, holder: &str, objective: &str) -> Option<i32> {
        if self.remote.is_some() {
            return None;
        }
        self.score
            .board
            .score(holder, objective)
            .map(|score| score.value)
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;

    /// A score a holder has reads back by its key and objective; other holders and
    /// objectives have none.
    #[test]
    #[ignore = "daily"]
    fn scoreboard_score_reads_the_holders_value() {
        with_guests(|game| {
            assert_eq!(game.scoreboard_score("guest", "kills"), None);
            game.score.board.score_mut("guest", "kills").value = 7;
            assert_eq!(game.scoreboard_score("guest", "kills"), Some(7));
            assert_eq!(game.scoreboard_score("guest", "deaths"), None);
            assert_eq!(game.scoreboard_score("someone_else", "kills"), None);
        });
    }
}
