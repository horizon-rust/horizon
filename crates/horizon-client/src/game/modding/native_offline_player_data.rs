//! A saved player file (`players/data/<uuid>.dat`) read and written whole for native server mods,
//! for a player who is not online: the vanilla player compound `PlayerDataStorage` keeps.
use super::Game;
use horizon_client::{
    lan::playerdata,
    save::{read_dat, write_dat},
};
use horizon_core::nbt::{Compound, Tag};
use mod_api::OfflinePlayerDataRefusal;

impl Game<'_> {
    /// Whether the player `uuid` is connected here: the host's own player or a guest.
    fn offline_player_online(&self, uuid: u128) -> bool {
        self.player(uuid).is_some()
            || self
                .lan
                .as_ref()
                .is_some_and(|host| host.hub.guests.iter().any(|g| g.uuid == uuid))
    }

    /// The player file of `uuid` as a compound, None for no file, a file that is not a compound,
    /// a client or a remote world.
    pub(super) fn offline_player_data(&self, uuid: u128) -> Option<Compound> {
        if self.remote.is_some() {
            return None;
        }
        match read_dat(&playerdata::path(&self.world_root, uuid))? {
            Tag::Compound(data) => Some(data),
            _ => None,
        }
    }

    /// Replaces the player file of `uuid` with `data`, refusing while the player is connected.
    pub(super) fn set_offline_player_data(
        &mut self,
        uuid: u128,
        data: Compound,
    ) -> Result<(), OfflinePlayerDataRefusal> {
        if self.remote.is_some() || self.lan.is_none() {
            return Err(OfflinePlayerDataRefusal::NoServer);
        }
        if self.offline_player_online(uuid) {
            return Err(OfflinePlayerDataRefusal::Online);
        }
        playerdata::directory(&self.world_root);
        write_dat(
            &playerdata::path(&self.world_root, uuid),
            Tag::Compound(data),
        )
        .map_err(|_| OfflinePlayerDataRefusal::Write)
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;

    /// A file that is not there reads None; a refused write leaves no file behind, and a file
    /// written as the save does reads back whole, with its values as they were.
    #[test]
    #[ignore = "daily"]
    fn an_offline_player_file_is_read_whole_and_refused_without_a_server() {
        with_game(|game| {
            let dir = tempfile::tempdir().unwrap();
            game.world_root = dir.path().into();
            let mut data = Compound::new();
            data.insert("Score".into(), Tag::Int(7));
            assert_eq!(game.offline_player_data(42), None, "no file yet");
            assert_eq!(
                game.set_offline_player_data(42, data.clone()),
                Err(OfflinePlayerDataRefusal::NoServer),
                "no server open",
            );
            assert!(!playerdata::path(dir.path(), 42).exists());
            playerdata::directory(dir.path());
            write_dat(&playerdata::path(dir.path(), 42), Tag::Compound(data)).unwrap();
            let read = game.offline_player_data(42).unwrap();
            assert_eq!(read.get(&"Score".into()), Some(&Tag::Int(7)));
            assert_eq!(game.offline_player_data(43), None, "another UUID");
        });
    }
}
