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
| `replay_open(id)` | world | Accepts a request and returns its id: opens the file once, through the recordings folder's handle, and checks it is a zip archive (`Unreadable` if not). The recording is read off the game's thread, from that open file, and opened at a frame's end. |
| `replay_close()` | world | Closes the replay at the end of the frame, as Escape does. |
| `replay_pause()`, `replay_resume()` | world | Stop and start the clock at once. |
| `replay_set_speed(speed)` | world | Sets the speed at once, `REPLAY_MIN_SPEED` (0.01) to `REPLAY_MAX_SPEED` (20); anything else, NaN included, is `InvalidSpeed`. |
| `replay_seek(ms)` | world | Moves the clock at the end of the frame, rebuilding the recording's world there. Only forward for a replay that doesn't own its session (`BackwardSeek`). |

`Ok(request)` from `replay_open` means the request is accepted; `request` is a non-zero id never given twice while
the game runs. Exactly one of `replay_opened` or `replay_open_failed` follows with that id, whatever happens; only
the game closing first promises neither. One thread reads recordings at a time: a later `replay_open` replaces a
request not yet ready, which fails with `Superseded`, and the replaced reading is cancelled.

A replay never displaces a session it would end: `replay_open` is `WorldBusy` while a world is being joined, a
multiplayer server is connected, or the single-player world is open to LAN or has splitscreen players (leave it with
`disconnect` first). A request whose session changes before the recording is ready (a world joined, left or opened,
the replay playing closed or replaced, or another world entry taking its place) fails with `WorldBusy`; a dimension
change inside the replay playing is no such change. From a plain single-player world, the world is saved and left as
Save and Quit does, and the replay opens on the title; if the person cancels that wait with Escape, the request fails
with `Unavailable` and the world stays left. A recording that turns out damaged once read fails with `Unreadable`.

Close and seek run at the end of the frame, outside the calling hook; one of them a frame, the last one winning, and
a request for a replay that has closed or been replaced since is dropped. Calls other than `replay_open` are
`NotOpen` while no replay is open, so seek to a start point from `replay_opened`, not right after `replay_open`.

The clock stays within the recording: `position_ms` never passes `duration_ms` (the recorded length, or the last
packet's time when that is later), and `ended` is true exactly when the clock is at `duration_ms`, whether it played
there or was sought there, paused or not.

Hooks follow playback, whoever drives it (a mod, the replays list or `/studio replay`):

- `replay_opened(recording, duration_ms, standalone, request)`: the replay is open and its first frame set up;
  `request` is the id `replay_open` returned, or 0 for a replay opened another way.
- `replay_open_failed(request, recording, error)`: an accepted `replay_open` opened nothing.
- `replay_advanced(position_ms)`: a once-a-frame sample of the clock, on a frame it moved. After a seek, the next one
  is exactly the seek's position, on the frame that first shows it; the clock runs on from the frame after.
- `replay_closed(recording)`: the replay is gone; before `replay_opened` of one opened over it.

While a replay plays, the recording's world is not the player's: movement, attack, use, drop, inventory and hotbar
keys do nothing, the wheel and a controller's bumpers don't step the hotbar or the spectator menu, and losing the
window's focus doesn't pause, with or without Studio's free camera. A mod still hears every key (`input_key`, its
key mappings), mouse button (`mouse_button_pre`/`_post`), the wheel and the bumpers (`mouse_scrolling`), and its
screens and prompts take input as usual, so a replay editor can run on a mod's own camera (`camera_pose`). Escape
closes the replay and P opens the pause menu.

Studio's editor saves its markers and camera path into the recording. For a replay a mod opened, it saves only into
the archive that was opened: if another file has been put at the recording's path since, the save is refused.

`ReplayState`, `ReplayError` and the hooks' events are `#[non_exhaustive]` for Rust callers only. On the wire every
field and variant is encoded and its layout hashed ([wire types](../../concepts/wire-types.md)), so they stay as they
are: adding one would break wasm mods built against this version.
