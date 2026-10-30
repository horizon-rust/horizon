# Horizon on Nintendo Switch (homebrew, .nro): status and plan

Reference: `skate3rust-switch` (a Skate 3 engine on **Bevy**) built with
[bevy-horizon](https://github.com/JohnUzoka/bevy-horizon) (toolchain, patched crates, NVK, libnx glue).
Horizon is **not** Bevy: it is winit + wgpu 30 + gilrs, so only part of that method carries over.

## What the reference does, and what Horizon takes from it

| Area | Reference | Horizon |
|---|---|---|
| Toolchain | Container: devkitA64 + libnx + nightly Rust, `-Zbuild-std`, target `aarch64-nintendo-horizon.json`, `scripts/build_game.sh` | **Done, unrun**: `packaging/switch/` (below) |
| Rendering | wgpu Vulkan on NVK (Mesa, mesa-switch); `wgpu-hal` patched (`VK_NN_vi_surface`, driver linked statically, NVK listed despite non-conformance, naga bounds checks); `raw-window-handle` Horizon handle | **Patches ported to wgpu 30.0.1** (below). NVK itself (`build_nvk.sh`, mesa-switch + 24 patches) is still to build in the container |
| Window / loop | `bevy_horizon` replaces winit | **Done as a winit backend**: `third_party/winit` is winit 0.30.13 plus `platform_impl/horizon` (below), so none of the 51 files that use `winit::` change |
| Input | `padConfigureInput` + `padUpdate` (player 1 + handheld) | **Done** in `crates/horizon-switch`, wired in `game/pad.rs` |
| Audio | cpal patched onto libnx's audren (48 kHz) | **Ported to cpal 0.16.0**: `third_party/cpal` (below) |
| Memory | ~3.1 GiB heap when launched as a game (hold R); `top_down_alloc`, paced uploads, small mesh slabs | **Budget in place** (`switch_profile.rs`, below); no `top_down_alloc` yet; unmeasured on hardware |
| Files | `sdmc:/switch/<game>/` beside the NRO; FAT32 (no file ≥ 4 GiB), case-insensitive | **Done**: layout and `tools/switch_export.py` (below) |
| Shaders | NVK cache in `sdmc:/.mesa` | Same |
| Packaging | NRO + `switch_export.py` copies the user's PC-prepared files | **Done**: `packaging/switch/build.sh` makes the NRO, `tools/switch_export.py` copies it with the resources |

No equivalent in the reference:

- **Prepare step (JDK):** stays PC-only; the Switch never runs it. The export script copies its output.
- **Mods (WebAssembly):** off on Switch at first (wasmtime/Cranelift needs executable memory). Try wasmtime's Pulley interpreter later.
- **Server / LAN / friends:** off at first; sockets work in libnx but are untested in the reference.
- **Distant terrain:** keep the LOD on, cap its range and mesh memory with the 4 GiB budget; measure on hardware.

## Controllers (done in this change)

`crates/horizon-switch` (pure logic tested on PC; `hid.rs` only builds on `target_os = "horizon"`):

- libnx buttons to Horizon's positional `Button` (Nintendo B is `South`, A is `East`); the game already
  shows Nintendo glyphs and has focus navigation, pointer mode, dead zones, sensitivity and rebinding
  (`gamepad.rs`), so all menus, inventory, recipe book and options need no new code.
- ZL/ZR are the triggers (attack / use), L/R the hotbar, sticks click for sprint / sneak, + is pause.
- Hot-plug: a change of HID style (Pro Controller, handheld, dual or single Joy-Con, none) releases
  every button for two frames, so nothing sticks when a Joy-Con is detached or attached to the grip.
- A single sideways Joy-Con: SL/SR as bumpers, stick and d-pad/buttons rotated. **Untested on hardware.**
- Touch screen (1280x720): one finger as a pointer, a second finger scrolls (`TouchTracker`). **Not yet
  connected to the game's mouse input** (needs the winit shim above).

Text fields: see "System keyboard" below.

## Order of work

1. Container + build script producing a hello-world NRO that links `horizon-core`/`horizon-world`.
2. wgpu 30 + NVK: port the `wgpu-hal` patch, show a clear colour.
3. winit shim (window, loop, keys, HOME/sleep, touch), then `horizon-client` boots to the title screen.
4. Memory budget, file layout, export script, swkbd, audio.
5. Hardware tests: handheld, docked, detached Joy-Con, Pro Controller, hot-plug.

Nothing here has run on a Switch; no Linux/devkitA64 toolchain was available while writing it.

## wgpu 30 patches (ported from bevy-horizon's wgpu 29.0.4 set)

- `third_party/wgpu-hal` (already patched for everyone): the Vulkan changes: `VK_NN_vi_surface`
  surface from `RawWindowHandle::Horizon`, the statically linked driver's `vkGetInstanceProcAddr` instead
  of `ash::Entry::load`, always-on debug-utils messenger, NVK adapter accepted, naga bounds-check
  clamping (`BH_ROBUST_BUFFER_ACCESS`), `BH_RT_COMPRESSION`, no libloading/DRM/RenderDoc on Horizon.
  Hunks that no longer applied (DRM exclusions) are not needed: wgpu 30 gates DRM on Linux/BSD by `build.rs`.
- `third_party/{wgpu,wgpu-core,wgpu-core-deps-windows-linux-android}`: Vulkan backend enabled for
  `target_os = "horizon"`; wgpu-core logs the call that lost the device and drops indirect-call
  validation in release builds.
- `third_party/raw-window-handle`: adds `HorizonWindowHandle` (an `NWindow`) and `HorizonDisplayHandle`.
- Only the Switch build uses the last three groups: `cargo build --config packaging/switch/cargo-patch.toml
  --target aarch64-nintendo-horizon.json ...`. Checked on the host only (`cargo check -p horizon-client
  --lib --config packaging/switch/cargo-patch.toml`); the `target_os = "horizon"` code has not been compiled.
- Not ported: bevy-horizon's allocator, upload pacing and timestamp-period fix (Bevy-side or NVK-side).

## winit backend (`third_party/winit`)

winit 0.30.13 with a new `platform_impl/horizon` (modelled on its Orbital backend), selected by
`target_os = "horizon"`; X11/Wayland dependencies are excluded there. Used through
`packaging/switch/cargo-patch.toml`. It provides:

- One window on libnx's default `NWindow` (`nwindowGetDefault`), raw handle `RawWindowHandle::Horizon`
  (the patched raw-window-handle, which wgpu-hal turns into a `VK_NN_vi_surface`).
- The event loop: `Resumed`, `Resized` on creation and when docking or undocking (1280x720 handheld,
  1920x1080 docked), `Focused` from the applet focus state, `CloseRequested` when `appletMainLoop`
  returns false (HOME, hbmenu), `RedrawRequested`, user events. `Wait`/`WaitUntil` sleep in 4 ms steps
  (HID has no wake-up); `Poll` yields the core, since a spinning thread starves lower-priority ones.
- The touch screen as a mouse (see "Touch" below).
- No keyboard events: the controllers are read by the game through `horizon-switch`; the on-screen
  keyboard handles text. No cursor grab (accepted and ignored); fullscreen is always on.

Type-checked on a PC with `winit`'s `horizon` feature (which builds the backend on any OS); never
compiled for the Switch or run. `horizon-client` still has Unix-only dependencies to settle when
the first real build is tried (`rustix`, `cpal`'s backend, `getrandom`, `libc`: bevy-horizon
patches the same crates, `.cargo/bevy-0.18.toml`).

## Building (`packaging/switch/`)

Needs Docker or Podman on Linux or WSL2. Not run yet: this machine had neither.

```sh
packaging/switch/build.sh all     # image, then NVK (slow, once), then the NRO
packaging/switch/build.sh nro     # just horizon-client -> target/aarch64-nintendo-horizon/release/horizon.nro
```

- `Containerfile`: devkitA64, libnx, `switch-mesa` (expat, zstd, zlib), a pinned nightly Rust.
- `build.sh` clones `../bevy-horizon` (the target spec, the patched libc, getrandom, memmap2, rustix,
  ring, and the NVK scripts) and `../mesa-switch` at fixed revisions, builds the image, builds and
  localizes NVK, then runs `cargo build -Z build-std -Z json-target-spec -p horizon-client
  --no-default-features` with three `--config` files: bevy-horizon's `.cargo/config.toml`,
  `cargo-bevy-horizon.toml` (its crate patches) and `cargo-patch.toml` (our wgpu, winit,
  raw-window-handle). It puts `Cargo.lock` back afterwards, then packs the NRO with `nacptool` and
  `elf2nro` and checks its asset section (sphaira refuses an NRO without one).
- `crates/horizon-switch/build.rs` links libnx, newlib and NVK on the Switch target;
  `horizon-client/build.rs` adds `-z notext`.
- `crates/horizon-switch/src/rt/`: bevy-horizon's runtime without Bevy (`compat.rs`: TLS pointer,
  `threadCreate` and POSIX shims, `userAppInit`/`userAppExit`, crash reports; `logging.rs`:
  `log.txt` and `panic.txt` beside the NRO; `memory.rs`; `lifecycle.rs`: romfs, sockets, closing them
  so the homebrew loader can start the next NRO). `horizon_switch::rt::init` runs first in `main`.
  Type-checked on aarch64 with `cargo check -p horizon-switch --features rt-check --target
  aarch64-pc-windows-msvc` (only the two `std::os::unix` errors remain, expected off Horizon).
- Not in the build: controllers through gilrs (replaced by `horizon-switch`), mods, and the
  top-down allocator that keeps the heap unfragmented.

## Audio (`third_party/cpal`)

cpal 0.16.0 plus bevy-horizon's Horizon host (`src/host/horizon/mod.rs`, from its cpal 0.15.3 patch),
which type-checks unchanged against 0.16's traits; `host/mod.rs`, `platform/mod.rs` and `build.rs`
(links libnx) get the same small edits. One output device, "Nintendo Switch", 48 kHz stereo through
audren; a thread of its own (priority 0x2B) converts f32 to 16-bit PCM into three queued buffers
(1024 frames by default; `BH_AUDIO_FRAMES`, `BH_AUDIO_PRIORITY`, `BH_AUDIO_STATS=1`), and an `atexit`
handler closes a stream that was never dropped (audren's transfer memory would keep heap pages
locked). `packaging/switch/build.sh` builds with `--features audio` and `cargo-patch.toml` applies
the patch. Type-checked with the `horizon-check` feature on a PC; the reference's host was tested on
hardware with cpal 0.15.3, this port not. Horizon's own `audio.rs` was not changed.

## Memory budget (`crates/horizon-client/src/switch_profile.rs`)

A program started as a game has about 3.1 GiB of heap for everything, GPU memory included, and three
usable cores. On the Switch (or any host with `HORIZON_SWITCH_PROFILE=1`):

- **First launch** (no `options.txt`): Performance preset, render distance 8, simulation distance 6,
  60 FPS with VSync, distant terrain off, one chunk worker, section uploads paced at 8 MiB a frame.
- **Every launch**, whatever the file says (a PC `options.txt` copied over): render distance at most 12,
  simulation distance 8, distant terrain 96 chunks, no supersampling, workers and upload budget never 0.
- The numbers are estimates, not measurements: `log.txt` has `[Memory]` lines (applet type, heap,
  malloc and the top-down region), so raise the caps only after reading them on a console. Docked
  mode (1920x1080) is untested and may need a lower resolution scale.
- Not done: bevy-horizon's `top_down_alloc` (large Rust blocks placed beside the GPU's, so freed
  memory does not fragment the heap), and a fixed texture-atlas or mesh-slab cap.
- std has no `current_exe` there: `horizon_core::exe::current_exe` falls back to `argv[0]`, the NRO's
  path, for the resources folder and the caches' fingerprints.

## SD card layout and export

```
switch/horizon/horizon.nro      start it from the Homebrew Menu holding R (an NRO started from the
                                Album gets far less memory)
switch/horizon/resources/       the prepared resources: `.local/horizon-client/26.3` of the PC
switch/horizon/                 HORIZON_HOME: worlds, options.txt, logs/, log.txt, panic.txt
.mesa/                          NVK's shader cache (the first launch compiles; deleting it is safe)
```

`python tools/switch_export.py E:\` (after `packaging/switch/build.sh nro`, and the PC preparation,
which needs a JDK) copies the NRO and the resources, copies only what changed on a rerun, leaves
saves and settings alone, and refuses a file of 4 GiB or more on FAT32, names that differ only by
case, and a card without room. `--check` lists what it would do. Tested on a temporary folder
(`tests/test_switch_export.py`), not on a real card; the prepared folder is copied whole, so its
size on the card is unknown until it is tried.

## System keyboard (`crates/horizon-switch/src/swkbd.rs`)

A on a text field (and chat or a sign, which open it by themselves) calls libnx's `swkbdShow`, the
system software keyboard, in place of Horizon's own on-screen keyboard (`osk.rs`), whenever the
controller setting "Keyboard" is not "Horizon" (so "Auto" is the system one on a Switch, and "Horizon"
keeps the pad-driven one). The applet blocks the game while it is open. Its text replaces the field's
(`osk::replace_with`: 256 backspaces, then the text) for chat, signs, searches, the anvil name, the
world name and seed fields; a book page is appended to instead, so a page is never lost; a chat
message is sent when the keyboard is confirmed. Cancelling changes nothing. It also works with the
touch screen in handheld mode, as the applet's own. `SwkbdConfig` is not mirrored: libnx fills a
16 KiB zeroed buffer. The `target_os = "horizon"` code in `game/pad_menu.rs` is type-checked by
no host build; the FFI declarations are (`--features ffi`). Not tried on a console.

## Touch (handheld mode)

The winit backend turns the touch screen into mouse events, so every menu, the inventory, the creative
tabs, sliders, text fields and the recipe book already respond, with no change to their code:

- One finger moves the cursor and holds the left button while down (tap = click, drag = drag a slider,
  carry a stack, scroll a scrollbar).
- A second finger dragging scrolls (a wheel event; dragging down scrolls up, like a touch pad).
- A quick tap of a second finger (under 300 ms, within 12 px) is a right click at the first finger:
  split a stack, pick up half, place one item.
- The game already hands the pointer back to the mouse when it moves (`pad_menu.rs`), so touching a menu
  leaves focus mode, and the next pad input returns to it.
- While playing (cursor captured), touches are ignored (`game.rs`, `window_event`): they would mine,
  place or scroll the hotbar. The controller plays; the touch screen is for menus.
- Docked, the screen is 1920x1080 and has no touch. Not tried on a console; the tap and scroll logic
  is tested (`horizon-switch` `touch.rs`), the `target_os = "horizon"` branch of `game.rs` was
  type-checked by building it once on a PC.

## Review before the first build

State: everything below type-checks or tests on a PC; nothing has been compiled for `aarch64-nintendo-horizon`
or run on a console. `cargo clippy -D warnings` and `cargo fmt --check` pass on the crates touched.

Fixed in the review (would have broken the first build or made a feature useless):

- `getrandom` 0.3.4 (horizon-friends) had no Horizon backend: `third_party/getrandom-0.3`, patched in
  `cargo-patch.toml`; bevy-horizon only patches 0.2 and 0.4.
- `arboard` (X11 and Wayland) is not built on the Switch; the game keeps copied text itself.
- `if-addrs` (`getifaddrs`) is not built there; Play with Friends offers no local address, the relay still works.
- The unix-socket control endpoint, Discord presence and `rustix`'s thread clock are left out on
  Horizon; `horizon-ctl`'s TCP endpoint is the transport.

Known risks, in the order I expect them to show:

1. **Crates that may not compile for an unknown OS**: `socket2` (`all` features), `iana-time-zone` (chrono
   `clock`), `cpufeatures`, `parking_lot_core`, a second `rustix` (0.38.44, unpatched), and uses of
   `libloading` in wgpu-hal 30 beyond the ones the 29.0.4 patch removed. The build will name them;
   bevy-horizon's `patches/` is the first place to look.
2. **Paths**: libnx paths start with `sdmc:`, which Rust's `Path` does not treat as absolute. Code that
   calls `std::path::absolute` (e.g. `mod_content.rs`) would prepend the working directory. Relative
   paths (from the NRO's folder) are safe; `HORIZON_HOME` is set to the NRO's folder by `rt::init`.
3. **`available_parallelism`** may be unsupported on the target (the callers fall back to 1 thread: LAN
   chunk warm-up then has none, far terrain has one sampler).
4. **Memory**: every number in `switch_profile.rs` is an estimate. Read `[Memory]` in `log.txt` first.
5. **NVK**: the GPU hang bevy-horizon reports in lit scenes (about 10 s in, "fifo idle timeout") may
   apply; docked mode, detached Joy-Con rotation and the touch gestures are untested.
6. `build.sh` has never run: the first run will most likely need a fix or two.

First-build checklist: `packaging/switch/build.sh all`; fix compile errors from risk 1; start from the
Homebrew Menu holding R; read `switch/horizon/log.txt` (and `panic.txt` after a crash); then
`BH_AUDIO_STATS=1`, `[Memory]` and frame times.
