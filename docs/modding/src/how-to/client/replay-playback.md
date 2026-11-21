# Replay playback

A client mod plays the recordings `/studio record` saves (`.mcpr` files in the game folder's `recordings/`) and
drives their clock, as a replay editor does. Native and wasm mods use the same calls; a wasm mod needs no file
access, because a recording is named by its **id**: its file name without `.mcpr` (`my_trip_1712000000` for
`recordings/my_trip_1712000000.mcpr`). An id is never a path. One that is empty, longer than 250 bytes, holds a
separator, a drive, `..` or a control character, ends in a dot or a space, or is a Windows device name (`CON`,
`NUL`, `COM1`...) is `ReplayError::InvalidId`, refused before any file is looked at. One that names no regular file
directly in `recordings/` is `NotFound`, and so is a link, or a `recordings/` folder that is itself a link. A replay
opened from a file elsewhere (`/studio replay open <path>`) has the empty id.

| Call | Access | What it does |
|---|---|---|
| `replay_state()` | client setup | The open replay as its clock stands: id, `standalone`, `position_ms` (0 to `duration_ms`), `duration_ms`, `speed`, `paused`, `ended`. None with no replay open, and while one is still opening. |
| `replay_open(id)` | world | Accepts a request: opens the file once and checks it is a zip archive (`Unreadable` if not). The recording is read off the game's thread, from that open file, and opened at a frame's end. |
| `replay_close()` | world | Closes the replay at the end of the frame, as Escape does. |
| `replay_pause()`, `replay_resume()` | world | Stop and start the clock at once. |
| `replay_set_speed(speed)` | world | Sets the speed at once, `REPLAY_MIN_SPEED` (0.01) to `REPLAY_MAX_SPEED` (20); anything else, NaN included, is `InvalidSpeed`. |
| `replay_seek(ms)` | world | Moves the clock at the end of the frame, rebuilding the recording's world there. Only forward for a replay that doesn't own its session (`BackwardSeek`). |

`Ok` from `replay_open` means the request is accepted. Exactly one of `replay_opened` or
`replay_open_failed(recording, error)` follows, unless a later `replay_open` replaces the request before its
recording is read. A replay never displaces another session: `replay_open` is `WorldBusy` while a world is being
joined or a multiplayer server is connected (leave it with `disconnect` first), and a request whose session changes
before the recording is ready (a world joined, left or opened, or another world entry taking its place) fails with
`WorldBusy`. From a single-player world, the world is saved and left as Save and Quit does, and the replay opens on
the title; if the person cancels that wait with Escape, the request fails with `Unavailable` and the world stays
left. A recording that turns out damaged once read fails with `Unreadable`.

Close and seek run at the end of the frame, outside the calling hook; one of them a frame, the last one winning, and
a request for a replay that has closed or been replaced since is dropped. Calls other than `replay_open` are
`NotOpen` while no replay is open, so seek to a start point from `replay_opened`, not right after `replay_open`.

The clock stays within the recording: `position_ms` never passes `duration_ms` (the recorded length, or the last
packet's time when that is later), and `ended` is true exactly when the clock is at `duration_ms`, whether it played
there or was sought there, paused or not.

Hooks follow playback, whoever drives it (a mod, the replays list or `/studio replay`):

- `replay_opened(recording, duration_ms, standalone)`: the replay is open and its first frame set up.
- `replay_open_failed(recording, error)`: an accepted `replay_open` opened nothing.
- `replay_advanced(position_ms)`: a once-a-frame sample of the clock, on a frame it moved. After a seek, the next one
  is exactly the seek's position, on the frame that first shows it; the clock runs on from the frame after.
- `replay_closed(recording)`: the replay is gone; before `replay_opened` of one opened over it.

While a replay plays, the recording's world is not the player's: movement, attack, use, drop, inventory and hotbar
keys do nothing, the wheel and a controller's bumpers don't step the hotbar or the spectator menu, and losing the
window's focus doesn't pause, with or without Studio's free camera. A mod still hears every key (`input_key`, its
key mappings), mouse button (`mouse_button_pre`/`_post`), the wheel and the bumpers (`mouse_scrolling`), and its
screens and prompts take input as usual, so a replay editor can run on a mod's own camera (`camera_pose`). Escape
closes the replay and P opens the pause menu.
