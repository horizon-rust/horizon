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
| `crates/horizon-jvm/java/dev/`, `build.rs` | Boot (`horizon-bridge.jar`, embedded in the binary) | ~900 |
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

Under ~500 lines each, in this order. Every PR builds, passes clippy and tests **without** `java-mods`; from the
crate PR on, CI also builds with it.

1. **This note.**
2. **General fixes the bridge needed, useful without Java** (each alone, any order, ~100–300 lines): streaming (a
   proto saved over a loaded column; forced chunks' priority), content tables for big mods (state tables, own
   states), port-pack fixes, memory for big packs (`BakedQuad`, streamed gameplay file, `trim_heap`), structure
   templates and noise seeds from data packs, wide frames on the in-memory connection, small fixes, the ReplayMod
   viewer (`--replay`). Any of these upstream doesn't want stays in the fork.
3. **Generic hook points in the lower crates**, no Java, named `mod_*` rather than `java_*` where generic (~400).
4. **`horizon-jvm`, behind the `java-mods` feature:** Cargo feature and CI job first, then the Rust core by file
   (JNI and start · events · content · GPU device).
5. **The Java platform**, by subsystem (server mirror · client · screens and graphics · entities · worldgen · mixin
   table · GPU), each in dependency order so every PR compiles. ~31,000 lines is ~60 PRs at 500: Ludovic decides
   whether new files under `crates/horizon-jvm/`, never linked into the plain build, may come in bigger PRs with
   CODEOWNERS `@iaxivers`.
6. **Fixtures and tests.**
7. **Gameplay**, by area, each with its own hooks: server tick and actions · screens and HUD · entities · worldgen
   · LAN guests · remote client · device mods.
8. **Launcher:** the switch, the Java mods tab, the `-java` build's install, 7 `tr()` keys in each language.
9. **CI:** Java 25, the `java_mods` input of `game-build.yml`, headless cases from `regress.py` with the fixture
   mods, the no-mod-code check (section 5).
10. **Docs:** `docs/java-bridge.md` (without the fork's task history), the testers' guide, a README section with
    the credits line: "Fabric mods bridge by iaxivers (Windows support by martipops), maintained by iaxivers."

Commits carry `Co-Authored-By: iaxivers` when someone else lands them. The fork merges main daily; a bridge failure
never blocks another merge.

## 3. Off by default

- **Cargo feature `java-mods`** (optional dependency `horizon-jvm` of `horizon-client`). Without it, `horizon-jvm`
  isn't compiled; hooks fall back to `java_mods/off.rs`, which answers "none". Upstream's normal release stays this
  build.
- **Launcher switch**, per instance, off by default: "Fabric 26.3 mods (beta)" on Home's play row. Off: the plain
  build, no Java mods tab, `--java-mods` never passed, whatever `mods-java/` holds.
- **Even the `-java` build** starts no Java without `--java-mods` (or `HORIZON_JAVA_MODS=1` for developers' scripts);
  the log says "Java mods: off (N jars in mods-java)".
- **Microsoft sign-in required.** Play with the switch on goes through the launcher's strict owner gate; the
  developer option `dev_offline` lets no Fabric instance through. **No offline path:** the fork's Prism wrapper
  and offline `--instance` play were removed.
- **No downloads before sign-in.** The `-java` build, Mojang's Java 25 runtime and Fabric's jars come only for a
  signed-in owner. The game itself downloads no Java. Mojang's files come from the player's install, as in main.
- **No cost when off**, to be measured before the gameplay PRs merge, on one machine, with
  `horizon-client --benchmark standard --render-distance 16` ([docs/benchmark.md](../benchmark.md), 5 repeats) for:
  main; the branch without the feature; the branch with the feature and the switch off. Compared: startup to the
  title screen and to in-world, average FPS, 1% and 0.1% lows, p99 frame time, resident memory, binary size.
  Differences must sit inside the 95% intervals. Also checked: no `libjvm` mapped in the process, no `.local/bridge`
  created (`tools/java_bridge/install_test.sh` with `OFF=1` already does both). Hot-path hooks (mesher, block
  writes) post their own before/after with their PR.

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
downloaded on the player's machine after sign-in, from `maven.fabricmc.net`, and checked against the pins. The
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
