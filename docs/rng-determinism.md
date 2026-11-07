# Repeatable random numbers

Horizon's unseeded random streams are not repeatable across runs by default. Vanilla's seeded
streams (`RandomSource.create(seed)`, worldgen, mob AI) already are; the unseeded ones come from

```java
// RandomSupport.generateUniqueSeed()
seedUniquifier.updateAndGet(x -> x * 1181783497276652981L) ^ System.nanoTime()
```

`System.nanoTime()` is the only nondeterministic term. Horizon mirrors the formula in
`horizon_core::random::unique_seed()`, with the same clock, so each run draws its own sequence
unless it is pinned. `HORIZON_RNG_SEED` replaces that clock term with one master seed, making a
Horizon run (and its screenshots, traces and tests) repeatable. `tools/rng_determinism.py` can
apply the same pinning to a vanilla oracle run for comparison; the two still need not create or
consume streams in the same order.

## Pin a seed

Java, through a `-javaagent`:

```sh
python3 tools/rng_determinism.py java --seed 42 -- java -cp … dev.horizonlab.parity.SomeOracle in.txt out.txt
```

The agent patches `RandomSupport.generateUniqueSeed()` and
`RandomSource.createThreadLocalInstance()` (Netty's thread-local random) to return
`uniquifier * STEP ^ 42`. Seeded sources are untouched. The sources are
`mods/horizon/src/test/java/dev/horizonlab/parity/rng/`; they build against the JDK alone, so the
same jar works with the client and server jars. Java 24+ is needed for the Class-File API the
transformer uses.

Horizon, through the environment:

```sh
export HORIZON_RNG_SEED=42
cargo run --profile fast -p horizon-client -- --resources .local/horizon-client/26.3 --world T
```

`HORIZON_RNG_SEED` accepts decimal, `0x` hex, a leading `-` and `_` separators. `HORIZON_STILL=1`
pins the seed to 0 when no seed is given, the way it already pins visual time, so a screenshot run
stays repeatable without another variable. The Java agent does the same when `HORIZON_STILL=1`.

The seed is process-wide: in single player the window and the server thread share one sequence,
as they share vanilla's `SEED_UNIQUIFIER` in one JVM.

## Check a run is repeatable

```sh
python3 tools/rng_determinism.py verify --seed 42 --count 64 --trace
```

`verify` runs `RngDeterminismOracle` under the agent and `client_probe rng_determinism` with the
same seed. Both create `count` streams, print each seed and `nextLong()` of `LegacyRandom` and
`XoroshiroRandom`, then draw from `RandomSource.create()` and the thread-local instance. This checks
that a pinned seed reproduces the same stream seeds and is a debugging aid for comparing creation
order; it does not imply that Horizon and vanilla produce the same gameplay, since they need not
create or consume streams in the same order. `tools/test_rng_determinism_parity.py` is the same
check in `tools/parity.py`, with `--trace` on.

## Trace the creation order

`--trace` writes one line per unseeded stream:

```
create 17 -7868760161517138634 PlayerList.placeNewPlayer
```

Java records `Class.method`; Horizon records `crate::module::function (file.rs)`. `verify --trace`
compares the `(index, seed)` columns, not the site text, and names the first stream whose index or
seed differs. That distinguishes the two failure modes:

* **The same streams, in the same order**: matching seeds mean the run is repeatable.
* **The same streams, in a different order** (or a different count): something creates a random
  source where the other side doesn't. The trace line before the first difference is the caller.

Tracing is for reproducing a divergence, not for matching site names across languages.

## Streams that must not consume the shared sequence

Effects whose creation order legitimately differs (particles are the usual example) should not
take "the next" seed. Name the stream instead:

```rust
let seed = horizon_core::random::pinned_seed("firework_spark").unwrap_or_else(unique_seed);
```

`pinned_seed(name)` is `RandomSupport.seedFromHashOf(name).seedLo() ^ master`, the same derivation
vanilla uses for named positional streams (`seedFromHashOf` is MD5 of the name's UTF-8). Java's
counterpart is `Deterministic.streamSeed(name)`. `None` means determinism is off; the caller keeps
its old behaviour. `seed_for(name)` is the same call falling back to `unique_seed()`.

This is also how `HORIZON_STILL` already breaks load-order ties: an end crystal's spin starts from
its position (`Crystal::start_time`), not from whichever crystal loaded first.

## What is not covered

* **Order remains the spec.** Determinism makes a run repeatable; it cannot make Horizon and
  vanilla agree when they call RNG differently. Use the trace to fix the call order, not the seed.
* `UUID.randomUUID()`, `SecureRandom` (chat sessions, telemetry, chat reports) and authentication
  are identity/security inputs, not gameplay RNG; the agent leaves them alone. Vanilla uses
  `java.util.Random` nowhere else in the pinned client (only the `RandomAccess` marker interface).
* Wall-clock values that are not RNG (toast timestamps, save times, `/pregen` output names) stay
  wall-clock. The deterministic seed only replaces values used as random seeds.

## Using it inside a harness

`tools/oracle_harness.py`'s `differential` takes `jvm=` and `rust_env=`:

```python
import rng_determinism

rng_determinism.ensure_agent()
oracle_harness.differential(
    "my-area", "MyOracle", "my_probe", make_input,
    scope="…", jvm=rng_determinism.jvm_options(42),
    rust_env=rng_determinism.environ(42))
```

That covers an oracle that constructs unseeded `RandomSource`s. Harnesses that pass every seed
explicitly do not need it.
