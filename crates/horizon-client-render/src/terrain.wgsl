// Terrain, sky and outline shaders, ported from the pinned vanilla
// terrain.vsh/fsh, fog.glsl and sample_lightmap.glsl.
struct Globals {
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    // environmental start/end, render-distance start/end
    fog: vec4<f32>,
    sky_color: vec4<f32>,
    // x: sky darkening, y: game time, z: aspect, w: tan(fov/2)
    misc: vec4<f32>,
    inverse_view: mat4x4<f32>,
    sunrise: vec4<f32>,
    // xyz: sun direction, w: star brightness
    sun: vec4<f32>,
    // x: sky fog end, y: cloud fog end, z: dark disc flag, w: RT lighting's plain glass's sprite
    // layers (`rt_glass_plain`)
    fog_ends: vec4<f32>,
    // Atlas rects (u0, v0, width, height) of end_sky.png and end_portal.png.
    end_portal_sprites: array<vec4<f32>, 2>,
    // x: vanilla's GameTime ((game time % 24000 + partial tick) / 24000), z: how many
    // dynamic_lights are set, w: 1 when the frame is drawn in linear light (scene_rgb)
    time: vec4<f32>,
    // Atlas rect of the current destroy stage (read by entity.wgsl).
    crumbling_sprite: vec4<f32>,
    outline_params: vec4<f32>,
    // Horizon Extras' dynamic lights: camera-relative position and level (dynamic_light.rs).
    dynamic_lights: array<vec4<f32>, 16>,
    colored_lights: array<vec4<f32>, 32>,
    colored_count: vec4<u32>,
    emissive_layers: array<vec4<u32>, 128>,
    emissive_count: vec4<u32>,
    // The sunrise fan's 16 triangles (gfx/sunrise_fan.rs): xyz the apex weight's normal, w the
    // triangle's volume with the camera; then each rim vertex's normal against the apex.
    sunrise_fan: array<vec4<f32>, 32>,
    // x: Options.textureFiltering (0 none, 1 RGSS, 2 anisotropic), y: the sprite arrays' width
    // in texels (vanilla's TextureSize), z: the mip level at which a sprite is one texel when
    // the views reach it, else 0 (`rotated_grid`).
    filtering: vec4<f32>,
    // HDR output's tier windows (hdr.rs `Hdr::windows`): x..w low to very bright,
    // [0] where the lift starts, [1] where it is full.
    hdr_windows: array<vec4<f32>, 2>,
    // HDR lighting: x how strongly the sky lights the world now (`Sky::sky_light_factor`),
    // y what the sky and fog get: the same, but none under water; z 1 with HDR output on.
    hdr_light: vec4<f32>,
    // Ray-traced shadows (gfx/shadow_rays.rs): xyz toward the sun or the moon, w the shadows'
    // strength (0: no shadows).
    rt_light: vec4<f32>,
    // RT lighting (gfx/shadow_rays.rs): the last frame's view projection (clip space from world
    // space relative to its camera), the camera's move since (xyz; w 1 when that frame's light can
    // be read; w 2: this frame's, xy the world target's size), and x: the terrain's vertices hold
    // their tint alone (mesher.rs SHADE_BITS),
    // y: HORIZON_RT_DEBUG=7, z: the light traced toward is the moon, w: the aerial perspective's
    // air density (times real air's).
    rt_prev_view_proj: mat4x4<f32>,
    rt_motion: vec4<f32>,
    rt_mode: vec4<f32>,
    // RT lighting's real sun (gfx/atmosphere.rs): xyz toward the sun, w 1 when the sky table
    // (`rt_sky`) holds its sky; rgb the light the rays trace toward (the sun's through the
    // atmosphere, or the moon's), in paper whites, w the sun's angular radius.
    rt_sun: vec4<f32>,
    rt_sun_light: vec4<f32>,
    // RT lighting's stars (terrain.wgsl `vs_rt_star`): w their brightness (vanilla's, 0 by day);
    // and the real sky's turn (gfx/sky_stars.rs `frame`): world directions from the equatorial
    // ones, at the latitude the sun's path leans by and the real sidereal time.
    rt_stars: vec4<f32>,
    rt_star_frame: mat4x4<f32>,
    // RT lighting's water: x and y its still and flowing sprite layers, z the time in seconds, w
    // the lava's layers (still + 4096 flowing, -1 none);
    // and the camera's place in the world (xyz, wrapped every 4096 blocks), for its waves.
    rt_water: vec4<f32>,
    rt_camera: vec4<f32>,
    // RT lighting's moon: toward it (xyz) and its angular radius (w); its phase in rt_stars.x.
    rt_moon: vec4<f32>,
    // The Earth's curvature (`earth_drop`): x the square of the radius it starts at (below 0 off),
    // y how far the sea's horizon dips under the horizontal, in radians; z RT lighting's ice and
    // packed ice sprite layers (`rt_ice`, ice + 4096 packed ice, -1 none); w whether the surf
    // and the wet shore are on (1 and more; gfx.rs `rt_shore`), its fraction the ambient light.
    earth: vec4<f32>,
    // The shooting stars' shower (`meteor_make`): its radiant (equatorial, xyz) and zenithal hourly
    // rate (w); its meteors' speed (km/s, x) and population index (y), the sporadic rate (z).
    rt_shower: vec4<f32>,
    rt_stream: vec4<f32>,
    // RT lighting's rain (rain.rs): x its rate in mm/h (0 without the drops), y how wet the shaded
    // surfaces are, z how far the shaded puddles have grown, w its cloud cover (gfx/atmosphere.rs
    // `overcast`, 0 to 1); and in full sun: x the surfaces' wetness, y the puddles', zw the wind
    // (m/s along x and z).
    rain: vec4<f32>,
    rain_sun: vec4<f32>,
    // x 1 when the frame is drawn in linear light (scene_rgb, `linear_on`); y RT lighting's plain
    // glass sprite layers, packed (gfx.rs `set_glass_layers`).
    linear: vec4<f32>,
}
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var sprites: texture_2d_array<f32>;
@group(0) @binding(2) var sprite_sampler: sampler;
@group(0) @binding(3) var lightmap: texture_2d<f32>;
@group(0) @binding(4) var lightmap_sampler: sampler;
// Layers past the first array's SPRITE_SPLIT (2048, gfx.rs): mods' sprites.
@group(0) @binding(5) var sprites_1: texture_2d_array<f32>;
@group(0) @binding(6) var sprites_2: texture_2d_array<f32>;
@group(0) @binding(7) var sprites_3: texture_2d_array<f32>;
@group(0) @binding(8) var sprites_4: texture_2d_array<f32>;
@group(0) @binding(9) var sprites_5: texture_2d_array<f32>;
@group(0) @binding(10) var sprites_6: texture_2d_array<f32>;
@group(0) @binding(11) var sprites_7: texture_2d_array<f32>;
// Per block sprite (x the layer in its array, y the array), the brightness its
// pixels must reach to shine in HDR output: r where the bright parts shine, g
// where all of it does (gfx/hdr_stats.rs, measured from the sprite itself).
@group(0) @binding(26) var emission_thresholds: texture_2d<f32>;
// RT lighting: this view's light from the last frame (rgb, and the distance in a), from the rays
// (gfx/shadow_rays.rs); 1x1 without it.
@group(0) @binding(12) var rt_history: texture_2d<f32>;
// RT lighting: this frame's light from the light sources, traced (shadow_rays.wgsl `block_mask`):
// rgb the light, a below 0 where the level's stands in.
@group(0) @binding(13) var rt_blocks: texture_2d<f32>;
// RT lighting's sky (gfx/atmosphere.rs): its radiance by azimuth from the sun (0..pi) and
// elevation, in paper whites.
@group(0) @binding(14) var rt_sky_view: texture_2d<f32>;
// RT lighting: this frame's sun reaching the pixel through what tints it (stained glass, water),
// rgb, all of it 1 (shadow_rays.wgsl `sun_mask`).
@group(0) @binding(15) var rt_sun_tint: texture_2d<f32>;
// RT lighting: this frame's depth prepass (the solid terrain alone), for the water to know how
// deep it is at its pixel.
@group(0) @binding(16) var rt_depth: texture_depth_2d;
// RT lighting: what the water at the pixel reflects, traced (Remix's primary surface replacement,
// shadow_rays_hw.wgsl `hw_reflect`): rgb its light, a 1 a surface, 0 the sky (reflected here).
@group(0) @binding(17) var rt_reflect: texture_2d<f32>;
// RT lighting: what the water at the pixel refracts, traced (shadow_rays_psr.wgsl `hw_refract`):
// rgb its light, a the distance the ray ran in the water (0: none, the scene behind shows).
@group(0) @binding(18) var rt_refract: texture_2d<f32>;
// RT lighting: the Moon's map (NASA's LROC colours, equirectangular, the near side in the middle).
@group(0) @binding(19) var rt_moon_map: texture_2d<f32>;
// RT lighting: the Milky Way's diffuse light (NASA's Deep Star Maps 2020, celestial coordinates,
// right ascension 0h in the middle and increasing to the left).
@group(0) @binding(20) var rt_milky_map: texture_2d<f32>;
// RT lighting: the Eye of Aldhani's field this frame (gfx/aldhani.wgsl), and its streaks by tile
// of the sky (each tile's count, then its meteors).
@group(0) @binding(21) var<storage, read> aldhani_field: AldhaniField;
@group(0) @binding(22) var<storage, read> aldhani_tiles: array<u32>;
// RT lighting's rain map (gfx/rain.rs, rain.rs `RainMap`): each column's rain height, top surface,
// light and whether it rains there, for the drops, the wet surfaces and the puddles.
@group(0) @binding(23) var<storage, read> rain_map: array<u32>;
// The traced reflection for the fragment being lit (`rt_reflection`), which `rt_shade` takes in
// place of the sky's: rgb its light, a 1 when there is one.
var<private> rt_mirror: vec4<f32>;
// How much of the sun the surface's own relief lets reach the fragment (gfx/parallax.rs
// `parallax_self_shadow`; 1 for a face without a height map), which `rt_shade` takes.
var<private> rt_relief_sun: f32 = 1.0;
// What the translucent surface at `pixel` (camera-relative `pos`, in front of the solid terrain)
// reflects, traced (`rt_reflect`); a 0 for a solid fragment or where the rays reached the sky.
fn rt_reflection(pixel: vec2<f32>, pos: vec3<f32>) -> vec4<f32> {
    if globals.rt_motion.w < 1.5 { return vec4(0.0); }
    let distance = length(pos);
    let bottom = rt_bottom(pixel, pos / max(distance, 1e-4));
    if bottom >= 0.0 && bottom < distance + 0.05 { return vec4(0.0); }
    return rt_psr_at(rt_reflect, pixel, false);
}
// The view's last resolved frame (gfx/taa.rs `last_color`, linear light; 1x1 without the temporal
// resolve), for the screen-space reflections.
@group(0) @binding(24) var rt_scene: texture_2d<f32>;
// RT lighting's clouds' shadows (gfx/rt_clouds.rs `shadow`): the light's transmittance through the
// clouds by where its ray crosses their base, world x and z modulo 4096 blocks; 1x1 white without.
@group(0) @binding(27) var rt_cloud_shadows: texture_2d<f32>;
@group(0) @binding(28) var rt_cloud_sampler: sampler;
// A screen-space reflection, where the rays have none (past the 8 chunks their structures hold, or
// a ray that found nothing there): the reflected ray from camera-relative `pos` along `dir`
// marched across the screen in growing steps (half a block to a kilometre) against this frame's
// solid depth (`rt_depth`, reversed: its linear distance is the clip w times the ratio of the
// depths), refined by halving where it passes behind a surface by less than the step, and the
// light there read from the view's last resolved frame (the scene a frame old: far off, as good
// as this one's). rgb that light, a its weight: fading out at the screen's edges, 0 where the ray
// leaves the screen, turns behind the camera or meets nothing (the sky then shows, as before).
fn rt_screen_reflection(pos: vec3<f32>, dir: vec3<f32>) -> vec4<f32> {
    if globals.rt_motion.w < 1.5 || textureDimensions(rt_scene).x < 2u { return vec4(0.0); }
    let size = vec2<f32>(textureDimensions(rt_depth));
    var near = 0.0;
    var far = 0.5;
    var found = false;
    for (var i = 0; i < 24; i++) {
        let c = globals.view_proj * vec4(pos + dir * far, 1.0);
        if c.w <= 1e-3 { return vec4(0.0); }
        let ndc = c.xy / c.w;
        if any(abs(ndc) > vec2(1.0)) { return vec4(0.0); }
        let at = clamp(vec2<i32>(vec2(0.5 + 0.5 * ndc.x, 0.5 - 0.5 * ndc.y) * size), vec2(0),
            vec2<i32>(size) - 1);
        let depth = textureLoad(rt_depth, at, 0);
        let ray_depth = c.z / c.w;
        if depth > ray_depth {
            // The surface there is nearer than the ray's point: by how much, in blocks.
            let behind = c.w - c.w * ray_depth / depth;
            if behind < max(far - near, 0.5) {
                found = true;
                break;
            }
        }
        near = far;
        far *= 1.35;
    }
    if !found { return vec4(0.0); }
    for (var i = 0; i < 5; i++) {
        let mid = 0.5 * (near + far);
        let c = globals.view_proj * vec4(pos + dir * mid, 1.0);
        let ndc = c.xy / c.w;
        let at = clamp(vec2<i32>(vec2(0.5 + 0.5 * ndc.x, 0.5 - 0.5 * ndc.y) * size), vec2(0),
            vec2<i32>(size) - 1);
        if textureLoad(rt_depth, at, 0) > c.z / c.w { far = mid; } else { near = mid; }
    }
    let c = globals.view_proj * vec4(pos + dir * far, 1.0);
    let ndc = c.xy / c.w;
    let scene = vec2<f32>(textureDimensions(rt_scene));
    let at = clamp(vec2<i32>(vec2(0.5 + 0.5 * ndc.x, 0.5 - 0.5 * ndc.y) * scene), vec2(0),
        vec2<i32>(scene) - 1);
    let edge = 1.0 - smoothstep(0.85, 1.0, max(abs(ndc.x), abs(ndc.y)));
    return vec4(textureLoad(rt_scene, at, 0).rgb, edge);
}
// How many sprite arrays hold layers (a pipeline constant): 1 for vanilla,
// which then samples exactly as before.
override SPRITE_ARRAYS: u32 = 1u;
override SPRITE_SPLIT: u32 = 2048u;
// Whether the solid shaders carry HDR output's lighting (a pipeline constant): the
// renderer builds its solid terrain without it for the frames HDR is off, where
// `hdr_light.z` is 0, since even a branch never taken costs Apple GPUs ~15% of a
// solid pixel's time (gfx.rs's `SolidPipeline`).
override HDR_LIGHTING: bool = true;
// A sprite sampled with the given gradients, from whichever array holds its layer.
fn sprite_grad(uv: vec2<f32>, layer: u32, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    let l = i32(layer % SPRITE_SPLIT);
    if SPRITE_ARRAYS == 1u {
        return textureSampleGrad(sprites, sprite_sampler, uv, l, dx, dy);
    }
    switch layer / SPRITE_SPLIT {
        case 0u: { return textureSampleGrad(sprites, sprite_sampler, uv, l, dx, dy); }
        case 1u: { return textureSampleGrad(sprites_1, sprite_sampler, uv, l, dx, dy); }
        case 2u: { return textureSampleGrad(sprites_2, sprite_sampler, uv, l, dx, dy); }
        case 3u: { return textureSampleGrad(sprites_3, sprite_sampler, uv, l, dx, dy); }
        case 4u: { return textureSampleGrad(sprites_4, sprite_sampler, uv, l, dx, dy); }
        case 5u: { return textureSampleGrad(sprites_5, sprite_sampler, uv, l, dx, dy); }
        case 6u: { return textureSampleGrad(sprites_6, sprite_sampler, uv, l, dx, dy); }
        default: { return textureSampleGrad(sprites_7, sprite_sampler, uv, l, dx, dy); }
    }
}
// The same at an explicit mip level.
fn sprite_level(uv: vec2<f32>, layer: u32, lod: f32) -> vec4<f32> {
    let l = i32(layer % SPRITE_SPLIT);
    if SPRITE_ARRAYS == 1u {
        return textureSampleLevel(sprites, sprite_sampler, uv, l, lod);
    }
    switch layer / SPRITE_SPLIT {
        case 0u: { return textureSampleLevel(sprites, sprite_sampler, uv, l, lod); }
        case 1u: { return textureSampleLevel(sprites_1, sprite_sampler, uv, l, lod); }
        case 2u: { return textureSampleLevel(sprites_2, sprite_sampler, uv, l, lod); }
        case 3u: { return textureSampleLevel(sprites_3, sprite_sampler, uv, l, lod); }
        case 4u: { return textureSampleLevel(sprites_4, sprite_sampler, uv, l, lod); }
        case 5u: { return textureSampleLevel(sprites_5, sprite_sampler, uv, l, lod); }
        case 6u: { return textureSampleLevel(sprites_6, sprite_sampler, uv, l, lod); }
        default: { return textureSampleLevel(sprites_7, sprite_sampler, uv, l, lod); }
    }
}
// texture_sampling.glsl's sampleNearest: the sprite's texels stay square pixels, blending only
// across the one screen pixel where two meet, so a linear (and anisotropic) sampler keeps them crisp.
// `tiled`: a distant tile's face takes its sprite again per block, so its gradients span many
// sprites while `uv` stays inside one (see `tile_uv` below).
fn sample_nearest(uv: vec2<f32>, layer: u32, pixel: vec2<f32>, du: vec2<f32>, dv: vec2<f32>,
                  texel_screen: vec2<f32>, tiled: bool) -> vec4<f32> {
    let texel_coords = uv / pixel;
    let center = round(texel_coords) - 0.5;
    var offset = texel_coords - center;
    offset = (offset - 0.5) * pixel / max(texel_screen, vec2(1e-12)) + 0.5;
    offset = clamp(offset, vec2(0.0), vec2(1.0));
    let at = (center + offset) * pixel;
    if tiled {
        // An anisotropic sampler lays its taps along those gradients, past the sprite's edge,
        // where the clamped sampler reads that edge texel again: long streaks on the far tiles.
        // Asking for the level the gradients give takes the one tap an anisotropy of 1 would.
        let level = max(length(du), length(dv)) / min(pixel.x, pixel.y);
        return sprite_level(at, layer, max(0.0, log2(level)));
    }
    return sprite_grad(at, layer, du, dv);
}
// sampleRGSS: four rotated-grid samples of the two nearest mip levels, faded in as the texels
// shrink below two screen pixels, over sample_nearest.
fn sample_rgss(uv: vec2<f32>, layer: u32, pixel: vec2<f32>, du: vec2<f32>, dv: vec2<f32>,
               texel_screen: vec2<f32>, tiled: bool) -> vec4<f32> {
    let max_texel = max(texel_screen.x, texel_screen.y);
    let min_pixel = min(pixel.x, pixel.y);
    let blend = smoothstep(min_pixel, min_pixel * 2.0, max_texel);
    // Only the side of the mix that has weight is sampled: near texels (blend 0) take
    // sample_nearest's one tap alone, far ones (blend 1) the rotated grid alone.
    if blend <= 0.0 {
        return sample_nearest(uv, layer, pixel, du, dv, texel_screen, tiled);
    }
    let rgss = rotated_grid(uv, layer, pixel, du, dv);
    if blend >= 1.0 {
        return rgss;
    }
    return mix(sample_nearest(uv, layer, pixel, du, dv, texel_screen, tiled), rgss, blend);
}
fn rotated_grid(uv: vec2<f32>, layer: u32, pixel: vec2<f32>, du: vec2<f32>,
                dv: vec2<f32>) -> vec4<f32> {
    let min_pixel = min(pixel.x, pixel.y);
    let du_len = length(du);
    let dv_len = length(dv);
    let effective = sqrt(min(du_len, dv_len) * max(du_len, dv_len));
    let exact = max(0.0, log2(effective / min_pixel));
    let low = floor(exact);
    let mip_blend = fract(exact);
    // From this level on the sprite is a single texel (0: the views stop before one is), and
    // the sampler clamps any level past it to it: there every tap reads the same value, so
    // one tap gives the four's mean. Distant tiles sit there.
    let single = globals.filtering.z;
    if single > 0.0 && low >= single {
        return sprite_level(uv, layer, low);
    }
    let high_single = single > 0.0 && low + 1.0 >= single;
    var offsets = array<vec2<f32>, 4>(
        vec2(0.125, 0.375), vec2(-0.125, -0.375), vec2(0.375, -0.125), vec2(-0.375, 0.125));
    var sum_low = vec4(0.0);
    var sum_high = vec4(0.0);
    for (var i = 0; i < 4; i++) {
        let p = uv + offsets[i] * pixel;
        sum_low += sprite_level(p, layer, low);
        if !high_single {
            sum_high += sprite_level(p, layer, low + 1.0);
        }
    }
    if high_single {
        sum_high = 4.0 * sprite_level(uv, layer, low + 1.0);
    }
    return mix(sum_low * 0.25, sum_high * 0.25, mip_blend);
}
// Distant terrain tiles (lod/mesh.rs) count their UVs in blocks past 1: each block takes the
// sprite again, as separate faces would under the clamped sampler. Sprite UVs in [0, 1] stay.
fn tile_uv(uv: vec2<f32>) -> vec2<f32> {
    return uv - max(ceil(uv) - 1.0, vec2(0.0));
}
fn sprite(tiled: vec2<f32>, layer: u32) -> vec4<f32> {
    // The gradients come from the unwrapped UV, so the mip level holds across block seams.
    return sprite_at(tiled, layer, dpdx(tiled), dpdy(tiled));
}
// `sprite` with the unwrapped UV's screen gradients given: the visibility buffer's resolve
// (visbuffer.wgsl) works them out from the quad, as its neighbours may be other quads.
fn sprite_at(tiled: vec2<f32>, layer: u32, du: vec2<f32>, dv: vec2<f32>) -> vec4<f32> {
    // terrain.fsh's two sampling paths: sampleRGSS for RGSS, sampleNearest otherwise.
    let uv = tile_uv(tiled);
    // The wrap shifts a face's UVs by whole blocks, so half of one tells a tile's face from a
    // block's (whose UVs stay inside its sprite).
    let tiled_face = any(tiled - uv >= vec2(0.5));
    let pixel = vec2(1.0 / globals.filtering.y);
    let texel_screen = sqrt(du * du + dv * dv);
    if u32(globals.filtering.x) == 1u {
        return sample_rgss(uv, layer, pixel, du, dv, texel_screen, tiled_face);
    }
    return sample_nearest(uv, layer, pixel, du, dv, texel_screen, tiled_face);
}

struct TerrainOut {
    // Made invariant where the world tests against a depth prepass for equality
    // (`HORIZON_WORLD_PREPASS`, gfx.rs `terrain_source`), so that both give each fragment the same
    // depth to the bit; not else, as it measured costly on the RTX 4090.
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) color: vec4<f32>,
    @location(3) spherical: f32,
    @location(4) cylindrical: f32,
    // How the face shines in HDR output (mesher.rs's `EMISSION_BITS`): 0 not
    // at all, 1 to 4 the tier with its bright parts, 5 to 8 the tier all over.
    @location(5) @interpolate(flat) emission: u32,
    // HDR lighting: block and sky light, as levels 0 to 15.
    @location(11) scene: vec2<f32>,
    @location(6) @interpolate(flat) fade: u32,
    // A distant tile's tile-relative position, for its fade (`faded`).
    @location(7) local: vec3<f32>,
    // terrain.vsh's chunkVisibility: the section's fade-in, from 16 blocks out.
    @location(8) visibility: f32,
    // The camera-relative height, for the water mirror's cut at the plane.
    @location(9) height: f32,
    @location(10) @interpolate(flat) emissive_layer: u32,
    // For ray-traced shadows: the sky and block light levels (0..15) the lightmap is read at.
    @location(12) light: vec2<f32>,
    // The camera-relative position.
    @location(13) pos: vec3<f32>,
    // For RT lighting: rgb the face's tint (its albedo with the sprite's texel); w, for its
    // water, how open it lies to the wind, 0 a swamp's still water, 0.6 a lake's, 1 the open
    // sea's (its colour's alpha's high nibble, mesher.rs `with_sea`), blended across the biomes
    // as their colours are. One vector, so the stage stays within the weakest GPUs'
    // inter-stage limit (gfx/inter_stage_tests.rs).
    @location(14) tint: vec4<f32>,
}
// Whether a fading tile or section leaves this pixel out (gfx/lod.rs's
// `fade_code`, `fade_out_code`). It fades in, or out, chunk by chunk: each
// column of its own grid (`local`, the tile-relative position: 16 blocks,
// 2^level for coarse tiles) starts in turn over the first half of the fade
// and fades over the second, as a 4x4 ordered pattern in the face's plane,
// its unit the power of two of a block nearest 1.5 pixels, so it stays on
// the terrain as the camera moves. `dx` and `dy`: `local`'s screen
// derivatives.
fn faded(code: u32, local: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>) -> bool {
    if code == 0u { return false; }
    // Sixty-fourths of the fade: 1 to 63 fading in, 65 to 128 fading out.
    let step = (code - 1u) % 129u;
    let out = step >= 64u;
    let progress = f32(select(step, step - 64u, out)) / 64.0;
    let size = exp2(f32(max((code - 1u) / 129u, 4u)));
    let cell = bitcast<vec2<u32>>(vec2<i32>(floor(local.xz / size)));
    var h = cell.x * 73856093u ^ cell.y * 19349663u;
    h = (h ^ (h >> 13u)) * 0x5bd1e995u;
    let start = f32((h ^ (h >> 15u)) & 1023u) / 2048.0;
    let ramp = clamp((progress - start) * 2.0, 0.0, 1.0);
    let shown = select(ramp, 1.0 - ramp, out);
    let n = abs(cross(dx, dy));
    var plane = local.xy;
    if n.x >= n.y && n.x >= n.z {
        plane = local.yz;
    } else if n.y >= n.z {
        plane = local.xz;
    }
    let pixel = max(length(dx), length(dy));
    let unit = exp2(ceil(log2(max(pixel * 1.5, 1.0 / 16.0))));
    let p = vec2<u32>(bitcast<vec2<u32>>(vec2<i32>(floor(plane / unit)))) & vec2(3u);
    let bayer = array<u32, 16>(0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u);
    return (f32(bayer[p.y * 4u + p.x]) + 0.5) / 16.0 >= shown;
}
// A fading tile or section is drawn a hair toward the camera (depth is
// reversed: nearer is larger), the newer the more, so it wins where it draws
// over the solid tiles it replaces. Distant tiles go back by 0.2% of their
// distance, so near chunks win over tiles still under them (gfx/lod.rs's
// `previous`, columns not complete yet): near water lies a little below a
// tile's (vanilla's fluid corner heights), and under a smaller step the
// tile's drew over it, two waters, opaque. A tile's UV step is 1/32, a
// section's 1/32768 (`terrain_uv`).
fn fade_depth(clip: vec4<f32>, w: f32) -> vec4<f32> {
    let code = fade_code(w);
    let tile = instance_step(w) > 0.001;
    var scale = select(1.0, 1.0 - 2e-3, tile);
    if code != 0u && (code - 1u) % 129u < 64u {
        scale *= 1.0 + 5e-6 * f32(64u - (code - 1u) % 129u);
    } else if code != 0u && !tile {
        // A section leaving (gfx/lod.rs's `HELD`, `fade_out_code`) goes 2% of
        // its distance toward the camera: far terrain's coarser blocks stand a
        // little out of the real ones, and the tile coming in under it hid it
        // at once, before its fade.
        scale *= 1.02;
    }
    return vec4(clip.xy, clip.z * scale, clip.w);
}
// Horizon Extras' dynamic lights (dynamic_light.rs): the block light, in the vertex's units
// (16 per level), a vertex at camera-relative `pos` gets from the frame's light sources, one
// level less per block. time.z of them are set: none when the extra is off, so no loop runs.
fn dynamic_block_light(pos: vec3<f32>) -> f32 {
    let count = min(u32(globals.time.z), 16u);
    var level = 0.0;
    for (var i = 0u; i < count; i++) {
        let l = globals.dynamic_lights[i];
        level = max(level, l.w - distance(pos, l.xyz));
    }
    return level * 16.0;
}
fn native_colored_light(pos: vec3<f32>) -> vec3<f32> {
    var rgb = vec3(0.0);
    for (var i = 0u; i < min(globals.colored_count.x, 16u); i++) {
        let at = globals.colored_lights[i * 2u];
        let color = globals.colored_lights[i * 2u + 1u];
        rgb += color.rgb * max(0.0, 1.0 - distance(pos, at.xyz) / at.w) * color.a;
    }
    return min(rgb, vec3(1.0));
}
fn sample_lightmap(sky: f32, block: f32) -> vec4<f32> {
    let uv = vec2(block, sky) * 16.0;
    return textureSampleLevel(lightmap, lightmap_sampler,
        clamp(uv / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0)), 0.0);
}
// mesher.rs's TerrainVertex holds the UV in fixed point; the instance's
// origin.w is the step: 1/32768 per unit for sections (a power of two on an
// integer below 2^24, so it decodes exactly), 1/32 per block for distant
// terrain tiles (TerrainVertex::tiled). A fading tile negates it past its
// dither code (gfx/lod.rs's `instance`): the step is the fraction either way.
fn terrain_uv(packed: vec2<u32>, w: f32) -> vec2<f32> {
    return vec2<f32>(packed) * instance_step(w);
}
// A whole section's step (a power of two) carries its fade-in in the mantissa's 23 bits, as
// `1 - visibility` (gfx.rs's `section_instance`): masked off, the step; the rest is
// `RenderSection.getVisibility`. A fading one (negative) is fully visible.
fn instance_step(w: f32) -> f32 {
    if w > 0.0 {
        return bitcast<f32>(bitcast<u32>(w) & 0xff800000u);
    }
    return fract(abs(w));
}
fn instance_visibility(w: f32) -> f32 {
    if w > 0.0 {
        return 1.0 - f32(bitcast<u32>(w) & 0x7fffffu) / 8388608.0;
    }
    return 1.0;
}
// A fading distant tile's dither code (gfx/lod.rs's `fade_code`), 0 when whole.
fn fade_code(w: f32) -> u32 {
    return u32(floor(max(-w, 0.0)));
}
// The Earth's curvature (gfx.rs `Renderer::earth_curvature`), for the eye alone: the world stays
// flat for the game, and a distant terrain tile's vertex sinks by its horizontal distance's square
// over the Earth's diameter, counted from the loaded chunks' radius (earth.x its square) so the
// tiles meet those flat chunks, which the rays trace, without a step: 2 m 5 km out, 21 m at 16 km,
// where the foot of an island drops behind the sea seen from 1.7 m up (its horizon 4.7 km away).
// Every pass draws the terrain through `terrain_vertex`, and positions are relative to the head,
// so a tile lands at the same place in each pass and in both eyes.
const EARTH_RADIUS_M: f32 = 6371000.0;
fn earth_drop(xz: vec2<f32>, flat_radius_sq: f32) -> f32 {
    return max(dot(xz, xz) - flat_radius_sq, 0.0) / (2.0 * EARTH_RADIUS_M);
}
// `pos` as drawn: lowered when its instance's w says a distant tile (`instance_step`).
fn earth_curved(pos: vec3<f32>, w: f32) -> vec3<f32> {
    let flat_sq = globals.earth.x;
    if flat_sq < 0.0 || instance_step(w) < 0.001 { return pos; }
    return vec3(pos.x, pos.y - earth_drop(pos.xz, flat_sq), pos.z);
}
@vertex fn vs_terrain(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>,
    @location(5) origin: vec4<f32>) -> TerrainOut {
    return terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin.xyz,
        origin.w);
}
// Flat section draws (gfx/flat.rs): one draw per arena page over the frame's
// index list. A section's vertices fill whole 256-vertex blocks of its page:
// the block gives its resident slot, the slot its camera-relative origin.
@group(1) @binding(0) var<storage, read> block_slots: array<u32>;
@group(1) @binding(1) var<storage, read> slot_origins: array<vec4<f32>>;
@vertex fn vs_terrain_flat(
    @builtin(vertex_index) vertex: u32,
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>
) -> TerrainOut {
    let origin = slot_origins[block_slots[vertex >> 8u]];
    return terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin.xyz,
        origin.w);
}
// Solid terrain draws in two passes, each over its own part of every mesh's quads
// (`mesher::Part`): first the quads with no cutout that are not fading, through
// `fs_opaque`, which never discards, so a tiled GPU (Apple's) keeps hiding the pixels
// behind them; then the rest through `fs_solid`. One discard anywhere in a pipeline's shader turned that hiding off for
// all the terrain it drew. The draw lists give each pass only its own part; a mesh that
// was not sorted (`Sides::unsorted`) gives both passes all of its quads, so each pass
// still leaves out the other's: decided before any of the vertex's work, outside the
// clip volume, it makes no pixel.
fn opaque_quad(layer: u32, w: f32) -> bool {
    return (layer & 0x8000u) != 0u && fade_code(w) == 0u;
}
fn left_out() -> TerrainOut {
    var out: TerrainOut;
    out.clip = vec4(2.0, 2.0, 2.0, 1.0);
    return out;
}
@vertex fn vs_terrain_opaque(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>,
    @location(5) origin: vec4<f32>) -> TerrainOut {
    if !opaque_quad(layer, origin.w) { return left_out(); }
    return terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin.xyz,
        origin.w);
}
@vertex fn vs_terrain_cutout(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>,
    @location(5) origin: vec4<f32>) -> TerrainOut {
    if opaque_quad(layer, origin.w) { return left_out(); }
    return terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin.xyz,
        origin.w);
}
@vertex fn vs_terrain_flat_opaque(
    @builtin(vertex_index) vertex: u32,
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>
) -> TerrainOut {
    let origin = slot_origins[block_slots[vertex >> 8u]];
    if !opaque_quad(layer, origin.w) { return left_out(); }
    return terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin.xyz,
        origin.w);
}
@vertex fn vs_terrain_flat_cutout(
    @builtin(vertex_index) vertex: u32,
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>
) -> TerrainOut {
    let origin = slot_origins[block_slots[vertex >> 8u]];
    if opaque_quad(layer, origin.w) { return left_out(); }
    return terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin.xyz,
        origin.w);
}
// Solid section draws that read their vertices from the arena page (gfx.rs's `pulled`
// pipelines; `mesher/packed.rs`): a draw's vertex indices below PACKED_BASE name vertices
// (`TerrainVertex`, six words, as `TERRAIN_ATTRIBUTES` reads them), from PACKED_BASE on a
// corner of the quad whose record is at that slot (`PACKED_BASE + 4 * slot + corner`).
// Either way they come out as the six words of the vertex, so the work after is the same.
// Binding 19: visbuffer.wgsl, in the same module, takes 2 to 18 of its own group 1.
@group(1) @binding(19) var<storage, read> arena_page: array<vec2<u32>>;
const PACKED_BASE: u32 = 0x10000000u;
// A vertex's words: position bits, then UV, layer with light, colour.
struct ArenaCorner {
    p: vec3<u32>,
    r: vec3<u32>,
}
fn arena_vertex(v: u32) -> ArenaCorner {
    let a = arena_page[v * 3u];
    let b = arena_page[v * 3u + 1u];
    let c = arena_page[v * 3u + 2u];
    return ArenaCorner(vec3(a.x, a.y, b.x), vec3(b.y, c.x, c.y));
}
// The offset a block corner adds on one component: `side` on the face's axis (rel 0), `du`
// on the next axis (rel 1), `dw` on the one after (rel 2).
fn corner_offset(rel: u32, side: u32, du: u32, dw: u32) -> u32 {
    return select(select(dw, du, rel == 1u), side, rel == 0u);
}
// `packed::packed_vertex`: corner `corner` of the record whose words are lo (0 to 2) and
// hi (3 to 5).
fn packed_corner(lo: vec3<u32>, hi: vec3<u32>, corner: u32) -> ArenaCorner {
    let orient = (hi.z >> 12u) & 0x3ffffu;
    let block = vec3(orient & 15u, (orient >> 4u) & 15u, (orient >> 8u) & 15u);
    let axis = (orient >> 12u) & 3u;
    let side = (orient >> 14u) & 1u;
    let start = (orient >> 15u) & 3u;
    let dir = (orient >> 17u) & 1u;
    let step = (start + select(corner, 4u - corner, dir == 1u)) & 3u;
    let du = select(0u, 1u, step == 1u || step == 2u);
    let dw = select(0u, 1u, step >= 2u);
    // No dynamic component stores: FXC (DirectX 12) rejects them (X3500). A component's offset
    // from the axis (0: the axis, 1: the next, 2: the one after) picks what it gets. Scalar
    // selects only: a vector select becomes GLSL's mix(uvec3, uvec3, bvec3), which old AMD
    // OpenGL drivers (Radeon HD 7400M, driver 15.301) resolve to the float overload and reject.
    let offset = vec3(
        corner_offset((3u - axis) % 3u, side, du, dw),
        corner_offset((4u - axis) % 3u, side, du, dw),
        corner_offset((5u - axis) % 3u, side, du, dw),
    );
    let pos = block + offset;
    let sel = lo.z >> 24u;
    let us = (sel >> (2u * corner)) & 1u;
    let vs = (sel >> (2u * corner + 1u)) & 1u;
    let u = (lo.z >> (6u * us)) & 63u;
    let v = (lo.z >> (12u + 6u * vs)) & 63u;
    var light = hi.z;
    if corner == 0u {
        light = hi.x >> 16u;
    } else if corner == 1u {
        light = hi.y;
    } else if corner == 2u {
        light = hi.y >> 12u;
    }
    light &= 0xfffu;
    let packed_light = ((light & 63u) << 2u) | (((light >> 6u) & 63u) << 10u);
    let g = (lo.y >> (8u * corner)) & 255u;
    let t = lo.x;
    let color = (g * ((t >> 16u) & 255u) / 255u) | ((g * ((t >> 8u) & 255u) / 255u) << 8u)
        | ((g * (t & 255u) / 255u) << 16u) | (t & 0xff000000u);
    return ArenaCorner(bitcast<vec3<u32>>(vec3<f32>(pos)),
        vec3((u << 10u) | (v << 26u), (hi.x & 0xffffu) | (packed_light << 16u), color));
}
// A solid draw's vertex `vertex`, and the slot its block is found by (`block_slots`, 256
// slots a block): the vertex's own, or its quad's record's. `packed`: it has a record.
struct SectionCorner {
    c: ArenaCorner,
    slot: u32,
    packed: bool,
}
fn section_corner(vertex: u32) -> SectionCorner {
    if vertex < PACKED_BASE {
        return SectionCorner(arena_vertex(vertex), vertex, false);
    }
    let slot = (vertex - PACKED_BASE) >> 2u;
    let corner = vertex & 3u;
    let a = arena_page[slot * 3u];
    let b = arena_page[slot * 3u + 1u];
    let c = arena_page[slot * 3u + 2u];
    // `packed::FULL`: the quad's four vertices, word 0 slots on.
    if (c.y & 0x80000000u) != 0u {
        return SectionCorner(arena_vertex(slot + a.x + corner), slot, true);
    }
    return SectionCorner(packed_corner(vec3(a.x, a.y, b.x), vec3(b.y, c.x, c.y), corner), slot,
        true);
}
// The colour as the vertex fetch converts a Unorm8x4 attribute, bit for bit (Apple's M1:
// times 1/255; `unpack4x8unorm` and the correctly rounded c / 255 each differ from it for
// some bytes, and moved a few pixels' colour or alpha cutout).
fn unorm8x4(c: u32) -> vec4<f32> {
    return vec4<f32>(vec4(c & 255u, (c >> 8u) & 255u, (c >> 16u) & 255u, c >> 24u))
        * (1.0 / 255.0);
}
fn corner_layer(c: ArenaCorner) -> u32 {
    return c.r.y & 0xffffu;
}
fn corner_vertex(c: ArenaCorner, origin: vec4<f32>) -> TerrainOut {
    let uv = vec2(c.r.x & 0xffffu, c.r.x >> 16u);
    let light = vec2((c.r.y >> 16u) & 0xffu, c.r.y >> 24u);
    return terrain_vertex(bitcast<vec3<f32>>(c.p), terrain_uv(uv, origin.w), corner_layer(c),
        unorm8x4(c.r.z), light, origin.xyz, origin.w);
}
@vertex fn vs_section(@builtin(vertex_index) vertex: u32,
    @location(5) origin: vec4<f32>) -> TerrainOut {
    return corner_vertex(section_corner(vertex).c, origin);
}
@vertex fn vs_section_opaque(@builtin(vertex_index) vertex: u32,
    @location(5) origin: vec4<f32>) -> TerrainOut {
    let s = section_corner(vertex);
    if !opaque_quad(corner_layer(s.c), origin.w) { return left_out(); }
    return corner_vertex(s.c, origin);
}
@vertex fn vs_section_cutout(@builtin(vertex_index) vertex: u32,
    @location(5) origin: vec4<f32>) -> TerrainOut {
    let s = section_corner(vertex);
    if opaque_quad(corner_layer(s.c), origin.w) { return left_out(); }
    return corner_vertex(s.c, origin);
}
@vertex fn vs_section_flat(@builtin(vertex_index) vertex: u32) -> TerrainOut {
    let s = section_corner(vertex);
    let origin = slot_origins[block_slots[s.slot >> 8u]];
    return corner_vertex(s.c, origin);
}
@vertex fn vs_section_flat_opaque(@builtin(vertex_index) vertex: u32) -> TerrainOut {
    let s = section_corner(vertex);
    let origin = slot_origins[block_slots[s.slot >> 8u]];
    if !opaque_quad(corner_layer(s.c), origin.w) { return left_out(); }
    return corner_vertex(s.c, origin);
}
@vertex fn vs_section_flat_cutout(@builtin(vertex_index) vertex: u32) -> TerrainOut {
    let s = section_corner(vertex);
    let origin = slot_origins[block_slots[s.slot >> 8u]];
    if opaque_quad(corner_layer(s.c), origin.w) { return left_out(); }
    return corner_vertex(s.c, origin);
}
// HDR lighting: how far a surface's light carries it above paper white, 0 to 1. Only the
// strongest light does, so dark stays as dark as in SDR; sky light by how the face meets the sun.
fn hdr_scene(block: f32, sky: f32, facing: f32) -> f32 {
    let b = smoothstep(11.0, 15.0, block);
    let s = smoothstep(11.0, 15.0, sky) * globals.hdr_light.x * facing;
    return max(b, s);
}
// The frame's alpha for it, between 1 (none) and the light sources' marks below 0.75.
fn hdr_scene_alpha(scene: f32) -> f32 {
    return 1.0 - 0.24 * scene;
}
// The face's normal, from the fragment's derivatives, for how it faces the sun.
var<private> hdr_normal: vec3<f32>;
fn hdr_facing() -> f32 {
    return mix(0.45, 1.0, max(dot(hdr_normal, globals.sun.xyz), 0.0));
}
// The wind (gfx/wind.rs): a header (`wind_f`), two maps of the wind the terrain makes around the
// camera, the newer fading in over the older (`wind_terrain`), and each sprite layer's way of
// swaying (`wind_class`), for the leaves, plants and vines to move in it (`wind_offset`), every
// terrain vertex the same in every pass (the depth prepass, the colour pass, the eyes'), so they
// keep meeting the same depths.
@group(0) @binding(25) var<storage, read> wind_buf: array<u32>;
// WIND-BEGIN: the text up to WIND-END is also the rays' (gfx/shadow_rays.rs `source` appends it
// after shadow_rays_hw.wgsl's own `wind_buf`), for the shadows to sway with what casts them.
const WIND_MAP: u32 = 32u;
const WIND_MAP_WORDS: u32 = 131072u;
const WIND_SIDE: i32 = 256;
const WIND_CELL: f32 = 8.0;
const WIND_LAYERS: u32 = 262176u;
fn wind_f(i: u32) -> f32 { return bitcast<f32>(wind_buf[i]); }
fn wind_class(layer: u32) -> u32 {
    let l = layer & 0x7fffu;
    return (wind_buf[WIND_LAYERS + (l >> 2u)] >> ((l & 3u) * 8u)) & 255u;
}
// mesher.rs `wind_pin`: the height's float's last bit, or -0.
fn wind_pinned(y: f32) -> bool { return (bitcast<u32>(y) & 0x80000001u) != 0u; }
// Map `m`'s texel `c` of its window (outside it, flat open ground), weighed by `w`: (exposure,
// slope x, slope z, how much a valley confines the wind) and (the valley's axis x, z).
fn wind_texel(m: u32, c: vec2<i32>, a: ptr<function, vec4<f32>>, b: ptr<function, vec2<f32>>,
    w: f32) {
    let h = 12u * m;
    if any(c < vec2(0)) || any(c >= vec2(WIND_SIDE)) || wind_buf[h + 11u] == 0u { return; }
    let o = vec2(i32(wind_buf[h + 2u]), i32(wind_buf[h + 3u]));
    let s = (o + c) & vec2(WIND_SIDE - 1);
    let at = WIND_MAP + m * WIND_MAP_WORDS + 2u * u32(s.y * WIND_SIDE + s.x);
    *a += w * unpack4x8snorm(wind_buf[at]);
    *b += w * unpack4x8snorm(wind_buf[at + 1u]).xy;
}
// Map `m` at camera-relative `p` (x, z), bilinear, weighed by `w`.
fn wind_map(m: u32, p: vec2<f32>, a: ptr<function, vec4<f32>>, b: ptr<function, vec2<f32>>,
    w: f32) {
    if w <= 0.0 { return; }
    let h = 12u * m;
    let u = (p - vec2(wind_f(h), wind_f(h + 1u))) / WIND_CELL - 0.5;
    let i = vec2<i32>(floor(u));
    let f = u - floor(u);
    wind_texel(m, i, a, b, w * (1.0 - f.x) * (1.0 - f.y));
    wind_texel(m, i + vec2(1, 0), a, b, w * f.x * (1.0 - f.y));
    wind_texel(m, i + vec2(0, 1), a, b, w * (1.0 - f.x) * f.y);
    wind_texel(m, i + vec2(1, 1), a, b, w * f.x * f.y);
}
fn wind_hash(c: vec2<f32>) -> f32 {
    var h = (u32(i32(c.x)) * 0x8da6b343u) ^ (u32(i32(c.y)) * 0xd8163841u);
    h = (h ^ (h >> 15u)) * 0x2c1b3c6du;
    h = (h ^ (h >> 12u)) * 0x297a2d39u;
    return f32((h ^ (h >> 15u)) & 0xffffu) / 65535.0;
}
fn wind_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(wind_hash(i), wind_hash(i + vec2(1.0, 0.0)), u.x),
        mix(wind_hash(i + vec2(0.0, 1.0)), wind_hash(i + vec2(1.0, 1.0)), u.x), u.y);
}
// The world position (wrapped every 4096 blocks) of camera-relative `pos`.
fn wind_world(pos: vec3<f32>) -> vec3<f32> { return pos + vec3(wind_f(4u), wind_f(5u), wind_f(6u)); }
// The wind at camera-relative `pos` (world `world`): xz its velocity in m/s, y the gust passing
// there (0 to 1). The prevailing wind bent by the terrain: faster on ridges and up the windward
// slopes, calm in hollows and in the lee, turned along a valley and faster through its narrows,
// faster with the altitude; the gusts a noise blown along it. gfx/wind.rs `bend` is its CPU copy.
fn wind_at(pos: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let base = vec2(wind_f(8u), wind_f(9u));
    let calm = length(base);
    var dir = base / max(calm, 1e-3);
    if calm < 1e-3 { dir = vec2(1.0, 0.0); }
    var a = vec4(0.0);
    var b = vec2(0.0);
    let fade = clamp(wind_f(28u), 0.0, 1.0);
    wind_map(0u, pos.xz, &a, &b, 1.0 - fade);
    wind_map(1u, pos.xz, &a, &b, fade);
    let exposure = a.x;
    let slope = a.yz;
    let confined = clamp(a.w, 0.0, 1.0);
    let axis = b / max(length(b), 1e-3);
    let along = dot(dir, axis);
    let turned = normalize(mix(dir, axis * select(-1.0, 1.0, along >= 0.0), 0.75 * confined) + 1e-4);
    var speed = calm * clamp(1.0 + 0.6 * exposure, 0.25, 1.8);
    speed *= clamp(1.0 + 1.5 * dot(dir, slope), 0.4, 1.6);
    speed *= 1.0 + confined * (0.8 * abs(along) - 0.4 * (1.0 - abs(along)));
    speed *= clamp(1.0 + (world.y - 63.0) / 192.0, 0.7, 1.8);
    let t = wind_f(7u);
    // The gusts drift with the prevailing wind's speed alone: a drift that followed the bent
    // speed would jump wherever the map changes.
    let g = (world.xz - turned * t * (2.0 + calm)) / 20.0;
    let gust = smoothstep(0.3, 0.85, 0.7 * wind_noise(g) + 0.3 * wind_noise(g * 2.7 + 17.0));
    return vec3(turned.x * speed, gust, turned.y * speed);
}
// How far a vertex of class `kind` (`wind_class`, not 0) at world `world` moves in wind `w`
// (`wind_at`), its sprite UV `uv`, `pinned` (`wind_pinned`), `open` how much the sky and the
// distance let it move (0 to 1). Leaves and vines flutter, every vertex the same at a corner the
// blocks share; a leaf corner touching a trunk, the ground or snow stays put, so the leaves hold
// to the wood and the snow on them stays on. A plant is anchored at its sprite's foot (v = 1) and
// bends most at its top (v = 0); a tall plant's lower half reaches half the bend at its top, the
// upper half the whole at its own. Every oscillation keeps a fixed frequency (the gusts change
// the amplitudes only): a frequency that followed the gust turns its phase, t times the change,
// into jumps.
fn wind_shape(kind: u32, world: vec3<f32>, w: vec3<f32>, uv: vec2<f32>, pinned: bool, open: f32)
    -> vec3<f32> {
    if kind == 0u || (kind == 1u && pinned) || open <= 0.0 { return vec3(0.0); }
    let speed = length(w.xz);
    let dir = w.xz / max(speed, 1e-3);
    let gust = w.y;
    let strength = clamp(speed / 4.0, 0.0, 1.6) * open;
    let t = wind_f(7u);
    if kind == 1u || kind == 5u {
        let phase = dot(world, vec3(0.37, 0.23, 0.31)) * 6.2831853;
        let amp = select(0.045, 0.06, kind == 5u) * strength;
        let flutter = vec3(sin(t * 2.3 + phase), 0.5 * sin(t * 3.1 + phase * 1.7),
            sin(t * 2.7 + phase * 1.3));
        return (flutter * (0.4 + 0.6 * gust) + vec3(dir.x, 0.0, dir.y) * gust) * amp;
    }
    // A slow phase across the world (a quarter radian a block): a plant's corners, under a block
    // apart, swing together instead of shearing its quads, and a field sways in waves.
    let slow = dot(world.xz, vec2(0.15, 0.2));
    let v = 1.0 - clamp(uv.y, 0.0, 1.0);
    var reach = v;
    if kind == 3u { reach = 0.5 * v; }
    if kind == 4u { reach = 0.5 + 0.5 * v; }
    let bend = reach * reach;
    let lean = 0.08 + 0.2 * gust;
    let sway = 0.06 * (0.5 + gust) * sin(t * 2.1 + slow);
    let side = 0.02 * (0.5 + gust) * sin(t * 2.9 + slow * 1.3);
    var h = (dir * (lean + sway) + vec2(-dir.y, dir.x) * side) * strength * bend;
    h *= min(1.0, 0.4 / max(length(h), 1e-4));
    // The stem keeps its length: its top comes down as it leans.
    return vec3(h.x, -0.5 * dot(h, h), h.y);
}
// How much the sky light `sky` (0 to 1) and the distance from the camera `d` let the wind move a
// vertex: nothing under a roof (as packs read `lmcoord.y`), fading out from 64 to 96 blocks.
fn wind_open(sky: f32, d: f32) -> f32 {
    if wind_f(10u) <= 0.0 { return 0.0; }
    return sky * sky * (1.0 - smoothstep(64.0, 96.0, d));
}
// How far the terrain vertex at camera-relative `pos` (sprite UV `uv`, sprite `layer`, sky light
// `sky` 0 to 1, `pinned`) moves in the wind; 0 with the wind off (HORIZON_RT_SKIP 4096).
fn wind_offset(pos: vec3<f32>, uv: vec2<f32>, layer: u32, sky: f32, pinned: bool) -> vec3<f32> {
    let kind = wind_class(layer);
    if kind == 0u { return vec3(0.0); }
    let open = wind_open(sky, length(pos));
    if open <= 0.0 { return vec3(0.0); }
    let world = wind_world(pos);
    return wind_shape(kind, world, wind_at(pos, world), uv, pinned, open);
}
// WIND-END
fn terrain_vertex(position: vec3<f32>, uv: vec2<f32>, layer: u32, color: vec4<f32>,
    light: vec2<u32>, origin: vec3<f32>, w: f32) -> TerrainOut {
    var local = position + origin;
    // The sections' vertices (not the distant terrain's tiles, whose UVs repeat by block) sway.
    if instance_step(w) < 0.001 {
        local += wind_offset(local, uv, layer, clamp(f32(light.x) / 240.0, 0.0, 1.0),
            wind_pinned(position.y));
    }
    let pos = earth_curved(local, w);
    var out: TerrainOut;
    out.clip = fade_depth(globals.view_proj * vec4(pos, 1.0), w);
    out.local = position;
    out.uv = uv;
    out.layer = layer;
    out.emissive_layer = 0xffffffffu;
    // The pairs are sorted by base layer: stop at the first past this one.
    let sprite_layer = layer & 0x7fffu;
    for (var i = 0u; i < min(globals.emissive_count.x, 128u); i++) {
        let pair = globals.emissive_layers[i];
        if pair.x == sprite_layer {
            out.emissive_layer = pair.y;
            break;
        }
        if pair.x > sprite_layer {
            break;
        }
    }
    let block = max(f32(light.y), dynamic_block_light(pos));
    // With RT lighting the vertex holds its tint alone and its shade, to four bits, in the alpha's
    // high nibble: vanilla's colour, for where the rays don't reach, is their product.
    var shade = 1.0;
    if globals.rt_mode.x > 0.5 {
        shade = (f32(u32(round(color.a * 255.0)) >> 4u) * 16.0 + 8.0) / 255.0;
    }
    out.tint = vec4(color.rgb, f32(u32(round(color.a * 255.0)) >> 4u) / 15.0);
    out.pos = pos;
    out.color = vec4(color.rgb * shade, 1.0) * sample_lightmap(f32(light.x) / 16.0, block / 16.0);
    out.light = vec2(f32(light.x), block) / 16.0;
    out.scene = vec2(block / 16.0, f32(light.x) / 16.0);
    if globals.colored_count.x > 0u {
        out.color = vec4(min(out.color.rgb + color.rgb * shade * native_colored_light(pos), vec3(1.0)), out.color.a);
    }
    out.emission = 15u - (u32(round(color.a * 255.0)) & 15u);
    out.height = pos.y;
    out.spherical = length(pos);
    out.cylindrical = max(length(pos.xz), abs(pos.y));
    out.fade = fade_code(w);
    const CHUNK_FULLY_VISIBLE_RANGE: f32 = 16.0;
    out.visibility = mix(1.0, instance_visibility(w),
        clamp((length(pos) - CHUNK_FULLY_VISIBLE_RANGE) / CHUNK_FULLY_VISIBLE_RANGE, 0.0, 1.0));
    return out;
}
fn linear_fog(d: f32, start: f32, end: f32) -> f32 {
    if d <= start { return 0.0; }
    if d >= end { return 1.0; }
    return (d - start) / (end - start);
}
// Linear Light (gfx/tonemap.rs): the frame goes to a float scene target in linear light, and a
// tone mapping takes it to the screen. Shaders still compute vanilla's gamma-space colours and
// decode them on the way out: a product of gamma values decodes to the product of the decoded
// ones, so lit surfaces come out as vanilla's, while blending, fog and light sources work in light.
fn linear_on() -> bool { return globals.linear.x > 0.5; }
fn decode_gamma(rgb: vec3<f32>) -> vec3<f32> { return pow(max(rgb, vec3(0.0)), vec3(2.2)); }
fn scene_rgb(rgb: vec3<f32>) -> vec3<f32> {
    if linear_on() { return decode_gamma(rgb); }
    return rgb;
}
// How much brighter than paper white a block that emits `emission` (0..15) shines in linear
// light: up to 4x for level 15 (lava, glowstone, sea lanterns), 1 for blocks that emit nothing.
fn emission_gain(emission: u32) -> f32 {
    let e = f32(emission) / 15.0;
    return 1.0 + 3.0 * e * e;
}
fn fog_amount(spherical: f32, cylindrical: f32) -> f32 {
    return max(linear_fog(spherical, globals.fog.x, globals.fog.y),
               linear_fog(cylindrical, globals.fog.z, globals.fog.w));
}
fn apply_fog(color: vec4<f32>, spherical: f32, cylindrical: f32) -> vec4<f32> {
    let f = fog_amount(spherical, cylindrical);
    return vec4(mix(color.rgb, globals.fog_color.rgb, f * globals.fog_color.a), color.a);
}
// A world colour as the frame target takes it: vanilla's, fogged in gamma space; in linear light
// that decoded (fog distances are tuned for gamma-space mixing: mixed in light, the fog would
// thicken), plus what a light source (`gain` > 1) shines beyond paper white, under the same fog.
fn scene_color(color: vec4<f32>, gain: f32, spherical: f32, cylindrical: f32) -> vec4<f32> {
    let fogged = apply_fog(color, spherical, cylindrical);
    if !linear_on() { return fogged; }
    let f = fog_amount(spherical, cylindrical) * globals.fog_color.a;
    let extra = decode_gamma(color.rgb) * ((gain - 1.0) * (1.0 - f));
    return vec4(decode_gamma(fogged.rgb) + extra, color.a);
}
// Ray-traced shadows (gfx/shadow_rays.rs): a face the sun does not reach keeps this much of its sky
// light level, and all its block light.
const SHADOW_SKY: f32 = 0.55;
// With Linear Light the frame's alpha (free then) is half the share of the pixel's light it keeps
// in the shadow: under 0.5 for solid terrain, 1 for everything else (entities, the hand, the sky),
// so the tone mapping darkens only terrain with the rays' mask (gfx/tonemap.wgsl).
// `light` is the sky and block light levels, `emission` the light the face emits.
fn sun_share(light: vec2<f32>, emission: u32) -> f32 {
    if emission > 0u { return 1.0; }
    let lit = sample_lightmap(light.x, light.y).rgb;
    let dark = sample_lightmap(light.x * SHADOW_SKY, light.y).rgb;
    let y = vec3(0.2126, 0.7152, 0.0722);
    return clamp(pow(dot(dark, y) / max(dot(lit, y), 1e-4), 2.2), 0.0, 1.0);
}
// RT lighting: what the rays found here last frame (x the sun's visibility, y the light around, z
// the sky seen, below 0 when no ray got there: past the grid), reprojected into this view's last
// mask; (1, 1, -1) where none of the four texels around showed this surface (a disocclusion, the
// view's edge): lit and open for that frame.
// RT lighting's masks hold a texel for `scale`x`scale` pixels where they are coarser than the view
// (HORIZON_RT_SCALE, the screen's at 4K, a VR eye's 2x2), each traced from one pixel of its block
// (shadow_rays.wgsl `pixel_of`). A fragment reads the four texels around it, each weighed
// bilinearly and by how near the surface its ray met lies to the fragment's (the distance in
// `rt_history`'s alpha), so a shadow's or a wall's edge stays where it is instead of showing the
// blocks a nearest read does; with no texel on its surface (a thin edge) the nearest. The weights
// once a fragment (`rt_taps`), for every mask read.
struct RtTaps {
    base: vec2<i32>,
    w: vec4<f32>,
    // A plain read: one texel (`base`) a pixel, or the nearest.
    nearest: bool,
}
var<private> rt_taps: RtTaps;
var<private> rt_taps_ready: bool = false;
fn rt_taps_at(pixel: vec2<f32>, pos: vec3<f32>) -> RtTaps {
    if rt_taps_ready { return rt_taps; }
    rt_taps_ready = true;
    let size = vec2<f32>(textureDimensions(rt_history));
    let scale = globals.rt_motion.xy / size;
    let top = vec2<i32>(size) - 1;
    let nearest = clamp(vec2<i32>(pixel / scale), vec2(0), top);
    rt_taps = RtTaps(nearest, vec4(0.0), true);
    if scale.x < 1.5 || globals.rt_mode.x < 0.5 { return rt_taps; }
    let s = floor(scale.x + 0.5);
    let at = (pixel - 0.5 - floor(s / 2.0)) / s;
    let base = vec2<i32>(floor(at));
    let f = at - floor(at);
    let d = length(pos);
    var w = vec4(0.0);
    for (var i = 0; i < 4; i++) {
        let t = clamp(base + vec2(i & 1, i >> 1u), vec2(0), top);
        let a = textureLoad(rt_history, t, 0).a;
        let bi = select(1.0 - f.x, f.x, (i & 1) == 1) * select(1.0 - f.y, f.y, i >= 2);
        let near = 1.0 - smoothstep(0.0, 0.05 + 0.02 * d, abs(a - d));
        w[i] = select(0.0, (bi + 0.01) * near, a >= 0.0);
    }
    let total = w.x + w.y + w.z + w.w;
    if total > 1e-3 { rt_taps = RtTaps(base, w / total, false); }
    return rt_taps;
}
// What the mirrors and the refraction traced at `pixel` (`rt_reflect`, `rt_refract`): where they are
// coarser than the view, read bilinearly between the four texels around (no 2x2 blocks on the
// water), over those that traced something (alpha above 0) when `traced`, else all of them (a
// reflection's alpha is its share of surface against the sky's). The light at a hit is one
// sample, not denoised: a texel's surface far brighter than the others' around (a firefly, which
// would cover 2x2 pixels and more) is left out.
fn rt_psr_at(t: texture_2d<f32>, pixel: vec2<f32>, traced: bool) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(t));
    let scale = globals.rt_motion.xy / size;
    let top = vec2<i32>(size) - 1;
    if scale.x < 1.5 { return textureLoad(t, clamp(vec2<i32>(pixel / scale), vec2(0), top), 0); }
    let s = floor(scale.x + 0.5);
    let at = (pixel - 0.5 - floor(s / 2.0)) / s;
    let base = vec2<i32>(floor(at));
    let f = at - floor(at);
    var taps: array<vec4<f32>, 4>;
    var w = vec4(0.0);
    var lum = vec4(0.0);
    var dimmest = 1e30;
    for (var i = 0; i < 4; i++) {
        taps[i] = textureLoad(t, clamp(base + vec2(i & 1, i >> 1u), vec2(0), top), 0);
        w[i] = select(1.0 - f.x, f.x, (i & 1) == 1) * select(1.0 - f.y, f.y, i >= 2) + 1e-3;
        if traced && taps[i].a <= 0.0 { w[i] = 0.0; }
        lum[i] = dot(taps[i].rgb, vec3(0.2126, 0.7152, 0.0722));
        if w[i] > 0.0 && taps[i].a > 0.0 { dimmest = min(dimmest, lum[i]); }
    }
    var sum = vec4(0.0);
    var total = 0.0;
    for (var i = 0; i < 4; i++) {
        if taps[i].a > 0.0 && lum[i] > 4.0 * dimmest + 0.05 { w[i] = 0.0; }
        sum += w[i] * taps[i];
        total += w[i];
    }
    if total <= 1e-4 { return vec4(0.0); }
    return sum / total;
}
// Mask `t` at the fragment at `pixel` (camera-relative `pos`), through `rt_taps`.
fn rt_mask(t: texture_2d<f32>, pixel: vec2<f32>, pos: vec3<f32>) -> vec4<f32> {
    let taps = rt_taps_at(pixel, pos);
    if taps.nearest { return textureLoad(t, taps.base, 0); }
    let top = vec2<i32>(textureDimensions(t)) - 1;
    var sum = vec4(0.0);
    for (var i = 0; i < 4; i++) {
        if taps.w[i] > 0.0 {
            sum += taps.w[i] * textureLoad(t, clamp(taps.base + vec2(i & 1, i >> 1u), vec2(0), top), 0);
        }
    }
    return sum;
}
fn rt_visibility(pos: vec3<f32>, pixel: vec2<f32>) -> vec3<f32> {
    // This frame's rays, traced on the depth prepass (rt_motion.xy the world target's size): the
    // mask at this pixel.
    if globals.rt_motion.w > 1.5 {
        return rt_mask(rt_history, pixel, pos).rgb;
    }
    if globals.rt_motion.w < 0.5 { return vec3(1.0, 1.0, -1.0); }
    let q = pos + globals.rt_motion.xyz;
    let c = globals.rt_prev_view_proj * vec4(q, 1.0);
    if c.w <= 0.0 { return vec3(1.0, 1.0, -1.0); }
    let ndc = c.xy / c.w;
    if any(abs(ndc) > vec2(1.0)) { return vec3(1.0, 1.0, -1.0); }
    let size = vec2<f32>(textureDimensions(rt_history));
    let px = vec2((ndc.x * 0.5 + 0.5) * size.x, (0.5 - ndc.y * 0.5) * size.y) - 0.5;
    let base = vec2<i32>(floor(px));
    let d = length(q);
    var best = vec3(1.0, 1.0, -1.0);
    var error = 0.2 + 0.03 * d;
    for (var i = 0; i < 4; i++) {
        let at = clamp(base + vec2(i & 1, i >> 1u), vec2(0), vec2<i32>(size) - 1);
        let h = textureLoad(rt_history, at, 0);
        let e = abs(h.a - d);
        if h.a >= 0.0 && e < error {
            error = e;
            best = h.rgb;
        }
    }
    return best;
}
// RT lighting's sky toward `d` (unit): the atmosphere's light (gfx/atmosphere.rs `sky_view`),
// and during the Eye of Aldhani its glow (`rt_camera.w` the seconds into it, below 0 none).
fn rt_sky(d: vec3<f32>) -> vec3<f32> {
    let flat = d.xz / max(length(d.xz), 1e-5);
    let sun = globals.rt_sun.xz / max(length(globals.rt_sun.xz), 1e-5);
    let azimuth = acos(clamp(dot(flat, sun), -1.0, 1.0)) / 3.14159265;
    // The table's horizon is level; the sea's dips under it with the eye's height (earth.y).
    let e = clamp((asin(clamp(d.y, -1.0, 1.0)) + globals.earth.y) / 1.5707963, -1.0, 1.0);
    let v = 0.5 + 0.5 * sign(e) * sqrt(abs(e));
    var sky = textureSampleLevel(rt_sky_view, lightmap_sampler, vec2(azimuth, v), 0.0).rgb;
    if globals.rt_camera.w >= 0.0 { sky += aldhani_glow(d, globals.rt_camera.w) + aldhani_burning(d); }
    return sky;
}
// ALDHANI-BEGIN: the Eye of Aldhani's field, the same text in gfx/aldhani.wgsl (which makes it),
// terrain.wgsl (the sky draws it) and gfx/shadow_rays.wgsl (the terrain is lit by it); a test keeps
// them equal.
// The slots a frame looks back over (one meteor at most each), the sky's tiles a side (a square
// folded over the upper half of the sky, `aldhani_tile`) and the streaks a tile holds at most.
const ALDHANI_METEORS: u32 = 327680u;
const ALDHANI_TILES: u32 = 64u;
const ALDHANI_TILE_CAP: u32 = 1024u;
// The burning meteors' glow by tile, red, green and blue, in `tiles` after the lists: summed by the
// field (fixed point, ALDHANI_GLOW_UNIT a paper white), then blurred over the sky (f32).
const ALDHANI_GLOW_SUM: u32 = 64u * 64u + 64u * 64u * 1024u;
const ALDHANI_GLOW_BLUR: u32 = ALDHANI_GLOW_SUM + 3u * 64u * 64u;
const ALDHANI_GLOW_UNIT: f32 = 1024.0;
// How far from its line a streak's light reaches at most, in radians (its glow fades to nothing
// there): a streak is binned into every tile within it.
const ALDHANI_REACH: f32 = 0.012;
// The event's seconds: the storm builds over a minute, holds, and dies out over the last minute.
const ALDHANI_EVENT: f32 = 300.0;
// Where the meteors go (the eye): a fixed direction of the world, due south eight degrees under
// the horizon, far beyond it. They fly away from the observer toward it, their streaks converging
// on it as they shrink into the haze beyond the mountains (their radiant, the opposite point, is
// eight degrees over the northern horizon, behind the observer facing the eye).
const ALDHANI_RADIANT: vec3<f32> = vec3<f32>(0.0, -0.1392, 0.99027);
// The light the field gives a face turned to it at the storm's height, in full moons (the moon's
// light overhead, `rt.held.z`), and the glow of the sky around the radiant at its brightest, in
// paper whites (radiance; Lambert's 1/pi in the units, as the sky table's).
const ALDHANI_MOONS: f32 = 16.0;
const ALDHANI_GLOW: f32 = 0.2;
// A meteor of the field: toward its streak's tail (xyz) and its core's width in pixels (w); toward
// its head (xyz) and its seed (w, for the sparkles); the light of its core at the head (rgb, paper
// whites) and its head's glow (w); the light it gives the terrain (rgb, luminance 1) and its share
// (w, the cdf's weight; 0: not burning).
struct AldhaniMeteor {
    tail: vec4<f32>,
    head: vec4<f32>,
    colour: vec4<f32>,
    light: vec4<f32>,
}
// x: the meteors' shares summed, y: the meteors burning, z: those drawn in the sky; the shares'
// running sum (`cdf[i]` the sum up to meteor i); each streak's great circle (its normal, xyz, 0
// for a point, and its angle, w), for the sky to pass a streak by from one read; and the meteors.
struct AldhaniField {
    total: vec4<f32>,
    cdf: array<f32, ALDHANI_METEORS>,
    lines: array<vec4<f32>, ALDHANI_METEORS>,
    meteors: array<AldhaniMeteor, ALDHANI_METEORS>,
}
// A hash of `n` to [0, 1) (a PCG round, as terrain.wgsl's `meteor_hash`).
fn aldhani_rand(n: u32) -> f32 {
    let state = n * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return f32(((word >> 22u) ^ word) >> 8u) / 16777216.0;
}
// How strong the storm is `t` seconds into the event (0 to 1; 0 before and after it).
fn aldhani_envelope(t: f32) -> f32 {
    return smoothstep(0.0, 60.0, t) * (1.0 - smoothstep(ALDHANI_EVENT - 60.0, ALDHANI_EVENT, t));
}
// The sky tile direction `d` falls in: the upper half of the sky folded flat (an octahedron's top,
// turned to a square), ALDHANI_TILES a side, the horizon on the square's edges.
fn aldhani_tile(d: vec3<f32>) -> u32 {
    let c = min(vec2<u32>(aldhani_square(d) * f32(ALDHANI_TILES)), vec2(ALDHANI_TILES - 1u));
    return c.y * ALDHANI_TILES + c.x;
}
// Where direction `d` falls on that square, 0 to 1 a side.
fn aldhani_square(d: vec3<f32>) -> vec2<f32> {
    let up = vec3(d.x, max(d.y, 0.0), d.z);
    let p = up.xz / (abs(up.x) + up.y + abs(up.z) + 1e-6);
    return clamp(vec2(p.x + p.y, p.x - p.y) * 0.5 + 0.5, vec2(0.0), vec2(1.0));
}
// The colours of the show's frame, in light (ALDHANI_PALETTE), by patches of sky rather than
// meteor by meteor (neighbours there differ by some 10 degrees of hue), richer than the frame.
// `around` and `theta` are where a meteor lights up about the radiant (0 east, pi west). The cold
// ones there, nine in ten of all, from turquoise to cyan, drifting slowly across the sky, and in
// the sky's bluest patches blue (`u` a meteor's own draw, 0.5 the patch's own colour):
fn aldhani_cold(around: f32, theta: f32, u: f32) -> vec3<f32> {
    let drift = 0.5 + 0.35 * sin(1.9 * around + 4.0 * theta + 1.0) + 0.25 * sin(3.7 * around - 6.0 * theta);
    let h = clamp(drift + 0.2 * (u - 0.5), 0.0, 1.0);
    let cold = mix(vec3(0.0, 1.0, 0.6), vec3(0.0, 0.78, 1.0), smoothstep(0.2, 0.8, h));
    return mix(cold, vec3(0.25, 0.45, 1.0), smoothstep(0.88, 0.98, h));
}
// The warm ones there, gold and red:
fn aldhani_warm(around: f32, theta: f32) -> vec3<f32> {
    let red = smoothstep(0.55, 0.85, 0.5 + 0.5 * sin(6.0 * theta + 2.0 * around));
    return mix(vec3(1.0, 0.7, 0.12), vec3(1.0, 0.22, 0.12), red);
}
// And their share there: none on the east, a fifth on the far west (the right, facing the
// radiant), one in twenty of all.
fn aldhani_warmth(around: f32) -> f32 {
    let west = smoothstep(0.1, 1.0, -cos(around));
    return 0.2 * west * west;
}
// The sky's own light toward `d` during the event (`t` seconds in), in paper whites, beside the
// glow its meteors make as they burn (terrain.wgsl `aldhani_burning`, from the field): the haze
// they light, pale and the same everywhere, thicker toward the eye and the radiant, and the eye
// itself, a bright glow on the horizon around it, white-hot in its middle.
fn aldhani_glow(d: vec3<f32>, t: f32) -> vec3<f32> {
    let strength = aldhani_envelope(t);
    if strength <= 0.0 { return vec3(0.0); }
    let c = clamp(dot(d, ALDHANI_RADIANT), -1.0, 1.0);
    let theta = acos(c);
    let up = max(d.y, 0.0);
    let pale = vec3(0.72, 0.9, 0.9);
    let sky = pale * (1.2 * (0.35 + 0.65 * abs(c)) * (0.55 + 0.45 * exp(-up * 2.0)));
    let core = exp(-theta * theta / (2.0 * 0.08 * 0.08));
    let wide = exp(-theta / 0.25) * (0.4 + 0.6 * exp(-up / 0.15));
    let eye = pale * (8.0 * wide) + mix(pale, vec3(1.0), 0.6) * (60.0 * core);
    // Under the horizon the ground hides it (the rays see the ground there).
    let above = smoothstep(-0.06, 0.0, d.y);
    return (sky + eye) * (ALDHANI_GLOW * strength * above);
}
// ALDHANI-END
// The Eye of Aldhani's streaks toward `d` (`pixel` radians a pixel), in paper whites: the field's
// meteors whose streaks cross `d`'s tile of the sky (no more than ALDHANI_TILE_CAP, however many
// burn), each a thin core about a pixel wide with a soft glow, brighter toward its head.
fn aldhani_streaks(d: vec3<f32>, pixel: f32) -> vec3<f32> {
    if globals.rt_camera.w < 0.0 || d.y < -0.02 { return vec3(0.0); }
    let tile = aldhani_tile(d);
    let count = min(aldhani_tiles[tile], ALDHANI_TILE_CAP);
    var sum = vec3(0.0);
    for (var i = 0u; i < count; i++) {
        let k = aldhani_tiles[ALDHANI_TILES * ALDHANI_TILES + tile * ALDHANI_TILE_CAP + i];
        let line = aldhani_field.lines[k];
        if abs(dot(d, line.xyz)) > ALDHANI_REACH { continue; }
        sum += aldhani_streak(aldhani_field.meteors[k], line, d, pixel);
    }
    return sum;
}
// The glow the Eye's burning meteors make toward `d` (the field's blurred grid, aldhani.wgsl
// `cs_aldhani_glow`): it comes and goes with them, in their colours where they burn.
const ALDHANI_BURNING: f32 = 0.08;
fn aldhani_burning(d: vec3<f32>) -> vec3<f32> {
    if d.y < -0.05 { return vec3(0.0); }
    let q = aldhani_square(d) * f32(ALDHANI_TILES) - 0.5;
    let base = vec2<i32>(floor(q));
    let f = q - floor(q);
    var glow = vec3(0.0);
    for (var i = 0; i < 4; i++) {
        let o = vec2(i & 1, i >> 1u);
        let c = clamp(base + o, vec2(0), vec2(i32(ALDHANI_TILES) - 1));
        let tile = u32(c.y) * ALDHANI_TILES + u32(c.x);
        let w = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        glow += w * vec3(bitcast<f32>(aldhani_tiles[ALDHANI_GLOW_BLUR + tile]),
            bitcast<f32>(aldhani_tiles[ALDHANI_GLOW_BLUR + 4096u + tile]),
            bitcast<f32>(aldhani_tiles[ALDHANI_GLOW_BLUR + 8192u + tile]));
    }
    return glow * (ALDHANI_BURNING * smoothstep(-0.05, 0.0, d.y));
}
// One streak's light toward `d`, drawn as the shooting stars are (`meteor_streak`): Gaussian lines
// a pixel wide at least that keep their light, a sharp head brighter than its trail in a soft halo,
// the trail fading and widening behind it, whiter just behind the head and its colour turning to
// the ember of sodium further back, a bright one's faint green train drifting off its line, and
// the short bright ones (a fireball breaking up near the radiant) sparkling: a few points along it
// flare and fade, each in its own rhythm.
fn aldhani_streak(m: AldhaniMeteor, line: vec4<f32>, d: vec3<f32>, pixel: f32) -> vec3<f32> {
    let a = m.tail.xyz;
    let b = m.head.xyz;
    let span = line.w;
    var across = length(d - b);
    var off = across;
    var along = 1.0;
    if any(line.xyz != vec3(0.0)) {
        let phi = atan2(dot(d, cross(line.xyz, a)), dot(d, a));
        along = clamp(phi / span, 0.0, 1.0);
        off = dot(d, line.xyz);
        across = select(min(length(d - a), length(d - b)), abs(off), phi >= 0.0 && phi <= span);
    }
    if across > ALDHANI_REACH { return vec3(0.0); }
    // The width (a Gaussian's sigma): a pixel's at least, the eye's blur where pixels are smaller.
    let wide = m.tail.w;
    let sigma = max(pixel * 0.62 * wide, 0.00025 * wide);
    if across > sigma * 14.0 + 0.004 { return vec3(0.0); }
    let gain = 0.62 * pixel * wide / sigma;
    // Its light fades to nothing ALDHANI_REACH away (the tiles it is binned into).
    let fade = 1.0 - across / ALDHANI_REACH;
    let top = max(m.colour.r, max(m.colour.g, m.colour.b));
    // The trail: widening as it diffuses toward the tail, whiter just behind the head, the ember
    // further back.
    // A needle: thinnest at its tail, where its glow has faded, widest at its head.
    let behind = 1.0 - along;
    let st = sigma * (0.4 + 0.6 * along) * (1.0 + 0.3 * behind);
    var colour = mix(m.colour.rgb, vec3(top), 0.25 * smoothstep(0.8, 1.0, along));
    colour = mix(colour, vec3(1.0, 0.42, 0.18) * top, 0.3 * smoothstep(0.4, 1.0, behind));
    let trail = gain * exp(-across * across / (2.0 * st * st))
        + 0.03 * exp(-across * across / (32.0 * sigma * sigma)) * fade * fade;
    var light = colour * (trail * (0.15 + 0.85 * along * along));
    // The head: a point brighter than the trail in a halo of a few pixels.
    let h2 = dot(d - b, d - b);
    let halo = sigma * 3.0;
    let white = mix(m.colour.rgb, vec3(top), 0.3);
    // Who it is (`aldhani_make`): its seed, and its way across the sky at its head (along its
    // great circle).
    let seed = u32(floor(m.head.w)) & 4095u;
    let way = select(select(vec3(0.0), normalize(b - a), span > 1e-6), normalize(cross(line.xyz, b)),
        any(line.xyz != vec3(0.0)));
    let aside = cross(b, way);
    let rel = d - b;
    let fwd = dot(rel, way);
    let lat = dot(rel, aside);
    let hue = m.colour.rgb / max(top, 1e-4);
    // A big one's shock layer: grown large against the air's mean free path (a metre-sized body
    // below about 100 km; the millimetre grains stay free-molecular, no shock), it drives a bow
    // shock a seventh of its radius ahead (0.14 R; 0.78 R rho_air / rho_shock for a sphere) where
    // the air is compressed, dissociated and ionised and glows itself (N2+'s violet-blue, N, O,
    // CN, NO), radiating as the eighth power of the speed (convection only the third), so at a
    // meteor's 35 to 72 km/s it outshines all else. Seen through an eye or a lens it is a glare,
    // a Moffat profile (a real optic's, its wings far wider than a Gaussian's): tight ahead where
    // the shock stands off, drawn out behind where it wraps the body and its wake; white at its
    // heart, the metals' colour mixed with the ionised air's blue in its wings. It shields the
    // body while it flies: a cap of light just ahead of the head, wider than it is deep, as
    // bright as the meteor is (none once it has burnt out).
    let alpha = sigma * 3.0;
    let cap = fwd - 1.8 * sigma;
    let k = select(1.6, 0.55, cap > 0.0);
    let q = (cap * cap) / (alpha * alpha * k * k) + (lat * lat) / (alpha * alpha * 1.8);
    let wings = mix(hue, vec3(0.55, 0.7, 1.0), 0.45) * top;
    let shock = smoothstep(0.15, 1.0, m.colour.w) * 2.2;
    light += (mix(vec3(top), wings, smoothstep(0.0, 2.0, q)) * pow(1.0 + q, -2.5)
        + wings * (0.12 * pow(1.0 + q / 9.0, -1.5))) * (shock * fade);
    light += white * (m.colour.w * (gain * gain * 1.2 * exp(-h2 / (2.0 * sigma * sigma))
        + 0.25 * exp(-h2 / (2.0 * halo * halo))) * fade * fade);
    // The train of a bright one: faint, green (the oxygen line), wider, drifting a little off its
    // line as it ages toward the tail, uneven along its length; none at the tail's very end.
    if m.colour.w > 0.3 {
        let drift = (aldhani_rand(seed + 3u) - 0.5) * 6.0 * sigma * behind;
        let o = off - drift;
        let tw = sigma * (2.0 + 3.0 * behind);
        let wisp = 0.75 + 0.25 * sin(along * 40.0 + f32(seed));
        light += mix(vec3(0.25, 1.0, 0.4), hue, 0.4) * top
            * ((min(m.colour.w, 1.5) - 0.3) * 0.12 * (gain * sigma / tw) * exp(-o * o / (2.0 * tw * tw))
            * wisp * 4.0 * behind * along);
    }
    // A bright one sheds pieces as it goes (the ablation's fragmentation): each comes off at some
    // point of the streak, falls behind as the air slows it and drifts a little aside, and glows
    // in its own slow swell, white-hot near the head, cooling toward sodium's orange behind.
    let shed = smoothstep(0.3, 1.2, m.colour.w);
    if shed > 0.02 && across < sigma * 10.0 {
        for (var j = 0u; j < 5u; j++) {
            let u = aldhani_rand(seed * 8u + j);
            let v = aldhani_rand(seed * 8u + j + 40u);
            let at = 0.35 + 0.6 * u;
            let p = normalize(normalize(mix(a, b, at)) + aside * ((v - 0.5) * sigma * 5.0 * (1.0 - at)));
            let pulse = smoothstep(0.15, 0.9, 0.5 + 0.5 * sin(globals.rt_water.z * (0.5 + 0.8 * v) + 6.28 * u));
            let q2 = dot(d - p, d - p) / (2.4 * sigma * sigma);
            light += mix(white, vec3(1.0, 0.6, 0.25) * top, 0.6 * (1.0 - at)) * (shed * 1.2 * pulse * exp(-q2));
        }
    }
    return light;
}
// RT lighting: the real sun's disc toward `d`, its radiance limb-darkened (0.53 degrees across,
// times the Sun Size setting: the angular radius in `rt_sun_light.w`).
fn rt_sun_disc(d: vec3<f32>) -> vec3<f32> {
    let radius = globals.rt_sun_light.w;
    let edge = cos(radius);
    let c = dot(d, normalize(globals.rt_sun.xyz));
    if c < edge || globals.rt_sun.y < -0.01 { return vec3(0.0); }
    let r = clamp((1.0 - c) / (1.0 - edge), 0.0, 1.0);
    let limb = 1.0 - 0.6 * (1.0 - sqrt(1.0 - r));
    // The disc's radiance is its light over its solid angle: a hundred thousand times paper white
    // and more, kept under the half floats' range. Its glare in the bloom is what makes it look as
    // big as it does to the eye (the disc itself is half a degree, a hundredth of the view).
    let light = select(globals.rt_sun_light.rgb, vec3(0.0), globals.rt_mode.z > 0.5);
    return min(light * (limb / (3.14159265 * radius * radius)), vec3(30000.0));
}
// RT lighting: the eye's glare around the sun (by night the moon) toward `d`, what makes a disc
// half a degree across look as big as it does: the CIE glare spread function's near term, a
// luminance of 10 E / theta^3 at theta degrees from a source of illuminance E (here E the light a
// white face turned to it gets, times pi), faded out from 3 to 15 degrees. Stiles and Holladay's
// 10 E / theta^2, used before, is the whole veil a real eye adds over tens of degrees: painted
// into the scene, then thresholded into the bloom on top, it lit a white blob some 20 degrees
// across around the sun (in VR, where it fills the view, a "bloom de folie"). The bloom's own
// spread of the disc gives the wider halo.
fn rt_glare(d: vec3<f32>) -> vec3<f32> {
    let toward = normalize(select(globals.rt_sun.xyz, globals.rt_moon.xyz, globals.rt_mode.z > 0.5));
    if toward.y < -0.05 { return vec3(0.0); }
    let theta = degrees(acos(clamp(dot(d, toward), -1.0, 1.0)));
    let radius = degrees(globals.rt_sun_light.w);
    let at = max(theta, radius);
    let fade = 1.0 - smoothstep(3.0, 15.0, theta);
    // The moon's a thirtieth as much: under the night sky the eye sees it sharp, its glare would hide
    // the disc the night's exposure lifts.
    // (0.03: at a tenth its flat veil over the disc still took the maria's contrast.)
    let eye = select(1.0, 0.03, globals.rt_mode.z > 0.5);
    // By night none on the moon's own disc, rising over a radius and a half past its limb: the
    // glare is the eye's veil around a source, and laid flat over the disc (some two paper whites
    // against the disc's one) it was what turned the moon into a white ball without a map.
    let off = select(1.0, smoothstep(radius, 2.5 * radius, theta), globals.rt_mode.z > 0.5);
    return globals.rt_sun_light.rgb * (10.0 * 3.14159265 * eye / (at * at * at)) * fade * off
        * smoothstep(-0.05, 0.02, toward.y);
}
// RT lighting: the real moon's disc toward `d` (0.52 degrees across, times the Moon Size
// setting): Minecraft's phase lit from the sun's side by Lommel-Seeliger's law (the regolith
// scatters back, its limb as bright as its middle at full moon), its face NASA's map of it (the
// maria, the craters and their rays), the earthshine faint on its night side; about 12 paper
// whites at full moon (2500 nits).
fn rt_moon_disc(d: vec3<f32>) -> vec3<f32> {
    let m = normalize(globals.rt_moon.xyz);
    let radius = globals.rt_moon.w;
    let c = dot(d, m);
    if c < cos(radius) || m.y < -0.01 { return vec3(0.0); }
    // The disc's frame: across it (t, b) and toward the viewer.
    let t = normalize(select(cross(m, vec3(0.0, 1.0, 0.0)), vec3(1.0, 0.0, 0.0), abs(m.y) > 0.99));
    let b = cross(t, m);
    let uv = vec2(dot(d, t), dot(d, b)) / sin(radius);
    let r2 = dot(uv, uv);
    if r2 >= 1.0 { return vec3(0.0); }
    let n = vec3(uv, sqrt(1.0 - r2));
    // The phase: 0 full (the sun behind the viewer), 4 new; the lit limb toward the sun.
    let phase = globals.rt_stars.x * 0.78539816;
    let sun = normalize(globals.rt_sun.xyz);
    var side = vec2(dot(sun, t), dot(sun, b));
    side = select(vec2(1.0, 0.0), normalize(side), dot(side, side) > 1e-8);
    let l = vec3(side * sin(phase), cos(phase));
    let mu0 = max(dot(n, l), 0.0);
    let mu = max(n.z, 1e-3);
    let lit = 2.0 * mu0 / (mu0 + mu);
    // Its face from NASA's map (it keeps its near side to the world): longitude about the axis
    // across the disc, the map's mean on the near side (0.40) brought to the Moon's albedo, 0.12.
    let lon = atan2(n.x, n.z);
    let lat = asin(clamp(n.y, -1.0, 1.0));
    let map = textureSampleLevel(rt_moon_map, lightmap_sampler,
        vec2(0.5 + lon / 6.2831853, 0.5 - lat / 3.14159265), 0.0).rgb;
    let albedo = map * 0.3;
    // Below the horizon's air: reddened and dimmed as the sky table's light toward it is.
    let air = clamp(m.y * 8.0 + 0.2, 0.0, 1.0);
    // Its colour through the air: by night the light the rays trace toward is the moon's (the
    // sunlight the regolith reflects, through the air toward the moon, gfx.rs), so its hue is the
    // disc's, orange to red as the moon sets; by day (the moon in a lit sky) the regolith's alone.
    var tint = vec3(1.0, 0.93, 0.85);
    if globals.rt_mode.z > 0.5 {
        let light = globals.rt_sun_light.rgb;
        tint = light / max(dot(light, vec3(0.2126, 0.7152, 0.0722)), 1e-6);
    }
    // 3 paper whites at full moon, not its 12 (2500 nits): the eye adapts to the moon itself, the
    // tone mapping can't, and at 12 the night's exposure would wash its maria out.
    // The earthshine on its night side: the Earth's light (the Earth as seen from the Moon is in
    // the opposite phase: full at new moon), its surface some 8.5 magnitudes (1/2500) under the
    // lit side's at its brightest (the Big Bear Earthshine Project, about 12 against 3.5 mag per
    // square arcsecond).
    let earth_lit = 0.5 - 0.5 * cos(phase);
    // By night 1 paper white, not the day's 3: at 3 its highlands stood at 4.5 and its maria at
    // 1.5, both past the tone curve's knee (0.9) under the night's exposure (0.7), and the disc
    // came out one white, its map gone. At 1 the maria sit near a third and the highlands at
    // white. Through the dusk, with the sun's height, as the exposure's top comes down.
    let full = mix(1.0, 3.0, smoothstep(-0.1, 0.1, globals.rt_sun.y));
    let earthshine = full * 4e-4 * earth_lit;
    return tint * (albedo / 0.12) * (full * lit + earthshine) * air;
}
// RT lighting: the light sources' traced light at this pixel (`rt_blocks`), this frame's rays only.
// RT lighting: the sun's colour through stained glass at this pixel (`rt_sun_tint`), divided by the
// visibility the mask already holds; white without this frame's rays.
fn rt_sun_colour(pixel: vec2<f32>, visible: f32) -> vec3<f32> {
    if globals.rt_motion.w < 1.5 { return vec3(1.0); }
    // Read as the visibility was (`rt_taps`).
    if rt_taps_ready { return rt_mask(rt_sun_tint, pixel, vec3(0.0)).rgb / max(visible, 1e-3); }
    let size = vec2<f32>(textureDimensions(rt_sun_tint));
    let at = clamp(vec2<i32>(pixel / globals.rt_motion.xy * size), vec2(0), vec2<i32>(size) - 1);
    return textureLoad(rt_sun_tint, at, 0).rgb / max(visible, 1e-3);
}
fn rt_traced(pixel: vec2<f32>) -> vec4<f32> {
    if globals.rt_motion.w < 1.5 { return vec4(0.0, 0.0, 0.0, -1.0); }
    if rt_taps_ready { return rt_mask(rt_blocks, pixel, vec3(0.0)); }
    let size = vec2<f32>(textureDimensions(rt_blocks));
    let at = clamp(vec2<i32>(pixel / globals.rt_motion.xy * size), vec2(0), vec2<i32>(size) - 1);
    return textureLoad(rt_blocks, at, 0);
}
// RT lighting's model: physical light, in linear light and multiples of paper white, on a surface's
// material (`RtMaterial`) from the rays. Nothing of vanilla's lightmap, its minimum brightness or
// its shading: a closed tunnel with no light in it is black. Light bouncing off surfaces (global
// illumination) is to come (docs/raytracing.md); until then a shadow is lit by the sky alone.
// - The sun (by night the moon): its colour where the rays reach, on the material's normal, as
//   diffuse light and a GGX specular highlight (Fresnel, Smith visibility); through leaves and
//   plants by their subsurface scattering.
// - The sky: its colour as much as the face sees of it, by the rays fanned toward it (z of `v`),
//   past the grid by the terrain's horizon around (shadow_rays.wgsl `far_sky`); where neither
//   got there, the sky light level squared from a dome (all of it on tops, 60 % on sides, a
//   quarter beneath); diffuse, and reflected as much as Fresnel says.
// - Light sources (torches, lava, glowstone): traced, each its colour falling off as the inverse
//   square of the distance with a shadow ray toward it (`traced`, shadow_rays.wgsl `block_light`);
//   past the grid or on a frame without this frame's rays, warm light from the block light level
//   (one level a block). A metal takes them through its specular lobe (it has no diffuse light).
// - The material's own emission (LabPBR's), added.
const BLOCK_LIGHT: vec3<f32> = vec3(3.0, 1.9, 0.9);
// A LabPBR emission of 1, in paper whites.
const MATERIAL_EMISSION: f32 = 4.0;
const RT_PI: f32 = 3.14159265;
// A surface as RT lighting lights it: its albedo (linear), its shading normal (a normal map's, or
// the face's), its own occlusion, its roughness (GGX alpha's square root), its reflectance at normal
// incidence, whether it's a metal, how much light scatters through it, and its emission.
struct RtMaterial {
    albedo: vec3<f32>,
    normal: vec3<f32>,
    ao: f32,
    roughness: f32,
    f0: vec3<f32>,
    metal: f32,
    subsurface: f32,
    emission: f32,
}
// A face without maps: rough, a dielectric's 4 % reflectance.
fn rt_plain(albedo: vec3<f32>, n: vec3<f32>) -> RtMaterial {
    return RtMaterial(decode_gamma(albedo), n, 1.0, 1.0, vec3(0.04), 0.0, 0.0, 0.0);
}
// A LabPBR specular texel `s` on albedo `albedo` (linear): red the perceptual smoothness, green the
// reflectance (230 and up a metal, its albedo then the reflectance), blue porosity up to 64 then
// subsurface scattering, alpha the emission (255 none).
fn rt_labpbr(albedo: vec3<f32>, n: vec3<f32>, ao: f32, s: vec4<f32>) -> RtMaterial {
    let g = round(s.g * 255.0);
    let metal = select(0.0, 1.0, g >= 230.0);
    let f0 = select(vec3(s.g), albedo, g >= 230.0);
    let b = round(s.b * 255.0);
    let subsurface = select(0.0, (b - 65.0) / 190.0, b >= 65.0);
    let a = round(s.a * 255.0);
    let emission = select(a / 254.0, 0.0, a >= 255.0);
    let roughness = max((1.0 - s.r) * (1.0 - s.r), 0.02);
    return RtMaterial(albedo, n, ao, roughness, f0, metal, subsurface, emission);
}
fn rt_fresnel(f0: vec3<f32>, cosine: f32) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(1.0 - clamp(cosine, 0.0, 1.0), 5.0);
}
// The sky light reaching a face by its sky light level (`light.x`, 0 to 15 as the vertex gives it:
// `terrain_vertex`, vanilla's lightmap coordinates over 16), where no ray says how much of the
// sky it sees: the level's share of full daylight, squared.
fn rt_sky_level(level: f32) -> f32 {
    return pow(clamp(level / 15.0, 0.0, 1.0), 2.0);
}
// RT lighting: how much of the sun's (or the moon's) light the clouds let through to the
// camera-relative `pos` (gfx/rt_clouds.rs `shadow`), softened by the penumbra the light's disc
// casts from as high as the clouds are: a few blocks under the cumulus at noon, tens at sunset.
const CLOUD_SHADOW_BASE: f32 = 800.0;
const CLOUD_SHADOW_SIDE: f32 = 4096.0;
fn rt_cloud_shadow(pos: vec3<f32>) -> f32 {
    let l = globals.rt_light.xyz;
    if l.y <= 0.03 || textureDimensions(rt_cloud_shadows).x < 2u { return 1.0; }
    var world = globals.rt_camera.xyz + pos;
    // The camera's height is wrapped too: below the bedrock's floor it comes back near the top.
    if globals.rt_camera.y > 3500.0 { world.y -= 4096.0; }
    let rise = max(CLOUD_SHADOW_BASE - world.y, 0.0);
    let q = world.xz + l.xz / l.y * rise;
    // The light's disc seen from the point spans this much at the clouds' middle (a few hundred
    // blocks into the slab), never under a texel.
    let reach = (rise + 400.0) / l.y;
    let radius = max(reach * globals.rt_sun_light.w, 4.0) / CLOUD_SHADOW_SIDE;
    let uv = q / CLOUD_SHADOW_SIDE;
    var sum = textureSampleLevel(rt_cloud_shadows, rt_cloud_sampler, uv, 0.0).r * 0.28;
    for (var i = 0; i < 6; i++) {
        let a = f32(i) * 1.0471976 + 0.3;
        let o = vec2(cos(a), sin(a)) * radius;
        sum += textureSampleLevel(rt_cloud_shadows, rt_cloud_sampler, uv + o, 0.0).r * 0.12;
    }
    // The map holds the 4096 blocks around where the camera's own column meets the clouds: past
    // 2 km from there it wraps, so the shadows fade out first.
    let camera_y = select(globals.rt_camera.y, globals.rt_camera.y - 4096.0, globals.rt_camera.y > 3500.0);
    let center = globals.rt_camera.xz + l.xz / l.y * max(CLOUD_SHADOW_BASE - camera_y, 0.0);
    return mix(sum, 1.0, smoothstep(1500.0, 1950.0, length(q - center)));
}
// The material `m` (on a face of normal `geo`, camera-relative `pos`) lit, before the fog; `light`
// its sky and block light levels (0 to 15), `v` the rays' visibility, `traced` the light sources'.
const FIREFLY_LUMINANCE: f32 = 5.0;
// HORIZON_RT_DEBUG=14 (`rt_terms_view`): the last `rt_shade`'s sun (with its highlight), its light
// around, how much of that is traced (`traced`'s alpha), its reflection; luminances.
var<private> rt_terms: vec4<f32>;
fn rt_shade(m: RtMaterial, geo: vec3<f32>, pos: vec3<f32>, light: vec2<f32>, v: vec3<f32>,
    traced: vec4<f32>, tint: vec3<f32>) -> vec3<f32> {
    let n = m.normal;
    let view = -normalize(pos);
    let nv = max(dot(n, view), 1e-3);
    let l = globals.rt_light.xyz;
    // The real sun's light through the atmosphere, or the moon's (gfx/atmosphere.rs).
    let sun = globals.rt_sun_light.rgb * globals.rt_light.w * v.x * tint * rt_relief_sun
        * rt_cloud_shadow(pos);
    let diffuse = m.albedo * (1.0 - m.metal);
    // The sun: lit where the face (not only its normal map) turns to it.
    let gate = smoothstep(0.0, 0.15, dot(geo, l));
    // Micro-shadowing (Chan 2018): a facet of the normal map gets little more light than the face
    // it is on, its neighbours' ridges shadowing it when the light grazes the face, so a low sun
    // doesn't pick out each groove's sunward wall as a bright line.
    let nl = min(max(dot(n, l), 0.0), 1.5 * max(dot(geo, l), 0.0) + 0.05);
    let h = normalize(l + view);
    let nh = max(dot(n, h), 0.0);
    let alpha = m.roughness * m.roughness;
    let a2 = alpha * alpha;
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let distribution = a2 / (RT_PI * dd * dd);
    let k = alpha * 0.5;
    let visibility = 0.25 / ((nl * (1.0 - k) + k) * (nv * (1.0 - k) + k));
    let fresnel = rt_fresnel(m.f0, dot(h, view));
    // Paper white is the light a white diffuse surface facing the sun gives back: the 1/pi of a
    // Lambertian surface is in the units, the specular lobe takes it back.
    let specular = fresnel * (distribution * visibility * RT_PI);
    let direct = diffuse * (1.0 - fresnel) * nl * gate;
    // The sun's highlight, kept under Remix's firefly filter (`fireflyFilteringLuminanceThreshold`
    // 1000 in its units, about five paper whites): a normal map's facet catching the low sun
    // would otherwise glitter pixel by pixel.
    let glint = sun * specular * (nl * gate);
    let glint_luminance = dot(glint, vec3(0.2126, 0.7152, 0.0722));
    let highlight = glint * min(1.0, FIREFLY_LUMINANCE / max(glint_luminance, 1e-6));
    // Through leaves and plants: the light from behind, a third of it out the other side, filtered
    // by the leaf on its way in and out (its colour twice: a deeper green than its face's).
    let through = diffuse * m.albedo * m.subsurface * 0.35 * max(dot(-geo, l), 0.0)
        * (1.0 - gate);
    // The sky and the light sources: traced (`traced`, the sky's light seen through rays in every
    // direction, accumulated), or past the grid from the light levels: the sky from where the face
    // sees most of it, between its normal and the zenith, as much as its sky light level says,
    // the light sources by the inverse square of the distance the block light level stands for.
    let sky_level = rt_sky_level(light.x);
    let dome = select(mix(0.6, 1.0, n.y), mix(0.6, 0.25, -n.y), n.y < 0.0);
    let seen = select(sky_level * dome, v.z, v.z >= 0.0);
    let open = v.y * m.ao;
    let steps = max(15.0 - light.y, 0.0);
    let level = select(vec3(0.0), BLOCK_LIGHT / ((1.0 + steps) * (1.0 + steps)), light.y > 0.0);
    // And the ambient light (the RT Ambient Light option, `globals.earth.w`'s fraction), which the
    // traced light holds already (shadow_rays.wgsl `rt.ambient_light`): past the grid, in its edge
    // band and on a frame without rays the terrain had none, as the entities have it everywhere.
    let by_level = (rt_sky(normalize(n + vec3(0.0, 1.0, 0.0))) * seen + level) * open
        + vec3(fract(globals.earth.w)) * m.ao;
    let around = mix(by_level, traced.rgb * m.ao, clamp(traced.a, 0.0, 1.0));
    // Reflected: the sky the way the view bounces off the surface, as much as Fresnel says (the
    // rougher, the less of it).
    let ambient_f = rt_fresnel(m.f0, nv) * (1.0 - m.roughness * 0.7);
    // The reflection fades as it dips under the face (horizon fading): a normal map's facet
    // turned away would otherwise reflect the bright horizon from inside the surface, the walls
    // of a plank's grooves glittering at sunset.
    let r = reflect(-view, n);
    let horizon = clamp(1.0 + 1.1 * dot(r, geo), 0.0, 1.0);
    var reflected = rt_sky(r) * seen * m.ao * ambient_f * (horizon * horizon);
    // Water and glass: what the rays found that way (Remix's primary surface replacement).
    if rt_mirror.a > 0.5 { reflected = rt_mirror.rgb * ambient_f; }
    // A metal has no diffuse light to take the light sources' (`around`: the torches, lava,
    // glowstone and the held lights, traced and denoised, with the sky's and the bounced light):
    // its specular lobe gathers them instead. The rougher, the wider the lobe, the more it reflects
    // the light around as a whole (a rough metal's reflection is its Fresnel times the light it
    // receives, as a diffuse surface's is its albedo times it); a polished one keeps more of the
    // sky's mirror image, but still a third of the light around, so a torch beside a gold block
    // lights it up at night.
    let lobe = mix(0.35, 1.0, m.roughness) * m.metal;
    reflected = mix(reflected, rt_fresnel(m.f0, nv) * around, lobe);
    // The rain's impacts (`rt_spray`): their broken crests scatter the sky's light.
    let spray = rt_sky(vec3(0.0, 1.0, 0.0)) * seen * (RAIN_SPRAY * rt_spray);
    if globals.rt_mode.y > 6.5 && globals.rt_mode.y < 7.5 {
        let y = vec3(0.2126, 0.7152, 0.0722);
        rt_terms = vec4(dot(sun * (direct + through) + highlight, y),
            dot(diffuse * (1.0 - ambient_f) * around, y), traced.a, dot(reflected, y));
    }
    // The snow's crystals that catch the sun toward the eye (`rt_snow`).
    let sparkle = sun * (rt_sparkle * gate);
    return sun * (direct + through) + highlight + sparkle + diffuse * (1.0 - ambient_f) * around
        + reflected
        + m.albedo * m.emission * MATERIAL_EMISSION + spray;
}
// No real surface is darker than charcoal or soot, which give back 2 to 4 % of the light, but a
// texture's near-black texels (sculk's body, deepslate's joints) decode to a few thousandths: no
// lamp showed them, and sculk was its specks on black. A dark albedo is raised toward
// `ALBEDO_FLOOR`, four times at most so its colour and the texture's pattern stay.
const ALBEDO_FLOOR: f32 = 0.02;
fn rt_albedo_floor(albedo: vec3<f32>) -> vec3<f32> {
    let y = dot(albedo, vec3(0.2126, 0.7152, 0.0722));
    return max(albedo * clamp(ALBEDO_FLOOR / max(y, 1e-5), 1.0, 4.0), vec3(0.25 * ALBEDO_FLOOR));
}
// RT lighting: material `m`'s linear colour, lit by the model and fogged. The fog scatters the
// light it gets: the sky's, as much as the surface sees of it (a sealed room's air carries none).
fn rt_material_lit(m_in: RtMaterial, geo: vec3<f32>, pos: vec3<f32>, pixel: vec2<f32>,
    light: vec2<f32>, spherical: f32, cylindrical: f32) -> vec3<f32> {
    var m = m_in;
    m.albedo = rt_albedo_floor(m.albedo);
    let v = rt_visibility(pos, pixel);
    rt_mirror = rt_reflection(pixel, pos);
    // A puddle: what the rays found it reflects.
    if rt_puddle > 0.0 {
        rt_mirror = rt_puddle_reflection(pixel);
        // HORIZON_RT_DEBUG=9: the puddles' traced reflection alone (magenta: none, the sky's).
        if globals.rt_mode.y > 2.5 && globals.rt_mode.y < 3.5 {
            return select(vec3(1.0, 0.0, 1.0), rt_mirror.rgb, rt_mirror.a > 0.5);
        }
    }
    let lit = rt_shade(m, geo, pos, light, v, rt_traced(pixel), rt_sun_colour(pixel, v.x));
    let f = rt_fade(spherical, cylindrical, pos) * globals.fog_color.a;
    let sky = select(rt_sky_level(light.x), v.z, v.z >= 0.0);
    // Aerial perspective: the air between the eye and the surface takes some of its light away
    // (blue the most, Rayleigh's) and scatters the sky's toward the eye, so far terrain fades into
    // the horizon's colour that way and loses its contrast. The air's extinction near the ground
    // (gfx/atmosphere.rs), times the Aerial Perspective setting (`rt_mode.w`); what it scatters in
    // tends to the horizon's sky as the distance grows, as much as the surface's air sees the sky.
    let toward = normalize(vec3(pos.x, 0.0, pos.z) + vec3(0.0, 0.05 * length(pos.xz), 0.0));
    let horizon = rt_sky(toward);
    let air = vec3(5.802e-3 + 4.44e-3, 13.558e-3 + 4.44e-3, 33.1e-3 + 4.44e-3);
    let through = exp(-air * (length(pos) * 0.001 * globals.rt_mode.w));
    let seen = lit * through + horizon * sky * (1.0 - through);
    // Past the loaded world's edge (vanilla's distance fog), the horizon alone.
    return mix(seen, horizon * sky, f);
}
// RT lighting's water (terrain.wgsl `rt_water`, shadow_rays_hw.wgsl `hw_tint` and `hw_reflect`): the
// surface's waves at world `p` (x, z) and `t` seconds, a sum of sixteen plane waves of every
// length from a long swell (6 m) down to capillary ripples (21 cm), their directions turning by
// the golden angle so no two line up, each travelling at its water-wave speed (gravity and surface
// tension). Every wave is the same slope's worth of sine whatever its length, capped so the
// finest ones keep their curvature (what focuses light) without roughening the glints: the height,
// its slopes and its curvature (the second derivatives along x, xz and z) are exact sums, the
// same in the shader that shows the water, the one that traces what it reflects and refracts, and
// the one that traces the sun's light through it (these three copies are identical).
struct WaterSurface {
    height: f32,
    slope: vec2<f32>,
    curvature: vec3<f32>,
}
// One plane wave: `d` its direction (x, z), its wavenumber (rad/m) and its slope's amplitude; `w`
// its angular frequency and its phase. The sixteen below are the law above written out, the
// directions turning 2.39996 rad (the golden angle) a wave, the wavenumbers 1.04 rad/m growing
// by 1.25 a wave (6 m to 21 cm), the slope min(1 / k, 0.06 min(1, k / 2.4)), the frequency
// 0.6 sqrt(9.81 k + 7.3e-5 k^3) (gravity and surface tension, blocks being metres; slowed to
// 0.6, a calmer sea than dispersion alone gives), the phase 2.1 a wave. Written out, no shader has to work them out: each costs a sine and a cosine.
fn water_wave(p: vec2<f32>, t: f32, footprint: f32, d: vec4<f32>, w: vec2<f32>) -> WaterSurface {
    // A ripple finer than about a pixel only shimmers: it fades out, and costs nothing then.
    let fade = 1.0 - smoothstep(0.75, 2.0, d.z * footprint);
    if fade <= 0.0 { return WaterSurface(0.0, vec2(0.0), vec3(0.0)); }
    let slope = d.w * fade;
    let phase = d.z * dot(d.xy, p) - w.x * t + w.y;
    let s = sin(phase);
    let dir = d.xy;
    return WaterSurface(slope / d.z * s, slope * cos(phase) * dir,
        -slope * d.z * s * vec3(dir.x * dir.x, dir.x * dir.y, dir.y * dir.y));
}
fn water_sum(a: WaterSurface, b: WaterSurface) -> WaterSurface {
    return WaterSurface(a.height + b.height, a.slope + b.slope, a.curvature + b.curvature);
}
// The sea state varies over the water, as wind and fetch make it vary on a real lake: three gains
// on the waves' slopes, smooth over the swell's 230 m, the wind sea's 70 m and the ripples' 23 m
// (the cat's paws, gusts drifting with the wind), about a mean of 1 (0.4 to 1.6, 0.15 to 1.85), from
// a value noise of the world position (no period: a lake 16 km across never shows the same water
// twice). The gains change slowly enough for their own slopes to be neglected.
fn water_hash(c: vec2<i32>) -> f32 {
    var h = (u32(c.x) * 0x8da6b343u) ^ (u32(c.y) * 0xd8163841u);
    h = (h ^ (h >> 15u)) * 0x2c1b3c6du;
    h = (h ^ (h >> 12u)) * 0x297a2d39u;
    return f32((h ^ (h >> 15u)) & 0xffffu) / 65535.0;
}
fn water_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    return mix(mix(water_hash(c), water_hash(c + vec2(1, 0)), u.x),
        mix(water_hash(c + vec2(0, 1)), water_hash(c + vec2(1, 1)), u.x), u.y);
}
// `sea` is how open the water lies (terrain.wgsl `TerrainOut::sea`: 0 a swamp's still water, 0.6 a
// lake's, 1 the open sea's): the swell and the wind sea grow with it, still water hardly moves.
const WATER_CALM: f32 = 0.6;
fn water_sea(q: vec2<f32>, t: f32, sea: f32) -> vec3<f32> {
    let still = mix(vec3(0.1, 0.25, 0.4), vec3(1.0), smoothstep(0.0, 0.6, sea));
    let open = mix(vec3(1.0), vec3(1.8, 1.3, 1.1), smoothstep(0.6, 1.0, sea));
    let swell = water_noise(q / 233.0 + vec2(0.003, 0.001) * t);
    let wind = water_noise(q / 71.0 + vec2(17.3, 5.1) + vec2(0.006, -0.004) * t);
    let gusts = water_noise(q / 23.0 + vec2(-3.7, 11.9) + vec2(0.045, 0.02) * t);
    // The wind's gains contrast more than the swell's: calm slicks beside rippled patches.
    let patches = smoothstep(vec2(0.15), vec2(0.85), vec2(wind, gusts));
    // All of it at six tenths (`WATER_CALM`): at full height the surface was a dense mesh of
    // glints and its caustics a net of hard lines on the bed, a pond's as a gale's.
    return vec3(0.4 + 1.2 * swell, 0.15 + 1.7 * patches) * still * open * WATER_CALM;
}
fn water_surface(q: vec2<f32>, t: f32, footprint: f32, sea: f32) -> WaterSurface {
    var s = WaterSurface(0.0, vec2(0.0), vec3(0.0));
    // The crests are not straight lines: a slow swirl bends the coordinates every wave is read at,
    // and a noise over a hundred metres and thirty turns their lines on the scale of a lake, so
    // they never run parallel far (its own slopes are neglected: it turns them by a few degrees).
    let swirl = 0.6 * vec2(sin(0.31 * q.y + 0.23 * t + 1.3), sin(0.27 * q.x - 0.19 * t));
    let bend = 12.0 * (vec2(water_noise(q / 97.0 + vec2(31.1, 7.7)),
        water_noise(q / 97.0 + vec2(-12.4, 23.5))) - 0.5)
        + 3.6 * (vec2(water_noise(q / 29.0 + vec2(5.3, -19.2)),
        water_noise(q / 29.0 + vec2(40.6, 2.9))) - 0.5);
    let p = q + swirl + bend;
    // The sea state there: the swell's, the wind sea's and the ripples' gains (`water_sea`).
    let g = water_sea(q, t, sea);
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.92106, 0.38942, 1.0400, 0.02600 * g.x), vec2(1.9165, 0.0)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.94221, 0.33503, 1.3000, 0.03250 * g.x), vec2(2.1427, 2.1)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.46845, -0.88349, 1.6250, 0.04063 * g.x), vec2(2.3956, 4.2)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.25138, 0.96789, 2.0312, 0.05078 * g.x), vec2(2.6784, 6.3)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.83916, -0.54389, 2.5391, 0.06000 * g.x), vec2(2.9946, 8.4)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.98616, -0.16580, 3.1738, 0.06000 * g.y), vec2(3.3481, 10.5)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.61516, 0.78840, 3.9673, 0.06000 * g.y), vec2(3.7433, 12.6)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.07896, -0.99688, 4.9591, 0.06000 * g.y), vec2(4.1853, 14.7)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.73160, 0.68173, 6.1989, 0.06000 * g.y), vec2(4.6796, 16.8)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.99996, -0.00849, 7.7486, 0.06000 * g.y), vec2(5.2323, 18.9)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.74308, -0.66921, 9.6858, 0.06000 * g.y), vec2(5.8506, 21.0)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.09587, 0.99539, 12.1072, 0.06000 * g.z), vec2(6.5425, 23.1)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.60169, -0.79873, 15.1340, 0.06000 * g.z), vec2(7.3170, 25.2)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.98320, 0.18252, 18.9175, 0.05286 * g.z), vec2(8.1846, 27.3)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.84827, 0.52956, 23.6469, 0.04229 * g.z), vec2(9.1574, 29.4)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.26778, -0.96348, 29.5586, 0.03383 * g.z), vec2(10.2503, 31.5)));
    return s;
}
// The height and the slopes along x and z, for the shaders that only look at the surface.
fn water_waves(p: vec2<f32>, t: f32, footprint: f32, sea: f32) -> vec3<f32> {
    let w = water_surface(p, t, footprint, sea);
    return vec3(w.height, w.slope);
}
// How wide a pixel at camera-relative `pos` lies on water of face `face`, in metres: its width at
// that distance, stretched along the view by the slant (a far lake seen almost edge on covers tens
// of metres a pixel that way), so the waves fade out before they alias into shimmer.
fn water_footprint(pos: vec3<f32>, face: vec3<f32>) -> f32 {
    let d = length(pos);
    let slant = abs(dot(pos, face)) / max(d, 1e-4);
    return d * 1.5 / globals.rt_motion.y / max(slant, 0.03);
}
// The slope variance (both axes summed) of the waves `water_surface` fades out at `footprint`: what
// the pixel still sees of them, as roughness (Toksvig's, a LEAN map's variance), so far water turns
// into a soft, rough mirror (wider glints, a softer Fresnel) and keeps the light the ripples
// scattered instead of dropping it. The same wavenumbers and slopes as the sixteen waves, in the
// sea state `sea` (`water_sea`).
fn water_lost_slope(footprint: f32, sea: vec3<f32>) -> f32 {
    var lost = 0.0;
    var k = 1.04;
    for (var i = 0; i < 16; i++) {
        let a = min(1.0 / k, 0.06 * min(1.0, k / 2.4))
            * select(select(sea.z, sea.y, i < 11), sea.x, i < 5);
        let fade = 1.0 - smoothstep(0.75, 2.0, k * footprint);
        lost += 0.5 * a * a * (1.0 - fade * fade);
        k *= 1.25;
    }
    return lost;
}
// Pure water's absorption a block (a metre), red most: deep water turns blue-green.
// Remix's water medium (its minecraft-logic confs): transmittance 0.6, 0.9, 0.89 over 20 m (a
// block a metre), so its extinction a block, -ln(T) / 20; the biome's water colour adds to it
// what that colour lacks (a swamp's murk), and colours the light the water scatters.
const WATER_ABSORPTION: vec3<f32> = vec3(0.025541, 0.005268, 0.005827);
const WATER_TINT_ABSORPTION: f32 = 0.06;
// The share of the light the water body scatters back toward the eye (its glow), with Remix's
// single-scattering albedo: 0.15 left open water ink navy save for the glints.
const WATER_GLOW: f32 = 0.3;
// What real water adds to Remix's clear medium: pure water's absorption a metre (Smith and Baker)
// at red, green and blue, which turns a shallow bed turquoise and deep water night blue.
const WATER_PURE_ABSORPTION: vec3<f32> = vec3(0.30, 0.045, 0.012);
// RT lighting's water, its own look: no texture (only the biome's colour), waves for its normal;
// the sky and the sun
// reflected as Fresnel says (a glint where the sun is); what lies under it seen through its
// depth, the light lost to the water's absorption (red first) and the water's own blue-green
// glow scattered toward the eye in its place. The depth: from the surface to the solid terrain
// behind it at this pixel (`rt_depth`). Then the air in front, as for the terrain.
fn rt_is_lava(layer: u32) -> bool {
    let packed = globals.rt_water.w;
    if packed < 0.0 { return false; }
    let l = f32(layer & 0x7fffu);
    let flow = floor(packed / 4096.0);
    return l == packed - flow * 4096.0 || l == flow;
}
// RT lighting's lava, its own look (no texture): molten rock flowing under a cooling crust. A
// heat field of domain-warped noise drifting with the time, bright veins where
// it cracks; the hot rock glows as a black body (dark red to yellow-white, in paper whites, to
// bloom), the cooled crust is dark basalt lit as the terrain is.
// A lattice point's random number, from its integer coordinates (PCG3D, Jarzynski and Olano
// 2020): exact whatever the compiler does with floats, so neighbouring cells agree.
fn lava_hash(p: vec3<f32>) -> f32 {
    var v = bitcast<vec3<u32>>(vec3<i32>(p)) * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return f32(v.x >> 8u) / 16777216.0;
}
fn lava_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(lava_hash(i), lava_hash(i + vec3(1.0, 0.0, 0.0)), u.x),
        mix(lava_hash(i + vec3(0.0, 1.0, 0.0)), lava_hash(i + vec3(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(lava_hash(i + vec3(0.0, 0.0, 1.0)), lava_hash(i + vec3(1.0, 0.0, 1.0)), u.x),
        mix(lava_hash(i + vec3(0.0, 1.0, 1.0)), lava_hash(i + vec3(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}
fn lava_fbm(p: vec3<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        sum += amp * lava_noise(q);
        q = q * 2.03 + vec3(1.7, 9.2, 4.1);
        amp *= 0.5;
    }
    return sum;
}
// The black body's colour from cooler (0) to hotter (1), linear.
fn lava_blackbody(x: f32) -> vec3<f32> {
    let deep = vec3(0.45, 0.015, 0.0);
    let orange = vec3(1.0, 0.22, 0.015);
    let white = vec3(1.0, 0.72, 0.32);
    return select(mix(orange, white, (x - 0.6) / 0.4), mix(deep, orange, x / 0.6), x < 0.6);
}
const LAVA_GLOW: f32 = 14.0;
// The lava's heat (0 cooled, 1 white-hot) at noise point `p`: domain-warped noise, bright veins
// where it cracks.
fn lava_heat(p: vec3<f32>) -> f32 {
    let warp = vec3(lava_fbm(p), lava_fbm(p + vec3(5.2, 1.3, 2.7)), lava_fbm(p + vec3(2.9, 7.1, 4.4)));
    let q = p + 2.2 * warp;
    let body = lava_fbm(q);
    let crack = 1.0 - abs(lava_fbm(q * 2.1 + vec3(3.1, 7.7, 0.0)) * 2.0 - 1.0);
    return clamp(0.95 * body + 0.7 * pow(crack, 7.0) - 0.15, 0.0, 1.0);
}
// The lava's look at world `world` on a face of normal `n` (any sign), `t` seconds into the
// water's clock: rgb the light its hot rock gives off (paper whites), w how much of it is cooled
// crust (dark basalt, `LAVA_ROCK`, lit as the terrain is). Triplanar: on the top it drifts on the
// world's plane, on a side (or a fall) it runs down, stretched as falling lava streams; a slope
// blends them by its normal, so no face sees the top's pattern smeared along it.
// The one source of the lava's look: the terrain's shader draws it (`rt_lava`), and the rays
// that meet lava (seen through water or glass, in a mirror: shadow_rays_hw.wgsl `hw_surface`)
// take this same text (gfx/shadow_rays.rs `LAVA_LOOK`), not the atlas's old sprite.
fn lava_look(world: vec3<f32>, n: vec3<f32>, t: f32) -> vec4<f32> {
    var w = pow(abs(n), vec3(4.0));
    w /= max(w.x + w.y + w.z, 1e-4);
    var heat = 0.0;
    if w.y > 0.01 {
        heat += w.y * lava_heat(vec3(world.x * 0.45, world.z * 0.45, t * 0.04));
    }
    let fall = (world.y + t * 1.2) * 0.2;
    if w.x > 0.01 {
        heat += w.x * lava_heat(vec3(world.z * 0.45, fall, t * 0.04 + 11.0));
    }
    if w.z > 0.01 {
        heat += w.z * lava_heat(vec3(world.x * 0.45, fall, t * 0.04 + 23.0));
    }
    let crust = 1.0 - smoothstep(0.28, 0.46, heat);
    let glow = lava_blackbody(heat) * (0.15 + LAVA_GLOW * heat * heat * heat) * (1.0 - 0.9 * crust);
    return vec4(glow, crust);
}
// The cooled crust's colour (gamma).
const LAVA_ROCK: vec3<f32> = vec3(0.07, 0.06, 0.055);
// End of the lava's look.
// RT lighting's edge of the world, Remix's distance fade (its `viewDistance` mode 3, function 1,
// noise scale 20, from 87.5 % of the distance to all of it): past the start the surface gives way
// to the horizon pixel by pixel, where a world-space noise (20 blocks across) is below how far it
// is into the fade, instead of vanilla's linear fog; the environment's fog (water, lava,
// blindness) stays vanilla's.
fn rt_fade(spherical: f32, cylindrical: f32, pos: vec3<f32>) -> f32 {
    let end = globals.fog.w;
    let into = linear_fog(cylindrical, end * 0.875, end);
    let world = pos + globals.rt_camera.xyz;
    let noise = lava_noise(vec3(world.x, world.y, world.z) / 20.0);
    let faded = select(0.0, 1.0, into > 0.0 && noise < into);
    return max(linear_fog(spherical, globals.fog.x, globals.fog.y), faded);
}
fn rt_lava(pos: vec3<f32>, n: vec3<f32>, pixel: vec2<f32>, light: vec2<f32>, spherical: f32,
    cylindrical: f32) -> vec3<f32> {
    let look = lava_look(pos + globals.rt_camera.xyz, n, globals.rt_water.z);
    let rock = rt_material_lit(rt_plain(LAVA_ROCK, n), n, pos, pixel, light, spherical, cylindrical);
    return look.rgb + rock * look.w;
}
fn rt_lava_on(layer: u32) -> bool {
    return globals.rt_mode.x > 0.5 && linear_on() && rt_is_lava(layer);
}
fn rt_is_water(layer: u32) -> bool {
    let l = f32(layer & 0x7fffu);
    return l == globals.rt_water.x || l == globals.rt_water.y;
}
fn rt_bottom(pixel: vec2<f32>, ray: vec3<f32>) -> f32 {
    if globals.rt_motion.w < 1.5 { return -1.0; }
    let size = vec2<f32>(textureDimensions(rt_depth));
    let at = clamp(vec2<i32>(pixel / globals.rt_motion.xy * size), vec2(0), vec2<i32>(size) - 1);
    let depth = textureLoad(rt_depth, at, 0);
    if depth <= 0.0 { return -1.0; }
    // The distance s along `ray` where clip z / w is `depth`.
    let m = globals.view_proj;
    let z = vec4(m[0].z, m[1].z, m[2].z, m[3].z);
    let w = vec4(m[0].w, m[1].w, m[2].w, m[3].w);
    let s = (z.w - depth * w.w) / (depth * dot(w.xyz, ray) - dot(z.xyz, ray));
    return s;
}
// What the water adds over what lies behind it, and how much of that shows through it, in colour
// (red absorbed first); its alpha for a blend without colour (`through`'s luminance).
struct WaterLight {
    added: vec3<f32>,
    through: vec3<f32>,
    alpha: f32,
}
// The fragment `rt_water_at` lights: camera-relative position, pixel, light levels, fog distances.
struct WaterIn {
    pos: vec3<f32>,
    clip: vec2<f32>,
    light: vec2<f32>,
    spherical: f32,
    cylindrical: f32,
}
fn rt_water(in: TerrainOut, face: vec3<f32>) -> WaterLight {
    return rt_water_at(in.pos, in.clip.xy, in.light, in.spherical, in.cylindrical, face, in.tint.rgb,
        in.tint.w);
}
// The same from a fragment's camera-relative `pos`, its `pixel`, light levels, fog distances, face,
// biome tint (gamma) and how open the water lies (`TerrainOut::sea`), for the parallax terrain's
// water too.
fn rt_water_at(pos: vec3<f32>, pixel: vec2<f32>, levels: vec2<f32>, spherical: f32,
    cylindrical: f32, face: vec3<f32>, tint: vec3<f32>, sea: f32) -> WaterLight {
    let in = WaterIn(pos, pixel, levels, spherical, cylindrical);
    let colour = pow(clamp(tint, vec3(0.0), vec3(1.0)), vec3(2.2));
    let absorption = WATER_ABSORPTION + WATER_PURE_ABSORPTION + WATER_TINT_ABSORPTION * (1.0 - colour);
    let world = in.pos + globals.rt_camera.xyz;
    var n = face;
    // The waves finer than the pixel's footprint, as roughness (none on a side, or near by).
    var rough = 0.0;
    // How steep the rain's impacts stand here (`rt_spray`).
    var spray = 0.0;
    if face.y > 0.5 {
        let footprint = water_footprint(in.pos, face);
        let w = water_waves(world.xz, globals.rt_water.z, footprint, sea);
        rough = water_lost_slope(footprint, water_sea(world.xz, globals.rt_water.z, sea));
        // The rain's ripples where it falls on this water (`rain_ripples`).
        let r = rt_rain_ripples(in.pos);
        // On the face as drawn: far off it leans with the Earth's curvature.
        n = normalize(face / face.y + vec3(-w.y - r.x, 0.0, -w.z - r.y));
        spray = min(length(r), 1.0);
    }
    let distance = length(in.pos);
    let ray = in.pos / max(distance, 1e-4);
    let view = -ray;
    let nv = max(dot(n, view), 1e-3);
    // Where waves finer than the pixel roughen it, each frame sees the sky and the Eye off one
    // facet drawn from their slopes (a Gaussian of the variance `rough`), with that facet's
    // Fresnel: the temporal resolve averages the draws into what the pixel's many facets reflect,
    // the long glitter a light makes on rough water and the brighter mean reflectance of tilted
    // facets at a slant, their energy kept. Near by, `rough` is 0: the surface itself.
    var mirrored = reflect(ray, n);
    var facing = nv;
    if rough > 1e-6 {
        let u = max(rt_dither(in.clip), 1e-4);
        let turn = 6.2831853 * rt_dither(in.clip + vec2(17.0, 59.0));
        let g = sqrt(-2.0 * log(u) * 0.5 * rough) * vec2(cos(turn), sin(turn));
        let facet = normalize(n + vec3(g.x, 0.0, g.y));
        let off = reflect(ray, facet);
        // A facet turned from the eye, or reflecting into the water, is hidden by its neighbours.
        if dot(facet, view) > 0.0 && dot(off, n) > 0.0 {
            mirrored = off;
            facing = dot(facet, view);
        }
    }
    let fresnel = 0.02 + 0.98 * pow(1.0 - facing, 5.0);
    let v = rt_visibility(in.pos, in.clip);
    let sky_seen = select(rt_sky_level(in.light.x), max(v.z, 0.0),
        v.z >= 0.0);
    let sun = globals.rt_sun_light.rgb * globals.rt_light.w * v.x * rt_cloud_shadow(in.pos);
    // Reflected: the sky that way, and the sun's glint (GGX, a calm surface).
    var reflected = rt_sky(mirrored) * max(sky_seen, 0.15 * v.x);
    // The Eye of Aldhani's streaks in it, blurred by the waves over a few pixels.
    if globals.rt_camera.w >= 0.0 {
        reflected += aldhani_streaks(mirrored, 6.0 * globals.misc.w / globals.outline_params.y)
            * max(sky_seen, 0.15 * v.x);
    }
    if globals.rt_motion.w > 1.5 {
        let traced = rt_reflection(in.clip, in.pos);
        if traced.a > 0.5 {
            reflected = traced.rgb;
        } else {
            // Past the rays' structures: what the screen shows that way (`rt_screen_reflection`).
            let seen = rt_screen_reflection(in.pos, mirrored);
            reflected = mix(reflected, seen.rgb, seen.a);
        }
        // HORIZON_RT_DEBUG=9: the traced reflection alone (magenta: none, the sky's).
        if globals.rt_mode.y > 2.5 && globals.rt_mode.y < 3.5 {
            return WaterLight(select(vec3(1.0, 0.0, 1.0), traced.rgb, traced.a > 0.5), vec3(0.0),
                1.0);
        }
    }
    // Seen from under the water (the surface's underside): past Snell's window (48.6 degrees off
    // the normal) the surface mirrors the water below whole, inside it a share of it; what it
    // mirrors is the water's own body, lit as the open water's glow is, not the sky below the
    // horizon (black), unless a traced ray met something there.
    var under_fresnel = -1.0;
    if face.y < -0.5 {
        let sin_t = 1.33 * sqrt(max(1.0 - nv * nv, 0.0));
        let body_lit = rt_sky(vec3(0.0, 1.0, 0.0)) * max(sky_seen, 0.3) + sun * max(globals.rt_light.y, 0.0);
        let body = body_lit * colour * vec3(0.206, 0.999, 0.903) * WATER_GLOW
            / (1.0 + 0.8 / max(globals.rt_light.y, 0.25));
        // A traced hit (its distance in a, past 1) shows through the water it crossed back down,
        // the body's light in its place: one teal mirror, darker only over a bed a few blocks off.
        var below = vec4(0.0);
        if globals.rt_motion.w > 1.5 { below = rt_reflection(in.clip, in.pos); }
        if below.a > 0.5 {
            let back = exp(-absorption * max(below.a - 1.0, 0.0));
            reflected = below.rgb * back + body * (1.0 - back);
        } else {
            reflected = body;
        }
        if sin_t >= 1.0 {
            under_fresnel = 1.0;
        } else {
            let cos_t = sqrt(1.0 - sin_t * sin_t);
            let rs = (1.33 * nv - cos_t) / (1.33 * nv + cos_t);
            let rp = (nv - 1.33 * cos_t) / (nv + 1.33 * cos_t);
            under_fresnel = 0.5 * (rs * rs + rp * rp);
        }
    }
    let l = globals.rt_light.xyz;
    let h = normalize(l + view);
    let nl = max(dot(n, l), 0.0);
    // A calm surface's, widened by the waves too fine to see (the sun's or moon's path of glitter).
    let a2 = 0.0004 + rough;
    let nh = max(dot(n, h), 0.0);
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let glint = a2 / (3.14159265 * dd * dd) * 0.25 / max(nl * nv, 1e-3) * nl * 3.14159265;
    reflected += sun * min(glint, 200.0);
    // Through the depth: what lies behind dims by the water's absorption, the water's own glow
    // (the light it scatters toward the eye) in its place.
    let bottom = rt_bottom(in.clip, ray);
    var thickness = select(64.0, max(bottom - distance, 0.0), bottom > 0.0);
    // Remix's primary surface replacement: what the bent ray met, through the water it crossed
    // (the scene behind, unbent, no longer shows).
    var refracted = vec4(0.0);
    if globals.rt_motion.w > 1.5 {
        refracted = rt_psr_at(rt_refract, in.clip, true);
        if refracted.a > 0.0 { thickness = refracted.a; }
    }
    let through = exp(-absorption * thickness);
    let lit = rt_sky(vec3(0.0, 1.0, 0.0)) * sky_seen + sun * max(l.y, 0.0);
    // What the water scatters back: Remix's single-scattering albedo (0.206, 0.999, 0.903) in the
    // biome's colour. The light scattered toward the eye along the ray is the sun's, itself
    // dimmed by the water above the scatterer (a slant path down, about the ray's own depth
    // times the sun's secant) and the scattered light by the path back to the eye: the integral
    // along the ray of both dimmings, so the glow saturates to the deep water's colour
    // (blue-green, red absorbed first) and a shallow bed shows through it.
    let both = 1.0 + 0.8 / max(l.y, 0.25);
    let glow = lit * colour * vec3(0.206, 0.999, 0.903) * WATER_GLOW * (1.0 - exp(-absorption * thickness * both)) / both;
    let fresnel_used = select(fresnel, under_fresnel, under_fresnel >= 0.0);
    var added = fresnel_used * reflected + (1.0 - fresnel_used) * glow
        + rt_sky(vec3(0.0, 1.0, 0.0)) * sky_seen * (RAIN_SPRAY * spray);
    var shows = (1.0 - fresnel_used) * through;
    // Past the rays' reach (no refraction traced: `refracted.a` 0) the floor drawn behind was lit as
    // if no water stood over it, the rays meeting none on the way to the sun: its light dims here
    // as the rays dim it near by (shadow_rays_hw.wgsl `hw_tint`), along the sun's path bent down into
    // the water to the floor's depth and through the surface's Fresnel, in colour (the caustics'
    // focus averages to 1), so far water shows its floor as the near water does.
    if refracted.a <= 0.0 && bottom > 0.0 && l.y > 0.02 && face.y > 0.5 {
        let bent = refract(-l, vec3(0.0, 1.0, 0.0), 1.0 / 1.33);
        let down = thickness * abs(ray.y);
        let sun_in = 1.0 - (0.02 + 0.98 * pow(1.0 - l.y, 5.0));
        shows *= sun_in * exp(-absorption * down / max(-bent.y, 0.1));
    }
    // The surf: where the open sea (`sea` near 1) runs shallow over a beach, the floor within two
    // metres, white water breaks in bands rolling in to the shore (a crest every nine seconds,
    // reaching the shallows later), a line of it at the very edge, torn into lace by the wind: a
    // white diffuse layer on the water, lit by the sky and the sun, hiding what lies under it.
    // (Off unless `HORIZON_RT_SHORE=1`, `globals.earth.w`: its veil hid the bed and its caustics.)
    if globals.earth.w > 0.5 && sea > 0.75 && bottom > 0.0 && face.y > 0.5 {
        let floor_depth = max(bottom - distance, 0.0) * abs(ray.y);
        let band = (1.0 - smoothstep(0.1, 2.0, floor_depth)) * smoothstep(0.75, 0.95, sea);
        if band > 0.0 {
            let t = globals.rt_water.z;
            let roll = 0.5 + 0.5 * sin(0.7 * t + 4.0 * floor_depth + 0.05 * (world.x - world.z));
            let crest = smoothstep(0.35, 0.9, roll);
            let edge = 1.0 - smoothstep(0.05, 0.6, floor_depth);
            let lace = 0.45 * water_noise(world.xz * 0.9 + vec2(0.2, 0.1) * t)
                + 0.35 * water_noise(world.xz * 3.1 - vec2(0.3, 0.05) * t)
                + 0.2 * water_noise(world.xz * 9.7 + vec2(0.1, -0.4) * t);
            let foam = band * smoothstep(0.38, 0.55, lace + 0.25 * crest)
                * max(max(crest, edge), 0.4);
            // Lit on the surface, in the open: the sun unshadowed by the water the floor's
            // visibility (`v`) was traced through.
            let white = 0.85 * (rt_sky(vec3(0.0, 1.0, 0.0)) * max(sky_seen, 0.5)
                + globals.rt_sun_light.rgb * globals.rt_light.w * max(l.y, 0.0) / 3.14159265);
            added = mix(added, white, foam);
            shows *= 1.0 - foam;
        }
    }
    // The air in front, as for the terrain: it dims both and adds the horizon's light.
    let f = rt_fade(in.spherical, in.cylindrical, in.pos) * globals.fog_color.a;
    let toward = normalize(vec3(in.pos.x, 0.0, in.pos.z) + vec3(0.0, 0.05 * length(in.pos.xz), 0.0));
    let horizon = rt_sky(toward) * sky_seen;
    let air = vec3(5.802e-3 + 4.44e-3, 13.558e-3 + 4.44e-3, 33.1e-3 + 4.44e-3);
    let air_through = exp(-air * (distance * 0.001 * globals.rt_mode.w)) * (1.0 - f);
    added = added * air_through + horizon * (1.0 - air_through);
    shows *= air_through;
    if refracted.a > 0.0 {
        return WaterLight(added + shows * refracted.rgb, vec3(0.0), 1.0);
    }
    let alpha = clamp(1.0 - dot(shows, vec3(0.2126, 0.7152, 0.0722)), 0.02, 1.0);
    return WaterLight(added, shows, alpha);
}
// RAIN-BEGIN: physical rain and the wet world it leaves (rain.rs, gfx/rain.rs), the same text in
// terrain.wgsl (the drops, the wet surfaces, the puddles) and gfx/shadow_rays.wgsl (what the
// puddles reflect); a test keeps them equal. `rain_map` is the rain map (rain.rs `RainMap`).
const RAIN_MAP: i32 = 256;
// A world column's word in the rain map (`c` in any wrap of 4096 blocks), 0 where the map doesn't
// hold it: outside its window (the texel holds another column) or not loaded.
fn rain_cell(c: vec2<i32>) -> u32 {
    let w = rain_map[u32((c.y & (RAIN_MAP - 1)) * RAIN_MAP + (c.x & (RAIN_MAP - 1)))];
    let key = u32((c.x >> 8u) & 15) | (u32((c.y >> 8u) & 15) << 4u);
    if (w >> 24u) != key || ((w >> 12u) & 15u) == 0u { return 0u; }
    return w;
}
// A column's rain height (one above its top block), its surface (rain.rs `Surface`), whether it
// rains there, whether it snows there (snow.rs), how many of its neighbours stand higher, the block
// light above it.
fn rain_height(w: u32) -> f32 { return f32(i32(w & 2047u) - 1024); }
fn snow_falls(w: u32) -> bool { return ((w >> 11u) & 1u) == 1u; }
fn rain_surface(w: u32) -> u32 { return (w >> 12u) & 15u; }
fn rain_falls(w: u32) -> bool { return ((w >> 20u) & 1u) == 1u; }
fn rain_higher(w: u32) -> f32 { return f32((w >> 21u) & 7u); }
fn rain_light(w: u32) -> f32 { return f32((w >> 16u) & 15u); }
// How far under the rain height a surface's top is (paths and farmland: a sixteenth).
fn rain_top(w: u32) -> f32 {
    return rain_height(w) - select(0.0, 0.0625, rain_surface(w) == 11u);
}
// How porous a surface is where the pack says nothing (0 the water runs off, 1 it drinks it all;
// Lagarde 2013's porosity): stone, wood, soil, sand, wool, metal and glass, leaves, partial
// blocks, grass.
fn rain_porosity(s: u32) -> f32 {
    switch s {
        case 1u: { return 0.35; }
        case 2u: { return 0.5; }
        case 3u, 11u: { return 0.7; }
        case 4u: { return 0.9; }
        case 5u: { return 1.0; }
        case 6u: { return 0.0; }
        case 7u: { return 0.1; }
        case 9u: { return 0.6; }
        default: { return 0.4; }
    }
}
// How much of the water a surface drinks rather than holding as a film (sand and wool stay matte,
// grass half: its blades hold drops, not a film).
fn rain_absorbs(s: u32) -> f32 {
    switch s {
        case 4u: { return 0.7; }
        case 5u: { return 1.0; }
        case 9u: { return 0.5; }
        default: { return 0.0; }
    }
}
// How readily puddles sit on a surface, added to the puddle field; below -1 never (leaves, metal
// and glass, partial blocks, water, the dry ones).
fn rain_puddle_bias(s: u32) -> f32 {
    switch s {
        case 1u, 2u: { return 0.0; }
        case 3u, 11u: { return 0.1; }
        case 9u: { return -0.45; }
        case 4u: { return -0.4; }
        default: { return -9.0; }
    }
}
// A cell's random number (PCG2D, Jarzynski and Olano 2020: exact on integers, so neighbouring
// pixels and both eyes agree), and a seed's.
fn rain_seed(c: vec2<i32>, k: u32) -> u32 {
    var v = bitcast<vec2<u32>>(c) * 1664525u + 1013904223u;
    v.x += v.y * 1664525u + k * 2654435761u;
    v.y += v.x * 1664525u;
    v ^= v >> vec2(16u);
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    return v.x ^ (v.y >> 16u);
}
fn rain_rand(n: u32) -> f32 {
    let state = n * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return f32(((word >> 22u) ^ word) >> 8u) / 16777216.0;
}
// Smooth value noise over the world's plane, 0 to 1, a feature a unit across.
fn rain_noise(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = p - floor(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = rain_rand(rain_seed(i, 7u));
    let b = rain_rand(rain_seed(i + vec2(1, 0), 7u));
    let c = rain_rand(rain_seed(i + vec2(0, 1), 7u));
    let d = rain_rand(rain_seed(i + vec2(1, 1), 7u));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
// How much rain reaches a surface at world `p` facing `n` (0 to 1; -1 where the map knows
// nothing): the columns around the point the face looks onto let the rain down to their height,
// blended over a block between their centres so a roof's edge leaves no staircase on the ground,
// read a little off where a noise says so that the dry line under an awning is not ruled.
fn rain_exposure(p: vec3<f32>, n: vec3<f32>) -> f32 {
    if n.y < -0.5 { return 0.0; }
    let jitter = vec2(rain_noise(p.xz * 1.3), rain_noise(p.zx * 1.3 + 19.0)) - 0.5;
    let q = p.xz + n.xz * 0.5 + jitter * 0.5 - 0.5;
    let base = vec2<i32>(floor(q));
    let f = q - floor(q);
    var sum = 0.0;
    var known = 0.0;
    for (var i = 0; i < 4; i++) {
        let o = vec2(i & 1, i >> 1u);
        let weight = mix(1.0 - f.x, f.x, f32(o.x)) * mix(1.0 - f.y, f.y, f32(o.y));
        let w = rain_cell(base + o);
        if w == 0u { continue; }
        known += weight;
        let top = rain_top(w);
        sum += weight * smoothstep(top - 0.6, top - 0.05, p.y) * select(0.0, 1.0, rain_falls(w));
    }
    if known < 0.5 { return -1.0; }
    return sum / known;
}
// How deep a puddle is at world `p` on a top face (0 none, 1 a full water surface). Puddles sit on
// the tops of the columns that hold them, where a field rises above the level the rain has filled
// them to (`level`, 0 to 1): a world-space noise a few blocks across, plus the hollows (neighbours
// standing higher, blended between column centres), plus the pack's relief (`relief`: 0 the top,
// 1 its deepest, 0.5 unknown), minus near an edge over lower ground or a block that holds none, so
// their shores are organic and run on across blocks at the same height.
fn rain_puddle(p: vec3<f32>, level: f32, relief: f32) -> f32 {
    if level <= 0.0 { return 0.0; }
    let c = vec2<i32>(floor(p.xz));
    let w = rain_cell(c);
    if w == 0u || !rain_falls(w) { return 0.0; }
    let bias = rain_puddle_bias(rain_surface(w));
    if bias < -1.0 { return 0.0; }
    let top = rain_top(w);
    if abs(p.y - top) > 0.02 { return 0.0; }
    let f = p.xz - floor(p.xz);
    // Toward a neighbour lower or holding none, the shore keeps back from the edge.
    var edge = 1.0;
    for (var i = 0; i < 4; i++) {
        let side = select(vec2<i32>(select(-1, 1, i == 1), 0), vec2<i32>(0, select(-1, 1, i == 3)),
            i >= 2);
        let distance = select(select(f.x, 1.0 - f.x, i == 1), select(f.y, 1.0 - f.y, i == 3), i >= 2);
        let n = rain_cell(c + side);
        let higher = n != 0u && rain_top(n) > top + 0.5;
        let level_with = n != 0u && abs(rain_top(n) - top) < 0.01
            && rain_puddle_bias(rain_surface(n)) > -1.0;
        if !(higher || level_with) { edge = min(edge, smoothstep(0.05, 0.45, distance)); }
    }
    // The hollows, blended between the centres of the columns level with this one.
    let q = p.xz - 0.5;
    let b = vec2<i32>(floor(q));
    let g = q - floor(q);
    var hollow = 0.0;
    for (var i = 0; i < 4; i++) {
        let o = vec2(i & 1, i >> 1u);
        let weight = mix(1.0 - g.x, g.x, f32(o.x)) * mix(1.0 - g.y, g.y, f32(o.y));
        let n = rain_cell(b + o);
        let k = select(rain_higher(w), rain_higher(n), n != 0u && abs(rain_top(n) - top) < 0.01);
        hollow += weight * k;
    }
    let noise = 0.55 * rain_noise(p.xz * 0.22) + 0.3 * rain_noise(p.xz * 0.57 + 7.0)
        + 0.15 * rain_noise(p.xz * 1.9 + 3.0);
    let field = noise + 0.08 * hollow + bias + 0.3 * (relief - 0.5) - 0.7 * (1.0 - edge);
    let threshold = 1.02 - 0.52 * level;
    return smoothstep(threshold, threshold + 0.05, field);
}
// Whether world `p` is on the top of its column under the rain with a film of water on it that
// mirrors (not a surface that drinks it, not grass, whose blades break the film, not a dry one):
// the rays trace its reflection.
fn rain_film(p: vec3<f32>) -> bool {
    let w = rain_cell(vec2<i32>(floor(p.xz)));
    if w == 0u || !rain_falls(w) { return false; }
    let s = rain_surface(w);
    return abs(p.y - rain_top(w)) < 0.02 && rain_absorbs(s) == 0.0 && s != 9u && s != 10u
        && s != 12u;
}
// The rain's impacts at world `p` (x, z), `t` seconds: rings spreading from where the drops strike,
// in two offset layers of quarter-block cells, each cell struck once a period (0.5 to 0.9 s) with a
// chance growing with the rate (`rate` mm/h): about thirty impacts a square metre a second in
// moderate rain. `spread` is how far a ring runs (blocks): from 0.15 standing water, a train of
// rings; under it the thin film of a wet surface, a small circle. `wet` (0 to 1) sharpens them: on a surface that is drying only the
// strongest show, a soaked one shows all (SEUS's idea, with a procedural field instead of its
// flipbook). The slope (x, z); a ring finer than a pixel (`footprint` blocks) fades out.
// RAIN_RING: how steep a ring's front stands at most on standing water (its slope), RAIN_CROWN on
// a film: steep enough to show in the reflection of an overcast sky, whose light barely changes
// with the direction.
const RAIN_RING: f32 = 2.5;
const RAIN_CROWN: f32 = 2.0;
fn rain_ripples(p: vec2<f32>, t: f32, rate: f32, footprint: f32, spread: f32, wet: f32)
    -> vec2<f32> {
    let fade = 1.0 - smoothstep(0.25 * spread, 0.6 * spread, footprint);
    if rate <= 0.0 || fade <= 0.0 { return vec2(0.0); }
    let chance = clamp(rate / 6.0, 0.3, 1.0);
    // Standing water's rings, finer than the film's: where they shrink under a pixel only their
    // fronts remain (no shimmer).
    let standing = spread >= 0.15;
    let fine = 1.0 - smoothstep(0.3, 0.8, footprint / select(0.4 * spread, 0.017, standing));
    var slope = vec2(0.0);
    for (var layer = 0u; layer < 2u; layer++) {
        let q = p / 0.25 + f32(layer) * vec2(0.5, 0.37);
        let base = vec2<i32>(floor(q));
        for (var i = 0; i < 9; i++) {
            let cell = base + vec2(i % 3 - 1, i / 3 - 1);
            let seed = rain_seed(cell, 11u + layer);
            let period = 0.5 + 0.4 * rain_rand(seed);
            let phase = t / period + rain_rand(seed + 1u);
            let age = fract(phase);
            let strike = seed ^ (u32(i32(floor(phase))) * 2654435761u);
            if rain_rand(strike) > chance { continue; }
            let centre = vec2<f32>(cell) + vec2(rain_rand(strike + 2u), rain_rand(strike + 3u));
            let d = (q - centre) * 0.25;
            let r = length(d);
            let x = r - sqrt(age) * spread;
            // On a film one ring; on standing water the drop's momentum goes into the surface as
            // a train of capillary rings about 1.7 cm apart, spreading behind the front and
            // crossing its neighbours'.
            var width = spread * (0.08 + 0.12 * age);
            var wavelength = 2.0 * width;
            if standing {
                width = spread * (0.15 + 0.35 * age);
                wavelength = 0.017 + 0.01 * age;
            }
            let envelope = exp(-x * x / (width * width)) * (1.0 - age) * (1.0 - age);
            let wave = mix(1.0, cos(x * 6.2832 / wavelength), fine);
            slope += 0.5 * wave * envelope * d / max(r, 1e-4);
        }
    }
    let sharp = 2.0 - 1.2 * wet * wet * wet;
    return sign(slope) * pow(min(abs(slope), vec2(1.0)), vec2(sharp)) * fade;
}
// RAIN-END
// The world position of camera-relative `pos`: x and z in the 4096-block wrap the camera's is kept
// in (`rt_camera`), y the real one.
fn rain_world(pos: vec3<f32>) -> vec3<f32> {
    var w = pos + globals.rt_camera.xyz;
    if globals.rt_camera.y >= 2048.0 { w.y -= 4096.0; }
    return w;
}
// The blocks a pixel spans at camera-relative `pos`.
fn rain_footprint(pos: vec3<f32>) -> f32 {
    return length(pos) * 2.0 * globals.misc.w / max(globals.outline_params.y, 1.0);
}
// The rain's ripples (slopes, x and z) on the water's top at camera-relative `pos`, where the rain
// falls on it: none outside the rain or under cover.
fn rt_rain_ripples(pos: vec3<f32>) -> vec2<f32> {
    if globals.rain.x <= 0.0 { return vec2(0.0); }
    let world = rain_world(pos);
    let w = rain_cell(vec2<i32>(floor(world.xz)));
    if w == 0u || !rain_falls(w) || rain_surface(w) != 10u || world.y < rain_height(w) - 1.2 {
        return vec2(0.0);
    }
    return rain_ripples(world.xz, globals.rt_water.z, globals.rain.x, rain_footprint(pos), 0.2, 1.0)
        * RAIN_RING;
}
// How much of a puddle the fragment being lit is (`rt_wet`): `rt_material_lit` then takes what the
// rays found it reflects (`rt_puddle_reflection`).
var<private> rt_puddle: f32 = 0.0;
// How steep the rain's impacts stand at the fragment being lit (0 to 1): their crests, broken
// water, catch the sky's light from every side (`rt_shade` adds RAIN_SPRAY of it).
var<private> rt_spray: f32 = 0.0;
const RAIN_SPRAY: f32 = 0.08;
// What the rays found the puddle at `pixel` reflects (shadow_rays_hw.wgsl `hw_reflect`): rgb its
// light, a 1 when it reflects a surface (0: the sky, reflected here).
fn rt_puddle_reflection(pixel: vec2<f32>) -> vec4<f32> {
    if globals.rt_motion.w < 1.5 { return vec4(0.0); }
    return rt_psr_at(rt_reflect, pixel, false);
}
// RT lighting's wet world (after Lagarde 2013, "Water drop 2b: dynamic rain and its effects"):
// material `m` on a face of normal `geo` at camera-relative `pos` (its pixel, its sky light level
// `sky` 0..1), as the rain has left it (`Globals::rain`): where the rain reaches it
// (`rain_exposure`, the sky light past the map), darker the more porous it is (`porosity`, the
// pack's LabPBR value, below 0 its surface's own), glossier as water fills it, except where it
// drinks the water (sand, wool) or runs off (metal, glass: a film). Sunlit surfaces dry first
// (`rain_sun`). On the tops of the columns that hold them, puddles (`relief`, the pack's depth
// there, 0.5 unknown): flat water, the rain's ripples on it while it falls.
fn rt_wet_rain(material: RtMaterial, geo: vec3<f32>, pos: vec3<f32>, pixel: vec2<f32>, sky: f32,
    porosity: f32, relief: f32) -> RtMaterial {
    var m = material;
    rt_puddle = 0.0;
    rt_spray = 0.0;
    // The shore's water first (`rt_shore`), whatever the weather: in the pores and as a film, as
    // the rain's.
    let shore = rt_shore(rain_world(pos), geo, pos);
    if shore > 0.0 {
        let s = rain_surface(rain_cell(vec2<i32>(floor(rain_world(pos).xz))));
        let p = select(rain_porosity(s), clamp(porosity, 0.0, 1.0), porosity >= 0.0);
        m.albedo *= mix(1.0, mix(0.6, 0.25, p), shore);
        let film = min(m.roughness, mix(0.08, 0.45, p));
        m.roughness = mix(m.roughness, film, shore * (1.0 - 0.5 * rain_absorbs(s)));
    }
    let shade = globals.rain;
    let sunny = globals.rain_sun;
    if max(max(shade.y, shade.z), max(sunny.x, sunny.y)) <= 0.0 { return m; }
    let world = rain_world(pos);
    var exposure = rain_exposure(world, geo);
    if exposure < 0.0 { exposure = smoothstep(0.875, 0.9375, sky) * select(0.0, 1.0, geo.y > -0.5); }
    if exposure <= 0.0 { return m; }
    // The block the face is of: its column's top surface when the face is on that top block.
    let own = rain_cell(vec2<i32>(floor(world.xz - geo.xz * 0.49)));
    let on_top = own != 0u && world.y > rain_top(own) - 1.01;
    let surface = select(1u, rain_surface(own), on_top);
    if surface == 12u { return m; }
    let sun = clamp(rt_visibility(pos, pixel).x, 0.0, 1.0) * clamp(globals.rt_light.y * 3.0, 0.0, 1.0)
        * (1.0 - globals.rt_mode.z) * globals.rt_light.w;
    let wet = mix(shade.y, sunny.x, sun) * exposure * select(0.5 + 0.5 * max(geo.y, 0.0), 1.0, geo.y > 0.5);
    let level = mix(shade.z, sunny.y, sun) * exposure;
    let p = select(rain_porosity(surface), clamp(porosity, 0.0, 1.0), porosity >= 0.0);
    // Water in the pores darkens it (less light scatters back out of the denser medium): about
    // half for stone, more the more porous.
    m.albedo *= mix(1.0, mix(1.0, mix(0.6, 0.25, p), 1.0 - m.metal), wet);
    // The water's film: smooth on a closed surface, satin in an open one's pores.
    let film = min(m.roughness, mix(0.08, 0.45, p));
    var rough = mix(mix(m.roughness, film, wet), m.roughness, rain_absorbs(surface));
    if surface == 6u { rough = m.roughness * mix(1.0, 0.5, wet); }
    m.roughness = rough;
    if geo.y > 0.9 {
        let holds = 1.0 - rain_absorbs(surface);
        // The water fills the relief as it soaks: the normal map's slopes fade (SEUS's and
        // Lagarde's wet surfaces), and the drops strike the film, small circles.
        m.normal = normalize(mix(m.normal, geo, wet * wet * wet * wet * holds * 0.85));
        let depth = rain_puddle(world, level, relief);
        if depth < 1.0 && holds > 0.0 {
            let circles = rain_ripples(world.xz, globals.rt_water.z, shade.x, rain_footprint(pos),
                0.12, wet) * (RAIN_CROWN * exposure * holds * (1.0 - depth));
            m.normal = normalize(m.normal + vec3(-circles.x, 0.0, -circles.y));
            rt_spray = min(length(circles), 1.0);
        }
        if depth > 0.0 {
            let slope = rain_ripples(world.xz, globals.rt_water.z, shade.x, rain_footprint(pos), 0.2,
                1.0) * (RAIN_RING * exposure);
            m.normal = normalize(mix(m.normal, normalize(vec3(-slope.x, 1.0, -slope.y)), depth));
            rt_spray = max(rt_spray, min(length(slope), 1.0) * depth);
            m.roughness = mix(m.roughness, 0.02, depth);
            m.f0 = mix(m.f0, vec3(0.02), depth);
            m.metal *= 1.0 - depth;
            m.albedo *= mix(1.0, 0.6, depth);
            m.ao = mix(m.ao, 1.0, depth);
            rt_puddle = depth;
        }
        // A wet film mirrors too: the rays trace what it reflects (`hw_puddle`).
        if rt_puddle <= 0.0 && wet > 0.2 && rain_film(world) { rt_puddle = 1e-3; }
    }
    // HORIZON_RT_DEBUG=10: where the rain reaches (red), the puddles (green), the rain map's
    // columns known (blue).
    // 11: how wet (red), how porous (green), how glossy (blue).
    if globals.rt_mode.y > 3.5 && globals.rt_mode.y < 5.5 {
        let known = select(0.0, 1.0, rain_cell(vec2<i32>(floor(world.xz))) != 0u);
        m.albedo = select(vec3(exposure, rt_puddle, known), vec3(wet, p, 1.0 - m.roughness),
            globals.rt_mode.y > 4.5);
        m.emission = 0.25;
    }
    return m;
}
// RT lighting's wet shore: a sand, soil or stone top at the water's edge (a water column of the rain
// map one or two blocks off, the top a block above that water at most) holds what the swash
// and the spray leave, soaked up to the waterline, the wet line running up and back over a few
// seconds. 0 dry to 1 soaked; none past 64 blocks or out of the map. Eight look-ups of the map at
// most, on those tops alone.
fn rt_shore(world: vec3<f32>, geo: vec3<f32>, pos: vec3<f32>) -> f32 {
    // Off unless `HORIZON_RT_SHORE=1` (`globals.earth.w`).
    if globals.earth.w < 0.5 || geo.y < 0.5 || dot(pos, pos) > 4096.0 { return 0.0; }
    let c = vec2<i32>(floor(world.xz));
    let own = rain_cell(c);
    let s = rain_surface(own);
    if own == 0u || (s != 1u && s != 3u && s != 4u) { return 0.0; }
    let top = rain_top(own);
    if world.y < top - 0.05 { return 0.0; }
    let swash = 0.5 + 0.5 * sin(0.7 * globals.rt_water.z + 0.13 * world.x + 0.11 * world.z);
    let dirs = array<vec2<i32>, 4>(vec2(1, 0), vec2(-1, 0), vec2(0, 1), vec2(0, -1));
    var wet = 0.0;
    for (var i = 0; i < 4; i++) {
        for (var d = 1; d <= 2; d++) {
            let n = rain_cell(c + dirs[i] * d);
            if n != 0u && rain_surface(n) == 10u {
                let rise = top - rain_height(n);
                let near = select(1.0, 0.55, d == 2);
                wet = max(wet, near * (1.0 - smoothstep(0.75 + 0.3 * swash, 1.35 + 0.3 * swash, rise)));
                break;
            }
        }
    }
    return wet;
}
// RT lighting's weather on a surface: wet as the rain left it (`rt_wet_rain`), then under the snow's
// cover where it snows (`rt_snow`). The terrain's and the parallax terrain's shading call it.
fn rt_wet(material: RtMaterial, geo: vec3<f32>, pos: vec3<f32>, pixel: vec2<f32>, sky: f32,
    porosity: f32, relief: f32) -> RtMaterial {
    return rt_snow(rt_wet_rain(material, geo, pos, pixel, sky, porosity, relief), geo, pos);
}
// How brightly the snow's crystals at the fragment being lit catch the sun toward the eye
// (`rt_shade` adds the sun's light times it): a mirror-flat facet now and then among the grains.
var<private> rt_sparkle: f32 = 0.0;
// Fresh snow, linear: nearly white (its blue in the shade is the sky's light).
const SNOW_ALBEDO: vec3<f32> = vec3(0.88, 0.9, 0.92);
// How deep the snow must lie to hide a column's top surface (rain.rs `Surface`), in centimetres:
// grass blades and leaves poke through longer, metal and glass shed it; none on water, nor on the
// dry surfaces (snow and ice already, lava, magma) or where the map knows nothing.
fn snow_full(s: u32) -> f32 {
    switch s {
        case 1u, 2u, 3u, 11u: { return 1.5; }
        case 4u, 5u: { return 1.0; }
        case 6u: { return 5.0; }
        case 7u: { return 2.5; }
        case 8u: { return 2.0; }
        case 9u: { return 3.0; }
        default: { return 0.0; }
    }
}
// Three octaves of the rain's value noise, 0 to 1.
fn snow_noise(p: vec2<f32>) -> f32 {
    return 0.55 * rain_noise(p * 1.7) + 0.3 * rain_noise(p * 4.3 + 11.0)
        + 0.15 * rain_noise(p * 13.0 + 5.0);
}
// RT lighting's snow cover (snow.rs `Cover`, `Globals::linear` w its depth in centimetres) on
// material `m`, a face of normal `geo` at camera-relative `pos`, where the rain map says it snows:
// - the top faces of a column's top block, in patches first (the noise against how deep it lies
//   over how deep that surface needs: grass and leaves longer, metal longest), then whole;
// - the top edge of that block's sides where they stand clear (the cover's thickness over the
//   edge, ragged);
// - the walls that face the wind, wherever they stand clear of the next column: snow plastered on
//   them in drifts, more the harder it blows (the prevailing wind, `Globals::rain_sun` zw).
// The snow is white, rough, flat (it fills the normal map's relief) and opaque to the light from
// behind a leaf; its crystals glint (`rt_sparkle`) where a pixel is small enough to see them.
fn rt_snow(material: RtMaterial, geo: vec3<f32>, pos: vec3<f32>) -> RtMaterial {
    var m = material;
    rt_sparkle = 0.0;
    let depth = globals.linear.w;
    if depth <= 0.0 { return m; }
    let world = rain_world(pos);
    let own = rain_cell(vec2<i32>(floor(world.xz - geo.xz * 0.49)));
    if own == 0u || !snow_falls(own) { return m; }
    let full = snow_full(rain_surface(own));
    if full <= 0.0 { return m; }
    let top = rain_top(own);
    var cover = 0.0;
    if geo.y > 0.7 {
        if world.y > top - 1.01 && world.y < top + 0.01 {
            let f = depth / full;
            cover = smoothstep(0.0, 0.25, f * 1.15 - snow_noise(world.xz));
        }
    } else if abs(geo.y) < 0.3 {
        // The next column the face looks onto: the face stands clear above its top.
        let next = rain_cell(vec2<i32>(floor(world.xz + geo.xz * 0.51)));
        let clear = next == 0u || world.y > rain_top(next) - 0.02;
        if clear {
            let along = vec2(dot(world.xz, vec2(-geo.z, geo.x)), world.y);
            let n = snow_noise(along * 2.0);
            // The rim: the cover's thickness over the top block's edge, once the top is white.
            if world.y > top - 1.01 && world.y < top + 0.01 {
                let thick = min(depth, 12.5) * 0.01 * (0.5 + n)
                    * smoothstep(0.4, 1.0, depth / full);
                cover = smoothstep(top - thick - 0.01, top - thick + 0.01, world.y);
            }
            // Plastered on the windward walls.
            let wind = globals.rain_sun.zw;
            let speed = length(wind);
            if speed > 0.1 {
                let facing = clamp(dot(geo.xz, -wind / speed), 0.0, 1.0);
                let amount = facing * clamp(speed / 5.0, 0.0, 1.0) * clamp(depth / 4.0, 0.0, 1.0);
                cover = max(cover, smoothstep(0.55, 0.75, amount * 0.7 + n * 0.5));
            }
        }
    }
    if cover <= 0.0 { return m; }
    m.albedo = mix(m.albedo, SNOW_ALBEDO, cover);
    m.roughness = mix(m.roughness, 0.7, cover);
    m.f0 = mix(m.f0, vec3(0.018), cover);
    m.metal *= 1.0 - cover;
    m.normal = normalize(mix(m.normal, geo, cover));
    m.ao = mix(m.ao, 1.0, 0.8 * cover);
    m.subsurface *= 1.0 - cover;
    m.emission *= 1.0 - cover;
    // Its crystals, a few in every 4 cm grain turned at random about the face: the ones that
    // mirror the sun toward the eye glint, seen only where the pixel is smaller than the grain.
    let near = 1.0 - smoothstep(0.015, 0.05, rain_footprint(pos));
    if near > 0.0 {
        let q = vec3<i32>(floor(world * 24.0));
        let seed = rain_seed(vec2(q.x + q.y * 7919, q.z), 29u);
        if rain_rand(seed) < 0.08 {
            let tilt = vec3(rain_rand(seed + 1u), rain_rand(seed + 2u), rain_rand(seed + 3u)) - 0.5;
            let facet = normalize(geo + 0.6 * tilt);
            let h = normalize(globals.rt_light.xyz - normalize(pos));
            rt_sparkle = cover * near * 1.5 * pow(max(dot(facet, h), 0.0), 400.0);
        }
    }
    return m;
}
// RT lighting's opaque packed ice (mesher.rs `rt_packed_ice_clear` off): compacted ice, not a
// matte block. Its face is polished and wet-looking (GGX roughness 0.2, ice's F0 0.018, whatever
// the pack's specular map says), so it carries the sky's and the lights' sheen, and its body lets
// the light through from behind it (subsurface 0.6), a little bluer than its texture.
fn rt_packed_ice(m_in: RtMaterial, layer: u32) -> RtMaterial {
    if rt_ice(layer) != 2u { return m_in; }
    var m = m_in;
    m.roughness = min(m.roughness, 0.2);
    m.f0 = vec3(0.018);
    m.metal = 0.0;
    m.subsurface = max(m.subsurface, 0.6);
    m.albedo = clamp(m.albedo * vec3(0.92, 0.98, 1.06), vec3(0.0), vec3(1.0));
    return m;
}
// RT lighting: a fragment without maps, its albedo (`albedo`, gamma) lit and fogged, wet as the
// rain left it.
fn rt_lit(albedo: vec3<f32>, in: TerrainOut, n: vec3<f32>) -> vec3<f32> {
    let m = rt_wet(rt_packed_ice(rt_plain(albedo, n), in.layer), n, in.pos, in.clip.xy,
        in.light.x, -1.0, 0.5);
    return rt_material_lit(m, n, in.pos, in.clip.xy, in.light, in.spherical, in.cylindrical);
}
// Whether every component is a finite number (by the bits: fast maths may drop `x != x`).
fn rt_finite4(v: vec4<f32>) -> bool {
    let bits = bitcast<vec4<u32>>(v) & vec4(0x7f800000u);
    return all(bits != vec4(0x7f800000u));
}
// HORIZON_RT_DEBUG=13: the RT lit colour `c` at the fragment at `pixel` (camera-relative `pos`),
// or which of its inputs is not a finite number, read as the colour read them (`rt_taps`): red
// the rays' mask (`rt_visibility`), blue the denoised light (`rt_traced`), green the sun's tint;
// yellow the colour itself from finite inputs, cyan the colour below 0, magenta the colour too dark
// to show (under 1e-3 paper white).
fn rt_checked(c: vec3<f32>, pos: vec3<f32>, pixel: vec2<f32>) -> vec3<f32> {
    let v = rt_visibility(pos, pixel);
    let traced = rt_traced(pixel);
    let tint = rt_sun_colour(pixel, v.x);
    if !rt_finite4(vec4(v, 0.0)) { return vec3(4.0, 0.0, 0.0); }
    if !rt_finite4(traced) { return vec3(0.0, 0.0, 4.0); }
    if !rt_finite4(vec4(tint, 0.0)) { return vec3(0.0, 4.0, 0.0); }
    if !rt_finite4(vec4(c, 0.0)) { return vec3(4.0, 4.0, 0.0); }
    if any(c < vec3(0.0)) { return vec3(0.0, 4.0, 4.0); }
    if dot(c, vec3(0.2126, 0.7152, 0.0722)) < 1e-3 { return vec3(4.0, 0.0, 4.0); }
    return c;
}
// A luminance on a log scale for the debug views: 1e-4 paper white and less 0, 1 and more 1.
fn rt_log(x: f32) -> f32 {
    return clamp(1.0 + 0.0752575 * log2(max(x, 1e-12)), 0.0, 1.0);
}
// HORIZON_RT_DEBUG=14: the parts of the lit colour (`rt_terms`), red the sun's and green the light
// around's on a log scale (1e-4 to 1 paper white), blue how much of the light around is traced (the
// denoised light's alpha: 0 the light levels alone, as past the grid).
fn rt_terms_view() -> vec3<f32> {
    return vec3(rt_log(rt_terms.x), rt_log(rt_terms.y), clamp(rt_terms.z, 0.0, 1.0));
}
// The face's normal toward the camera, from the fragment's position (faces are flat: exact).
fn face_normal(pos: vec3<f32>) -> vec3<f32> {
    let n = normalize(cross(dpdx(pos), dpdy(pos)));
    return select(n, -n, dot(n, pos) > 0.0);
}
// RT lighting is on for the solid terrain (`rt_solid`).
fn rt_solid_on() -> bool { return globals.rt_mode.x > 0.5 && linear_on(); }
// RT lighting's solid fragment, kept (not cut out, not faded): lava its own way (`rt_lava`); the
// albedo (the texel times the tint) under the model's light, fogged in light. A light-emitting
// block is lit as any other, and only its burning texels shine (a torch's flame and embers, not its
// stick: `rt_burning`).
fn rt_solid(in: TerrainOut, texel: vec4<f32>) -> vec4<f32> {
    let n = face_normal(in.pos);
    if rt_lava_on(in.layer) {
        return vec4(rt_lava(in.pos, n, in.clip.xy, in.light, in.spherical, in.cylindrical), 1.0);
    }
    if in.emission == 0u {
        // HORIZON_RT_DEBUG=7: the rays' visibility (sun red, light around green).
        if globals.rt_mode.y > 1.5 && globals.rt_mode.y < 2.5 {
            let t = rt_traced(in.clip.xy);
            return vec4(select(t.rgb, vec3(1.0, 0.0, 1.0), t.a < 0.0), 1.0);
        }
        if globals.rt_mode.y > 0.5 && globals.rt_mode.y < 1.5 {
            return vec4(max(rt_visibility(in.pos, in.clip.xy), vec3(0.0)), 1.0);
        }
        let lit = rt_lit(texel.rgb * in.tint.rgb, in, n);
        if globals.rt_mode.y > 5.5 && globals.rt_mode.y < 6.5 {
            return vec4(rt_checked(lit, in.pos, in.clip.xy), 1.0);
        }
        if globals.rt_mode.y > 6.5 && globals.rt_mode.y < 7.5 { return vec4(rt_terms_view(), 1.0); }
        return vec4(lit, 1.0);
    }
    let lit = rt_lit(texel.rgb * in.tint.rgb, in, n);
    // Without the per-layer flags (parallax.rs `thin_layers`) a block's emission is its own
    // colour as bright as its level, as before the black bodies.
    let glow = scene_color(vec4((texel * in.color).rgb, 0.0), emission_gain(in.emission),
        in.spherical, in.cylindrical).rgb;
    return vec4(mix(lit, glow, rt_burning(texel.rgb)), 1.0);
}
// How much of a light-emitting block's texel `c` (gamma) burns: its brightest, where a torch's
// flame and a lamp's glow are; a stick's wood, darker, is lit by the light, not a light itself.
fn rt_burning(c: vec3<f32>) -> f32 {
    return smoothstep(0.6, 0.85, max(c.r, max(c.g, c.b)));
}
// The light a burning texel `c` (gamma) gives off, a black body's: its temperature read from its
// colour (red the coolest, about 1300 K; yellow-white the hottest, 2200 K), its colour Planck's
// law's at that temperature through the CIE 1931 observer (fitted, linear sRGB), its luminance the
// visible part's steep rise with it (as exp(-hc / lambda k T) at 555 nm, at half the slope, so a
// flame's cool red tips still show), about 25 paper whites at a torch's 1900 K (a candle's flame is
// some 10 000 cd/m2). A texel bluer than red (soul fire) isn't a black body: its own colour.
fn rt_flame(c: vec3<f32>) -> vec3<f32> {
    if c.b > c.r { return decode_gamma(c) * 8.0; }
    let warmth = clamp((c.g / max(c.r, 1e-3) - 0.2) / 0.8, 0.0, 1.0);
    let kelvin = mix(1300.0, 2200.0, warmth);
    let colour = vec3(1.0, 0.0968 + 0.000241 * (kelvin - 1300.0), max((kelvin - 1925.0) * 0.000102, 0.0));
    let luminance = 60.0 * exp(-12965.0 * (1.0 / kelvin - 1.0 / 2200.0));
    return colour / dot(colour, vec3(0.2126, 0.7152, 0.0722)) * luminance;
}
// A fire's flame (fire, soul fire, the campfires: parallax.rs `thin_layers` bit 2) in place of
// its sprite's texels, which gave a still picture its colours: the sprite keeps the flame's shape
// (its alpha), the shader gives it a heat, noise streaming up with the time (world-fixed, so the
// planes of a campfire and the blocks of a blaze agree), hottest at the foot and cooling to the
// tips, which the noise tears off as they rise. Its colour is a black body's by that heat
// (`rt_flame`, which reads the temperature from a texel's green over its red: 0.2 the coolest,
// 1.0 the hottest); soul fire burns blue. The result is the light and the flame's coverage, to be
// cut by a dither (`rt_dither`: the temporal resolve softens it), a few ALU on the flame's
// fragments and no more geometry.
fn rt_fire(texel: vec3<f32>, uv: vec2<f32>, pos: vec3<f32>) -> vec4<f32> {
    let world = pos + globals.rt_camera.xyz;
    let t = globals.rt_water.z;
    // 0 at the foot of the sprite, 1 at its tip.
    let rise = 1.0 - uv.y;
    // Tongues of two sizes streaming up, the small ones faster.
    let p = vec3(world.x * 2.2, world.y * 1.3 - t * 2.4, world.z * 2.2);
    let n = 0.62 * lava_noise(p) + 0.38 * lava_noise(p * 2.3 + vec3(5.2, -t * 1.8, 1.3));
    let heat = clamp((1.2 - 1.05 * rise) * (0.35 + 1.1 * n), 0.0, 1.0);
    let coverage = smoothstep(0.1, 0.32, heat);
    if texel.b > texel.r {
        let cold = mix(vec3(0.02, 0.2, 1.0), vec3(0.25, 0.8, 1.0), heat);
        return vec4(cold * (1.0 + 5.0 * heat * heat), coverage);
    }
    return vec4(rt_flame(vec3(1.0, 0.2 + 0.8 * heat, 0.0)), coverage);
}
// An interleaved gradient noise (Jimenez 2014) of the pixel, changing every frame, to cut a
// coverage with: the temporal resolve averages a partial coverage into a soft edge.
fn rt_dither(pixel: vec2<f32>) -> f32 {
    let frame = floor(globals.rt_water.z * 60.0) % 64.0;
    let p = pixel + 5.588238 * frame;
    return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715))));
}
// RT lighting's depth prepass (gfx.rs `rt_prepass`): the solid terrain's depth alone, with the
// same alpha test, for this frame's rays to trace from before the world is drawn.
@fragment fn fs_depth(in: TerrainOut) {
    let color = sprite(in.uv, in.layer & 0x7fffu) * in.color;
    if (color.a < 0.5 && (in.layer & 0x8000u) == 0u) || faded(in.fade, in.local, dpdx(in.local), dpdy(in.local)) { discard; }
}
// RT lighting's cover of the translucent terrain (gfx.rs `draw_cover`): where water or glass lies
// in front of the prepass's depth, every quad of it whole (no alpha test, no fade), for the rays'
// mirrors and refraction to look for it only there (shadow_rays_hw.wgsl `hw_reflect`).
@fragment fn fs_cover() -> @location(0) vec4<f32> {
    return vec4(1.0);
}
// RT lighting's glass (shadow_rays_hw.wgsl `hw_glass`: Remix's translucent material as RTX Remix
// for Minecraft gives the block atlas): stained glass, panes, plain glass and every translucent
// block but water. A dielectric of index 1.52, its normal the pack's map (or, without one, drawn
// glass's faint waviness, `rt_glass_wave`), smooth or as rough as the pack's specular map says.
// Fresnel weighs what it reflects (what the rays met that way, or the sky, and the sun's glint)
// against what the rays met through it (`rt_refract`, shadow_rays_psr.wgsl `hw_refract_glass`:
// bent by its faces as built, dimmed by Beer-Lambert over the length run inside, deeper in colour
// where the ray slants through). The map's slopes bend the view further, here: the traced picture
// is read where the tilted facet sends the ray, which a flat back face turns by about (n - 1)
// times the tilt, over the distance to what lies behind; a rough face spreads it over the angle
// its roughness gives. Plain glass lays its texel over all of it as a diffuse layer as opaque as
// its alpha (Remix's `useDiffuseLayer`). Without traced refraction (no hardware rays) what lies
// behind shows through the glass's colour over its measurement distance along the bent ray.
const GLASS_INDEX: f32 = 1.52;
const GLASS_TRANSMITTANCE: f32 = 0.97;
// The glass a fragment shows (`rt_glass_at`): its camera-relative position, pixel, light levels
// and fog distances; its face's normal toward the camera and its surface's (the map's, or the
// waviness); its texel times the face's tint (linear) and the texel's alpha; its roughness (GGX
// alpha's square root); its sprite layer.
struct GlassIn {
    pos: vec3<f32>,
    clip: vec2<f32>,
    light: vec2<f32>,
    spherical: f32,
    cylindrical: f32,
    face: vec3<f32>,
    normal: vec3<f32>,
    albedo: vec3<f32>,
    alpha: f32,
    roughness: f32,
    layer: u32,
}
// Whether sprite `layer` is the plain glass's (its block's or its pane's edge: gfx.rs
// `set_glass_layers`, packed as the lava's).
fn rt_glass_plain(layer: u32) -> bool {
    let packed = globals.linear.y;
    if packed < 0.0 { return false; }
    let l = f32(layer & 0x7fffu);
    let pane = floor(packed / 4096.0);
    return l == packed - pane * 4096.0 || l == pane;
}
// RT lighting's ice (mesher.rs `rt_ice_kind`, gfx.rs `set_ice_layers`): 1 where sprite `layer`
// is the ice's, 2 the packed ice's, 0 neither. Both go through the glass's path with ice's own
// optics: index 1.31 (F0 0.018), the faint tint and the frost a little bubbled lake ice has, and
// for packed ice (compressed snow, full of air) a rough face and a milky body that hides what
// lies more than a block or two behind it. Their frost is a diffuse layer lit as the terrain is,
// brighter on the texture's light streaks, which scatters the sun through it (`subsurface`).
const ICE_INDEX: f32 = 1.31;
const ICE_F0: f32 = 0.018;
fn rt_ice(layer: u32) -> u32 {
    let packed = globals.earth.z;
    if packed < 0.0 { return 0u; }
    let l = f32(layer & 0x7fffu);
    let p = floor(packed / 4096.0);
    if l == packed - p * 4096.0 { return 1u; }
    return select(0u, 2u, l == p);
}
// A dielectric's reflectance at index 1.52 (Schlick's, F0 = ((n - 1) / (n + 1))^2), the cosine on
// the air's side.
fn rt_glass_fresnel(cosine: f32) -> f32 {
    return 0.0426 + 0.9574 * pow(1.0 - clamp(cosine, 0.0, 1.0), 5.0);
}
// Drawn glass's faint waviness, for glass without a normal map: the slopes of a world-fixed noise
// a block or so across, a few hundredths, so a window bends what is behind it a little and its
// reflections ripple.
fn rt_glass_wave(pos: vec3<f32>, face: vec3<f32>) -> vec3<f32> {
    let p = (pos + globals.rt_camera.xyz) * 1.3;
    let e = 0.15;
    let h = lava_noise(p);
    let g = vec3(lava_noise(p + vec3(e, 0.0, 0.0)), lava_noise(p + vec3(0.0, e, 0.0)),
        lava_noise(p + vec3(0.0, 0.0, e))) - h;
    let slope = (g - face * dot(g, face)) / e;
    return normalize(face - slope * 0.035);
}
// Where camera-relative `p` lands in the world target, in pixels (z: whether it is in front).
fn rt_screen(p: vec3<f32>) -> vec3<f32> {
    let c = globals.view_proj * vec4(p, 1.0);
    let ndc = c.xy / max(c.w, 1e-4);
    return vec3(vec2(0.5 + 0.5 * ndc.x, 0.5 - 0.5 * ndc.y) * globals.rt_motion.xy,
        select(0.0, 1.0, c.w > 1e-4));
}
// How far on the screen (pixels, at most 48) the point `d` along `bent` from `pos` lies from the
// one `d` along `straight`.
fn rt_shift(pos: vec3<f32>, bent: vec3<f32>, straight: vec3<f32>, d: f32) -> vec2<f32> {
    let a = rt_screen(pos + bent * d);
    let b = rt_screen(pos + straight * d);
    if a.z < 0.5 || b.z < 0.5 { return vec2(0.0); }
    let s = a.xy - b.xy;
    return s * min(1.0, 48.0 / max(length(s), 1e-4));
}
// What the rays met through the translucent surface at `pixel` (`rt_refract`).
fn rt_refracted(pixel: vec2<f32>) -> vec4<f32> {
    return rt_psr_at(rt_refract, pixel, true);
}
// RT lighting's weather on glass (not ice): where the rain reaches a pane (`rain_exposure`),
// drops bead on it, each a little lens, renewed as the rain strikes, and now and then one grows
// heavy enough to run down a vertical face, wiping a wet trail behind it that wavers and dries
// from its top; once the rain stops the drops dry off with the wetness. Where it snows, frost
// creeps in from the edges of the window (a block's face, its glass neighbours' joined) on a
// window to the outside as the cover builds (`globals.linear.w`), in
// feathery fronts, and recedes as it melts. Returns the slopes the water gives the face (along
// `t1`, `t2`, its tangents: `t2` up on a wall), x and y, and the frost's cover in z. Nothing past
// 48 blocks, nor where the drops would be under a pixel.
const GLASS_DROP_CELL: f32 = 0.05;
// Whether a clear texel of plain glass (its block's or its pane's: alpha 0 in most of it, which
// the translucent terrain drops) is kept to show the weather on it (`rt_glass_weather`): with RT
// lighting, within 48 blocks, while anything is wet or where the snow's cover lies in a column it
// snows in. The pane is glass all over: kept, it is shaded as the rest of it, its reflection too.
fn rt_glass_weathered(in: TerrainOut) -> bool {
    return in.emission == 0u && rt_glass_weathered_at(in.pos, in.layer);
}
// `rt_glass_weathered` for a fragment at camera-relative `pos` of sprite `layer` (the parallax
// terrain's too, gfx/parallax.rs).
fn rt_glass_weathered_at(pos: vec3<f32>, layer: u32) -> bool {
    if globals.rt_mode.x < 0.5 || !linear_on() { return false; }
    // Any glass but ice (the translucent terrain's every face but water's): a pack's own glass
    // sprites (connected textures, RTX packs) are not the plain glass's.
    if rt_is_water(layer) || rt_ice(layer) != 0u || dot(pos, pos) > 2304.0 { return false; }
    if globals.rain.y > 0.0 || rt_glass_forced() { return true; }
    if globals.linear.w <= 0.0 { return false; }
    let column = rain_cell(vec2<i32>(floor(rain_world(pos).xz)));
    return column != 0u && snow_falls(column);
}
// HORIZON_RT_DEBUG=15: what the weather does to glass (red its frost, green its drops' slopes,
// blue a quarter each for: reached, wet, a column where it snows, the rain reaching it); 16: the
// weather forced on every plain glass within 48 blocks (soaked, a downpour, full frost), to look
// at it whatever the sky does.
fn rt_glass_debug() -> bool { return globals.rt_mode.y > 7.5 && globals.rt_mode.y < 8.5; }
fn rt_glass_forced() -> bool { return globals.rt_mode.y > 8.5 && globals.rt_mode.y < 9.5; }
var<private> rt_glass_seen: vec4<f32> = vec4(0.0);
// Which of the glass face's in-plane neighbours are glass too (mesher.rs `glass_edges`, in its
// untinted colour's low bits), set from its tint before `rt_glass_at`.
var<private> rt_glass_neighbours: u32 = 0u;
fn rt_glass_edges(tint: vec3<f32>) -> u32 {
    if min(tint.r, min(tint.g, tint.b)) < 251.5 / 255.0 { return 0u; }
    let c = vec3<u32>(round((vec3(1.0) - tint) * 255.0));
    return (c.x & 1u) | (c.y & 1u) << 1u | (c.z & 3u) << 2u;
}
// How open to the sky the side of a glass face at world `p` facing `face` lies, or the other side
// (0 to 1): the frost forms on a window's outside, not on glass within a house. A wall's two sides
// are the columns a little off it either way, open where their tops lie below it; a roof's, its own
// column's top block. Unknown columns count as open.
fn rt_glass_outside(p: vec3<f32>, face: vec3<f32>) -> f32 {
    if abs(face.y) > 0.5 {
        let w = rain_cell(vec2<i32>(floor(p.xz)));
        if w == 0u { return 1.0; }
        return smoothstep(rain_top(w) - 1.15, rain_top(w) - 0.95, p.y);
    }
    var open = 0.0;
    for (var s = -1.0; s < 2.0; s += 2.0) {
        let w = rain_cell(vec2<i32>(floor(p.xz + face.xz * (0.6 * s))));
        if w == 0u { return 1.0; }
        open = max(open, smoothstep(rain_top(w) - 0.6, rain_top(w) - 0.05, p.y));
    }
    return open;
}
const GLASS_STREAK_CELL: f32 = 0.0833;
fn rt_glass_weather(pos: vec3<f32>, face: vec3<f32>, t1: vec3<f32>, t2: vec3<f32>, sky: f32)
    -> vec3<f32> {
    if dot(pos, pos) > 2304.0 { return vec3(0.0); }
    let world = rain_world(pos);
    let column = rain_cell(vec2<i32>(floor(world.xz)));
    var out = vec3(0.0);
    // Frost, in a column where it snows, on a window to the outside (`rt_glass_outside`), along
    // the outline of the whole window: an edge with glass beyond it is no edge
    // (`rt_glass_neighbours`).
    let forced = rt_glass_forced();
    let cover = select(globals.linear.w, 6.0, forced);
    let snowy = column != 0u && snow_falls(column);
    rt_glass_seen = vec4(0.0, 0.0, 0.25 + select(0.0, 0.25, snowy), 0.0);
    if (snowy || forced) && cover > 0.0 {
        let grow = smoothstep(0.0, 1.5, cover) * select(rt_glass_outside(world, face), 1.0, forced);
        let a = abs(face);
        let plane = select(select(world.xy, world.xz, a.y > 0.5), world.zy, a.x > 0.5);
        let local = fract(plane);
        let e = rt_glass_neighbours;
        let far = 3.0;
        let edge = min(min(select(1.0 - local.x, far, (e & 1u) != 0u), select(local.x, far, (e & 2u) != 0u)),
            min(select(1.0 - local.y, far, (e & 4u) != 0u), select(local.y, far, (e & 8u) != 0u)));
        // The window's foot: this block's, or the one below's when glass goes on down.
        let foot = select(local.y, local.y + 1.0, (e & 8u) != 0u);
        let fern = 0.55 * lava_noise(world * 9.0) + 0.3 * lava_noise(world * 23.0)
            + 0.15 * lava_noise(world * 61.0);
        // The bottom edge gathers most (the cold air sinks along the pane).
        let reach = grow * (0.18 + 0.32 * smoothstep(0.6, 0.0, foot) * step(a.y, 0.5));
        out.z = grow * (1.0 - smoothstep(reach * 0.6, reach + 0.04, edge + (fern - 0.5) * 0.22))
            * (0.65 + 0.35 * fern);
    }
    var shade = globals.rain;
    if forced { shade = vec4(12.0, 1.0, shade.z, shade.w); }
    let wet = shade.y;
    if wet <= 0.0 { return out; }
    rt_glass_seen.z += 0.25;
    var exposure = rain_exposure(world, face);
    if exposure < 0.0 { exposure = smoothstep(0.875, 0.9375, sky) * select(0.0, 1.0, face.y > -0.5); }
    if forced { exposure = 1.0; }
    if exposure <= 0.0 { return out; }
    rt_glass_seen.z += 0.25;
    let footprint = rain_footprint(pos);
    let fade = 1.0 - smoothstep(0.15 * GLASS_DROP_CELL, 0.45 * GLASS_DROP_CELL, footprint);
    if fade <= 0.0 { return out; }
    let t = globals.rt_water.z;
    let falling = shade.x > 0.0;
    let uv = vec2(dot(world, t1), dot(world, t2));
    let wall = abs(face.y) < 0.5;
    // The runs first: a column of the face a twelfth of a block wide, one drop at a time
    // sliding down a block in a second or two, its trail wet behind it for half a block.
    var trail = 0.0;
    var slope = vec2(0.0);
    if wall && falling {
        let cx = floor(uv.x / GLASS_STREAK_CELL);
        let cy = floor(uv.y);
        let seed = rain_seed(vec2<i32>(i32(cx), i32(cy)), 41u);
        let busy = clamp(shade.x / 20.0, 0.08, 0.6);
        let speed = 0.6 + 0.9 * rain_rand(seed + 1u);
        let cycle = 1.0 / speed + 2.0 + 6.0 * rain_rand(seed + 2u);
        let round = floor(t / cycle + rain_rand(seed + 3u));
        let rs = seed ^ (u32(round) * 2654435761u);
        if rain_rand(rs) < busy {
            let age = (t / cycle + rain_rand(seed + 3u) - round) * cycle;
            let head = 1.0 - age * speed;
            let fy = uv.y - cy;
            let wobble = 0.18 * sin(uv.y * 23.0 + rain_rand(rs + 4u) * 6.28)
                + 0.1 * sin(uv.y * 57.0 + rain_rand(rs + 5u) * 6.28);
            let x = (uv.x / GLASS_STREAK_CELL - cx) - 0.5 - wobble * 0.5;
            if head > -0.1 {
                // The drop: a bead a little wider than its trail.
                let d = vec2(x * GLASS_STREAK_CELL, fy - head) / (GLASS_STREAK_CELL * 0.32);
                let r2 = dot(d, d);
                if r2 < 1.0 {
                    slope += d * 1.6 / sqrt(max(1.0 - r2, 0.05));
                    trail = 1.0;
                }
            }
            let above = fy - max(head, 0.0);
            if above > 0.0 && above < 0.5 && head < 1.0 {
                let width = 0.14 * (1.0 - above / 0.5) + 0.04;
                if abs(x) < width {
                    let k = x / width;
                    slope.x += k * 0.5 * (1.0 - above / 0.5);
                    trail = max(trail, 1.0 - above / 0.5);
                }
            }
        }
    }
    // The beads: one cell in three or so holds one while it rains (fewer as the pane dries), each
    // lasting a few seconds before a new strike takes its place; none in a run's trail.
    let cell = floor(uv / GLASS_DROP_CELL);
    let seed = rain_seed(vec2<i32>(cell), 43u);
    let life = 3.0 + 5.0 * rain_rand(seed + 1u);
    let round = floor(t / life + rain_rand(seed + 2u));
    let rs = seed ^ (u32(round) * 2246822519u);
    let share = select(0.35 * wet, clamp(0.25 + shade.x / 40.0, 0.25, 0.55), falling) * wet;
    if trail < 0.5 && rain_rand(rs) < share {
        let centre = vec2(0.25 + 0.5 * rain_rand(rs + 3u), 0.25 + 0.5 * rain_rand(rs + 4u));
        let radius = 0.12 + 0.2 * rain_rand(rs + 5u);
        let d = (uv / GLASS_DROP_CELL - cell - centre) / radius;
        let r2 = dot(d, d);
        if r2 < 1.0 { slope += d * 1.2 / sqrt(max(1.0 - r2, 0.05)) * (1.0 - trail); }
    }
    out.x = slope.x * exposure * fade;
    out.y = slope.y * exposure * fade;
    return out;
}
fn rt_glass_at(g: GlassIn) -> WaterLight {
    let distance = length(g.pos);
    let ray = g.pos / max(distance, 1e-4);
    let view = -ray;
    // A clear texel (kept for the weather, `rt_glass_weathered`) is clear glass whatever sprite.
    let plain = rt_glass_plain(g.layer) || g.alpha < 0.01;
    let ice = rt_ice(g.layer);
    // Ice's optics (`rt_ice`): its index, a smooth face with a little frost on clear ice, a rough
    // one on packed ice (unless the pack's specular map says rougher).
    let index = select(GLASS_INDEX, ICE_INDEX, ice > 0u);
    let f0 = select(0.0426, ICE_F0, ice > 0u);
    var roughness = g.roughness;
    if ice == 1u { roughness = max(roughness, 0.12); }
    if ice == 2u { roughness = max(roughness, 0.5); }
    // The weather on it (`rt_glass_weather`): the water's slopes bend the facet, the frost is a
    // diffuse layer as the ice's, rough.
    var n = g.normal;
    var weather_frost = 0.0;
    if ice == 0u {
        let t1 = normalize(cross(select(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0),
            abs(g.face.y) > 0.9), g.face));
        let t2 = cross(g.face, t1);
        let w = rt_glass_weather(g.pos, g.face, t1, t2, g.light.x);
        n = normalize(n - (t1 * w.x + t2 * w.y) * 0.35);
        weather_frost = w.z;
        roughness = mix(roughness, max(roughness, 0.55), weather_frost);
        if rt_glass_debug() {
            let c = vec3(w.z, min(length(w.xy), 1.0), rt_glass_seen.z);
            return WaterLight(c, vec3(0.0), 1.0);
        }
    }
    // A map's facet turned from the eye is seen edge on: kept just inside the side it can see.
    let facing = dot(n, view);
    if facing < 0.02 { n = normalize(n + view * (0.02 - facing)); }
    let nv = max(dot(n, view), 1e-3);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - clamp(nv, 0.0, 1.0), 5.0);
    let v = rt_visibility(g.pos, g.clip);
    let sky_seen = select(rt_sky_level(g.light.x), max(v.z, 0.0),
        v.z >= 0.0);
    let sun = globals.rt_sun_light.rgb * globals.rt_light.w * v.x * rt_cloud_shadow(g.pos);
    // Reflected: the sky that way, faded as it dips under the face, or what the rays met off the
    // face, read where the facet's normal sends the reflection instead (as far off as a few
    // blocks away would be).
    let mirrored = reflect(ray, n);
    let horizon_fade = clamp(1.0 + 1.1 * dot(mirrored, g.face), 0.0, 1.0);
    var reflected = rt_sky(mirrored) * max(sky_seen, 0.15 * v.x) * horizon_fade * horizon_fade;
    if globals.rt_motion.w > 1.5 {
        let shift = rt_shift(g.pos, mirrored, reflect(ray, g.face), 4.0);
        var traced = rt_reflection(g.clip + shift, g.pos);
        if traced.a < 0.5 { traced = rt_reflection(g.clip, g.pos); }
        if traced.a > 0.5 {
            reflected = traced.rgb;
        } else {
            let seen = rt_screen_reflection(g.pos, mirrored);
            reflected = mix(reflected, seen.rgb, seen.a);
        }
    }
    // The sun's glint (GGX, the glass's roughness, a smooth face's at least 0.02).
    let l = globals.rt_light.xyz;
    let h = normalize(l + view);
    let nl = max(dot(n, l), 0.0);
    let alpha = max(roughness * roughness, 0.02);
    let a2 = alpha * alpha;
    let nh = max(dot(n, h), 0.0);
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let glint = a2 / (RT_PI * dd * dd) * 0.25 / max(nl * nv, 1e-3) * nl * RT_PI;
    reflected += sun * min(glint * smoothstep(0.0, 0.15, dot(g.face, l)), 200.0);
    // Through it: in bent by the facet, out through a flat face parallel to this one.
    let inward = refract(ray, n, 1.0 / index);
    var outward = refract(inward, g.face, index);
    if dot(outward, outward) < 1e-6 { outward = ray; }
    let bottom = rt_bottom(g.clip, ray);
    let behind = clamp(select(8.0, bottom - distance, bottom > 0.0), 0.05, 6.0);
    var refracted = vec4(0.0);
    var beam = 0.0;
    if globals.rt_motion.w > 1.5 {
        let centre = rt_refracted(g.clip);
        beam = max(centre.a - 1.0, 0.0);
        if centre.a > 0.0 {
            let at = g.clip + rt_shift(g.pos, outward, ray, behind);
            // A rough face spreads what it lets through over about its GGX alpha: four taps
            // around that point, turned by the pixel's noise (the temporal resolve smooths them).
            let spread = alpha * behind * 0.5;
            let far = g.pos + outward * behind;
            let t1 = normalize(cross(g.face, select(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0),
                abs(g.face.y) > 0.9)));
            let t2 = cross(g.face, t1);
            let base = rt_screen(far);
            let u = (rt_screen(far + t1 * spread).xy - base.xy) * base.z;
            let w = (rt_screen(far + t2 * spread).xy - base.xy) * base.z;
            var sum = vec4(0.0);
            let taps = select(1, 4, roughness > 0.1 && length(u) + length(w) > 1.0);
            let turn = rt_dither(g.clip) * 6.2831853;
            for (var i = 0; i < taps; i++) {
                let a = turn + f32(i) * 1.5707963;
                let r = select(0.0, 0.7, taps > 1);
                let s = rt_refracted(at + (u * cos(a) + w * sin(a)) * r);
                if s.a > 0.0 { sum += vec4(s.rgb, 1.0); }
            }
            refracted = select(vec4(centre.rgb, 1.0), vec4(sum.rgb / max(sum.a, 1.0), 1.0),
                sum.a > 0.0);
        }
    }
    var added = fresnel * reflected;
    var shows = vec3(1.0 - fresnel);
    // Ice's frost (`rt_ice`): its face's diffuse layer's opacity, more on the texture's light
    // streaks (bubbles, cracks). Traced, the ice's body is the rays' (shadow_rays_psr.wgsl
    // `ice_body`: Beer-Lambert over the whole length the ray runs inside the ice, so a hill of it
    // is opaque). Untraced, it is guessed from how far behind the next solid thing is drawn (at
    // most 6 blocks): what its bubbles scatter adds to the frost, what it absorbs (red most,
    // glacier blue) dims what shows through.
    var frost = weather_frost * 0.85;
    var body = vec3(1.0);
    if ice > 0u {
        let streak = smoothstep(0.3, 0.85, dot(g.albedo, vec3(0.2126, 0.7152, 0.0722)));
        if refracted.a > 0.0 {
            frost = select(0.3, 0.2, ice == 1u) + 0.25 * streak;
        } else {
            // The rays' extinction a block (shadow_rays_hw.wgsl `ice_extinction`, cloudy ice):
            // what is lost its bubbles scatter, the frost's layer.
            let sigma = select(vec3(4.5, 4.2, 4.0), vec3(2.2, 1.9, 1.7), ice == 1u);
            let kept = exp(-sigma * behind);
            let share = max(dot(kept, vec3(0.2126, 0.7152, 0.0722)), 1e-4);
            frost = select(0.35, 0.15, ice == 1u) + 0.3 * streak;
            body = kept / share;
            frost = 1.0 - (1.0 - frost) * share;
        }
    }
    if refracted.a <= 0.0 {
        // Untraced: the glass's colour over its measurement distance along the bent ray.
        let colour = select(clamp(g.albedo, vec3(0.0), vec3(1.0)), vec3(1.0), plain)
            * GLASS_TRANSMITTANCE;
        let slant = max(dot(inward, -g.face), 0.2);
        shows *= pow(max(colour, vec3(1e-3)), vec3(1.0 / slant));
    }
    // Ice's body: its absorption on what shows through, its frost in front of it.
    shows *= body * (1.0 - frost);
    // The air in front, as for the terrain: it dims both and adds the horizon's light.
    let f = rt_fade(g.spherical, g.cylindrical, g.pos) * globals.fog_color.a;
    let toward = normalize(vec3(g.pos.x, 0.0, g.pos.z) + vec3(0.0, 0.05 * length(g.pos.xz), 0.0));
    let horizon = rt_sky(toward) * sky_seen;
    let air = vec3(5.802e-3 + 4.44e-3, 13.558e-3 + 4.44e-3, 33.1e-3 + 4.44e-3);
    let air_through = exp(-air * (distance * 0.001 * globals.rt_mode.w)) * (1.0 - f);
    added = added * air_through + horizon * (1.0 - air_through);
    shows *= air_through;
    // Plain glass's diffuse layer, lit (and fogged) as the terrain is.
    if plain && g.alpha > 0.0 {
        let layer = rt_material_lit(RtMaterial(g.albedo, n, 1.0, 1.0, vec3(0.04), 0.0, 0.0, 0.0),
            g.face, g.pos, g.clip, g.light, g.spherical, g.cylindrical);
        added = mix(added, layer, g.alpha);
        shows *= 1.0 - g.alpha;
    }
    // Ice's frost, under the face's reflection: a pale blue-white layer, its texel's colour half
    // way to the white of snow, lit as the terrain is, the sun scattering through it from behind.
    if frost > 0.0 {
        // The weather's frost on glass is its own white, whatever the glass's texel.
        var white = mix(clamp(g.albedo, vec3(0.0), vec3(1.0)), vec3(0.86, 0.93, 1.0), 0.5);
        if ice == 0u { white = vec3(0.86, 0.9, 0.95); }
        let layer = rt_material_lit(RtMaterial(white, n, 1.0, 1.0, vec3(0.02), 0.0, 1.0, 0.0),
            g.face, g.pos, g.clip, g.light, g.spherical, g.cylindrical);
        added += (1.0 - fresnel) * frost * layer;
    }
    if refracted.a > 0.0 {
        // The sun behind the glass: its beam through both walls (their colour along the bent ray,
        // the far face's Fresnel and the second diffuse layer; the near face's are in `shows`),
        // spread over the lobe the faces' roughness gives it (GGX, each face turning the light by
        // (1 - 1/n) of its facets' tilt). Nothing is added: the glass reflects what Fresnel says,
        // lets through what its colour does and absorbs the rest, and this lobe is that let-through
        // sunlight seen from its direction (it sums to what the floor behind receives): a sharp sun
        // through clear glass, a wide glow through frosted glass.
        if beam > 0.0 {
            let colour = select(clamp(g.albedo, vec3(0.0), vec3(1.0)), vec3(1.0 - g.alpha), plain)
                * GLASS_TRANSMITTANCE;
            let slant = max(dot(inward, -g.face), 0.2);
            let walls = pow(max(colour, vec3(1e-3)), vec3(1.0 / slant)) * (1.0 - fresnel)
                * body * (1.0 - frost);
            let spread = max(alpha * 2.0 * (1.0 - 1.0 / index), 0.01);
            let s2 = spread * spread;
            let c = max(dot(ray, l), 0.0);
            let d = c * c * (s2 - 1.0) + 1.0;
            let lobe = min(s2 / (d * d), 200.0);
            refracted = vec4(refracted.rgb
                + globals.rt_sun_light.rgb * globals.rt_light.w * beam * walls * lobe, 1.0);
        }
        return WaterLight(added + shows * refracted.rgb, vec3(0.0), 1.0);
    }
    let opacity = clamp(1.0 - dot(shows, vec3(0.2126, 0.7152, 0.0722)), 0.02, 1.0);
    return WaterLight(added, shows, opacity);
}
// The plain terrain's glass: its texel, the face's tint, and the waviness for a normal.
fn rt_glass(in: TerrainOut, face: vec3<f32>, texel: vec4<f32>) -> WaterLight {
    rt_glass_neighbours = rt_glass_edges(in.tint.rgb);
    return rt_glass_at(GlassIn(in.pos, in.clip.xy, in.light, in.spherical, in.cylindrical, face,
        rt_glass_wave(in.pos, face), decode_gamma(texel.rgb * in.tint.rgb), texel.a, 0.0, in.layer));
}
// hdr.wgsl's brightness: the mean of luminance and the brightest channel, in
// linear light.
fn brightness(c: vec3<f32>) -> f32 {
    let l = pow(max(c, vec3(0.0)), vec3(2.2));
    return 0.5 * (dot(l, vec3(0.2126, 0.7152, 0.0722)) + max(l.r, max(l.g, l.b)));
}
// HDR output's mark (hdr.wgsl): the frame's alpha below 0.75, a band of 0.175 per
// tier (1 low .. 4 very bright, from 0.525 down to 0) and in it how much the pixel
// lifts, from its own brightness `y` before lighting through the tier's window.
fn hdr_mark(tier: u32, y: f32) -> f32 {
    let t = clamp(tier, 1u, 4u) - 1u;
    let lift = smoothstep(globals.hdr_windows[0][t], globals.hdr_windows[1][t], y);
    return 0.175 * f32(3u - t) + 0.165 * (1.0 - lift);
}
@fragment fn fs_solid(in: TerrainOut) -> @location(0) vec4<f32> {
    return solid_color(in);
}
// `fs_solid` for the opaque pass (`opaque_quad`): no fade, no cutout, no discard.
@fragment fn fs_opaque(in: TerrainOut) -> @location(0) vec4<f32> {
    if HDR_LIGHTING && globals.hdr_light.z != 0.0 {
        hdr_normal = normalize(cross(dpdy(in.local), dpdx(in.local)));
    }
    let texel = sprite(in.uv, in.layer & 0x7fffu);
    var color = texel * in.color;
    color = mix(globals.fog_color * vec4(1.0, 1.0, 1.0, color.a), color, in.visibility);
    if rt_solid_on() { return rt_solid(in, texel); }
    return solid_finish(in, texel, color);
}
// The water mirror (gfx.rs's reflection pass): what lies below the water plane
// (`fog_ends.w`, camera-relative) would show above it in the mirror, so it is cut away.
@fragment fn fs_solid_mirror(in: TerrainOut) -> @location(0) vec4<f32> {
    if in.height < globals.fog_ends.w { discard; }
    return solid_color(in);
}
fn solid_color(in: TerrainOut) -> vec4<f32> {
    let dx = dpdx(in.local);
    let dy = dpdy(in.local);
    if HDR_LIGHTING && globals.hdr_light.z != 0.0 {
        hdr_normal = normalize(cross(dy, dx));
    }
    if faded(in.fade, in.local, dx, dy) { discard; }
    // Bit 15 of the layer marks opaque leaves (forceOpaque): no alpha cutout.
    let texel = sprite(in.uv, in.layer & 0x7fffu);
    var color = texel * in.color;
    // terrain.fsh: the fade-in mixes toward the fog colour.
    color = mix(globals.fog_color * vec4(1.0, 1.0, 1.0, color.a), color, in.visibility);
    if color.a < 0.5 && (in.layer & 0x8000u) == 0u && !rt_lava_on(in.layer) { discard; }
    if rt_solid_on() { return rt_solid(in, texel); }
    return solid_finish(in, texel, color);
}
// The rest of a solid pixel's colour once it is kept: emission and fog.
fn solid_finish(in: TerrainOut, texel: vec4<f32>, color: vec4<f32>) -> vec4<f32> {
    return solid_finish_at(in, texel, color, dpdx(in.uv), dpdy(in.uv));
}
// `solid_finish` with the UV's screen gradients given (`sprite_at`).
fn solid_finish_at(in: TerrainOut, texel: vec4<f32>, color: vec4<f32>, du: vec2<f32>,
                   dv: vec2<f32>) -> vec4<f32> {
    // Below 0.75, alpha tells HDR output (hdr.wgsl) a light source's pixel
    // shines, at which tier (mesher::hdr_tier) and how much (hdr_mark), by the
    // texel's own brightness, so its light level and face shading don't change
    // it. Only pixels as bright as their sprite's threshold may: its bright
    // parts, or all of it. Nothing else reads the frame's alpha.
    // Fog turns a far surface into the fog's colour: it takes the fog's light with it, or a
    // fully fogged mountain would stand out from the fog behind it.
    // Without HDR output nothing reads the alpha, so its lighting is not worked out.
    var emissive = 1.0;
    if HDR_LIGHTING && globals.hdr_light.z != 0.0 {
        let fogged = max(linear_fog(in.spherical, globals.fog.x, globals.fog.y),
            linear_fog(in.cylindrical, globals.fog.z, globals.fog.w)) * globals.fog_color.a;
        emissive = hdr_scene_alpha(mix(hdr_scene(in.scene.x, in.scene.y, hdr_facing()),
            globals.hdr_light.y, fogged));
        if in.emission > 0u {
            // A light source's own face shines as one or stays as drawn, without the lighting.
            emissive = 1.0;
            let whole = in.emission > 4u;
            let tier = select(in.emission, in.emission - 4u, whole);
            let l = in.layer & 0x7fffu;
            let t = textureLoad(emission_thresholds, vec2(i32(l % SPRITE_SPLIT), i32(l / SPRITE_SPLIT)), 0);
            let y = brightness(texel.rgb);
            // All over means all over: the sprite's range is one frame's, and an animated sprite's
            // other frames dip below it. Fog hides a light source as it hides everything else.
            if whole || y >= t.r {
                emissive = hdr_mark(tier, y * (1.0 - fogged));
            }
        }
    }
    if in.emissive_layer != 0xffffffffu {
        // The high bit: an OptiFine overlay, drawn cutout (opaque where alpha >= 0.1).
        var mask = sprite_at(in.uv, in.emissive_layer & 0x7fffffffu, du, dv);
        if (in.emissive_layer & 0x80000000u) != 0u {
            mask.a = select(0.0, 1.0, mask.a >= 0.1);
        }
        return scene_finish(in, vec4(mix(color.rgb, mask.rgb, mask.a), mix(emissive, hdr_mark(3u, 1.0), mask.a)));
    }
    return scene_finish(in, vec4(color.rgb, emissive));
}
// A solid pixel's colour as the frame target takes it (`scene_color`); with Linear Light its alpha
// is the share of its light the rays' mask may take (`sun_share`, gfx/tonemap.wgsl).
fn scene_finish(in: TerrainOut, color: vec4<f32>) -> vec4<f32> {
    var out = scene_color(color, emission_gain(in.emission), in.spherical, in.cylindrical);
    if linear_on() { out.a = 0.5 * sun_share(in.light, in.emission); }
    return out;
}
// HDR lighting on what's mostly opaque among translucent faces (water), drawn again after
// them with only alpha written: `fs_translucent` blends its alpha as coverage, so a lake's
// value would be diluted. Clear glass leaves what's behind it, light sources included, alone.
@fragment fn fs_translucent_mark(in: TerrainOut) -> @location(0) vec4<f32> {
    let dx = dpdx(in.local);
    let dy = dpdy(in.local);
    hdr_normal = normalize(cross(dy, dx));
    if sprite(in.uv, in.layer & 0x7fffu).a < 0.6 { discard; }
    let fogged = max(linear_fog(in.spherical, globals.fog.x, globals.fog.y),
        linear_fog(in.cylindrical, globals.fog.z, globals.fog.w)) * globals.fog_color.a;
    let scene = mix(hdr_scene(in.scene.x, in.scene.y, hdr_facing()), globals.hdr_light.y, fogged);
    return vec4(0.0, 0.0, 0.0, hdr_scene_alpha(scene));
}
// Water, glass and other see-through faces draw whole as they fade in, the
// layer drawn first in front (gfx.rs's water tiers): dithered, the faded-out
// pixels showed the layer behind through the one in front, twice the water.
@fragment fn fs_translucent(in: TerrainOut) -> @location(0) vec4<f32> {
    let texel = sprite(in.uv, in.layer & 0x7fffu);
    var color = texel * in.color;
    color = mix(globals.fog_color * vec4(1.0, 1.0, 1.0, color.a), color, in.visibility);
    if color.a < 0.01 && !rt_glass_weathered(in) { discard; }
    // RT lighting: water its own way (`rt_water`), glass its own (`rt_glass`).
    if globals.rt_mode.x > 0.5 && linear_on() && in.emission == 0u {
        let n = face_normal(in.pos);
        if rt_is_water(in.layer) {
            let w = rt_water(in, n);
            return vec4(w.added / w.alpha, w.alpha);
        }
        let g = rt_glass(in, n, texel);
        return vec4(g.added / g.alpha, g.alpha);
    }
    return scene_color(color, emission_gain(in.emission), in.spherical, in.cylindrical);
}

// Native still-fluid boxes use block sprites and lightmap without terrain AO/emission
// packing or entity-model diffuse shading. The absolute sprite layer occupies glint UV x.
struct NativeFluidOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) sprite: u32,
    @location(3) spherical: f32,
    @location(4) cylindrical: f32,
}
@vertex fn vs_native_fluid(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(3) color: vec4<f32>, @location(4) light: vec4<u32>,
    @location(6) sprite: vec2<f32>) -> NativeFluidOut {
    let block = max(f32(light.y), dynamic_block_light(position));
    var lit = color * sample_lightmap(f32(light.x) / 16.0, block / 16.0);
    if globals.colored_count.x > 0u {
        lit = vec4(min(lit.rgb + color.rgb * native_colored_light(position), vec3(1.0)), lit.a);
    }
    return NativeFluidOut(globals.view_proj * vec4(position, 1.0), uv, lit, u32(sprite.x),
        length(position), max(length(position.xz), abs(position.y)));
}
@fragment fn fs_native_fluid(in: NativeFluidOut) -> @location(0) vec4<f32> {
    // The source translucent shader has no alpha discard; transparent texels write depth too.
    let color = sprite_grad(in.uv, in.sprite, dpdx(in.uv), dpdy(in.uv)) * in.color;
    return apply_fog(color, in.spherical, in.cylindrical);
}

// Native terrain models retain floating-point transformed normals. Fullbright still receives
// entity directional shading; the material's alpha test and mip policy are independent.
// Diffuse/normal policy: Flywheel 1.0.6, copyright2021-2024 Jozufozu (MIT);
// full permission/warranty notice in THIRD_PARTY_NOTICES.md.
override NATIVE_MODEL_ALPHA_CUTOFF: f32 = 0.0;
struct NativeModelOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) sprite: u32,
    @location(3) spherical: f32,
    @location(4) cylindrical: f32,
    @location(5) normal: vec3<f32>,
}
@vertex fn vs_native_model(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec4<u32>,
    @location(5) normal_z: u32, @location(6) normal_xy: vec2<f32>) -> NativeModelOut {
    let block = max(f32(light.y), dynamic_block_light(position));
    var lit = color * sample_lightmap(f32(light.x) / 16.0, block / 16.0);
    if globals.colored_count.x > 0u {
        lit = vec4(min(lit.rgb + color.rgb * native_colored_light(position), vec3(1.0)), lit.a);
    }
    return NativeModelOut(globals.view_proj * vec4(position, 1.0), uv, lit, layer >> 16u,
        length(position), max(length(position.xz), abs(position.y)),
        vec3(normal_xy, bitcast<f32>(normal_z)));
}
@fragment fn fs_native_model(in: NativeModelOut) -> @location(0) vec4<f32> {
    var color = sprite_grad(in.uv, in.sprite, dpdx(in.uv), dpdy(in.uv)) * in.color;
    if NATIVE_MODEL_ALPHA_CUTOFF > 0.0 && color.a < NATIVE_MODEL_ALPHA_CUTOFF { discard; }
    if any(in.normal != vec3(0.0)) {
        let light0 = normalize(vec3(0.2, 1.0, -0.7));
        let light1 = normalize(vec3(-0.2, select(1.0, -1.0, globals.time.y > 0.5), 0.7));
        let shade = min(1.0, (max(0.0, dot(light0, in.normal))
            + max(0.0, dot(light1, in.normal))) * 0.6 + 0.4);
        color = vec4(color.rgb * shade, color.a);
    }
    return apply_fog(color, in.spherical, in.cylindrical);
}

// Sky (SkyRenderer): the 10-vertex sky disc fan at y = 16 (radius 512) in
// the sky colour with sky fog (sky.fsh: fog 0..FogSkyEnd on the spherical
// distance, a step at FogSkyEnd on the cylindrical one), evaluated per pixel
// along the view ray with the fan's interpolated vertex distances; the
// clear colour (fog colour) elsewhere; the sunrise/sunset fan blended over
// it; the dark disc (y = -4) below the horizon when the camera is low.
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}
@vertex fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2(f32(i32(i & 1u) * 4 - 1), f32(i32(i >> 1u) * 4 - 1));
    var out: SkyOut;
    out.clip = vec4(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}
fn view_ray(ndc: vec2<f32>) -> vec3<f32> {
    return normalize((globals.inverse_view * vec4(ndc.x * globals.misc.z * globals.misc.w,
        ndc.y * globals.misc.w, -1.0, 0.0)).xyz);
}
const PI: f32 = 3.14159265;
// Interpolated (spherical, cylindrical) distance where the ray meets a disc
// fan at height y, or negative when it misses.
fn disc_distance(d: vec3<f32>, y: f32) -> vec2<f32> {
    if d.y * y <= 0.0 { return vec2(-1.0); }
    let p = d * (y / d.y);
    let h = length(p.xz);
    let phi = atan2(p.z, p.x);
    let sector = PI / 4.0;
    let delta = phi - (floor(phi / sector) * sector + sector * 0.5);
    let edge = 512.0 * cos(sector * 0.5) / cos(delta);
    if h > edge { return vec2(-1.0); }
    let s = h / edge;
    let ay = abs(y);
    let rim = sqrt(512.0 * 512.0 + y * y);
    return vec2(ay + s * (rim - ay), ay + s * (512.0 - ay));
}
fn sky_fog(color: vec3<f32>, dist: vec2<f32>) -> vec3<f32> {
    let end = globals.fog_ends.x;
    let f = max(linear_fog(dist.x, 0.0, end), linear_fog(dist.y, end, end));
    return mix(color, globals.fog_color.rgb, f * globals.fog_color.a);
}
// Weight of the fan centre where the ray crosses the sunrise fan (0 when missed):
// Moller-Trumbore from the origin along d, over normals built once a frame
// (gfx/sunrise_fan.rs). The barycentrics are apex/s, k1/s and -k0/s; s is -det.
fn sunrise_weight_full(d: vec3<f32>) -> f32 {
    if globals.sunrise.a <= 0.001 { return 0.0; }
    var best = 0.0;
    var k0 = dot(d, globals.sunrise_fan[16].xyz);
    for (var i = 0; i < 16; i++) {
        let triangle = globals.sunrise_fan[i];
        let k1 = dot(d, globals.sunrise_fan[16 + (i + 1) % 16].xyz);
        // A hit needs s on the side of w, so k0 and k1 on either side of 0.
        if k0 * triangle.w <= 0.0 && k1 * triangle.w >= 0.0 {
            let apex = dot(d, triangle.xyz);
            let s = apex + k1 - k0;
            if abs(s) >= 1e-6 && s * triangle.w > 0.0 && apex * s >= 0.0 && k1 * s >= 0.0
                && k0 * s <= 0.0 {
                best = max(best, apex / s);
            }
        }
        k0 = k1;
    }
    return best;
}
// The rim's YZ projection is an ellipse. Comparing its unsquashed X/Y
// magnitudes with tan(PI/8) selects one of sixteen sectors without atan2.
fn sunrise_triangle_weight(d: vec3<f32>, i: u32) -> f32 {
    let k0 = dot(d, globals.sunrise_fan[16u+i].xyz);
    let k1 = dot(d, globals.sunrise_fan[16u+(i+1u)%16u].xyz);
    let triangle = globals.sunrise_fan[i];
    let apex = dot(d, triangle.xyz);
    let s = apex + k1 - k0;
    if abs(s) >= 1e-6 && s * triangle.w > 0.0 && apex * s >= 0.0
        && k1 * s >= 0.0 && k0 * s <= 0.0 {
        return apex / s;
    }
    return 0.0;
}
fn sunrise_weight(d: vec3<f32>) -> f32 {
    if globals.sunrise.a <= 0.001 { return 0.0; }
    if dot(d.yz, d.yz) < 1e-10 { return sunrise_weight_full(d); }
    // The camera lies in the rim plane. A forward ray can reach the fan
    // only on the apex side. Keep a guard for rounded coplanar normals.
    let apex = dot(d, globals.sunrise_fan[0].xyz);
    if apex * globals.sunrise_fan[0].w < -1e-4 { return 0.0; }
    let y = dot(d, globals.sunrise_fan[16].xyz);
    let x = -dot(d, globals.sunrise_fan[20].xyz);
    let ax = abs(x);
    let ay = abs(y);
    let tangent = 0.414213562373095;
    let quadrant_sector = select(select(0u, 1u, ay > ax * tangent),
        select(2u, 3u, ax < ay * tangent), ay > ax);
    let sector = select(select(8u + quadrant_sector, 7u - quadrant_sector, y >= 0.0),
        select(15u - quadrant_sector, quadrant_sector, y >= 0.0), x >= 0.0);
    var best = max(sunrise_triangle_weight(d, sector),
        sunrise_triangle_weight(d, (sector + 8u) % 16u));
    // Test adjacent sectors only at a rounded boundary; their exact rim
    // normals remain the authority for hit tests and interpolation weights.
    let k0 = dot(d, globals.sunrise_fan[16u+sector].xyz);
    let k1 = dot(d, globals.sunrise_fan[16u+(sector+1u)%16u].xyz);
    if min(abs(k0), abs(k1)) < max(ax, ay) * 0.001 {
        best = max(best, sunrise_triangle_weight(d, (sector + 15u) % 16u));
        best = max(best, sunrise_triangle_weight(d, (sector + 1u) % 16u));
        best = max(best, sunrise_triangle_weight(d, (sector + 7u) % 16u));
        best = max(best, sunrise_triangle_weight(d, (sector + 9u) % 16u));
    }
    return best;
}
// The sky's colour along the view ray `d` (a shader pack's sky stage gets it as its input:
// gfx/native_sky.rs builds that stage from this section).
fn sky_along_ray(d: vec3<f32>) -> vec3<f32> {
    var color = sky_along_ray_base(d);
    let w = sunrise_weight(d);
    if w > 0.0 {
        color = mix(color, globals.sunrise.rgb, w * globals.sunrise.a);
    }
    return color;
}
// The sky without the sunrise fan: the renderer rasterizes the fan's sixteen
// triangles over this colour instead of testing them in every sky pixel.
fn sky_along_ray_base(d: vec3<f32>) -> vec3<f32> {
    var color = globals.fog_color.rgb;
    let dist = disc_distance(d, 16.0);
    if dist.x >= 0.0 {
        color = sky_fog(globals.sky_color.rgb, dist);
    }
    return color;
}
@fragment fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    // `hdr_scene_alpha`, spelled out: this part of the file also goes into shader packs' skies.
    return vec4(scene_rgb(sky_along_ray(view_ray(in.ndc))), 1.0 - 0.24 * globals.hdr_light.y);
}
@fragment fn fs_sky_base(in: SkyOut) -> @location(0) vec4<f32> {
    return vec4(scene_rgb(sky_along_ray_base(view_ray(in.ndc))), 1.0 - 0.24 * globals.hdr_light.y);
}
// The sunrise/sunset fan as geometry: the clip position is projected by the
// CPU with the sky's unjittered view, and the apex weight (1 at the apex, 0 at
// the rim) interpolates exactly the barycentric the per-pixel test computed.
struct SunriseOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) weight: f32,
}
@vertex fn vs_sunrise(@location(0) clip: vec4<f32>, @location(1) weight: f32) -> SunriseOut {
    return SunriseOut(clip, weight);
}
@fragment fn fs_sunrise(in: SunriseOut) -> @location(0) vec4<f32> {
    return vec4(globals.sunrise.rgb, in.weight * globals.sunrise.a);
}
// The first-person hand's depth clear inside the world pass: `vs_sky`'s triangle lies at the
// far plane, written over the whole depth target with no colour.
@fragment fn fs_depth_reset(in: SkyOut) -> @location(0) vec4<f32> {
    return vec4(0.0);
}
@fragment fn fs_dark_disc(in: SkyOut) -> @location(0) vec4<f32> {
    let d = view_ray(in.ndc);
    let dist = disc_distance(d, -4.0);
    if dist.x < 0.0 { discard; }
    return vec4(scene_rgb(sky_fog(vec3(0.0), dist)), 1.0);
}

// rendertype_lines.vsh: screen-space stroke and camera-space depth separation.
fn outline_clip(vertex: u32, start: vec3<f32>, end: vec3<f32>, width: f32) -> vec4<f32> {
    let position = select(start, end, vertex >= 2u);
    let normal = normalize(end - start);
    let view_shrink = 1.0 - 1.0 / 256.0;
    let line_start = globals.view_proj * vec4((position * globals.outline_params.w) * view_shrink, 1.0);
    let line_end = globals.view_proj * vec4(((position + normal) * globals.outline_params.w) * view_shrink, 1.0);
    let ndc1 = line_start.xyz / line_start.w;
    let ndc2 = line_end.xyz / line_end.w;
    let screen = globals.outline_params.xy;
    let direction = normalize((ndc2.xy - ndc1.xy) * screen);
    var offset = vec2(-direction.y, direction.x) * width / screen;
    if offset.x < 0.0 { offset = -offset; }
    if vertex % 2u != 0u { offset = -offset; }
    return vec4((ndc1 + vec3(offset, 0.0)) * line_start.w, line_start.w);
}
// How far the wind moves the hit outline's point at camera-relative `pos`: what `wind_offset`
// gives its block's vertices there (gfx/wind.rs `Frame::outline`: the block's class, its sky light
// and its foot's height in the header's last words). A plant bends by its height above its foot,
// as its sprite's v does; a leaf corner moves as the leaves' corner there (unpinned).
fn outline_wind(pos: vec3<f32>) -> vec3<f32> {
    let kind = wind_buf[29];
    if kind == 0u { return vec3(0.0); }
    let open = wind_open(wind_f(30u), length(pos));
    if open <= 0.0 { return vec3(0.0); }
    let world = wind_world(pos);
    let v = 1.0 - clamp(pos.y - wind_f(31u), 0.0, 1.0);
    return wind_shape(kind, world, wind_at(pos, world), vec2(0.0, v), false, open);
}
struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) spherical: f32,
    @location(1) cylindrical: f32,
    @location(2) color: vec4<f32>,
}
@vertex fn vs_line(@builtin(vertex_index) vertex: u32,
    @location(0) start: vec3<f32>, @location(1) end: vec3<f32>,
    @location(2) argb: vec4<f32>, @location(3) width: f32) -> LineOut {
    // The hit block's outline sways as its leaves or its plant do (`outline_wind`).
    let swayed_start = start + outline_wind(start);
    let swayed_end = end + outline_wind(end);
    let position = select(swayed_start, swayed_end, vertex >= 2u);
    var out: LineOut;
    out.clip = outline_clip(vertex, swayed_start, swayed_end, width);
    out.spherical = length(position);
    out.cylindrical = max(length(position.xz), abs(position.y));
    // ARGB in a little-endian u32 reads as B, G, R, A.
    out.color = argb.zyxw;
    return out;
}
@fragment fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    return scene_color(in.color, 1.0, in.spherical, in.cylindrical);
}
// Gizmo lines (rendertype_lines with a per-vertex colour and LineWidth in
// pixels; no layering transform).
struct DebugLineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) spherical: f32,
    @location(1) cylindrical: f32,
    @location(2) color: vec4<f32>,
}
@vertex fn vs_debug_line(@builtin(vertex_index) vertex: u32,
    @location(0) start: vec3<f32>, @location(1) end: vec3<f32>,
    @location(2) argb: vec4<f32>, @location(3) width: f32) -> DebugLineOut {
    let position = select(start, end, vertex >= 2u);
    let view_shrink = 1.0 - 1.0 / 256.0;
    let line_start = globals.view_proj * vec4(position * view_shrink, 1.0);
    let line_end = globals.view_proj * vec4((position + (end - start)) * view_shrink, 1.0);
    let ndc1 = line_start.xyz / line_start.w;
    let ndc2 = line_end.xyz / line_end.w;
    let screen = globals.outline_params.xy;
    let direction = normalize((ndc2.xy - ndc1.xy) * screen);
    var offset = vec2(-direction.y, direction.x) * width / screen;
    if offset.x < 0.0 { offset = -offset; }
    if vertex % 2u != 0u { offset = -offset; }
    var out: DebugLineOut;
    out.clip = vec4((ndc1 + vec3(offset, 0.0)) * line_start.w, line_start.w);
    out.spherical = length(position);
    out.cylindrical = max(length(position.xz), abs(position.y));
    // ARGB in a little-endian u32 reads as B, G, R, A.
    out.color = argb.zyxw;
    return out;
}
@fragment fn fs_debug_line(in: DebugLineOut) -> @location(0) vec4<f32> {
    return scene_color(in.color, 1.0, in.spherical, in.cylindrical);
}

// RT lighting's sky (gfx.rs draws it in place of fs_sky and the dark disc): the atmosphere's
// (gfx/atmosphere.rs) and the real sun's disc, in light; the Eye of Aldhani's streaks over it.
@fragment fn fs_rt_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let d = view_ray(in.ndc);
    let sky = rt_sky(d);
    let pixel = length(fwidth(d));
    // What lies beyond the Moon (the Milky Way) it hides, even new and dark; the air before it
    // not, nor the meteors burning in it, a hundred kilometres up, in front of the Moon. Behind
    // the rain's clouds (`Globals::rain`'s cover) the discs, the glare and the night's lights are
    // hidden.
    let beyond = rt_milky_way(d, sky, pixel) * rt_moon_hides(d, pixel);
    let behind = 1.0 - globals.rain.w;
    return vec4(sky + (rt_sun_disc(d) + rt_moon_disc(d) + rt_glare(d) + beyond
        + rt_meteor(d, sky, pixel) + aldhani_streaks(d, pixel)) * behind, 1.0);
}
// 0 toward the Moon's disc (above the horizon), 1 off it, over a pixel at its edge.
fn rt_moon_hides(d: vec3<f32>, pixel: f32) -> f32 {
    let m = normalize(globals.rt_moon.xyz);
    if m.y < -0.01 { return 1.0; }
    let off = acos(clamp(dot(d, m), -1.0, 1.0)) - globals.rt_moon.w;
    return smoothstep(-0.5 * pixel, 0.5 * pixel, off);
}
// RT lighting's real stars (gfx/sky_stars.rs): the Yale Bright Star Catalogue's 9096 stars down
// to magnitude 6.5 at their equatorial places, turned to the sky above the latitude the sun's
// path leans by (`rt_star_frame`), a small quad each added to the sky. Their light by their
// magnitude, their colour by their temperature; the atmosphere dims and reddens them toward the
// horizon, the sky's own light outshines them (none by day, the faint ones go first at dusk), and
// the air above makes them twinkle.
struct StarOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) light: vec3<f32>,
}
@vertex fn vs_rt_star(@builtin(vertex_index) vertex: u32, @builtin(instance_index) id: u32,
    @location(0) star: vec4<i32>) -> StarOut {
    var out: StarOut;
    out.clip = vec4(0.0, 0.0, 2.0, 1.0);
    let packed = u32(star.w) & 0xffffu;
    let magnitude = f32(packed & 255u) / 25.0 - 2.0;
    let kelvin = f32(packed >> 8u) * 100.0;
    let d = normalize((globals.rt_star_frame * vec4(vec3<f32>(star.xyz) / 32767.0, 0.0)).xyz);
    if globals.rt_stars.w <= 0.0 || d.y <= 0.0 { return out; }
    let airmass = 1.0 / max(d.y + 0.025, 0.03);
    return rt_sky_point(vertex, d, magnitude, star_colour(kelvin) * rt_scintillation(id, airmass));
}
// RT lighting's planets (gfx/sky_planets.rs): Mercury to Uranus where their orbits put them on
// the sky's date, turned with the stars, drawn like them by their magnitude, coloured by the
// sunlight their surfaces give back. They hardly twinkle: a planet's disc, a few to tens of
// arcseconds, spans many of the air's cells, whose flickers average out; only through the thick
// air near the horizon does a little of the star's flicker show.
@vertex fn vs_rt_planet(@builtin(vertex_index) vertex: u32, @builtin(instance_index) id: u32,
    @location(0) planet: vec4<f32>, @location(1) colour: vec4<f32>) -> StarOut {
    var out: StarOut;
    out.clip = vec4(0.0, 0.0, 2.0, 1.0);
    let d = normalize((globals.rt_star_frame * vec4(planet.xyz, 0.0)).xyz);
    if globals.rt_stars.w <= 0.0 || d.y <= 0.0 { return out; }
    let airmass = 1.0 / max(d.y + 0.025, 0.03);
    let low = 0.12 * smoothstep(10.0, 30.0, airmass);
    let twinkle = mix(vec3(1.0), rt_scintillation(id + 20011u, airmass), low);
    return rt_sky_point(vertex, d, planet.w, colour.rgb * twinkle);
}
// A point of light on the night sky (a star's, a planet's) toward `d` (above the horizon), of
// magnitude `magnitude` and colour `colour` (its flicker in it): its quad's corner `vertex`.
fn rt_sky_point(vertex: u32, d: vec3<f32>, magnitude: f32, colour: vec3<f32>) -> StarOut {
    var out: StarOut;
    out.clip = vec4(0.0, 0.0, 2.0, 1.0);
    let brightness = globals.rt_stars.w;
    // The Moon passes in front of the stars and planets (an occultation), dark when new.
    let moon = normalize(globals.rt_moon.xyz);
    if moon.y > -0.01 && dot(d, moon) > cos(globals.rt_moon.w) { return out; }
    let sky = rt_sky(d);
    let glare = exp(-dot(sky, vec3(0.2126, 0.7152, 0.0722)) * 60.0);
    // Through the air: airmass over the zenith's, more of the blue lost (the stars redden low).
    let airmass = 1.0 / max(d.y + 0.025, 0.03);
    let air = exp(-vec3(0.12, 0.18, 0.3) * airmass);
    // Paper whites at the quad's middle: a magnitude 0 star 20 (the eye adapted to the night),
    // each magnitude 2.5 times dimmer.
    let flux = 20.0 * pow(10.0, -0.4 * magnitude);
    out.light = colour * (flux * brightness * glare) * air;
    if max(out.light.r, max(out.light.g, out.light.b)) < 0.004 { return out; }
    let corners = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
        vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
    let corner = corners[vertex];
    let centre = globals.view_proj * vec4(d * 500.0, 1.0);
    // Two pixels across, a little more for the brightest.
    let radius = (1.2 + 0.6 * clamp(-magnitude * 0.5 + 0.5, 0.0, 2.0)) * 2.0 / globals.outline_params.xy;
    out.clip = vec4(centre.xy + corner * radius * centre.w, 0.0, centre.w);
    out.corner = corner;
    return out;
}
@fragment fn fs_rt_star(in: StarOut) -> @location(0) vec4<f32> {
    let falloff = exp(-dot(in.corner, in.corner) * 3.0);
    return vec4(in.light * falloff, 0.0);
}
// A star's scintillation, the light of each of red, green and blue: the air's turbulent cells,
// blown across its beam by the wind, focus and spread it a few to a few tens of times a second
// (a point of light: a planet's disc, wider than a cell's angle, averages them out and hardly
// twinkles). More through more air (its spread grows as airmass^1.75, Young's), and low down the
// air's dispersion splits the beam by colour, each crossing other cells: its colours flicker apart
// around its own, a white one flashing red, green and blue near the horizon, a red one mostly in
// its red. Its mean stays its light.
fn rt_scintillation(id: u32, airmass: f32) -> vec3<f32> {
    let t = globals.rt_water.z;
    let seed = f32(id);
    // Overhead the flicker is plain to see still (the eye catches a star's tenth), low down wild.
    let spread = clamp(0.16 + 0.05 * pow(airmass, 1.75), 0.16, 0.75);
    // The beams' parting, as time apart through the same moving cells: none overhead, a tenth of
    // a second toward the horizon (several of the flickers' periods).
    let lag = 0.1 * clamp((airmass - 1.5) / 8.0, 0.0, 1.0);
    let at = vec3(t - lag, t, t + lag);
    let phase = vec3(fract(seed * 0.618), fract(seed * 0.414), fract(seed * 0.302)) * 6.2831853;
    let wave = sin(at * (7.3 + 3.0 * fract(seed * 0.17)) * 6.2831853 + phase.x) * 0.5
        + sin(at * (13.1 + 5.0 * fract(seed * 0.29)) * 6.2831853 + phase.y) * 0.33
        + sin(at * (23.7 + 7.0 * fract(seed * 0.37)) * 6.2831853 + phase.z) * 0.17;
    return max(vec3(1.0) + spread * 1.6 * wave, vec3(0.0));
}
// A blackbody's colour at `kelvin`, normalized (Tanner Helland's fit), in light.
fn star_colour(kelvin: f32) -> vec3<f32> {
    let t = clamp(kelvin, 2000.0, 30000.0) / 100.0;
    var c = vec3(1.0);
    if t <= 66.0 {
        c.g = clamp(0.39008158 * log(t) - 0.63184144, 0.0, 1.0);
        c.b = select(clamp(0.54320679 * log(t - 10.0) - 1.19625409, 0.0, 1.0), 0.0, t <= 19.0);
    } else {
        c.r = clamp(1.29293619 * pow(t - 60.0, -0.1332047592), 0.0, 1.0);
        c.g = clamp(1.12989086 * pow(t - 60.0, -0.0755148492), 0.0, 1.0);
    }
    return pow(c, vec3(2.2));
}
// The Milky Way as it is: NASA's map of its diffuse light (its dark rifts, its bulge in
// Sagittarius, the Magellanic Clouds), turned with the stars (`rt_star_frame`, equatorial to
// world) and dimmed like them by the air and the sky's own light. Its brightness from its surface
// brightness: about 20 magnitudes a square arcsecond at the galactic centre (the map's 1), which
// the stars' scale (a magnitude 0 star 20 paper whites over a couple of pixels, 175 arcseconds
// across at 3440x1440) puts at 0.015 paper whites. The eye sums an extended light over its area
// (Piper's law: its threshold falls as the square root of the area, Blackwell 1946), which a
// screen's pixel does not, and the exposure stops at +1.5: the band is brought up by that
// summation, MILKY_SUMMATION 10 (about half the square root of a degree over a 175 arcsecond
// pixel): the naked eye's Milky Way under a dark country sky (Bortle 2 to 3, as France's would be
// without light pollution), plain but quiet, not a long exposure's. Less its floor away from the
// band (MILKY_FLOOR, the faint stars the sky draws itself), its rifts as the map has them, its
// colours as the eye sees so faint a light; the map's level the pixel covers (`pixel`, its angle), sharp up close and steady far.
const MILKY_SUMMATION: f32 = 10.0;
const MILKY_FLOOR: f32 = 0.005;
fn rt_milky_way(d: vec3<f32>, sky: vec3<f32>, pixel: f32) -> vec3<f32> {
    let brightness = globals.rt_stars.w;
    if brightness <= 0.0 || d.y <= 0.0 { return vec3(0.0); }
    let m = globals.rt_star_frame;
    let e = normalize(vec3(dot(d, m[0].xyz), dot(d, m[1].xyz), dot(d, m[2].xyz)));
    // Equatorial: z the celestial north pole, x the vernal equinox (right ascension 0).
    let ra = atan2(e.y, e.x);
    let dec = asin(clamp(e.z, -1.0, 1.0));
    let uv = vec2(fract(0.5 - ra / 6.2831853), 0.5 - dec / 3.14159265);
    let texel = 6.2831853 / f32(textureDimensions(rt_milky_map).x);
    let level = max(log2(pixel / texel), 0.0);
    let map = max(textureSampleLevel(rt_milky_map, lightmap_sampler, uv, level).rgb - MILKY_FLOOR,
        vec3(0.0));
    let glare = exp(-dot(sky, vec3(0.2126, 0.7152, 0.0722)) * 60.0);
    let air = exp(-vec3(0.12, 0.18, 0.3) * (1.0 / max(d.y + 0.025, 0.03)));
    // Seen as its own light is, not as the summation shows it: about a thousandth of a candela a
    // square metre, the rods' (tonemap.wgsl `night_vision`'s grey with its slight blue), the cones
    // only toward its brightest.
    let own = map * 0.015;
    let cones = smoothstep(log(0.002), log(0.03), log(max(dot(own, vec3(0.2126, 0.7152, 0.0722)),
        1e-7)));
    let tint = vec3(0.80, 0.90, 1.08);
    let rods = tint * (dot(own, vec3(0.05, 0.55, 0.40)) / dot(tint, vec3(0.2126, 0.7152, 0.0722)));
    return mix(rods, own, cones) * (MILKY_SUMMATION * brightness * glare) * air;
}
// The shooting stars' model (`rt_meteor`). A hash of `n` to 32 random bits (a PCG round), for the meteors' draws.
fn meteor_hash(n: u32) -> u32 {
    let state = n * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}
fn meteor_rand(n: u32) -> f32 {
    return f32(meteor_hash(n) >> 8u) / 16777216.0;
}
// A meteor, drawn from its slot's number `seed`: `since` seconds after the slot's start, in slots of
// `period` seconds each holding one with the chance `chance`. The sky of the stars' `frame` holds
// the shower's radiant (`rt_shower`).
struct Meteor {
    on: bool,
    // Seconds since it lit up.
    time: f32,
    // Toward where it lights up, and its heading (unit vectors), the km from the observer to it,
    // the km of its path, its speed in km/s.
    a: vec3<f32>,
    w: vec3<f32>,
    reach: f32,
    span: f32,
    speed: f32,
    // The magnitudes brighter than the faintest shown, and its peak light (nits).
    bright: f32,
    power: f32,
    // Its light curve: the peak's place along the path and the air's scale heights it falls
    // through, and a fragment's flare.
    peak_at: f32,
    fall: f32,
    flare_at: f32,
    flare: f32,
    breaks: bool,
    // Its composition: sodium, magnesium, iron, calcium.
    comp: vec4<f32>,
}
fn meteor_make(seed: u32, since: f32, period: f32, chance: f32, frame: mat4x4<f32>) -> Meteor {
    var o: Meteor;
    o.on = false;
    if meteor_rand(seed) > chance { return o; }
    o.time = since - period * meteor_rand(seed + 1u);
    if o.time < 0.0 { return o; }
    // Its stream: the shower's, in its share of the hour's meteors (`rt_shower.w` against the
    // sporadic `rt_stream.z`) times the sine of its radiant's height (a low radiant's meteors
    // spread over more air, the zenithal rate's correction; none while it is set), or a sporadic
    // one from a random radiant; where the meteoroid comes from.
    let radiant_sh = normalize((frame * vec4(globals.rt_shower.xyz, 0.0)).xyz);
    let sporadic = globals.rt_stream.z;
    let pick = meteor_rand(seed + 2u) * (sporadic + globals.rt_shower.w);
    let shower = pick >= sporadic;
    if shower && meteor_rand(seed + 18u) > radiant_sh.y { return o; }
    let ry = 0.12 + 0.88 * meteor_rand(seed + 3u);
    let ph = 6.2831853 * meteor_rand(seed + 4u);
    let rc = sqrt(1.0 - ry * ry);
    let radiant = select(vec3(rc * cos(ph), ry, rc * sin(ph)), radiant_sh, shower);
    let speed = select(14.0 + 56.0 * pow(meteor_rand(seed + 5u), 1.3), globals.rt_stream.x, shower);
    let index = select(2.8, globals.rt_stream.y, shower);
    // Magnitude from the population index (the count grows as index^magnitude), to 6 (the
    // hourly rates count to 6.5): many faint, a few bright, a fireball (-4) one in a few hundred.
    let faintest = 6.0;
    let magnitude = max(faintest + log(max(meteor_rand(seed + 6u), 1e-4)) / log(index), -9.0);
    o.bright = max(2.2 - magnitude, 0.0);
    let top = min(78.0 + 0.6 * speed + 8.0 * meteor_rand(seed + 7u) + 0.8 * min(o.bright, 8.0), 125.0);
    let bottom = max(top - 15.0 - 5.0 * o.bright, 52.0);
    o.span = min((top - bottom) / radiant.y, 4.0 * speed);
    o.speed = speed;
    // Where it lights up: anywhere in the sky.
    let az = 6.2831853 * meteor_rand(seed + 8u);
    let el = radians(15.0 + 65.0 * meteor_rand(seed + 9u));
    let a = vec3(cos(el) * cos(az), sin(el), cos(el) * sin(az));
    o.a = a;
    o.w = -radiant;
    o.reach = top / a.y;
    // It falls `top - bottom` km, the air's density growing by e every 7 km (the scale height),
    // and its light peaks where the air is a third as dense as where it burns out.
    o.fall = (top - bottom) / 7.0;
    o.peak_at = clamp(1.0 - 1.0986 / o.fall, 0.3, 0.9);
    o.breaks = meteor_rand(seed + 11u) < 0.25;
    o.flare_at = 0.3 + 0.6 * meteor_rand(seed + 12u);
    o.flare = 0.8 + 2.0 * meteor_rand(seed + 13u);
    // A long one (a grazer, its radiant low) burns the longest and brightest: up to three times
    // the light, into HDR's highlights.
    let long = smoothstep(60.0, 200.0, o.span);
    o.power = min(50.0 * pow(10.0, -0.4 * magnitude), 300.0) * (1.0 + 2.0 * long);
    o.comp = vec4(meteor_rand(seed + 14u), meteor_rand(seed + 15u), meteor_rand(seed + 16u), 0.7 * meteor_rand(seed + 17u));
    o.on = true;
    return o;
}
// Its light along its path (`x` 0 to 1, 1 at its end), 1 at most: the classical single-body curve,
// L / Lmax = 9/4 (rho / rho_max) (1 - rho / (3 rho_max))^2 in the air's density rho, which grows
// exponentially as it falls: a long faint rise (a needle's thin tip where it lights up), a peak late
// on and an abrupt end where the meteoroid is spent (rho three times the peak's); a fragment of a
// quarter flares at some point and ends soon after.
fn meteor_curve(m: Meteor, x: f32) -> f32 {
    let rho = exp((x - m.peak_at) * m.fall);
    var curve = select(0.0, 2.25 * rho * (1.0 - rho / 3.0) * (1.0 - rho / 3.0), rho < 3.0);
    if m.breaks {
        let q = (x - m.flare_at) / 0.025;
        curve = (curve + m.flare * exp(-q * q)) * exp(-max(x - m.flare_at - 0.06, 0.0) / 0.01) / (1.0 + 0.5 * m.flare);
    }
    return curve;
}
// Its colour (largest channel 1) at `x`: sodium orange-yellow rising along the path, magnesium
// blue-green early, iron yellow-white, calcium violet and the air's red, by composition, the
// speed's excitation and the brightness (the faint ones look white).
fn meteor_colour(m: Meteor, x: f32) -> vec3<f32> {
    let hot = clamp((m.speed - 18.0) / 45.0, 0.0, 1.0);
    let w_na = m.comp.x * (0.25 + 1.3 * x) * (1.4 - hot);
    let w_mg = m.comp.y * (1.4 - 0.9 * x) * (0.4 + hot);
    let w_fe = m.comp.z * 0.8;
    let w_ca = m.comp.w * hot * (1.4 - 1.2 * x);
    let w_red = hot * hot * max(0.35 - x, 0.0) * 3.0 + (1.0 - hot) * x * x * 0.8;
    // A fireball (magnitude -4 and brighter, one in a few hundred): the magnesium and oxygen's
    // blue-green head, glaring, its end turning to sodium's orange-yellow.
    let fire = smoothstep(4.0, 8.0, m.bright);
    let w_na_f = w_na + fire * 1.6 * x * x;
    let w_mg_f = w_mg + fire * 2.5 * (1.2 - x);
    var colour = (w_na_f * vec3(1.0, 0.6, 0.15) + w_mg_f * vec3(0.3, 1.0, 0.6) + w_fe * vec3(1.0, 0.9, 0.65)
        + w_ca * vec3(0.6, 0.45, 1.0) + w_red * vec3(1.0, 0.3, 0.2)) / (w_na_f + w_mg_f + w_fe + w_ca + w_red + 1e-3);
    colour = colour / max(max(colour.r, colour.g), max(colour.b, 1e-3));
    return mix(vec3(1.0), colour, clamp(0.45 + 0.08 * m.bright, 0.0, 0.9));
}
// Toward its head `along` km down its path.
fn meteor_direction(m: Meteor, along: f32) -> vec3<f32> {
    return normalize(m.reach * m.a + along * m.w);
}
// Shooting stars. The time is cut into slots of `rt_stars.y` seconds, each holding a meteor with the
// chance `rt_stars.z`; everything about it is drawn from the
// slot's number (`meteor_make`), so no frame carries any state and a quiet sky costs a few hashes.
// A meteoroid falls along a straight line, parallel to the others of its stream, away from the
// stream's radiant: the year's showers on their dates (`sky_stars::meteors`: the Quadrantids,
// Perseids, Geminids and the rest, at their zenithal hourly rates, an outburst or a storm some
// years) stand fixed among the stars, so a shower is only there while its radiant is above the
// horizon, as many as the sine of its height says, its meteors short near the radiant and long far
// from it, which the perspective of the line gives for free; the rest are sporadic (about ten an
// hour, a random radiant, 14 to 70 km/s, index 2.8). Its magnitude follows the population index (many faint, few bright; the mass
// follows), and sets how deep it goes: it lights up at 80 to 120 km (higher when faster) and burns
// out 15 km lower (a bright one, 5 km more per magnitude). The light is the classical single-body
// curve, the glow stays behind the head for a tenth of a second, so the streak is the head's few
// kilometres; the colour changes along the path with its composition; the brightest, fast ones
// leave a faint green train (the oxygen line at 557.7 nm) that lasts seconds. Dimmed and reddened
// by the air toward the horizon, outshone by the sky's own light like the stars, and none by day.
fn rt_meteor(d: vec3<f32>, sky: vec3<f32>, pixel: f32) -> vec3<f32> {
    let period = globals.rt_stars.y;
    if period <= 0.0 || globals.rt_stars.w <= 0.0 || d.y <= 0.0 { return vec3(0.0); }
    let now = globals.rt_water.z;
    let slot0 = u32(now / period);
    let slots = 2u + u32(8.0 / period);
    var sum = vec3(0.0);
    for (var k = 0u; k < slots; k++) {
        if k > slot0 { break; }
        let slot = slot0 - k;
        let m = meteor_make(slot * 32u + 1u, now - f32(slot) * period, period, globals.rt_stars.z,
            globals.rt_star_frame);
        if m.on { sum += meteor_streak(m, d, pixel); }
    }
    if all(sum == vec3(0.0)) { return sum; }
    let airmass = 1.0 / max(d.y + 0.025, 0.03);
    let air = exp(-vec3(0.12, 0.18, 0.3) * airmass);
    let glare = exp(-dot(sky, vec3(0.2126, 0.7152, 0.0722)) * 60.0);
    return sum * air * (glare * globals.rt_stars.w);
}
// The light, in nits, of meteor `m` at the pixel looking toward `d`, `pixel` radians wide: what a
// long exposure and the eye see. The head is a sharp point, over-exposed, in a soft halo; the trail
// behind it fades and tapers, green just behind a fast head (the oxygen and magnesium lines) and
// orange-red further back (sodium, the air's nitrogen); a fragment's flare throws sparks that lag
// behind the head and drift off the line; the brightest, fast ones leave a persistent train that
// widens and drifts with the winds at 90 km for seconds. The lines are Gaussian, a pixel wide at
// least (the eye's own blur, an arcminute, where pixels are smaller: VR, ultrawide), and keep their
// light as a whole whatever their width, so a thin one stays continuous and steady under the TAA's
// jitter instead of stair-stepping or shimmering.
fn meteor_streak(m: Meteor, d: vec3<f32>, pixel: f32) -> vec3<f32> {
    // It is seen along its great circle: `phi` the angle along it from where it lights up, `across`
    // the angle off it.
    let wa = dot(m.w, m.a);
    let wv = m.w - wa * m.a;
    let we = length(wv);
    if we < 1e-3 { return vec3(0.0); }
    let e = wv / we;
    let n = cross(m.a, e);
    let across = dot(d, n);
    let big = min(m.bright, 8.0);
    // The band it can light: the halo's and the train's drift's reach.
    if abs(across) > pixel * (14.0 + 2.5 * big) + 0.004 { return vec3(0.0); }
    // The width (a Gaussian's sigma): a pixel's at least, the eye's blur where pixels are smaller,
    // wider when bright; `gain` keeps the light as a whole where the width is not a pixel's.
    // A short one is a smaller meteoroid, less matter burning: a thinner line.
    let thin = mix(0.55, 1.0, smoothstep(10.0, 60.0, m.span));
    let sigma = max(pixel * (0.62 + 0.04 * big), 0.00025 * (1.0 + 0.1 * big)) * thin;
    let gain = 0.62 * pixel / sigma;
    let s2 = 2.0 * sigma * sigma;
    let a2 = across * across;
    // The head: where it is now (gone past the end of its path, its light curve is zero), a point
    // brighter than the trail, in a halo of a few pixels, more when bright (the glare).
    let along_h = min(m.time * m.speed, m.span);
    let x_h = along_h / m.span;
    let to_h = m.reach * m.a + along_h * m.w;
    let dh = to_h / length(to_h);
    let r2 = dot(d - dh, d - dh);
    let halo_sigma = sigma * (2.5 + 0.25 * big);
    // Only a bright one shows a head brighter than its trail, in a glare; a faint one is a needle
    // to its tip.
    let bold = smoothstep(0.5, 4.0, m.bright);
    let head = gain * gain * mix(0.7, 2.5, bold) * exp(-r2 / s2)
        + bold * (0.03 + 0.01 * big) * exp(-r2 / (2.0 * halo_sigma * halo_sigma));
    var light = mix(meteor_colour(m, x_h), vec3(1.0), 0.5) * (m.power * meteor_curve(m, x_h) * head);
    // A fireball's shock layer (the Eye's `aldhani_streak` says why): only a big one has it (a
    // metre-sized body below about 100 km; small grains stay free-molecular), its air ionised a
    // seventh of the body's radius ahead of it, glowing blue-white as the eighth power of the
    // speed: the glare in front of the head, where the terminal burst blooms as it breaks up.
    let fireball = smoothstep(4.0, 7.0, m.bright);
    let shock = fireball * clamp(pow(m.speed / 45.0, 8.0), 0.03, 4.0);
    let burst = fireball * exp(-pow((m.time - m.span / m.speed) / 0.1, 2.0));
    let to_next = m.reach * m.a + (along_h + 1.0) * m.w;
    let ahead = normalize(to_next / length(to_next) - dh);
    let bow = normalize(dh + ahead * (sigma * (1.5 + 2.0 * burst)));
    let bow2 = dot(d - bow, d - bow);
    let glare_sigma = halo_sigma * (1.0 + 2.5 * burst);
    light += mix(meteor_colour(m, x_h), vec3(0.75, 0.85, 1.0), 0.5) * (m.power
        * (shock * 0.05 * meteor_curve(m, x_h) + burst * 0.4) * exp(-bow2 / (2.0 * glare_sigma * glare_sigma)));
    // The trail: the glow the head left along the path, fading behind it.
    let phi = atan2(dot(d, e), dot(d, m.a));
    let theta = atan2(we, wa);
    let rest = sin(theta - phi);
    if phi > 0.0 && phi < theta && rest > 1e-5 {
        let along = m.reach * sin(phi) / rest;
        let age = m.time - along / m.speed;
        if along <= m.span && age > 0.0 {
            let x = along / m.span;
            let dist = m.reach * sin(theta) / rest;
            let curve = meteor_curve(m, x);
            let glow = 0.07 + 0.03 * min(m.bright, 6.0);
            // Its end at the head, smoothed over a pixel (no stair-step at the tip).
            let tip = smoothstep(0.0, pixel * dist / m.speed, age);
            let wake = exp(-age / glow) * tip;
            // Its colour: the composition's at the head, green just behind a fast one (the oxygen
            // and magnesium lines), then the ember of sodium and the air's nitrogen.
            let hot = clamp((m.speed - 18.0) / 45.0, 0.0, 1.0);
            var colour = meteor_colour(m, x);
            colour = mix(colour, vec3(0.4, 1.0, 0.55),
                (0.45 * hot) * clamp(age / (0.5 * glow), 0.0, 1.0) * exp(-age / 0.7));
            colour = mix(colour, vec3(1.0, 0.42, 0.18), 0.55 * clamp(age / glow - 0.7, 0.0, 1.0));
            // As wide as it is bright (a brighter stretch's glare is wider, in the eye and in a
            // photograph): a needle, thin where it lit up and where its wake fades behind the
            // head, widest at its brightest; widening a little as the wake diffuses. A bright one
            // in a soft halo.
            let lit = sqrt(clamp(curve * wake, 0.0, 1.0));
            let st = sigma * (0.4 + 0.8 * lit) * (1.0 + 0.2 * min(age / glow, 2.0));
            let line = (0.62 * pixel / sigma) * exp(-a2 / (2.0 * st * st))
                + 0.03 * clamp(m.bright / 4.0, 0.0, 1.0) * exp(-a2 / (32.0 * sigma * sigma));
            light += colour * (m.power * curve * wake * line);
            // The train of a bright, fast one: faint, green (the oxygen line at 557.7 nm), seconds;
            // it widens as it diffuses, drifts with the winds at 90 km (a pixel or two a second), and
            // is uneven along its length.
            let train = clamp((m.speed - 30.0) / 30.0, 0.0, 1.0)
                * clamp((m.bright - 3.0) / 4.0, 0.0, 1.0);
            if train > 0.0 {
                let lasts = 1.2 + 0.35 * m.bright;
                let drift = (m.comp.w / 0.7 - 0.5) * 0.0012 * age;
                let off = across - drift;
                let stt = 2.0 * sigma + 0.0005 * age;
                let wide = (0.62 * pixel / stt) * exp(-off * off / (2.0 * stt * stt));
                let wisp = 0.75 + 0.25 * sin(along * (0.9 + m.comp.z) + 6.0 * m.comp.x);
                light += vec3(0.25, 1.0, 0.4)
                    * (train * 0.004 * m.power * sqrt(max(curve, 0.0)) * exp(-age / lasts) * wide * wisp);
            }
        }
    }
    // A fragment's sparks: three pieces thrown at the flare, slower than the body so they lag
    // behind the head, a pixel or two off the line, orange (sodium, iron), out in a fifth of a second.
    if m.breaks && m.bright > 1.5 {
        let age_f = m.time - m.flare_at * m.span / m.speed;
        if age_f > 0.0 && age_f < 0.6 {
            for (var k = 0u; k < 3u; k++) {
                let r1 = fract(m.comp.x * 7.31 + f32(k) * 0.618);
                let r2k = fract(m.comp.y * 5.17 + f32(k) * 0.618);
                let along_s = m.flare_at * m.span + m.speed * age_f * (0.55 + 0.35 * r1);
                let to_s = m.reach * m.a + along_s * m.w;
                let ds = normalize(to_s / length(to_s) + n * ((r2k - 0.5) * 0.012 * age_f));
                let q2 = dot(d - ds, d - ds);
                light += vec3(1.0, 0.65, 0.3) * (m.power * m.flare * 0.12 * exp(-age_f / (0.08 + 0.06 * r1))
                    * gain * gain * exp(-q2 / s2));
            }
        }
    }
    return light;
}
// RT lighting's rain drops (gfx/rain.rs, rain.rs): instanced streaks, no vertex data. The tracks
// of RAIN_NEAR² one-block cells around the camera, then RAIN_FAR² four-block ones, RAIN_TRACKS
// (RAIN_FAR_TRACKS) a cell, each drop's size, speed and phase hashed from its cell's world coordinates: the same drops
// for both eyes. A band RAIN_*_BAND tall around the camera holds the falling drops.
const RAIN_NEAR: u32 = 64u;
const RAIN_FAR: u32 = 32u;
const RAIN_TRACKS: u32 = 40u;
const RAIN_FAR_TRACKS: u32 = 24u;
const RAIN_NEAR_BAND: f32 = 16.0;
const RAIN_FAR_BAND: f32 = 48.0;
// The smallest drop drawn and the largest, in mm: the smaller ones only veil the distance (the
// froxels' medium), a larger one breaks up as it falls.
const RAIN_DROP_MIN: f32 = 0.5;
const RAIN_DROP_MAX: f32 = 6.0;
// A streak is the drop's path over the eye's exposure (s, about its persistence): a fast drop a
// line, drizzle near a dot.
const RAIN_EXPOSURE: f32 = 0.033;
// How long the circle a drop leaves where it lands lasts (s), and how wide it grows (blocks).
const RAIN_SPLASH: f32 = 0.5;
const RAIN_CIRCLE: f32 = 0.12;
struct RainOut {
    @builtin(position) clip: vec4<f32>,
    // A streak: across it (-1 to 1) and along it (0 its head, 1 its tail); an impact's circle: its
    // quad (-1 to 1 both ways).
    @location(0) uv: vec2<f32>,
    // The light the drop sends to the eye (paper whites), and how much of the pixel it covers.
    @location(1) colour: vec3<f32>,
    @location(2) alpha: f32,
    // 0 a streak; the circle an impact leaves, 1 + its age (0 to 1).
    @location(3) splash: f32,
    // An impact's circle: the world point under the fragment (x, z, `rain_world`'s) and the
    // ground it lies on (y), so that it shows on that column's top alone, not hanging past its
    // edge over lower ground or into a neighbour's leaves.
    @location(4) world: vec3<f32>,
}
// The sky table toward `d` (`rt_sky` without the Eye of Aldhani's glow, which the vertex stage
// does not see).
fn rain_sky(d: vec3<f32>) -> vec3<f32> {
    let flat = d.xz / max(length(d.xz), 1e-5);
    let sun = globals.rt_sun.xz / max(length(globals.rt_sun.xz), 1e-5);
    let azimuth = acos(clamp(dot(flat, sun), -1.0, 1.0)) / 3.14159265;
    let e = asin(clamp(d.y, -1.0, 1.0)) / 1.5707963;
    let v = 0.5 + 0.5 * sign(e) * sqrt(abs(e));
    return textureSampleLevel(rt_sky_view, lightmap_sampler, vec2(azimuth, v), 0.0).rgb;
}
// The light a drop at world `p` sends toward the eye along `view` (from the eye): a sphere of
// water shows the scene behind it bent and turned over through a wide angle, mostly the sky's
// light, and reflects a little of it; the sun (the moon) shines through it when the eye looks
// toward it (refraction's forward lobe, Henyey-Greenstein g 0.8) and comes back at 40-42 degrees
// from the antisolar point in colour (the rainbow), unless the columns toward it stand above the
// drop; the light sources light it by the block light above its column (one level less a block
// up) and the moving ones.
fn rain_drop_light(p: vec3<f32>, view: vec3<f32>, column: u32, ground: f32) -> vec3<f32> {
    // Through a sphere of water the scene behind shows turned over: the sky above the eye's line
    // comes out below it, so a drop shows the sky's light whatever is behind it.
    let sky = rain_sky(normalize(vec3(view.x, 0.8, view.z))) * 1.1
        + rain_sky(vec3(0.0, 1.0, 0.0)) * 0.3;
    let l = globals.rt_light.xyz;
    var lit = globals.rt_sun_light.rgb * globals.rt_light.w;
    for (var k = 1; k <= 6; k++) {
        let q = p + l * (f32(k) * 2.5);
        let w = rain_cell(vec2<i32>(floor(q.xz)));
        if w != 0u && rain_height(w) > q.y {
            lit = vec3(0.0);
            break;
        }
    }
    let c = dot(view, l);
    let g = 0.8;
    let forward = (1.0 - g * g) / (12.566 * pow(1.0 + g * g - 2.0 * g * c, 1.5));
    let bow = degrees(acos(clamp(-c, -1.0, 1.0)));
    let off = (vec3(bow) - vec3(42.3, 41.6, 40.8)) / 0.7;
    let rainbow = exp(-off * off) * 0.25;
    var level = rain_light(column) - max(p.y - ground, 0.0);
    let rel = p - rain_world(vec3(0.0));
    let count = min(u32(globals.time.z), 16u);
    for (var i = 0u; i < count; i++) {
        let d = globals.dynamic_lights[i];
        level = max(level, d.w - distance(rel, d.xyz));
    }
    let torch = BLOCK_LIGHT * pow(clamp(level / 15.0, 0.0, 1.0), 2.0) * 0.5;
    return sky + lit * (forward * 6.0 + rainbow) + torch;
}
// Where a drop stops over a column of the rain map (`rain_cell`): its top, a water's a little under.
fn rain_ground(column: u32) -> f32 {
    return rain_top(column) - select(0.0, 0.11, rain_surface(column) == 10u);
}
// Where a drop's leaning path first met what stops it (`rain_landing`): `kind` 0 nowhere (in the
// air down to its height), 1 on the top of `column` at `ground`, over `at` (x, z), 2 on a column's
// side or above the band's top (born under cover): no circle.
struct RainLanding {
    kind: u32,
    column: u32,
    ground: f32,
    at: vec2<f32>,
}
// The drop whose path is `start - lean * (h - base)` (x, z at height h), now at height `y`, born at
// `top`: the columns its path crossed, walked back up from `y` cell by cell. In each column the
// path spends heights `low` to `high`; it struck the column whose ground stands at `low` or above,
// on its top when the ground is under `high`, else on its side (it came in below the top). The
// highest such column is the first it met coming down. RAIN_LANDING_CELLS columns at most, an
// unknown one ends the walk.
const RAIN_LANDING_CELLS: i32 = 40;
fn rain_landing(start: vec2<f32>, lean: vec2<f32>, base: f32, y: f32, top: f32) -> RainLanding {
    var r: RainLanding;
    r.kind = 0u;
    r.column = 0u;
    r.ground = 0.0;
    r.at = vec2(0.0);
    let p = start - lean * (y - base);
    // Climbing a block moves the path by `-lean`: the heights at which it next crosses a cell's
    // edge in x and in z, and the climb between two edges.
    let d = -lean;
    let still = abs(d) < vec2(1e-5);
    let delta = 1.0 / max(abs(d), vec2(1e-5));
    let fl = floor(p);
    let ahead = select(p - fl, fl + 1.0 - p, d >= vec2(0.0));
    var next = select(y + ahead * delta, vec2(1e9), still);
    let step = vec2<i32>(select(-1, 1, d.x >= 0.0), select(-1, 1, d.y >= 0.0));
    var cell = vec2<i32>(fl);
    var low = y;
    for (var i = 0; i < RAIN_LANDING_CELLS; i++) {
        let c = rain_cell(cell);
        if c == 0u { break; }
        let high = min(min(next.x, next.y), top);
        let g = rain_ground(c);
        if g >= low {
            r.column = c;
            r.ground = g;
            r.kind = select(2u, 1u, g < high);
            r.at = start - lean * (g - base);
        }
        if high >= top { break; }
        low = high;
        if next.x < next.y {
            cell.x += step.x;
            next.x += delta.x;
        } else {
            cell.y += step.y;
            next.y += delta.y;
        }
    }
    return r;
}
@vertex fn vs_rain(@builtin(vertex_index) vertex: u32, @builtin(instance_index) id: u32)
    -> RainOut {
    var out: RainOut;
    out.clip = vec4(2.0, 2.0, 2.0, 1.0);
    out.uv = vec2(0.0);
    out.colour = vec3(0.0);
    out.alpha = 0.0;
    out.splash = 0.0;
    out.world = vec3(0.0);
    let rate = globals.rain.x;
    if rate <= 0.0 { return out; }
    let near_count = RAIN_NEAR * RAIN_NEAR * RAIN_TRACKS;
    let near = id < near_count;
    let side = select(RAIN_FAR, RAIN_NEAR, near);
    let size = select(4.0, 1.0, near);
    let band = select(RAIN_FAR_BAND, RAIN_NEAR_BAND, near);
    let local = select(id - near_count, id, near);
    let tracks = select(RAIN_FAR_TRACKS, RAIN_TRACKS, near);
    let index = local / tracks;
    let camera = rain_world(vec3(0.0));
    let origin = vec2<i32>(floor(camera.xz / size)) - i32(side / 2u);
    let cell = origin + vec2(i32(index % side), i32(index / side));
    let seed = rain_seed(cell, local % tracks + select(64u, 0u, near));
    // Marshall-Palmer at this rate: the drops over RAIN_DROP_MIN a cubic metre, against a 50 mm/h
    // downpour's, decide how many tracks hold one.
    let slope = 4.1 * pow(rate, -0.21);
    let heaviest = 4.1 * pow(50.0, -0.21);
    let share = (exp(-slope * RAIN_DROP_MIN) / slope) / (exp(-heaviest * RAIN_DROP_MIN) / heaviest);
    if rain_rand(seed) > share { return out; }
    // Its diameter from that exponential law between the bounds, its terminal velocity (Atlas et
    // al. 1973, rain.rs `terminal_velocity`).
    let span = 1.0 - exp(-slope * (RAIN_DROP_MAX - RAIN_DROP_MIN));
    let diameter = RAIN_DROP_MIN - log(1.0 - rain_rand(seed + 1u) * span) / slope;
    let speed = max(9.65 - 10.3 * exp(-0.6 * diameter), 0.5);
    let wind = globals.rain_sun.zw;
    let lean = wind / speed;
    // Its height in the band, falling with the time, and where it is, leaning with the wind.
    let t = globals.rt_water.z;
    let y = camera.y - band * 0.5 + band * fract(rain_rand(seed + 4u) - t * speed / band);
    let start = (vec2<f32>(cell) + vec2(rain_rand(seed + 2u), rain_rand(seed + 3u))) * size;
    let xz = start - lean * (y - camera.y);
    var column = rain_cell(vec2<i32>(floor(xz)));
    if column == 0u { return out; }
    // It stops on the first block that stops motion (vanilla's rain heightmap: leaves stop it,
    // grass and flowers let it through to the ground), a water's top a little under it, and leaves
    // a circle there for a moment, on that column's top alone.
    var ground = rain_ground(column);
    var head = vec3(xz.x, y, xz.y);
    var splash = -1.0;
    let land = rain_landing(start, lean, camera.y, y, camera.y + band * 0.5);
    if land.kind == 0u {
        // In the air all the way from the band's top: it falls on, if it rains here.
        if !rain_falls(column) { return out; }
    } else {
        // Stopped on the way: on a column's side (a wall, a trunk, a canopy's edge) or born under
        // cover it leaves no circle; on a top, a circle where its path met it, not under it now.
        if land.kind != 1u || !rain_falls(land.column) { return out; }
        column = land.column;
        ground = land.ground;
        let since = (ground - y) / speed;
        if !near || since > RAIN_SPLASH { return out; }
        splash = since / RAIN_SPLASH;
        head = vec3(land.at.x, ground, land.at.y);
    }
    let rel = head - camera;
    let distance = length(rel);
    // Faded out toward the grid's edge and the band's ends, and right at the eye.
    let reach = f32(side) * size * 0.5;
    var fade = 1.0 - smoothstep(reach * 0.7, reach * 0.95, length(rel.xz));
    if !near { fade *= smoothstep(26.0, 31.0, length(rel.xz)); }
    fade *= 1.0 - smoothstep(band * 0.36, band * 0.48, abs(rel.y));
    fade *= smoothstep(0.25, 0.9, distance);
    if fade <= 0.0 { return out; }
    let pixel = rain_footprint(rel);
    let toward = rel / max(distance, 1e-4);
    out.colour = rain_drop_light(head, toward, column, ground);
    // HORIZON_RT_DEBUG=10: the drops in magenta.
    if globals.rt_mode.y > 3.5 && globals.rt_mode.y < 4.5 { out.colour = vec3(4.0, 0.0, 4.0); }
    let corner = vec2(f32(vertex & 1u) * 2.0 - 1.0, f32(vertex >> 1u));
    // The streak: its path over the exposure, a pixel wide at least. A drop covers a streak's
    // pixel for its diameter over the streak's length of the exposure (Garg and Nayar 2006), less
    // when it is thinner than the pixel.
    let streak = speed * RAIN_EXPOSURE + 0.01;
    let width = max(diameter * 0.001, pixel);
    let opacity = clamp(60.0 * diameter * 0.001 / streak, 0.0, 0.6)
        * sqrt(diameter * 0.001 / width);
    if splash >= 0.0 {
        // On standing water (a pond, a puddle) a drop leaves no mark of its own: it gives its
        // momentum to the surface, whose own rings carry it (`rain_ripples`).
        let level = max(globals.rain.z, globals.rain_sun.y);
        if rain_surface(column) == 10u || rain_puddle(head, level, 0.5) > 0.3 { return out; }
        // On the ground, the circle its water spreads into, flat and widening: a quad lying
        // there, its ring drawn by fs_rain; larger drops leave wider ones.
        let extent = RAIN_CIRCLE * (0.6 + 0.2 * diameter);
        let at = rel + vec3(corner.x * extent, 0.006, (corner.y * 2.0 - 1.0) * extent);
        out.clip = globals.view_proj * vec4(at, 1.0);
        out.uv = vec2(corner.x, corner.y * 2.0 - 1.0);
        out.splash = 1.0 + splash;
        out.world = vec3(head.x + corner.x * extent, ground,
            head.z + (corner.y * 2.0 - 1.0) * extent);
        // The same water over a growing ring is a thinner film: as opaque as the drop at first,
        // then fainter as the ring's area grows (its radius and width, as fs_rain draws them,
        // against theirs at the first frames; a thin film shows less than in proportion to its
        // thickness), never more than the pixels it spans.
        let ring = (0.05 + 0.9 * sqrt(splash)) * (0.05 + 0.08 * splash);
        out.alpha = opacity * fade * pow(min(1.0, 0.0035 / ring), 0.6)
            * min(1.0, extent * 0.08 / pixel);
        return out;
    }
    // A thin quad along the streak, turned to the eye about it.
    let down = normalize(vec3(wind.x, -speed, wind.y));
    let tail = rel - down * streak;
    let across = normalize(cross(down, (rel + tail) * 0.5) + vec3(0.0, 1e-5, 0.0));
    let at = mix(rel + down * pixel, tail - down * pixel, corner.y)
        + across * (corner.x * width * 0.5);
    out.clip = globals.view_proj * vec4(at, 1.0);
    out.uv = corner;
    out.alpha = opacity * fade;
    return out;
}
@fragment fn fs_rain(in: RainOut) -> @location(0) vec4<f32> {
    var a = in.alpha;
    if in.splash > 0.5 {
        // Only on the top it landed on: the part of the circle past its column's edge is over
        // another column (lower ground, a wall's side, a neighbour's leaves) and not there.
        let column = rain_cell(vec2<i32>(floor(in.world.xz)));
        if column == 0u || abs(rain_ground(column) - in.world.y) > 0.01 { discard; }
        // A thin circle spreading from the impact (fast at first, as a capillary ring slows),
        // widening and fading as it goes.
        let age = in.splash - 1.0;
        let radius = sqrt(age) * 0.9;
        let width = 0.05 + 0.08 * age;
        let x = (length(in.uv) - radius) / width;
        a *= exp(-x * x) * (1.0 - age) * (1.0 - age);
    } else {
        a *= (1.0 - in.uv.x * in.uv.x) * smoothstep(0.0, 0.12, in.uv.y)
            * (1.0 - smoothstep(0.75, 1.0, in.uv.y));
    }
    return vec4(in.colour * a, a);
}
// RT lighting's snow flakes (gfx/snow.rs, snow.rs): instanced quads, no vertex data. The tracks of
// SNOW_NEAR² one-block cells, then SNOW_FAR² four-block ones, SNOW_TRACKS (SNOW_FAR_TRACKS) a
// cell; the cells tile the world (a tile SNOW_NEAR, or 4 SNOW_FAR, blocks wide, and SNOW_*_BAND
// tall), every tile the same, and each flake is drawn at the copy of its place nearest the camera:
// the wind blows the field through the grid with no edge to empty from, and the flakes stay put in
// the world as the head moves (a band that followed the camera would carry them up and down with
// it, as the rain's does, unseen at 8 m/s, not at 1). Each flake's size, speed and spiral are
// hashed from its cell in the tile: the same flakes for both eyes.
const SNOW_NEAR: u32 = 48u;
const SNOW_FAR: u32 = 32u;
const SNOW_TRACKS: u32 = 64u;
const SNOW_FAR_TRACKS: u32 = 32u;
const SNOW_NEAR_BAND: f32 = 12.0;
const SNOW_FAR_BAND: f32 = 24.0;
// The smallest flake drawn and the largest, as their water's diameter (mm): the smaller ones only
// veil the distance (the froxels' snow medium). A flake is four times as wide (snow.rs
// `flake_diameter`): 4 mm to 2 cm.
const SNOW_MELT_MIN: f32 = 1.0;
const SNOW_MELT_MAX: f32 = 5.0;
// How much larger a flake is drawn than it is: a game's snow, seen as snow (a real flake, a few
// millimetres, is lost in a headset's pixels past a couple of metres). It falls at its real size's
// speed.
const SNOW_LOOK: f32 = 3.0;
// The rate whose flakes fill every track (mm/h of water: a heavy blizzard).
const SNOW_HEAVIEST: f32 = 8.0;
// A flake's blur is its path over the eye's exposure (s), shorter than a drop's streak.
const SNOW_EXPOSURE: f32 = 0.02;
// How long (s) the wind's change where a flake is carries it: a gust pushes the flakes it meets
// from then on, it doesn't move their whole fall at once.
const SNOW_GUST_MEMORY: f32 = 1.5;
// How long (s) a flake rests where it lands before it joins the cover.
const SNOW_REST: f32 = 1.2;
struct SnowOut {
    @builtin(position) clip: vec4<f32>,
    // Across the flake (-1 to 1) and along its blur (0 its head, 1 its tail).
    @location(0) uv: vec2<f32>,
    // The light it sends to the eye (paper whites), how much of the pixel it covers at most, and
    // the share of the quad's length each round end takes.
    @location(1) colour: vec3<f32>,
    @location(2) alpha: f32,
    @location(3) cap: f32,
}
// `v` brought within half a `period` of 0, both components.
fn snow_wrap(v: vec2<f32>, period: f32) -> vec2<f32> { return v - period * floor(v / period + 0.5); }
// Where the flake of tile cell `start` (world, in the tile) is drawn after falling `fallen` s in
// the prevailing wind `base` and, for the last SNOW_GUST_MEMORY s, the wind `wind` where it is, its
// spiral at `flutter`: the copy nearest the camera's `camera` (x, z).
fn snow_place(start: vec2<f32>, base: vec2<f32>, wind: vec2<f32>, fallen: f32, flutter: vec2<f32>,
    camera: vec2<f32>, period: f32) -> vec2<f32> {
    let drift = base * fallen + (wind - base) * min(fallen, SNOW_GUST_MEMORY);
    return camera + snow_wrap(start + drift + flutter - camera, period);
}
// The light a flake at world `p` sends toward the eye along `view` (from the eye): a small white
// body, not a lens. The sky's light around it (the sky above and the sky behind the eye), the sun
// (the moon) by a diffuse sphere's phase (Lambert's sphere: full with the sun behind the eye,
// dark with it behind the flake) plus the crystals' forward lobe (the flakes lit up against the
// low sun), unless the columns toward it stand above the flake; now and then, near, a crystal's
// glint as it tumbles; the light sources by the block light above its column and the moving ones.
fn snow_flake_light(p: vec3<f32>, view: vec3<f32>, column: u32, ground: f32, seed: u32, near: bool)
    -> vec3<f32> {
    let sky = rain_sky(vec3(0.0, 1.0, 0.0)) * 0.55
        + rain_sky(normalize(vec3(-view.x, 0.35, -view.z))) * 0.45;
    let l = globals.rt_light.xyz;
    var lit = globals.rt_sun_light.rgb * globals.rt_light.w;
    for (var k = 1; k <= 6; k++) {
        let q = p + l * (f32(k) * 2.5);
        let w = rain_cell(vec2<i32>(floor(q.xz)));
        if w != 0u && rain_height(w) > q.y {
            lit = vec3(0.0);
            break;
        }
    }
    let c = dot(view, l);
    let a = acos(clamp(-c, -1.0, 1.0));
    let sphere = (sin(a) + (3.14159265 - a) * cos(a)) / 3.14159265;
    let g = 0.7;
    let forward = (1.0 - g * g) / (12.566 * pow(1.0 + g * g - 2.0 * g * c, 1.5));
    var glint = 0.0;
    if near {
        let tick = u32(floor(globals.rt_water.z * 7.0 + 7.0 * rain_rand(seed + 9u)));
        if rain_rand(seed * 31u + tick) > 0.985 { glint = 3.0; }
    }
    var level = rain_light(column) - max(p.y - ground, 0.0);
    let rel = p - rain_world(vec3(0.0));
    let count = min(u32(globals.time.z), 16u);
    for (var i = 0u; i < count; i++) {
        let d = globals.dynamic_lights[i];
        level = max(level, d.w - distance(rel, d.xyz));
    }
    let torch = BLOCK_LIGHT * pow(clamp(level / 15.0, 0.0, 1.0), 2.0) * 0.8;
    return 0.9 * (sky + torch) + lit * (0.6 * sphere + 0.5 * forward + glint);
}
@vertex fn vs_snow(@builtin(vertex_index) vertex: u32, @builtin(instance_index) id: u32)
    -> SnowOut {
    var out: SnowOut;
    out.clip = vec4(2.0, 2.0, 2.0, 1.0);
    out.uv = vec2(0.0);
    out.colour = vec3(0.0);
    out.alpha = 0.0;
    out.cap = 0.5;
    let rate = globals.linear.z;
    if rate <= 0.0 { return out; }
    let near_count = SNOW_NEAR * SNOW_NEAR * SNOW_TRACKS;
    let near = id < near_count;
    let side = select(SNOW_FAR, SNOW_NEAR, near);
    let size = select(4.0, 1.0, near);
    let band = select(SNOW_FAR_BAND, SNOW_NEAR_BAND, near);
    let local = select(id - near_count, id, near);
    let tracks = select(SNOW_FAR_TRACKS, SNOW_TRACKS, near);
    let index = local / tracks;
    let period = f32(side) * size;
    let tile = vec2(i32(index % side), i32(index / side));
    let seed = rain_seed(tile, local % tracks + select(224u, 160u, near));
    // More flakes the harder it snows, and larger (Gunn and Marshall's sizes, snow.rs `slope`).
    if rain_rand(seed) > clamp(pow(rate / SNOW_HEAVIEST, 0.45), 0.06, 1.0) { return out; }
    let slope = 2.55 * pow(rate, -0.48);
    let span = 1.0 - exp(-slope * (SNOW_MELT_MAX - SNOW_MELT_MIN));
    let melted = SNOW_MELT_MIN - log(1.0 - rain_rand(seed + 1u) * span) / slope;
    let diameter = 4.0 * melted;
    // Locatelli and Hobbs' aggregates (snow.rs `terminal_velocity`), each a little its own.
    let speed = 0.8 * pow(diameter, 0.16) * (0.85 + 0.3 * rain_rand(seed + 5u));
    let t = globals.rt_water.z;
    let camera = rain_world(vec3(0.0));
    // Its height in its tile's band, falling with the time (`phase` 1 at the band's top, 0 at its
    // foot), the copy nearest the camera, and the seconds since it left the band's top.
    let phase = fract(rain_rand(seed + 4u) - t * speed / band);
    let dy = band * phase - camera.y;
    let y = camera.y + dy - band * floor(dy / band + 0.5);
    let fallen = (1.0 - phase) * band / speed;
    // Its spiral as it tumbles: a few centimetres across, turning one way or the other; in a
    // blizzard's wind the eddies swirl it about over tens of centimetres.
    let radius = (0.02 + 0.08 * rain_rand(seed + 6u)) * (1.0 + 0.5 * length(globals.rain_sun.zw));
    let spin = (1.5 + 3.0 * rain_rand(seed + 7u)) * select(-1.0, 1.0, rain_rand(seed + 8u) > 0.5);
    let turn = 6.2831853 * rain_rand(seed + 9u);
    let flutter = radius * vec2(cos(t * spin + turn), sin(t * spin + turn));
    let start = (vec2<f32>(tile) + vec2(rain_rand(seed + 2u), rain_rand(seed + 3u))) * size;
    let base = globals.rain_sun.zw;
    var wind = base;
    var xz = snow_place(start, base, base, fallen, flutter, camera.xz, period);
    // Near, the wind the terrain makes where it is (gfx/wind.rs) and its gust carry it.
    if near && wind_f(10u) > 0.0 {
        let here = vec3(xz.x, y, xz.y);
        let w = wind_at(here - camera, here);
        wind = w.xz * (0.8 + 0.6 * w.y);
        xz = snow_place(start, base, wind, fallen, flutter, camera.xz, period);
    }
    var column = rain_cell(vec2<i32>(floor(xz)));
    if column == 0u || !snow_falls(column) { return out; }
    var ground = rain_top(column);
    var head = vec3(xz.x, y, xz.y);
    var vel = vec3(wind.x, -speed, wind.y)
        + vec3(-flutter.y, 0.0, flutter.x) * spin;
    var rest = 1.0;
    if y < ground {
        // It landed `since` s ago: near, it rests where it landed a moment (not on water, where
        // it melts), then joins the cover.
        let since = (ground - y) / speed;
        if !near || since > SNOW_REST { return out; }
        let landed = fallen - since;
        let then = t - since;
        let still = radius * vec2(cos(then * spin + turn), sin(then * spin + turn));
        let at = snow_place(start, base, wind, landed, still, camera.xz, period);
        column = rain_cell(vec2<i32>(floor(at)));
        if column == 0u || !snow_falls(column) || rain_surface(column) == 10u { return out; }
        ground = rain_top(column);
        head = vec3(at.x, ground + 0.004, at.y);
        vel = vec3(0.0);
        rest = 1.0 - since / SNOW_REST;
    }
    let rel = head - camera;
    let distance = length(rel);
    // Faded out toward the tiles' seams (the grid's edge, the band's ends), between the grids,
    // where its fall wraps (from the band's foot to its top, which moves it sideways) and right at
    // the eye.
    let reach = period * 0.5;
    var fade = 1.0 - smoothstep(reach * 0.7, reach * 0.95, length(rel.xz));
    if !near { fade *= smoothstep(18.0, 23.0, length(rel.xz)); }
    fade *= 1.0 - smoothstep(band * 0.36, band * 0.48, abs(rel.y));
    if rest >= 1.0 { fade *= smoothstep(0.0, 0.06, phase) * (1.0 - smoothstep(0.94, 1.0, phase)); }
    fade *= smoothstep(0.15, 0.6, distance) * rest;
    if fade <= 0.0 { return out; }
    let pixel = rain_footprint(rel);
    // A flake well under a pixel is left to the froxels' veil: drawn, the far ones would lay a
    // grey haze of faint dots over the whole view.
    let d = diameter * SNOW_LOOK * 0.001;
    if d < 0.4 * pixel { return out; }
    let toward = rel / max(distance, 1e-4);
    out.colour = snow_flake_light(head, toward, column, ground, seed, near);
    // HORIZON_RT_DEBUG=10: the flakes in cyan.
    if globals.rt_mode.y > 3.5 && globals.rt_mode.y < 4.5 { out.colour = vec3(0.0, 4.0, 4.0); }
    let corner = vec2(f32(vertex & 1u) * 2.0 - 1.0, f32(vertex >> 1u));
    // A round flake facing the eye, blurred along its path over the exposure as much as the path
    // crosses the view, a pixel wide at least: its coverage spread over the quad (whose soft disc
    // fills about 0.6 of it).
    let width = max(d, pixel);
    let moving = length(vel);
    var axis = select(vec3(0.0, -1.0, 0.0), vel / max(moving, 1e-5), moving > 1e-4);
    axis -= toward * dot(axis, toward);
    let crossing = length(axis);
    if crossing > 1e-3 {
        axis /= crossing;
    } else {
        axis = normalize(cross(toward, vec3(1.0, 0.0, 0.0)) + vec3(0.0, 1e-4, 0.0));
    }
    let side_axis = normalize(cross(axis, toward));
    let total = moving * SNOW_EXPOSURE * crossing + width;
    let at = rel + axis * (width * 0.5 - corner.y * total) + side_axis * (corner.x * width * 0.5);
    out.clip = globals.view_proj * vec4(at, 1.0);
    out.uv = corner;
    out.cap = clamp(width * 0.5 / total, 1e-3, 0.5);
    out.alpha = min(0.785 * d * d / (width * total) / 0.6, 0.95) * fade;
    return out;
}
@fragment fn fs_snow(in: SnowOut) -> @location(0) vec4<f32> {
    let along = max(max(in.cap - in.uv.y, in.uv.y - (1.0 - in.cap)), 0.0) / in.cap;
    let r = length(vec2(in.uv.x, along));
    let a = in.alpha * (1.0 - smoothstep(0.5, 1.0, r));
    return vec4(in.colour * a, a);
}
