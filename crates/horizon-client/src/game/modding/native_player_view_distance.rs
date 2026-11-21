//! `Player.setViewDistance` for native server mods, through a `PlayerRef`
//! (docs/modding/src/how-to/native-mods.md): one player's chunk radius, from 2 to
//! 32 and at most the server's view distance. Their guest record gets the radius
//! (the same rule `PlayerList.setViewDistance` applies to every record), so the
//! chunks follow it, and their client is told the radius at once.
use super::Game;
use horizon_protocol::packets::play::{self, Clientbound as C};
use mod_api::PlayerRef;

impl Game<'_> {
    /// `Player.setViewDistance`. False when the value is outside 2 to 32, or
    /// the player's record is not in the player list here.
    pub(super) fn player_set_view_distance(&mut self, player: PlayerRef, chunks: i32) -> bool {
        if !(2..=32).contains(&chunks) {
            return false;
        }
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        let distance = chunks.min(host.hub.config.view_distance);
        let mut found = false;
        host.hub.guests.update_all(|g| {
            if g.uuid == uuid {
                g.view_distance = distance;
                g.center = None;
                found = true;
            }
        });
        found && self.native_send(uuid, C::SetChunkCacheRadius(play::Distance { distance }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_player_death::tests::add_guest;

    #[test]
    #[ignore = "daily"]
    fn a_players_view_distance_is_their_own_and_clamped_to_the_servers() {
        with_game(|game| {
            let player = add_guest(game);
            // add_guest's server view distance is 10.
            assert!(game.player_set_view_distance(player, 4));
            assert!(game.player_set_view_distance(player, 32));
            assert!(!game.player_set_view_distance(player, 1));
            assert!(!game.player_set_view_distance(player, 33));
            let guest = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .as_slice()
                .iter()
                .find(|g| g.uuid == 123)
                .unwrap();
            assert_eq!(guest.view_distance, 10);
        });
    }
}
