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

## Combined native app link (current milestone)

The iOS runtime library now exports the same host ABI as the standalone Rust
scaffold. The original Horizon entry is kept reachable by the linker, but is
NOT called: `horizon_ios_game_is_ready()` is intentionally false.
The compile-probe workflow additionally builds a native iOS SwiftUI app linked
against the actual game static library and attempts an unsigned IPA package.
This is not proof that Horizon starts or renders on iOS. A future change must
integrate Horizon's event loop, Metal surface, lifecycle and input without
editing gameplay. The original, lightweight host CI still links the stub.
