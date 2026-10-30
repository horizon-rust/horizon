# Native mods (experimental)

Native mods are **mixin bundles** that change the game's own Rust source before it is compiled, with
[horizon-mixins](https://github.com/horizon-rust/horizon-mixins) (MIT, by levzzz5154, made for Horizon). A bundle
is a `horizon-mixin.toml` and a few `.rs` helpers that hook into named functions of the game (HEAD, BEFORE_CALL,
RETURN, MODIFY_ARG, OVERWRITE...; the tool's README lists them). A port such as Veinminer is three HEAD hooks and
one module. Start a mod from [horizon-native-mod-template](https://github.com/horizon-rust/horizon-native-mod-template).

Native mods are native code with the player's rights, not sandboxed. The launcher turns them on only when the player
installs one (after a notice) and builds them only behind a red warning; the registry lists one only after a review; and
they play in a game of their own, built for the instance they are installed in.

## Pinned to a Nightly

A native mod builds on one exact Horizon commit: `[compatibility] horizon_revision` in its `horizon-mixin.toml`,
and `horizon` in the `horizon-native.json` of its registry file (`.hnmod`, `horizon_mods::index::native`). Mods
pin a **Nightly's commit** (or a release's). Every Nightly ships its source and keeps its commit for good as the
tag `nightly-source/<date>-<short>` ([release-flow.md](release-flow.md#nightly-channel)), so a pinned mod still
builds after its Nightly is replaced. All the mods of one build must pin the same commit; when they don't, the
launcher says which mod pins which, and the player installs versions made for the same Nightly. A bundle with no
pin builds on the commit of the installed version the build starts from.

### Mods on the hook points

A mod that hooks only Horizon's native hook points (`crates/horizon-client/src/game/native_hooks.rs`: empty
functions the game calls at fixed places, plus helpers, kept stable across releases) can name the version it
needs instead of pinning: `"hooks": 1` in its `.hnmod`'s `horizon-native.json`, the `NATIVE_HOOKS_API` of that
file when it was made. Hook points are only ever added: each addition raises `NATIVE_HOOKS_API`, and a mod builds
on any Horizon whose version is the one it needs or a later one. Its `horizon` is then only the commit it was tested on. The launcher builds it on the commit the other mods
pin, else on the installed version's commit (the game the player plays), after checking that commit's
`NATIVE_HOOKS_API` (read from GitHub) is at least the mod's; when it isn't and nothing pins, it builds on the commit the
mod was tested on. In its own copy of the mod it sets `horizon_revision` to the commit it builds on. Horizon's
own native mods (Zoom, FPS counter, Keystrokes, Minimap; horizon-rust/horizon-mods, `native/<name>/`) are made this way, so any of them build together
with no re-release for each Nightly. The `mods` feature (WebAssembly mods) is passed to the build only when the source has it.

API 2 adds the item panel beside container screens and its recipe pages (`native_panel_items`,
`native_panel_recipes`, and the hook `native_panel_event` for the player's clicks, searches and keys over items), and
the game's items and recipes (`native_recipes`, `native_known_recipes`, `native_place_recipe`, `native_give`): what a
recipe viewer needs, in builds with or without the `mods` feature. A WebAssembly mod's panel shows only while no native
mod shows one.

API 3 adds authoritative world lifecycle events through `native_world_lifecycle`: open ready, open failed,
and final close. Bundles that use these events need `"hooks": 3`.

API 4 adds authoritative server tick events through `native_server_tick`: start and end of each non-paused
server tick, with `world_running`. A frozen world reports false except during a requested step. Bundles that
use these events need `"hooks": 4`.

API 8 adds the mod message channel and the server event hooks: `native_net_from_client` (a message a player's
client half sent — the data is untrusted), `native_net_from_server` (a message the server half sent the
window), `native_player_hurt` and `native_entity_hurt` (before armour; cancel, or take from, the amount),
`native_player_died`, `native_mob_spawned`, `native_mob_killed`, `native_entity_interact` (claim the use),
`native_item_dropped`, `native_item_picked_up` (cancel) and `native_block_breaking` (cancel: the block
stands). The helpers beside them send on the channel (`native_net_send_to_player`,
`native_net_send_to_server`, in every build), spawn and steer mobs, send particles and sounds, set redstone
output, show displays, read and move fluid tanks, and publish on the compiled-in mods' bus
(`native_mod_publish`); every helper but the net sends needs the `mods` feature. The new catalog entries also
carry `side`, `cancellable` and the NeoForge event they mirror in `neoforge`. Bundles that use these need
`"hooks": 8`.

## What the launcher does

In the window: install a native mod from the Mods page. The first one shows a notice once (the first build downloads
Rust and can take 20 to 30 minutes; later builds are faster) and turns on Settings → Game → **Native mods
(experimental)**, which stays off for players who never install one; `mods install` does the same. Then **Build**. The launcher names the mods, warns in red, and builds only from "I trust these mods: build"; the
row shows each step, the error if one fails, and the build the instance plays. From the command line:
`horizon-launcher native-build [FOLDER] [--instance NAME] --trust-native-code`. FOLDER is a bundle, an unpacked
`.hnmod`, or a folder of them; without it, the native mods turned on in the instance. It is not an MCP tool:
trusting native code is the player's decision, never an agent's.

1. **Rust.** The launcher's own toolchain ([`rust_toolchain`](../launcher/src/rust_toolchain.rs)): Rust 1.99.1's
   official `rustc`, `cargo` and `rust-std` (and `rust-mingw` on Windows), each pinned by SHA-256, downloaded once
   from static.rust-lang.org into `<launcher data>/rust/`, with its own Cargo home `<launcher data>/cargo/`. No
   rustup, nothing on the system; the player's own Rust is never used. Windows uses the `windows-gnu` host, which
   links without Visual Studio. `rust-mingw` only links, so on Windows the toolchain also brings MinGW-w64
   (`rust_toolchain::MINGW`: WinLibs' GCC 16.2.0 on MinGW-w64 14.0.0, MSVCRT like `rust-mingw`, one zip pinned by
   SHA-256, its `mingw64/` without the documentation) on the build's `PATH`: `ring`, `hidapi` and wasmtime compile C,
   and `windows-sys`' `raw-dylib` imports need `dlltool` and the assembler it runs. The linker is the system's on
   macOS (the Command Line Tools: the launcher asks for `xcode-select --install`) and Linux (`cc`).
   `horizon-launcher rust status|install|remove`.
2. **horizon-mixins**, at the commit the mods name (their `.hnmod`'s `mixins`), else `native_mods::MIXINS_REV`,
   built once with that cargo (`cargo install --locked --git … --rev …`) into `native/tools/<commit>/`.
3. **The source** of the pinned commit: GitHub's archive of it, once per commit (`native/source/<commit>/`, about
   63 MB unpacked), with the commit in `.horizon-revision`. Never the player's own checkout. Links in the
   archive are left out: the tool refuses them and nothing needs them to build.
4. **Mixins.** The bundles are copied first, so the build and its key use that copy whatever happens to the
   folder meanwhile. `horizon-mixins apply` stages the modded source; the launcher copies only the files that
   changed into `native/stage/`, so Cargo, which goes by file times, recompiles only the crates the mods touch.
5. **Compile**: `cargo build --locked --profile release --features horizon-client/mods -p horizon-client`, the
   release's own command (game-build.yml), into `native/target/`. Cargo downloads the locked crates from
   crates.io (Horizon has no git dependencies). The log is `native/build.log`; a failure shows its last lines (a
   horizon-mixins diagnostic in one line).
6. **Install**: a new version, `native-<commit>-<key>`, a copy of the starting version's folder (resources
   included, cloned on APFS, Btrfs and ReFS) with the built game in place of its own, and a `native-mods.json`
   saying which commit, mods, horizon-mixins and Rust made it. The instance the mods are installed in plays it (a
   folder built from the command line, or mods of the default home, go to the **Native mods** instance). That
   instance gets each mod's settings from the default home the first time it plays it (its
   `horizon-mods/.horizon-mod-data/<id>/` folder and `key.<id>.` lines of `horizon-mods.txt`, where the mod's
   WebAssembly version kept them); nothing it already has is overwritten.

## Cache and disk

The key hashes the commit, the horizon-mixins commit, Rust's version, the profile and features, and every file of
every bundle. The same set on the same commit is never built twice. A change gives a new build, and
`native/target/` stays, so a rebuild compiles only `horizon-client` and what the mods touched.

Measured 2026-10-28 on the Mac mini (M4 Pro, 14 cores) with Veinminer 0.3.0 on Nightly `83f15fa`,
`CARGO_BUILD_JOBS=8` while other agents' builds ran (upper bounds):

| What | Size, time |
|---|---|
| Rust 1.99.1 (`<launcher data>/rust/`) | 453 MB; 58 s to download and unpack |
| horizon-mixins (`native/tools/`) | 7 MB; built once in 19 s |
| Crates in the launcher's Cargo home (`<launcher data>/cargo/`) | 289 MB |
| Source of one commit, and the stage | 64 MB each |
| First build: release profile, the whole game | 9 min 10 s of compiling, 9 min 41 s in all; `native/target/` 1.5 GB |
| Rebuild after a helper's code changed | 2 min 46 s of compiling (only `horizon-client`), 2 min 53 s in all |
| A change horizon-mixins ignores (a comment) | nothing compiled; 31 s, mostly the version folder's copy |

Measured 2026-10-29 on Windows 11 (Ryzen 7 9800X3D, 16 threads), the same mod and Nightly, an empty `native/target/`
(horizon-mixins, the source and the crates were already downloaded):

| What | Size, time |
|---|---|
| Rust 1.99.1 and MinGW-w64 (`<launcher data>/rust/`) | 1.7 GB, MinGW-w64 875 MB of it |
| Crates in the launcher's Cargo home | 657 MB |
| First build: release profile, the whole game | 6 min 57 s of compiling, 8 min 3 s in all; `native/target/` 2.0 GB |

Measured 2026-10-28 on Linux (an x86_64 server with no Rust of its own), Zoom on Horizon `1149c92`, 3 jobs while CI
ran: `rust install` in 11 s (592 MB), the first build in 12 min 9 s. The built game ran Zoom's proof (under `xvfb-run`:
the game needs a display even with `--offscreen`).

Only the native builds an instance plays are kept (`native_mods::prune`); `native/target/` stays for the next one.

## Trust

- **Off until the player installs one.** Settings → Game → Native mods stays off for a player who never installs a
  native mod; the first install shows the notice and turns it on. The command needs `--trust-native-code`; the window
  shows a warning in red before every build: the mods run with full access to the computer, and their ids and
  versions. The Mods page says who made each one, and "Not made or endorsed by the Horizon project" for any mod not
  Horizon's own.
- **The registry** (horizon-rust/horizon-mods) lists them as their own kind, `native-mod`, only after a maintainer
  has read the source: the `.hnmod` contains `.rs` and `.toml` files and may contain data files under
  `mixins/<id>/data/`. The launcher checks the bundle without running anything. Native code is trusted.
  The file is pinned by SHA-256 like every registry file. An update is reviewed again; `revoked.toml` takes one down.
  A WebAssembly mod's entry may turn `native-mod` once (never back): its `.hmod` versions stay published and
  are checked as `.hmod`s, the index lists only its `.hnmod` versions, and the launcher's update keeps the
  player's `.hmod` (the game runs it until a native build has the mod, then skips it); removing the mod removes both.
- **Isolation of data.** Native mods run only in the instance built for them, with its own worlds, options and logs.
- What this does not do: sandbox the compiled game. horizon-mixins' checks stop mistakes, not a malicious helper
  ([security notes](https://github.com/horizon-rust/horizon-mixins/blob/main/docs/security.md)).

## Multiplayer

- **Single player and LAN host:** the mods run in the host's game and its integrated server, so a server-side
  hook applies to every player of the world.
- **Joining a server:** only client-side hooks do anything; the protocol is unchanged.
- **Dedicated servers:** native mods are not built yet. Lifecycle hook call sites are ready for when the pipeline builds them (`--no-default-features`).

## Left to do

- Rebuild after the mods or the Nightly change (asking first, as it compiles for minutes).
- A prebuilt horizon-mixins from its CI instead of building it on the player's computer.

## Named hook catalog (draft)

`native-hooks.json` maps stable hook names and contract versions to the current
source targets. A contract includes the signature, call timing and behavior.
Keep an existing contract or add a new version when it changes incompatibly.
The catalog does not cover helper methods or arbitrary private game code.
Named injection resolution needs a matching horizon-mixins change; existing raw
targets and the `hooks` metadata continue to work as before.

### Reading server blocks (draft)

A native server tick hook can call `native_server_block` with its context, a dimension id and a block position. It returns the block name and state properties from a loaded chunk. It does not load a chunk or open a dimension. Reads outside that hook, from a stale context, from unavailable dimensions or outside the world bounds return an error.

### Neighbour observations (draft)

`native_neighbor_changed` observes each authoritative server `neighborChanged` before the block handles it. It supplies the dimension, position, current state ID, changed block ID and moved flag. Resolve IDs with the level's registries during the hook; the hook does not load chunks or change the update queue.

### Server block changes (draft)

`native_block_changed` observes a committed authoritative server `Level.setBlock` change before block side effects and neighbour updates. It supplies the dimension, position and old and new state IDs. It does not observe chunk loading or direct storage writes that bypass `Level.setBlock`.

### The mod message channel (API 8)

A mod in two halves — one in the window, one beside the world — talks over the game's own wire: a channel
the mod names, and bytes. `native_net_from_client` hears what a player's client half sent (everything in
`data` is the player's own bytes: untrusted, checked by nothing before the hook), `native_net_from_server`
hears what a server half sent the window. The helpers `native_net_send_to_player` and
`native_net_send_to_server` send the other way; they work in every build, with or without the `mods`
feature. A channel is 1 to 64 bytes, without whitespace, and not in the game's `minecraft:` and `horizon:`
namespaces; a payload is at most 30 000 bytes; at most 32 messages a player a tick each way, the rest
dropped in silence. Only a client with the `horizon` brand is sent one — a vanilla client is sent nothing.
The wire is the `horizon:mod_net` payload the compiled-in mods use, so both halves of one mod — a
compiled-in one and a hooked one — share a channel.

### Server event hooks (API 8)

The event hooks fire on the authoritative server, before the compiled-in mods are told, in every build.
Cancellable ones take `amount: &mut f32` / `canceled: &mut bool` or a `claimed: &mut bool`; several mods
share a point through those. `native_player_hurt` and `native_entity_hurt` run before the armour and the
rest of vanilla (a cancelled hurt deals none; what the hooks leave, the armour and the mods then work on);
`native_block_breaking` cancelled keeps the block standing — no drop, no tool wear, no statistic;
`native_item_picked_up` cancelled leaves the stack on the ground; `native_entity_interact` claimed keeps
the vanilla use from running. `native_player_died` carries the death message's cause, `native_mob_spawned`
fires for every mob the server spawns (`natural` when the world did it by itself),
`native_mob_killed` names the killer, `native_item_dropped` the stack. Players are named by their UUID
(the `native_acting_player` helper reads the one acting); blocks by position and state id.

### Mob, effect, redstone, display and fluid helpers (API 8)

`native_spawn_entity` spawns a mob of a kind the game knows with the `NativeSpawn` options (it belongs to
the helper's own `native` mark, as a compiled-in mod's mobs belong to it, and counts against the same
1024-mob budget), `native_entity_teleport`, `native_entity_push`, `native_entity_set_health` and
`native_remove_entity` steer the helper's own mobs. `native_spawn_particle`, `native_play_sound` and
`native_play_sound_to` send what the compiled-in `effects` capability sends, under the same caps.
`native_redstone_set_output` sets what a block gives out on each face (down, up, north, south, west, east;
`strong` powers through, as a lever does) and the game updates the neighbours.
`native_show_text_display`, `native_show_block_display`, `native_show_item_display`,
`native_move_display` and `native_hide_display` show the same display entities the compiled-in
`displays` capability sends. `native_fluid_tanks`, `native_fluid_insert` and `native_fluid_extract` read
and move the shared fluid tanks (millibuckets; a tank's own limits hold; nothing is created).
`native_mod_publish` puts a message on the compiled-in mods' `inter-mod` bus. All of these need the
`mods` feature: a build without it has none of the state they touch.
