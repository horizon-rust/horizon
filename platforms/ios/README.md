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
