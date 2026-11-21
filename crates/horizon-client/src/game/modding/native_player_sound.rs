//! `Player.playSound(Location, ...)` for native server mods, through a `PlayerRef`
//! (docs/modding/src/how-to/native-mods.md): one sound for one player at a
//! position, drawn from the server's sound seeds like a level sound and sent
//! only to that player's client. The checks are those of `level_play_sound`.
use super::Game;
use super::native_hooks::mod_sound_ok;
use mod_api::{PlayerRef, SoundSource};

impl Game<'_> {
    /// `Player.playSound`: plays `sound` at `at` for that player only. False
    /// for a sound the checks refuse, and for a player who is not on here.
    pub(super) fn player_play_sound_at(
        &mut self,
        player: PlayerRef,
        sound: &str,
        source: SoundSource,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
    ) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        if !mod_sound_ok(sound, at, source.index(), volume, pitch) {
            return false;
        }
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        // `Level.playSound` draws the seed at the call, as the level sound does.
        let seed = host.hub.next_sound_seed();
        let packet = host
            .hub
            .sound_packet(sound, at, volume, pitch, source.index() as i32, seed);
        self.native_send(uuid, packet)
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_player_death::tests::add_guest;
    use horizon_protocol::packets::play::Clientbound as C;
    use mod_api::SoundSource;

    #[test]
    #[ignore = "daily"]
    fn a_sound_at_a_position_goes_to_that_player_at_eight_units_per_block() {
        with_game(|game| {
            let player = add_guest(game);
            game.as_guest(123, |game| {
                assert!(game.player_play_sound_at(
                    player,
                    "minecraft:entity.experience_orb.pickup",
                    SoundSource::Players,
                    [1.5, 2.0, -3.25],
                    1.0,
                    1.0,
                ));
                assert!(!game.player_play_sound_at(
                    player,
                    "minecraft:entity.experience_orb.pickup",
                    SoundSource::Players,
                    [1.5, 2.0, -3.25],
                    -1.0,
                    1.0,
                ));
            })
            .unwrap();
            assert!(matches!(
                game.guest_packets.as_slice(),
                [C::Sound(p)] if p.x == 12 && p.y == 16 && p.z == -26 && p.volume == 1.0
            ));
        });
    }
}
