# Replay playback

A client mod plays the recordings `/studio record` saves (`.mcpr` files in the game folder's `recordings/`) and
drives their clock, as a replay editor does. Native and wasm mods use the same calls; a wasm mod needs no file
access, because a recording is named by its **id**: its file name without `.mcpr` (`my_trip_1712000000` for
`recordings/my_trip_1712000000.mcpr`). An id is never a path. One with a separator, a drive, `..` or a control
character is `ReplayError::InvalidId`, refused before any file is looked at; one that names no regular file directly
in `recordings/`, a link leading out of it included, is `NotFound`.

| Call | What it does |
|---|---|
| `replay_state()` | The open replay as its clock stands: recording id, `standalone`, `position_ms`, `duration_ms`, `speed`, `paused`, `ended`. None with no replay open. |
| `replay_open(id)` | Reads the recording now (`Unreadable` if it can't be played, changing nothing) and opens it at the end of the frame. A live world is left first, as Disconnect does; a replay playing is closed first. |
| `replay_close()` | Closes the replay at the end of the frame, as Escape does. |
| `replay_pause()`, `replay_resume()` | Stop and start the clock at once. |
| `replay_set_speed(speed)` | Sets the speed at once, `REPLAY_MIN_SPEED` (0.01) to `REPLAY_MAX_SPEED` (20); anything else, NaN included, is `InvalidSpeed`. |
| `replay_seek(ms)` | Moves the clock at the end of the frame, rebuilding the recording's world there. Only forward for a replay opened over a world (`BackwardSeek`). |

Open, close and seek leave a world or apply the recording's packets, so they run at the end of the frame, outside
the calling hook; one request a frame, the last one winning. A close or seek reaches only the replay it was asked
of: if another opens first, the request is dropped. Calls other than `replay_open` are `NotOpen` while no replay is
open, so seek to a start point from `replay_opened`, not right after `replay_open`.

Three hooks follow playback, whoever drives it (a mod, the replays list or `/studio replay`):

- `replay_opened(recording, duration_ms, standalone)`: the replay is open and its first frame set up.
- `replay_advanced(position_ms)`: the clock moved this frame, by playing or by a seek; at most once a frame, never on
  a frame it stood still (paused, at the end).
- `replay_closed(recording)`: the replay is gone; before `replay_opened` of one opened over it.

While a replay plays, the recording's world is not the player's: movement, attack, use, drop, inventory and hotbar
keys and the wheel's hotbar step do nothing, and losing the window's focus doesn't pause, with or without Studio's
free camera. A mod still hears every key (`input_key`, its key mappings), mouse button (`mouse_button_pre`/`_post`)
and the wheel (`mouse_scrolling`), and its screens take input as usual, so a replay editor can run on a mod's own
camera (`camera_pose`). Escape closes the replay and P opens the pause menu.
