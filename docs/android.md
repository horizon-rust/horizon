# Horizon on Android (first pieces, untested)

A first Android build of the game: a NativeActivity app (`android_main` in `horizon-client`'s
`main.rs`), Vulkan through wgpu, and touch controls. **This was written without a compiler, an
Android toolchain or a device, so it has never been built or run.** Expect the first build to need
fixes.

## Build

`.github/workflows/android.yml` builds an APK on GitHub: Actions tab, "android", "Run workflow",
then download `horizon-android-apk` from the run. By hand, with the NDK and `cargo-apk`:

    rustup target add aarch64-linux-android
    cargo install cargo-apk --version 0.10.0 --locked
    cargo apk build --release -p horizon-client --bin horizon-client --no-default-features

`--no-default-features` leaves out sound and controllers (cpal and gilrs have no Android build set
up here yet). The APK is signed with whatever key `CARGO_APK_RELEASE_KEYSTORE` names.

## The game's files

Horizon ships none of Mojang's files, and `horizon-prepare` needs Java, which a phone doesn't
have. So the game starts only if the prepared resources are already in its folder:
`Android/data/dev.horizonlab.horizon/files/.local/horizon-client/26.3` (what `horizon-prepare`
writes on a computer; copy the folder over USB). That folder is also the game folder: `options.txt`,
`saves/`, `logs/latest.log`. Preparing on the phone is [#1750](https://github.com/horizon-rust/horizon/issues/1750).

## Touch controls

`crates/horizon-client-input/src/touch.rs` turns touches into the virtual pad's state, so the game
treats them like a controller (`game/pad.rs` merges them with a real one; the controller settings
and their bindings apply): drag on the left half for the move stick, on the right half to look, and
the buttons at the screen's edges (Hit, Use, Jump, Sneak, Sprint, Inventory, Pause, hotbar left and
right). In menus and inventories a touch is the mouse (`game/touch.rs`).

## Not done

- Leaving the app and coming back: the window's surface is lost and the renderer isn't rebuilt.
- Sound, controllers, text input (the on-screen keyboard), safe areas around notches.
- Looking is a rate stick, not a direct drag; a drag-to-turn mode is the obvious next step.
- armv7 and x86_64 builds (`build_targets` has arm64 only).
