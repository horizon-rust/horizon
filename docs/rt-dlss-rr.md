# DLSS Ray Reconstruction for RT lighting (design; stages 1, 4 and 5 wired on the desktop)

Part of the RTX 4090 build ([rtx-4090.md](rtx-4090.md)): Windows, an RTX 4090, Vulkan only (no DX12), VR on
Apple Vision Pro through an OpenXR runtime (two eyes of 4400x3520 at 90 Hz) and the desktop window. This
branch already drops main's rule of "no NVIDIA DLL at run time" ([raytracing.md](raytracing.md)). The owner
approved NVIDIA's proprietary binaries for this.

**Why.** An eye-1 frame in VR measures 5.97 ms, of which trace 1.60, the denoiser (`denoise.wgsl`, 5 à-trous
passes) 0.55 and TAA (`taa.wgsl`) 0.45, plus the accumulation inside the trace kernel (`accumulate`,
MAX_HISTORY 30). DLSS Ray Reconstruction (RR, NGX's feature "DLSS-D", `NVSDK_NGX_Feature_RayReconstruction` =
13) replaces the accumulation, the denoiser and TAA with one learned pass, and it can upscale, so the rays
can be traced at a lower internal resolution. That is the biggest lever left for both quality and cost.

**State.** The code is `crates/horizon-rtx/src/dlss_rr.rs`, behind the cargo feature `dlss-rr` (off by default;
`horizon-client` and `horizon-client-render` pass it down, and it turns `rtx` on). Without the feature every crate
builds as it did before. With it, nothing runs unless `HORIZON_RT_DLSS_RR=1`. **Stage 1 is wired** (desktop
window only): with the switch, NGX's instance and device extensions are enabled when the window's Vulkan
instance and device are made, NGX is initialised on the device, its render sizes are logged, and one DLSS-RR
feature instance is made for the window's size, made again on resize and released before NGX shuts down.
Stage 1 ran on the RTX 4090 (SDK v310.9.16, 10 November 2026; §7). **Stages 4 and 5 are wired, not yet
run on a GPU** (§9): with the switch, the window's view is traced raw (no accumulation, no rest in turns,
no à-trous filtering), the world is drawn with RR's jitter, and RR is evaluated where the TAA resolve ran,
its output drawn back over the scene. Stage 5 then ran on the RTX 4090 (clean, sharp, 2.6 ms at
2580x1080). On RR's frames the guides are now made from the world pass's own jittered depth, with the
entities, water and glass in it, and moving entities have motion vectors of their own (§3.1, §9). Without
the switch (or without the feature) the kernel, the denoiser, TAA and every shader's output are as before.
The tester's steps are in §8 (stage 1) and §9 (stages 4 and 5, and the guides).

## 1. Loading NGX on Vulkan from Rust

There are no official Rust bindings. The SDK (github.com/NVIDIA/DLSS) ships these pieces:

- **headers**: `nvsdk_ngx.h`, `nvsdk_ngx_vk.h`, `nvsdk_ngx_defs*.h`, `nvsdk_ngx_params*.h`, and
  `nvsdk_ngx_helpers*_vk.h`. The helpers are `static inline` C functions such as `NGX_VULKAN_CREATE_DLSSD_EXT1`
  and `NGX_VULKAN_EVALUATE_DLSSD_EXT`. They do nothing more than set named parameters and call the exported
  functions.
- **a static library**, `nvsdk_ngx`. It exports the C functions and loads the feature DLL at run time. The
  files are `lib/Windows_x86_64/x64/nvsdk_ngx_d.lib` (built for the dynamic CRT, which Rust's MSVC targets use by
  default) and `nvsdk_ngx_s.lib` (for `crt-static`). On Linux it is `lib/Linux_x86_64/libnvsdk_ngx.a`, which
  needs `stdc++` and `dl`.
- **the runtime**, `nvngx_dlssd.dll`, holding the RR network. The driver can update it. The static library
  looks for it in the folders given in `NVSDK_NGX_FeatureCommonInfo::PathListInfo` and beside the executable.

There are two ways to bind it:

| | Hand-written `ffi` + link the static lib (chosen) | Small C shim (`cc` + the SDK's helpers) |
|---|---|---|
| Build needs | `DLSS_SDK_DIR` only | `DLSS_SDK_DIR`, the Vulkan SDK headers and a C compiler (MSVC/clang) |
| Fidelity to the helpers | Re-implements about 40 `Set*` calls (names checked, see §7) | Exact, calls NVIDIA's inline code |
| Breakage on SDK update | Parameter names are a stable string ABI; struct layouts are pinned by a test | None from the helpers |
| bindgen | Not needed | Optional |

The hand-written layer is the realistic option. The ABI is small: about 12 `NVSDK_NGX_VULKAN_*` functions,
8 `NVSDK_NGX_Parameter_*` functions, 10 small `#[repr(C)]` structs and string keys. `NVSDK_CONV` is `__cdecl`
on MSVC (on x86-64 that is the C convention) and empty on GCC/Clang, so `extern "C"` is correct. dlss_wgpu
(the Bevy community crate, docs.rs/dlss_wgpu) takes the shim route, with bindgen and `wrap_static_fns`. If the
hand-written helpers ever drift from NVIDIA's, the shim is the fallback.

**Build (`crates/horizon-rtx/build.rs`).** Without `dlss-rr` the script does nothing. With `dlss-rr`:

- With `DLSS_SDK_DIR` set and the library present, it links `nvsdk_ngx_{d,s}` (Windows) or `nvsdk_ngx` +
  `stdc++` + `dl` (Linux).
- With `DLSS_SDK_DIR` missing or wrong, the build fails and says so.
- With `HORIZON_DLSS_RR_NO_LINK=1` and no SDK, it only prints a warning and links nothing. That is enough for
  `cargo check` and `cargo clippy` on Linux CI or this container. A binary that calls NGX then fails to link.

This is the only exception to the hard error.

**Calls, in order (`dlss_rr.rs`):**

1. Before the Vulkan instance exists, call `required_instance_extensions()`, which wraps
   `NVSDK_NGX_VULKAN_GetFeatureInstanceExtensionRequirements(&FeatureDiscoveryInfo, &count, &VkExtensionProperties*)`.
   `FeatureDiscoveryInfo` carries the SDK version (`NVSDK_NGX_Version_API` = 0x15), the feature id 13, a
   project id with `NVSDK_NGX_ENGINE_TYPE_CUSTOM`, a data path and `FeatureCommonInfo` (DLL paths, logging).
2. Before the device exists, call `required_device_extensions()`. It calls
   `NVSDK_NGX_VULKAN_GetFeatureRequirements(instance, physical, &info, &FeatureRequirement)`, then
   `..._GetFeatureDeviceExtensionRequirements(instance, physical, &info, &count, &props)`. It returns `Err` on a GPU
   or driver without RR: that is the fallback point. The deprecated `NVSDK_NGX_VULKAN_RequiredExtensions` is not
   used.
3. Call `Ngx::init()`, which runs `NVSDK_NGX_VULKAN_Init_with_ProjectID(projectId, engineType, engineVersion,
   dataPath, VkInstance, VkPhysicalDevice, VkDevice, vkGetInstanceProcAddr, vkGetDeviceProcAddr,
   &FeatureCommonInfo, NVSDK_NGX_Version_API)`. Both loader functions are passed (the header calls them
   optional; dlss_wgpu passes the entry's `vkGetInstanceProcAddr` and the instance's `vkGetDeviceProcAddr`, and
   so does Horizon now; the scaffold passed NULL for the second). Then `NVSDK_NGX_VULKAN_GetCapabilityParameters`. The
   `SuperSamplingDenoising.Available` key must be 1. If it is not, `SuperSamplingDenoising.NeedsUpdatedDriver` and
   `.FeatureInitResult` explain why. `NVSDK_NGX_VULKAN_Init(appId: u64, ...)` is the alternative for an
   NVIDIA-assigned application id. `HORIZON_DLSS_PROJECT_ID` overrides the built-in UUID.
4. `DlssRr::new()` allocates a parameter block (`NVSDK_NGX_VULKAN_AllocateParameters`). It sets what
   `NGX_VULKAN_CREATE_DLSSD_EXT1` sets: node masks 1, `Width`/`Height` (render size), `OutWidth`/`OutHeight`,
   `PerfQualityValue`, `DLSS.Feature.Create.Flags`, `DLSS.Enable.Output.Subrects`, `DLSS.Denoise.Mode` =
   DLUnified, `DLSS.Roughness.Mode` and `DLSS.Use.HW.Depth`. Then it calls
   `NVSDK_NGX_VULKAN_CreateFeature1(VkDevice, VkCommandBuffer, 13, params, &handle)`, which records setup
   commands into a command buffer.
5. Each frame, `DlssRr::evaluate()` (or `record()`, which also does the wgpu split) sets what
   `NGX_VULKAN_EVALUATE_DLSSD_EXT` sets (§3), then calls `NVSDK_NGX_VULKAN_EvaluateFeature_C(cmd, handle, params,
   NULL)`.
6. `Drop` calls `NVSDK_NGX_VULKAN_ReleaseFeature` and `DestroyParameters`. The last `Arc<Ngx>` waits for the
   device's submitted work (`Device::poll(wait)`), then calls `DestroyParameters` on the capability parameters
   and `NVSDK_NGX_VULKAN_Shutdown1(VkDevice)`. `Ngx` holds a `wgpu::Device`, so the VkDevice cannot be
   destroyed before NGX shuts down. NGX is initialised once per device (a second `Ngx::init` on the same device
   is refused, since one `Shutdown1` would end both).

Between 3 and 4, `Ngx::optimal_settings` does what the SDK's inline `NGX_DLSSD_GET_OPTIMAL_SETTINGS` does: it
reads the `DLSSDOptimalSettingsCallback` pointer from the capability parameters (none means an old DLL or
driver: `FAIL_OutOfDate`), sets `Width`, `Height`, `PerfQualityValue` and `RTXValue` = 0 on them, calls it, and
reads `OutWidth`/`OutHeight` and the `DLSS.Get.Dynamic.{Min,Max}.Render.{Width,Height}` range.

## 2. How it fits wgpu

**Extensions at creation. This is the main integration risk.** NGX's instance and device extensions must be
enabled when Vulkan objects are created. wgpu-hal's callbacks take `&'static CStr`, so `dlss_rr::leak()` leaks
the few names once. **Done for the desktop window (stage 1)** as described below; the original analysis follows
it.

- **Instance (done)**: `open_backend` (`gfx.rs`, `dlss_rr::create_instance`) makes wgpu's instance as before,
  asks NGX for its instance list (`GetFeatureInstanceExtensionRequirements`, no instance needed) and compares
  it with what wgpu-hal enabled (`InstanceShared::extensions`). If every name is there, that instance is kept
  unchanged. If not, and the loader offers the missing names, the instance is made again with
  `wgpu::hal::vulkan::Instance::init_with_callback` (the missing names pushed in the callback) and
  `wgpu::Instance::from_hal`, as dlss_wgpu does. Cost of that second path: wgpu-core then takes the
  environment's flags (`third_party/wgpu-core/src/instance.rs`, `from_hal_instance`), so a release build
  validates indirect draws unless `WGPU_VALIDATION_INDIRECT_CALL=0` (as in VR). The log says which path ran.
  If the loader lacks a name, wgpu's instance is kept and RR stays off.
- **Device (done)**: `horizon_hdr::open_vulkan_device_with(adapter, desc, extra)`. Its `extra` list comes from
  `dlss_rr::device_extensions` (`GetFeatureRequirements`, then `GetFeatureDeviceExtensionRequirements`). It is
  empty when the GPU or driver lacks RR or when the driver does not offer every name, so the device is then made
  exactly as before. With names, the HDR-metadata check is no longer a reason to leave opening to wgpu; each
  name is pushed only if offered; and if that open fails, the device is opened again without them. With no
  extra names, `open_vulkan_device_with` is `open_vulkan_device`, whose behaviour is unchanged. After the device
  exists, `Session::start` checks again, from the device itself, that every instance and device name NGX wants
  is enabled, logs both lists, and refuses to init NGX otherwise.
- **VR (not done)**: `crates/horizon-xr` is untouched; the list below still applies there, and `Renderer::new`
  logs "not wired for VR yet" with the switch on in VR.

- **Desktop instance**: `wgpu::Instance::new(instance_descriptor(..))` (`crates/horizon-client-render/src/gfx.rs:2940`,
  `fn instance_descriptor` at `gfx.rs:2370`) has no hook. Two options:
  - When RR is wanted, build it with `wgpu::hal::vulkan::Instance::init_with_callback` (`third_party/wgpu-hal/src/vulkan/instance.rs:570`,
    `CreateInstanceCallbackArgs::extensions`) and `wgpu::Instance::from_hal`, as dlss_wgpu does.
  - Or check at start that NGX's list is a subset of what wgpu-hal already enables
    (`Instance::desired_extensions`, `instance.rs:227`), and turn RR off otherwise.

  NGX's instance list is expected to be `VK_KHR_get_physical_device_properties2`-class extensions that wgpu
  already has, but this is **unverified**.
- **Desktop device**: `horizon_hdr::open_vulkan_device` (`crates/horizon-hdr/src/lib.rs:13-86`) already uses
  `open_with_callback` and pushes extra extensions (HDR metadata, SER, opacity micromaps). RR's names go into
  the same callback (`args.extensions.push`). **There is a trap:** that function returns `None` when the GPU
  lacks `VK_EXT_hdr_metadata` (`lib.rs:31-33`), and `gfx.rs:3033-3042` then falls back to plain
  `adapter.request_device`, which adds no extensions at all. The HDR check has to become optional for the RR
  extensions, or RR has to be turned off on that path. Note also that `open_with_callback` has an
  `enumerate_device_extension_properties` gate. Each RR name has to be checked as offered before it is pushed,
  or device creation fails.
- **VR**: OpenXR creates both objects (`crates/horizon-xr/src/lib.rs`, `open_vulkan`).
  - Add NGX's instance names to `instance_extensions` before `xr.create_vulkan_instance` (`lib.rs:1062-1080`).
  - Add NGX's device names to `device_extensions` beside `RtxExtensions` (`lib.rs:990-1040`, pushed at
    `lib.rs:1143-1150`) before `xr.create_vulkan_device`.
  - `required_device_extensions` needs the raw instance and physical device, which exist at that point
    (`physical`, `ash_instance`).
- **Raw handles**: `dlss_rr::Image::of(adapter, view)` takes `TextureView::as_hal::<Vulkan>().raw_handle()`
  (VkImageView), `Texture::as_hal::<Vulkan>().raw_handle()` (VkImage) and
  `Adapter::texture_format_as_raw` (VkFormat, the same mapping wgpu uses). The device handles come from
  `Device::as_hal::<Vulkan>()`, as in `RawTlas::new` (`crates/horizon-rtx/src/lib.rs`).
- **Recording**: wgpu records an encoder's commands only at `finish`, so raw commands cannot share an encoder
  with wgpu's (`RawTlas::build`). Use the sun pass's split instead (`crates/horizon-rtx/src/sun.rs:593-601`):
  1. `encoder.transition_resources(..)` puts every input into `TextureUses::RESOURCE`
     (`SHADER_READ_ONLY_OPTIMAL`, or `DEPTH_STENCIL_READ_ONLY_OPTIMAL` for depth) and the output into
     `STORAGE_READ_WRITE` (`GENERAL`).
  2. The encoder is finished into `split_commands`.
  3. A fresh encoder takes the raw `EvaluateFeature` through `as_hal_mut`.
  4. Recording carries on in a third encoder.

  `gfx.rs:14900` submits `split_commands` before the frame's last buffer, in order. NGX records its own
  barriers inside the call and leaves each image in the layout it found. wgpu's tracker still believes those
  layouts, so no hand-back is needed. That behaviour is the documented contract of NGX's Vulkan path and is
  **unverified** here; check it with the validation layers. dlss_wgpu documents that DLSS triggers Vulkan
  validation errors that are safe to ignore.
- **Command buffer for `CreateFeature1` (done)**: `Session::create` makes a wgpu command encoder, takes its raw
  `VkCommandBuffer` through `as_hal_mut` (wgpu-core opens it, so it is recording, as the header requires),
  calls `CreateFeature1` into it, finishes it and submits it on its own with `queue.submit`, outside any
  frame. Before a feature is released (resize, exit), `Device::poll(wait)` makes sure that submission has run.

## 3. Inputs: what RR reads and where it comes from

How the renderer composes light today: the kernel writes **light masks** at the rays' resolution (sun
visibility, block light, direct light, the sun's colour, reflection, refraction:
`crates/horizon-client-render/src/gfx/shadow_rays.wgsl:96,345,353,362,388,391`). The **world raster pass** then
multiplies them into the textured terrain at full resolution, using a depth-aware upsampling (`terrain.wgsl:1018`
`rt_taps_at`; `fs_solid`/`fs_opaque` at `terrain.wgsl:3050-3054`). No "colour" image exists before the scene
image, and no G-buffer exists either. RR's noisy colour is therefore **the world pass's output when it reads
un-accumulated, un-denoised masks**.

| RR input (NGX key) | Exists today | Missing / to do |
|---|---|---|
| Noisy colour (`Color`): 1 spp radiance × albedo, linear HDR | The scene (`tonemap.rs:119` `SCENE_FORMAT` Rgba16Float), composed in `terrain.wgsl`. Its masks are accumulated over up to 30 frames (`shadow_rays.wgsl:1845` MAX_HISTORY, `accumulate` at `:1915`) and denoised (`denoise.wgsl`) | A kernel mode that returns `trace`'s result unaccumulated (skip `accumulate`, `:1915`). It also has to stop the "rest in turns" scheduling (`rt.frame.z`, `trace` at `:1665`), because RR wants a fresh sample at every pixel every frame. The denoiser is skipped. The terrain reads the raw masks. ReSTIR's temporal reuse can stay (RR is meant to sit on top of ReSTIR). **Done (stage 4, §9)**: `raw_frame` |
| Output (`Output`) | | A storage-capable Rgba16Float at output size, which replaces TAA's history as the resolved scene (`taa.rs:332` `resolve`; called at `gfx.rs:14363-14378`) |
| Depth (`Depth`) | Depth32Float, reversed Z (`GreaterEqual`, `gfx.rs:3687,3838`). In VR the prepass is at the rays' resolution (`gfx.rs:1787` `half_prepass`, `:8409-8418`) | Hardware depth: `DLSS.Use.HW.Depth` = 1 plus `DepthInverted`. It has to be the depth of the colour's own pixel grid |
| Motion vectors (`MotionVectors`) | **Guide** (§3.1): `rt_guides` writes an Rg16Float at the render size, render pixels from this frame's position to last frame's (previous − current, x right, y down), with the jitter taken out, from the depth and the kernel's `prev_view_proj`/`motion`; the sky by its direction. On RR's frames, entities with their own motion from frame generation's `ObjectMotion` (`gfx/frame_gen/motion.rs`), drawn over the world's depth and fed by `dlss_eval` | `MV.Scale` = 1, 1 (already pixels), `MVJittered` = 0. Not covered: entities posed on the GPU (`skin`, not in `Frame::entities`), entities whose vertex layout changed since last frame, water and lava surface animation, falling blocks, pistons, block entities (chests, bells), particles. Use the `DLSS.Input.Bias.Current.Color.Mask` or the transparency layer for the last ones |
| Diffuse albedo (`DLSS.Input.DiffuseAlbedo`) | **Guide** (§3.1): Rgba8Unorm, linear, the texel × vertex tint of the triangle a short hardware ray finds at the depth's surface (`hw_hit`, entities included); under water or glass what lies behind × (1 − F); fallback the voxel grid's map colour, else mid grey; the sky 0.5 (NVIDIA's default) | Particles and the rain have none of their own; refraction is not applied to what lies behind water or glass |
| Specular albedo (`DLSS.Input.SpecularAlbedo`) | **Guide** (§3.1): Rgba8Unorm, the preintegrated environment BRDF of F0, roughness and N·V (Karis 2014's `EnvBRDFApprox`), not F0 | F0 0.04 everywhere (0.02 on puddles and wet films): no LabPBR F0/metals yet. Whether NVIDIA's guide wants exactly this approximation is **unverified** (its sample code could not be fetched) |
| Normals (`GBuffer.Normals`) | **Guide** (§3.1): Rgba16Float, world-space unit normal toward the camera (camera-relative world axes: the instances are only translated), roughness in w; water and glass their flat face normal; the sky 0 | Geometric face normals only (no LabPBR normal maps, no water waves); the depth's own normal where the ray misses. `DLSS.Roughness.Mode` = Packed. `WorldToViewMatrix` and `ViewToClipMatrix` are needed in any case (row-major) |
| Roughness (`GBuffer.Roughness`) | **Guide**, in the normals' w (§3.1): 1, 0.05 on the rain's puddles and 0.2 on wet films (where the kernel mirrors them, `hw_puddle`), blended toward 0 by F on water and glass; the sky 0 | The kernel reads no LabPBR smoothness (its reflections are mirrors on water, glass and puddles only), so neither do the guides |
| Specular hit distance (`DLSSD.SpecularHitDistance`) | Only the water hit's distance (`shadow_rays.wgsl` `water_at`); `hw_mirror`'s hit is not kept | Not made by the guides: `hw_reflect` would have to return its `hw_hit` t (an R16F written by the kernel). Specular motion vectors (`GBuffer.SpecularMvec`) are the alternative |
| Jitter (`Jitter.Offset.X/Y`) | TAA: Halton(2,3) over 16 phases in pixels, image moved right and down (`taa::jitter`, `jittered`). RR: the same sequence over 32 phases (`dlss_eval::PHASES`, `taa::jitter_of`), as NVIDIA's RR guide asks (at least 32) | Pass the same pixels. **Unverified**: NGX's sign convention against "image moves right/down" (`HORIZON_DLSS_RR_JITTER_SIGN`). Test with a still camera (the image must not swim) and the SDK's dev DLL overlay |
| Exposure (`ExposureTexture`, `DLSS.Pre.Exposure`) | Auto-exposure runs after the resolve (`gfx.rs:8127`, `exposure.rs`). The scene is linear, in paper whites | `IsHDR` = 1, pre-exposure 1. Either feed last frame's exposure as a 1x1 R32F texture or set `AutoExposure` |
| Reset (`Reset`) | None. The kernel's history is dropped only when its masks are recreated (resize); `masks.last` at `shadow_rays.rs:1181`, `motion.w` at `:1211-1217` | A reset flag for the view: teleport (a camera jump of more than a few blocks), dimension change, world open, resize, RT toggled, shader pack change. Per view |
| Render subrect (`DLSS.Render.Subrect.Dimensions.*`) | | The used size of the inputs (dynamic resolution) |

### 3.1 The guides pass (`gfx/rt_guides.rs`, `rt_guides.wgsl`)

Off unless `rt_guides::guides_wanted()`: `HORIZON_RT_GUIDES=1`, `HORIZON_RT_DLSS_RR=1` (read directly, no
cargo feature needed) or `HORIZON_RT_GUIDES_SHOW` set. Without it no shader, pipeline, texture or
dispatch exists and the kernel's text is unchanged. It needs hardware rays (the no-hardware kernel makes
no guides) and runs only with RT lighting.

- **Where.** `rt_guides.wgsl` is appended to the hardware kernel (`shadow_rays::source(true, ..)`) into a
  second shader module, entry `cs_guides`. Its pipeline layout is the kernel's group 0 plus its own group
  1 (five storage textures, the depth it reads, the entities' motion, a uniform with the jitter), and it
  runs with the kernel's own bind group for the view: the uniform, structures, arena and sprites as the
  kernel saw them. One texel per depth pixel. Per-view textures, kept for the five views as the kernel
  keeps its masks. `ShadowRays::guides(view)` returns the `Guides` views for the evaluate stage.
- **Which depth.** Every view but RR's: right after the kernel, on the RT depth prepass (solid terrain,
  the clean matrix), jitter 0: the window's render size, VR's prepass at the rays' size (`half_prepass`).
  RR's view (`set_raw_view`): the kernel keeps its bind group (`raw_guides`) and `ShadowRays::run_guides`
  dispatches the pass just before the evaluation, on the **world pass's own depth**, the one RR is given:
  jittered as the colour is, with the entities, water, glass and everything else that writes depth (the
  hand is drawn after the evaluation, so it is not in it). The pass reads each pixel where its sample lies
  on the unjittered grid (`guide_centre`: the pixel's centre less the jitter, since `taa::jittered`
  moves a point `jitter` pixels right and down), so positions, normals, albedos and motion all belong to
  the colour's own sample.
- **How a pixel is found.** The depth gives the camera-relative position. One hardware ray (`hw_hit`) runs
  across the surface only, from `margin` before it to `margin` past it (`margin` = 0.05 + 0.006 × distance),
  so the player's own body and the view model near the camera are never met. If its hit lies within
  0.03 + 0.004 × distance of the depth, the triangle's texel colour and face normal are taken (terrain, and
  the entities' quads, which `hw_hit` textures from the dynamic BLAS). Otherwise a second short ray
  (`guide_cover`, `hw_reflect`'s test) looks for water or glass at the depth. If neither (geometry outside
  the structures, the far field): the depth's normal (`guide_normal`) and the voxel grid's map colour.
- **Water and glass** (world depth only: they write depth, so the depth, and RR's depth input, is the
  transparent surface's). NVIDIA's RR guide (§3.4.1) says that without specular motion vectors the depth,
  normal and hit distance must agree on one surface, the opaque background or the transparent one, while
  the albedos and roughness may be blended toward the transparent surface where it dominates. So: the
  normal is the surface's flat face normal (no waves: the guide warns against noisy guides); with F the
  Schlick Fresnel of F0 0.02 (water) or 0.04 (glass) at N·V, the diffuse albedo is what lies behind (a
  `hw_hit` 64 blocks on along the unrefracted ray, times what else tints it) × (1 − F), the specular albedo
  the background's × (1 − F) plus the surface's mirror (`env_brdf_approx` of its F0 at roughness 0), and the
  roughness `mix(1, 0, F)`. The motion is the surface's (the camera's). Left: what lies behind is not
  refracted (the guide's advice is to refract the guides as the colour is, or default to the background as
  here); water's absorption is not in the diffuse albedo; ice and stained glass are glass.
- **What gets what.** Solid terrain: its triangle. Entities (world depth): their quad's texel and face.
  Sky: NVIDIA's defaults (diffuse 0.5, specular 0, normal 0, roughness 0), rotation-only motion. Lava: its
  atlas texel as albedo (its emission belongs in the colour). On the prepass (no RR), entities, water and
  glass are not in the depth and get the terrain behind them. Particles, the rain and anything that does
  not write depth get what lies behind them in every case.
- **Entity motion** (RR only). `dlss_eval` feeds frame generation's `ObjectMotion` every RR frame
  (`track_entities`: `Frame::entities` and `entity_motion`, matched by entity id and vertex layout) and
  draws it over the world's depth before the guides (`object_motion`: the jittered matrix to place the
  vertices, this frame's and last frame's clean matrices for the vector). The guides take that vector
  (UV × render size) wherever it is set, the camera's elsewhere. Entities posed on the GPU (`skin`) are
  not in `Frame::entities` and keep the camera's motion, as in frame generation.
- **Conventions.** Diffuse albedo Rgba8Unorm linear; specular albedo Rgba8Unorm (env BRDF); normals
  Rgba16Float world space, roughness in w; motion Rg16Float, render pixels, previous − current, x right,
  y down, without the jitter (`MV.Scale` 1, `MVJittered` 0: the Programming Guide §3.6.2 says NGX then uses
  them as they are, and §3.7.3 that the jitter offsets are given apart in any case).
- **Cost.** One short ray a render pixel (a window of a few centimetres to a block, on its own triangle):
  far cheaper than the kernel's primary-surface rays, about one extra "first hit" ray a pixel, plus four
  texture writes (13 B/px). At 3440x1440 that is 5 M short rays; to be measured on the 4090.
- **Looking at them.** `HORIZON_RT_GUIDES_SHOW=albedo|normal|roughness|motion|specular|source|depth` draws the
  window's guide over the whole frame (HUD included, so screenshots take it), stretched to the window:
  albedo and specular gamma-encoded, normals as `n × 0.5 + 0.5` (up is green, +x red, +z blue), roughness
  in grey, motion as red (right) and green (down) around grey 0.5 with 16 px a frame at full scale, and
  `source` green where the ray found the triangle, red where the depth's fallback was used, yellow on
  puddles and wet films, blue on water and glass, black for the sky, and `depth` the distance, white near
  to black far (grey at about 32 blocks, black sky). With RR on they show the guides RR was given (the
  world's depth); without, the prepass's. With HDR output the picture is drawn on the HDR swapchain too
  (at paper-white level or below, so it looks dim).

## 4. VR

- **One feature per view**, as the kernel and TAA keep one history per view (window = 0, eyes, quad-view
  insets: `shadow_rays.rs:12` `KEPT`). In VR that means two `DlssRr` on one `Ngx`.
- **Resolutions.** The eye is 4400x3520 (15.5 MP, 1.87 times 4K). In the sharp zone the rays already run at
  2x2 pixels a ray, and the VR prepass is at 2200x1760 (`half_prepass`). DLSS's published scales give these
  render sizes:

  | Mode | Scale | Render size |
  |---|---|---|
  | Quality | 1/1.5 | 2934x2347 |
  | Balanced | 0.58 | 2552x2042 |
  | Performance | 1/2 | 2200x1760 |

  `Quality::render_size` encodes these scales, and NGX's `DLSSDOptimalSettingsCallback` is the authority.
  **Performance matches the existing ray density**, so RR at Performance would cost no extra rays and the
  world raster pass could drop to 2200x1760.
- **Cost (estimates, to be measured).** I found no public table of RR execution times. NVIDIA's Super
  Resolution table puts the 4090 at roughly 0.5 to 1 ms at 4K output, and RR is several times heavier:
  community figures put it at 1.5 to 3 ms at 4K output on a 4090. That is an **estimate**. RR scales roughly
  with output pixels, so a full 4400x3520 eye would take about **3 to 6 ms, times two eyes**. That alone is
  most of the 11.1 ms budget, against today's 1.0 ms of denoiser + TAA per eye. **VRAM (estimate):** inputs
  at 2200x1760 are about 34 B/px (colour 8, albedos 4+4, normals 8, motion 4, depth 4, hit distance 2), about
  132 MB per eye, plus the 124 MB Rgba16Float output and NGX's internal history (unknown; measure with
  `DLSSDGetStatsCallback`). Plan on 0.3 to 0.6 GB for both eyes.
- **Run RR only in the sharp zone.** RR's model expects a uniform 1-spp signal. Two things break that:
  - **The periphery traces one ray a 4x4 block of texels** (`cs_sparse`, `shadow_rays.wgsl:2192`; scale in
    `shadow_rays.rs:17-30`). That is blocky, correlated structure, not noise. RR would keep it as detail or
    smear it.
  - **Eye 2 takes eye 1's light** where both see a surface (`from_first`, `:2095`; `rt.mode.z` 1 = eye 1's
    accumulated light, 2 = its denoised light, `taken_out` at `:395`). Mode 2 has no meaning under RR (nothing
    is denoised before RR). Mode 1, with eye 1's *raw* sample reprojected, gives eye 2 a fresh but
    eye-correlated sample every frame. RR may accept it; this has to be measured (A/B: eye 2 traces its own
    rays).

  The plan is to evaluate RR in a gaze-centred rectangle only:
  - Its inputs are written into zone-sized textures by a compose pass, so input subrects are not needed.
  - The output goes to a subrect (`DLSS.Enable.Output.Subrects`, `DLSS.Output.Subrect.Base.X/Y`) of the eye
    image.
  - The periphery keeps today's accumulation, denoiser and TAA, with a feathered seam.

  The zone follows the gaze, so the zone's motion vectors must include the zone's own move since last frame
  (`mv += offset_now - offset_last`). With the 25° sharp half-angle (`SHARP_DEGREES`) the zone is roughly
  0.35 to 0.45 of the eye's tangent extent on each side, so 12 to 20 % of the pixels: about **0.5 to 1.2 ms
  an eye (estimate)**.
- The desktop window has no foveation (`Foveation::screen`), so it is the first place to try RR on the whole
  frame.

## 5. Distribution and licence

- **Nothing NVIDIA's is committed.** That covers no headers, no `.lib` and no DLL. `tools/check_public_tree.py`
  and the hook guard Mojang's files, and now also any file named `nvsdk_ngx*`, `nvngx*` or
  `libnvidia-ngx*` (`PRIVATE_PREFIXES`).
- **Dev machine**: clone the SDK (github.com/NVIDIA/DLSS, release 310.x) outside the repo, set
  `DLSS_SDK_DIR`, and build the game with `--features rtx,dlss-rr` (§8).
- **Runtime**: `nvngx_dlssd.dll` from the SDK's `lib/Windows_x86_64/rel/` folder goes beside
  `horizon-client.exe`, or into a folder that `DLSS_RUNTIME_DIR` names. (This doc first said
  `HORIZON_DLSS_DIR`; the code reads `DLSS_RUNTIME_DIR`.) The renderer passes both, in that order, as
  `FeatureCommonInfo::PathListInfo` (`dlss_rr::Paths`), and logs each folder and whether the DLL is in it. The
  SDK's `dev/` DLL draws a debug overlay (jitter and MV checks). The `rel`/`dev` folder names come from
  dlss_wgpu's code (`lib/<platform>/{rel,dev}`), not from a listing of the SDK. NGX's own logs go to the game
  home's `logs/ngx` (the data path).
- **Licence (brief)**: the DLSS SDK licence lets the runtime DLLs ship with the application, and requires the
  copyright and licence text from the programming guide (section 9.5 per dlss_wgpu's README) in the product's
  notices (THIRD_PARTY_NOTICES.md). Neither the SDK nor its headers may be redistributed in source form here.
  It forbids reverse engineering. NVIDIA asks for a project or application id. Read the SDK's `LICENSE.txt`
  before any release; this summary is not legal advice.
- **Non-NVIDIA GPU, old driver, missing DLL, init failure**: each step returns `Err`. These are
  `required_device_extensions` (`GetFeatureRequirements`), `Ngx::init` (`SuperSamplingDenoising.Available`)
  and `DlssRr::new`. The renderer logs the reason once and keeps the current accumulation, denoiser and TAA.
  No extension is enabled that the GPU did not offer.

## 6. Staged plan and A/B

The switch is `HORIZON_RT_DLSS_RR=1` (`dlss_rr::wanted()`). It is off by default, and any failure falls back
automatically.

| Stage | Change (files) | A/B measurement |
|---|---|---|
| 0 (this) | `horizon-rtx/src/dlss_rr.rs`, `build.rs`, the `dlss-rr` feature | Builds; nothing changes at runtime |
| 1. Extensions + init (**done, desktop**; VR left) | `horizon-hdr/src/lib.rs` (`open_vulkan_device_with`), `gfx.rs` (`open_backend`: instance and device; `Renderer::new` and `resize`: `dlss_rr::Session`), `dlss_rr.rs` (`create_instance`, `device_extensions`, `Paths`, `Session`, `Ngx::optimal_settings`), the `dlss-rr` feature in `horizon-client`/`horizon-client-render`. VR (`horizon-xr/src/lib.rs`, both lists) is still to do | Startup time, frame time unchanged with RR wanted but unused; validation layers clean apart from NGX's known ones |
| 2. Motion vectors | **Done** in the guides pass (§3.1): the camera's, and on RR's frames the entities' own (`ObjectMotion` on the world's depth); left: GPU-posed entities, block animation, particles | Its cost (expected below 0.1 ms); a debug view (`HORIZON_RT_DEBUG`) of the vectors on moving mobs |
| 3. G-buffer | **Done without MRTs**: the guides pass (§3.1) traces them, on RR's frames from the world's jittered depth (entities, water and glass included). Left: LabPBR normals, roughness and F0, refraction of what lies behind water and glass, particles | Raster cost of the extra attachments, per eye |
| 4. Raw kernel mode (**done, desktop**, §9) | `shadow_rays.wgsl` `raw_frame` (`rt.mode.w` bit 8): no `accumulate`, no rest in turns; `denoise.wgsl` one pass-through (`params.step.w` 1); `ShadowRays::set_raw_view`; horizon-rtx `SunPass::set_never_rest` (the bounce pass agrees) | Trace time without accumulation; the noisy image itself (`HORIZON_DLSS_RR_SHOW=input`) |
| 5. RR at DLAA, desktop (**done, unrun**, §9) | `gfx/dlss_eval.rs`; `Session::evaluate` and `DlssRr::record` (horizon-rtx); `gfx.rs` `rr_now`, in place of `taa.resolve`, through `split_commands`; reset on the first frame, a resize, a cut or a frame without RR | Desktop 3440x1440: RR ms against accumulate + denoise + TAA (0.55 + 0.45 + accumulation); stills and motion captures against today's |
| 6. RR upscaling, desktop | World pass and rays at `Quality::render_size`; hand and HUD unchanged after it | ms and quality at Quality/Balanced/Performance against the current RT scale 2 path |
| 7. VR sharp zone | Zone-sized compose, output subrect, per-eye features, zone-move motion | Eye frame ms (today 5.97), the seam; eye 2 sharing on/off |
| 8. VR whole eye (only if 7 leaves budget) | Uniform rays in the periphery at lower density instead of 4x4 sharing | Both eyes against 11.1 ms |

## 7. What was checked against NVIDIA's public headers

**Checked** (the SDK's `include/` on github.com/NVIDIA/DLSS, main branch, read on 9 November 2026), as
interface facts only, with no header text copied:

- **Function names, parameter types and order**: `NVSDK_NGX_VULKAN_Init`, `_Init_with_ProjectID`, `_Shutdown1`,
  `_GetCapabilityParameters`, `_AllocateParameters`, `_DestroyParameters`, `_GetScratchBufferSize`,
  `_CreateFeature1`, `_ReleaseFeature`, `_EvaluateFeature_C`, `_GetFeatureRequirements`,
  `_GetFeatureInstanceExtensionRequirements` and `_GetFeatureDeviceExtensionRequirements`.
- **Parameter functions**: `NVSDK_NGX_Parameter_Set{I,UI,F,ULL,VoidPointer}` / `Get{I,UI,VoidPointer}`. They are
  exported functions, not inline.
- **Calling convention**: `NVSDK_CONV` is `__cdecl` on MSVC and empty elsewhere.
- **Struct fields**: `NVSDK_NGX_Resource_VK` (union, `Type`, `ReadWrite`), `ImageViewInfo_VK`, `BufferInfo_VK`,
  `FeatureCommonInfo`, `PathListInfo`, `LoggingInfo`, `FeatureDiscoveryInfo`, `Application_Identifier`,
  `ProjectIdDescription` and `FeatureRequirement`.
- **Enum values**: `Result_Success` 0x1, the failure mask, `Version_API` 0x15, feature 13, `PerfQuality`, the
  DLSS feature flags, `EngineType`, the denoise, roughness and depth modes, and `Resource_VK_Type`.
- **Parameter-key strings** for every key `dlss_rr::ffi::names` lists. The mapping of each
  `NVSDK_NGX_VK_DLSSD_Eval_Params` field to its key and setter (normals → `GBuffer.Normals`, roughness →
  `GBuffer.Roughness`, specular hit distance → `DLSSD.SpecularHitDistance`, specular motion vectors →
  `GBuffer.SpecularMvec`, matrices as `float*` through `SetVoidPointer`, zero scale or exposure treated as 1),
  and of the create helper (denoise mode set to DLUnified).
- **Library and DLL names and folders**: from dlss_wgpu's build script and README.
- **Stage 1, read on 10 November 2026** (main branch headers and dlss_wgpu's `src/`): the
  `DLSSDOptimalSettingsCallback` helper (keys in and out, `RTXValue` = 0, `FAIL_OutOfDate` = 0xBAD0000C when
  the pointer is missing, the callback type taking the capability parameters); the `DLSS.Get.Dynamic.*` key
  strings; that `CreateFeature1`'s command buffer must be "open and recording"; that the two extension queries
  need no `NVSDK_NGX_Init` and go before the instance and the device are made; dlss_wgpu's init (both loader
  functions passed), its instance path (`init_with_callback` + `from_hal`) and its drop order (wait idle,
  `DestroyParameters`, `Shutdown1`).

- **Stage 5, read on 10 November 2026** (main branch headers and dlss_wgpu's `src/context.rs`,
  `render_parameters.rs`): every key `NGX_VULKAN_EVALUATE_DLSSD_EXT` sets that Horizon feeds, including
  the input sub-rectangle bases (all 0), `TonemapperType` (0, the string tone mapper) and the
  `DLSS.Indicator.Invert.*` axes, now set as the helper does; which inputs the helper's struct lists as
  required (albedos, normals, roughness, colour, output, depth, motion, jitter, render subrect) and that
  the two matrices are among its optional "research" fields; `DLSS.Roughness.Mode` packed = roughness in
  `normals.w`; dlss_wgpu's evaluation: `transition_resources` to `RESOURCE` for inputs and
  `STORAGE_READ_WRITE` for the output, the image views' whole subresource, the `texture_format_as_raw`
  format, then the evaluation on the encoder's raw command buffer.

- **The guides, read on 10 November 2026** (main branch: `include/nvsdk_ngx_helpers_dlssd_vk.h`,
  `doc/DLSS-RR Integration Guide.pdf`, `doc/DLSS_Programming_Guide_Release.pdf`): motion vectors in render
  pixels toward last frame's position, and with `MVJittered` 0 NGX uses them without taking any jitter out
  (Programming Guide §3.6.1, §3.6.2); the jitter offsets are in render pixels, within ±0.5, given apart
  whether or not the vectors hold them (§3.7.3; the helper's comment: "render pixel space"); Halton is what
  DLSS was trained on and RR wants at least 32 phases (RR guide §3.6; Programming Guide §3.7.1); the sky's
  defaults (diffuse albedo 0.5, specular albedo, normals and roughness 0; RR guide §3.4.1 to §3.4.4); the
  blending of albedos and roughness toward a dominating transparent surface with depth, normal and hit
  distance kept on one surface (§3.4.1); defaulting refracted motion to the background's as "sensible, but
  imperfect", and constant guides rather than noisy ones on refracting surfaces (§3.5); the transparency
  layer and colour-before-transparency inputs as NGX's route for particles (§3.4.10, §3.4.11; not used).

**Verified on the RTX 4090** (stage 1, SDK v310.9.16, reported 10 November 2026): NGX asked for and got the
device extensions `VK_NVX_binary_import`, `VK_NVX_image_view_handle`, `VK_KHR_buffer_device_address` and
`VK_KHR_push_descriptor`; NGX initialised with the self-chosen project id; `SuperSamplingDenoising.Available`
= 1; the feature instance was made at 2580x1080 (DLAA), made again on resize, released, and NGX shut down
without a crash; the Windows link needed nothing beyond `nvsdk_ngx_d.lib`; the `rel` DLL folder name held.

**Unverified:**

- whether NGX needs Vulkan device *features* besides its extensions (the header names none; wgpu enables
  buffer device address with ray queries; stage 1 ran without asking for more);
- that `Device::poll(wait)` is enough before `ReleaseFeature`/`Shutdown1` once frames are evaluated
  (dlss_wgpu calls `vkDeviceWaitIdle`);
- the instance extension list (the device list is verified above);
- NGX's layout and barrier contract on Vulkan (`DlssRr::record` adds a full memory barrier on each side);
- the jitter sign (`HORIZON_DLSS_RR_JITTER_SIGN`, §9);
- that a self-chosen UUID is accepted as a project id outside development (accepted in development);
- the licence wording (read `LICENSE.txt`);
- every performance and VRAM figure in §4.

The struct layouts are pinned by a test (`dlss_rr::tests`), which runs only with the feature on. On Linux it runs
without the SDK (`HORIZON_DLSS_RR_NO_LINK=1 cargo test -p horizon-rtx --features dlss-rr`): the linker drops the
unused NGX symbols.

## 8. Stage 1 on the RTX 4090 PC (tester's steps)

Windows, Vulkan, a recent NVIDIA driver. Nothing NVIDIA's goes into the repository: keep the SDK outside it.

1. Get the SDK at a fixed release, outside the repo:
   `git clone --depth 1 --branch v310.9.16 https://github.com/NVIDIA/DLSS C:\sdk\DLSS`
   (v310.9.16 is the newest tag on 10 November 2026; the declarations were checked against `main`, `Version_API`
   0x15).
2. In the shell that builds: `set DLSS_SDK_DIR=C:\sdk\DLSS` (PowerShell: `$env:DLSS_SDK_DIR="C:\sdk\DLSS"`).
   The build links `C:\sdk\DLSS\lib\Windows_x86_64\x64\nvsdk_ngx_d.lib` (the dynamic-CRT library, which Rust's
   default `x86_64-pc-windows-msvc` target needs; `nvsdk_ngx_s.lib` only with `+crt-static`). A wrong folder fails
   the build with a message naming the file it looked for. The static library is C++; its MSVC runtime comes
   in through the library's own default-library directives (dlss_wgpu links nothing more on Windows). If the
   link still reports unresolved Windows symbols, that list is what to send back (**unverified**: no Windows
   link was run for this stage).
3. Build: `cargo build --profile fast -p horizon-client --features rtx,dlss-rr`
4. Copy `C:\sdk\DLSS\lib\Windows_x86_64\rel\nvngx_dlssd.dll` beside `target\fast\horizon-client.exe`, or
   leave it there and `set DLSS_RUNTIME_DIR=C:\sdk\DLSS\lib\Windows_x86_64\rel`. (`dev\nvngx_dlssd.dll` draws
   NVIDIA's debug overlay once evaluation is wired.)
5. Run with the switch, from a scratch game folder (CLAUDE.md's rules): `set HORIZON_RT_DLSS_RR=1`, optionally
   `set HORIZON_DLSS_RR_LOG=1` (NGX's own messages at info level; `2` verbose), then start
   `target\fast\horizon-client.exe --resources <...>\.local\horizon-client\26.3 --game-dir <scratch>`.
   The Graphics API must be Vulkan (Auto picks it on an x64 PC).

**Expected log lines** (all start with `DLSS-RR:`; names and sizes are examples):

```
DLSS-RR: NGX searches C:\...\target\fast for the runtime: nvngx_dlssd.dll found
DLSS-RR: DLSS_RUNTIME_DIR is not set: only the executable's folder is searched
DLSS-RR: NGX's data path (its own logs): C:\...\logs\ngx
DLSS-RR: NGX asks for N instance extension(s): VK_KHR_...
DLSS-RR: wgpu's Vulkan instance already enables all of them
   (or: the Vulkan instance is made with VK_... added (...))
DLSS-RR: Ray Reconstruction is supported on this GPU and driver (NVSDK_NGX_VULKAN_GetFeatureRequirements); NGX asks for N device extension(s): VK_NVX_binary_import, VK_NVX_image_view_handle, ...
DLSS-RR: instance extensions NGX needs: enabled [...], not enabled [none]
DLSS-RR: device extensions NGX needs: enabled [...], not enabled [none]
DLSS-RR: NGX initialised (project id, custom engine); Ray Reconstruction is available (SuperSamplingDenoising.Available = 1)
DLSS-RR: NGX's render sizes for a 3440x1440 output: DLAA 3440x1440, Quality 2293x960, Balanced ..., Performance 1720x720, Ultra Performance ...
DLSS-RR: feature instance created for 3440x1440 (DLAA: render size = output size; nothing is evaluated yet)
```

Resizing the window logs `feature instance for WxH released` then `feature instance created for W'xH'`.
Closing the game logs the last `released`, then `DLSS-RR: NGX shut down`.

**Failures** each log one `DLSS-RR: off: ...` (or `off on this GPU`) line naming the call and NGX's result code,
and the game runs exactly as without the switch: the device is made without NGX's extensions when the
requirement query fails, and RT lighting keeps its accumulation, denoiser and TAA. The usual ones: no
`nvngx_dlssd.dll` in a searched folder (`Ray Reconstruction is not available (SuperSamplingDenoising.Available
= 0; ...)`, with `SuperSamplingDenoising.FeatureInitResult` as the code), an old driver (`needs a newer driver`,
with the version NGX asks for), DX12 or OpenGL chosen (`needs Vulkan`), VR (`not wired for VR yet`).

Please send back the whole `DLSS-RR:` block and, if anything failed, the newest file in `logs\ngx`.

## 9. Stages 4 and 5: RR evaluated on the desktop window (DLAA)

**What is wired** (all behind the `dlss-rr` feature and `HORIZON_RT_DLSS_RR=1`; desktop window only, render
size = output size = the world's size, which a render scale can make differ from the window's):

- **Raw kernel mode (stage 4).** `ShadowRays::set_raw_view(Some(0))` (`gfx/shadow_rays.rs`) sets bit 8 of
  `rt.mode.w` for the window's view. The kernel (`shadow_rays.wgsl` `raw_frame`) then skips `accumulate`
  (only the moving caster's mark is taken off the face, as `accumulate` does) and never rests the bounced
  light in turns (`trace`'s `rest`); ReSTIR DI/GI's temporal reuse stays, as the design says. The RTX 4090
  bounce pass is told the same through `SunPass::set_never_rest` (its flag bit 4, the kernel's `SKIP` 16),
  so the two agree on which texels rest. The à-trous denoiser is replaced by one pass that only adds the
  direct light (`denoise.wgsl`, `params.step.w` 1, a buffer made on first use). Every other view keeps
  bit 8 clear and the five passes; without the switch no bit, buffer or pass changes, so every shader's
  output is as before.
- **Jitter.** On RR frames the world is drawn with `dlss_eval`'s own Halton(2, 3) jitter (32 phases,
  `taa::jitter_of`), advanced once a frame evaluated; TAA's history is freed. The RT depth prepass stays
  on the clean matrix, but RR's guides are made from the world's own jittered depth with the same jitter
  (§3.1), so every guide is on the colour's grid; the motion vectors have the jitter taken out
  (`MVJittered` 0). The hand is drawn after the evaluation (not in the world pass), as before.
- **Guides.** Just before the evaluation: frame generation's `ObjectMotion` draws the entities' own
  motion over the world's depth (`DlssEval::object_motion`), then `ShadowRays::run_guides` runs the guides
  pass on that depth with the kernel's bind group of this frame. A frame where the kernel did not run
  for the window makes no guides and falls back to TAA.
- **Evaluation (stage 5).** Where `taa.resolve` ran (`gfx.rs`, `rr_done`), `DlssEval::evaluate`
  (`gfx/dlss_eval.rs`) makes the feature follow the world's size (`Session::resize`), then
  `Session::evaluate` (horizon-rtx `dlss_rr.rs`) takes the raw handles of: the scene (noisy colour, linear
  HDR, `IsHDR`), the world's depth (reversed Z, `DepthInverted`, hardware depth), and the guides' motion,
  diffuse albedo, specular albedo and normals with roughness in w (`DLSS.Roughness.Mode` packed). Exposure:
  NGX's `AutoExposure` (no exposure texture), pre-exposure 1. Reset: the first evaluation of a feature, a
  resize, a camera move of more than 8 blocks in a frame, any frame without RR's evaluation before this
  one. `DlssRr::record` transitions the inputs to `RESOURCE` and the output to `STORAGE_READ_WRITE`, ends
  the frame's encoder into `split_commands`, records `EvaluateFeature` between two full memory barriers in
  a command buffer of its own, and the frame carries on in a new encoder. The output (Rgba16Float storage
  image) is drawn back over the scene (`wgpu::util::TextureBlitter`); the rain, haze, bloom, tone mapping,
  hand and HUD follow unchanged. It also stands in for TAA's last frame for the screen-space reflections.
- **Fallbacks.** A frame whose rays did not run, or whose guides are not the world's size, is resolved by
  TAA instead (and RR's history resets). Any `Err` (NGX's evaluation, a missing or mis-sized image, a
  non-Vulkan texture) logs one `DLSS-RR: off: the evaluation failed (...)` line, drops the session (feature
  released, NGX shut down) and the rest of the run is the normal path (that frame resolves through TAA).

**Debug switches:**

- `HORIZON_DLSS_RR_RAW=1`: the kernel traces the window raw (no accumulation, the bounce never resting), as NVIDIA's guide asks. Measured on the RTX 4090 at 1720x720, still camera: 28.6 ms of GPU a frame against 10.4 without RR, for about 3 ms of stamped RR passes: the bounce traced at every texel every frame is the rest. So by default the kernel accumulates and rests in turns as for every other view, and RR only takes the à-trous denoiser's and the TAA's place; whether its image holds up on that accumulated input is to be judged on the GPU.
- `HORIZON_DLSS_RR_SHOW=input`: RR still runs, but its output is not drawn back: the screen shows the raw
  noisy input RR was given. Noise there is expected; wrong colours, black areas or smearing there mean the
  input is at fault, not RR.
- `HORIZON_DLSS_RR_JITTER=0`: no jitter (and RR told 0): if the image calms, the jitter is the suspect.
- `HORIZON_DLSS_RR_JITTER_SIGN=x`, `y` or `xy`: the jitter NGX is told is negated on those axes (Horizon's
  is how far the image moves, right and down). Try each with a still camera: the right one does not swim.
- `HORIZON_RT_GUIDES_SHOW=motion|normal|albedo|roughness|specular|source|depth` (§3.1) pictures the
  guides RR was given.
- NVIDIA's `dev` DLL (`lib\Windows_x86_64\dev\nvngx_dlssd.dll` in place of the `rel` one) draws its overlay
  with the jitter and motion vector checks.

### Tester's steps (RTX 4090, Windows, Vulkan)

1. Build as in §8 (steps 1 to 4): `cargo build --profile fast -p horizon-client --features rtx,dlss-rr`
   with `DLSS_SDK_DIR` set, `nvngx_dlssd.dll` beside the executable or in `DLSS_RUNTIME_DIR`.
2. From a scratch game folder, with RT lighting on in a world: `set HORIZON_RT_DLSS_RR=1`, then start the
   game as in §8 step 5. Walk around, look at shadows, torch light in a cave, water and glass.
3. Expected log lines, after §8's stage 1 lines (sizes are examples):

   ```
   DLSS-RR: feature instance created for 2580x1080 (DLAA: render size = output size)
   DLSS-RR: first evaluation recorded at 2580x1080 (DLAA, reset on, jitter 0.000, -0.167); RT lighting's accumulation, denoiser and TAA are skipped for the window
   ```

   With a render scale, the feature is made again at the world's size first (`feature instance for WxH
   released`, then `created for W'xH'`). On a failure: `DLSS-RR: off: the evaluation failed
   (NVSDK_NGX_VULKAN_EvaluateFeature_C failed (NGX result 0xbad000..)); back to RT lighting's accumulation,
   denoiser and TAA for the rest of the run`.
4. A/B: the same place with and without `HORIZON_RT_DLSS_RR=1`; then `HORIZON_DLSS_RR_SHOW=input` to see
   the raw input; with `HORIZON_GPU_PASSES=1` the pass timings show `DLSS Ray Reconstruction` and a single
   `rays: denoiser` pass on RR frames.
5. The guides, with RR on (`HORIZON_RT_DLSS_RR=1` plus `HORIZON_RT_GUIDES_SHOW=<view>`), one screenshot
   each in a scene with a mob walking, a pond or the sea, a glass window and terrain in the distance:
   - `depth`: the mob, the water's surface and the glass appear in it (they did not before); no hand.
   - `source`: terrain and the mob green, water and glass blue, red only far away or at odd geometry,
     black sky. A mob in red means its quads are not in the hardware structure.
   - `albedo`: the mob in its own colours (not the ground behind it); under water and glass the bottom
     or the room behind, a little darker at grazing angles; the sky mid grey.
   - `normal`: sharp per-face colours on the mob; water flat (green, no waves); the sky mid grey (0).
   - `roughness`: white terrain and mobs, darker on water and glass toward grazing angles, black sky.
   - `motion`: grey everywhere with a still camera, and the walking mob alone tinted (it was grey
     before). The vector points back to where the pixel was: a mob walking left on screen is redder
     than grey, one walking right less red (bluish green); walking toward the bottom of the screen less
     green, toward the top more. Turning the camera tints the whole frame smoothly.
   - `specular`: dark grey on terrain, brighter on water and glass toward grazing angles.
   With a still camera none of them should shimmer at edges more than the scene does (they follow the
   jitter now); compare one with `HORIZON_DLSS_RR_JITTER=0`, which should look the same but steady.
6. Please send back: the `DLSS-RR:` block, screenshots with and without RR (and with `SHOW=input`), the
   guide screenshots of step 5, whether a still camera swims (and which `JITTER_SIGN` fixes it), whether a
   walking mob ghosts or smears with RR (and whether water and glass smear), any Vulkan validation messages
   besides NGX's known ones, and the newest file in `logs\ngx` if anything failed.

### Unverified (nothing of stages 4 and 5 has run: the machine that wrote them has no GPU, no SDK and no
Windows linker; only `cargo check`, the unit tests and naga's validation of the shaders ran)

- that NGX accepts the inputs at all (formats Rgba8Unorm albedos, Rgba16Float normals, Rg16Float motion,
  Depth32Float depth, Rgba16Float colour and output) and the image views wgpu made;
- the barrier and layout contract around `EvaluateFeature` (the full barriers are a guess at safety, not a
  measured need), and that the wgpu tracker stays right after it (validation layers will tell);
- the jitter sign, and whether 16 phases suit RR at DLAA;
- that the guides now sit on the colour's jittered grid (`guide_centre`'s sign: a wrong sign would make
  the `SHOW` views shimmer at edges with a still camera, steady with `HORIZON_DLSS_RR_JITTER=0`);
- RR's look with entities, water and glass on their new guides, and with particles (no guides of their
  own); that the entities' quads in the hardware structure match the drawn ones closely enough for the
  short ray (else `source` shows them red);
- that `ObjectMotion`'s vectors (made for frame generation) are right for RR: sign, the jitter, and the
  previous matrix `dlss_eval` builds from last frame's clean one;
- the cost of the second ray on water and glass pixels and of the entity motion pass;
- that `AutoExposure` is the right choice against feeding last frame's exposure;
- the cost (RR ms against 0.55 + 0.45 ms + the accumulation) and the VRAM;
- that the raw kernel mode leaves nothing temporal behind besides ReSTIR's reuse (the radiance cache
  still averages over frames, by design);
- that dropping the session after a failed evaluation mid-frame is safe (it waits for the device first).
