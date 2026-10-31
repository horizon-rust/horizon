# Horizon: a map for agents

Minecraft: Java Edition 26.3 rewritten in Rust (wgpu). Vanilla is the specification wherever it has the
feature. [CONTRIBUTING.md](CONTRIBUTING.md) is how we work; this page is where things are.
For the most-changed areas, feature → files → harness: [docs/architecture.md](docs/architecture.md).

## Rules that bite

- **Mojang's files stay out of git**: game jars, assets, decompiled source, prepared resources (`.local/`).
  `tools/check_public_tree.py` rejects them; `git config core.hooksPath .githooks` runs it on commit.
- **Scratch homes for every run.** Any game or launcher you start gets fresh `mktemp -d` folders in
  `HORIZON_HOME`, `HORIZON_LAUNCHER_HOME`, `XDG_DATA_HOME` and `XDG_CONFIG_HOME`, plus
  `HORIZON_LAUNCHER_NO_SELF_UPDATE=1` and `HORIZON_NO_AUDIO=1`; the launcher also gets
  `HORIZON_LAUNCHER_TRASH`, `HORIZON_LAUNCHER_CLIPBOARD` and `HORIZON_LAUNCHER_DESKTOP` inside that home.
  Real player data (`~/.local/share/{applications,horizon,horizon-launcher}`, `~/.local/bin`) stays untouched.
- **Disclosure.** Everything an agent posts on GitHub ends with a `[!NOTE]` alert naming the model and harness
  that wrote it; that note is its only attribution. Take the model from your own context ("you are powered by
  X") and prettify the slug (`claude-opus-5-5` → Opus 5.5).
  - Pull requests and issues: the body opens with the problem, then what was done, and ends with
    `> [!NOTE]` / `> Filed by Opus 5.5 using Claude Code.`
  - Comments and reviews end with `> [!NOTE]` / `> This comment was written by an AI agent (Opus 5.5, Claude Code).`
  - Unknown model: `Filed by an AI agent using <harness>; model unavailable.`
  - Commit messages describe the change and nothing else.
- **Conventional commits**: every commit message and PR title is `<type>(<scope>): <description>`, with
  `type` one of `feat fix perf refactor docs test build ci chore revert` and `scope` the area or crate
  (`fix(mobs): stop zombies walking through doors`). PR titles become the release notes with the prefix
  dropped, so the description says what changed for players, not which code moved.
- Everything players run is Rust. Python and Java are for dev tools and harness drivers in `tools/`.
- Horizon-only features follow [docs/design-system.md](docs/design-system.md) and are off by default where
  they would change the vanilla look.
- **Small PRs** (< ~500 lines). Labels: `feature` (players get something new), `skip-release` (they see no
  change), none for a fix. `release-now` is the maintainer's call alone. Merges ship in batches of 5, or after
  6 hours ([docs/release-flow.md](docs/release-flow.md)).
- **Contributors first**: `gh pr list --state open --search <area>` before you start, and build on their open work.
  An issue assigned to someone else is theirs: pick another.

## Crates

Game workspace (root `Cargo.toml`); `launcher/`, `crates/horizon-telemetry-server` and
`crates/horizon-mod-registry` are separate workspaces.

| Crate | Owns |
|---|---|
| `horizon-client` | The game: a lib (`src/lib.rs`: gameplay, networking, title screens), the bin (`src/main.rs` → `game.rs` + `game/`), `horizon-ctl` (`src/bin/horizon-ctl/`), and the dedicated server (`--dedicated`) |
| `horizon-client-render` | The client's world data, rendering and screens (chunks, meshing, light, streaming, `gfx`, HUD, menus, settings), re-exported by horizon-client under the same paths. Reaches no gameplay |
| `horizon-client-core` | The client lib's bottom layer: `text.rs`, `lang.rs`, `snbt.rs`, `nbt_path.rs`, `component.rs`, `scoreboard.rs`, `bossbar.rs`, `gamerules.rs`, `block_entity_nbt.rs`, registry/particle/sound tables |
| `horizon-client-input` | Key bindings and names, toggle keys, mouse, controllers and glyphs, clipboard, window options |
| `horizon-items` | Blocks and items below gameplay: `blocks.rs` (block facts), `items.rs`, `item_data.rs` (components), `enchant.rs`, `cooldowns.rs`, `bundle.rs`, `item_nbt.rs` |
| `horizon-command` | Command parsing (Brigadier): the vanilla tree, argument types, selectors, suggestions, the tree's network form |
| `horizon-world` | Vanilla world data with no gameplay loop: NBT, regions, chunks, `datafix/`, `worldgen/`, `level/` (server-side block behaviour, ticks, redstone), entity physics, effects, food, damage |
| `horizon-core` | Simulation primitives: Java `Random`s, `jmath` (HotSpot-exact `Math`), positions, noise, density, paletted storage, `version.rs` (the pinned version) |
| `horizon-protocol` | Network protocol 777: codecs, framing, encryption, `packets/{handshake,status,login,configuration,play}.rs`, `entity_data.rs` |
| `horizon-assets` | Resource decoding: block-state and item models, baking, atlases |
| `horizon-prepare` | Builds `.local/horizon-client/26.3` from the player's own Minecraft install (runs the Java exporters) |
| `horizon-server`, `horizon-server-files` | The `horizon-server` supervisor (`server.toml`, CLI, control API; [docs/hosting.md](docs/hosting.md)); `server.properties`, eula, ops/whitelist/bans, usercache |
| `horizon-anticheat` | The dedicated server's anticheat, off by default; the hub runs it (`lan/anticheat.rs`) ([docs/anticheat.md](docs/anticheat.md)) |
| `horizon-friends`, `horizon-relay` | Play with Friends: invite codes, encrypted tunnel, the relay ([docs/play-with-friends.md](docs/play-with-friends.md)) |
| `horizon-mods`, `cargo-horizon-mod`, `horizon-mod-registry` | Content mods (`.hmod`, no code), the registry index and `.hpack`, `cargo horizon-mod`, the registry's submission service ([docs/mods.md](docs/mods.md)). Code mods are native, built by the launcher ([docs/native-mods.md](docs/native-mods.md)) |
| `horizon-port-pack` | Porting Java modpacks ([docs/modpacks.md](docs/modpacks.md)) |
| `horizon-shader-api`, `horizon-shaders` | Horizon's WGSL shader packs (`.hshader`; [docs/api/shaders.md](docs/api/shaders.md)); the clean-room `shaders/` folder format, no wgpu |
| `horizon-theme` | Design tokens and fonts for the launcher, the game's own menus and the website |
| `horizon-telemetry`, `horizon-telemetry-server`, `horizon-bench` | Opt-in data collection, its ingest server, the benchmark report format |
| `horizon-dualsense`, `horizon-hdr`, `horizon-threads`, `horizon-release-sign`, `horizon-bots`, `xtask` | DualSense output, HDR swapchain metadata, thread priorities, signed release checksums, load-test bots, repo tasks |
| `launcher/` | GPUI launcher: `main.rs` (UI), `backend.rs` (installs, updates), `cli.rs`, `mcp.rs`, `*_page.rs` |

Elsewhere: `tools/` (harnesses, dev scripts), `tests/` (Python unit tests: `python3 -m unittest discover -s
tests`), `mods/horizon/src/test/java/dev/horizonlab/parity/` (Java oracles), `docs/`, `.github/workflows/`.
The website (playhorizon.dev, private repo horizon-rust/horizon-site) builds from main's `README.md`,
`launcher/links.json`, `docs/api/`, `roadmap/roadmap.json` and `crates/horizon-theme/tokens.css`: edits there go live.

## Build and test

```sh
python3 tools/pr_ready.py                  # the self-check before review; paste its block in the PR (--base upstream/main from a fork)
python3 tools/pr_ready.py --quick          # same without harnesses
cargo test -p horizon-client --lib --bins --tests   # skips the examples, as pr_ready does; `--doc` separately
cargo build --profile fast -p horizon-client        # optimized and quick to link: to play or run lan_e2e
python3 tools/parity.py --changed origin/main       # the harnesses your branch can affect (CI's parity job)
python3 tools/parity.py --changed-files paths.txt --list   # which harnesses cover these files, without running them
python3 tools/parity.py mob_ai sculk       # named harnesses
python3 tools/lan_e2e.py --check chest     # two real games over LAN
python3 tools/agent_control_e2e.py         # horizon-ctl / MCP end to end
```

- **Harness** = `tools/test_<name>_parity.py` + a Java oracle (`Vanilla*Oracle.java`) + a Rust probe
  (`crates/<crate>/examples/<name>_{probe,fixture}.rs`, or the game binary with a fixture env var:
  `game/{dispense_fixture,sculk_fixture,command_harness,net_harness}.rs`). It runs oracle and probe on the
  same inputs against the real 26.3 jars and compares their JSON. Copy a nearby harness for a new one.
- Harnesses and the game need the vanilla data in `.local/` ([docs/parity-workflow.md](docs/parity-workflow.md));
  without it, pr_ready lists the harnesses for CI. Oracle runs are cached (`tools/parity_shim.py`; `--fresh` reruns).
- Probes build into `.local/parity-target/`; harnesses find binaries through `lab.cargo_target()`.
- **Blast radius**: a game-binary file (`game.rs`, `game/`) reruns ~25 harnesses; a client-lib,
  `horizon-world` or `horizon-core` file reruns 100+. Plan for it.
- The 5 slow `mod_content::tests::` are `#[ignore]`d (`-- --ignored`); pr_ready runs them when mod content,
  assets, blocks or items change.
- Linux needs `libasound2-dev libudev-dev`; `--no-default-features` drops audio and controllers (the server build).
- **Drive the game** ([docs/horizon-ctl-commands.md](docs/horizon-ctl-commands.md)):
  `target/fast/horizon-ctl --socket $PWD/ctl.sock launch --world W --creative --offscreen --wait --resources $PWD/.local/horizon-client/26.3`,
  then `horizon-ctl --socket … state|keys|turn|command|screenshot|quit`.

## Vanilla reference

- **Port ledger first** for parity work: `python3 tools/port_ledger.py issue N` (or `where Class.method`,
  `what file:line`), and `check --base origin/main` before the PR. The recipe:
  [.claude/skills/vanilla-parity/SKILL.md](.claude/skills/vanilla-parity/SKILL.md).
- Decompiled sources: `.local/minecraft/26.3/sources/{client,server}/` (`python3 tools/lab.py decompile`).
- Bytecode (callers through lambdas, every overload):
  `python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom`.
- Vanilla's `Math` goes through `horizon_core::jmath` and its RNG through `horizon_core::random`, with random
  draws in vanilla's order.

## horizon-client: where things live

`L:` = lib (`crates/horizon-client/src/`, or `crates/horizon-client-render/src/` for the modules that crate
holds, under the same `horizon_client::<module>` paths), `G:` = bin (`crates/horizon-client/src/game/`). The
lib can't see `Game`; bin code reaches the lib as `horizon_client::<module>`.

| Subsystem | Files |
|---|---|
| Game loop, tick, frame | `game.rs`: `fn run`, `impl ApplicationHandler for Game` (`window_event`, `about_to_wait`), `fn tick`, `fn tick_world`, `fn render`; L: `clock.rs`, `spans.rs`; `horizon-client-input/src/window.rs` |
| Rendering | L: `gfx.rs` (`Renderer::new`, `Renderer::render`), `gfx/` (cull, occlusion, translucency, upload, timing, lod, weather, stars, end_sky), `*.wgsl`, `particles.rs`, `block_entity_render.rs`, `item_render.rs`, `fog.rs`, `clouds.rs`, `camera.rs`, `first_person.rs`; G: `render_tick.rs`, `item_draw.rs` |
| Chunks, streaming, light | L: `streaming.rs` (worker pool: generate, light, mesh), `world.rs`, `chunk.rs`, `chunk_map.rs`, `light.rs`, `mesher.rs` + `mesher/`, `vanillagen.rs`, `lod.rs` + `lod/`, `ticket_levels.rs`; G: `world_loading.rs`, `postprocess.rs`; game.rs `fn update_world` |
| World generation | `crates/horizon-world/src/worldgen/` (`generator*.rs`, `feature_graph.rs`, `structure/`, `carver/`) |
| Block behaviour | `crates/horizon-world/src/level/` (`mod.rs`, `tick.rs`, `hooks.rs`, `redstone.rs`, `place.rs`, `interact.rs`); game.rs `fn tick_blocks`, `fn process_level_events` |
| Mobs | L: `mobs.rs` (types), `mobs/` (`ai.rs`, `goals.rs`, `brain.rs`, `*_brain.rs`, `nav.rs`, `path.rs`, `species.rs`, `spawning.rs`, `render.rs`, `anim.rs`); G: `creatures.rs` (ticking, spawning, mob events), one file per mob family (`dragon.rs`, `wither.rs`, `villagers.rs`, `piglins.rs`, `golems.rs`...), `natural_spawning.rs` |
| Other entities | L: `entities.rs`, `projectiles.rs`, `arrows.rs`; G: `boats.rs`, `minecarts.rs`, `mounts.rs`, `armor_stands.rs`, `paintings.rs`, `frames.rs`, `fireworks.rs`, `tnt.rs`, `explosion.rs` |
| Player actions | game.rs (`fn use_hand`, `fn use_item`, `fn attack`, `fn tick_mining`, `fn break_block`); G: `hand.rs`, `combat.rs`, `consume.rs`, `potions.rs`, `shield.rs`, `crossbow.rs`, `trident.rs`, `fishing.rs`; L: `motion.rs` + `motion/`, `collisions.rs`; crate `horizon-items` |
| Inventory and containers | L: `menu.rs` (`AbstractContainerMenu.clicked`), `container_screen.rs`, `containers.rs`, `creative.rs`, `crafting.rs`, `recipe_book*.rs`, `stations.rs`, `merchant.rs`; game.rs `fn menu_ops`, `fn click_inventory`; G: `stations.rs`, `trading.rs`, `recipe_book.rs`, `machines.rs` |
| Commands | parse: crate `horizon-command`; run: G: `commands.rs` (dispatch) + `commands/*.rs`, `functions.rs`, `command_blocks.rs`; L: `scoreboard_cmd.rs`, `data_cmd.rs`; `horizon-client-core`: `nbt_path.rs`, `scoreboard.rs` |
| UI screens | `Screen` enum in game.rs; L: `title.rs` + `title/`, `options_ui.rs`, `multiplayer.rs`, `pack_screen.rs`, `graphics_ui.rs`, `screen_draw.rs`, `ui.rs` (widgets), `hud.rs`, `hud_layout.rs`, `hud_overlays.rs`, `chat.rs`, `tab_list.rs`; G: `startup_menu.rs`, `screens.rs` (pause, death), `options.rs`, `lan_screen.rs`, `chat.rs`, `sign_edit.rs`, `books.rs` |
| Multiplayer client | L: `net.rs` (network thread); G: `remote.rs` (`fn handle_packet`, `fn tick_remote`), `remote_*.rs` |
| LAN host / server side | L: `lan/` (`conn.rs` handshake + login threads, `hub.rs` + `hub/` play state, `chunks.rs`, `entity.rs`, `tracking.rs`, `guest.rs`, `playerdata.rs`); G: `lan.rs` (`LanHost`), `lan_actions.rs` (`fn as_guest`, `fn tick_guests`), `lan_*.rs` per feature, `lan_entities.rs` |
| Integrated server | G: `integrated.rs` (the server thread), `levels.rs` (one level per dimension, `fn tick_server`), `server_travel.rs` |
| Dedicated server | G: `dedicated.rs`, `dedicated/status.rs`; crates `horizon-server-files`, `horizon-server`, `horizon-anticheat` |
| Save and load | L: `save.rs` (level.dat, regions, playerdata), `entity_nbt.rs`, `options_file.rs`; `horizon-client-core/src/block_entity_nbt.rs`; G: `saving.rs`, `persist.rs` (entities); `crates/horizon-world/src/{region.rs,nbt.rs,datafix/}` |
| Input | game.rs `fn window_event`, `fn key_pressed`, `fn play_mouse`; crate `horizon-client-input`; L: `osk.rs`; G: `pad.rs`, `pad_menu.rs` |
| Audio | L: `audio.rs` (mixer), `sound_engine.rs`, `sound_events.rs`, `music.rs`, `ambience.rs`; G: `music.rs`, `subtitle_hud.rs` |
| Resource packs | L: `packs.rs` + `packs/`, `pack_screen.rs`, `atlas_sources.rs`, `sprites.rs`, `mipmap.rs`, `model_library.rs`, `font.rs`; `horizon-client-core/src/lang.rs`; crate `horizon-assets`; G: `packs.rs` |
| Agent control | L: `control.rs` (socket), `bin/horizon-ctl/`; G: `control.rs`, `agent.rs`, `events.rs`, `ui_control.rs` ([docs/agent-control.md](docs/agent-control.md)) |
| Mods | crate `horizon-mods`; G: `mods.rs`; L: `mod_content.rs`, `mod_list.rs` |
| Horizon extras, settings | L: `extras.rs`, `graphics.rs`, `settings.rs`, `minimap.rs`; G: `extras.rs`, `tweaks.rs`, `options.rs` |

## How `Game` is organised

- `struct Game` (game.rs) holds everything, with fields private to the bin. game.rs keeps the core (tick,
  render, input, inventory, world open/close); each `game/<feature>.rs` adds an `impl Game<'_>` block for one
  feature, with its own state types. A feature lives in `game/<thing>s.rs` as `fn tick_<thing>` or
  `fn <verb>_<thing>`; its caller is in game.rs's `fn tick` / `fn tick_world`.
- Mode fields: `remote` (this window is a client of a server: multiplayer, LAN guest, or single player on its
  server thread), `lan` (`LanHost`: the world is open to the network), `integrated` (the window's own server
  thread), `serving` (this `Game` *is* that server thread), `options.dedicated`. With `remote` set, `fn tick`
  returns `tick_remote()` and runs no local simulation.
- A server's players run the same gameplay code as the local player: `as_guest` swaps a guest's state
  (position, inventory, mode, mining, item use) into `Game` for one call (`game/lan_actions.rs`), and
  `levels.rs` swaps whole dimensions the same way. Changes reach clients through the block log, level events
  and the entity tracker (`lan/tracking.rs`, `game/lan_entities.rs`).
- Shared read-only data: `Resources` (`res`: blocks, items, recipes, mob types, biomes) and `PackResources`
  (`pack`), both in game.rs.

## Single player is the server thread

`game/integrated.rs` runs a second `Game` (`serving`) on a "Server thread" with one level per dimension; the
window joins it through memory as a client (`remote`), exactly as a LAN guest or a dedicated server's player
does ([docs/server-levels.md](docs/server-levels.md)). This is what players run, so one fix serves single
player, LAN and dedicated servers. Open `[server-thread]` issues list the gaps. Test it when you touch
player-facing gameplay:

```sh
cargo run --profile fast -p horizon-client -- --resources .local/horizon-client/26.3 --world T
python3 tools/lan_e2e.py --check <name>                  # host on its server thread, guest a server client
python3 tools/parity.py dedicated_server server_entities server_player_damage
```

The **in-frame** path (`--in-frame`, `--no-server-thread`, `HORIZON_SERVER_THREAD=0`, `fn enter_dimension`) is
deprecated and about to be removed: features, fixes and tests target the server thread, and code you touch
moves off in-frame. Only the deterministic rule fixtures still run on it, until they are ported.

## More docs

[docs/README.md](docs/README.md) is the index. Reach for [docs/parity-workflow.md](docs/parity-workflow.md)
on harness runs, gates and shards; [docs/native-client.md](docs/native-client.md) on game options and
controls; [docs/known-issues.md](docs/known-issues.md) before chasing a bug that may be known.
