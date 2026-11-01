# Building the Android library

`horizon-client` has an example, `horizon_android`, that builds as a `cdylib` (`libhorizon_android.so`) with the
`android_main` entry point NativeActivity loads. On other platforms it is empty.

## What is verified

On Linux with NDK r27c, this builds the library for arm64 and exports `android_main` and `ANativeActivity_onCreate`:

    rustup target add aarch64-linux-android
    N=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin
    export CC_aarch64_linux_android=$N/aarch64-linux-android26-clang \
           AR_aarch64_linux_android=$N/llvm-ar \
           CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$N/aarch64-linux-android26-clang
    cargo build --target aarch64-linux-android -p horizon-client --no-default-features \
        --example horizon_android

## APK

With the Android SDK (platform 34, build-tools 34.0.0) and the NDK, `cargo-apk` packages and signs it:

    cargo install cargo-apk --locked
    export ANDROID_HOME=... ANDROID_NDK_ROOT=...
    cd crates/horizon-client
    cargo apk build --profile fast --no-default-features --example horizon_android

This gives `target/fast/apk/examples/Horizon.apk` (about 23 MB, arm64, minSdk 26, landscape, signed with the debug keystore
in `packaging/android/`, so it installs with `adb install` but is not for publishing). The settings are in
`[package.metadata.android]` in `crates/horizon-client/Cargo.toml`; `packaging/android/AndroidManifest.xml` is an unused
hand-written equivalent. The `fast` profile built in about 7 minutes on 4 cores.

Not done yet: running on a device or emulator (the APK was built and its signature and manifest checked, never installed), sound (`--no-default-features` leaves out `audio` and `controllers`;
the audio backend needs an Android build of its dependency), and a release/size-optimised build (the debug library is
about 350 MB).

## How it works

- `android_main` (examples/android/main.rs) sets the working directory to the app's private data folder, stores the
  `AndroidApp` in `horizon_client::ANDROID_APP`, and calls the program's `main`.
- `new_event_loop()` in `game.rs` builds the winit event loop from that activity.
- The game folder is the private data folder: `options.txt`, `saves/`, `touch_layout.txt`. Game resources prepared
  from the player's own Minecraft files must be placed there; without them the game shows its missing-resources message.
- `examples/android/game.rs` and `game/` are symlinks to `src/` so the example can hold the game as `crate::game`.
- No system clipboard on Android: copy and paste work inside the game only.

## Logs when something goes wrong

The app also copies its log to the clipboard 10 seconds after start and again when the game stops: open any text field and paste.

The Android build sends its log, stdout and stderr (panic messages included) to logcat under the tag `Horizon`:

    adb logcat -s Horizon

It also sets `HORIZON_HOME` to the app's external folder (`Android/data/dev.horizon.mobile/files/`) so `logs/latest.log`
and `crash-reports/` land somewhere `adb pull` can reach, and `HOME` to the private data folder.

## First run: sign in, then Play downloads the game

The screen is the desktop launcher's Night look redrawn in the game's UI (the desktop launcher is GPUI, which has
no Android backend): the banded night sky with stars, the top bar with Play, Versions, Mods, Servers, Patch notes
and Settings, display titles and numbered rows, from the same tokens (`horizon_theme::night`) and fonts. Drag to
scroll; tap to press. `HORIZON_LAUNCHER_PAGE=settings` (and `versions`, `mods`, `servers`, `notes`) opens on a page,
and `HORIZON_MOBILE_LAUNCHER=1 horizon-client` shows it on a computer.

What each page does today: **Play** (state, Play, sign-in code, download progress, Resources and account rows),
**Settings** (Microsoft account sign in/out, sign-in app, folders, versions) and **Patch notes** (the project's
releases) work; **Versions** lists what is installed; **Mods** and **Servers** only say they are not in the mobile
launcher yet. Left out of the sky: the clouds, sun and skylines.

With no prepared game files the Android build opens its own launcher screen (`mobile_launcher.rs`,
`game/mobile_launcher.rs`, crate `horizon-bootstrap`):

1. **Sign in with Microsoft** shows a short code and the page to enter it at (Microsoft's device-code
   flow, so no password goes through the app), then checks Xbox Live, the Minecraft services and that the
   account owns Minecraft: Java Edition. The refresh token is kept in `account.json` in the app folder, and
   the next launch signs in again by itself.
2. **Play** (only for a verified owner) downloads the pinned version from Mojang into `minecraft/` in the
   official launcher's layout (client and server jars, libraries, asset index and objects), each file checked
   against Mojang's sha1, resuming where it stopped, then runs `horizon-prepare` on it.
3. **Start Horizon** restarts the app, which now finds the prepared files and opens the game.

**The Azure app.** Sign-in needs a Microsoft Azure app (client ID): copy it and press **Paste client ID** on Play or in Settings, or put it on the first line of
`msa_client_id.txt` in the app folder (or `Android/data/dev.horizon.mobile/files/`), or build with
`HORIZON_MSA_CLIENT_ID`. Mojang must approve the app for the Minecraft services; until it does, sign-in
fails at the last step with that message. Horizon's own ID is not in this repository.

**Java 25.** Preparing runs the vanilla exporters, which are Mojang's own Java code and need Java 25
(`research-lock.json`). Android has none, so Play installs one first (`horizon-bootstrap`'s `java.rs`): Termux builds
OpenJDK 25 natively for Android arm64 and publishes it as `.deb` packages; the app downloads the JDK (about 115 MB)
and four small libraries (`libz.so.1`, `libandroid-spawn.so`, `libiconv.so`, `libc++_shared.so`) from `packages.termux.dev`, checks each
against the SHA-256 in the repository index, and unpacks them to `runtime/java` and `runtime/lib`. `horizon-prepare`
runs `bin/java` and `bin/javac` there with `LD_LIBRARY_PATH` and a private temp folder (`java.rs`, `android_env`). The
app targets Android 9 (API 28) because from Android 10 an app that targets 29 or later may not run programs from its
own folder. The exporters' Java sources are packed into the app by `build.rs` and unpacked next to the lock file.
Checked here: the real Termux package unpacks correctly, and the whole preparation (every exporter, ending in
`Prepared Horizon native-client resources`) runs from the same minimal folder with a desktop Java 25. Not checked:
Termux's Java actually running on an Android device.

Try the screen on a computer with `HORIZON_MOBILE_LAUNCHER=1 horizon-client`; check the download with
`cargo run -p horizon-bootstrap --example fetch -- DIR` (add `--assets` for the sounds).

## Signing in the way the EDU-DONUT bot does

A 16-digit hex app ID (not an Azure UUID) selects the live.com device-code flow that `prismarine-auth` and other
bots use: `login.live.com/oauth20_connect.srf` with the scope `service::user.auth.xboxlive.com::MBI_SSL`, polls to
`oauth20_token.srf` that keep going through `invalid_grant` until the code is entered, then Xbox Live with a `t=`
ticket, XSTS, the Minecraft login, the ownership check and the profile. The **Use Minecraft's own app ID** button on
Play (and in Settings) saves the Minecraft launcher's ID, `00000000402b5328`, as the app ID. It is never the default:
the player presses it after reading that Microsoft's page will name Minecraft as the app, that Horizon is not
approved by Mojang, and that the risk is theirs. The flow is checked against a local mock of Microsoft, Xbox and the
Minecraft services (`msa.rs` tests: request shapes, cookie, `t=` ticket, owner and non-owner), not against the real
services.

## Touch in the game

- **Menus and screens** (title, pause, inventory, chat, the onboarding screens): a finger is the mouse, and a white
  arrow shows where it is. Tap to click; a second finger tapped is a right click and dragged scrolls.
- **Playing** (the pointer captured): the left half is a floating joystick, the right half turns the camera (a quick
  tap uses, a still hold attacks), and the buttons of the editable layout (`touch_layout.txt`) sit on top.
- **The ≡ button** (bottom left, with the frame rate) opens the touch menu: *Controls* (edit or reset the button
  layout, show or hide the buttons, their opacity) and *Game Menu* (copy the log, FPS display, quit). Drawn in
  `touch_ui.rs`, driven from `game/touch_menu.rs`; settings in `touch_settings.txt`.
- `HORIZON_TOUCH=1 horizon-client` (or `=menu`) shows the overlay on a computer.
- Android has no pointer to grab, so the game counts the pointer as captured whenever it would grab it; that is
  what makes looking around and the controls work.

### Binding keys to buttons

In the touch menu choose *Edit control layout*. Drag a button to move it, tap it to select it, then press **Key**:
a picker shows every key and mouse button (letters, digits, F1-F12, Esc, Tab, Enter, Space, Shift, Ctrl, Alt, the
arrows, punctuation, left/middle/right mouse). The button then presses that key like a keyboard would: held while
your finger is down (or latched with **Toggle**), and a tap shorter than a game tick still counts. The default
layout has W/A/S/D buttons, Jump, Sneak, Sprint, Pause (Esc), Inventory and Chat. The ≡ menu's *Pause menu* row
presses Esc: from a world it opens the pause menu, which has Save and Quit to Title.

The game, as a client of its own single-player server, returns before its local tick, so touch is applied at
the very start of the tick and the joystick vector goes through `pad_move()`, which both paths read.

## The launcher is the home screen; instances; mods

The app opens on the launcher screen. **Play** writes a note (`start-game`, with the instance's name) and restarts the
app; the next start sees it and runs the game in that instance (winit allows one event loop per process, so the
launcher and the game are separate starts). In the game, the ≡ menu's *Back to the Horizon launcher* restarts the
app without the note, which shows the launcher again (`launch.rs`, `start_mode` in the Android example).

- **Instances** (Play page): chips for each instance, **+ New instance** (a dialog; the system keyboard comes up) and
  **Delete**. An instance is a folder under `instances/` that the game uses as its working directory: its own saves,
  options, mods, resource packs and touch layout. The prepared game files are shared (`HORIZON_RESOURCES`). The first
  start with instances moves the old single folder's saves and options into `Default`.
- **Mods page**: tabs for mods, resource packs and shader packs; what the instance has (with Remove); and Horizon's
  registry (`horizon-rust.github.io/horizon-mods/index.json`) with Install. A download is checked as on the desktop
  (`horizon_mods::index::check_file`: size, SHA-256, revocation, signature, the file's own manifest) and kept only if it
  passes. **Resource packs** have a source switch like the desktop's: **Modrinth** (search, most downloaded first, *Load more*, an
  option to show packs not marked for this game; the download's size and SHA-512 are checked, `modrinth.rs`) or Horizon's
  registry. Modrinth mods, data packs and modpacks are not in the mobile launcher.
- **Touch menus**: a Back button beside ≡ on every menu screen (Esc, which the on-screen buttons never covered);
  buttons resize by dragging the blue corner handle in edit mode; **Rename** gives a button a name (system keyboard);
  *Movement stick* in the ≡ menu swaps the W/A/S/D buttons for a fixed stick; the key picker has tabs.

### Keys on menu screens, F3, camera

- **Keys** (beside Back on menu screens) opens the key picker in *send* mode: the key you tap is pressed in the game. When
  Options > Controls waits for a key to bind, the picker opens by itself and the key (or mouse button) you tap becomes the
  binding. A touch screen has no keyboard, and the on-screen buttons are not shown over menus.
- **F3** and its combinations go through the game's debug-key path first (`debug_key`), as a keyboard's do, for a button bound
  to F3 and for the picker.
- The camera is applied as each touch event arrives, like a mouse, instead of once per 20 Hz game tick; *Camera sensitivity*
  in the ≡ menu scales it. A new instance starts with `renderDistance:6`.

### Online-mode servers

The game joins online-mode servers with a session: your name, UUID and Minecraft access token (docs/api/launch.md). Play
writes it to `session.json` in the app folder (mode 0600) just before the game starts; the next start points
`HORIZON_SESSION_FILE` at it, and the game reads it once and deletes it (it refuses one older than two minutes). The
token comes from the sign-in the launcher did when it opened, so it needs a verified sign-in: when the launcher could
only use the saved account offline, the game starts without a session and online servers refuse the player.

### The button editor

*Edit control layout* opens a small box at the top middle with the tools in a 5 by 2 grid (Add, Remove, Action, Key, Rename,
- , +, Toggle, Reset, Done), in the style of the ≡ menu. Nothing under the box is touched when you press its edge.

### Marketplace and key binds

- The Mods page lists everything in Horizon's registry, with a search box on every tab (the registry's lists are filtered
  here; Modrinth's resource packs are searched online). **Native mods** (41 of the registry's 43 projects at the time of
  writing) are Rust patches compiled into a custom build of the game, which needs a computer: they are listed with their
  authors and downloads but have no Install button.
- Buttons bound to a key (the W/A/S/D cross, anything from the picker) send it as a keyboard does, through the player's key
  binds (`translate_all`, as the `KeyboardInput` path): after Sprint is rebound to W, the W button sprints and a Ctrl button does
  not. The Jump, Sneak and Sprint *action* buttons press their action whatever it is bound to.

## Mods and packs folders

The launcher installs a registry mod into `horizon-mods/` (what the game's Options > Mods reads) and turns it on in
`horizon-mods.txt`; `.hmod` files an older build left in `mods/` are moved there when the list is opened. A resource
pack that fails to load now shows the game's reason in the "failed to load" toast (and the log).
