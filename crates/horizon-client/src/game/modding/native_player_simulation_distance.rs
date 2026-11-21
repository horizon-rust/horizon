//! `Player.setSimulationDistance` for native server mods, through a `PlayerRef`:
//! the player's client gets `ClientboundSetSimulationDistancePacket` for the
//! value, 2 to 32. Only that client is told; the server keeps its own simulation
//! distance and the other players keep theirs.
use super::Game;
use horizon_protocol::packets::play::{self, Clientbound as C};
use mod_api::PlayerRef;

impl Game<'_> {
    /// `Player.setSimulationDistance`. False when the value is outside 2 to 32
    /// or the player is not on here.
    pub(super) fn player_set_simulation_distance(
        &mut self,
        player: PlayerRef,
        chunks: i32,
    ) -> bool {
        if !(2..=32).contains(&chunks) {
            return false;
        }
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        self.native_send(
            uuid,
            C::SetSimulationDistance(play::Distance { distance: chunks }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_player_death::tests::add_guest;
    use horizon_protocol::packets::play::Clientbound as C;

    #[test]
    #[ignore = "daily"]
    fn a_simulation_distance_is_sent_to_that_player_only() {
        with_game(|game| {
            let player = add_guest(game);
            game.as_guest(123, |game| {
                assert!(game.player_set_simulation_distance(player, 6));
                assert!(!game.player_set_simulation_distance(player, 1));
                assert!(!game.player_set_simulation_distance(player, 40));
            })
            .unwrap();
            assert!(matches!(
                game.guest_packets.as_slice(),
                [C::SetSimulationDistance(d)] if d.distance == 6
            ));
        });
    }
}
