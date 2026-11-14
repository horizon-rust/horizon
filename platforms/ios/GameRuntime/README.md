# Original Horizon game: iOS cross-compilation probe

The current official game binary calls
`horizon_official::register()` followed by `horizon_game::main()`.
This crate references those exact ORIGINAL upstream interfaces; no gameplay
source is copied or changed.

This is a compile/linkability investigation only. **Do not link or call the
entry from the Swift host yet**: the existing engine assumes desktop-style
event-loop ownership, and its window/input/render lifecycle must be integrated
properly before invoking the game. Even a successful `cargo check` is not
a playable IPA or proof that final native linking works.

The workflow `.github/workflows/ios-runtime-probe.yml` performs a native
iOS Rust target check and uploads a compiler log. Compiler errors are kept
visible without blocking the separate, already-working iOS host build. Once
platform compilation passes, switch to `cargo build` and link into the host;
then validate iOS window ownership, Metal, resources and gameplay on device.
