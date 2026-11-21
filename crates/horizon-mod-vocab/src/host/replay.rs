//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `replay` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ReplayClientHost {
    /// Side: client.
    /// The replay this window plays, if any, as its clock stands now; none while no replay
    /// is open, including while one asked for by `replay_open` is still opening.
    #[access(ClientSetup)]
    fn replay_state(&self) -> Option<ReplayState>;
    /// Side: client.
    /// Opens the recording `recording` for playback, at the end of this frame, not inside the
    /// calling hook; `replay_opened` tells when it is open. The id is the recording's file name
    /// in the recordings folder without `.mcpr`, never a path: an id with a path separator, a
    /// drive, `..` or a control character is `InvalidId`, and one that names no regular file
    /// inside the folder (a link leading out of it included) is `NotFound`. The recording is
    /// read before anything changes, so a damaged one is `Unreadable` and leaves the world or
    /// the title screen as it is. A replay is never played over a live world: the world is left
    /// as Disconnect does (a single-player world is saved first), and the replay opens on the
    /// title once it is. A replay already playing is closed first (`replay_closed`). A later
    /// `replay_open`, `replay_close` or `replay_seek` in the same frame replaces this request.
    /// `Unavailable` for a splitscreen guest, a server, or while the game is closing.
    #[access(ClientSetup)]
    fn replay_open(&mut self, recording: &str) -> Result<(), ReplayError>;
    /// Side: client.
    /// Closes the replay at the end of this frame, as Escape does: a replay opened from the
    /// title screen goes back there. `replay_closed` tells when it is closed. A later
    /// `replay_open`, `replay_close` or `replay_seek` in the same frame replaces this request.
    /// `NotOpen` while no replay is open.
    #[access(ClientSetup)]
    fn replay_close(&mut self) -> Result<(), ReplayError>;
    /// Side: client.
    /// Pauses the replay's clock now; pausing a paused replay changes nothing. `NotOpen` while
    /// no replay is open.
    #[access(ClientSetup)]
    fn replay_pause(&mut self) -> Result<(), ReplayError>;
    /// Side: client.
    /// Starts the replay's clock again now. `NotOpen` while no replay is open.
    #[access(ClientSetup)]
    fn replay_resume(&mut self) -> Result<(), ReplayError>;
    /// Side: client.
    /// Sets the playback speed now: 1 is real time, 0.5 half and 2 twice as fast.
    /// `InvalidSpeed`, changing nothing, for a speed that is not finite or is outside
    /// [`REPLAY_MIN_SPEED`]..=[`REPLAY_MAX_SPEED`]; `NotOpen` while no replay is open.
    #[access(ClientSetup)]
    fn replay_set_speed(&mut self, speed: f64) -> Result<(), ReplayError>;
    /// Side: client.
    /// Moves the replay's clock to `ms` from the recording's start (at most its length) at
    /// the end of this frame, rebuilding the world the recording shows there; the next
    /// `replay_advanced` reports the new position. A standalone replay seeks either way; one
    /// opened over a world only forward (`BackwardSeek`). A later `replay_open`,
    /// `replay_close` or `replay_seek` in the same frame replaces this request, and a request
    /// for a replay closed or replaced since is dropped. `NotOpen` while no replay is open.
    #[access(ClientSetup)]
    fn replay_seek(&mut self, ms: u32) -> Result<(), ReplayError>;
}
