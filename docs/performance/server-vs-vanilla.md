# Horizon's server against vanilla

Horizon’s dedicated server was measured against vanilla 26.3, Paper and Fabric + Lithium with the same deterministic
bots, world seed and server settings. The run used a four-thread Intel Core i7-7500U laptop with 32 GB of memory;
the servers ran one at a time with Java 25 where applicable.

The bots joined one per second, flew in the `mixed` behaviour (walking, exploring and idling), and were measured after
a warm-up. Both servers used seed `12345`, view and simulation distance 10, offline mode, creative mode and the same
heap settings. Horizon and vanilla were compared at 1, 10, 25 and 50 bots.

## Results

MSPT is the average tick time from each server's own `/tick query`; 50 ms is the 20 TPS budget. RSS is the mean
resident memory of the server process tree. “New chunks/s” counts chunks the bots had not received before.

| Bots | Server | MSPT | P95 | P99 | TPS | RSS | CPU | Start | Chunks/s | New chunks/s |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | Horizon | 13.70 | 19.06 | 27.38 | 20.00 | 0.72 GB | 46% | 8.3 s | 9.94 | 0.01 |
| 1 | vanilla | 17.31 | 24.68 | 36.43 | 20.00 | 3.44 GB | 55% | 14.9 s | 9.99 | 0.00 |
| 10 | Horizon | 22.93 | 39.83 | 53.33 | 20.00 | 0.92 GB | 67% | 8.2 s | 61.21 | 1.78 |
| 10 | vanilla | 22.94 | 36.92 | 56.48 | 20.00 | 3.71 GB | 101% | 14.7 s | 61.63 | 1.84 |
| 25 | Horizon | 30.92 | 48.25 | 72.00 | 20.00 | 1.01 GB | 99% | 8.5 s | 155.81 | 5.78 |
| 25 | vanilla | 37.35 | 48.20 | 82.66 | 20.00 | 3.94 GB | 191% | 15.5 s | 158.52 | 5.71 |
| 50 | Horizon | 47.83 | 62.42 | 73.21 | 20.00 | 1.11 GB | 143% | 8.3 s | 302.68 | 11.02 |
| 50 | vanilla | watchdog stopped the server | — | — | — | — | — | — | — | — |

At 50 bots, vanilla's watchdog stopped the server during warm-up after it fell more than 16 seconds behind. Horizon
remained below the 50 ms average-tick budget in this run. The comparison is a measurement on one machine, not a claim
that every workload or computer will produce the same result.

The full procedure is in [the server benchmark plan](../server.md#9-benchmark-plan-bots-against-vanilla-and-paper),
and the runner is [`tools/server_bench.py`](../../tools/server_bench.py). The run also included Paper and Fabric +
Lithium; those results are retained with the benchmark data for reproducibility.
