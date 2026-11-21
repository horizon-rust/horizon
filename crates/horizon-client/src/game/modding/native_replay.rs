//! A client mod reading and driving the platform's replay playback (#19247): the state read,
//! open, close, pause, resume, speed and seek. Opening reads the recording off the game's thread
//! and installs it at a frame's end; close and seek leave a world or apply the recording's
//! packets, so they too run at the end of the frame, outside every hook, as `disconnect` does.
//! Pause, resume and speed only set the clock and apply at once.
use super::{ClientQuit, Game, PendingWorldEntry};
use horizon_client::studio::ReplayDriver;
use mod_api::{REPLAY_MAX_SPEED, REPLAY_MIN_SPEED, ReplayError, ReplayState, WorldEpoch};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

/// The longest recording id, in bytes: a file name most file systems still take with `.mcpr`.
const MAX_RECORDING_ID: usize = 250;

/// Names Windows gives to devices in every folder, whatever follows them after a dot.
const DEVICE_NAMES: [&str; 30] = [
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM1", "COM2", "COM3", "COM4", "COM5",
    "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5",
    "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// A mod's close or seek, carried out at the end of the frame. One a frame: a later request
/// replaces an earlier one.
pub(super) enum ReplayRequest {
    /// Close the replay of this generation.
    Close { generation: u64 },
    /// Seek the replay of this generation.
    Seek { generation: u64, ms: u32 },
}

/// The sessions a request was made in: the world's lifecycle and the replay playing. A request
/// that meets other ones when it is carried out is dropped.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ReplayScope {
    epoch: Option<WorldEpoch>,
    generation: u64,
}

/// A mod's accepted `replay_open`: its recording read and indexed on a thread of its own.
pub(super) struct ReplayOpening {
    recording: String,
    scope: ReplayScope,
    prepared: Receiver<io::Result<ReplayDriver>>,
}

/// Whether `id` can name a recording, judged as text alone: one plain file name, 1 to 250 bytes,
/// without a separator, drive, stream, `..`, NUL or control character, not ending in a dot or a
/// space (Windows drops those, so two ids would name one file), and not a Windows device name.
fn valid_id(id: &str) -> bool {
    let mut components = Path::new(id).components();
    let base = id
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ');
    !id.is_empty()
        && id.len() <= MAX_RECORDING_ID
        && !id.contains("..")
        && !id.ends_with(['.', ' '])
        && !id
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
        && !DEVICE_NAMES
            .iter()
            .any(|device| device.eq_ignore_ascii_case(base))
        && matches!(
            (components.next(), components.next()),
            (Some(Component::Normal(_)), None)
        )
}

/// The recording file a mod names by `id` in `dir`: `<dir>/<id>.mcpr`. The id is checked as text
/// first (`valid_id`), touching no file, so it can't be used to probe the disk. Then `dir` must be
/// a real folder, not a link to one, and the file a regular file, not a link, whose real path is
/// directly in `dir`'s. A missing file, a link, a reparse point, a redirected folder or an escape
/// all answer `NotFound`, the same as one another.
pub(super) fn recording_path(dir: &Path, id: &str) -> Result<PathBuf, ReplayError> {
    if !valid_id(id) {
        return Err(ReplayError::InvalidId);
    }
    let folder = std::fs::symlink_metadata(dir).map_err(|_| ReplayError::NotFound)?;
    if !folder.is_dir() || reparse_point(&folder) {
        return Err(ReplayError::NotFound);
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

/// The recording `id` names in `dir`, opened once (`recording_path`'s checks), without following
/// a link the file may have become since, and checked again once open. The file starts as a zip
/// archive does, or it is `Unreadable`; it comes back rewound. Nothing later opens it by name.
pub(super) fn open_recording(dir: &Path, id: &str) -> Result<(PathBuf, File), ReplayError> {
    let path = recording_path(dir, id)?;
    let mut file = open_no_follow(&path).map_err(|_| ReplayError::NotFound)?;
    let metadata = file.metadata().map_err(|_| ReplayError::NotFound)?;
    if !metadata.is_file() || reparse_point(&metadata) || recording_path(dir, id).is_err() {
        return Err(ReplayError::NotFound);
    }
    let mut magic = [0; 4];
    if file.read_exact(&mut magic).is_err()
        || magic != *b"PK\x03\x04"
        || file.seek(SeekFrom::Start(0)).is_err()
    {
        return Err(ReplayError::Unreadable);
    }
    Ok((path, file))
}

/// Opens `path` for reading; a link at its last component is opened as itself, not followed.
#[cfg(windows)]
fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
}

#[cfg(not(any(windows, unix)))]
fn open_no_follow(path: &Path) -> io::Result<File> {
    File::open(path)
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

impl Game<'_> {
    /// The recordings folder replays are opened from, the one `/studio record` writes to.
    fn recordings_dir(&self) -> PathBuf {
        self.replay
            .recordings_dir
            .clone()
            .unwrap_or_else(horizon_client::studio::recorder::default_recordings_dir)
    }

    /// The id of the recording at `path`, under the one policy `replay_open` resolves by: its
    /// file name without `.mcpr` when that id names this very file in the recordings folder,
    /// else empty (a recording opened from elsewhere, or whose name no id can hold).
    pub(super) fn recording_id(&self, path: &str) -> String {
        let path = Path::new(path);
        let Some(id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".mcpr"))
        else {
            return String::new();
        };
        let same = recording_path(&self.recordings_dir(), id)
            .ok()
            .and_then(|by_id| by_id.canonicalize().ok())
            .is_some_and(|by_id| path.canonicalize().is_ok_and(|real| real == by_id));
        if same { id.to_owned() } else { String::new() }
    }

    /// A replay can be opened or driven by a mod: a client that is neither a splitscreen guest
    /// nor closing.
    fn replay_available(&self) -> bool {
        self.options.is_client_start()
            && self.client_quit.get() == ClientQuit::Running
            && self.pending_save.is_none()
    }

    /// A session a replay must not displace: a world entry waiting, a single-player world
    /// starting, or a server joined or being joined (a replay's own stand-in excepted).
    fn replay_world_busy(&self) -> bool {
        self.pending_entry.is_some()
            || self.integrated_starting.is_some()
            || (!self.replay.playing() && self.integrated.is_none() && self.remote.is_some())
    }

    fn replay_scope(&self) -> ReplayScope {
        ReplayScope {
            epoch: self.native_lifecycle.epoch(),
            generation: self.replay.generation,
        }
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

    /// Accepts a mod's request to open a recording by id: checks it, opens its file once and
    /// reads it on a thread of its own; the frame's end that finds it ready installs it.
    pub(super) fn replay_open(&mut self, recording: &str) -> Result<(), ReplayError> {
        if !self.replay_available() {
            return Err(ReplayError::Unavailable);
        }
        if self.replay_world_busy() {
            return Err(ReplayError::WorldBusy);
        }
        let (path, file) = open_recording(&self.recordings_dir(), recording)?;
        let (send, prepared) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("replay-prepare".into())
            .spawn(move || {
                let _ = send.send(ReplayDriver::from_file(file, &path));
            })
            .map_err(|_| ReplayError::Unavailable)?;
        // An earlier request still being read is dropped with its thread's answer.
        self.replay.opening = Some(ReplayOpening {
            recording: recording.to_owned(),
            scope: self.replay_scope(),
            prepared,
        });
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

    /// Sets the playback speed; a speed out of range, NaN included, changes nothing.
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
        // A replay on a session it doesn't own can't rebuild an earlier world.
        if !self.replay.standalone && ms.min(driver.duration_ms()) < driver.position_ms() {
            return Err(ReplayError::BackwardSeek);
        }
        self.replay.request = Some(ReplayRequest::Seek {
            generation: self.replay.generation,
            ms,
        });
        Ok(())
    }

    /// The end of the frame: a mod's close or seek runs now, outside every hook, then an open
    /// whose recording is ready. A close or seek of a replay closed or replaced since is dropped.
    pub(in crate::game) fn native_replay_requests(&mut self) {
        if let Some(request) = self.replay.request.take()
            && self.replay_available()
        {
            let current = |game: &Self, generation| {
                game.replay.driver.is_some() && game.replay.generation == generation
            };
            match request {
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
        }
        self.poll_replay_opening();
        // A request a hook made while these ran (a seek from `replay_opened`) waits for the
        // next frame's end; its generation keeps it from reaching another replay.
    }

    /// An accepted open whose recording is read: installed if the sessions it was asked in are
    /// still the ones here, else told as failed.
    fn poll_replay_opening(&mut self) {
        let Some(opening) = &self.replay.opening else {
            return;
        };
        let prepared = match opening.prepared.try_recv() {
            Ok(prepared) => prepared,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(io::Error::other("the reading thread ended")),
        };
        let opening = self.replay.opening.take().expect("checked above");
        let refused = if !self.replay_available() {
            Some(ReplayError::Unavailable)
        } else if self.replay_world_busy() || self.replay_scope() != opening.scope {
            Some(ReplayError::WorldBusy)
        } else {
            None
        };
        if let Some(error) = refused {
            self.native_replay_open_failed(&opening.recording, error);
            return;
        }
        match prepared {
            Ok(driver) => self.open_prepared_replay(driver, opening.recording),
            Err(error) => {
                log::warn!("Replay {:?} could not be read: {error}", opening.recording);
                self.native_replay_open_failed(&opening.recording, ReplayError::Unreadable);
            }
        }
    }

    /// Drops a mod's read replay waiting as the world entry, if that is what waits, telling the
    /// mod it opened nothing: another entry takes its place (`WorldBusy`) or the person cancels
    /// the wait (`Unavailable`). Every accepted `replay_open` ends in opened or failed.
    pub(in crate::game) fn displace_pending_replay(&mut self, error: ReplayError) {
        if let Some(PendingWorldEntry::PreparedReplay { recording, .. }) = self
            .pending_entry
            .take_if(|entry| matches!(entry, PendingWorldEntry::PreparedReplay { .. }))
        {
            self.native_replay_open_failed(&recording, error);
        }
    }

    /// Opens a recording already read for a mod. From a single-player world, the world is
    /// saved and left first and the replay opens on the title (`PendingWorldEntry`), carrying
    /// the read recording, never its name.
    pub(super) fn open_prepared_replay(&mut self, driver: ReplayDriver, recording: String) {
        if let Err(error) = self.save_studio_camera_track() {
            log::warn!("Studio: could not save camera path: {error}");
        }
        let entry = |driver| PendingWorldEntry::PreparedReplay {
            driver: Box::new(driver),
            recording: recording.clone(),
        };
        if self.replay.driver.is_none() && (self.remote.is_some() || self.integrated.is_some()) {
            self.pending_entry = Some(entry(driver));
            if let Err(error) = self.close_world() {
                log::error!("Replay {recording:?}: could not leave the world: {error}");
                self.pending_entry = None;
                self.native_replay_open_failed(&recording, ReplayError::Unavailable);
            }
            return;
        }
        // Replay setup builds the client registries, which need the world tables.
        if self.remote.is_none() && self.res.rest.get().is_none() {
            let driver = match self.try_defer_world_entry(entry(driver)) {
                Ok(None) => return,
                Ok(Some(PendingWorldEntry::PreparedReplay { driver, .. })) => *driver,
                Ok(Some(_)) => unreachable!("the entry handed back is the one given"),
                Err(error) => {
                    log::error!("Replay {recording:?}: {error}");
                    self.native_replay_open_failed(&recording, ReplayError::Unavailable);
                    return;
                }
            };
            if let Err(error) = self.join_world_rest() {
                log::error!("Replay {recording:?}: {error}");
                self.native_replay_open_failed(&recording, ReplayError::Unavailable);
                return;
            }
            return self.start_prepared_replay(driver, recording);
        }
        self.start_prepared_replay(driver, recording);
    }

    fn start_prepared_replay(&mut self, driver: ReplayDriver, recording: String) {
        let path = driver.path().display().to_string();
        match self.start_studio_replay(driver, recording.clone(), &path) {
            Ok(answer) if answer["ok"].as_bool() == Some(true) => {}
            Ok(answer) => {
                log::warn!(
                    "Replay {recording:?} could not be opened: {}",
                    answer["error"].as_str().unwrap_or_default()
                );
                self.native_replay_open_failed(&recording, ReplayError::Unavailable);
            }
            Err(error) => {
                log::error!("Replay {recording:?} could not be opened: {error}");
                self.native_replay_open_failed(&recording, ReplayError::Unavailable);
            }
        }
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
