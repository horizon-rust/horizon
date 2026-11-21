//! A client mod reading and driving the platform's replay playback (#19247): the state read,
//! open, close, pause, resume, speed and seek. Open, close and seek leave a world or apply the
//! recording's packets, so they run at the end of the frame, outside every hook, as `disconnect`
//! does; pause, resume and speed only set the clock and apply at once.
use super::{ClientQuit, Game};
use mod_api::{REPLAY_MAX_SPEED, REPLAY_MIN_SPEED, ReplayError, ReplayState};
use std::path::{Component, Path, PathBuf};

/// The longest recording id, in bytes: a file name most file systems still take with `.mcpr`.
const MAX_RECORDING_ID: usize = 250;

/// A mod's open, close or seek, carried out at the end of the frame. One a frame: a later
/// request replaces an earlier one.
pub(super) enum ReplayRequest {
    /// Open the recording with this id, resolved again when the request runs.
    Open(String),
    /// Close the replay of this generation.
    Close { generation: u64 },
    /// Seek the replay of this generation.
    Seek { generation: u64, ms: u32 },
}

/// The recordings folder replays are opened from, the one `/studio record` writes to.
fn recordings_dir() -> PathBuf {
    horizon_client::studio::recorder::default_recordings_dir()
}

/// The recording file a mod names by `id` in `dir`: `<dir>/<id>.mcpr`. The id is checked as text
/// first, touching no file, so it can't be used to probe the disk: one path component, no
/// separator, drive, `..`, NUL or control character. Then the file must be a regular file whose
/// real path is directly in `dir`'s; a missing file, a link, a reparse point or an escape all
/// answer `NotFound`, the same as one another.
pub(super) fn recording_path(dir: &Path, id: &str) -> Result<PathBuf, ReplayError> {
    let lexical = !id.is_empty()
        && id.len() <= MAX_RECORDING_ID
        && !id.contains("..")
        && !id
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'));
    let mut components = Path::new(id).components();
    if !lexical
        || !matches!(
            (components.next(), components.next()),
            (Some(Component::Normal(_)), None)
        )
    {
        return Err(ReplayError::InvalidId);
    }
    let file = dir.join(format!("{id}.mcpr"));
    let metadata = std::fs::symlink_metadata(&file).map_err(|_| ReplayError::NotFound)?;
    if !metadata.is_file() || reparse_point(&metadata) {
        return Err(ReplayError::NotFound);
    }
    let real_dir = dir.canonicalize().map_err(|_| ReplayError::NotFound)?;
    let real_file = file.canonicalize().map_err(|_| ReplayError::NotFound)?;
    if real_file.parent() != Some(real_dir.as_path()) {
        return Err(ReplayError::NotFound);
    }
    Ok(file)
}

/// A Windows reparse point (a link, a junction, a cloud placeholder) that `is_file` may not show.
#[cfg(windows)]
fn reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn reparse_point(_: &std::fs::Metadata) -> bool {
    false
}

/// The recording id of the archive at `path`: its file name without `.mcpr`.
pub(super) fn recording_id(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl Game<'_> {
    /// A replay can be opened or driven by a mod: a client that is neither a splitscreen guest
    /// nor closing.
    fn replay_available(&self) -> bool {
        self.options.is_client_start()
            && self.client_quit.get() == ClientQuit::Running
            && self.pending_save.is_none()
    }

    /// The replay this window plays, for a client mod.
    pub(super) fn replay_state(&self) -> Option<ReplayState> {
        let driver = self.replay.driver.as_ref()?;
        Some(mod_api::__private::built::replay_state(
            self.replay.recording.clone(),
            self.replay.standalone,
            driver.position_ms(),
            driver.duration_ms(),
            driver.speed(),
            driver.is_paused(),
            driver.is_finished(),
        ))
    }

    /// Asks to open a recording by id at the end of this frame, once it is known to be one.
    pub(super) fn replay_open(&mut self, recording: &str) -> Result<(), ReplayError> {
        if !self.replay_available() {
            return Err(ReplayError::Unavailable);
        }
        let path = recording_path(&recordings_dir(), recording)?;
        // Read before anything changes, as `open_studio_replay` does over a live world.
        horizon_client::studio::ReplayDriver::open(&path).map_err(|_| ReplayError::Unreadable)?;
        self.replay.request = Some(ReplayRequest::Open(recording.to_owned()));
        Ok(())
    }

    /// Asks to close the replay at the end of this frame.
    pub(super) fn replay_close(&mut self) -> Result<(), ReplayError> {
        if self.replay.driver.is_none() {
            return Err(ReplayError::NotOpen);
        }
        self.replay.request = Some(ReplayRequest::Close {
            generation: self.replay.generation,
        });
        Ok(())
    }

    /// Pauses the replay's clock.
    pub(super) fn replay_pause(&mut self) -> Result<(), ReplayError> {
        let driver = self.replay.driver.as_mut().ok_or(ReplayError::NotOpen)?;
        driver.set_paused(true);
        Ok(())
    }

    /// Starts the replay's clock again.
    pub(super) fn replay_resume(&mut self) -> Result<(), ReplayError> {
        let driver = self.replay.driver.as_mut().ok_or(ReplayError::NotOpen)?;
        driver.set_paused(false);
        Ok(())
    }

    /// Sets the playback speed; a speed out of range changes nothing (`f64::clamp` would let NaN
    /// through to the clock).
    pub(super) fn replay_set_speed(&mut self, speed: f64) -> Result<(), ReplayError> {
        if !(REPLAY_MIN_SPEED..=REPLAY_MAX_SPEED).contains(&speed) {
            return Err(ReplayError::InvalidSpeed);
        }
        let driver = self.replay.driver.as_mut().ok_or(ReplayError::NotOpen)?;
        driver.set_speed(speed);
        Ok(())
    }

    /// Asks to seek the replay at the end of this frame.
    pub(super) fn replay_seek(&mut self, ms: u32) -> Result<(), ReplayError> {
        let driver = self.replay.driver.as_ref().ok_or(ReplayError::NotOpen)?;
        // An in-world replay keeps the world it was opened over and can't rebuild an earlier one.
        if !self.replay.standalone && ms.min(driver.duration_ms()) < driver.position_ms() {
            return Err(ReplayError::BackwardSeek);
        }
        self.replay.request = Some(ReplayRequest::Seek {
            generation: self.replay.generation,
            ms,
        });
        Ok(())
    }

    /// The end of the frame: a mod's open, close or seek runs now, outside every hook. A close
    /// or seek of a replay closed or replaced since is dropped, and an open whose recording
    /// went away since the call opens nothing.
    pub(in crate::game) fn native_replay_requests(&mut self) {
        let Some(request) = self.replay.request.take() else {
            return;
        };
        if !self.replay_available() {
            return;
        }
        let current = |game: &Self, generation| {
            game.replay.driver.is_some() && game.replay.generation == generation
        };
        match request {
            ReplayRequest::Open(recording) => {
                let path = match recording_path(&recordings_dir(), &recording) {
                    Ok(path) => path,
                    Err(error) => {
                        log::warn!("Replay {recording:?} could not be opened: {error:?}");
                        return;
                    }
                };
                match self.open_studio_replay(&path.to_string_lossy()) {
                    Ok(answer) if answer["ok"].as_bool() == Some(true) => {}
                    Ok(answer) => log::warn!(
                        "Replay {recording:?} could not be opened: {}",
                        answer["error"].as_str().unwrap_or_default()
                    ),
                    Err(error) => log::error!("Replay {recording:?} could not be opened: {error}"),
                }
            }
            ReplayRequest::Close { generation } => {
                if current(self, generation) {
                    self.close_studio_replay();
                }
            }
            ReplayRequest::Seek { generation, ms } => {
                if current(self, generation)
                    && let Err(error) = self.seek_studio_replay(ms)
                {
                    log::warn!("Replay seek to {ms} ms failed: {error}");
                }
            }
        }
        // A request a hook made while this one ran (a seek from `replay_opened`) waits for the
        // next frame's end; its generation keeps it from reaching another replay.
    }

    /// Tells mods the replay's clock moved: at most once a frame, only when its position differs
    /// from the last one told, and not before `replay_opened`.
    pub(super) fn announce_replay_advance(&mut self) {
        let Some(position) = self.replay.driver.as_ref().map(|r| r.position_ms()) else {
            return;
        };
        if self.replay.reported_ms.is_none_or(|told| told == position) {
            return;
        }
        self.replay.reported_ms = Some(position);
        self.native_replay_advanced(position);
    }
}
