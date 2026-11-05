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
  `dedicated_server` failed here on every run, and the 2026-11-04 parallel commit
  results below predate the fix. Other modes keep the unseeded shuffle.
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

## Validation and benchmark

The focused tests cover topology/bridge merge and split, stable ordering,
negative/extreme chunk coordinates, migration/removal/unload/dormancy, exact
sparse-grid equivalence, chunk-by-chunk topology against a whole rebuild,
negative-space reads, distant ID/full-scan reads,
foreign bounds growth, sparse populated-cell queries, shared RNG/Gaussian
consumption, complete mutable-mob transactions and exact indexed player
selection. At implementation revision `4c5925eb2`, the full mob-AI vanilla oracle passes all 272 scenarios and all
127,179 mob ticks. Forced-parallel dedicated-server, server-entity,
player-damage and natural-spawning harnesses pass. Live server-thread/LAN
checks pass for mobs, portals, projectiles/vehicles and blasts; the mob check
was repeated against the final runtime.

The repository check at that implementation revision passes formatting, clippy (including the server
feature configuration), 5,853 workspace Rust tests, 627 Python tests and 24
content-root tests. Launcher formatting/clippy and 463 launcher tests pass;
its translation-completeness test fails on missing Hebrew strings. The exact
unchanged test also fails against base `9697aaf14`, independently reproducing
that existing failure.

Before integration with newer upstream changes, 225 distinct affected parity
entries passed without a recorded behavior mismatch. The remaining broad sweep
was stopped when the user requested delivery after the extended run; it is not
reported as a completed sweep. Save-fixture setup and an item-order oracle run
required retries; the fixture startup and both item-order cases then succeeded.

Current main `e41334ac0` is integrated at `98aaa6e9e`. The merged server passes
`cargo check -p horizon-client --lib`, formatting, seven region topology and
concurrency tests, and 15 focused mob transaction, player-index and dependency
tests. These include a new controlling-rider input regression and the complete
serial/parallel mutable-state, ordered-event and RNG comparison. The full
workspace/parity sweep and performance measurements were not repeated after
that integration; the measured engine revisions are explicit below.

`tools/bench_region_ticks.py` starts 100 real `horizon-bots` clients, teleports
them to a 512-block grid in a fixed-seed flat world, and summons 70 persistent
AI-enabled zombies per player in interleaved global order (7,000 total). Natural
spawning and random block ticks are off; players have resistance, and mobs are
invulnerable to keep the population stable. Each mode uses a fresh world and
scratch user directories. The script rejects changed player/mob counts and
collects whole-tick median/p95/p99 from the server performance report, individual
mob-phase timings, process-tree CPU and RSS, and prediction/replay counts. Setup runs while frozen, and both engines warm
up for the same fixed number of simulation ticks before timing starts.

```sh
cargo build --profile fast -p horizon-client -p horizon-client-lib -p horizon-bots \
  --bin horizon-client --bin horizon-ctl --bin horizon-bots
python3 tools/bench_region_ticks.py --bin target/fast
```

The retained serial and parallel modes use identical commands, seed, view and
simulation distance, fixed-tick warmup and measurement windows in the same binary.
`--before-bin` instead uses a separately built pre-change engine for the baseline;
the final comparison uses the PR's original base revision rather than attributing
improvements common to both new execution modes to parallel scheduling.
This is a quick workload comparison, not a claim about every mob mix: shared
RNG, native mods, active raids and unbounded observations can require replay.

### Measured comparison (2026-11-03)

Same 100 connected standing players, 512-block spacing, 7,000 invulnerable
AI-enabled zombies, seed 12345, flat terrain, view/simulation distance 2,
200 simulation warmup ticks and a 20-second measurement. Both engines use the
`fast` optimized profile and Rust 1.99.0 on a Ryzen 5 5600X (6 cores/12 threads).
The new pool has eight workers. Validation scheduling/compilation was paused
for measurement after active harnesses finished; other desktop applications
remained running. This is one quick sweep on a shared machine.

The original engine was built from base `9697aaf14`; the new engine was built
from `4c5925eb2`. The direct serial control uses the new engine with region
execution disabled. The complete inputs, percentile summaries and selected
phase timings are in [the measurement JSON](performance/server-regions-100-players.json).

| Execution | Tick median ms | Tick p95 ms | Tick p99 ms | Mob median ms | Mob p95 ms | Mob p99 ms | CPU % | Mean RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Original engine | 133.74 | 189.87 | 418.22 | 99.2 | 142.2 | 365.8 | 98.50 | 442.38 |
| New engine, direct serial control | 182.63 | 550.65 | 679.61 | 136.1 | 404.9 | 609.9 | 81.99 | 454.81 |
| New engine, parallel regions | 91.56 | 101.05 | 110.53 | 60.2 | 66.3 | 72.9 | 236.50 | 521.65 |

CPU 100% means one logical core. Peak RSS was 443.96/456.22/547.60 MiB,
respectively. The reports contain 139/79/219 ticks; timed mob spans cover
138/78/218 ticks, with the initial frozen/report boundary omitted. Every
measurement kept all 100 players and 7,000 mobs. Parallel execution committed
all 7,000 predictions across 100 regions per tick, with no replay in this
workload. The direct serial control had substantial tail variation; this
single sweep does not establish a small-workload or serial-mode speedup.

Against the original engine, parallel execution reduced median mob time by
39%, whole-tick median by 32%, p95 by 47% and p99 by 74%, using more CPU and
about 79 MiB more mean RSS. This dense collision workload still runs below
20 TPS (6.93 before, 10.90 after). The results establish a live improvement,
not a guarantee that every 100-player mob mix fits a 50 ms tick budget.

```sh
python3 tools/bench_region_ticks.py --bin .local/parity-target/fast \
  --before-bin /path/to/base-build/target/fast --modes legacy,control,parallel \
  --warmup 10 --settle 20 --seconds 20
```

### After the coordinator work (2026-11-04)

Two runs of the same workload at `ce803b1ea`, parallel mode only, with the
`fast` profile plus frame pointers and line tables, on the same shared machine
(other applications running, the bots on the same CPU). Their full reports are
under `coordinator_work` in [the measurement JSON](performance/server-regions-100-players.json).

| Run | Tick average ms | Median | p95 | p99 | Max | Ticks over 50 ms | Mob median ms | CPU % | Mean RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 47.44 | 47.09 | 51.06 | 53.72 | 54.82 | 26 of 401 | 28.4 | 394.65 | 544.63 |
| 2 | 49.06 | 48.05 | 56.25 | 66.28 | 75.04 | 88 of 401 | 28.9 | 399.49 | 558.37 |

Both runs hold 20 TPS with all 7,000 predictions committed and no replay, and
the average tick fits the 50 ms budget. Single ticks still go over it, so this
workload has no margin on this machine, and runs differ by a millisecond or
two. The remaining time is mostly the workers' 16 ms round, the 7 ms commit walk
and 8 ms of `tick_lan`.

The time came from the coordinator's walks over every mob, each a cache miss
per mob: game events and melee hits find their listeners and mob directly,
states and entity sends are built on the workers, a mob's rarely used fields
sit behind a box so a clone copies less, and the scans a tick repeats run on the
workers (`Game::mob_slots`) or not at all. The replaced mobs are freed by one
worker at the end of the tick, so those scans do not queue behind the frees.

### After the portal and sculk work (2026-11-04)

One run of the same workload at `4d8283a8c`, with the same settings and build
flags as above, on the same shared machine. The user's own Horizon client was
running at about 8% of a core. Its full report is under
`portal_and_sculk_workers` in [the measurement JSON](performance/server-regions-100-players.json).

| Run | Tick average ms | Median | p95 | p99 | Max | Ticks over 50 ms | TPS | Mob median ms | CPU % | Mean RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 50.76 | 49.53 | 57.68 | 75.24 | 91.18 | 178 of 394 | 19.67 | 30.1 | 404.37 | 548.74 |

This run is no faster overall than the two before it (47.44 and 49.06 ms
average), and it falls below 20 TPS. It does not show a regression either: those
runs were at `ce803b1ea`, before two merges of main, and there is no run of the
merged main without these changes, so one run cannot separate this work from
main or from the machine's load. Spans these changes do not touch also rose
(`mob_predict` 15.82/16.25 to 16.80 ms, `tick_lan` 7.93/8.14 to 8.83 ms).

The spans these changes target did drop on the coordinator. All 7,000
predictions still committed, with no replay.

| Span | Before, wall ms | After, wall ms | Before, coordinator CPU ms | After, coordinator CPU ms |
| --- | ---: | ---: | ---: | ---: |
| `entity_portals` | 1.68, 1.82 | 1.04 | 1.65, 1.78 | 0.66 |
| `tick_sculk_entities` | 1.80, 1.83 | 1.70 | 1.50, 1.54 | 0.80 |

The sculk span's wall time barely moved, because the coordinator now waits for
the workers and then stores each mob's `Steps`. The passenger index has no span
of its own, and `mob_guest_events` did not move (1.97/2.02 to 2.05 ms), so this
run shows no gain from it.

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
  the existing code.

The pool-against-serial tests need prepared resources, so they are `#[ignore]`d:
`pool_and_serial_find_the_same_portals_in_the_same_order`,
`pool_and_serial_raise_the_same_events_and_keep_the_same_steps` and
`entity_hurt_player_reads_riders_only_when_a_criterion_can`. Set
`HORIZON_TEST_RESOURCES` and run them with `--include-ignored`.

The harnesses `tools/parity.py --changed-files` selects for these files were
stopped before the run finished, at the user's request, and three of them failed
without being looked into. `lan_e2e` was not run. The PR records the details.

### Parallel commit (2026-11-04)

One run of the same workload, with the same settings and build flags as above,
on the same shared machine, from the commit that adds this section (on
`2362e29c5`, which merges main at `23bbd8117`). The user's own Horizon client was
running at about 17-24% of a core. Its full report is under `parallel_commit` in
[the measurement JSON](performance/server-regions-100-players.json).

| Run | Tick average ms | Median | p95 | p99 | Max | Ticks over 50 ms | TPS | Mob median ms | CPU % | Mean RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 46.69 | 45.58 | 53.56 | 59.67 | 91.01 | 66 of 400 | 19.97 | 25.7 | 448.39 | 537.28 |

Every round was clean: 409 clean, 0 fell back, counting the warmup. All 7,000
predictions committed, with no replay.

`mob_classify` is the classification and the clean commit together; `mob_walk`
is empty when the walk is skipped. The "before" figures are the three earlier
runs above.

| Span | Before, wall ms | After, wall ms | Before, coordinator CPU ms | After, coordinator CPU ms |
| --- | ---: | ---: | ---: | ---: |
| `mob_walk` + `mob_classify` | 7.03, 7.26, 7.68 | 3.24 | 6.92, 7.17, 7.51 | 1.29 |

So the walk's span saved 3.8 to 4.4 ms of wall time, at or just under the low end
of the 4-5 ms estimated before the work. The clean commit did not take the span
to zero, because the workers still do the work and the coordinator still merges.

The whole tick is less clear. The average fell by 0.75, 2.37 and 4.07 ms against
the three earlier runs (47.44, 49.06 and 50.76 ms). There is no run of this
binary with `HORIZON_PARALLEL_COMMIT=0`, so these comparisons cross merges of main
and the machine's load:

- `mob_projectiles` fell from 0.70-0.85 ms to nothing because of main's early
  return in `tick_projectiles`. That is part of the tick's drop and not this work.
- Spans this work does not touch did not fall: `mob_predict` 15.82/16.25/16.80 to
  17.39 ms, `tick_lan` 7.93/8.14/8.83 to 8.80 ms.
- The process used more CPU in total (394.65/399.49/404.37 to 448.39%). One run
  can't say how much of that is the commit.

**Checks.** `cargo fmt --check`; clippy `-D warnings` on `horizon-client` and
`horizon-client-lib`, all targets; `cargo test -p horizon-client --lib`;
`cargo test -p horizon-client-lib --lib` for `mobs::` and `lan::`;
`clean_commit_matches_the_serial_walk` with prepared resources. Ten deliberate
breaks of the clean path were each caught by that test. With
`HORIZON_SIMULATION_MODE=parallel`, `mob_ai`, `natural_spawn`, `dedicated_server`,
`server_entities` and `server_player_damage` passed. `dedicated_server` failed
its first attempt, then passed on the runner's retry without reused oracle
results and on two more runs; the first attempt's log was overwritten, so why it
failed is not known. `mob_ai`'s probe has its own mob loop, so it does not reach
the clean path.

**`tools/ci/region-verify.sh` fails on `dedicated_server`, with or without
parallel commit.** The panic is "the commit and its replay differ" in a long
jump's `chosen`. `optimal_jump_vector` follows vanilla's `Collections.shuffle`
with no seeded random, so a commit and its replay can pick different angles for
the same frog or goat. It failed 3 of 3 runs with `HORIZON_PARALLEL_COMMIT=0`. With
the shuffle temporarily pinned, a local change that was not committed, all four
verify harnesses passed, so this draw is the only cause there. It has not been
fixed. The script's other tests pass when run on their own. It has not been run
on the commit before this merge, so when it began to fail is not known.

**Not run.** No control run of this binary, no second run, and no workload with
mixed species, moving players or mobs outside the entity-ticking area. `lan_e2e`
was not run.

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

The viewer-parallel experiment was removed: a repeated 200-player comparison
changed 92.28 to 92.05 ms, while 100 players regressed from 41.86 to 42.16 ms.
The initial baseline itself ranged from 92.28 to 95.90 ms at 200 players. These
results do not establish a whole-tick benefit. At `c0b066072`, only profiling and benchmark
reproducibility changes remained. The subsequent chunk-range overlap below is
a separate implementation; the rejected visibility path remains removed.

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

### Rejected visibility experiment: fresh measurements

Baseline: exact PR head `71aff0ed91f158bbcc5574f2fc854f35d73d25a5`.
Both binaries used the optimised `fast` profile, the same prepared 26.3 resources,
forced parallel simulation and eight region workers on a Ryzen 5 5600X (six
physical cores). Each run used a fresh seed-12345 flat world, view/simulation
radius 2, stationary players 512 blocks apart, 70 persistent invulnerable AI
zombies each, 200 fixed warmup ticks and a 20-second measurement. Runs were
sequential, without active compilation or oracle execution. Desktop applications
and an existing game process shared the machine; this was not an isolated host.
The benchmark now records load averages as well as process CPU/RSS.

Initial baseline averages at 50/100/200 players were 22.92/43.07/95.90 ms;
initial candidate averages were 20.67/41.27/91.50 ms. Repeating each untouched
baseline immediately before its candidate removed that apparent improvement:

| Players | Baseline avg/p50/p95/p99 ms | Rejected candidate avg/p50/p95/p99 ms | TPS before → candidate | CPU % before → candidate | Mean RSS MiB before → candidate |
| ---: | --- | --- | --- | --- | --- |
| 50 | 20.58/20.15/22.66/24.27 | 20.65/20.41/22.99/25.28 | 20.03 → 20.03 | 330.34 → 330.34 | 412.29 → 413.51 |
| 100 | 41.86/41.41/45.72/48.15 | 42.16/41.82/45.55/48.31 | 20.01 → 20.01 | 438.46 → 438.44 | 520.24 → 527.09 |
| 200 | 92.28/91.36/98.56/105.75 | 92.05/91.28/98.28/101.87 | 10.83 → 10.86 | 462.65 → 467.54 | 774.4 → 813.8 |

Every spread-out run committed all predictions: zero replays, RNG/read conflicts
and fallback rounds. The connected four-player/280-zombie case (32-block spacing)
changed 4.92 to 4.98 ms; twelve players with 840 zombies/skeletons/spiders changed
5.23 to 5.28 ms. Both also kept expected counts and zero replays. These are two
runs per binary at each large size and one per binary in the extra cases, not a
statistical demonstration of equivalence. The negative/near-zero repeated
results do not justify retaining the extra parallel visibility path.

The first 200-player candidate report separated player gameplay (4.55 ms), entity
snapshots (2.98 ms), tracking preparation (1.35 ms), visibility (0.21 ms), entity
encoding/distribution plus its wait (2.46 ms), chunk output (0.06 ms) and output
queuing (0.03 ms). Mob prediction still waited 31.51 ms. Streaming preparation
was 7.19 ms; application and unloading retained their existing world ordering.
These are elapsed spans with overlapping parents, not additive costs.

The new compression/queue counters and per-worker prediction profiler were added
after the rejected experiment. Those metrics are unavailable for the before/after
sweep. There is no measured socket delivery latency or window-peak backlog; no
latency improvement is claimed. That experiment retained no additional mob start/join API, speculative
replication, queue, thread pool or world/chunk clone at `c0b066072`.

### Final diagnostics validation

`pr_ready.py --quick --base 71aff0ed91f158bbcc5574f2fc854f35d73d25a5` passed
formatting, clippy, public-tree checks, 644 Python tests and 4,096 Rust tests.
The quick run explicitly deferred parity. An earlier broad sweep was interrupted
after nine completed passes; it was not a complete pass. The final quick report
lists 153 affected harness cases for the broader CI scope.

`tools/ci/region-verify.sh` passed all four real-jar verify-mode harnesses
(`dedicated_server`, `server_entities`, `server_player_damage`, `natural_spawn`),
15 transaction tests and six resource-backed gameplay tests, including clean
commit versus serial and guest damage attribution. Separately, the 15 transaction
tests also passed with `HORIZON_REGION_PROFILE=1`.

Live server-thread LAN mobs/chest checks passed. The damage harness exits zero
but reports same-hit armor ordering as 999.04 health versus vanilla's 996.0.
The untouched baseline reproduced that mismatch on a fresh retry; the baseline's
first attempt failed earlier on its fall-damage assertion. These observations
are not a clean armor-parity pass and are not fixed by this diagnostics change.

A final profiling-enabled 200-player smoke run retained all 14,000 mobs and
committed every prediction without replay/fallback. Per-round worker elapsed
sums were approximately 60 ms preparing inputs, 153 ms simulating and 29 ms
building observations/results; these overlapped within a 31.55 ms prediction
barrier and must not be added to elapsed tick time. Entity snapshots took
2.96 ms, encoding/distribution 2.50 ms and serial visibility 0.67 ms. The report
recorded 200 actual compression calls and 20.06 ms summed compression-worker
time across the window. End samples had zero unframed network bytes and no
queued/generating/decorating/finishing/lighting/meshing work (7,868 proto chunks
remained cached). Pending bytes exclude framed lookahead and kernel socket
queues; zero here does not establish delivery latency or peak backpressure.
This profiler smoke run is not a claim of performance improvement.

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

### Fresh overlap measurements

Baseline is exact PR head `c0b06607260065356dafa22be5f3c60beb91237c`, saved
before editing. Same `fast` profile, prepared resources, parallel mode and eight
workers on the six-core Ryzen 5 5600X. Both binaries use fresh seed-12345 flat
worlds, players 512 blocks apart, 70 persistent invulnerable AI zombies each,
view/simulation distance 2, exactly 200 warmup ticks and 20-second windows. All
performance runs are sequential, without compilers or oracles running. Desktop
applications and an existing game share the host; system load averages are
recorded. This compares the follow-up to the current PR, not to main or to the
historical `71aff0ed9` benchmark.

The headline values below are means of per-run tick averages, with two baseline
runs at each size, two implementation runs at 50/100, and five at 200:

| Players | Baseline ms/tick | Implementation ms/tick | Saved | TPS before → after | Process CPU % before → after | Mean RSS MiB before → after |
| ---: | ---: | ---: | ---: | --- | --- | --- |
| 50 | 20.73 | 20.30 | 0.43 ms (2.05%) | 20.03 → 20.03 | 327.88 → 359.85 | 420.73 → 419.76 |
| 100 | 42.10 | 41.30 | 0.80 ms (1.91%) | 20.00 → 20.01 | 436.02 → 445.88 | 554.94 → 545.93 |
| 200 | 92.48 | 89.01 | 3.47 ms (3.75%) | 10.80 → 11.23 | 458.58 → 476.40 | 831.54 → 828.95 |

At 200 players, the three same-binary `HORIZON_REGION_OVERLAP=0` runs were
89.54/89.57/89.65 ms, averaging 89.59 ms versus 89.01 ms with overlap. Bracketing
each serial-preparation run with the adjacent overlap runs gives differences
of 0.50/0.72/0.54 ms. Thus the whole change saves about 3.47 ms in this workload,
with about 0.58 ms observed in the overlap ablation; the remainder must not be
credited to overlap alone. The coordinator prepared approximately 8,000 predicate
answers in 0.72 ms during a 31.70 ms prediction interval, then waited 30.94 ms
for the join (first run). Streaming preparation decreased from about 7.2 ms to
3.0 ms. These nested/overlapping spans must not be added to parent durations.

CPU is a tradeoff, not a claimed saving. At 50 players both engines keep 20 TPS,
but process CPU rises about 32 percentage points. Performing the preparation
serially at 50 players measured 20.63 ms and 359.96% CPU, so disabling overlap
alone did not remove that increase. No new threads or pools are created. RSS
varies between runs; the table does not establish a memory improvement.

All performance runs retained expected player/mob counts and committed every
prediction: zero RNG/read conflicts, replays or fallback rounds. Queue samples
ended with zero unframed network bytes and no queued/generating/lighting/meshing
work. They are end samples, not window maxima or delivery-latency measurements.
The range-preparation log samples were accepted in the steady stationary cases.
Publication remains in the original tick; no extra tick of delivery latency is
introduced. Wire latency itself was not measured.

Four connected players/280 zombies changed 5.09 to 4.93 ms; 32 players with
2,240 mixed zombies/skeletons/spiders changed 13.55 to 13.25 ms. Those are single
pairs, not statistical equivalence claims. The mixed workload exercises the
preparation path (more than 1,024 proto coordinates). A live verify-mode run
with 20 warmup ticks then 20 seconds passed, with 15 complete measured rounds
of serial/committed-state comparison and no mismatches. Its deliberately slow
verification cost is not a performance comparison. The preceding attempt with
200 warmup ticks hit the driver's warmup timeout at about 1.3 seconds per tick;
that attempt did not complete and is not counted as passed.

Per-run summaries below preserve the tail values. `avg/p50/p95/p99` are the
individual report values; RSS is mean/peak, not an aggregate percentile:

| Case | Avg/p50/p95/p99 ms | TPS | Process CPU % | RSS mean/peak MiB |
| --- | --- | ---: | ---: | --- |
| 50 baseline 1 | 20.90/20.66/23.21/25.97 | 20.03 | 325.40 | 417.02/424.67 |
| 50 baseline 2 | 20.55/20.37/22.02/23.04 | 20.03 | 330.37 | 424.44/431.79 |
| 50 overlap 1 | 20.42/20.04/22.30/26.06 | 20.03 | 359.81 | 411.13/419.99 |
| 50 overlap 2 | 20.18/19.85/21.86/23.53 | 20.03 | 359.88 | 428.40/435.62 |
| 100 baseline 1 | 42.36/41.83/46.32/50.67 | 20.01 | 438.38 | 550.63/559.20 |
| 100 baseline 2 | 41.84/41.45/45.92/49.14 | 19.98 | 433.66 | 559.25/571.52 |
| 100 overlap 1 | 41.36/40.88/44.76/47.83 | 20.01 | 443.48 | 551.35/560.56 |
| 100 overlap 2 | 41.23/40.96/44.49/47.35 | 20.01 | 448.28 | 540.51/550.86 |
| 200 baseline 1 | 92.25/91.57/99.64/103.06 | 10.83 | 458.76 | 826.99/860.47 |
| 200 baseline 2 | 92.71/91.98/99.88/102.00 | 10.78 | 458.40 | 836.09/851.38 |
| 200 overlap 1 | 89.13/88.29/95.46/100.17 | 11.21 | 477.36 | 824.92/850.11 |
| 200 overlap 2 | 88.95/88.22/95.99/102.00 | 11.24 | 477.01 | 831.89/850.82 |
| 200 overlap 3 | 88.76/87.92/95.85/100.45 | 11.26 | 472.33 | 829.18/849.14 |
| 200 overlap 4 | 88.95/88.14/95.36/100.21 | 11.24 | 477.52 | 827.38/854.04 |
| 200 overlap 5 | 89.27/88.40/96.08/100.92 | 11.20 | 477.76 | 831.38/854.22 |
| 200 serial preparation 1 | 89.54/88.42/97.13/109.74 | 11.16 | 472.81 | 833.25/858.02 |
| 200 serial preparation 2 | 89.57/88.54/96.30/108.72 | 11.16 | 472.79 | 839.90/871.55 |
| 200 serial preparation 3 | 89.65/88.93/96.83/101.57 | 11.15 | 472.73 | 825.96/850.23 |

Final validation of the retained implementation: `pr_ready.py --quick --base
c0b06607260065356dafa22be5f3c60beb91237c` passed formatting, clippy, public-tree,
644 Python tests, 4,100 Rust tests and doc-test checks. The 153 affected broader
parity cases are explicitly deferred by `--quick`, not reported as passed.
`tools/ci/region-verify.sh` passed all four focused real-jar verify-mode harnesses,
18 transaction/lifecycle tests and six resource-backed gameplay tests. Live LAN
mobs/chest passed; the damage check still reports the baseline armor-order
mismatch described above. Packet publication and gameplay mutation phases remain unchanged.

The required port-ledger check against `origin/main` exited zero, but reported
existing stale comment citations and advancement mapping differences in the
broader PR. It is not a clean ledger audit; the cited comments already exist at
`c0b066072`, and this follow-up changes no advancement implementation.
