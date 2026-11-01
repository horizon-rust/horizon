# Horizon Mobile

A fork of [horizon-rust/horizon](https://github.com/horizon-rust/horizon) (MIT) aimed at Android first.
Imported from upstream commit `7aa0e28faee33841b60906e69f16b47e54681a7f`. Horizon ships none of Mojang's
files: the player supplies their own Java Edition 26.3 assets.

## Status

| Piece | State |
|---|---|
| Upstream source | Imported unchanged |
| `crates/horizon-touch` | Done and unit-tested: floating joystick, look drag, tap to use, hold to attack, jump/sneak buttons, tappable hotbar. No platform dependencies |
| Wiring touch into `horizon-client` | Done for the playing screen (`touch_event` / `touch_tick` in `game.rs`): joystick feeds the pad-stick path, look feeds the mouse accumulator, tap/hold call `play_mouse`, jump/sneak set Space/Shift, hotbar calls `key_pressed`. Type-checks on Linux; untested at runtime |
| Touch in menus, inventory, chat | Done (`touch_as_mouse`): off the playing screen the first finger is replayed as cursor moves plus left press/release through the normal window handlers, so buttons, drag-and-drop in inventories and sliders get the same events a mouse sends. Type-checks; untested at runtime. No right-click, shift-click or scroll by touch yet, and no soft keyboard for text boxes |
| Editable on-screen buttons | Done in logic and wired (`layout.rs`, `draw_touch`): buttons for jump, sneak, sprint, attack, use, inventory, chat, keyboard, drop, swap hands, view and menu. Tap the **Edit** gear (top right) to enter edit mode: drag a button to move it, tap to select, then use the bar to Add, Remove, change Action, resize (- / +), make it a Toggle (latches held actions like sprint) or Reset. Saved to `touch_layout.txt` in the game folder. Drawn only once a finger has touched the window (always on Android). The Keyboard action opens chat, which brings up the system keyboard; there is no free-typing keyboard during play |
| Hotbar alignment | Hotbar touch zones are computed from the game's GUI scale to match the vanilla hotbar; unverified on a real screen |
| Android library | Builds: `libhorizon_android.so` for arm64 with the NDK (`docs/android.md`), exporting `android_main`. Desktop-only code gated (clipboard, key layout). No sound. `cargo apk build` makes a signed ~23 MB arm64 `Horizon.apk` (docs/android.md); never installed or run on a device |
| Mobile UI (scaled menus, on-screen inventory/chat) | Not started |
| Launcher screen in the desktop launcher's Night look (all six pages) | Redrawn in the game's UI; Play, Settings, Patch notes, Versions work, Mods and Servers are placeholders (docs/android.md). Checked on a desktop, not on a device |
| Launcher: Microsoft sign-in, owner check, Play downloads Mojang's files | Built (`horizon-bootstrap`, `mobile_launcher`): parsing checked against the real 26.3 version file, screen rendered and checked on a desktop. The sign-in and the download were never run end to end: no Azure client ID, and this sandbox can't reach Mojang from Rust. Needs `msa_client_id.txt` and Mojang's approval of that app (docs/android.md) |
| Java 25 on the device | Termux's OpenJDK 25 (native Android build) is downloaded and unpacked by Play (`java.rs`); the preparer runs it. Unpacking and the full preparation checked on a computer; Termux's Java never run on a device |
| Loading the player's own assets on a phone | Not started (upstream #1750: prepare without Java). The game reads them from the app's data folder |
| iOS | Not started |

The Android library compiles, but nothing here has been installed or run on a device.

## Next steps

1. Check the drawn controls and hotbar zones on a real screen.
2. Install the APK on a device or emulator and fix what breaks (the first run has not happened).
   `#[no_mangle] android_main`, and gate the desktop-only dependencies (`arboard`, `gilrs`, Discord presence, `rustix` use).
3. Replace the Java-based asset preparation with a path that runs on-device.
4. Mobile UI: larger hit targets, GUI scale default, soft keyboard for chat and signs.

## Try the touch crate

    cargo test -p horizon-touch
