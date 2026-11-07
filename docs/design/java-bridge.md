# The Java bridge: Fabric 26.3 mods in Horizon

A design note for taking the Java bridge into main. The bridge is by
[iaxivers](https://github.com/iaxivers) (Windows support by [martipops](https://github.com/martipops)), who
maintains it. It comes from the fork [iaxivers/horizon-java-bridge](https://github.com/iaxivers/horizon-java-bridge/tree/java-bridge),
where its full documentation is: `docs/java-bridge.md`, the hook-by-hook list `docs/java-bridge-upstream.md`, costs
in `docs/java-bridge-perf.md`, the mod survey in `docs/java-bridge-survey.md`.

What it does: a player turns on "Fabric 26.3 mods (beta)" for an instance and plays real Fabric 26.3 mod jars,
unchanged, in Horizon. Horizon stays the game (world, ticks, physics, rendering, saving, networking). A Java VM in
Horizon's process runs Fabric Loader, Fabric API, the mods and their mixins against the real 26.3 classes from the
player's own install, and only where a mod changes something. Off, nothing of it runs.

## 1. Where it lives

Its own files, all new:

| Path | What | Lines (fork, 2026-11-05) |
|---|---|---|
| `crates/horizon-jvm/src/` | Rust core: JVM start (libjvm loaded at run time, JNI), events, mods' content as Horizon content, the headless client's GPU device | ~9,600 |
| `crates/horizon-jvm/java/platform/` | The platform: a Fabric mod (`horizon`) that mirrors Horizon's world to the mods' server and client. Compiled on the player's machine against their own game jar | ~30,700 |
| `crates/horizon-jvm/java/dev/`, `build.rs` | Boot (`horizon-bridge.jar`; a release build carries its sources and compiles them on the player's machine, as the platform's) | ~900 |
| `crates/horizon-jvm/{fabric-libs,fabric-api}.tsv` | Fabric Loader, its libraries and Fabric API: URLs and SHA-256 pins | 10 |
| `crates/horizon-jvm/java/fixtures/`, `tests/` | Test mods built from source, integration tests | ~4,400 |
| `crates/horizon-client/src/game/java_*.rs` | Gameplay side: the server tick, actions, screens, entities, worldgen, guests | ~9,900 |
| `<file>/java.rs` (42 files) | Child modules next to the file they hook (a child sees its parent's private items, so nothing in Horizon's file becomes `pub`) | ~4,600 |
| `launcher/src/java_variant.rs`, `java_mods.rs`, `java_mods_page.rs`, `java_release.rs` | The switch, the `-java` build's install, the Java mods tab | ~2,000 |
| `tools/java_bridge/` | Headless tests (`regress.py`, `screen_test.sh`, `lan_test.sh`, `install_test.sh`...) | dev tools |

**Hooks in Horizon's own files:** 141 files, +1,835 / −438 lines against main's `73d3ee3d2`. Each is one call, a
field, a `mod` line or an enum arm (`Screen::JavaMod`, `HomeTab::JavaMods`...). The biggest: `lan_actions.rs`
(guest packets, +91), `worldgen/generator.rs` (noise-only chunks, +50), `mod_content.rs` (mods' own block states).
The lower crates (`horizon-world`, `-core`, `-protocol`, `-client-render`) get generic hook points with no Java in
them: `LevelAccess::foreign_fluid*`, a block-write hook in `world.rs`, chunk tickets, mod tints and fluids in the
mesher, mod biomes. The full table, file by file, with why each can't live elsewhere:
[docs/java-bridge-upstream.md](https://github.com/iaxivers/horizon-java-bridge/blob/java-bridge/docs/java-bridge-upstream.md).

Not part of this: NeoForge 26.3 and Forge 1.12 (Cleanroom) work in the fork (`java/neoforge`, `java/platform112`).
Their loaders are LGPL; they stay in the fork unless agreed separately.

## 2. The PR split

Under ~500 lines each (the `horizon-jvm` subsystem PRs excepted, below), in this order. Every PR builds, passes clippy and tests **without** `java-mods`; from the
crate PR on, CI also builds with it.

1. **This note.**
2. **General fixes the bridge needed, useful without Java** (each alone, any order, ~100–300 lines): streaming (a
   proto saved over a loaded column; forced chunks' priority), content tables for big mods (state tables, own
   states), port-pack fixes, memory for big packs (`BakedQuad`, streamed gameplay file, `trim_heap`), structure
   templates and noise seeds from data packs, wide frames on the in-memory connection, small fixes, the ReplayMod
   viewer (`--replay`). Any of these upstream doesn't want stays in the fork.
3. **Generic hook points in the lower crates**, no Java, named `mod_*` rather than `java_*` where generic (~400).
4. **`horizon-jvm`, behind the `java-mods` feature**, in the subsystem PRs of the table below (Rust core and Java
   platform together, each a few thousand lines at most, in dependency order so every one compiles and passes its
   tests). New files only, never linked into the plain build; Ludovic decides whether these may be bigger than the
   usual 500 lines, with CODEOWNERS `@iaxivers`.
5. **Gameplay**, by area, each with its own hooks: server tick and actions · screens and HUD · entities · worldgen
   · LAN guests · remote client · device mods.
6. **Launcher:** the switch, the Java mods tab, the `-java` build's install, 7 `tr()` keys in each language.
7. **CI:** Java 25, the `java_mods` input of `game-build.yml`, headless cases from `regress.py` with the fixture
   mods, the no-mod-code check (section 5).
8. **Docs:** `docs/java-bridge.md` (without the fork's task history), the testers' guide, a README section with
    the credits line: "Fabric mods bridge by iaxivers (Windows support by martipops), maintained by iaxivers."

### `crates/horizon-jvm` by subsystem

Lines counted on the fork's `java-bridge` (2026-11-05), Rust and Java; the fixture mods and integration tests go
with the PR they test. `P/` is `java/platform/dev/horizonlab/platform/`. Tests marked *(game)* need a prepared 26.3
and run on a developer's machine or `regress.py`, not on CI.

| # | Subsystem | Files | Lines | Tests |
|---|---|---|---|---|
| J1 | Crate, build, Fabric's pins, ownership gate, player-side compile | `Cargo.toml`, `build.rs`, `fabric-{libs,api}.tsv`, `src/{libs,runtime,gate,platform}.rs`, `java/dev/` (boot, game provider), `java/platform/*.json` | 2,049 | unit `gate::`, `runtime::`; `tests/boot.rs` (JVM up, the bridge compiled from its sources); `check_no_fabric.py --tree` |
| J2 | JVM start, natives, Fabric start | `src/lib.rs`, `P/HorizonLog`, `HorizonServices`, `HorizonFailures`, `HorizonProfile`, `HorizonJson`, `mixin/PlatformPlugin`, `mixin/mods/ModsPlugin` | 2,824 | `tests/fabric.rs` (fixtures `loadermod`, `failmod`, `hellomod`) |
| J3 | Rust→Java calls, the world view, facades | `src/events.rs`, `src/world.rs`, `src/facade.rs` | 3,792 | unit `world::`, `facade::`; `tests/facade.rs`, `facade_stress.rs` (fixture `testmod`) |
| J4 | Server mirror | `P/HorizonServer.java`, the server accessors (`mixin/*Access.java`) | 3,810 | *(game)* `tests/real_game.rs`; regress `pot`, `terminal` |
| J5 | Server mirror's mixins | `mixin/` server, level, chunk map, player list, game mode (`MinecraftServerMixin`, `ServerLevelMixin`, `LevelMixin`, `ServerChunkCacheMixin`…) | 1,207 | regress `placed`, `lost`; fixture `eventsmod` |
| J6 | World state | `P/HorizonBlockEntities`, `Ticks`, `Light`, `Chunks`, `ChunkLoads`, `Tickets`, `Items`, `Hoppers`, `Fluids`, `Neighbours`, `VanillaActions`, `SavedData`, `Functions`, `Combat`, `Data`, `DataFixer` + their mixins | 3,969 | regress block entity, hopper, ticket cases; fixture `dev` |
| J7 | Mods' hooks and the mixin table | `P/HorizonHooks`, `HorizonMixinTable`, `HorizonMixinInit` | 2,317 | fixture `mixinmod`; regress `--code-check` (no mod's packages) |
| J8 | Mods' content as Horizon content | `src/content.rs`, `src/renderers.rs`, `P/HorizonContent` | 869 | `tests/content.rs`, `tests/modrinth.rs` (offline), unit `renderers::` |
| J9 | Headless client | `P/HorizonClient.java` | 4,144 | `tests/client_side.rs`; fixture `keysmod` |
| J10 | Client state and its mixins | `P/HorizonClientState`, `ClientBlockEntities`, `ClientCommands`, client mixins (`ClientLevelMixin`, `CameraMixin`, `MouseHandlerMixin`…) | 933 | fixture `configmod`; regress client cases |
| J11 | Mods' networking | `P/HorizonLoop`, `HorizonPacketTap`, `HorizonConfiguration`, networking mixins | 1,835 | fixture `joinmod`; regress LAN and server cases |
| J12 | Screens, HUD, textures | `P/HorizonGraphics`, `ScreenEvents`, `Textures`, `Sprites`, `ItemModels`, `LazyModels`, `ClientConfig`, `ModMenuApi`… + GUI mixins | 3,539 | regress screen cases (`pot`, recipe viewers) |
| J13 | Mods' entities | `P/HorizonEntities`, `HorizonEntityRender`, `HorizonSpawner`, `HorizonPackets`, `Particles`, `Sounds` + mixins | 3,357 | fixtures `crittermod`, `cartmod`, `geckomod`; regress `entity`, `spit` |
| J14 | Mods' worldgen | `P/HorizonWorldgen`, `NoiseExport`, `Surface`, `Biomes`, `BiomeMemo`, `Tints` + worldgen mixins | 2,497 | fixture `geomod`; regress worldgen cases |
| J15 | Mods' GPU device | `src/gpu/` (`mod.rs`, `spirv.rs`), `P/HorizonGpu` | 3,223 | unit `gpu::`; fixture `kineticmod` |
| J16 | Device mods (voice chat) | `src/device.rs`, `P/HorizonDeviceMods`, `HorizonDeviceClient` | 1,008 | unit `device::`; `tools/java_bridge/voice_test.sh` |

About 41,400 lines in 16 PRs, plus 2,634 lines of fixture mods and 1,745 of integration tests spread over them
(counted on the fork's `bridge-ownership-1`). J4 and
J9 are one file each, past 3,000 lines; if that's too big, they are split into classes first, on the fork. Not
included: `src/{neoforge,boot112}.rs` and `java/{neoforge,platform112}` (the NeoForge and Forge 1.12 prototypes stay
in the fork).

Commits carry `Co-Authored-By: iaxivers` when someone else lands them. The fork merges main daily; a bridge failure
never blocks another merge.

## 3. Off by default

- **Cargo feature `java-mods`** (optional dependency `horizon-jvm` of `horizon-client`). Without it, `horizon-jvm`
  isn't compiled; hooks fall back to `java_mods/off.rs`, which answers "none". Upstream's normal release stays this
  build.
- **Launcher switch**, per instance, off by default: "Fabric 26.3 mods (beta)" on Home's play row. Off: the plain
  build, no Java mods tab, `--java-mods` never passed, whatever `mods-java/` holds.
- **Even the `-java` build** starts no Java unless the launcher started it with an owner record the game confirms
  (below); the log says "Java mods: off (N jars in mods-java)". The developers' opt-in (`HORIZON_JAVA_MODS=1`)
  exists only in builds with the `java-mods-dev` feature, which no release enables; the NeoForge and Forge 1.12
  prototypes only with `java-mods-prototypes`: the release build is Fabric only.
- **A warning first:** turning the switch on shows, once per instance, that mods are Java programs with full access
  to the computer and the Minecraft account, to add only mods you trust, and that it is a beta.

### Ownership, in plain words

The maintainers' conditions (a) and (b), and what the fork does about them (fork `bridge-ownership-1`,
`bridge-gate-2`):

- **You can't play Fabric mods in Horizon without owning Minecraft.** They run only when Horizon's launcher starts
  the game for a signed-in Microsoft account that owns Java Edition, and the game checks it again itself:
  1. Before Play, the launcher's owner gate (the same check plain Horizon uses) asks the Minecraft services, with
     the account's token, whether it owns Java Edition.
  2. With Fabric mods on, the launcher then writes an **owner record** for this one start: that account's profile
     UUID and Minecraft token, in a fresh file only the player can read, in the launcher's folder (the session
     file it already hands the game, written for the owner), and passes its path to the game.
  3. The game reads the record once, as its process starts, and deletes it. It must be under ten minutes old (an
     instance's pre-launch command may run first) and name an account in the launcher's `accounts.json`. Then the game asks the Minecraft services itself with that token: the profile must
     be that UUID and the entitlements must include Java Edition. Only a token Mojang issued to an owner passes.
- **Setting things by hand runs no mods and downloads nothing.** `horizon-client --java-mods` without the
  launcher, the old "launcher word" variable, or a record written by hand all end the same way: the game says
  "Fabric 26.3 mods run only when Horizon's launcher starts the game for a signed-in account that owns Minecraft:
  Java Edition…" with the reason. With mods in `mods-java/` it then stops, as when mods fail to start, rather than
  open a modded world without them; with none it plays. Nothing is fetched: no Fabric, no Java, nothing of Mojang's.
- **Why a token and not the launcher's stored answer:** the launcher keeps "this account owns the game" in the
  OS keychain; anything the game could read without it (a file, a variable) a player could write too. The token is
  the one proof the game can check without inventing crypto. The cost: Fabric mods need the services reachable when
  the game starts (plain Horizon still plays offline).
- **The game never downloads anything of Mojang's.** No Java runtime, no game jar, no assets, with or without mods.
  The game jar is the one the player's own install prepared, as in main.
- **The launcher downloads only after the owner gate passes**: the `-java` game build, Mojang's Java 25 runtime
  (for the mods) and, through the game it starts, Fabric's jars. With no owner signed in, the switch and Play say
  "Sign in…" and nothing is downloaded. An installed `-java` build is not replaced at every Play; a new version
  brings its own, or the player asks (`java-mods update`).
- **Developers keep their harnesses** in their own builds only: `java-mods-dev` adds `HORIZON_JAVA_MODS=1` and a
  stand-in for the Minecraft services, for the bridge's test scripts; release builds have neither.
- **Horizon never ships, copies or reads Fabric Loader, Mixin, Sat4j, ASM or Fabric API.** None of their files is
  in the repository or a release. The release build carries only Horizon's own Java sources (the bridge and its
  platform mod) and compiles nothing against Fabric there: the build machine fetches none of Fabric's files. On the
  player's machine, after the gate, the game fetches them from Fabric's own server (`maven.fabricmc.net`), checks
  their pinned SHA-256, runs them unchanged and compiles Horizon's sources against them there.
- **These are checks, not DRM.** As in every launcher, they keep Horizon from offering a way around ownership;
  they can't stop someone who edits the program.

How it's checked: unit tests of the game's decision (`horizon_jvm::gate`: variables set by hand, stale, shared or
unknown records → no mods before any network call; the services' answers, against a local stand-in) and of the
launcher (the record only from a fresh "yes", private, on Play's command); a test that the release builds enable
`java-mods` alone; regress `direct`, `forged_word`, `forged_record` (a scratch install started without a real
owner: the message, no Fabric or Java fetched, the game stops before opening the world) and `owner` (a record a
stand-in services accepts: Fabric fetched there, the mod loads); `check_no_fabric.py` on the repository and on every
release archive, and the release job fails if Fabric's files appear on its machine.

### The cost when off (condition c)

Measured with `tools/java_bridge/perf_off.sh` (`--benchmark standard`, release builds, alternating runs, one
machine): upstream main; the fork built without `java-mods`; the fork built with it as the release builds it, the
switch off and a jar in `mods-java/`. Compared: startup to the title screen and to in-world, average FPS, frame p50
and p99, 1% lows, peak memory, binary size. With the switch off, the game reads its arguments once and lists
`mods-java/` once; no JVM, thread or library is loaded.

Machine: AMD Ryzen 5 9600X, AMD Radeon RX 9070 XT (RADV, Vulkan), Fedora 43 (kernel 6.19.12). Upstream
a27b7f20f, fork 8cff8dbc4. `--benchmark standard`, mean ± stdev over 5 runs each (frames: over every scene's
flight). Measured 2026-11-05 with `tools/java_bridge/perf_off.sh 5`.

| | Upstream main | java-bridge, no feature | java-mods build, switch off |
|---|---|---|---|
| Runs | 5 | 5 | 5 |
| Start to title (ms) | 772 ± 259 | 687 ± 8 | 676 ± 22 |
| Start to in-world (ms) | 2678 ± 300 | 2492 ± 68 | 2462 ± 110 |
| Average FPS | 3699 ± 338 | 3707 ± 383 | 3597 ± 379 |
| Frame p50 (ms) | 0.25 ± 0.02 | 0.25 ± 0.02 | 0.25 ± 0.02 |
| Frame p99 (ms) | 0.74 ± 0.13 | 0.74 ± 0.16 | 0.77 ± 0.16 |
| 1% low FPS | 856 ± 161 | 847 ± 161 | 827 ± 169 |
| Peak RSS (MB) | 1119 ± 11 | 1116 ± 15 | 1114 ± 12 |
| Binary (MB) | 89.9 | 87.9 | 93.4 |

**Reading it:** every difference is inside one standard deviation, so with the switch off the bridge costs
nothing measurable in start time, frame time or memory. The only real cost is the binary: +3.5 MB for the
`java-mods` build (the bridge's code and its Java sources). The plain build (no feature), which is what upstream
releases ship, is the same as main.

Hot-path hooks (mesher, block writes) post their own before/after with their PR.

## 4. Licences

Checked from the pinned jars themselves (`fabric-libs.tsv`, `fabric-api.tsv`, SHA-256 verified), 2026-11-05:

| Component | Version | Licence | Source of the fact |
|---|---|---|---|
| Fabric Loader | 0.20.1 | Apache-2.0 | `LICENSE_fabric-loader`, `fabric.mod.json` |
| ↳ shaded: tiny-remapper, mapping-io, class-tweaker (FabricMC), Gson | | Apache-2.0 | |
| ↳ shaded: Sat4j (dependency solver) | | EPL-1.0 or LGPL-2.1 | `about-sat4j.html` |
| MixinExtras (nested in the loader) | 0.5.5 | MIT | `LICENSE_MixinExtras` |
| Sponge Mixin (Fabric's) | 0.17.5+mixin.0.8.7 | MIT | `LICENSE.txt` |
| ASM, asm-analysis/commons/tree/util | 9.10.1 | BSD-3-Clause | `Bundle-License` |
| Fabric API, all 44 modules | 0.165.0+26.3 | Apache-2.0 | each module's `fabric.mod.json` |
| Rust crates of `horizon-jvm` | | all permissive | `cargo metadata`: all but `arbitrary` 1.5.0 (MIT or Apache-2.0) already in main's `Cargo.lock` |

**Sat4j is the one non-permissive piece.** It is inside Fabric Loader's own jar, which Horizon runs unchanged, as a
launcher runs it. Nobody reads, copies or modifies it, and Horizon doesn't ship it. Ludovic to confirm this is
acceptable.

**Nothing in this list is in the repository or in a release.** The loader, its libraries and Fabric API are
downloaded on the player's machine after sign-in, from `maven.fabricmc.net`, and checked against the pins; the
release build machine fetches and reads none of them (section 3, "Ownership"). The
bridge's own code is original (MIT, as Horizon). **Mods keep their own licences and are never shipped**: the player
adds the jars, the bridge runs them as Fabric does, and nobody reads their source.

## 5. No mod-specific code

Nothing in the bridge targets one mod's internals. It provides Fabric's platform and vanilla's classes; a mod that
fails reveals a generic gap, fixed for every mod.

`python3 tools/java_bridge/regress.py --code-check` enforces it: it scans the bridge's code (`crates/horizon-jvm`
without fixtures, the game's `java_*` files), strings included (`Class.forName`, mixin targets), for any mod's
package root. The roots: a list of 39 known mods (`KNOWN_PACKAGES`), plus every jar passed with `--jars` or `--pack`
(their entrypoints and mixin packages, nested jars too). One hit fails. A comment may name a mod; code may not. The
one exception is a mod's public API the bridge stands in for when the mod is absent (`ALLOWED_APIS`, each with a
reason; today only Mod Menu's `com.terraformersmc.modmenu.api`). CI runs it without jars; the full run with jars
runs before each bridge PR.

## 6. Status: a beta, nowhere near finished

What works, from a survey of the 60 most-downloaded Fabric 26.3 mods on Modrinth (each alone with its
dependencies, headless):

- **39 work cleanly**: libraries, Mod Menu, Cloth Config, Xaero's maps, JEI, REI, Jade and others.
- **16 load and play with one feature missing.**
- **4 left out on purpose**: renderer replacements (Sodium, Iris...); Horizon has its own renderer.
- **1 stops the game** (Essential).

Content mods also play (Farmer's Delight, Biomes O' Plenty, mod mobs, machines and their screens), an 81-jar pack
plays on a desktop, LAN and dedicated servers work, Linux and Windows. Create is in progress.

**But some mods won't work, and some features are broken.** Mods cost what they cost on Fabric: the 81-jar pack
starts in ~3 minutes and uses ~4 GB on a 2-core box. Most failures are still unknown: it needs beta testers to find
which mods and features fail.

**Reporting.** One bug per report, as an issue on
[iaxivers/horizon-java-bridge](https://github.com/iaxivers/horizon-java-bridge/issues) (it comes to the maintainer,
not to Horizon's queue), with:

1. What happened and what was expected, and the steps if known.
2. The mod list: `horizon-launcher java-mods list`, the Java mods tab, or the modpack's name.
3. `logs/latest.log` from the game folder (replaced at each start: copy it first). The tab also marks the mods
   that failed in the last game from it ("[Java mods] <mod> failed in …").
4. A screenshot (F2) and coordinates (F3) for anything visible; the newest `crash-reports/` file after a crash.

## 7. What upstream provides

- **The `-java` game build in releases**: `game-build.yml` with its `java_mods` input on (Linux and Windows;
  `horizon-client` plus licences, built with `HORIZON_BRIDGE_PLATFORM=source`, since CI has no Minecraft jar), signed
  with the release key like the plain build. Until then, the launcher takes the fork's signed `java-mods`
  pre-release instead (`java_release.rs`, keys in `launcher/java-mods-keys.txt`), which upstream would need to accept.
- **The Microsoft sign-in client ID**: the fork's launcher already uses Horizon's Azure app
  (`HORIZON_MSA_CLIENT_ID`, the repository variable `launcher.yml` reads), with Ludovic's OK. Upstream keeps
  providing it; nothing new is registered.

## 8. The ask: an open beta in Horizon's launcher

The bridge needs testers to find what fails, and testers come from Horizon's own launcher, not from a fork's
download page. So the proposal is to put the **launcher's switch** in early, before the game code:

1. **First code PR: the launcher side** (`launcher/src/java_variant.rs`, `java_release.rs`, `java_mods_page.rs`,
   the switch on Home's play row, `launcher/java-mods-keys.txt`). Off by default, labelled beta. It downloads
   nothing and starts no Java before the owner gate (section 3).
2. **With it in a release**, a player turns on "Fabric 26.3 mods (beta)" and the launcher installs the fork's
   signed `-java` build, as `java_release.rs` already does. That build includes the owner check for a game
   started directly (regress `direct`). Upstream releases ship no `-java` build until the code is in main, as
   point 4 of the maintainers' plan asks.
3. **An announcement**: "Fabric 26.3 mods are in the launcher as an open beta: turn them on, play your mods,
   report bugs." Reports go to the fork's issues (section 6), not to Horizon's queue.
4. **The game code follows** (section 2, hooks first, then J1…J16), and upstream's own `-java` build replaces the
   fork's once it is in main. The launcher then stops reading the fork's release.

What upstream would accept by merging step 1: the launcher trusts one more signing key (the fork's, for the
`-java` build only) until its own releases carry that build. The key, the signed manifest and the checks are in
`java_release.rs`, with tests.
