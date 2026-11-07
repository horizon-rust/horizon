# Server simulation ownership and region execution

The server runs region-owned mob transactions on a bounded persistent pool,
then commits them in the original entity order. Independence is checked from
actual observations and shared RNG checkpoints. A region boundary never hides
a target, collision candidate, owner, projectile, or player.

## Ownership and lifecycle

`game/simulation.rs::LevelSimulation` owns the mutable world/chunks, ordered
entity families (mobs, projectiles, vehicles, items, experience and decorations),
block tick queues, chunk tickets, spawn scratch, death bookkeeping, region
indexes and outgoing level/entity events. `Game` and `levels::Parked` hold this
same owner. Dimension swaps move it as one value. Existing typed raid, sculk,
portal, streaming and save services retain their existing coordinated phases
and dimension-specific swaps. Read-only registries, collision tables, recipes,
biomes and mob types are borrowed from `Resources`/`PackResources`.

`ServerCoordination` owns the dimension manager, server-wide entity IDs, the
entity admission cutoff and one lazy persistent pool shared by all levels.
The mutable level owner is `Send`; the immutable `TickResources` contract is
`Send + Sync`. Native callbacks are excluded from worker inputs.

`server_regions::Regions` groups eight-connected resident chunks. Its identity
is the minimum chunk coordinate in the component, valid until topology changes.
A loaded bridge merges regions; unloading it splits them. The chunk-map
membership revision also detects resets and replacement. Block changes alone
do not rebuild topology. Resident connectivity defines ownership, not proof
that two simulations commute.

Topology follows `ChunkMap::deltas_since` a chunk at a time. An added chunk joins
or merges the regions around it. A removed chunk keeps its region whole when the
chunks around it still connect among themselves; when they don't, the region may
have split, and the whole map is rebuilt. So is a reset or a gap in the deltas.
Legacy mode and automatic mode below its mob threshold keep no regions at all.

Every entity retains its global insertion slot. Region-local entity indexes
preserve the filtered order of those slots. At the tick barrier an entity belongs
to exactly one resident component, or to none when its chunk is unloaded; the
coordinator's walk reaches it either way. Movement may cross owners during the tick; ownership is
frozen until the next barrier, when migration, removals and new entities are
indexed together. Migration never loads or retains a chunk. Existing admission,
entity-ticking ticket, despawn, portal, persistence and save rules remain in
charge. Concatenating region lists would change interleaved entity ordering;
the coordinator always walks the original global list.

## Transaction and publication barrier

1. Complete the existing spawn/raid and native pre-tick phases. Refresh topology
   and local indexes, and build the ordered `Seen` snapshot and spatial grid.
2. Reserve level-owned EntitySection insertion order in global entity order.
   Build immutable player indexes and per-mob difficulty/despawn contexts.
3. Give each job exclusive ownership of its mutable mob inputs. Workers borrow
   world/chunk/resources; they do not clone the world or its chunks. A sparse
   overlay contains only that region's updated `Seen` entries and grid cells.
4. Tick each region in its original local order, including AI, navigation,
   effects, movement, collision, damage and despawn decisions. Same-region
   pushes and single-target brain writes reach later local inputs immediately.
5. Join all jobs. Walk global entity slots and validate each prediction's
   observations, same-region prefix and level RNG checkpoint. Commit an
   unchanged transaction by moving its resulting mob/events into place.
   Otherwise run that mob serially against the current authoritative inputs
   and invalidate the rest of that owner's predicted prefix.
6. Apply immediate pushes and brain events at the existing point in that walk.
   Complete deaths, loot, event effects, projectiles and replication in their
   original coordinated phases. Clients only see completed authoritative state.

No new cross-region delayed-event queue is used. A replay has no externally
visible speculative events. The full mutable-mob tests compare every field,
event sequence and final RNG against the retained serial execution, including
breeding, overlapping owners and nectar-bearing bees that consume level RNG.
The serial reference also runs against a complete snapshot without the spatial
index, checking that indexed execution preserves the filtered results.

`SeenView` prevents untracked raw-slice observations. ID lookups record slots;
full scans record a complete-snapshot read; spatial queries record exact grid
columns, including empty columns. An earlier foreign entity entering an empty
queried column invalidates a prediction. Bounds growth, retained empty cells
and a change between indexed and full-scan queries also invalidate it. No
fixed margin or maximum entity speed is assumed. Sparse grid overlays preserve
the original candidate order and legacy full-scan fallback rules.

Audited box-filtering callers use `others_intersecting`, which enforces the
exact intersection predicate at its API. Position-range searches retain their
inclusive range predicate. For these bounded queries, a sparse lookup scans
populated cells when that is cheaper than walking empty columns, preserving
global slot order without declaring the entire snapshot observed. The query
extent comes from the requested geometry and actual maximum body width, not a
region margin. Validation likewise scans changed populated cells for very large
areas, so an enormous query cannot cause an unbounded empty-cell walk. Legacy
candidate-only callers without an equivalent geometric predicate retain their
existing scan and conservative dependency rules.

Species-filtered observations have their own ordered index and read set.
Zombie village searches and villager POI ticket scans can read all villagers
without falsely depending on unrelated zombies. A foreign member's changed
state, removal, or a change into/out of the queried species invalidates that
read, including a query that originally found nobody. Same-region membership
changes are supplied by the sparse overlay in global slot order. Genuine global
villager POI dependencies therefore remain coordinated through validation and
replay; an empty villager population does not serialize every zombie region.

The player directory uses exact distance searches over all players. Local
players seed the search, but pruning uses distance bounds over the complete
tree. Original host/guest tie order, dead/creative/spectator filtering, bound
targets, sleeping owners and lead holders are preserved; nonfinite inputs use
the original ordered scan.

## Cross-region dependency audit

| Dependency | Coordination |
| --- | --- |
| Level RNG | Each job starts at the same checkpoint. A result that consumed it commits only at that exact authoritative stream state, including cached Gaussian state; otherwise serial replay consumes the original draws. A result with no level draws never overwrites the authoritative stream. No per-region seed is substituted. |
| Mob observations and collisions | The complete snapshot remains visible. Actual slot, spatial, species and full-scan reads are validated against earlier foreign writes; local overlays preserve immediate local observations. |
| Pushes and brain memories | Pending pushes retain event order. Known single-recipient writes are applied to later owned inputs; foreign recipients invalidate their owner's prefix. Riding and school-follow changes conservatively invalidate all prefixes and run the remaining conflicting work serially. |
| EntitySection insertion | A per-level allocator reserves order before jobs run. Rejected work cannot consume a process-global insertion counter or change candidate ordering. |
| Raider hurt ordering | Raider species and active raid participants stay serial because their hurt sequence remains globally coordinated. A worker stops before that entity; the coordinator runs it normally. |
| Native mods and tracing | Loaded native mods, entity tick listeners and RNG tracing retain serial execution. Arbitrary callbacks are never speculated. Content-block contacts keep their existing server-thread scope. |
| Projectiles and vehicles | The existing ordered hit, homing, passenger and steering phases operate on completed mob state. Their dependencies can span any region and are not declared independent. Projectile admission/join rules are preserved. |
| Explosions, block writes and navigation | Existing explosion, damage, block and event handlers remain coordinated. Navigation sees the same borrowed world and collision-change list during the mob phase. No explosion radius is used as an independence margin. |
| Spawning and caps | Natural/custom spawning, raids, pack finalization, global/per-player caps, IDs and their shared RNG stay in their existing serial order before entity transactions. Spawn scratch follows its level owner. |
| Death, despawn and drops | Existing ticket/despawn checks, corpse removal, death effects and loot processing surround the validated mob tick at the original points. Non-ticking despawns invalidate affected observations. |
| Replication and chunk lifecycle | All workers join before publication, streaming application or unload can mutate the borrowed world. Existing event buffers and entity tracking publish committed results, including authoritative replay results. |

Sound ID interning uses persistent-thread local caches instead of process-wide
mutexes in living-entity ticking. There is no global mob-loop lock. Worker
results contain owned mob state and events; there are no unsafe world pointers.

## Execution controls

Automatic mode uses region transactions for at least 128 mobs in at least two
resident regions. Small or connected workloads retain the direct serial path.
The threshold only controls scheduling overhead; it never limits interactions.

A region that wastes its predictions (half or more thrown away, four rounds in a
row) sits out in automatic mode: 20 ticks, doubling up to 640 while each new try
is wasted too, and forgiven by a round that commits. Its mobs run on the
coordinator meanwhile, exactly as a replay does, and other regions keep their
workers. This only changes cost. Level RNG draws (school leaders, nectar bees,
villagers) and shared villager reads are the usual causes. The forced modes below
never sit out, so that they test everything. How much this saves on a mixed
population has not been measured; `tools/bench_region_ticks.py --species
zombie,cow,villager` runs one.

- `HORIZON_SIMULATION_MODE=legacy`: retained direct serial mob loop.
- `HORIZON_SIMULATION_MODE=serial`: region transactions on the server thread.
- `HORIZON_SIMULATION_MODE=parallel`: force region transactions, including small
  test scenarios.
- `HORIZON_SIMULATION_MODE=verify`: `parallel`, and every commit is also replayed
  on the coordinator from the same mob and RNG. A different mob, event list, RNG
  or keep decision panics and names the first difference. One draw is repeated
  on purpose: a long jump's angle shuffle is unseeded, as in vanilla, so this mode
  seeds it from the mob and its target (`mobs/frog.rs`). Without that,
  `dedicated_server` failed here on every run. Other modes keep the unseeded shuffle.
  The check catches what the
  read tracking can't see: a new thread-local, a global counter, or a `MobWorld`
  override only one of the two paths sets. CI runs the server harnesses this way
  (`tools/ci/region-verify.sh`).
- `HORIZON_PARALLEL_COMMIT=0`: every round takes the serial walk, with no
  classification. Any other value, or none, leaves clean rounds on the workers
  ([below](#parallel-commit)). `verify` runs a clean round on both paths and
  compares them; `serial` has no workers, so it always walks.
- `HORIZON_SIMULATION_WORKERS=1..8`: bounded pool size; defaults to the project's
  worker-count policy, capped at eight. Pool construction failure falls back
  to serial execution.
- `HORIZON_REGION_METRICS=1`: region, predicted, committed and replayed counts,
  and whether the round was clean or fell back, with the reason.

What a prediction replaces is guarded in debug builds too: `TickRound::commit`
panics when the walk wrote to the mob since its prediction was made. Every event
that writes to another mob says so in `MobEvent::immediate_write`, an exhaustive
match that the prediction, the invalidation and `apply_brain_events` all read.

A panic during the mob tick leaves the mob list in `MobRescue`, and the crash save
takes it back, so the save doesn't write the level's entities as if it had no mobs.

## Parallel commit

The commit walk (step 5 above) costs about 7 ms at 100 players and most of a
tick's mobs have nothing for it to decide. A round that is provably independent
skips it: each mob is applied from its own prediction on the workers, and only
what needs an order runs on the coordinator. Everything else walks as before.

What each step of the walk can observe, and where it goes in a clean round:

| Step | Reads or writes | Clean round |
| --- | --- | --- |
| Loaded, ticking and canceled gates | The mob's position, tickets, native callbacks | Decided: a mob they stop has no prediction, which makes the round not clean; native mods already keep the round off the workers |
| `apply_mob_pushes` before the commit | Pushes from earlier slots | Not run: only same-owner pushes are allowed, and the target's prediction applied them, so the walk's copy would be swapped out anyway |
| `blocks_changed` | The tick's collision changes | Not run: the prediction did it |
| Validation (`conflict`, RNG checkpoint) | Earlier foreign writes, the level random | Replaced by the classifier, which is stricter (below) |
| Swap, `Committed` | The mob and its prediction | Worker |
| `bindings` | Own id only | Worker decides, coordinator writes the changes in slot order |
| `events` (cross-owner pushes void an owner) | Other owners | Not reachable: cross-owner pushes aren't clean |
| Corpses, `gone`, `dead`, `died` | Own mob, own id | Worker decides, coordinator clones and records in slot order |
| `take_mob_pushes` | Own events | Worker; those at a passed slot are merged in slot order for the end of the walk |
| Shulker bullet target, guest events | Own events, guest ids | Worker; guest events are merged in slot order |
| Snapshot entry | Own slot | Worker |
| `grid.update` | Shared cell lists, `reach` | Worker decides, coordinator updates only the mobs that changed cell or reach, in slot order, so the lists come out identical |
| `snapshot_changed` | The round's change log | Not needed: nothing validates after the classifier |
| `apply_brain_events`, `immediate_brain_write` | Other mobs, `failed` | Not reachable: any event that writes to another mob isn't clean |
| Event order, RNG, section order | Slot order; level random; reserved per slot before the jobs | Merged in slot order; a clean round draws nothing; unchanged |

**Classifying.** Before anything changes, the workers look at each region's
predictions against tick-start state, and the coordinator joins what they find.
A round is clean only when all of these hold:

- Every mob has a prediction. No mob is unloaded, outside the entity-ticking area
  or a raider, no region is sitting out and no prediction was cut short.
- No prediction drew from the level random.
- No event writes to another mob (`MobEvent::immediate_write`). This counts writes
  inside the region too.
- Pushes only aim at mobs of the same owner.
- No read meets a write of any other owner's prediction, whichever slot it came
  from: a full scan, a slot, a species or a queried grid column. Query shapes are
  checked against the round's final reach and an upper bound on its cell count.
  The walk's grid always lies between the prediction's and that one, and a shape
  only grows with them, so a shape that holds on both holds throughout.

These are stricter than validation, which only knows the slots already walked, so
a clean round is one the walk would commit whole. Any other round runs today's
walk from the start: nothing has been applied, so there is nothing to undo. The
walk also stays for `executor()` returning `None` and for pools with one worker.

**The cost of falling back.** A round that fails on the first check pays nothing.
One that fails later pays the classification and then the walk. How much this
costs on a workload that falls back every round has not been measured.

**Limits.** This only helps levels whose every mob is predicted, every tick. One
mob outside the entity-ticking area sends the whole round to the walk, and a
server's view distance is normally larger than its simulation distance, so mobs
can stand there. Level RNG users (school leaders, nectar bees, villagers) and
breeding do the same. The benchmark's zombies avoid all of these, so its result
is the best case. The server harnesses are nearer the other end: in verify mode,
`dedicated_server`, `server_entities`, `server_player_damage` and `natural_spawn`
together had 2 clean rounds and 1,078 that fell back as `Unpredicted`. How often
a real server's rounds are clean has not been measured.

`clean_commit_matches_the_serial_walk` runs the same scenes through the real
`tick_creature_entities` on the walk, the clean path and verify mode, and takes
the walk's result as expected: mobs, snapshot, grid, level random, keep list,
events in order, guest events, deaths, pushes, bindings and stats. Its scenes are
a clean round with crowds pushing each other, a cross-region read, a brain write
to another mob, a death and a removal, a guest hit and a mob with no prediction.
It needs prepared resources, so it is `#[ignore]`d: set `HORIZON_TEST_RESOURCES`
and pass `--include-ignored`. The classifier's own tests need no data. In verify
mode, a clean round also runs on a copy and is compared with the walk's result,
but the CI harnesses reach that in very few rounds (see the limits above), so
this test is the clean path's main coverage. The benchmark's clean rounds were
not compared with a walk.

## Coordinator, portal and sculk work

The coordinator's own time went to walks over every mob, each a cache miss
per mob. Now game events and melee hits find their listeners and mob directly,
states and entity sends are built on the workers, a mob's rarely used fields
sit behind a box so a clone copies less, and the scans a tick repeats run on the
workers (`Game::mob_slots`) or not at all. The replaced mobs are freed by one
worker at the end of the tick, so those scans do not queue behind the frees.

What changed, with gameplay meant to stay the same. Without workers
(`executor()` returns `None`), both worker passes keep the serial code:

- `entity_hurt_player` walks every mob for passengers only when one of the
  trigger's criteria names `passenger` at any depth, or a `minecraft:reference`
  that might. Other callers of `mob_entity` keep the whole view.
- The mobs' part of the `find_entity_portals` pass sweeps on the workers over a
  read-only view. Workers read the kept palette answers, scan a chunk that has
  none, and leave its key for the coordinator to store afterwards. Other entity
  kinds and the ridden-vehicle pass stay serial. Mobs come first in the walk, so
  the inside list keeps its order.
- `tick_mob_sculk_events` classifies each mob on the workers once the tables are
  built. A quiet mob only stores its new `Steps`; the rest run the existing code
  in list order. After a block write or a chunk membership change
  (`World::revision`, `ChunkMap::membership_revision`), every remaining mob runs
  the existing code. Main's game-event rework (`6b826d204a`), merged here on
  2026-11-06, replaced that pass: `tick_mob_sculk_events` is serial again, and
  this classification and its test are gone.

The pool-against-serial tests need prepared resources, so they are `#[ignore]`d:
`pool_and_serial_find_the_same_portals_in_the_same_order` and
`entity_hurt_player_reads_riders_only_when_a_criterion_can`. Set
`HORIZON_TEST_RESOURCES` and run them with `--include-ignored`.

## Output preparation audit (PR #8666 follow-up)

The mob barrier protects borrowed world data. `tick_lan` is not a replication-only
phase: `tick_guests` finishes mining, item use, pickups and vehicle portals before
`lan_entities` describes the committed state. Moving that phase ahead of mob
commit changes which mob/world state player actions observe. Moving streaming
insertion or unload across prediction changes the borrowed collision/navigation
world. Neither is an independent job.

Networking already overlaps simulation: `lan/io_pool.rs` owns a fixed socket pool
and performs framing, shared compression, encryption and writes. Each connection
stays on one worker, with a 96 MiB pending-byte disconnect limit and 256 KiB framed
lookahead. `PlayerSender` holds ordered packet bodies until the existing flush.
Chunk section/heightmap warm-up and body encoding already use the chunk encoder's
workers; generation, lighting and meshing use streaming workers. Mob entity
snapshot construction and entity change/pairing encoding already use the region
pool. Adding another encoder pool would compete with these workers on six cores.

The rejected follow-up experiment moved viewer visibility selection to the
existing pool.
Each viewer owned its tracker, removal-event list, packet buffer and chunk-test
scratch; jobs only read the committed entity probes, section changes and chunk
membership. They joined before reducing new-pair flags and encoding entity sends.
The per-viewer removal packet still preceded pairing and movement packets, and
actual queuing/publication stayed at the existing phase. The scoped pool call
bounded outstanding work by that tick's viewers, propagated panics after joining,
and retained no world, generation or pending result across ticks. Serial mode and
duplicate entity IDs retained the serial path. It used no speculative replication.

New elapsed spans separate `lan_player_gameplay`, `lan_entity_snapshot`,
`tracking_prepare`, `tracking_visibility`, `tracking_encode`, `lan_output_queue`,
`chunk_output_prepare` and `chunk_output_queue`. Streaming separates application
of completed jobs, unloading and preparation of new jobs. These are nested spans;
add neither children to parents nor asynchronous worker CPU to wall time.
`tracking_encode` includes its scoped wait and per-viewer buffer distribution;
socket compression is already off-thread and is not attributed to this span.

The viewer-parallel experiment was removed: repeated 100- and 200-player
comparisons showed no whole-tick benefit. The chunk-range overlap below is a
separate implementation; the rejected visibility path remains removed.

`HORIZON_REGION_PROFILE=1` additionally logs three **sums of worker elapsed
milliseconds**, once per prediction barrier: input cloning/pending writes
(`prepare`), player/world-view construction and the mob tick (`simulate`), and
push/brain-write classification plus snapshot/grid/result construction
(`observations`). These sums can exceed tick duration because jobs overlap;
they are not CPU times or new children to add to `mob_predict`. Profiling adds
clock reads per mob only when explicitly enabled. Normal execution instantiates
the same loop without those clock reads. No worker-local span buffers accumulate.

Dedicated reports now include `queues_at_end`: pending network bytes (total and
largest connection) and the active level's streaming-stage counts. They are end
samples, **not** window peaks or backpressure-event counts. `network_compression`
reports actual compression calls (shared-frame cache hits excluded) and summed
worker elapsed milliseconds while spans were enabled. Counter snapshots at the
window boundaries are approximate; compression includes building frame contents,
not socket delivery latency. The ordinary disabled path adds no clock reads.

## Overlapping chunk-range preparation with mob prediction

The follow-up after `c0b066072` uses the coordinator's prediction wait to prepare
full-chunk range predicates. `RegionExecutor::predict_with` dispatches prediction
into the **existing** pool with `in_place_scope`. The coordinator computes an
owned `RangePreparation` while the workers simulate mobs, then joins the scope
before any authoritative commit. `HORIZON_REGION_OVERLAP=0` runs the same
preparation after prediction for an ablation; it keeps the prepared answers and
all application/publication points identical.

This is specifically the `in_range(x, z, 1)` predicate used when streaming
considers finishing proto chunks. Other ranges, priorities, decoration selection,
world insertion, unload, chunk submission and replication stay at their original
points. The prepared answers also avoid repeatedly checking the same view
geometry in the later streaming loop. The whole improvement therefore includes
both cheaper preparation/application and overlap; the ablation separates them.

The job owns coordinates and integer geometry: centre/radius, extra player view
areas, forced/mod-forced tickets and the pregeneration band. It owns no world,
chunk, mob, player or native-callback reference. It does not change RNG, entity
order or callback state. The snapshot is limited to 1,024–65,536 proto coordinates,
2–1,024 view areas and at most 64 forced/mod-forced entries, with at least 512
predicted mobs. Other cases retain the original path. It adds no threads or
persistent queue: one preparation per level is joined within this tick.

At the existing streaming phase the complete geometry basis is compared with
current values. Changed player views/tickets/pregeneration invalidate and discard
all prepared answers. New coordinates use the original predicate; removed chunks
still pass through current membership and lifecycle checks. No membership or
world-content fact is cached, so membership changes alone require no stale-world
retention or whole-world/chunk clone. Lookups are enabled only inside the validated
streaming pass, and the result is freed at its end (also discarded when paused).
No speculative state is published and no extra tick of latency is introduced.

`streaming_range_capture` and `streaming_range_prepare` measure coordinator work;
`mob_join_wait` measures the interval after preparation until the scope joins.
The latter is elapsed join time, not another cost to add to `mob_predict`.
When profiling is enabled, `streaming range preparation` log records report
accepted/discarded results and their coordinate count.

Tests compare prepared predicates against the original unindexed geometry across
range boundaries, forced tickets and pregeneration bands; reject each changed
geometry component; and cover missing coordinates. A channel-handshake test
requires coordinator work to execute before prediction can finish. Both panic
directions test joining and subsequent pool reuse. Full mob transaction comparisons
now exercise scoped prediction, including conflict/replay and shared-RNG cases.

API audit: inspected only Rayon-core's `in_place_scope` declaration at
[`7af20d7`](https://github.com/rayon-rs/rayon/blob/7af20d7692d5decbcb4adcaa079cd607e3e50814/rayon-core/src/thread_pool/mod.rs),
under its MIT licence. No implementation was copied or adapted.

## Validation

The focused tests cover topology/bridge merge and split, stable ordering,
negative/extreme chunk coordinates, migration/removal/unload/dormancy, exact
sparse-grid equivalence, chunk-by-chunk topology against a whole rebuild,
negative-space reads, distant ID/full-scan reads, foreign bounds growth, sparse
populated-cell queries, shared RNG/Gaussian consumption, complete mutable-mob
transactions and exact indexed player selection.

Checks on `32ec9e94cc`, which merges main at `5a83184c7e` (2026-11-07):

- `tools/pr_ready.py --quick` passed: formatting, clippy with warnings denied
  (including the server build), 7,471 workspace Rust tests, 718 Python tests,
  27 content-root tests and the launcher's checks with 738 tests. Its scope
  check warns that the branch is 27,170 lines of code against a 500-line guide.
- The full `tools/pr_ready.py` was stopped during its harness step, at the
  owner's request; every step before it had passed.
- `tools/ci/region-verify.sh` passed: `dedicated_server`, `server_entities`,
  `server_player_damage` and `natural_spawn` in verify mode, then 20
  `mobs::region_tick` tests, `in_range_follows_geometry` and the four
  resource-backed gameplay tests.

The harness sweep is incomplete. On the merge commit before the fix below
(`26cbe95788`), the full `pr_ready` ran 396 of the 513 harnesses before the
run was killed: 384 passed and 12 failed. Three of the twelve came from this
branch: `raid_village`, `boat_leasher` and `mob_quad_leash` failed because
`Game` still had a second, empty `persist` beside `LevelSimulation::persist`,
which code merged from main read and wrote (armor stands and paintings saved
their raw NBT where saves never look; TNT owners, raids, advancements and
guest actions found no UUID). `32ec9e94cc` removes it, and the three pass.
Two more pass on `32ec9e94cc` when run alone: `command_exec` (three fire
`fill` mismatches in the sweep) and `bed_explosion` (two knockback values
0.01 apart in the sweep, a timeout waiting for entity data on a rerun beside
other runs). `frame_map_writes` runs `command_exec`'s driver in the same
`.local/command-exec-parity` folder, so the two can't run side by side. The
other seven fail on main at `5a83184c7e` too; they are listed under failures
below. The 384 passes predate the fix, which changes 18 gameplay files, and
117 harnesses were not run.

## Benchmark

`tools/bench_region_ticks.py` starts real `horizon-bots` clients, teleports
them onto a square grid `--spacing` blocks apart in a fixed-seed flat world, and
summons 70 persistent, invulnerable, AI-enabled zombies around each player in
interleaved global order. Natural spawning and random block ticks are off and
the players have resistance, so the population stays the same. Setup runs while
the server is frozen; both engines then step the same 200 warmup ticks before
a 20-second measurement. Each run gets a fresh world and scratch user folders.
The script rejects a run whose player count changes, or, for standing players,
whose mob count changes. It records whole-tick average/median/p95/p99 from the
server's own report, the mob phase, process-tree CPU and RSS, load averages and
the region counters. `--behaviour walk` has each bot wander within 48 blocks of
its teleport, on the ground, at 5.6 blocks a second.

Chunks stay loaded two chunks past the view distance N around each player
(`lan_update_streaming` keeps `view_distance + 1`, finished one further), a
square 2N+5 chunks across. Players are separate regions only when `--spacing`
is above (2N+6)×16 blocks.

```sh
cargo build --profile fast -p horizon-client -p horizon-client-lib -p horizon-bots \
  --bin horizon-client --bin horizon-ctl --bin horizon-bots
# main's merge base, built the same way into its own target directory
python3 tools/bench_region_ticks.py --bin target/fast --before-bin <main>/target/fast \
  --modes legacy,parallel --workers 8 --mobs-per-player 70 --warmup 10 --seconds 20 \
  --view-distance 16 --simulation-distance 10 --spacing 640 --settle 180 \
  --behaviour stand --players 100
```

### Measurements (2026-11-07)

Before is main at the merge base `5a83184c7e`; after is this branch's code at
`32ec9e94cc` with `HORIZON_SIMULATION_MODE=parallel` and eight workers. Both
use the `fast` profile and Rust 1.99.0 on a Ryzen 5 5600X (6 cores, 12
threads), with the bots on the same machine. Each case ran once per binary,
main first, on a desktop shared with other idle sessions (load averages 0.6 to
3.8 at the start of each run). The full results are in
[`performance/server-regions.json`](performance/server-regions.json).

**View distance 16, simulation distance 10, 640 blocks apart, 180 s to load.**
Every run kept all its players and mobs, finished loading before measuring,
and committed every prediction, with no replay and no fallback round.

| Players | Main avg/p50/p95/p99 ms | Branch avg/p50/p95/p99 ms | Avg change | TPS | Mob median ms | CPU % | Mean RSS MiB | Regions |
| --- | --- | --- | ---: | --- | --- | --- | --- | ---: |
| 50 standing | 43.52/42.96/46.89/51.46 | 28.73/28.26/31.12/37.45 | −34.0% | 20.00 → 20.02 | 25.5 → 11.7 | 104 → 272 | 771 → 832 | 50 |
| 100 standing | 100.42/99.22/108.85/116.92 | 66.76/65.53/74.93/85.22 | −33.5% | 9.95 → 14.96 | 54.5 → 25.2 | 104 → 346 | 1,273 → 1,363 | 100 |
| 200 standing | 237.16/232.74/266.08/280.49 | 146.44/142.93/174.40/183.33 | −38.3% | 4.21 → 6.82 | 121.2 → 52.1 | 109 → 331 | 2,263 → 2,402 | 200 |
| 100 walking | 121.72/119.02/137.01/149.27 | 158.12/153.58/183.51/191.18 | **+29.9%** | 8.21 → 6.32 | 68.0 → 102.6 | 119 → 153 | 1,378 → 1,490 | 3.3 |

The walkers move up to 48 blocks (three chunks) from their spot, which closes
the three-chunk gap between their loaded squares, so the 100 players formed
3.3 regions on average. Nearly all the mobs then belong to one region, which one
worker ticks in order, and the transaction's prediction and commit cost comes
on top. The branch is slower than main there.

**The same at 576 blocks: one connected region.** 576 blocks is 36 chunks, one
less than a loaded square, so neighbouring squares touch and every player's
mobs belong to one region.

| Players | Main avg ms | Branch avg ms | Change | TPS | Regions |
| --- | ---: | ---: | ---: | --- | ---: |
| 50 standing | 44.11 | 58.86 | **+33.4%** | 20.00 → 16.97 | 1 |
| 100 standing | 94.58 | 126.58 | **+33.8%** | 10.56 → 7.89 | 1 |
| 100 walking | 114.40 | 151.21 | **+32.2%** | 8.74 → 6.61 | 1 |

The 200-player run of that sweep (211.41 → 124.26 ms, 200 regions) is left
out: its 60 s settle was too short, chunks were still generating at the end, and
its squares had not grown into each other yet.

**View distance 2, simulation distance 2, 512 blocks apart**, the workload of
earlier versions of this page. Every run kept all its players and mobs.

| Players | Main avg/p50/p95/p99 ms | Branch avg/p50/p95/p99 ms | Avg change | TPS | CPU % | Mean RSS MiB | Regions |
| --- | --- | --- | ---: | --- | --- | --- | ---: |
| 50 standing | 35.09/34.88/36.84/38.76 | 20.89/20.69/22.62/23.62 | −40.5% | 20.01 → 20.02 | 139 → 326 | 367 → 435 | 50 |
| 100 standing | 79.29/78.90/83.28/85.71 | 46.58/46.34/49.15/50.99 | −41.3% | 12.60 → 20.00 | 104 → 425 | 491 → 576 | 100 |
| 200 standing | 181.02/180.36/190.05/193.54 | 96.11/95.89/100.97/102.40 | −46.9% | 5.52 → 10.40 | 109 → 440 | 711 → 868 | 200 |
| 100 walking | 93.18/93.39/100.15/102.37 | 55.09/53.33/59.62/130.93 | −40.9% | 10.72 → 18.13 | 114 → 371 | 534 → 624 | 98.5 |

The standing runs committed every prediction in clean rounds. In the walking
run every round (366 of 366) fell back as `Unpredicted`: at simulation distance
2 the walkers leave some of their mobs outside the entity-ticking area, which
predicted 6,414 of the 7,000 mobs a tick on average. The walk still saved 41%
on average, but its p99 rose to 130.93 ms and the slowest tick took 145.86 ms.

## Failures and limits

Harnesses that fail on this branch and on main at `5a83184c7e` alike:

- `biome_source`: mismatch at output line 20117 (vanilla `biome 3`, Rust `biome 2`).
- `frame_map_writes`: "script record count differs: expected 192, got 16346".
- `lunge_motion`: "the server ran 2 ticks for move N of the jump", a known flaky harness; it failed its retry on both.
- `management_protocol`: this machine has no `.local/datagen`.
- `loaded_pig_tags`, `loot_mine_pot_components`, `mob_discard`: the harness stops on its own arguments (`--reference-only` or `--resources` expected) before running.

Limits of this change and of these measurements:

- **Few large regions are slower than main.** With one region the branch is
  about 33% slower in forced parallel mode, and walking players at view
  distance 16 merged into 3.3 regions and were 30% slower. Automatic mode keeps
  the serial path for a single region, but uses region transactions from two
  regions and 128 mobs, so the walking case's cost would reach a default server.
  Automatic mode was not benchmarked.
- Every case ran once per binary, main always first, on a shared desktop with
  the bots on the same CPU. Run-to-run variation was not measured.
- The workload is the best case for regions: invulnerable zombies, no level RNG
  users, villagers, breeding, raiders or native mods. No mixed population ran.
- The branch uses 2.5 to 4 times the CPU and 60 to 160 MiB more memory.
- At 200 players neither engine holds 20 TPS (view distance 16: 4.21 → 6.82;
  view distance 2: 5.52 → 10.40).
- The full harness sweep did not finish (see Validation), the port-ledger check
  and the live LAN checks were not rerun on this head, and the earlier report of
  `lan_e2e`'s damage check (999.04 health against vanilla's 996.0, also on the
  base it was compared with then) was not checked again.
