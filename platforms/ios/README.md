# Nova — Horizon iOS host (scaffold)

This directory is an additive iOS/iPadOS host for the ORIGINAL Horizon game.
It does not port the desktop launcher and does not modify gameplay crates.

## Current status

- SwiftUI launch screen, including a disabled Play button until the game is wired in.
- Metal-backed MTKView wrapper available for the eventual game window.
- Rust staticlib with a stable C ABI header. Its game functions intentionally
  return *not ready*; they do NOT launch Horizon yet.
- GitHub Actions builds the Rust iOS bridge, compiles an unsigned iOS app, and
  packages the app as an unsigned IPA artifact for inspection.
- A signed, installable IPA and a running Horizon game are NOT yet available.

## Build on a Mac

Requires Xcode, Rust, the aarch64-apple-ios Rust target, and XcodeGen.

```bash
rustup target add aarch64-apple-ios
cargo test --manifest-path platforms/ios/RustBridge/Cargo.toml
cargo build --manifest-path platforms/ios/RustBridge/Cargo.toml \
  --release --target aarch64-apple-ios \
  --target-dir platforms/ios/Build/rust-target
mkdir -p platforms/ios/Build/rust
cp platforms/ios/Build/rust-target/aarch64-apple-ios/release/libhorizon_ios_bridge.a \
   platforms/ios/Build/rust/
cd platforms/ios
xcodegen generate --spec project.yml
xcodebuild -project HorizonIOS.xcodeproj -scheme HorizonIOS \
  -configuration Release -sdk iphoneos -destination 'generic/platform=iOS' \
  -derivedDataPath Build/Derived CODE_SIGNING_ALLOWED=NO build
```

Unsigned IPAs produced by CI are build artifacts, not ready for installation
on standard devices. Signing requires an Apple development or distribution
identity and provisioning appropriate to the target device.

## Next engineering tasks

1. Attempt a locked aarch64-apple-ios build of the original Horizon client
   and audit errors without changing gameplay files.
2. Establish an iOS-safe entry point and event-loop ownership; the current
   upstream entry::main() isn't automatically an iOS app lifecycle.
3. Connect the existing wgpu renderer to an iOS Metal-backed surface.
4. Wire host lifecycle, assets, sandbox paths, and original keyboard/mouse
   events. Implement touchscreen mapping after the game runs.
5. Replace the explicitly disabled bridge launch entry point only when actual
   in-process gameplay startup has been tested.

See root PLAN.txt for architectural constraints and upstream update policy.

## Game archive import

Nova can copy an existing ZIP file from Files.app into its sandbox at
`Documents/Nova/Imports/GameFiles.zip` without trying to run, extract, or
interpret its contents. The importer checks only the ZIP header; it does NOT
prove that the archive contains valid prepared Horizon assets. The original
game expects a prepared resource root containing `horizon-resources.json`
and resources under `assets/minecraft/`, and the current desktop entry
creates its own winit event loop. Do NOT enable Play simply because a ZIP
was imported. Examine the ZIP version and directory structure first. Do not
commit user-supplied Minecraft assets to this public repository.

## Metal surface integration diagnostic

Nova's "Test Metal display" screen creates an MTKView, presents a simple
GPU-cleared frame, and registers a **non-owning** UIView pointer through
`horizon_ios_set_metal_view` in either Rust bridge variant. It unregisters
the pointer when dismissed. This proves the SwiftUI -> Rust ABI and native
drawable creation can be exercised; it does NOT pass a wgpu surface to Horizon,
does NOT start Horizon's event loop, and is not game rendering.

## Player-owned resource ZIP extraction (no bundled assets)

Nova can import a player's prepared Horizon resources ZIP from Files.app,
then "Extract and verify game files" **locally**, using the shared Rust
`platforms/ios/AssetImporter` component (with no network calls). The ZIP may
contain the files at its root or inside one wrapper folder such as `26.3/`.

The installer rejects ZIP traversal paths, symlinks, duplicate names,
oversized entries and excessive expansion. It streams decompression into a
private temporary folder, checks `horizon-resources.json` as JSON and
requires `facts.json`, `registries.json`, and nonempty `assets/minecraft/`.
On validation success it replaces resources transactionally; failure preserves
the previous working copy. The prepared files live in
`Application Support/Nova/Resources` (excluded from iCloud backup).
Nova's temporary cached ZIP copy is deleted after successful preparation,
without touching the user's original selected ZIP.

**This is structural validation only**, not compatibility or runtime testing.
Neither an existing ZIP nor installed files enable the Play button. Horizon's
original winit/Metal lifecycle must still be connected.

**No game assets or downloaded ZIPs are committed or distributed.** Both IPA
build workflows check the final archive file listing to reject bundled assets.

## Single-lifecycle iOS game startup experiment

The integrated game-linked Nova app now has a **cold-start experiment**, not
a normal in-app Play transition. Because winit 0.30 on iOS runs
`UIApplicationMain`, it cannot start after SwiftUI has already initialized
UIKit. Nova has a single entrypoint (`NovaEntryPoint`) which chooses ONE:

1. Normal SwiftUI Nova launcher; prepare the player's own game assets locally.
2. If the user explicitly tapped "Try Horizon (next launch)" and then fully
   terminates and reopens Nova, run the ORIGINAL official Horizon client from
   the process main thread. That process is owned by winit, not SwiftUI.

The one-shot boot marker is cleared BEFORE the game starts, so a startup crash
doesn't leave Nova in an endless game-boot loop. Launcher-only builds ignore
this mode. Game-linked builds use a private resources path from
`Application Support/Nova/Resources` and game home from
`Application Support/Nova/Game`.

Only the CI checkout of `crates/horizon-client/src/game/launch_options.rs`
is patched to allow the iOS resource root to be provided through the private
`NOVA_IOS_RESOURCES` environment variable. No upstream gameplay source is
committed or permanently changed.

**Status:** Startup compiles only until device verification. Winit lifecycle,
Metal menu rendering, iOS audio, sandbox filesystem, backgrounding and input
may still fail. This is NOT a playable or device-verified build. The regular
Nova launcher still works, but there is no hot-switch back to it during a
winit-owned session. A later version should support a clean single-lifecycle
launcher-to-game handoff without relaunching.

## Exportable black-screen diagnostics

The launcher now offers **View Logs** and **Export Logs**. Diagnostics persist
in the app's private `Application Support/Nova/Diagnostics/nova-diagnostics.log`
across crashes and cold launches, and are excluded from iCloud backup. The log
is capped to about 2 MB with a previous-log rollover when Swift next writes.
Only choosing Export Logs opens the system share sheet; there is no telemetry
upload. Don't share logs publicly without checking their contents.

The cold-start Swift entry records which branch was chosen; the full Rust
bridge writes stages before it enters the original Horizon startup. The
GitHub Actions runner injects small **iOS-only, diagnostic-only** temporary
breadcrumbs into the existing Horizon loading/window code to record winit
lifecycle creation, window creation, renderer init and the first frame.
No original Horizon gameplay sources are modified in git.

To capture a black screen, start an experiment, fully close and reopen Nova
to run it, then force-close Nova and reopen normally. The one-shot startup
flag is cleared before the game entry, so the launcher returns on the next
cold start. Open View Logs or use Export Logs to share the log. A log with
no renderer success is evidence of an early startup problem, not proof of a
Metal failure.
