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

3. **The bounce pass: ReSTIR GI whole in the pipeline** (`horizon-rtx/shaders/gi.rgen`, a second ray generation
   of the sun pass's pipeline, its set and its hit group): `HORIZON_RT_PIPELINE=gi` (or `all` with the sun pass),
   `HORIZON_RT_SER=0` to trace without reordering and time what it gives. The kernel's `restir_gi` ported one for
   one, each hit shaded after `reorderThreadNV`: the new sample (cosine-weighted ray, the hit's colour from its
   sprite or the entity atlas, its sun through a shadow ray, the rest from the radiance cache or the last frame's
   screen, else a second bounce lit the same way), then the reuse (the last reservoir where the surface was and two
   neighbours', the visibility ray) and the kernel's turns (`tile_of`, `restir_last`). It reads the kernel's sky
   table, last masks, last reservoirs and cache (set bindings 18 to 24) and writes the reservoir as the kernel
   stores it (`gi_a`, `gi_b`), which the kernel takes at bindings 62 and 63 (`restir_gi`, `rt.mode.w` bit 1).
   Left to the kernel (`gi_b` w 0) so the picture stays its own: a ray (or a sun ray at a hit) that crossed water,
   glass or ice (`sun.rahit` sets the payload below 0 on a translucent candidate); a hit with no sprite colour; a
   second bounce lit by neither cache (`open_light`); the Eye of Aldhani; texels the kernel rests. **One
   difference:** the reuse's visibility ray meets the drawn triangles, where the kernel's marches the voxel grid
   (`ray`): compare captures. Gap: VR's second eye still runs the pass where it takes the first eye's light.
   At stake: GI was 70 % of the trace (about 1.5 of 2.2 ms in open terrain), against the pass's own cost (the sun
   pass, a coherent ray SER can't help, cost 0.4 to 0.7 ms more than its ray in the kernel) and what a ray
   generation can't do that the kernel does (workgroup tiles skipped whole, shared rays in VR's periphery).

4. **Opacity micromaps on the cutout sections** (on with `rtx`, `HORIZON_RT_OMM=0` turns them off; measured 5 November,
   below; `horizon-rtx/src/micromap.rs`
   the CPU side, `omm.rs` the raw builds, `gfx/hw_micromaps.rs` the game's side): `VK_EXT_opacity_micromap`, the
   Vulkan side of DXR 1.2's opacity micromaps (CLAS, PTLAS and the other new builds target BLAS rebuilds, 0.12 to
   0.35 ms here). When a section is meshed, each triangle of its alpha-tested geometries gets a 4-state micromap from
   its sprite's alpha (the array's top level, `>= 128` opaque as `hw_opaque`) and its texture coordinates: about a
   micro-triangle a texel (level 4 on a 16x16 face), two levels finer when a quarter is left unknown; uniform
   triangles take a special index (opaque leaves, sprites past `SPRITE_SPLIT`: fully opaque; animated ones: unknown).
   Identical triangles share their micromap (a cache by layer and UVs, then per section). The section's structure
   (wgpu's) is then built by a raw command with the micromap attached (only with the raw TLAS), after the size check
   against wgpu's (`wgpu-hal` patch: `AccelerationStructure::size`); what doesn't fit (pool 64 MB, ring slot 4 MB a
   frame, the size) is built by wgpu as before. horizon-hdr enables the extension and its feature; the sun/GI pipeline
   gets `RAY_TRACING_OPACITY_MICROMAP`. Log lines: "built with opacity micromaps, ... MB of pool" at start, then every
   500 sections the count, those left to wgpu and the share of alpha-tested triangles still left to the shader.
   Measure the forest (`rays: trace`, with and without the variable, pipeline on and off): the any-hits and the
   kernel's candidate loop on leaves should mostly go. Risks without validation: the full barriers
   (`ALL_COMMANDS`), the ring slot reuse four frames later, the 4-state index type `UINT32`.
   **Measured (FIXE01, 5 November, `HORIZON_RT_PIPELINE=all`):** the structure with its micromap takes 35 to 45 %
   more than wgpu makes for the quads (more on small sections), so these sections' structures are made for 8/5 their
   quads plus 16 a geometry. Then 4627 sections carry micromaps (173 left to wgpu), 5 % of their alpha-tested
   triangles left to the shader, 56 of the 64 MB of pool used. Forest `rays: trace` 3.86 to 2.59 ms, 0.14 ms from
   the force-opaque floor (2.45); the frame median moves less than its 0.7 ms spread between sessions. Without the
   pipeline the kernel's ray queries gained nothing in the first (overflowing) run: remeasure. Open terrain: the
   translucent tint (`HORIZON_RT_SKIP=128` passes glass, water and ice untinted) costs 0.55 ms, 1.0 ms to the floor:
   the water's sixteen waves per candidate are the next target.

**First runs:** with the Vulkan validation layers (`VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation`), log lines
"Horizon RTX: the top-level structure built by raw Vulkan commands" and "the sun pass as a ray tracing pipeline".
Likely trouble spots if validation complains: the scratch alignment (`SCRATCH_ALIGN` 256, rounded address), the image
layouts in the descriptor writes (`DEPTH_STENCIL_READ_ONLY_OPTIMAL` for the depth, `SHADER_READ_ONLY_OPTIMAL` for the
sprites and atlas, `GENERAL` for the output: what wgpu-hal's `derive_image_layout` gives the states asked for), the
barriers' stages, a buffer whose usage lacks what the transition needs.

## Update, 6 November: toward 90 fps in the headset (PR #2)

The target is now 90 fps in VR, both eyes at 4400x3520 (KRVR's RenderResolution 4400 on the user's Apple Vision Pro;
the first fake-runtime runs used 3132x3016). PR #2 (`claude/project-thread-tb0ay4`, based on this branch) holds:

- **VR's device gets the RTX extensions** (micromaps, SER, ray tracing pipelines: `horizon-xr` `rtx_extensions`).
- **In VR only the first eye touches `hw_rays`' state** (`hw_update`, raw builds, the TLAS): the second eye's raw BLAS
  builds used to replace the first's before submission, which lost the device with micromaps (4956cf386). Keep it so.
- **Measured at 3132x3016** (fake runtime, `--hz 1000`, look_around, ~9000 frames): GPU 8.60 ms a frame with micromaps,
  9.01 without; CPU median 6.3. First eye 4.75 ms: both eyes' depth prepass 0.96, raster 0.60, structures 0.59,
  trace 0.49, froxels inject 0.41, denoiser 0.40, integrate 0.31, TAA 0.26. Second eye 2.23 ms. The per-eye tables are
  in the project files (`vr-4956-report.txt`).
- **Switches added for the A/B** (each gain on by default, the variable restores the old behaviour):
  `HORIZON_RT_FAR_EVERY_FRAME=1` (the far field reran every frame, twice a VR frame: now when a shadow would move),
  `HORIZON_RT_PREPASS_SPLIT=0` (the rays' prepass drew every quad through the alpha test: now the opaque ones without a
  fragment shader), `HORIZON_FROXEL_TILE` (16 by default; the VR eye has 3.1 times the screen's froxels at 4400x3520),
  `HORIZON_WORLD_PREPASS=1|2` (the world over a depth prepass, off by default until measured).
- **Measured at 4400x3520** (GPU a frame): 4956cf386 12.03 ms, the prepass split 11.85 (11.98 without), the far field
  gate -0.5, froxels of 24 pixels 11.16 (now VR's default, e60ba36e5), 97ffe98c7 11.00. A later session read 12.58
  for a build with no relevant change: its tables showed the hand drawn, `froxels: apply` doubled (fog) and the raster
  up 0.5 an eye, another scene. Compare VR builds back to back, the time and the weather fixed
  (`/time set 6000`, `doDaylightCycle false`, `/weather clear`, `doWeatherCycle false`).
- **Measured on integ-final** (PRs #2 to #12 merged, 4400x3520): 11.14 ms a frame. The denoiser filtering VR's
  periphery by its 4x4 blocks (2db53a830, `HORIZON_RT_DENOISE_FOVEA=0` for the old) gives 0.11 ms; VR's rays' prepass
  at the rays' resolution, 2200x1760 an eye (486eb379a, `HORIZON_RT_PREPASS_HALF=0` for the old), gives 0.76 ms.
- **VR stalls, closed:** the fake runtime froze a frame about 2.5 s every 180 frames, writing its captures inside
  `xrEndFrame`; it writes them on a thread of its own now. KRVR's own log shows no such waits. Without them, integ-final
  is at 10.10 ms a frame (eye 1 6.37, eye 2 3.67), the means of `Horizon VR: GPU means` over the same frames.
- **Desktop, closed:** between b5b5252a7 and 482620a06 the rays' prepass read 0.33 ms more, `world: phase 1` 0.9 to
  1.1, the forest's trace 0.65. An A-B-A run with the clocks logged found the game capped at 30 fps, the GPU down to
  1635 MHz for f05270165 against 2200 to 2300 for b5b5252a7: at the same clock the prepass is 0.06 ms more.
  integ-final's trace read 0.9 ms more at 1905 MHz, not yet compared at one clock. Compare uncapped, or with the
  clocks locked and logged.
- **The second eye's denoiser** (0.53 to 0.63 ms) filters only what that eye traced itself: where it took the first
  eye's light, it copies the first eye's denoised light (`HORIZON_RT_SHARE_DENOISED=0` for the old, docs/raytracing.md
  item 10). Measured on 109a15996: that denoiser 0.59 to 0.25 ms.
- **The grading and the ACES curve in one pass** when nothing reads the graded light after (no HDR output,
  screenshot or video): one full-size float target written and read less an eye; the stages `grading` and
  `tone curve` become `grading and tone curve (ACES)` (`HORIZON_GRADE_FUSED=0` for the old). Measured on 109a15996:
  0.168 to 0.062 ms an eye in VR.
- **The mirrors' search ray only where water or glass lies in front** (docs/raytracing.md item 11): a cover of the
  translucent terrain over the rays' prepass, and the kernel skips the camera ray that looked for water or glass
  elsewhere (`HORIZON_RT_PSR_COVER=0` for the old; `HORIZON_RT_SKIP=8` drops the mirrors whole, the upper bound).
  Measured on 109a15996, a view without water: the first eye's trace 0.09 ms less, the second's 0.05, the cover
  0.05 ms (for both eyes).
- **Measured on 109a15996** (VR 4400x3520, fake runtime without captures, 6000, clear, GPU unlocked at 2880 MHz, ten
  runs back to back): **8.96 ms a frame** (eye 1 5.88, eye 2 2.98), under the 11.1 of 90 Hz. Without VRS 11.36. The
  first eye's largest stages: the world's raster 1.17, both eyes' rays' prepass 1.02, the trace 0.76 (ReSTIR GI 0.27
  of it, `HORIZON_RT_SKIP=2`; the sun, the light sources, the mirrors and the far field in the noise), the denoiser
  0.6, structures 0.58, TAA 0.32. Run to run noise about 0.3 ms; the first run of a series is the slowest. The
  per-stage table is `pr2-item0-109a.txt` in the project files.
- **The TAA's copy back only when something draws over the scene** (docs/raytracing.md item 12,
  `HORIZON_TAA_BLIT=1` for the old), **ReSTIR GI's visibility ray with hardware rays** (item 13,
  `HORIZON_RT_GI_DDA=1`), **shadow rays ending on their first hit** (item 14, `HORIZON_RT_FIRST_HIT=0`). Measured on
  dd9002142 (`pr2-dd90.txt`): 8.31 ms a frame (eye 1 5.38, eye 2 2.82); the TAA 0.38 to 0.25 ms an eye while no
  vignette shows; the first hit about 0.05 ms; the visibility within the noise. Two runs of the defaults differ by
  0.6 ms at one clock: the fake world's spawn moves from launch to launch, so captures at a fixed pose need a
  teleport to fixed coordinates (`spreadplayers` failed there).
- **The vignette in the tone mapping's pass, over the finished frame as vanilla draws it** (item 15,
  `HORIZON_VIGNETTE_CURVE=0` for the old): its quad was 0.087 ms an eye and kept the TAA's copy back. Measured on
  68218d890 at a fixed pose (a platform at 160 120 160, the vignette forced with `/worldborder warning distance
  2147483647`): 9.18 and 9.30 ms a frame against 9.66, the screen effects' stage 0.081 to 0.001 ms. The old look was
  redder at the edges than vanilla's. At the same pose `HORIZON_RT_GI_DDA=1` costs the same as the default, and
  `HORIZON_RT_SHARE_DENOISED=0` 9.44 against 9.19.
- **The shared denoised light, checked texel by texel** (docs/raytracing.md item 10, `HORIZON_RT_SHARE_CHECK=0` for
  the old): with 68218d890 a block at the right eye's right edge, which the left eye doesn't see, came out black
  with the shared light on and lit with `HORIZON_RT_SHARE_DENOISED=0`. The periphery's blocks shifted their centre's
  first-eye texel onto all sixteen texels, unchecked, so a texel past an edge took the sky's light. Each texel now
  takes the first eye's texel showing its own surface, or is filtered from its own light. Measured on 93137e0d4: no
  cost, but the black zone stays (nearly the same with `HORIZON_RT_SHARE_CHECK=0`), so it is not the periphery's
  shift. `HORIZON_RT_DEBUG=12` shows what the second eye took, and `HORIZON_RT_SHARE_GUARD=1` is a trial guard
  (docs/raytracing.md item 10).
- **The right eye's black zone, fixed** (29c78b580): since 4956cf386 the camera's offset to the ray structures' anchor
  (`hw_offset`) was set only with the structures' update, which the first eye alone runs, so the second eye's own
  hardware rays left from the wrong place: its sun shadowed, its light around low, wherever the left eye doesn't
  show the surface. The shared light only hid it. `HORIZON_RT_DEBUG=14` (the lit colour's parts) found it; 13
  checks the colour's inputs. Measured on 29c78b580 at the fixed pose: the zone as the left eye (64 62 60), 0.16 of
  255 from `HORIZON_RT_SHARE_DENOISED=0`; 9.510 and 9.517 ms a frame against 9.880 without the shared light (eye 2's
  denoiser 0.209 against 0.454 ms; tables in the project files' `pr2-29c7-means.txt`).
- **Ideas weighed and left:** the second eye skipping the exposure meter (the grading's local tone mapping reads the
  meter's chain, level 2, so the eye needs its own); a conservative depth reuse across the TAA jitter (holes through
  cutout gaps); the TAA's blit folded into its resolve (the hand and the rain draw over the resolved scene, so the
  history needs its own copy). **Still open:** VR's world raster over the prepass (needs one matrix for both, the
  jitter), occlusion culling in the eyes (the eyes can't share a depth-based cull: it would test against both layers
  of the multiview prepass).

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
2. **Port ReSTIR GI into the pipeline** (done, untested: the bounce pass, above; measure it with
   `HORIZON_RT_SER` on and off, against the kernel alone) (`shadow_rays.wgsl` `restir_gi`, `hw_hit`, the radiance cache reads and
   writes), the real target of shader execution reordering: the bounce rays' hits shaded after the reorder. The sun
   pass is the template: same descriptor set (extend it with the kernel's other bindings: masks, reservoirs, cache,
   lights, cells), same `Rt` uniform, one more raygen. Keep the WGSL kernel as the fallback and compare captures.
3. **Opacity micromaps for the cutouts** (done, untested: item 4 above, `HORIZON_RT_OMM=1`). Was: per cutout sprite a 2-state or 4-state micromap
   (level 4: 256 microtriangles a triangle, a texel each on 16x16 sprites, "unknown" where a microtriangle mixes),
   referenced per triangle by the sprite layer. Needs the section BLAS built raw too (wgpu's build takes no pNext), which
   the raw TLAS already allows (any device address works as an instance). Worth up to the forest's 1 to 2 ms.
4. **CPU**: after the light cells, what the next flight shows (`rt_moving` goes with the raw TLAS; `update_world`'s
   remainder; `far heights` 0.4 ms needs a list of changed tiles from the LOD, see `gfx/far_heights.rs` `set_far`).
5. **NGX (DLSS Ray Reconstruction)**: left for an optional native mod; NVIDIA's SDK may not be in the repository.
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
