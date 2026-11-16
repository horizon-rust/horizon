# Rebuild native mods

A native build uses the instance's pinned Horizon commit. Editing or disabling a mod rebuilds
the mods against that same prebuilt SDK. A mod made for another commit is built when it fits this
one ([What the launcher does](04-what-the-launcher-does.md#mods-made-for-another-build),
[the rule](../../reference/sdk/04-fit.md)); building never repins a mod.

## Update the base

`native-build --update` selects a newer installed release or Nightly and checks every mod against
it. A build without a published SDK says it has no mod tools yet; use a build that has them.

## Toolchain and storage

The launcher uses its pinned Rust toolchain, not the player's Rust installation. It isolates
compiler flags, Cargo configuration and target settings from the player's environment. Build
inputs and outputs stay in managed launcher storage. On Windows the build tools require paths
the linker can open; the launcher chooses a managed ASCII path when necessary.

Cargo unpacks crates with file times from 1970, which FAT32 and exFAT, the usual formats of
portable drives, can't store. When the build folder is on such a drive, Cargo's home moves to
`%ProgramData%\horizon-launcher-<id>\cargo`, named for the build folder, and uninstalling removes
it with the data.
