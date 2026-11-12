# Compact translucent particle upload microbenchmark (PR #14670)

This reproduces **host-side vertex packing, `wgpu::Queue::write_buffer`, queue submission and GPU completion** for 44-byte and 32-byte vertex formats. It is NOT a Horizon frame-time or renderer benchmark; it does not construct Horizon's render pipelines, draw particles, or test HDR/output-format transitions. Results must not be presented as an FPS speedup.

## Run

On Windows with Rust stable and a Vulkan/DX12 graphics driver:

```powershell
cargo run --release --manifest-path benchmarks/particle-upload-wgpu/Cargo.toml
$env:BENCH_BACKEND = "dx12"
cargo run --release --manifest-path benchmarks/particle-upload-wgpu/Cargo.toml
$env:BENCH_BACKEND = "vulkan"
cargo run --release --manifest-path benchmarks/particle-upload-wgpu/Cargo.toml
```

The program prints the adapter actually selected, byte counts and CSV. It requires a working GPU: missing adapters are an error, not a silent skip. Each row reports the median of seven timed series after 20 warm-up submissions; 90 or 180 iterations per series. A submission is intentionally awaited each time. Timing is wall-clock latency including staging, command submission and synchronization, not GPU hardware timestamp-query duration.

The standalone program models the PR's `prepare()` eligibility scan and 44-to-32-byte mapping. Unlike Horizon's real `Uploads` implementation, it uses direct `queue.write_buffer` calls, and does not draw any geometry. The `full_with_cpu_us` column sends original bytes; `compact_with_cpu_us` also includes scanning and packing the compact vector.

## Initial measurements — 2026-11-12

Machine: AMD Ryzen 7 8845H / Radeon 780M, Windows, AMD driver 32.0.11039.6002, Rust 1.99.1, `wgpu 30.0.1`, release profile.

| Quads | Full bytes | Compact bytes | Vulkan full (µs) | Vulkan compact (µs) | DX12 full (µs) | DX12 compact (µs) |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 64 | 11,264 | 8,192 | 118.65 | 116.07 | 128.22 | 122.81 |
| 128 | 22,528 | 16,384 | 183.64 | 152.93 | 129.68 | 113.73 |
| 256 | 45,056 | 32,768 | 124.36 | 127.62 | 127.58 | 138.17 |
| 512 | 90,112 | 65,536 | 141.27 | 158.56 | 137.24 | 127.52 |
| 1,024 | 180,224 | 131,072 | 167.82 | 142.59 | 165.37 | 146.35 |
| 4,096 | 720,896 | 524,288 | 216.39 | 224.06 | 198.66 | 194.25 |

These are samples from one run of each backend; timing is noisy and the winner varies. At 256 quads, neither backend in this sample favors the compact upload. The `>=256` threshold remains a heuristic, **not a measured optimal crossover**. With packing included, compact can be slower despite the 27.27% lower byte count.

**Unit correction:** bytes per quad = 4 vertices × (44 or 32). At 256 quads, the original is **44 KiB**, compact is **32 KiB**, saving **12 KiB** (or 45.056/32.768/12.288 decimal kB). Prior PR-comment labels `45.06 KiB` / `32.77 KiB` incorrectly used decimal kB values as KiB.

## Proof still required before merging

1. Measure a **real particle-heavy Horizon scene**, both variants with identical settings. Capture median/p95 frame time, GPU frame/render-pass time if available, `Uploads` time, FPS, number of translucent quads, and actual fallback rate. Include repeats and raw logs. Synthetic upload timing does not substitute for this.
2. On an actual GPU, explicitly run `cargo test -p horizon-client-render compact_particle_gpu_pixels_match_full_vertices -- --ignored --nocapture`. The test is marked `#[ignore]`, so it is not covered by the default CI test command. Attach the result logs. For full renderer parity, request the maintainer's `run-parity` label; the fork cannot self-trigger that runner.
3. Verify in-game HDR on/off and output-format transitions. Source review shows `Renderer::new` constructs the compact pipeline alongside `entity_pipeline` using the same `frame_format`, `linear_light`, `entity_bias`, but this is **not** an observed runtime lifecycle test.

Do not claim this benchmark proves improved FPS or that GPU parity/HDR lifecycle tests passed.
