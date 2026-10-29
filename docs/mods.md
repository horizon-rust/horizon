# Horizon mods

Horizon gets its own mod system, written in Rust and run in a sandbox, plus
two ways to bring over what Java players use: native Horizon ports of popular
mods and the data-driven parts of packs and mods. Fabric and NeoForge jars do
**not** run as they are: their code calls and patches Minecraft's Java classes, and
Horizon has none. What carries over is the data and assets inside a jar
([modpacks.md](modpacks.md)).

Mods are **off and absent by default**. A fresh install has no `horizon-mods`
folder, loads no runtime and plays exactly like vanilla 26.3.

| Part | Where | State |
|---|---|---|
| The contract (WIT) | `crates/horizon-mod-sdk/wit/horizon.wit` | 0.1: log, game-read, chat, hud, storage, command, ui; tick, frame, chat and command hooks |
| SDK for mod authors | `crates/horizon-mod-sdk` | 0.1 |
| Example mods | `crates/horizon-mod-sdk/examples/coords-hud`, `examples/predator-missile` | coordinates, facing and time HUD, `/where`, `/coords`; `/missile`, a guided missile from vanilla commands |
| Horizon's official mods | `mods/official/` | the extras moving to the registry: FPS Counter (`fps-overlay`), Keystrokes, Zoom, Minimap; Far View (`far-view`), distant terrain; and Recipes, a recipe viewer |
| Runtime | `crates/horizon-mods` | wasmtime 48 LTS, component model |
| Authoring tool | `crates/cargo-horizon-mod` | `cargo horizon-mod new [--template code\|content] / build / run / test / check`, and the registry's CI (`registry build / review / list`) |
| In the game | `crates/horizon-client/src/game/mods.rs`, Options > Mods | client mods |
| Launcher mod browser (Modrinth, CurseForge with a key) | [mod-browser.md](mod-browser.md) | resource packs, modpacks |
| Registry index (`index.json`, `.hpack`), signatures | `horizon_mods::index`, [api/mod-index.md](api/mod-index.md) | schema 1; launcher and website next |
| Server mods, registries | this page, [Roadmap](#roadmap) | planned |

## Design goals

1. **Safe to install.** A mod is untrusted code. It runs in a WebAssembly
   sandbox with no file, network, process or clock access, a memory cap and a
   time budget per call. It can only do what its capabilities grant, and the
   player sees those before turning it on.
2. **One file for every platform.** The same `.hmod` runs on Windows, macOS
   and Linux, x86-64 and ARM: authors build once, players never meet "wrong
   build for my OS".
3. **Free when unused.** No mod on: no engine, no thread, no compile, one
   branch per hook. Measured below.
4. **Vanilla stays vanilla.** Mods add; they cannot patch the game's code the
   way Mixins do. What they may change is a list of hooks the game calls at
   fixed points, so the parity harnesses keep testing the same game.
5. **Stable for authors.** The contract is a versioned WIT package; a mod
   built for API 0.1 keeps loading on later 0.1.x games.

## How it works

```
 mod crate (Rust)                         Horizon
 ───────────────                          ───────
 impl Mod for MyMod { ... }               horizon-mods::Mods
 export_mod!(MyMod)                         ├─ Engine (cranelift), made on first mod
        │ cargo horizon-mod build           ├─ one Store per mod: 64 MiB, fuel per call
        ▼                                   ├─ Linker: log, game, chat, hud (WIT imports)
 my_mod.hmod = zip {                        └─ calls the events export: init, tick,
   horizon-mod.toml,                             frame, command, chat
   mod.wasm  (a WebAssembly component)
 }
```

- **WebAssembly components.** Mods compile to `wasm32-unknown-unknown`;
  `cargo horizon-mod build` wraps the module into a
  [component](https://component-model.bytecodealliance.org/) for the
  `horizon:mods/horizon-mod` world. The WIT file is the whole API: host
  functions the mod imports (`log`, `game`, `chat`, `hud`) and the `events`
  interface it exports. No WASI: the component cannot even name a file or a
  socket.
- **Capabilities are imports.** Every imported interface except `log` is a
  capability. The manifest declares them; the game reads the component's
  actual imports and refuses a mod whose code imports one its manifest does not
  declare (`uses capability chat, missing from horizon-mod.toml`). So the list
  the player reads is the truth, not a promise.
- **Hooks on request.** `init` returns a `registration`: the commands the mod
  owns and which of `tick`, `frame` and `chat` it wants. The game calls only
  those. A mod that wants nothing per frame costs nothing per frame.
- **Retained HUD.** A mod sets its panel (corner and coloured lines) when it
  changes; the game draws it every frame with the extras' overlay code. Most
  HUD mods need no frame hook at all.
- **Snapshots, not callbacks into the world.** Once per tick (and before a
  command) the game copies what mods may read (player position, look, health,
  food, dimension, world clock) into a snapshot shared by all mods. Mods never
  hold references into the game, so they cannot stall or corrupt it.

### What a mod can do (API 0.1)

| Interface | Capability | Functions |
|---|---|---|
| `log` | always | `write(level, message)`, in the game log as `[mod <id>]` |
| `game` | `game-read` | `get-player()`: x y z, yaw, pitch, health, food, dimension; `get-clock()`: game time, day time; `raycast(distance)`: the first block along the view (fluids pass), up to 1024 blocks and across the distant terrain, with the point hit and the block's id |
| `chat` | `chat` | `show(text)`: a line only the local player sees, after the mod's name (8 per call) |
| `hud` | `hud` | `set-panel(corner, lines)`, `clear()`: up to 16 lines of 256 characters |
| `storage` | `storage` | `get(key)`, `set(key, value)`, `delete(key)`: a per-mod folder of up to 64 files, 4 MiB total, at most 1 MiB written per call |
| `resources` | `resources` | `read(path)`: bytes from this program's packaged assets and data. Experimental, with `experimental = ["resources"]`, client API 0.2 or server API 0.3 |
| `command` | `command` | `run(line)`: a vanilla command run as the player, with the player's permissions, after the hook returns (8 per call); mods' own commands are not run. `run-quiet(lines)`: up to 256 a call, the same rights, nothing in the chat (single player and Horizon's servers run them 64 a tick and log one line a batch; elsewhere they go as chat commands and their feedback shows) |
| `ui` | `ui` | `open-screen`, `close-screen`, `set-settings`: a screen the mod owns and a page in Options > Mods, of text, button, item and list widgets the game lays out and draws with its own menus' code; `browser.set-items`, `browser.poll`: a recipe viewer's panel of items right of the inventory and container screens (not the creative one), which the game pages and draws with its own slots and search box, and the mod hears its clicks, its searches and the mod's own keys (`input.register-key`) pressed over any item on the screen; `browser.set-recipes`: recipes in the panel's place (crafting grids, cooking, stonecutting, smithing and brewing layouts, their slots cycling through their stacks, a note and an action button each), paged by the game, with clicks on their slots, action buttons, page turns and a back button. **Experimental:** the manifest needs `experimental = ["ui"]` |
| `render` | `render` | `set-hud(shapes)`: up to 1024 fills, four-cornered shapes, GUI sprites, items and text, kept on the HUD (over the hotbar, under chat and screens, hidden with F1) until replaced; `set-corner(corner, w, h, shapes)`: a box the game places in a corner, stacked with its overlays and other panels, as `hud`'s panels are; `text-width(text)`; `frame-stats()`: the game's FPS, min and 1% low; `gui-size()`; `shake(strength, seconds)`: the view turns, the player doesn't, scaled by Distortion Effects; `set-zoom(zoom)`: the field of view divided by up to 10, eased or at once, with slower turning, while playing and not through a spyglass (the strongest of several mods' zooms counts); `set-minimap(minimap)`: a map of the world around the player in a corner, as a map item colours it (the floor underground and under roofs), square or round, turning with the player or north up, with the player's arrow; the game keeps its tiles within its own frame budget (one map at a time, the first loaded mod's); `set-far-terrain(far-terrain)`: the world past the render distance as distant terrain, out to 64 to 1024 chunks, built by the game's own workers (the player's LOD Distance, when on, wins; the first loaded mod's). The game's own HUD renderer draws them: a mod never supplies shader code. **Experimental:** `experimental = ["render"]` |
| `input` | `input` | `claim-items(ids)`: the use button on those items is the mod's (`use-start`, `use-stop` with the ticks held; the game doesn't place, eat or swing); `register-key(name, label, key)`: up to 16 keys (`key.keyboard.r`), heard while playing as `key-down` / `key-up` (the game's own mapping on that key still runs; a screen opening lets held keys up), mouse buttons as `key.mouse.4`. The player rebinds them on the mod's page in Options > Mods, where the game lists them first as Key Binds rows (click, then press a key or mouse button; Esc binds none), even for a mod without `ui`; the choice is kept in `horizon-mods.txt` (`key.<mod>.<name>:key.keyboard.z`) and wins over `register-key`'s default; `claim-wheel(on)`: the wheel's steps become `wheel` events and the hotbar doesn't move (the one input a mod takes from the game; a mod's keys never do); `held()`; `mapping(name)`: a movement or click mapping's key label and whether it is held; `clicks-per-second()`; `poll()` returns the events, at most 256 kept. **Experimental:** `experimental = ["input"]` |
| `recipes` | `recipes` | `items()`: every item in the registry's order, with its name in the player's language and its tags; `all()`: every recipe (crafting, smelting, blasting, smoking, campfire, stonecutting, smithing, brewing), its slots as the stacks that fit each, its results, cooking time and experience, and the recipe book's id for it; `known()`: the book ids the player's recipe book holds; `place(book-id, all)`: the recipe book's click on a recipe, in the open inventory's or crafting table's grid (refused for a recipe the player's book doesn't hold, as vanilla's server refuses it); `give(stack)`: in creative mode, the stack into the player's inventory, as from the creative inventory. The recipes are the world's in single player and on Horizon's servers, vanilla 26.3's on any other server (a client is sent no others), while `known` follows that server's book. The game makes the tables once, when a mod first reads them. **Experimental:** `experimental = ["recipes"]` |

| Hook (export) | When |
|---|---|
| `init() -> result<registration, string>` | once, after loading; an error keeps the mod off and is shown |
| `tick(tick)` | each game tick while in a world (20 per second) |
| `frame(seconds)` | each rendered frame |
| `command(name, args) -> result<option<string>, string>` | `/name args` typed in chat, for a command the mod registered |
| `chat(text)` | each chat line the player receives, as plain text |

Commands: vanilla's command names win. A mod asking for `/give` does not get
it (logged), and two mods cannot share a name (the first one loaded keeps it).
The chat box colours and Tab-completes a mod's commands like vanilla's, for
every player whatever their permission level, and takes any text after the
name (the mod reads it as `args`; API 0.1 has no way to declare argument types).

### Planned interfaces

Each is a new capability, added in a 0.x minor version so older mods keep
loading. Ordered by what the first native ports need:

| Interface | For | Notes |
|---|---|---|
| `keys` | keybinds | registered binds appear in Controls; press/release events |
| `target` | the crosshair's block or entity, with its state and NBT summary | Jade/WTHIT-style tooltips |
| `inventory-read` | the player's inventory, held item, open container | AppleSkin, recipe viewers |
| `world-read` | blocks and biomes around the player, within the render distance | minimaps, schematics |
| `render` (world) | world-space lines, boxes and billboards, beside the HUD shapes `render` has | Litematica-style placement, waypoints |
| `registries` | new blocks, items, entities and recipes, mostly data: JSON models, textures, loot and recipes in the `.hmod`'s `assets/` and `data/` | server side; see [Multiplayer](#multiplayer) |
| `server` | world events (entity spawn/death), game rules, world read/write, typed server commands | after [server mods 0.3](#server-mods-api-03-experimental)'s players, chat, blocks and commands |
| `net` | a mod's own client↔server channel (custom payloads) | only between the two halves of the same mod |

There is no file, socket, process or raw-input interface, on purpose.

## Choosing the runtime

| Option | Sandbox | One file for all OSes | Typed API | Cost | Verdict |
|---|---|---|---|---|---|
| **wasmtime + component model** | yes: memory, fuel, no imports but ours | yes | WIT → Rust on both sides | 85 ns per empty hook call; 50 ms to compile a 64 KB mod on load | **chosen** |
| wasmtime, core modules with a hand ABI | yes | yes | no: pointers and lengths by hand | a few ns less per call | not worth the fragile ABI |
| wasmi (interpreter) | yes | yes | no components | no compile time, execution ~10-50x slower than cranelift code | kept in mind for a platform without JIT |
| Lua (mlua) or another script | partial (strip the stdlib, hope) | yes | no | fast to write | not Rust; weak sandbox |
| native `cdylib` plugins | none | no: per OS and arch, unstable Rust ABI | yes | none | unsafe for players |
| an embedded JVM | none per mod | yes | Java | 100+ MB, seconds of startup | see the research |

wasmtime 48 is a long-term-support line (security fixes until August 2028)
and needs Rust 1.95. It reads WebAssembly with the same wasm-tools release
(0.254) as the `wit-component` that packs mods and the SDK's `wit-bindgen`, so
the build carries one copy of them. Only its `runtime`, `cranelift`,
`component-model` and `std` features are on: no WASI, no async, no GC
proposal, no profiling. Its cranelift backend generates native code for x86-64
and ARM64 on Windows, macOS and Linux, which is why one `.hmod` serves them
all.

The game links it only with its `mods` feature: every release build turns it
on (`release.yml`), and a build from source needs
`cargo build --release -p horizon-client --features mods` to run code mods.
Without it (the tests and probes) wasmtime and cranelift aren't built, a
quarter of a clean build; content mods still load, and the game logs why the
code mods don't start.

### Performance

Measured on the Mac mini (M4 Pro), release build, `cargo test -p
horizon-mods --features runtime,pack --release --test sandbox -- --ignored --nocapture`:

| | per call |
|---|---|
| tick with no mod loaded | 1.2 ns (one check) |
| empty `tick` hook (call into the sandbox and back, fuel on) | 85.5 ns |
| empty `frame` hook | 87.5 ns |
| coords-hud's `tick` (reads the player and clock, formats 3 lines, compares) | 2.3 µs |
| loading coords-hud (read, compile with cranelift, instantiate, `init`) | 50 ms, once |

In the game, same world and view, `horizon-ctl bench 10` three times each
(4 other sessions shared the machine, so the tails are noisy; the median is
the stable figure):

| | median frame | mean FPS |
|---|---|---|
| no `horizon-mods` folder | 2.62-2.71 ms | 149-163 |
| coords-hud on | 2.72-2.78 ms | 151-254 |

At 20 ticks per second coords-hud costs 46 µs per second of game time: well
under 0.01 ms per frame. The frame-time medians differ by less than the
run-to-run spread.

**Budgets.** Each call gets fuel (about one unit per WebAssembly instruction):
2 billion for `init`, 50 million for `tick`, 10 million for `frame`, 100
million for a command or chat line. A runaway loop ends in a fraction of a
second (the sandbox test's endless `/spin` is cut well within its 5 s limit)
instead of hanging the game. The target budgets a mod should stay under are
tighter: **1 ms per tick and 0.1 ms per frame**. The runtime keeps each mod's
average call time (`Entry::cost`); the Mods page will show it, and a mod over
budget for long will be flagged, then paused.

### Crash isolation

A trap (panic, out-of-bounds access, `unreachable`), running out of fuel or
hitting the 64 MiB memory cap stops **that mod only**. The game says so in
chat (`Mod Test Guest stopped: it took too long (an endless loop?)`), marks it
on the Mods page and carries on; the other mods keep running. Host functions
clip what a mod hands back (chat lines per call, HUD lines and lengths). Tests:
`crates/horizon-mods/tests/sandbox.rs` (an endless loop, a panic and a 1 GiB
allocation, next to a healthy mod; `cargo test -p horizon-mods --features
runtime,pack`).

## Playing with mods

1. Put `.hmod` files (or mod folders holding `horizon-mod.toml` and
   `mod.wasm`) in `horizon-mods/` beside `options.txt` (in the launcher's game
   folder).
2. Options > **Mods** lists them with what each may do ("May see your
   position, health and the time, draw text on your screen, ...") and a
   switch. They start off; the choice is kept in `horizon-mods.txt`
   (`mod.<id>:true`).
3. A mod that failed to load or stopped shows why, in red, on its row.

Two turned-on mods never share an `id` while running (it is also capability
`storage`'s directory): the one `find` lists first starts, the other stays
off with the reason on its row.

Benchmarks (`--benchmark`) never load mods.

## Making a mod

```sh
rustup target add wasm32-unknown-unknown
cargo install --git https://github.com/horizon-rust/horizon cargo-horizon-mod
cargo horizon-mod new my-mod        # Cargo.toml, horizon-mod.toml, src/lib.rs, tests/hello.toml
                                    # (--template content: a content mod, no code)
cd my-mod
cargo horizon-mod run               # build, then start the game with it
cargo horizon-mod build             # again after an edit: the running game reloads it
cargo horizon-mod test              # build, then run tests/*.toml with no game
```

`run` starts `horizon-client` (on `PATH`, or `--game PATH`, or
`HORIZON_CLIENT`) with `--dev-mod target/horizon-mod/<id>.hmod`. A dev mod is
always on, and the game checks its file once a second: rebuild and it restarts
with "Mods reloaded" in chat. Its in-memory state starts over; whatever it
saved with capability `storage` survives the reload.

Before you submit it to the registry, run what its CI runs:

```sh
cargo horizon-mod check target/horizon-mod/my_mod.hmod   # --json for the report as JSON
```

It reads the manifest and content, refuses imports the capabilities don't
cover, then plays the mod in the sandbox for 40 ticks, 120 frames, a chat line
and each of its commands: it must start and never trap or overrun. It checks a
`.hpack` and a resource pack `.zip` too, and a signature with `--key <public
key> --signature <file>.minisig`.

### Testing a mod

`cargo horizon-mod test` builds the mod, then runs each `tests/<name>.toml` in
the sandbox the game uses, with no game and no window: the test plays the
game's part (the player and the clock the mod reads, ticks, frames, chat lines,
commands; for a server mod, players joining, chatting, typing commands,
breaking and placing blocks and leaving) and checks what the mod showed. A script fails at its first wrong
step, with what it expected and what it got; the command fails if any script
does. `--debug` builds without optimizing, `--hmod <file>` tests a `.hmod`
already built, and a name runs only the scripts whose path holds it. Each
script gets a scratch copy of the mod, so its `storage` starts empty and is
removed afterwards.

```toml
# tests/panel.toml (examples/coords-hud)
[player]                 # where the script starts; unset fields keep the defaults
x = 12.5                 # (0.5 64 0.5, yaw 0, 20 health and food, the Overworld)
z = -3.25
yaw = 180.0

[clock]                  # day-time 0 is 6:00 on day 1; each tick adds one
day-time = 29999

[[step]]
ticks = 1
expect.panel = ["XYZ 12.5 / 64.0 / -3.2", "Facing North (-Z)", "Day 2, 12:00"]
expect.corner = "top-left"

[[step]]
command = "coords"
expect.feedback = "Coordinates panel hidden"
```

A step's parts happen in this order:

| Key | Does |
| --- | --- |
| `reload = true` | stops the mod and starts it again, as a rebuild does in the game: its memory starts over, its `storage` stays |
| `title-screen = true` | no player and no clock until a step sets them |
| `blocks = [{ x = 0, y = 63, z = 0, state = "minecraft:stone" }]` | server mods with `world`: those blocks of the Overworld, before the rest of the step (every other block is air, every biome the dimension's plainest: plains, nether wastes, the end) |
| `player = { x = 3.0, ... }`, `clock = { day-time = 0 }` | changes those fields (`x y z yaw pitch health food dimension`, `game-time day-time`) |
| `ticks = n`, `frames = n` | that many ticks (the game's tick counter and the clock move on), then frames 1/60 s apart |
| `chat = "<Steve> hi"` | a chat line the player receives |
| `command = "where north"` | the player types `/where north`; a command the mod doesn't own fails the step |
| `break = { x = 1, y = 63, z = 0, state = "minecraft:stone" }`, `place = { ... }` | server mods: the player breaks or places that block |
| `leave = "Alex"` | server mods: that player leaves |

A server mod starts with nobody on: `join = "Alex"` (after `reload` and `title-screen`) brings a
player in at the defaults, in survival, with the id
`00000000-0000-4000-8000-000000000001` for the first name that joined, `...2`
for the second, the same id when a name joins again. `player`, `chat`,
`command`, `break` and `place` are then that player's doing: the one named by
`as = "Steve"`, or the first one on. `player` takes `game-mode` there, and the
script's own `[player]` and `[clock]` are a client mod's.

Then `expect` checks what the mod did during the step; a key left out is not
checked:

| `expect.` | Checks |
| --- | --- |
| `commands = ["where"]` | the commands the mod owns |
| `chat = ["..."]` | every chat line it showed, in order (`[]`: none) |
| `feedback = "..."`, `error = "..."` | the command's answer, shown plain or in red |
| `panel = ["..."]`, `corner = "top-left"` | its HUD panel's lines (`[]`: no panel) and corner |
| `told = ["Alex: Welcome!"]`, `broadcast = ["..."]` | server mods: every `players.tell` (as `name: text`) and `players.broadcast`, in order |
| `blocks = [{ x = 2, y = 64, z = 0, state = "minecraft:glowstone" }]` | server mods with `world`: those blocks of the Overworld after the step, the mod's `set-blocks` made (the script checks a state's shape; the game checks its name) |
| `stopped = "panic"` | the mod was turned off, for a reason holding this text; without it, a mod that traps, panics or overruns fails the test |

Unknown keys are errors, so a typo never passes silently. The same runner is
`horizon_mods::testing` for Rust code that drives a mod (`TestGame`,
`Script`): Horizon's own tests run the examples' scripts with it. Scripts test
any mod, whatever it was written in.

### Template layout

```
my-mod/
├── Cargo.toml           # cdylib, horizon-mod-sdk, release: opt-level "s", LTO
├── horizon-mod.toml     # the manifest
├── src/lib.rs           # impl Mod, export_mod!
└── tests/hello.toml     # cargo horizon-mod test
```

A mod is a type implementing `horizon_mod_sdk::Mod`; its state lives in the
type:

```rust
use horizon_mod_sdk::{Mod, Registration, chat};

struct Hello { greeted: u32 }

impl Mod for Hello {
    fn init() -> Result<(Self, Registration), String> {
        Ok((Hello { greeted: 0 }, Registration::new().command("hello")))
    }
    fn command(&mut self, _name: &str, args: &str) -> Result<Option<String>, String> {
        self.greeted += 1;
        chat::show(&format!("Hello {args}! ({} so far)", self.greeted));
        Ok(None)
    }
}
horizon_mod_sdk::export_mod!(Hello);
```

Hooks not asked for keep their default bodies. The full example is
[`coords-hud`](../crates/horizon-mod-sdk/examples/coords-hud/src/lib.rs). Logic
that does not touch the API (formatting, maths) is ordinary Rust and unit-tests
on the host with `cargo test`.

### The manifest

```toml
id = "coords_hud"            # [a-z0-9_]{1,64}: settings key, log name
name = "Coordinates HUD"
version = "0.1.0"
description = "..."
authors = ["..."]
api = "0.1"                  # the API the SDK targets
side = "client"              # client | server | both
capabilities = ["game-read", "hud", "chat"]
experimental = []            # experimental capabilities it uses, e.g. ["ui"]
```

Unknown keys are refused, so a typo is an error, not a silently ignored
permission. An experimental capability (`ui`, `render`, `input`, `recipes` or `resources`) may change in any release
([stability](api/stability.md)): a mod uses it only by naming it in
`experimental` as well, so nobody depends on it by accident.

### Reading packaged bytes

Capability `resources` grants permission to “read its own packaged assets and data”.
Client programs use `api = "0.2"`. Server programs use `api = "0.3"` and `side = "server"`.
Both declare `capabilities = ["resources"]` and `experimental = ["resources"]`.
Code is required unless the program also independently declares `registries`.

```rust,ignore
use horizon_mod_sdk::resources;

let bytes = resources::read("data/resource_lab/species/sample.json")?
    .ok_or("missing sample")?;
```

Server Rust programs use the same module at `horizon_mod_sdk::server::resources`.
Declare `players` too when the component imports the existing server event player types.
The call works during `init`, without a world. It reads the locally installed package.
It does not transfer files between clients and servers. Existing `side = "both"` uses client exports.

Paths are literal `assets/<namespace>/<file>` or `data/<namespace>/<file>` names, at most 1024 bytes,
checked with the ordinary content path rules. The namespace must be the program's id or one of its
manifest's `namespaces`. Owner ids and extra namespaces cannot be `minecraft`, `c` or `horizon`.
The host checks ownership and the sorted validated inventory before opening a file.
Missing owned names return `Ok(None)`, empty files `Ok(Some(Vec::new()))`.
Invalid paths, another owner's namespace, listed but unavailable files and limits return short errors
without host paths or partial bytes. Other packages, merged resources and runtime storage are unavailable.
Equal namespace names in two resource packages remain separate.

Ordinary assets and data, including extra namespaces, need `resources` or `registries`.
Registration declarations in `data/<id>/horizon/` and shared `minecraft` or `c` tags still need `registries`.
Files from a program with only `resources` are not merged into the game's resource root.
`storage` remains saved runtime state with its existing quotas.

Packages allow 65,536 content files, 16 MiB each and 1024 MiB total. Every callback, including `init`,
allows 64 resource requests and 32 MiB of charged resource I/O. Invalid paths and misses count as requests.
Before reading, the host reserves the smaller of 16 MiB and the remaining allowance. Success refunds
unused bytes, while failure spends the reservation. Reads stop after that reservation plus one probe byte.
Exhaustion performs no resource I/O. These limits bound I/O volume, not callback latency.
The guest memory limit remains 64 MiB and existing fuel limits apply.

Directory inventory skips symbolic links, including top level assets and data links, and only lists regular files.
An owned confined directory handle prevents reads from escaping after a path changes. A store keeps one ZIP index
for an archive. Byte edits to listed directory files can be visible immediately. New names need a supported reload:
rebuild or touch `mod.wasm`, or atomically replace the `.hmod`. Reload validates content and execution side again.
Failure drops the old store and commands while other owners keep running.

For Rust host callers, `content::FileReader` now owns its handle and has no lifetime parameter.
`Content::reader()` can outlive `Content` and a rename of its directory. Its `read` signature is unchanged.

This candidate uses provisional API level 8 on base `1feb28c`. It collides with draft PR 2888's levels 8 through 18.
Do not publish an SDK, registry entry or binary from this candidate. Before upstream integration, assign the actual
next level on the merged API substrate and rebuild the guests and generated reference. Do not skip levels.

### Content mods (API 0.2)

A mod with `api = "0.2"` and capability `registries` may carry `assets/` and
`data/` in the `.hmod`, and needs no `mod.wasm` if it has no code
(`cargo horizon-mod new --template content` starts one; `build` in a folder
with no `Cargo.toml` packs the manifest, `assets/` and `data/`, and `check`
reads it without a trial run). It adds
blocks and items by declaring them in `data/<id>/horizon/block/*.json` and
`data/<id>/horizon/item/*.json`, each copying a vanilla template
([modpacks.md](modpacks.md#registries-02-data-driven-blocks-and-items)):

```json
{ "template": "minecraft:iron_block",
  "properties": { "hardness": 4.0, "light": 3, "sounds": "minecraft:amethyst_block" },
  "item": true }
```

The fence: files go in the mod's own namespaces (`assets/<id>/`,
`data/<id>/`, and those its manifest lists), plus additions to vanilla tags
(`data/minecraft/tags/`) and to the convention tags NeoForge and Fabric mods
share (`data/c/tags/`).
Templates and `sounds` name vanilla blocks; unknown keys are refused.

A block may optionally declare `states` to register its own property domains instead of
copying the template's domains. This example is deliberately a full-cube stone physics
fallback. It is not a measured machine implementation:

```json
{
  "template": "minecraft:stone",
  "states": {
    "properties": {"multipos": ["top", "bottom", "base"]},
    "default": "multipos=top",
    "aliases": {
      "multipos=top": "minecraft:stone",
      "multipos=bottom": "minecraft:stone",
      "multipos=base": "minecraft:stone"
    }
  }
}
```

Property names are sorted in state keys. Values keep their declared array order, and
every Cartesian variant needs exactly one alias. `default` must name one complete
variant. For example, `facing` with four directions and `multipos` with three positions
requires 12 aliases, including `facing=north,multipos=top`. Aliases name complete existing
states of the declared template. A template with no properties uses `minecraft:stone`,
not `minecraft:stone[]`. A directional alias may be
`minecraft:furnace[facing=north,lit=false]`. Omitted template properties are refused.

The bounds follow the observed Pixelmon block DATA maxima: eight properties, 26 values
per domain and 4374 states per block. Names and values are nonempty lowercase resource
tokens of at most 64 bytes. The existing total world state limit still applies. Without
`states`, old declarations retain their template states and default unchanged.

The resource plan registers the actual names, property maps and default in facts, command
and model tables, then copies physics, shapes and other per-state rows through each
explicit alias. Numeric references in copied rows stay within the custom block. Alias
references preserve custom-only properties, and missing or ambiguous references fail with
an error. Several custom variants may share a template alias without becoming one state.

Owned blockstate assets take priority. Without one, ordinary template model variants are
expanded through the same aliases. Multipart template models require an owned blockstate
file. Unknown aliases, missing model variants and ambiguous model mappings are errors.
This state schema does not implement multiblock placement or linked breaking, machine
processing, block entities, custom collision shapes or rotation rules absent from the
chosen template. Registering `multipos` does not supply those behaviors. An alias to stone
retains its cube collision and identity transforms even when custom properties say
`facing` or `multipos`.

Some Java mods ship their files under other names than their mod id (Towns
and Towers, mod id `t_and_t`, under `kaisyn` and `towns_and_towers`). A
content mod lists those in its manifest:

```toml
id = "t_and_t"
api = "0.2"
capabilities = ["registries"]
namespaces = ["kaisyn", "towns_and_towers"]
```

A listed namespace is the mod's as its id is: its `assets/` and `data/`
files load, and the game reads its tags, recipes, names and loot. Blocks and
items are still declared under `data/<id>/horizon/` only. `minecraft`, `c`
and `horizon` cannot be listed. Two mods that are on may not own the same
namespace (one's id or listed namespace being the other's): the game keeps
the first in load order and leaves the other's content out, with
"mods A and B both use the namespace N; turn one of them off" in the log. Paths
use vanilla's resource characters, at most 65,536 files, 16 MiB each and
1024 MiB in all. A mod whose content is refused stays off, with the reason on
its row. The game reads this content into its block and item tables
(`horizon_client::mod_content`, docs/modpacks.md). A tag entry naming an id
nobody registers (a block a port left out, another mod's item) is dropped,
so the rest of the tag still loads; vanilla would drop the whole tag.

Tags merge as a data pack's do. Every mod's file for one tag (vanilla's
`data/minecraft/tags/`, the convention `data/c/tags/`, or another mod's) adds
its values after vanilla's and the mods' before it, in load order. A file with
`"replace": true` first drops what came before it, vanilla's entries and the
earlier mods', and the mods after it still add theirs, as a higher data pack
does. Since the result depends on load order, the game says so in the log:
"mod towns replaces data/minecraft/tags/worldgen/biome/has_structure/village_plains.json:
vanilla's entries and those of adds are left out, ...". Tags are the only
`minecraft` files a mod may hold: its other content stays in its own
namespaces.

A mod adds to structure sets and template pools that are not its own
(vanilla's villages, another mod's houses) with a set or pool file in its own
namespace holding `horizon:add_to`, a key vanilla data never has. Its entries
are appended to each set or pool named, after every mod's own files, in mod
and file order; the file is no set or pool itself. A name nobody defines is
skipped.

```json
{ "horizon:add_to": "minecraft:villages",
  "structures": [ { "structure": "mymod:village_beach", "weight": 10 } ] }
```

A pool addition holds `"elements"` as a pool does. This is what lithostitched's
`add_structure_set_entries` and `add_template_pool_elements` modifiers do on
NeoForge and Fabric, and `horizon-port-pack carry` writes them from those.

A structure (and any of its pool elements, in place of the structure's) may
hold `horizon:enhanced_terrain_adaptation`: MoogsStructureLib's terrain
kernel, which carves or buries terrain around the
pieces on top of vanilla's `terrain_adaptation`. `kernel_size` and
`kernel_distance` shape the kernel; `top` and `bottom` (`carve`, `bury` or
`none`) act above and below a piece's floor; `bottom_offset`, `padding`
(`x`, `top`, `bottom`, `z`) and MoogsStructureLib's `band` (`bottom`, `top`,
`piece_heights`) move the box it measures from. On a structure,
`rigid_junctions` adds rigid pieces' junctions as well and
`inflate` grows the start's box by the kernel radius. `kernel_size` 0 is
none.

```json
{ "horizon:enhanced_terrain_adaptation": { "kernel_size": 24, "kernel_distance": 24,
    "top": "carve", "bottom": "none", "rigid_junctions": true, "inflate": true } }
```

### Server mods (API 0.3, experimental)

A server mod runs where the world is simulated: on the dedicated server, on a
LAN host and on single player's own server, never in a player's window. Its
manifest says `side = "server"` and `api = "0.3"` (and only a server mod may);
its component targets the WIT world `horizon-server-mod` instead of
`horizon-mod`. Experimental: it may change in any release
([stability](api/stability.md)).

| Interface | Capability | Functions |
|---|---|---|
| `log` | always | as for client mods |
| `storage` | `storage` | as for client mods, in the server's `horizon-mods/` |
| `resources` | `resources` | this program's own package bytes, as above, with `experimental = ["resources"]` |
| `players` | `players` | `online()`: id, name, position, dimension, game mode, health of each player on; `tell(id, text)`, `broadcast(text)`: system messages after the mod's name, 8 per call |
| `world` | `world` | `get-block(dimension, x, y, z)`: the state in a loaded chunk, `minecraft:oak_log[axis=y]`; `get-biome(dimension, x, y, z)`: `minecraft:plains`, ...; `set-blocks(dimension, changes)`: up to 4096 per call, checked at once, made as `/setblock` makes them (neighbours, light, block entities, every player told) after the call, before that dimension's next tick |

| Hook (export `server-events`) | When |
|---|---|
| `init() -> result<registration, string>` | once, after loading |
| `tick(tick)` | each server tick |
| `player-joined(player)`, `player-left(player)` | `registration.joins` |
| `chat(player, text)` | `registration.chat`: each chat message a player sends |
| `command(player, name, args, values)` | `/name args` typed by a player, for a command it registered, with its arguments' values (below); `ok(some)` is that player's feedback, `err` their red error |
| `block-broken(player, pos, state)`, `block-placed(player, pos, state)` | `registration.blocks`, with the state as `minecraft:oak_log[axis=y]` |

Capabilities that act on a local player (`game-read`, `chat`, `hud`) are for
client mods; `players` is for server mods. In Rust: `impl
horizon_mod_sdk::server::ServerMod` and `export_server_mod!`; the example is
[`welcome`](../crates/horizon-mod-sdk/examples/welcome/src/lib.rs), and with
`world`, [`beacons`](../crates/horizon-mod-sdk/examples/beacons/src/lib.rs).

**Typed commands.** Each command in `registration.commands` lists its
arguments, in order: a name and a kind, required or optional (optional ones
last). The server adds them to the command tree every player receives, after
vanilla's commands, so a player's chat box completes and colours them as it
does vanilla's (player names, coordinates, bounds); when a player sends one,
the server parses the line the way vanilla's parsers do and gives the mod the
values, by argument name, or answers with vanilla's error (`Integer must not
be more than 64, found 65`, `No player was found`) without calling the mod.

| Kind | Takes | Value | Chat box parser |
|---|---|---|---|
| `word` | one word: letters, digits, `_ - . +` | `word(string)` | `brigadier:string` (word) |
| `text` | the rest of the line, only last | `text(string)` | `brigadier:string` (greedy) |
| `integer(min, max)` | a whole number within the bounds given | `integer(s32)` | `brigadier:integer` |
| `number` | a number | `number(f64)` | `brigadier:double` |
| `boolean` | `true` or `false` | `boolean(bool)` | `brigadier:bool` |
| `player` | an online player's name, `@s` or `@p` (whoever typed it) | `player(player)` | `minecraft:entity` (one player) |
| `block-pos` | `x y z`, each a whole number, `~` or `~n` (from whoever typed it) | `block-pos(block-pos)` | `minecraft:block_pos` |

A command with no arguments takes nothing after its name. In Rust,
`Registration::command_with("beacon", [Argument::integer("height", Some(1),
Some(64)).optional()])`, read with `args.integer("height")`; `command(name)`
alone takes free text, an optional `text` argument named `args`
(`args.text()` is the line as typed either way).

**Capability `world`.** Reads are of the server's loaded chunks as they are
during the call: the game lends a server mod's call the levels' chunk maps
(moved, not copied, and only while a running mod has `world`), so a read
costs a map lookup and no copy of the world. A dimension with no chunk loaded
at a position reads as none. Writes go through the same code as `/setblock`,
so block updates, light, block entities and every client (LAN guests,
dedicated server players) see them as vanilla's; the parity harnesses test
that code. Changes for a dimension whose level isn't ticking wait for it
(in-frame single player ticks only the player's), up to 65,536 blocks; a
change in a chunk that isn't loaded when it is made is skipped, as `/setblock`
refuses one.

**Where they run.** The game that simulates the world starts the server mods
that are on, and only that one:

| Mode | Runs server mods | Players they see |
|---|---|---|
| Single player (default) | the server thread | the window's player, a connection like any other |
| Single player `--in-frame` | the window's own game, beside its client mods | the window's player |
| Open to LAN, Play with Friends | the host's server thread (or its window, in-frame) | the host and every guest |
| Dedicated server (`--dedicated`, `horizon-server`) | the server | every connected player |

A window that is a client of a server (single player's window, a LAN guest,
any multiplayer server) never runs them; its Mods page lists a server mod with
the others, and its switch takes effect the next time a world opens. The mods
live in `horizon-mods/` of the game folder, or of the server's folder on a
dedicated server, and are turned on as client mods are: Options > Mods, or
`mod.<id>:true` in `horizon-mods.txt`. `--dev-mod <file>` runs one always
on and reloads it when its file changes, on a server too.

**When hooks run.** At the end of each server tick the game tells the server
mods who joined and who left since the last one, then what players said,
broke and placed, in order, then calls `tick`. A command a player types
reaches its mod at once, whatever the player's permission level, and its
answer goes to that player. `tell` reaches one player, the window's own
chat for the in-frame player and a system message for a connection;
`broadcast` reaches every player and the server's log. With no server mod
on, each of these points costs one branch (measured in the PR that added
them).

**Not yet.** Block events are a player's own breaking and placing, not
explosions, pistons or other mods. Command arguments don't take entities,
items or block states yet, and `^` coordinates; in-frame single player's chat
box lists server mods' commands by name only (the server still parses their
arguments).

## Publish your mod

Horizon's registry lists mods, modpacks (`.hpack`) and resource packs made for Horizon, each reviewed before it is
published. Players find them at [playhorizon.dev/mods](https://playhorizon.dev/mods) and in the launcher (Mods, then
**Horizon** beside Modrinth), which checks every file before it installs it.

1. Run `cargo horizon-mod check` on the file (above): the registry runs the same checks.
2. Put the source somewhere public, under a licence that lets players run it: reviewers read it, and the file must be
   built from it.
3. Open [playhorizon.dev/mods/submit](https://playhorizon.dev/mods/submit), sign in with GitHub (it reads only your
   public profile) and fill in the page: the file, its name and summary, licence, source address, and optionally a
   minisign public key with the file's `.minisig`.
4. The form checks the file again and opens a pull request in
   [horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods) that adds `mods/<id>.toml`, with you as its
   owner. That repository's checks run the file in the sandbox and post their report on the pull request; answer the
   reviewers there.
5. A reviewer merges it: within minutes the file is a release of horizon-mods, and the project is on the site and in
   the launcher.

Once it is published, its page and its card in the launcher show its downloads (GitHub's counts of its files,
refreshed every hour) and its rating: players signed in with GitHub on its page give it 1 to 5 stars, one rating per
account, which they can change. Its owners can't rate it, and nobody sees who rated.

An update is the same form with a new version; only the project's `owners` can publish one, signed with its key if it
has one. A published file never changes. Without the form, open the pull request by hand (the repository's README
says how). The formats: [api/mod-index.md](api/mod-index.md).

### Horizon's official mods

The Horizon extras that are mods now, and Horizon's own recipe viewer, live in `mods/official/<name>/`, one crate each in the game's workspace, so
`cargo clippy --workspace` and pr_ready cover them. They use only the public API, like any other mod, with the same
options on their settings page as the extra had and the same look. Each is published by a maintainer: build it on
`main` with the toolchain CI uses (`cd mods/official/<name> && cargo horizon-mod build`), then open a pull request in
horizon-mods with its `mods/<id>.toml` (owners `ludovic111`, `source` pointing at the folder) and the file's
SHA-256. The reviewer builds it the same way and merges only when the hashes match: the `.wasm` holds no local paths,
so the same rustc and `Cargo.lock` give the same file.

| Mod | Id | Replaces | Settings (storage key: values) |
|---|---|---|---|
| FPS Counter | `fps_overlay` | Extras > FPS counter | `corner`: `top_left` (default), `top_right`, `bottom_left`, `bottom_right`; `lows`: `false` (default), `true` |
| Keystrokes | `keystrokes` | Extras > Keystrokes | `corner`: as above, default `top_left`; `cps`: `true` (default), `false` |
| Zoom | `zoom` | Extras > Zoom | `level`: `2`, `3`, `4` (default), `6`, `8`, `10`; `smooth`: `true` (default), `false`; `sensitivity` (slower turning): `true` (default), `false`. The key is the game's: `key.zoom.zoom` in `horizon-mods.txt`, default `key.keyboard.z` |
| Minimap | `minimap` | Extras > Minimap | `corner`: as above, default `top_right`; `size`: `64`, `96` (default), `128`; `shape`: `square` (default), `round`; `zoom` (blocks per pixel): `0.5`, `1` (default), `2`, `4`; `rotate`: `false` (default), `true`. The game draws the map (`render.set-minimap`) |
| Far View | `far_view` | Graphics > LOD Distance and Far Detail | `distance` (chunks): `64`, `128`, `256` (default), `512`, `1024`; `detail`: `full` (default), `reduced`. The game builds and draws the distant terrain (`render.set-far-terrain`); the player's LOD Distance, when on, wins |
| Recipes | `recipes` | nothing (new) | `give` (a click on the item panel gives the item in creative mode: one, a stack with a right click): `false` (default), `true`. The keys are the game's: `key.recipes.recipe` (default `key.keyboard.r`) and `key.recipes.uses` (default `key.keyboard.u`) in `horizon-mods.txt`. Every item sits in the item panel (`browser`) with a search box (words in the name or id, `@namespace`, `#tag`); R or a click shows an item's recipes, U or a right click its uses, a tab per station, and + fills the grid with a recipe the player knows (`recipes.place`). On servers that are not Horizon it shows the game's own 26.3 recipes; known recipes and + follow the server's recipe book |

## Versions and compatibility

- The API is the WIT package `horizon:mods@0.1.0`. New functions and
  interfaces arrive in patch versions (0.1.1, ...); a game serving 0.1.N loads
  mods built for 0.1.0 to 0.1.N. A breaking change makes `0.2` (and later
  `1.0`); the game then links both versions side by side for at least one
  release cycle, so existing mods keep working while authors move.
- The manifest's `api` names the line it needs (`0.1`); the game refuses a
  line it does not serve with a clear message.
- Within a line, each function has an API level, the one it arrived at
  ([stability](api/stability.md#mod-api-levels)). A mod needs the level of
  the newest function it calls, read from its file: an older Horizon refuses
  it with "made for a newer Horizon: update Horizon", and the registry lists
  the level so installers take a version the game can run.
  `cargo horizon-mod check` prints it ("needs mod API level 1 or newer").
- Horizon follows Minecraft releases; the mod API does not have to. Vanilla
  concepts mods see (dimensions, block and item ids) are the game's own ids,
  so a version port changes the data, not the API.

## Multiplayer

- **Client mods** (`side = "client"`, all of 0.1) run in the player's game,
  in single player and on any server, vanilla or Horizon. They see only what
  the client already knows (the snapshot), like F3 does, and act only on the
  local screen and chat. A server cannot see them.
- **Server mods** (`side = "server"`, API 0.3) run where the world is
  simulated: single player's server thread (or the window, in-frame), a LAN
  host and the dedicated server ([Server mods](#server-mods-api-03-experimental)).
  They use only vanilla messages, so any client, vanilla or Horizon, can join.
- **Both** (`side = "both"`): a server mod with a client half (new blocks,
  their textures, a custom screen). On join the Horizon server announces its
  mods (id, version, SHA-256) on a `horizon:mods` custom payload channel; a
  Horizon client with matching client halves enables them for that session, or
  offers to download them from the same source the launcher uses. Vanilla
  clients are refused only if the server says its mods need a client.
- Registries (new blocks and items) are server-owned and synced like vanilla's
  registry sync, so ids match on both ends.

## Distribution

- **The file.** `.hmod`: a zip holding `horizon-mod.toml`, `mod.wasm` and,
  later, `assets/` and `data/`. `mod.wasm` is capped at 32 MiB.
- **Integrity.** Every download is checked against the SHA-256 the index
  lists before the file reaches `horizon-mods/`.
- **Signing.** Authors sign `.hmod` files with an Ed25519 key (minisign
  format: a `.hmod.minisig` beside it); the index pins each project's public
  key, so an update signed by another key is refused. Unsigned local files
  stay allowed, with an "unsigned" mark on the Mods page.
- **Where mods come from.** Horizon's registry ([Publish your mod](#publish-your-mod)): reviewed pull requests in
  [horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods), files as its release assets, and an
  `index.json` on GitHub Pages that the launcher and the website read. A file the registry takes down
  (`revoked.toml`) is refused by the launcher and turned off where installed.
- **The launcher.** A Mods tab per installation: browse and search the index,
  install, update, turn on and off (it writes the same `horizon-mods.txt`),
  see capabilities before installing. The launcher's CLI and MCP tools gain
  `mods list|search|install|remove|enable|disable`, so agents can set up a
  modded game the same way players do.

## Roadmap

1. Design (this page) and the Java research.
2. SDK, example mod, runtime, `cargo horizon-mod`, the game's Mods page: API 0.1.
3. `storage` (done), `keys`, `target`, `inventory-read`, `ui`: enough for the
   first native ports (zoom presets, AppleSkin-style food HUD, Jade-style
   tooltips).
4. Launcher Mods tab, the index, signatures; the Mods page shows cost and
   update state.
5. `recipes`, `world-read`, `render`: recipe viewer and schematics ports.
6. Server mods, `registries`, `net`, data-driven content from `.hmod`
   `data/` and `assets/`, multiplayer sync.

Modpacks ([modpacks.md](modpacks.md)) move data-driven `registries` first:
API 0.2 is content-only `.hmod`s (blocks, items, recipes, loot, models and
textures), then server hooks (0.3) and block entities, menus, capabilities and
`net` (0.4), in the order the ATM10 inventory needs them.

## Experimental server entities

Server programs may opt into `entities` to register owned kinematic types and create, update or remove
ephemeral host records. The six new functions use provisional local level 9 after resource level 8.
Both collide with draft PR 2888 and require allocation and retesting on the merged substrate before release.
See [the entity contract](api/mod-entities.md) for limits, transactional callbacks and host cleanup.
Stage E adds no rendering, networking, collision, AI or persistence.


## Experimental private program messages

The pinned candidate offers `horizon_mod_sdk::messages` to client programs and
`horizon_mod_sdk::server::messages` to server programs. Declare capability and experimental
opt-in `messages`. The three actual new imports use provisional local level 13, not a
released allocation. Existing guests need no new exports or registration fields.

Client programs call `subscribe(sender, channel)` during init and `poll()` in existing
hooks. Poll returns `Result<Vec<Message>, String>` with
`Message { sender: String, channel: String, payload: Vec<u8> }`. The receiver ID and
subscription epoch come from the host store. A client cannot send an action, target another
connection or choose the ID of a different local program through this API.

Server programs call `send(player, receiver, channel, payload) -> Result<(), String>`.
`player` is an authoritative online player UUID from the existing player API or callback.
The sender is the running program's ID. Delivery requires an exact receiver, sender and
channel subscription on that player's own connection. There is no broadcast or chat
fallback. Subscribe and poll fail on server stores; send fails on client stores.

Payloads are at most 32768 bytes. Each callback allows eight send attempts and 64 total
message calls. Names use ASCII resource tokens, at most 64 bytes per owner ID and 128 per
channel. A client program may hold 32 subscriptions, with 64 across one client runtime.
There are at most 64 queued incoming messages across client programs and 64 queued outgoing
messages for one player, with 256 total unsent deliveries and 4096 server subscription
routes. Queue and subscription failures are ordinary errors. Callers that already saved a
successful action should treat a display-send failure separately from that action.

Send success means bounded enqueue, not a UI receipt or a second gameplay transaction.
Remote inboxes may fill or connections may close before delivery. Hosts stop adding private
payloads to a connection whose existing output exceeds 2 MiB. UI consumers can request a
fresh view through their existing authenticated command path. Binary payloads never enter
player chat or public renderer state.

In-frame delivery uses the same subscriptions and receiver leases as integrated memory and
TCP delivery. Fixed CustomPayload channels carry read-only subscription advertisements,
private deliveries and resets. A subscription packet applies only to its authenticated
connection. World close, respawn, disconnection and program reload clear queues and replace
receiver epochs. Old in-flight deliveries fail the new epoch check. A server reload also
sends a reset to subscribed clients before accepting their new advertisements. Sender store
destruction revokes its unsent deliveries. The program owns payload schemas and UI session
or revision rules; the transport adds no gameplay action authority.

### Experimental physical server inventory

Server programs can opt into `capabilities = ["inventory"]` and
`experimental = ["inventory"]`. The additive import is
`horizon_mod_sdk::server::inventory`. Existing registration and exports are unchanged.
`held` and `give` use provisional local level 15. The additive string read uses
provisional local level 19. The restricted machine integer read uses local level 21.
These are private allocations, not released API numbers.

```rust,ignore
use horizon_mod_sdk::server::inventory::{self, Hand};
let held = inventory::held(&player.id, Hand::Main)?;
let variant = inventory::component_string(&player.id, Hand::Main, "my_program:kind")?;
let inserted = inventory::give(&player.id, "minecraft:stone", 12)?;
```

`held(player: &str, hand: inventory::Hand) -> Result<Option<inventory::ItemStack>, String>`
reads the authoritative selected main hand or offhand at this point in the callback.
`Hand` is `Main` or `Off`; it is separate from the existing interaction hand enum.
`ItemStack` contains only `item_id: String` and `count: u32`. Empty hands return `None`.
The record shape is unchanged. General NBT and inventory removal are not exposed.

`component_string(player: &str, hand: inventory::Hand, key: &str)
-> Result<Option<String>, String>` reads one effective custom string in the same
authoritative hand. The component namespace must be owned by the calling program:
its manifest ID or an explicitly claimed namespace. Keys are valid namespaced
identifiers of at most 128 bytes, and returned UTF-8 strings are at most 256 bytes.
Unpaired UTF-16 surrogates are rejected rather than replaced. Each attempted read
uses the same 16-operation callback allowance as `held` and `give`.

The native item catalog retains custom component prototype values from the loaded
`items.json`. The existing component-patch kernel resolves those defaults with
the actual held stack's saved patch. An explicit `!namespace:key` entry whose value
is an empty compound represents removal. Bare absence of a patch entry inherits
the prototype. A live value together with a removal, duplicate entries or a
malformed removal marker returns an error.

An empty hand returns None. With a fully known prototype, an absent or explicitly
removed component also returns None. An unavailable item prototype, non-string
effective value, oversized string, invalid key, offline player or unavailable
callback context returns Err. A patched string can replace a non-string default,
and a valid removal can remove it. There is no default value guessed for any item.
The read neither changes slots nor grants an action or consumption authority.

`custom_data_integer(player: &str, hand: inventory::Hand, field: &str)
-> Result<Option<i64>, String>` is an additional restricted read under the existing
`inventory` capability and experimental opt-in. It accepts only the fields `tm`
and `Gen9Offset` under the fixed `minecraft:custom_data` root, and only while holding
the registered item `pixelmon:tm_gen9` or `pixelmon:tr_gen8`. This is not a general
NBT path API or an exception allowing arbitrary vanilla component reads.

It reads the same authoritative main or offhand slot as `held`. Empty hands return
None. An unsupported field or non-machine held item returns Err. The catalog retains
the actual loaded root prototype for those two items with the existing native
JSON-to-NBT conversion. The existing component-patch kernel then resolves the
actual saved stack's whole root replacement or explicit
`!minecraft:custom_data` empty-compound removal. Bare patch absence inherits the
prototype. A root replacement does not merge missing nested fields from the
prototype. Duplicate, conflicting or malformed patch entries return Err.

A fully known absent root or absent field returns None. An unavailable prototype,
wrong root type or unavailable server context returns Err. Only actual NBT Byte,
Short, Int and Long values return their exact signed value as i64. Floating-point,
string, list or compound field values are rejected, with no coercion or default
index. The query borrows the root and returns one integer; it neither serializes
nor exposes unrelated custom data. Every attempted call shares the existing
16-operation inventory callback limit, including errors. Old record and export
shapes remain unchanged, and older native inventory implementations default to an
unavailable error.

The existing vanilla CustomData NBT serialization carries the real compound.
This read adds no wire format, numeric component registration or packet. Programs
must still validate exact item identity, number mapping, generation, compatibility
and their explicit reuse policy, then use the existing admitted save and
consumption acknowledgment. No integer value alone authorizes teaching or spending.

Recipe result parsing and ordinary saved stacks already retain custom strings in
`StackData.other`. This API alone does not preserve them over the native wire.
The separately implemented Horizon string envelope uses existing
`minecraft:custom_data` packets; its transport handoff must be present before a
program relies on remote string identity. No numeric component type or new packet
is registered by this read API. Physical actions still require their existing
authoritative stack checks and successful consumption acknowledgment.

`give(player: &str, item: &str, count: u32) -> Result<u32, String>` inserts plain registered
items synchronously into that player's 36 native inventory slots. It shares the existing
native insertion function and registered stack limits, including component-aware merging.
It returns the number actually inserted, including zero when full. Any remainder stays
with the program's accounting and is never queued or dropped into the world. Counts must
be 1 through 1024. Air, unregistered IDs and item strings over 256 bytes fail before insertion.
Each callback permits 16 attempted inventory operations, including errors, and eight
attempted `give` calls. Init has no physical inventory context.

Targets are exact UUID strings from the authoritative online player snapshot. Invalid or
offline IDs and unavailable inventory contexts return `Err`, without insertion. The active
actor uses their actual swapped-in inventory during guest actions. Hosted callbacks and
explicit guest commands can access the available hub players as well. If the hub is lent
to an outer native action, a non-actor's unavailable slots return an explicit context error.
No stale hub shadow is used. Slots are lent only for synchronous callbacks and restored
before normal gameplay resumes. Existing native inventory and open-menu synchronization
carry changes to clients. There is no additional inventory network authority.

Insertion is a physical side effect, not a callback transaction. An inserted item survives
a later callback error, trap or program snapshot write failure. Returning a consume-zero
interaction result permits insertion to change the held stack. Positive interaction
consumption still requires the original held stack to match and consumes only after the
callback succeeds. A program must use the returned inserted count when retaining leftovers.
Native player inventory persists through the ordinary world save. An insertion result is
not a disk-save acknowledgement. There is no atomic transaction or crash recovery guarantee
between player saves and program `world_storage` files, or between multiple files. Existing
world-storage owner admission is unchanged.

Loaded machine state reads remain `server::world::get_block(dimension, x, y, z)` with
`world` capability and experimental opt-in. It returns `Option<String>` with `None` for an
unloaded position. The inventory API does not load blocks or chunks.

This source addition has not been compiled or exercised in a game.

### Experimental server environment context

The existing server `world` capability and experimental opt-in additionally expose
`horizon_mod_sdk::server::world::environment(dimension: &str, x: i32, y: i32, z: i32)`.
It returns `Result<Option<world::EnvironmentInfo>, String>` without loading chunks.
This new import uses provisional local level 16, not a released allocation. Existing
functions, guest exports and registration shapes are unchanged. Old host `WorldView`
providers default to `Ok(None)` and remain source-compatible.

`EnvironmentInfo` has `game_time: i64`, `day_time: i64`, `raining: bool`, `thundering: bool`,
`rain_level: f32`, `thunder_level: f32` and `loaded: Option<LoadedEnvironment>`.
Native levels share the Overworld's stored game time, day time and weather. Time values
are signed stored ticks with no modulo or day/night inference. Flags are stored weather
cycle flags, including in dimensions where they do not imply precipitation. Levels are
native `Weather::levels(1.0)` at the current tick; thunder is scaled by rain. Neither the
flags nor these levels alone mean that rain falls at the queried position.

`None` means the native context or dimension is not lent. A known dimension can return
clock/weather with `loaded = None` for a missing chunk or a Y outside its height.
Invalid X/Z outside 29,999,984 blocks or dimension names over 128 bytes return `Err`.
Each callback allows 64 attempted environment queries, including rejected queries.
Each loaded query scans at most one 384-cell stored heightmap column. It does not prime
a whole chunk's heightmap or run lighting, physics, world generation or networking.

`LoadedEnvironment` contains:

- `block: String` and `biome: String`, from the registered loaded cell and biome container.
- `sky_light: Option<u8>` and `block_light: Option<u8>`, raw stored light nibbles 0 through 15
  only when the native chunk is initialized as lit. They do not include sky darkening or
  guarantee pending relighting has completed. Uninitialized light is unknown.
- `static_collision: Option<StaticCollision>` with `Empty`, `FullBlock` or `Partial`.
  These classify native cached collision shapes. FullBlock is the canonical unit cube.
  Contextual liquids, powder snow, scaffolding and unavailable dynamic shapes are unknown.
  Partial includes other nonempty cached geometry and promises no full supporting top face.
- `fluid: Option<FluidContext>` with `kind: String`, `amount: u8`, `falling: bool`, using the
  native stored block-state fluid parser. Known empty fluid is `minecraft:empty`, amount 0.
  Unmodeled custom fluid behavior or undecodable fluid properties are unknown. There is no
  inferred air, fluid simulation, flow vector or neighboring fluid height query.
- `surface_y: Option<i32>`, one above the highest stored MOTION_BLOCKING state in this column,
  using the native motion-blocking-or-fluid heightmap predicate. An empty column returns the
  native chunk storage minimum, -64. This is not WORLD_SURFACE, a collision support promise
  or a can-see-sky result. It uses the same stored column range as the native heightmap.
- `rain_at_position: Option<bool>`, the native point-rain conjunction: current rain level
  above 0.2, stored sky light 15, MOTION_BLOCKING surface at or below Y and native biome
  precipitation as rain. The biome query follows native `BiomeInfo::snows_at`, including
  its sea level 63 semantics. An independently false term proves false; otherwise unknown
  light leaves the result unknown. Snow and biomes without precipitation are not rain.

These are native stored and template facts. Content aliases retain their declared physics
limitations. Missing or unsupported facts remain `None`; programs must choose eligibility
explicitly and cannot interpret unknown as clear weather, air, daylight or sky visibility.
World storage admission, renderer JSON and sender authorization are unchanged.
This source addition has not been compiled or exercised in a game.

### Native sky visibility

`server::world::sky_visible(dimension: &str, x: i32, y: i32, z: i32)` returns
`Result<Option<bool>, String>` under the existing `world` capability and experimental
opt-in. It is a separate additive query and does not change `EnvironmentInfo`. The new
import has provisional local level 17. Old providers default to `Ok(None)`.

This query follows the existing native mob world's named `Level.canSeeSky` semantics:
Nether returns false, otherwise initialized stored sky light must equal 15. Source evidence
is [MobWorld::can_see_sky](../crates/horizon-client/src/mobs/world.rs#L204), the native
[spawn sky check](../crates/horizon-client/src/mobs/spawn_rules.rs#L108) and the explicit
[BlockAndLightGetter.canSeeSky check](../crates/horizon-client/src/game/trident.rs#L233).
The separate advancement and stat adapters use bounded column approximations; this API
follows the mob and spawn light query rather than adopting those approximations.

The SDK host first requires a lent dimension, an in-height position, a resident chunk and
the native chunk's initialized `lit` flag. Missing or uninitialized data returns `None`,
including in the Nether. The ordinary world's missing-chunk full-light fallback is never
used. It scans no columns, loads no chunks and uses no surface-height, collision, weather
or time inference. It shares the existing 64 attempted environment queries per callback
and the same coordinate and dimension-name bounds.

This is the native stored-light visibility result, not a new independent roof simulator
or a light-job completion acknowledgement. Stored light can still await native relighting.
It does not promise a new vanilla-parity result. Programs should retain `None` as unknown.
The source was not compiled or exercised in a game.

### Experimental local sound playback

Client programs can use `horizon_mod_sdk::audio` with `capabilities = ["audio"]` and
`experimental = ["audio"]`. It is client-only and adds no required guest exports.
The new import has provisional local level 18, not a released SDK allocation.

```rust,ignore
use horizon_mod_sdk::audio::{self, Category, Position};
audio::play("pixelmon:pixelmon.mob.bulbasaurf",
    Some(Position { x: 1.0, y: 65.0, z: 2.0 }), Category::Neutral, 0.5, 1.0)?;
```

The exact signature is
`audio::play(event: &str, position: Option<Position>, category: Category, volume: f32, pitch: f32)`
returning `Result<(), String>`. `Position` has `x`, `y`, `z` as `f64`. Categories are
`Master`, `Music`, `Record`, `Weather`, `Block`, `Hostile`, `Neutral`, `Player`, `Ambient`,
`Voice` and `Ui`, mapped to the native SoundSource order. `Some(position)` uses native
positional attenuation; `None` is listener-relative without attenuation.

The event must be an exact registered namespaced ID in the active merged resource root,
not a path. Existing content overlays already carry packaged `assets/` into that root.
The native sound registry reads `assets/<namespace>/sounds.json`; its declared sound names
resolve to `assets/<namespace>/sounds/<path>.ogg`. For example, JSON key
`pixelmon.mob.bulbasaurf` in `assets/pixelmon/sounds.json` registers event
`pixelmon:pixelmon.mob.bulbasaurf`. A declared sound name `pixelmon:pixelmon/bulbasaurf`
resolves `assets/pixelmon/sounds/pixelmon/bulbasaurf.ogg`. That event exists only if the
content actually provides the declaration. This SDK stages no assets and accepts no
filesystem paths, resource bytes or arbitrary decoder requests.

Each callback allows eight attempted calls including errors. At most 64 program requests
are queued across the client. IDs are at most 128 bytes, volume is finite 0 through 1,
pitch finite 0.5 through 2 and each position coordinate finite within 30 million blocks.
A current client world and native output are required. Unknown events, disabled output,
audio-less builds, absent worlds and exhausted bounds return ordinary `Err` values.
Init outside a world cannot play sounds. Master/category volume settings and existing
native no-audio/device behavior remain in effect.

Success means accepted into the bounded queue, not decoded, played or audible. Native
output routing rechecks registration and uses the existing SoundEngine event resolution,
random selection, decoder, cache and mixer. It suppresses positional network capture so
program audio stays local. Missing files, silent event choices, decode/device failures,
muted settings or native channel limits can still prevent sound. No audibility or decoder
completion acknowledgement is exposed. Later callback errors do not promise cancellation
of an accepted request. Requests from stopped or replaced program owners are dropped.

World close, disconnect and world-session changes drop queued requests. Resource registry
changes drop pending requests as well. There is no program-owned decoded cache or second
audio pipeline, no binary chat and no new server authority. Existing gameplay, renderer
JSON, sender authorization and world-storage admission are unchanged.
This source path has not been compiled, decoded or played.

### Experimental read-only entity collision probes

Server programs with the existing `entities` capability and experimental opt-in can
call `horizon_mod_sdk::server::entities::probe_colliding`. Only this new function
uses provisional local level 20. Existing registration records, movement records,
guest exports and `move_colliding` behavior are unchanged.

```rust,ignore
let accepted = entities::probe_colliding(&handle, &dimension, &expected, &delta)?;
```

The signature is `probe_colliding(entity: &str, dimension: &str,
start: &entities::Transform, delta: &entities::MovementDelta)
-> Result<entities::MovementResult, String>`. The expected transform includes
finite x/y/z and yaw/pitch. Its coordinates, angles and dimension must equal the
actual owned record's current baseline. There is no tolerance, angle normalization
or implicit relocation. A stale, foreign or unloaded handle, mismatched baseline
or unavailable collision context returns Err.

The probe reads the callback's existing owned transaction when one is already
present, otherwise the published owned record. It never creates a transaction,
stages a record or changes a record's transform, revision or lifecycle events.
Only the existing callback attempt counters advance. Probes share the eight
collision attempts with `move_colliding` and the 64 entity operations with all
entity calls, including errors. Delta must be finite and at most four blocks per
axis. Dimension input is at most 128 bytes.

The same lent native `WorldView::move_colliding` read-only implementation resolves
the registered type's AABB against resident block collision shapes. No new movement
algorithm is added: step height stays zero, and the existing scan bounds, unloaded
chunk errors and unsupported contextual collision errors apply. Result placement
is validated but not applied. The on_ground result is the existing tiny downward
support probe, not a promise about fluids, full footprint support or future ticks.

For persistent movement, prepare eligibility and dry support separately, check the
native baseline against the program's durable record, probe, validate the accepted
result and save the candidate through the admitted world-storage path. Only after
that save succeeds should the program publish the saved transform with existing
`set_state`. An error or failed save must hold the durable pose. A failed later
publication can leave the native baseline stale; another probe then rejects until
ordinary publication restores that same durable baseline. It must not trigger a
direct-position movement fallback.

Existing `move_colliding` stages an update in the callback's entity transaction.
Transaction commit follows callback success and is independent of a program's
world-storage acknowledgment. It therefore cannot serve as a pre-save planning
query. Probe success also does not make program save and later publication one
atomic transaction. This source slice has not been built or run.
