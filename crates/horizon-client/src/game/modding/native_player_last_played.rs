//! `player_last_played`: when a player, on or off, last played.
use super::Game;
use horizon_client::lan::playerdata;
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

/// Milliseconds since the Unix epoch at `time`; None for a time before it or past an `i64`.
fn epoch_millis(time: SystemTime) -> Option<i64> {
    i64::try_from(time.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()
}

/// When the player's file under `world` was last saved, None without one.
fn saved_millis(world: &Path, uuid: u128) -> Option<i64> {
    epoch_millis(
        std::fs::metadata(playerdata::path(world, uuid))
            .ok()?
            .modified()
            .ok()?,
    )
}

impl Game<'_> {
    /// The current time for a player who is on, else when their player file was last saved.
    /// None without a file, and with no world open.
    pub(super) fn player_last_played(&self, uuid: u128) -> Option<i64> {
        let online = self
            .lan
            .as_ref()
            .is_some_and(|host| host.hub.online_players().any(|g| g.uuid == uuid));
        if online {
            return epoch_millis(SystemTime::now());
        }
        if self.world_root.as_os_str().is_empty() {
            return None;
        }
        saved_millis(&self.world_root, uuid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The saved file's own modification time, in milliseconds; no file reads None.
    #[test]
    fn a_saved_files_modification_time_is_when_the_player_last_played() {
        let dir = std::env::temp_dir().join(format!("native-last-played-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let uuid = 0x1234_u128;
        assert_eq!(saved_millis(&dir, uuid), None, "no file");
        let file = playerdata::path(&dir, uuid);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"x").unwrap();
        let at = UNIX_EPOCH + Duration::from_millis(1_700_000_123_456);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(at)
            .unwrap();
        assert_eq!(saved_millis(&dir, uuid), Some(1_700_000_123_456));
        assert_eq!(saved_millis(&dir, 0x5678), None, "another player's file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A time before the epoch has no milliseconds to report.
    #[test]
    fn a_time_before_the_epoch_reads_none() {
        assert_eq!(epoch_millis(UNIX_EPOCH - Duration::from_secs(1)), None);
        assert_eq!(epoch_millis(UNIX_EPOCH + Duration::from_millis(5)), Some(5));
    }
}
