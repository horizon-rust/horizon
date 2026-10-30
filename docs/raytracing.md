# Ray tracing ("RTX"): where it stands and the plan

Goal: a hybrid renderer, rays where they beat raster (sun and moon shadows, AO, reflections on smooth surfaces,
GI last) and classic techniques elsewhere, working with LabPBR materials
([resource-packs.md](resource-packs.md#labpbr-maps)), HDR output ([graphics.md](graphics.md)) and VR's foveated
rendering ([vr.md](vr.md)), on a whole world, not just the few chunks around the player. **Every ray effect works
on the screen and in VR through one code path.** The base for all of it, decided before the rays: **the scene
drawn in linear HDR light** (step 2), with a tone mapping per output.

## Step 1, done: a tilted sun path

Vanilla's sun crosses the zenith, so noon shadows fall straight down and show nothing. **Sun Path Tilt**
(Advanced Graphics → Quality, 40° by default, 0° = vanilla; [graphics.md](graphics.md)) leans the path south as shader
packs' `sunPathRotation` does. One function gives the light direction (`crates/horizon-client/src/celestial.rs`,
`celestial::direction(angle, frame.sun_path_tilt)`): the sky's sun, moon and stars, `frame.sun_direction` for
Horizon packs, Iris packs' `sunPosition` / `shadowLightPosition` and shadow map. The ray-traced shadows below trace toward
that same direction. Iris's sign was checked against its `CelestialUniforms` (`Y -90, Z sunPathRotation,
X angle`): a pack's -40 puts the noon sun in the south; Horizon used to mirror it to the north.

HDR: the HDR pass lifts the sun and moon by the mark their celestial pass leaves in the frame's alpha, wherever they
are, so the tilted sun shines the same (by construction; not yet looked at on an HDR display, the captures are
offscreen SDR).

## Step 2, started: the scene in linear HDR light

**Done (Linear Light, [graphics.md](graphics.md))**: the float scene per view (window and VR eyes), shaders
writing linear light with sources above 1.0, the hand and screen effects in the scene, the SDR tone mapping
(identity below 0.9, shoulder above), vanilla's frame untouched when it is off; HDR output from the scene (PQ
or scRGB, the shoulder at the display's peak, the HUD at paper white where it drew). **Next in this step**:
glowing particles and entities' emissive parts.

Today the world is drawn in SDR, in vanilla's gamma space, into an 8-bit target of the window's format, and
`hdr.wgsl` guesses the light sources afterwards (the emission mark in the alpha, `shows_world` to leave out the hand,
the HUD and the vignette) and lifts them. That is a patch, not light: glowing particles and far LOD terrain are not
lifted, a pixel the vignette darkens loses its lift (the fade on `fix/hdr-vignette-lift` softens the edge), and
nothing above 1.0 exists for the PBR or the shadows to work with. The plan:

1. **A scene-referred linear target** (`Rgba16Float`, one per view; the eyes' too). Sky, terrain, entities,
   particles, clouds and LOD tiles write linear radiance in multiples of paper white; sources go above 1.0: the sun
   and moon discs, lava, emissive blocks (the mesher's `EMISSION_BITS`), LabPBR emission (`_s` alpha), glowing
   particles. The PBR sun term, the RT shadows and the sky are all computed here.
2. **A tone mapping per output**, one last pass: **SDR** (the window in SDR, and the Vision Pro through KRVR,
   which is SDR only) maps with a real curve (a shoulder toward white, hue kept), not a clamp at 1.0; **HDR10 PQ**
   or **scRGB** (an HDR desktop) map paper white and the peak from the existing Paper White and Peak Brightness
   options, rolling off toward the peak. HDR metadata as `horizon-hdr` sends it now.
3. **Screen-wide effects in linear, before the tone mapping**: vignette, damage and nausea tints, the underwater
   and powder-snow overlays, the night-vision and darkness effects. **The hand, the HUD and text are composed after
   it, at paper white**, so they are never tone mapped or lifted: `shows_world` and the alpha mark go away.
4. **A vanilla mode that stays exact**: with HDR off, no shader pack and no PBR, the frame is drawn as it is today
   (8-bit, vanilla's gamma-space blending, which a linear target would change: vanilla blends translucency and fog
   in gamma space), so 26.3's look and the screenshot parities keep holding pixel for pixel. Linear light is what
   HDR output, LabPBR lighting and RT shadows turn on; the tone mapping of the SDR window is chosen so that a scene
   with no source above 1.0 maps to itself.
5. **Shader packs**: Horizon packs' `post` stage gets the linear scene (its `scene_color` documented as such, a new
   API version); Iris packs keep their own targets and gamma (they tone map themselves), composed as SDR.

Cost, by bandwidth (one target read or written once = one pass; the scene is about 4 passes a frame: clear,
drawing with its overdraw and blending, the tone mapping's read):

| | Pixels | 8-bit target, a pass | `Rgba16Float`, a pass | Extra a frame (x4) | At 1 TB/s (RTX 4090) |
|---|---|---|---|---|---|
| Desktop 3440x1440 | 4.95 M | 20 MB | 40 MB | 80 MB | +0.08 ms (+11 GB/s at 144 Hz) |
| VR stereo 4096x3288 an eye | 26.9 M | 108 MB | 215 MB | 430 MB | +0.43 ms (+39 GB/s at 90 Hz) |

Memory: one more `Rgba16Float` per view, 40 MB at 3440x1440 and 215 MB for the two 4096x3288 eyes. In VR the GPU
works 2.5 ms of an 11 ms frame (vr.md), so the 0.4 ms fits; the quad views have 9.2 M pixels, a third of that.
To measure, not assume: GPU timestamps of the tone-mapping pass and of the frame with HDR off and on, at 3440x1440
and on the fake VR runtime (stereo and quad views). Where VR stands: the quad views draw no shader pack yet, and
**shader packs don't draw in the VR eyes at all** (a pack draws through the world post target, which the eyes
skip; vr.md, "Not there yet"); the linear target per view is the place to fix both, since the eyes and the window
would then share one scene path.

## Step 3, started: ray-traced sun and moon shadows (approach A)

**Done**, the voxel grid traced in compute, approach A below, with no hardware ray tracing and no experimental API:

- `crates/horizon-client/src/occupancy.rs`: each section's blocks to the rays (solid, cutout, nothing), two bits a
  block, computed by the mesher with the mesh (uniform sections cost nothing).
- `gfx/voxels.rs`: the grid around the camera, 16x16 chunks and the world's height, toroidal: 512-bit masks per
  8x8x8 brick (solid, cutout) plus brick and 32-block brick occupancy, about 6 MB. A camera chunk border writes
  only the columns that came in; a block change rewrites its section's 1 KB.
- `gfx/shadow_rays.rs`: a ray per pixel of the view's depth, after the world pass; the solid terrain writes the
  share of its light it keeps in the shadow into the scene's alpha, the tone mapping (SDR and HDR) applies the mask.
  The same pass for the window and each VR eye, with the foveation the views carry: the screen sharp all over,
  VR 2x2 pixels a ray in the 25-degree sharp zone around the gaze and 8x8 beyond (workgroup-shared rays).
- Measured (RTX 4090): +1.2 ms GPU at 3440x1440, morning sun, noise at noon; fake VR, two 4096x3288 eyes, about
  +0.3 ms GPU and +0.3 ms CPU (grid writes). Inline rays in the terrain shader cost 2-5x that (every fragment drawn
  traced, overdraw included): the pass on the depth is what made it fit.

**Also done**: ambient occlusion from the same pass and grid (six fixed rays two blocks long a pixel, the ambient
share only), +1-1.4 ms GPU at 3440x1440.

**Direction**: the rays replace vanilla's shading rather than adding to it. Today they scale vanilla's lit
colour (vertex AO, face shading and the lightmap stay baked in). Next is an RT lighting mode where the terrain
writes only its albedo (no vertex AO, no face shading baked into its vertex colour) and the light comes from the
rays: the sun with its shadows, the sky through the AO rays (sky visibility), block light from the light levels
until rays or a probe cache take it over (GI). The RT AO then shades corners alone, which is why it already
counts the blocks right beside a surface.

**The target** (asked for): all of the lighting redone, nothing kept from vanilla's:

1. RT lighting from the albedo: the sun with its shadows, the sky seen through rays, block light with the
   AO (done, `rtLighting`).
   - With Parallax and a LabPBR pack the same model lights the material: the normal map's normal, the
     map's AO, from `_s` the roughness (GGX highlight, Smith visibility, Schlick Fresnel), the reflectance
     or metal, the subsurface scattering (leaves, plants lit from behind) and the emission (done).
2. Shadows from every block as it is shaped (done in the voxel grid, static): each state's model voxelized at
   8x8x8 from its faces' opaque texels (iron bars' gaps, leaves' holes, plants' stems), its outline for blocks
   drawn without faces; up to 1024 shapes, an id beside the block's colour in the grid's `R16Uint` texels,
   rays crossing a shaped block cell by cell (leaves and plants let 35 % through a cell). It stays for the
   diffuse light (sky, GI, bounces) and for GPUs without hardware rays.
   - The hybrid (done for the terrain, `gfx/hw_rays.rs`): where the GPU traces rays (wgpu's experimental
     `EXPERIMENTAL_RAY_QUERY`, Vulkan; `HORIZON_HW_RT=0` turns it off), the sun's and the light sources'
     shadow rays meet the drawn triangles themselves. Each section within the grid's reach gets a
     bottom-level structure over its solid quads where they lie in the vertex arena (no copy, the shared
     quad indices); a top-level one, made each frame, holds them relative to the camera. Every candidate
     hit is alpha-tested in the shader against the quad's sprite at the hit's texture coordinate, so
     iron bars, fences, leaves and flowers shadow exactly as drawn. Measured at 3440x1440: 3.0 to 3.7 ms
     GPU with RT lighting against 3.9 with the voxel grid's shadow rays. `examples/rt_query_probe.rs`
     checks a machine's hardware rays. Entities, the view model and the cloud tiles are in it too (see the
     table). Next: VR's device, the arena's later pages in the alpha test.
   - Translucent quads (stained glass, water, ice) are a second geometry of each section's structure:
     the shadow rays cross them and take their colour (texel times tint, as much as it is opaque), so
     stained glass colours the sunlight and the light sources' light; water gives caustics instead.
8. Denoising (done): the sky's and the bounced light, accumulated over frames, then an edge-stopping
   a-trous filter (`gfx/denoise.wgsl`: 5x5 taps, steps 1, 2, 4, 8, weights stopped at distance jumps
   and changes of face). The light sources' direct light is kept apart, accumulated but not blurred.
9. Light sources are areas (done): a full block emitting light its whole volume, a torch's flame a
   tenth of a block above its floor; a point of it a frame, accumulated: soft shadows whose
   penumbra widens with distance. The falloff is from that point (the block under a torch is no
   longer lit as if touching the light). Each 8-block cell keeps its eight strongest lights, four
   drawn a frame and weighted, and only lights touching air count (a lava lake lights its shores,
   not from under its surface). A ray that spends its budget in a long cave is taken as stopped: no
   sky leaks into caves.
10. The real night sky (done, `gfx/sky_stars.rs`): the Yale Bright Star Catalogue's 9096 stars
   (public domain, by way of Bretton Wade's MIT-licensed conversion) drawn where they are over the
   latitude the sun's path leans by (43 degrees for Aix-en-Provence) at the real sidereal time (the
   date and the in-game hour), coloured by their temperature, dimmed and reddened toward the horizon,
   outshone by the sky's light; the Milky Way along the galactic plane, brighter toward Sagittarius.
11. Water (done, first step): no texture; waves for its normal; the sky and the sun's glint reflected
   by Fresnel; its depth from the depth prepass, what lies under it tinted by pure water's
   absorption (dual-source blending, where the GPU has it) and its blue-green glow in its place;
   caustics on what it covers from its waves' curvature. Refraction is next (a copy of the frame
   before the water).
   - Sun Size (1x real to 20x): the disc's angular size, the shadows' penumbra with it, the same light.
   - Aerial perspective: the air between the eye and the terrain takes light away (blue most) and scatters
     the horizon's sky in, real air's density by default (16 km of LOD terrain shows it), denser as set.
   - Light keeps bouncing: a bounce ray hitting a surface the last frame showed takes the light found
     there (sky, light sources, bounced light), a bounce more each frame.
   - Test worlds used by shader developers (not shipped, downloaded by hand): PTGISM by wrenchie48
     (planetminecraft.com, coloured rooms for colour bleeding, RGB room, coloured lights, path-traced AO)
     and TobyNight's Ray Tracing Shader Test Map (2026, GI, materials, glass, water). Seen in TobyNight's:
     the coloured panels' light bleeds onto the walls; two rays a pixel over eight frames leave too much
     grain, which a spatial denoiser is to take out.
3. Emissive blocks that light: lava, torches, glowstone as light sources traced with rays; hard shadows for
   torches and small lights (done). The mesher lists each section's visible emitters with a colour by
   block (`mesher.rs` `light_colour`); `gfx/voxels.rs` keeps for every 8-block cell of the grid the four
   whose light reaches it most, and the rays' pass traces a shadow ray toward each, falling off as the
   inverse square of the distance to the light's block. The level's light stands in past the grid.
   Rays start past their own block, so slab and path tops are lit until the grid holds shapes.
4. The real sun, not vanilla's (done): its light and colour through the atmosphere by its elevation, white
   overhead and red as it sets (`gfx/atmosphere.rs`); its 0.53° disc drawn in the sky in place of vanilla's
   sprite, and its shadow rays toward a new point of the disc each frame, accumulated: the penumbra widens
   with the distance to what casts it as the real one does (under a pixel for a slab, centimetres for a
   wall, visible from trees and overhangs).
5. Atmospheric scattering for the sky (done): Rayleigh and Mie scattering and ozone, Bruneton's and
   Hillaire's Earth, single scattering times measured multiple scattering, from a transmittance table made
   once and a 64x32 sky-view table made again on the CPU as the sun moves. The sky drawn, the sky's light on
   the terrain and the fog all come from it, nothing of vanilla's sky colour or sunrise fan.
7. Global illumination, one bounce (done): two rays a pixel over the surface's hemisphere, cosine weighted,
   in directions new each frame; one that gets out brings the sky's light from the sky table, one that hits
   a block that block's light (its map colour from a block-colour grid beside the voxel masks, `R8Uint`,
   25 MB, run-length encoded on the CPU, lit by the sun through a shadow ray, the sky and the nearest light
   source through a shadow ray). The rays' results are accumulated over frames (about eight, reprojected,
   recognised by their distance): no fixed pattern's bands, no second shadow, light round corners. RT AO's
   six rays no longer run with RT lighting: these rays are the occlusion.
   Measured on the RTX 4090 at 3440x1440 in the test scene: 4.06 ms GPU with RT lighting, 3.88 ms with the
   shadows alone (RT lighting off). Not yet measured in a headset.
6. RT AO (done, from the albedo once 1 lands) and MSAA, if its cost on the float scene allows (VR first).

**Next**: that RT lighting mode; soft shadows (a jittered cone and temporal
accumulation, careful in VR); plants and entities; cascades past the grid; comparing with approach B.

## Next: the state of the art, NVIDIA's RTX Remix for Minecraft Java (October 2026)

The reference to reproduce, read but not run: renderer 0.1.0, materials 0.58, showcase world 0.11.1. A Fabric
mod for 1.21.1 that hands the scene to the dxvk-remix runtime, which **path traces** it. Its techniques:
- **Direct light:** ReSTIR DI, every emissive block a light, culled by a "light reach".
- **Indirect light:** ReSTIR GI with NVIDIA's neural radiance cache (CUDA) and SHaRC's hash-grid cache, up to four
  bounces.
- **Denoising and upscaling:** DLSS Ray Reconstruction.
- **Volumetrics:** froxels, with one medium keyed to the camera (water, rain, cave).
- **Sky, clouds:** a LUT sky; mesh clouds.
- **Geometry:** one BLAS per section, GPU-resident.
- **Distance:** a noisy fade into the sky at 70-80 % of the render distance.
- **Post:** auto-exposure, bloom and local tone mapping.
- **Materials:** LabPBR plus a `_sss` tint map, thin or thick SSS by block.

No VR and no performance figures. Its code is Apache-2.0 (ideas free; the sky shader reusable with its notices);
NVIDIA's SDK binaries and headers are not. Its materials pack and showcase world are CC BY 4.0, tested locally,
never put in the repository.

**Where Horizon stands against it** (after c0a06f7cd):
- **Already there:** shadows from the drawn triangles (hardware rays, alpha-tested); area lights (eight a cell,
  four sampled); one bounce plus feedback from the last frame; an a-trous denoiser; coloured shadows through stained
  glass; water (absorption, caustics); the real sky and stars.
- **Missing, ranked by what shows:**

1. **ReSTIR DI:** reservoirs over each cell's candidates, reused in time and between neighbours, so that every
   torch, lantern, glowstone and lava face casts soft shadows at a shadow ray or two a pixel; the "light reach" is
   the cells' range.
   Done (`shadow_rays.wgsl` `restir_light`):
   - **Candidates:** a pixel weighs its cell's lights (up to eight) by their unshadowed light and draws one (RIS).
   - **Reuse:** it merges the last frame's reservoirs where the surface was and at two neighbours (history capped
     at twenty frames), so lights found nearby carry over.
   - **Shading:** one shadow ray toward a point of the chosen light, then accumulated.
   - **Stale reservoirs:** each holds its light's block, so the list being made again drops the stale ones.
   - **Check:** in the test scene's hall (thirty torches and lanterns), the same light as four rays a light cell
     a pixel, at a quarter of the shadow rays. Its cost is unmeasured so far: RTX Remix Minecraft held the GPU at
     99 % during the run.
2. **A world-space radiance cache** (SHaRC-like hash grid): the bounce rays write the light they find into
   voxel-sized world cells, and read it where they hit, for multi-bounce light that holds still at two rays a pixel.
   It is portable; NRC isn't.
   Done (`shadow_rays.wgsl` `CacheCell`, `radiance_cache.wgsl`):
   - **The table:** 2^20 cells, half a block near the camera and twice as large each doubling of the distance, one
     a face direction.
   - **Writing:** every pixel adds the light other than the sun's that its rays found (fixed-point atomics); a
     pass once a frame averages each cell over about ten frames and empties those unseen for 240.
   - **Reading:** a bounce ray reads the cell it hits before falling back to the last frame's screen, so light
     bounces off surfaces the screen no longer shows, the same for both eyes.
3. **The denoiser:** variance-guided (SVGF-class) for the indirect and the direct light, stable in VR (no ghosting
   between the eyes, history kept per eye), and maybe sparse tracing (a third of the pixels a frame).
   Variance-guided, done for the indirect light (`denoise.wgsl`):
   - **Variance:** the accumulation keeps the indirect light's second moment (the sun mask's alpha), giving each
     pixel's variance.
   - **Filtering:** the a-trous passes also weigh a neighbour by how far its light differs, against six standard
     deviations of that noise (less at the coarser steps). Grain smooths out and contact shadows and lit edges
     stay.
   - **Per eye:** the history is per eye already.
   - **Still to do:** sparse tracing and the direct light's own filter.
4. **Post-processing on the linear HDR scene:**
   - auto-exposure, capped by day, night and rain;
   - bloom;
   - a local or ACES tone mapping;
   - KRVR's SDR output kept right.
5. **Water, second step:** refraction (a copy of the frame before the water), normal detail, the view from under
   water (a camera medium). Already better than Remix: a pool lit from above.
6. **Volumetric clouds** in the sky and in the rays (their shadows), replacing vanilla's.
7. **Froxel volumetrics:** god rays through the air, the camera's medium (water, cave, rain), shadows through glass
   within it.
8. **SSS:** `_sss` tint maps when a pack has them, and a thin/thick list by block (leaves and plants thin).
9. **The distance fade:** a coherent-noise fade into the LOD terrain's shading (Horizon draws 1024 blocks, against
   Remix's 400).
10. **Scene residency:** sections' structures rebuilt only when they are meshed again (already); the top-level
    structure on the GPU (instances written by a compute pass from the resident list), for the CPU in VR.
    Started: the instances stand around an anchor block that moves only when the camera is 64 blocks away (the
    rays add the camera's offset from it). The top-level structure is made again only when the anchor moves or
    sections come and go, not each frame the camera moves.
11. **Entities, particles, the held light** in the rays: entities' structures made each frame from their drawn
    vertices.

**Where Horizon is to do better:**
- **VR:** one path for both eyes, rays at KRVR's 1648x1328 an eye, foveated ray density.
- **Portability:** wgpu and Vulkan, no CUDA, hardware rays optional, with the voxel grid behind them.
- **Range:** 1024 blocks of LOD terrain.
- **Version:** 26.3.

**The order, decided by the user: Remix first, as it is, then Horizon's adaptations.**

- **Step one: reproduce Remix's techniques and values**, without the old hybrid plan's shortcuts.
  - **Language:** the host code stays in Rust in Horizon's crates, the GPU work in WGSL through wgpu.
  - **Nothing native from Remix:** no Remix runtime, no dxvk, no JNI or Java, no CUDA, no NVIDIA DLL at run time.
  - **NVIDIA-only pieces** are reimplemented from their published papers, with every gap noted below: NRC on CUDA,
    DLSS Ray Reconstruction, the RTXDI and SHaRC SDKs. No SDK code or header is copied.
  - **Escape hatches**, allowed if measurements call for them:
    - wgpu's experimental features (ray queries, subgroups);
    - Vulkan extensions through `wgpu-hal`, behind an availability check and always with a plain-WGSL fallback:
      ray reordering (`VK_EXT_ray_tracing_invocation_reorder`), cooperative matrices (`VK_KHR_cooperative_matrix`)
      for a neural cache.
- **Step two, apart: the adaptations to Horizon.**
  - VR, rays at KRVR's resolution, foveation.
  - The 1024-block LOD terrain.
  - The CPU budget.
  - Meanwhile one code path for the screen and VR is kept, without optimising for VR at fidelity's expense.
- **Test scene for each step:** a copy of "NVIDIA RTX Demo" with the RTX-Remix-Materials pack, the scene to compare
  with Remix. Each step's GPU cost is noted beside what Remix shows on the same scene, to see where an escape hatch
  would pay.

Remix's values, from its `native/rtx.conf` and its per-situation confs (`minecraft-logic/confs`):

- **Scale:** sceneScale 0.01, so 1 block = 1 m.
- **Auto-exposure:**
  - average mode 0, speed 10, centre metering 0.75;
  - EV from -2.5 to +1.5, capped at -0.5 at dusk and night, and at -1.0 in rain and thunder.
- **Local tone mapping:**
  - exposure 0.214;
  - shadows 1.4 (1.75 in rain, 2.0 in thunder);
  - highlights 2.5;
  - preference sigma 3.5, offset -0.1;
  - the non-legacy ACES.
- **Bloom:** 8 steps; burn 0.1 and threshold 250 by day, burn 1.5 and threshold 1 by night.
- **Translucency and materials:**
  - direct and indirect translucent shadows on;
  - opacity micromaps off;
  - anisotropic filtering off, smooth relief filtering off, UV bounds looping on.
- **Volumetrics:** froxels to 256 m; the runtime's own atmosphere off; settling over 8 frames. Media:
  - water: transmittance 0.6, 0.9, 0.89 over 20 m; albedo 0.206, 0.999, 0.903; anisotropy 0.4;
  - cave: transmittance 0.85 over 20 m; albedo 0.9; anisotropy -0.2; no fog remap;
  - rain: transmittance 0.996 over 10 m;
  - thunder: transmittance 0.97 over 10 m.
- **Distance:** a noise fade (mode 3, function 1, noise scale 20) from 358.4 to 409.6 m, scaled with the view
  distance.
- **Sparse rendering:** quality 0.3, with RTXDI's reuse for the pixels not traced.
- **Caches:**
  - SHaRC: 1000 frames accumulated, stale after 1024, queried for thin opaque primary hits;
  - NRC: 600 m bounds, wrapped positions, no reset on camera cuts;
  - NEE cache: reshuffle resilience off.
- **Scene:** GPU-resident; BLAS merging minimised (one dynamic BLAS a mesh); the view model (the arms) on, with
  perspective correction.

**Each technique, as Remix or an open equivalent** (status after 9348fe274):

| Technique | Horizon | Status |
|---|---|---|
| Primary visibility | rasterised G-buffer (the depth prepass and the terrain's fragments), rays from it; on water, stained glass and ice the ray pass replaces the reflected surface (`hw_reflect`): a ray from the camera meets the nearest translucent triangle in front of the solid depth, turns off it (off the waves on water), meets what it reflects and lights it with the sun's shadow ray and the radiance cache; the material model takes it in place of the sky (`rt_mirror`) | open equivalent of Remix's PSR for reflections. Gaps: primary visibility rasterised, not traced; refraction not replaced (what lies under the water is the scene behind it, unbent); one reflection, no PSR chain; emissive surfaces reflect only through the cache |
| Direct light | ReSTIR DI over 8 lights a cell, temporal and spatial reuse from the last frame, one shadow ray | open equivalent of RTXDI. Gaps, to do: no NEE cache, no light reach beyond the cell's 16 blocks, no sparse rendering (`quality 0.3`) |
| Every emissive block a light | emitters touching air, coloured by block (`mesher.rs` `light_colour`) | as Remix in kind; to do: per-block multipliers, the held light |
| Indirect light | ReSTIR GI: one cosine ray a pixel, resampled with the last frame's reservoir and two neighbours' (same face and distance), a visibility ray for reused samples. With hardware rays the bounce rays meet the drawn triangles (alpha-tested, through tinted glass and water), the hit's normal from its triangle and its colour from its texel times the vertex tint; the hit is lit by the sun's shadow ray and the radiance cache, and a cache miss traces a second bounce (lit by the sun and the cache). Without them, the voxel grid's blocks and map colours. GPU: unchanged within the noise (8.5 against 8.7 ms, 3440x1440, the night hall) | open equivalent of Remix's ReSTIR GI and path tracer. Gaps: two traced bounces then the cache, where Remix traces up to 4; the neighbours' Jacobian taken as 1; the hit's material is its albedo only (no normal map, roughness or emission at the hit); the vertex tint keeps vanilla's directional shade |
| Radiance cache | a hash grid of surface cells (SHaRC-like): each cell averages its frames up to 1000 and is emptied after 1024 frames unseen, Remix's SHaRC values; read by the bounce rays' hits | open equivalent of SHaRC. Gaps: NRC is not reproduced, since an MLP trained each frame needs cooperative matrices to be fast (later escape hatch: `VK_KHR_cooperative_matrix`); the cells are surface cells a 1/1024 block of light apart, not SHaRC's level-of-detail by distance |
| Denoiser | after NRD's ReLAX, Remix's denoiser without DLSS Ray Reconstruction: a per-pixel history of up to 30 frames (each frame adds 1/(n+1)), reprojected bilinearly with face and distance checks; then a variance-guided a-trous filter (4 passes, SVGF) over the indirect and the direct light together, blurring wider where the history is under 4 frames (ReLAX's history fix); the slow history clamped to a 6-frame fast history's mean within 2 standard deviations (ReLAX's history clamping), so a light switched on or off shows within 6 frames | open equivalent of ReLAX, gaps: Remix's default is DLSS Ray Reconstruction (NVIDIA-only, not reproduced); to do: anti-firefly, separate specular |
| Translucent shadows | direct (sun, lights) through stained glass and water | as Remix for the direct light; to do: indirect translucent shadows |
| Sky | `sky_atmosphere.wgsl`, a WGSL port of Remix's `sky_atmosphere.comp` (Apache-2.0, notices in `THIRD_PARTY_NOTICES.md`): its constants, a 1024x256 transmittance table, a 64x32 single-scattering view of 16 view and 16 light steps, analytic multiple scattering 0.65 and the faked ground, on the GPU when the sun moves | as Remix. Differs: a last pass resamples its world-fixed sky into Horizon's sky-view table (azimuth from the sun, square-root elevation); the sun and moon discs have angular radius 0 in the table (they are drawn on their own); Mie anisotropy 0.8 is assumed (the mod sets it); the sun's light on the ground is the CPU twin of the transmittance table (16 steps, sea level) |
| Clouds | `rt_cloud_mesh.rs`: meshes of the clouds' noise density (surface nets), a 700-1300 block slab in 1024-block tiles to 6.5 km, 32-block cells near and 64 beyond, moved by the wind without meshing again, meshed on a worker (65k triangles, 63 ms for the whole ring); shaded by a pass with the sun, the sky table and a through-the-edge lobe. `HORIZON_RT_CLOUDS=march` keeps a raymarched variant, `0` vanilla's. GPU about 0.2 ms at 3440x1440 | open equivalent, gaps: surface nets rather than marching cubes (fewer triangles, no table); in the rays since: each tile is an opaque BLAS built once (rebuilt when its key changes), its instance moved by the wind (placed again after a block of drift), so the sun's and the lights' shadow rays meet them (cloud shadows on the terrain); the bounce and reflection rays skip them (ray mask), the sky pass already carries their light; a crack can show at the 32/64 seam; the clouds are drawn after water and glass |
| Volumetrics | froxels to 256 m (`froxels.rs`) with Remix's camera media (water, cave, rain, thunder), the sun's shadow rays through tinted glass and water (its caustics), the sky's light and one importance-picked light source a froxel, 8-frame settling; on with RT lighting, the medium from the camera's sky light and the weather; under water it replaces vanilla's water fog. GPU about 0.2 ms in the open at 3440x1440 | as Remix's media; gaps: the open-air medium is ours (0.95 over 100 m); rain and thunder albedo and anisotropy guessed; the voxel fallback treats shaped blocks as full cubes |
| Water | its own look whatever the pack (no texture, only the biome's colour): waves, Fresnel, the traced reflection, Remix's medium (transmittance 0.6, 0.9, 0.89 over 20 m, single-scattering albedo 0.206, 0.999, 0.903), the biome colour adding its absorption and tinting the scattered light, caustics from the waves on the sun's rays through it | as Remix for the medium and reflections; gaps: procedural waves where Remix uses its normal-map flipbook; refraction not traced |
| Lava | its own look whatever the pack: domain-warped noise flowing with the time (an integer hash, PCG3D), black-body glow from dark red to yellow-white up to 14 paper whites, a cooling basalt crust lit as the terrain | open equivalent (Remix's lava is its material asset); to do: its light in the bounce rays' hits |
| SSS | `_sss` maps' tints (labPBR's subsurface channel), thin transmission for leaves, plants and glass-like blocks and thick for the rest (`gfx/transmission.rs`) | as Remix in kind |
| Post-processing | `exposure.rs`, `bloom.rs`, `tonemap.rs`: auto-exposure (centre metering 0.75, EV -2.5..+1.5, a log-luminance mean), local tone mapping (exposure fusion: 0.214, shadows 1.4, highlights 2.5, sigma 3.5, offset -0.1), Hill's fit of ACES, a threshold bloom of 8 steps; Remix's situations (`PostParams` dusk, night, rain, thunder) picked each frame from the storm levels and the sun's height (`HORIZON_POST_SITUATION` holds one). About 0.25 ms at 3440x1440 (estimated) | Remix's values; open equivalent of its algorithms, gaps: the exposure speed's formula, its target key (-2.6, measured on a noon scene), the fusion's preferred level (-1.5 stops) and the bloom threshold's units (200 Remix units a paper white) are guesses; HDR output keeps its own curve |
| Distance fade | vanilla's fog at the edge | to do: Remix's noise fade at 70-80 % of the view distance |
| Presets | per-effect settings | to do: low, medium, high and ultra, as Remix |
| Scene residency | one BLAS a section, built once; the top-level structure rebuilt only when its anchor moves or sections change | as Remix in kind; to do: instances written on the GPU |
| View model, entities, particles, held light | `hw_rays.rs` `finish`: the entities' solid quads as drawn (a dynamic BLAS over a copy of their vertices, fast-build, remade every frame; one merged BLAS rather than one a mesh) and the view model in a BLAS of its own, both in the TLAS at the camera's offset from the anchor. The shadow rays alpha-test the entity texel from the shared atlas (entity.wgsl's 0.1 cut-off) and the bounce and reflection rays take its colour and face normal. Particles drawn with the entities (material 0) come with them. `HORIZON_HW_DYNAMIC=0` leaves it all out. GPU cost: under 0.1 ms at 3440x1440 with a pig, a cow and a zombie near (13.1 ms against 13.2 ms on a busy GPU, within the noise) | as Remix in kind (dynamic geometry in the rays, shadows and reflections of entities and of the arms); gaps: one merged BLAS for all entities, not one a mesh; the view model's perspective correction is the ratio of the two fields of view on the view-space xy (Remix's own is a projection fix of its shader); translucent entity layers, crumbling, glint and water masks are left out, and translucent entity texels are opaque above alpha 0.1; the froxels treat entities as opaque quads (no atlas binding there); the hand shades no terrain but its own shadow and bounce; the held light is not a ray-traced light |

**Work order** (step one):

1. Primary surface replacement through water and glass.
2. ReSTIR GI, with bounces up to four through the cache.
3. The NEE cache, light reach and sparse rendering.
4. The sky ported from Remix.
5. Froxel volumetrics with the camera's media.
6. Water's normals and its underwater medium.
7. The distance fade.
8. Presets.
9. Entities, particles, the view model and the held light.

Alongside, in their own worktrees: post-processing, clouds and SSS (merged), froxel volumetrics (in progress).

**Who does what:**

- **The main session:** the ray pass and the integration (`shadow_rays.*`, `denoise.wgsl`, `voxels.rs`, `hw_rays.rs`,
  `gfx.rs`, `terrain.wgsl`).
- **Sonnet helper agents:** pieces in files of their own.
- **Haiku agents:** repetitive measurements and inventories.

**Measuring:** each step is one commit, measured on the desktop (3440x1440) and in the simulated headset
(`tools/vr_fake_run.py`), and compared on the copy of NVIDIA's world.

## What exists: the spike (`origin/feat/raytracing-spike`, 42c752d6b)

A research spike, not on main (it branched 2 800 commits ago; a worktree of it is `../horizon-rt`, and its build is
the `horizon-client-rt.exe` of a player install). Its report is `docs/raytracing.md` on that branch. In short:

- **wgpu 30** `EXPERIMENTAL_RAY_QUERY` behind `ExperimentalFeatures::enabled()` (unsafe, so in a tiny
  `horizon-gpu-experimental` crate), Vulkan and Metal checked; DX12 needs DXC (Horizon compiles DX12 with FXC).
- **One BLAS per chunk section**, built in place from the section arena's vertex buffer and the shared quad index
  buffer (no copy), compacted, at most 512 builds a frame; a remeshed section rebuilds its BLAS. **One TLAS rebuilt
  every frame** with every section, camera-relative. **One shadow ray per solid-terrain pixel** from a variant of the
  terrain fragment shader (`terrain_rt.wgsl`): a shadowed pixel's sky light is scaled by 0.55, block light untouched.
- Measured at 1280x720 (RTX 4080, Vulkan): rays + TLAS **about +1 ms of GPU**; the **CPU** is the problem, the
  TLAS fill costing 0.6-1.1 ms a frame at 12 chunks and **2-7 ms at 32** (wgpu validating every instance). Capped
  to 8 chunks, it came back near vanilla. BLAS: 35-41 µs GPU + 21-26 µs CPU per section; memory about **22 bytes
  per triangle** compacted. Limits: leaves and plants cast square shadows (cutouts are opaque in the BLAS), only
  solid terrain receives, no shader packs, hard shadows only.

## Ray tracing: the plan (from step 3)

### Rules

- **One code path for the screen and VR.** Every ray effect is a pass that takes a list of views: one for the
  window, two for the stereo eyes, four for the quad views. The same passes, structures and denoiser run in both
  modes; nothing is written for the desktop and ported to VR later. A desktop frame is the one-view case.
- **Rays at their own resolution.** Each pass traces into a ray target sized by a *ray scale* of the view, not by
  the render target: KRVR streams about 1648x1328 an eye, so tracing at 4096x3288 throws away about 84 % of the
  rays (1 - 4.4 M / 26.9 M). In VR the ray targets default to the streamed size; on the desktop to the window's.
- **Shared between the eyes**: one world structure, one probe cache, and world-space denoising state where
  possible (probes, voxel AO); screen-space history stays per view, reprojected with each eye's own matrices.
- **Hybrid, not all rays**: rays where they beat raster, classic techniques elsewhere. Everything works in the
  linear HDR scene of step 2, and vanilla's sky light, block light and vertex AO stay the base that rays refine.

### What is traced, and what stays classic

| Effect | Near | Far / fallback | Rays per ray pixel |
|---|---|---|---|
| Sun and moon shadows | rays, 1 a pixel or fewer, temporal denoise for the penumbra (a cone of the 0.5° disc) | cascaded shadow maps from the LOD tiles; a heightmap march for the farthest LOD terrain | 1 |
| Ambient occlusion | short rays (1-2 m) at half resolution, accumulated, multiplied over vanilla's vertex AO | vanilla vertex AO | 2 on a quarter of the pixels = 0.5 |
| Reflections | smooth surfaces only (water, glass, metals, LabPBR smoothness above a threshold): screen space first, a ray when it misses | sky or probe colour | 1 on 10-20 % of pixels = 0.1-0.2 |
| Global illumination | last: a probe cache (a grid around the camera, rays from probes, a few thousand probes a frame) | vanilla sky and block light | per probe, not per pixel: about 0.3 |
| Classic | emissive sources (block light, dynamic lights; shadows only for the sun and a few near sources), refraction and caustics, sky, volumetric fog, clouds | | 0 |

### Two ways to trace, to evaluate first

**A. A voxel grid traced in compute** (as SEUS PTGI, Rethinking Voxels and Photon do). A camera-centred 3D texture,
addressed toroidally (moving one chunk writes one slab, nothing is rebuilt): one byte a block (an id into a small
table: full cube, cutout, slab or stair shape, emissive, transparent), plus a bit-per-block occupancy mip or a
brickmap (8³ bricks under a grid of brick pointers) so the DDA skips empty space. **A block change is one byte
written** (`queue.write_texture` of the dirty bricks), with no build. It is plain compute, so it runs on every
backend wgpu has (Vulkan, DX12 with FXC, Metal from M1, GPUs without RT hardware), without the experimental API.
Weaknesses: non-cubic shapes are approximated (or get a sub-voxel mask per id), and entities aren't in it.

**B. Hardware BLAS/TLAS with ray queries** (the spike): exact triangles, and entities possible, but a build per
remeshed section and a TLAS to maintain. Rebuilt every frame, the spike's TLAS cost 2-7 ms of CPU a frame at 32
chunks; the plan is to keep it and rebuild only on change, writing the instances ourselves
(`build_acceleration_structures_unsafe_tlas`), under 0.3 ms. It also depends on wgpu's experimental API: wgpu 30
has **ray queries only**. `EXPERIMENTAL_RAY_TRACING_PIPELINES` is implemented in wgpu-hal, but it has no wgpu-core
or wgpu entry point, so there is no shader binding table. A Vulkan-direct pipeline would lose Metal and duplicate
wgpu's resource tracking, and it buys nothing for shadow or AO rays.

**Proposed**: A as the base (near shadows, AO, GI probes, rough reflections), then B for what A gets wrong: non-cubic
shapes close to the camera, entities and sharp reflections. The first measurement decides: the same shadow rays
from the same depth buffer, through A and through B (the spike's BLAS), at 12 and 32 chunks, desktop and fake VR.

| Memory and updates | 12 chunks (25² chunks x 384 = 61 M blocks) | 32 chunks (65² x 384 = 415 M blocks) |
|---|---|---|
| A: 1 byte a block, dense | 61 MB | 415 MB |
| A: brickmap (10-20 % of bricks stored) + 1-bit occupancy | 6-12 MB + 8 MB | 40-80 MB + 52 MB |
| B: BLAS, compacted, 22 B a triangle (spike) | 65-110 MB | 0.6-0.85 GB (Metal: 2-3 GB) |
| B: TLAS + instances (64 B each) | about 1 MB | 6-10 MB |
| A block change | A: a few bytes, < 0.01 ms; B: 1-3 BLAS, 40 µs GPU + 25 µs CPU each | same |
| Whole world loaded, or a teleport | A: 0.06 GB written over a few frames; B: 0.2 s of GPU, spread over frames | A: 0.4 GB; B: 1.1 s GPU + 0.65 s CPU, spread |

Rays don't need the whole render distance: shadows and AO trace within `rt_distance` (8-12 chunks by default) and
hand over to the cascades; GI probes and reflections may read coarser mips farther out.

### Cost a frame (estimates, to measure: rays at 0.25-1 ms per million on an RTX 4090)

The spike bounds hardware shadow rays at about 1 ms per million on a 4080; coherent shadow rays and a brickmap DDA
should come nearer 0.25-0.5. Per ray pixel the table above sums to about 2 rays (shadows 1, AO 0.5, reflections
0.1-0.2, GI 0.3).

| Target | Ray pixels | Rays a frame (about 2 a pixel) | GPU | Budget |
|---|---|---|---|---|
| Desktop 3440x1440 | 4.95 M | 9.9 M | 2.5-10 ms | 6.9 ms a frame at 144 Hz, 2-3 of them raster |
| VR, rays at 4096x3288 an eye (not planned) | 26.9 M | 54 M | 13-54 ms | over budget |
| VR, rays at KRVR's 1648x1328 an eye | 4.4 M | 8.8 M | 2.2-8.8 ms | about 8 ms of GPU spare at 90 Hz (2.5 used of 11) |
| The same, foveated (full density within 25° of the gaze, a quarter beyond) | about 1.9 M equivalent | 3.8 M | 1-3.8 ms | fits |
| The linear HDR target (step 2), for reference | | | +0.08 ms desktop, +0.43 ms VR | |

**Foveation pays here.** In raster it was slower: Horizon is CPU-bound (7.2 ms CPU against 2.5 ms GPU) and pixels
are cheap. Rays are GPU work per pixel, so fewer rays away from the gaze is a direct saving. The ray passes read the
gaze (`Frame::gaze`) through a rate image like VRS's; in the quad views, the insets trace at full density and the
wide views sparsely. Rays traced inline in a fragment shader already get cheaper with `HORIZON_XR_VRS=1`.

**Denoise carefully in VR**: trails and shimmer show much more in a headset. Temporal accumulation is clamped
(neighbourhood clamp, short history after a disocclusion, reset on fast head turns), the history is reprojected per
eye, and world-space caches (probes, voxel AO) carry what can be shared, so the two eyes don't disagree. An effect
too noisy for VR stays off there rather than being smeared.

### PBR and HDR

- Rays gate the **direct sun term**. With LabPBR: `N·L` on the normal-mapped normal, the specular lobe from `_s`
  (smoothness, F0 or metals), and POM's self-shadowing kept. Without a pack's maps: vanilla's flat normal and a
  sky-light dimming (the spike settled on 0.55), so the vanilla look stays the default.
- In the linear scene the sun term is radiance in multiples of paper white. Sunlit metal and water highlights go
  above 1.0, and each output's tone mapping takes them to the HDR peak, or rolls them off in SDR (KRVR is SDR only).

### Measuring on a whole world, desktop and VR

- **Desktop**: bring the spike's `tools/rt_shadow_bench.py` over (perf_bench's world, seed 1234, clear noon, a
  village and the snowy peaks, 10 s still), plus a flight across the world at constant speed for the update churn.
  12 and 32 chunks, LOD 0 and 1024, tilt 0° and 40° (a tilted sun makes rays longer), each effect on and off, A
  against B. Report GPU timestamps per pass (`gfx/timing.rs`), the CPU frame without waits, structure updates a
  frame and video memory (wgpu's allocation report and `nvidia-smi`), in SDR and HDR, at 3440x1440.
- **VR, the same runs with the same binary**: the fake runtime (`tools/vr_fake_run.py`, `look_around.txt`,
  unpaced) in stereo, quad views and VRS, rays at 4096x3288 and at 1648x1328, foveated or not. Then the Vision Pro
  through KRVR with SteamVR's frame timing, looking for trails and shimmer during fast head turns. Pass: 90 Hz with
  CPU and GPU under 11 ms, and the CPU no more than 0.3 ms above ray tracing off.
- **Visual**: captures at noon, tilt 0° and 40°, effects on and off, in a forest (cutouts), a village and a cave;
  the HDR debug view (`HORIZON_HDR_DEBUG=1`).

### Order and rough size

1. The linear HDR scene, a tone mapping per output, the vanilla mode kept exact, measured (five to seven days).
2. The ray pass frame: a list of views (1, 2 or 4), ray targets at their own scale, per-view history, the gaze's
   rate image (two to three days). Everything after it runs on the desktop and in VR from its first commit.
3. Prototype A (voxel brickmap, toroidal updates, DDA) and port B (the spike, with `celestial::direction`) far
   enough to trace the same shadow rays; measure both (four to five days), and pick the base.
4. Near sun and moon shadows with a temporal penumbra; cascades and the LOD heightmap far away (five days).
5. AO at half resolution (two days); reflections, screen space and then rays (four days).
6. LabPBR's sun term and specular in the linear scene (three days).
7. The GI probe cache (a project of its own, two to three weeks).
8. Settings ("Ray tracing", per effect, off by default) and the launcher toggle; Metal from M3 and DXC on DX12 for B.
