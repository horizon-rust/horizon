//! Replay playback as a client mod reads and drives it (`replay_state`, `replay_open`...).

/// The replay this window plays, as `replay_state` reads it this frame.
/// Mods receive this snapshot from the game; they do not construct it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ReplayState {
    /// The recording's id: its file name in the recordings folder without `.mcpr`, as
    /// `replay_open` takes it.
    pub recording: String,
    /// The replay was opened from the title screen and owns its world: closing it returns
    /// there, and it can seek backwards.
    pub standalone: bool,
    /// Where the replay's clock is, in ms from the recording's start.
    pub position_ms: u32,
    /// The recording's length in ms.
    pub duration_ms: u32,
    /// The playback speed: 1 is real time, between [`REPLAY_MIN_SPEED`] and [`REPLAY_MAX_SPEED`].
    pub speed: f64,
    /// The clock is paused.
    pub paused: bool,
    /// The clock reached the recording's end; a seek starts it again.
    pub ended: bool,
}

/// The slowest playback speed `replay_set_speed` takes.
pub const REPLAY_MIN_SPEED: f64 = 0.01;
/// The fastest playback speed `replay_set_speed` takes.
pub const REPLAY_MAX_SPEED: f64 = 20.0;

/// Why a replay call changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ReplayError {
    /// Not a recording id: empty, longer than 255 bytes, or holding a path separator, a
    /// drive or `..`, a NUL or another control character.
    InvalidId,
    /// No recording has this id: no such regular file in the recordings folder. A link
    /// that leads out of the folder counts as none.
    NotFound,
    /// The recording is there but can't be played (damaged, or of another version).
    Unreadable,
    /// No replay is open.
    NotOpen,
    /// The speed is not a finite number between [`REPLAY_MIN_SPEED`] and [`REPLAY_MAX_SPEED`].
    InvalidSpeed,
    /// A replay opened over a world can't seek back before where it is.
    BackwardSeek,
    /// This client can't play a replay now: a splitscreen guest or a server, or the game
    /// is closing.
    Unavailable,
}
