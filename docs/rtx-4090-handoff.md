# The RTX 4090 build: handoff (4 November 2026)

For the agent that takes the branch over. Read [rtx-4090.md](rtx-4090.md) first (the target, the measurements, the
phases), then this page: what is done, what is untested, what to do next and in what order, and the rules this branch
runs under. Everything measured was measured by the user on the RTX 4090 (Windows 11, 3440x1440, LOD 1024, the
RTX-Remix-Materials pack); nothing here has run on a GPU in the agent's session.

## Where things are

- **Branch:** `claude/serene-ride-28glsa` on `Thib1404/horizon`, based on `claude/rt-sun-path` (the fork's default
  branch, merged in at 934b65a98). **Pull request:** Thib1404/horizon#1, draft, `skip-release`, CI green since the
  Python suite and the fork-specific tests were fixed (the last push fixes an acoustics race the `verify` job hit).
- **The decision (from the user):** this branch diverges from Horizon's main on purpose. One machine, no portability
  rule, ray tracing at a cost that holds 90 Hz in VR; the good ideas go back to main later. Everything specialised sits
  behind the `rtx` cargo feature or an environment variable, so the branch builds and runs as main does without it.
- **Build on the PC:** `cargo build --profile fast -p horizon-client` (the normal build) and `cargo build --profile
  fast -p horizon-client --features rtx` (the Vulkan island); both give `target/fast/horizon-client.exe`. Measuring:
  `HORIZON_SPIKES=20` plus `horizon-ctl fly ... ` and `stats` for the CPU, `HORIZON_GPU_PASSES=1` and
  `tools/gpu_passes_report.py game.log --best 0.25` (or `tools/gpu_now.py`) for the GPU stages.
- **How the user works with the agent:** the user runs the measurements on the PC through their own local agent and
  pastes the report; the cloud agent writes code, tests it without a GPU (unit tests, naga parsing of the shaders,
  clippy, `glslangValidator`), commits and pushes. Each phase is measured before the next.

## Done, measured

| What | Where | Measured |
|---|---|---|
| Phase 0, CPU in flight: the voxel grid in runs per column, its masks on the mesh worker; light cells gated and now on their own thread; BLAS nearest first; mesh uploads in a nearest-first queue under an adaptive time ceiling; remote backlog smoothed; block entities by column; rain map and far height map incremental; the far terrain (LOD) on its workers; wgpu-core's per-instance TLAS CPU trimmed | `gfx/voxels.rs`, `occupancy.rs`, `gfx/hw_rays.rs`, `streaming/uploads.rs`, `game/remote.rs`, `block_entities.rs`, `rain.rs`, `gfx/far_heights.rs`, `lod.rs`, `gfx/lod.rs`, `third_party/wgpu-core` | CPU p99 40 → 30 ms, slow frames 168 → 78 (before the threaded light cells, which attack the remaining leader, `rt_grid` 8.3 ms) |
| Phase 1: the any-hit floor switch; the fully opaque quads in opaque geometries | `gfx/shadow_rays.rs` (`HORIZON_RT_FORCE_OPAQUE`), `mesher/sides.rs`, `gfx/hw_rays.rs`, the `_hw.wgsl` shaders | floor 1.3 to 1.7 ms of the trace; the opaque geometries gave 0.3 ms (see "open questions") |
| Phase 2: RT Ray Density option (Auto, Full, Half) | `graphics.rs`, `graphics_ui.rs` | not measured as an option; `rt-scale 2` was measured earlier (trace and denoiser a quarter) |
| Structures: 0.12 to 0.35 ms a frame even placing blocks in bursts | | DirectX's CLAS/PTLAS dropped as a lever |

## Done, untested on a GPU (the `rtx` feature, crate `crates/horizon-rtx`)

1. **The top-level structure built raw** (`horizon-rtx/src/lib.rs` `RawTlas`, `hw_rays.rs`): Horizon's own instance
   buffer (`TLAS_INPUT`), rows written where they change, `vkCmdBuildAccelerationStructuresKHR` through
   `CommandEncoder::as_hal_mut`, then `mark_acceleration_structures_built`. `HORIZON_RAW_TLAS=0` keeps wgpu's build
   with the feature on. Expected to remove `rt_moving`'s 2.5 ms of CPU.
2. **The sun pass as a ray tracing pipeline with shader execution reordering** (`horizon-rtx/src/sun.rs`,
   `shaders/sun.{rgen,rmiss,rahit}` and their `.spv`, compiled with `glslangValidator -V --target-env vulkan1.2`):
   `HORIZON_RT_PIPELINE=1`. The raygen traces the kernel's sun ray for the primary surface (its `surface_at`,
   `surface_normal`, `random2`, `sun_disc_ray` ported one for one) as a `hitObjectNV`, `reorderThreadNV`, the any-hit
   is the kernel's `hw_opaque`; the kernel reads the result at binding 61 in place of tracing (`rt.mode.w`,
   `sun_from_pass` in `shadow_rays.wgsl`). Gaps: the translucent geometry passes light untinted (no glass colour, no
   water caustics in the pass), VR's sparse zones keep the kernel's ray. The device gets the pipelines through wgpu's
   `EXPERIMENTAL_RAY_TRACING_PIPELINES` (gfx.rs) and the reorder extension through `horizon-hdr`'s device hook.
   Descriptor sets are a ring of four; resources are put in state by `transition_resources`.

**First runs:** with the Vulkan validation layers (`VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation`), log lines
"Horizon RTX: the top-level structure built by raw Vulkan commands" and "the sun pass as a ray tracing pipeline".
Likely trouble spots if validation complains: the scratch alignment (`SCRATCH_ALIGN` 256, rounded address), the image
layouts in the descriptor writes (`DEPTH_STENCIL_READ_ONLY_OPTIMAL` for the depth, `SHADER_READ_ONLY_OPTIMAL` for the
sprites and atlas, `GENERAL` for the output: what wgpu-hal's `derive_image_layout` gives the states asked for), the
barriers' stages, a buffer whose usage lacks what the transition needs.

## Open questions, with the way to answer them

1. **Why did the opaque geometries give 0.3 ms of the 1.3 to 1.7 the floor promised?** The log line "Horizon RT
   lighting: N sections' structures built, X % of their solid quads in opaque geometries" (every 2048 sections,
   `hw_rays.rs` `STAT_EVERY`) says whether the pack's sprites pass the "every texel alpha 255" test (`sprites.rs`
   `Sprites.opaque`). If X is low: loosen the test to what the shader confirms anyway (`alpha >= 0.5` for every
   texel) or precompute a per-sprite verdict from the alpha cutoff. If X is high: the rest of the any-hit cost is the
   cutouts (leaves: opacity micromaps) and the translucent tint (`hw_tint`, the water's sixteen waves per candidate);
   add a skip bit to `HORIZON_RT_SKIP` that makes the translucent candidates pass untinted, to measure that share.
2. **The forest's forced-opaque figure moved from 2.99 to 1.66 ms between two runs** (three reports only the second
   time): remeasure with more reports before concluding.
3. **Does the threaded light cells change land `rt_grid` under 1 ms?** The next flight measurement says.

## Next, in order

1. **Validate bricks 1 and 2 on the GPU** (above). Fix what the layers report. Then the picture with and without
   `HORIZON_RT_PIPELINE=1` (same, outside glass and water), then `rays: trace` in `gpu-passes`.
2. **Port ReSTIR GI into the pipeline** (`shadow_rays.wgsl` `restir_gi`, `hw_hit`, the radiance cache reads and
   writes), the real target of shader execution reordering: the bounce rays' hits shaded after the reorder. The sun
   pass is the template: same descriptor set (extend it with the kernel's other bindings: masks, reservoirs, cache,
   lights, cells), same `Rt` uniform, one more raygen. Keep the WGSL kernel as the fallback and compare captures.
3. **Opacity micromaps for the cutouts** (`VK_EXT_opacity_micromap`): per cutout sprite a 2-state or 4-state micromap
   (level 4: 256 microtriangles a triangle, a texel each on 16x16 sprites, "unknown" where a microtriangle mixes),
   referenced per triangle by the sprite layer. Needs the section BLAS built raw too (wgpu's build takes no pNext), which
   the raw TLAS already allows (any device address works as an instance). Worth up to the forest's 1 to 2 ms.
4. **CPU**: after the light cells, what the next flight shows (`rt_moving` goes with the raw TLAS; `update_world`'s
   remainder; `far heights` 0.4 ms needs a list of changed tiles from the LOD, see `gfx/far_heights.rs` `set_far`).
5. **NGX (DLSS Ray Reconstruction)**: the user's decision on NVIDIA's binaries and licence; then rays at about 60 %
   of the eye's size and the denoiser replaced. The largest remaining GPU lever.
6. Later: a neural radiance cache on `VK_NV_cooperative_vector`; CLAS/PTLAS only if a measurement ever shows the
   builds mattering (they do not today).

## Rules this branch runs under

- CLAUDE.md's rules hold: never commit Mojang's files; PR bodies start with the agent line ("> 🤖 Opened by
  Thibault's AI agent (Claude Code), not by Thibault."), comments with "🤖 Agent:", commits end with
  `Co-Authored-By` and the session line; `skip-release` on this PR; small commits with a why.
- Checks before a push (no GPU here): `cargo fmt --all -- --check`, `cargo clippy -p horizon-client --all-targets
  --no-deps -- -D warnings` with and without `--features rtx`, `cargo test -p horizon-client --lib --bins --features
  rtx`, `cargo test -p horizon-rtx`, `python3 -m unittest discover -s tests`; the shaders' `.spv` recompiled with
  `glslangValidator` when the GLSL changes (`apt-get install glslang-tools`). `CARGO_INCREMENTAL=0` keeps the disk
  within its allowance. The machine needs `libasound2-dev libudev-dev` for the default features.
- The vendored `third_party/wgpu-hal` and `wgpu-core` carry Horizon patches (marked "Horizon" in comments): the TLAS
  CPU trims, `AccelerationStructure::raw_handle`/`raw_buffer`, the new hal `tlas_instance_write_bytes`.
- Agents: the user is fine with Opus subagents for parallel, file-disjoint work (the LOD, the rain/far heights, the
  opaque geometries and the first CPU map were done that way); the lead keeps `hw_rays.rs`, `voxels.rs`,
  `shadow_rays.*` and `horizon-rtx`, and commits.
