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
    // y how far the sea's horizon dips under the horizontal, in radians.
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
    let size = vec2<f32>(textureDimensions(rt_reflect));
    let at = clamp(vec2<i32>(pixel / globals.rt_motion.xy * size), vec2(0), vec2<i32>(size) - 1);
    return textureLoad(rt_reflect, at, 0);
}
// The view's last resolved frame (gfx/taa.rs `last_color`, linear light; 1x1 without the temporal
// resolve), for the screen-space reflections.
@group(0) @binding(24) var rt_scene: texture_2d<f32>;
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
fn sprite(uv: vec2<f32>, layer: u32) -> vec4<f32> {
    if SPRITE_ARRAYS == 1u {
        return textureSample(sprites, sprite_sampler, uv, i32(layer));
    }
    // The array is chosen per fragment: sample with the derivatives taken here.
    let dx = dpdx(uv);
    let dy = dpdy(uv);
    let l = i32(layer % SPRITE_SPLIT);
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

struct TerrainOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) color: vec4<f32>,
    @location(3) spherical: f32,
    @location(4) cylindrical: f32,
    // The light the face emits, 0 to 15 (mesher.rs's `EMISSION_BITS`).
    @location(5) @interpolate(flat) emission: u32,
    // A distant tile's fade (`terrain_fade`): 1 drawn whole.
    @location(6) @interpolate(flat) fade: f32,
    // For ray-traced shadows: the sky and block light levels (0..15) the lightmap is read at.
    @location(7) light: vec2<f32>,
    // The camera-relative position.
    @location(9) pos: vec3<f32>,
    // For RT lighting: the face's tint (its albedo with the sprite's texel).
    @location(8) tint: vec3<f32>,
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
fn sample_lightmap(sky: f32, block: f32) -> vec4<f32> {
    let uv = vec2(block, sky) * 16.0;
    return textureSampleLevel(lightmap, lightmap_sampler,
        clamp(uv / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0)), 0.0);
}
// mesher.rs's TerrainVertex holds the UV in fixed point; the instance's
// origin.w is the step: 1/32768 per unit for sections (a power of two on an
// integer below 2^24, so it decodes exactly), 1/32 per block for distant
// terrain tiles (TerrainVertex::tiled). A distant tile fading in or out
// (gfx/lod.rs `instance`) has 2 + progress or 4 + progress there instead,
// with the step of 1/32.
fn terrain_uv(packed: vec2<u32>, w: f32) -> vec2<f32> {
    return vec2<f32>(packed) * select(w, 1.0 / 32.0, w >= 1.0);
}
// The fade an instance's origin.w gives: below 4 the part of the pattern drawn, from its
// start (1: all of it); 4 and over, 4 + the part left out, from its start.
fn terrain_fade(w: f32) -> f32 {
    if w < 2.0 { return 1.0; }
    if w < 4.0 { return w - 2.0; }
    return w;
}
// Whether a fragment at `pixel` of a fading tile is drawn: a 4x4 ordered dither, so a tile
// fading in and the one fading out with the same progress draw opposite pixels.
fn faded_out(fade: f32, pixel: vec2<f32>) -> bool {
    let p = vec2<u32>(pixel) & vec2(3u);
    let bayer = array<u32, 16>(0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u);
    let t = (f32(bayer[p.y * 4u + p.x]) + 0.5) / 16.0;
    if fade < 4.0 { return t >= fade; }
    return t < fade - 4.0;
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
// `pos` as drawn: lowered when its instance's origin.w says a distant tile (`terrain_uv`).
fn earth_curved(pos: vec3<f32>, w: f32) -> vec3<f32> {
    let flat_sq = globals.earth.x;
    if flat_sq < 0.0 || w < 1.0 / 64.0 { return pos; }
    return vec3(pos.x, pos.y - earth_drop(pos.xz, flat_sq), pos.z);
}
@vertex fn vs_terrain(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>,
    @location(5) origin: vec4<f32>) -> TerrainOut {
    var out = terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin);
    out.fade = terrain_fade(origin.w);
    return out;
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
    var out = terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin);
    out.fade = terrain_fade(origin.w);
    return out;
}
fn terrain_vertex(position: vec3<f32>, uv: vec2<f32>, layer: u32, color: vec4<f32>,
    light: vec2<u32>, origin: vec4<f32>) -> TerrainOut {
    let pos = earth_curved(position + origin.xyz, origin.w);
    var out: TerrainOut;
    out.clip = globals.view_proj * vec4(pos, 1.0);
    out.uv = uv;
    out.layer = layer;
    let block = max(f32(light.y), dynamic_block_light(pos));
    // With RT lighting the vertex holds its tint alone and its shade, to four bits, in the alpha's
    // high nibble: vanilla's colour, for where the rays don't reach, is their product.
    var shade = 1.0;
    if globals.rt_mode.x > 0.5 {
        shade = (f32(u32(round(color.a * 255.0)) >> 4u) * 16.0 + 8.0) / 255.0;
    }
    out.tint = color.rgb;
    out.pos = pos;
    out.color = vec4(color.rgb * shade, 1.0) * sample_lightmap(f32(light.x) / 16.0, block / 16.0);
    out.light = vec2(f32(light.x), block) / 16.0;
    out.emission = 15u - (u32(round(color.a * 255.0)) & 15u);
    out.spherical = length(pos);
    out.cylindrical = max(length(pos.xz), abs(pos.y));
    out.fade = 1.0;
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
fn linear_on() -> bool { return globals.time.w > 0.5; }
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
fn rt_visibility(pos: vec3<f32>, pixel: vec2<f32>) -> vec3<f32> {
    // This frame's rays, traced on the depth prepass (rt_motion.xy the world target's size): the
    // mask at this very pixel.
    if globals.rt_motion.w > 1.5 {
        let size = vec2<f32>(textureDimensions(rt_history));
        let at = clamp(vec2<i32>(pixel / globals.rt_motion.xy * size), vec2(0), vec2<i32>(size) - 1);
        return textureLoad(rt_history, at, 0).rgb;
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
// they light, pale and the same everywhere, thicker toward the radiant, and the eye itself, a
// bright glow on the horizon around the radiant, white-hot in its middle.
fn aldhani_glow(d: vec3<f32>, t: f32) -> vec3<f32> {
    let strength = aldhani_envelope(t);
    if strength <= 0.0 { return vec3(0.0); }
    let c = clamp(dot(d, ALDHANI_RADIANT), -1.0, 1.0);
    let theta = acos(c);
    let up = max(d.y, 0.0);
    let pale = vec3(0.72, 0.9, 0.9);
    let sky = pale * (1.2 * (0.35 + 0.65 * (0.5 + 0.5 * c)) * (0.55 + 0.45 * exp(-up * 2.0)));
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
// RT lighting: the eye's veiling glare around the sun (by night the moon) toward `d`, what makes
// a disc half a degree across look as big as it does: Stiles and Holladay's law, a luminance of
// 10 E / theta^2 at theta degrees from a source of illuminance E (here E the light a white face
// turned to it gets, times pi), faded out past 30 degrees and inside the disc.
fn rt_glare(d: vec3<f32>) -> vec3<f32> {
    let toward = normalize(select(globals.rt_sun.xyz, globals.rt_moon.xyz, globals.rt_mode.z > 0.5));
    if toward.y < -0.05 { return vec3(0.0); }
    let theta = degrees(acos(clamp(dot(d, toward), -1.0, 1.0)));
    let radius = degrees(globals.rt_sun_light.w);
    let at = max(theta, radius);
    let fade = 1.0 - smoothstep(10.0, 30.0, theta);
    // The moon's a tenth as much: under the night sky the eye sees it sharp, its glare would hide
    // the disc the night's exposure lifts.
    let eye = select(1.0, 0.1, globals.rt_mode.z > 0.5);
    return globals.rt_sun_light.rgb * (10.0 * 3.14159265 * eye / (at * at)) * fade
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
    let earthshine = 3.0 * 4e-4 * earth_lit;
    return tint * (albedo / 0.12) * (3.0 * lit + earthshine) * air;
}
// RT lighting: the light sources' traced light at this pixel (`rt_blocks`), this frame's rays only.
// RT lighting: the sun's colour through stained glass at this pixel (`rt_sun_tint`), divided by the
// visibility the mask already holds; white without this frame's rays.
fn rt_sun_colour(pixel: vec2<f32>, visible: f32) -> vec3<f32> {
    if globals.rt_motion.w < 1.5 { return vec3(1.0); }
    let size = vec2<f32>(textureDimensions(rt_sun_tint));
    let at = clamp(vec2<i32>(pixel / globals.rt_motion.xy * size), vec2(0), vec2<i32>(size) - 1);
    return textureLoad(rt_sun_tint, at, 0).rgb / max(visible, 1e-3);
}
fn rt_traced(pixel: vec2<f32>) -> vec4<f32> {
    if globals.rt_motion.w < 1.5 { return vec4(0.0, 0.0, 0.0, -1.0); }
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
// - The sky: its colour as much as the face sees of it, by the rays fanned toward it (z of `v`);
//   where none got there, the sky light level squared from a dome (all of it on tops, 60 % on
//   sides, a quarter beneath); diffuse, and reflected as much as Fresnel says.
// - Light sources (torches, lava, glowstone): traced, each its colour falling off as the inverse
//   square of the distance with a shadow ray toward it (`traced`, shadow_rays.wgsl `block_light`);
//   past the grid or on a frame without this frame's rays, warm light from the block light level
//   (one level a block).
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
// The material `m` (on a face of normal `geo`, camera-relative `pos`) lit, before the fog; `light`
// its sky and block light levels (0 to 15), `v` the rays' visibility, `traced` the light sources'.
const FIREFLY_LUMINANCE: f32 = 5.0;
fn rt_shade(m: RtMaterial, geo: vec3<f32>, pos: vec3<f32>, light: vec2<f32>, v: vec3<f32>,
    traced: vec4<f32>, tint: vec3<f32>) -> vec3<f32> {
    let n = m.normal;
    let view = -normalize(pos);
    let nv = max(dot(n, view), 1e-3);
    let l = globals.rt_light.xyz;
    // The real sun's light through the atmosphere, or the moon's (gfx/atmosphere.rs).
    let sun = globals.rt_sun_light.rgb * globals.rt_light.w * v.x * tint * rt_relief_sun;
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
    let by_level = (rt_sky(normalize(n + vec3(0.0, 1.0, 0.0))) * sky_level * dome + level) * open;
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
    // The rain's impacts (`rt_spray`): their broken crests scatter the sky's light.
    let spray = rt_sky(vec3(0.0, 1.0, 0.0)) * seen * (RAIN_SPRAY * rt_spray);
    return sun * (direct + through) + highlight + diffuse * (1.0 - ambient_f) * around + reflected
        + m.albedo * m.emission * MATERIAL_EMISSION + spray;
}
// RT lighting: material `m`'s linear colour, lit by the model and fogged. The fog scatters the
// light it gets: the sky's, as much as the surface sees of it (a sealed room's air carries none).
fn rt_material_lit(m: RtMaterial, geo: vec3<f32>, pos: vec3<f32>, pixel: vec2<f32>,
    light: vec2<f32>, spherical: f32, cylindrical: f32) -> vec3<f32> {
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
    let fade = 1.0 - smoothstep(1.5, 4.0, d.z * footprint);
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
fn water_sea(q: vec2<f32>, t: f32) -> vec3<f32> {
    let swell = water_noise(q / 233.0 + vec2(0.003, 0.001) * t);
    let wind = water_noise(q / 71.0 + vec2(17.3, 5.1) + vec2(0.006, -0.004) * t);
    let gusts = water_noise(q / 23.0 + vec2(-3.7, 11.9) + vec2(0.045, 0.02) * t);
    // The wind's gains contrast more than the swell's: calm slicks beside rippled patches.
    let patches = smoothstep(vec2(0.15), vec2(0.85), vec2(wind, gusts));
    return vec3(0.4 + 1.2 * swell, 0.15 + 1.7 * patches);
}
fn water_surface(q: vec2<f32>, t: f32, footprint: f32) -> WaterSurface {
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
    let g = water_sea(q, t);
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
fn water_waves(p: vec2<f32>, t: f32, footprint: f32) -> vec3<f32> {
    let w = water_surface(p, t, footprint);
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
        let fade = 1.0 - smoothstep(1.5, 4.0, k * footprint);
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
fn rt_lava(pos: vec3<f32>, n: vec3<f32>, pixel: vec2<f32>, light: vec2<f32>, spherical: f32,
    cylindrical: f32) -> vec3<f32> {
    let world = pos + globals.rt_camera.xyz;
    let t = globals.rt_water.z;
    // Triplanar: on the top it drifts on the world's plane, on a side (or a fall) it runs down,
    // stretched as falling lava streams; a slope blends them by its normal, so no face sees the
    // top's pattern smeared along it.
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
    let rock = rt_material_lit(rt_plain(vec3(0.07, 0.06, 0.055), n), n, pos, pixel, light,
        spherical, cylindrical);
    return glow + rock * crust;
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
    return rt_water_at(in.pos, in.clip.xy, in.light, in.spherical, in.cylindrical, face, in.tint);
}
// The same from a fragment's camera-relative `pos`, its `pixel`, light levels, fog distances, face
// and biome tint (gamma), for the parallax terrain's water too.
fn rt_water_at(pos: vec3<f32>, pixel: vec2<f32>, levels: vec2<f32>, spherical: f32,
    cylindrical: f32, face: vec3<f32>, tint: vec3<f32>) -> WaterLight {
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
        let w = water_waves(world.xz, globals.rt_water.z, footprint);
        rough = water_lost_slope(footprint, water_sea(world.xz, globals.rt_water.z));
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
    let sun = globals.rt_sun_light.rgb * globals.rt_light.w * v.x;
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
        let size = vec2<f32>(textureDimensions(rt_refract));
        let at = clamp(vec2<i32>(in.clip / globals.rt_motion.xy * size), vec2(0),
            vec2<i32>(size) - 1);
        refracted = textureLoad(rt_refract, at, 0);
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
    let glow = lit * colour * vec3(0.206, 0.999, 0.903) * 0.15 * (1.0 - exp(-absorption * thickness * both)) / both;
    var added = fresnel * reflected + (1.0 - fresnel) * glow
        + rt_sky(vec3(0.0, 1.0, 0.0)) * sky_seen * (RAIN_SPRAY * spray);
    var shows = (1.0 - fresnel) * through;
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
// rains there, how many of its neighbours stand higher, the block light above it.
fn rain_height(w: u32) -> f32 { return f32(i32(w & 4095u) - 2048); }
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
// moderate rain. `spread` is how far a ring runs (blocks): wide on standing water, a small circle
// on the thin film of a wet surface. `wet` (0 to 1) sharpens them: on a surface that is drying only the
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
            let width = spread * (0.08 + 0.12 * age);
            let envelope = exp(-x * x / (width * width)) * (1.0 - age) * (1.0 - age);
            slope += 0.5 * cos(x * 3.14159 / width) * envelope * d / max(r, 1e-4);
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
    let size = vec2<f32>(textureDimensions(rt_reflect));
    let at = clamp(vec2<i32>(pixel / globals.rt_motion.xy * size), vec2(0), vec2<i32>(size) - 1);
    return textureLoad(rt_reflect, at, 0);
}
// RT lighting's wet world (after Lagarde 2013, "Water drop 2b: dynamic rain and its effects"):
// material `m` on a face of normal `geo` at camera-relative `pos` (its pixel, its sky light level
// `sky` 0..1), as the rain has left it (`Globals::rain`): where the rain reaches it
// (`rain_exposure`, the sky light past the map), darker the more porous it is (`porosity`, the
// pack's LabPBR value, below 0 its surface's own), glossier as water fills it, except where it
// drinks the water (sand, wool) or runs off (metal, glass: a film). Sunlit surfaces dry first
// (`rain_sun`). On the tops of the columns that hold them, puddles (`relief`, the pack's depth
// there, 0.5 unknown): flat water, the rain's ripples on it while it falls.
fn rt_wet(material: RtMaterial, geo: vec3<f32>, pos: vec3<f32>, pixel: vec2<f32>, sky: f32,
    porosity: f32, relief: f32) -> RtMaterial {
    var m = material;
    rt_puddle = 0.0;
    rt_spray = 0.0;
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
    if globals.rt_mode.y > 3.5 {
        let known = select(0.0, 1.0, rain_cell(vec2<i32>(floor(world.xz))) != 0u);
        m.albedo = select(vec3(exposure, rt_puddle, known), vec3(wet, p, 1.0 - m.roughness),
            globals.rt_mode.y > 4.5);
        m.emission = 0.25;
    }
    return m;
}
// RT lighting: a fragment without maps, its albedo (`albedo`, gamma) lit and fogged, wet as the
// rain left it.
fn rt_lit(albedo: vec3<f32>, in: TerrainOut, n: vec3<f32>) -> vec3<f32> {
    let m = rt_wet(rt_plain(albedo, n), n, in.pos, in.clip.xy, in.light.x, -1.0, 0.5);
    return rt_material_lit(m, n, in.pos, in.clip.xy, in.light, in.spherical, in.cylindrical);
}
// The face's normal toward the camera, from the fragment's position (faces are flat: exact).
fn face_normal(pos: vec3<f32>) -> vec3<f32> {
    let n = normalize(cross(dpdx(pos), dpdy(pos)));
    return select(n, -n, dot(n, pos) > 0.0);
}
@fragment fn fs_solid(in: TerrainOut) -> @location(0) vec4<f32> {
    let n = face_normal(in.pos);
    if rt_lava_on(in.layer) && !faded_out(in.fade, in.clip.xy) {
        return vec4(rt_lava(in.pos, n, in.clip.xy, in.light, in.spherical, in.cylindrical), 1.0);
    }
    // Bit 15 of the layer marks opaque leaves (forceOpaque): no alpha cutout.
    let texel = sprite(in.uv, in.layer & 0x7fffu);
    let color = texel * in.color;
    if (color.a < 0.5 && (in.layer & 0x8000u) == 0u) || faded_out(in.fade, in.clip.xy) { discard; }
    // RT lighting: the albedo (the texel times the tint) under the model's light, fogged in light.
    // Light-emitting blocks shine by themselves.
    if globals.rt_mode.x > 0.5 && linear_on() && in.emission == 0u {
        // HORIZON_RT_DEBUG=7: the rays' visibility (sun red, light around green).
        if globals.rt_mode.y > 1.5 && globals.rt_mode.y < 2.5 {
            let t = rt_traced(in.clip.xy);
            return vec4(select(t.rgb, vec3(1.0, 0.0, 1.0), t.a < 0.0), 1.0);
        }
        if globals.rt_mode.y > 0.5 && globals.rt_mode.y < 1.5 {
            return vec4(max(rt_visibility(in.pos, in.clip.xy), vec3(0.0)), 1.0);
        }
        return vec4(rt_lit(texel.rgb * in.tint, in, n), 1.0);
    }
    // RT lighting: a light-emitting block is lit as any other, and only its burning texels
    // shine (a torch's flame and embers, not its stick: `rt_burning`).
    if globals.rt_mode.x > 0.5 && linear_on() {
        let lit = rt_lit(texel.rgb * in.tint, in, n);
        // Without the per-layer flags (parallax.rs `thin_layers`) a block's emission is its own
        // colour as bright as its level, as before the black bodies.
        let glow = scene_color(vec4(color.rgb, 0.0), emission_gain(in.emission), in.spherical,
            in.cylindrical).rgb;
        return vec4(mix(lit, glow, rt_burning(texel.rgb)), 1.0);
    }
    // Alpha 0 tells HDR output (hdr.wgsl) a light-emitting block's bright pixels
    // may shine. Nothing else reads the frame's alpha.
    let emissive = select(1.0, 0.0, in.emission > 0u);
    var out = scene_color(vec4(color.rgb, emissive), emission_gain(in.emission), in.spherical,
        in.cylindrical);
    if linear_on() { out.a = 0.5 * sun_share(in.light, in.emission); }
    return out;
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
    if (color.a < 0.5 && (in.layer & 0x8000u) == 0u) || faded_out(in.fade, in.clip.xy) { discard; }
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
    let packed = globals.fog_ends.w;
    if packed < 0.0 { return false; }
    let l = f32(layer & 0x7fffu);
    let pane = floor(packed / 4096.0);
    return l == packed - pane * 4096.0 || l == pane;
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
    let size = vec2<f32>(textureDimensions(rt_refract));
    let at = clamp(vec2<i32>(pixel / globals.rt_motion.xy * size), vec2(0), vec2<i32>(size) - 1);
    return textureLoad(rt_refract, at, 0);
}
fn rt_glass_at(g: GlassIn) -> WaterLight {
    let distance = length(g.pos);
    let ray = g.pos / max(distance, 1e-4);
    let view = -ray;
    let plain = rt_glass_plain(g.layer);
    // A map's facet turned from the eye is seen edge on: kept just inside the side it can see.
    var n = g.normal;
    let facing = dot(n, view);
    if facing < 0.02 { n = normalize(n + view * (0.02 - facing)); }
    let nv = max(dot(n, view), 1e-3);
    let fresnel = rt_glass_fresnel(nv);
    let v = rt_visibility(g.pos, g.clip);
    let sky_seen = select(rt_sky_level(g.light.x), max(v.z, 0.0),
        v.z >= 0.0);
    let sun = globals.rt_sun_light.rgb * globals.rt_light.w * v.x;
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
    let alpha = max(g.roughness * g.roughness, 0.02);
    let a2 = alpha * alpha;
    let nh = max(dot(n, h), 0.0);
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let glint = a2 / (RT_PI * dd * dd) * 0.25 / max(nl * nv, 1e-3) * nl * RT_PI;
    reflected += sun * min(glint * smoothstep(0.0, 0.15, dot(g.face, l)), 200.0);
    // Through it: in bent by the facet, out through a flat face parallel to this one.
    let inward = refract(ray, n, 1.0 / GLASS_INDEX);
    var outward = refract(inward, g.face, GLASS_INDEX);
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
            let taps = select(1, 4, g.roughness > 0.1 && length(u) + length(w) > 1.0);
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
    if refracted.a <= 0.0 {
        // Untraced: the glass's colour over its measurement distance along the bent ray.
        let colour = select(clamp(g.albedo, vec3(0.0), vec3(1.0)), vec3(1.0), plain)
            * GLASS_TRANSMITTANCE;
        let slant = max(dot(inward, -g.face), 0.2);
        shows *= pow(max(colour, vec3(1e-3)), vec3(1.0 / slant));
    }
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
            let walls = pow(max(colour, vec3(1e-3)), vec3(1.0 / slant)) * (1.0 - rt_glass_fresnel(nv));
            let spread = max(alpha * 2.0 * (1.0 - 1.0 / GLASS_INDEX), 0.01);
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
    return rt_glass_at(GlassIn(in.pos, in.clip.xy, in.light, in.spherical, in.cylindrical, face,
        rt_glass_wave(in.pos, face), decode_gamma(texel.rgb * in.tint), texel.a, 0.0, in.layer));
}
@fragment fn fs_translucent(in: TerrainOut) -> @location(0) vec4<f32> {
    let n = face_normal(in.pos);
    let texel = sprite(in.uv, in.layer & 0x7fffu);
    let color = texel * in.color;
    if color.a < 0.01 || faded_out(in.fade, in.clip.xy) { discard; }
    // RT lighting: water its own way (`rt_water`), glass its own (`rt_glass`).
    if globals.rt_mode.x > 0.5 && linear_on() && in.emission == 0u {
        if rt_is_water(in.layer) {
            let w = rt_water(in, n);
            return vec4(w.added / w.alpha, w.alpha);
        }
        let g = rt_glass(in, n, texel);
        return vec4(g.added / g.alpha, g.alpha);
    }
    return scene_color(color, emission_gain(in.emission), in.spherical, in.cylindrical);
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
fn rotate_x90(v: vec3<f32>) -> vec3<f32> { return vec3(v.x, -v.z, v.y); }
// Weight of the fan centre where the ray crosses the sunrise fan (0 when missed).
fn sunrise_weight(d: vec3<f32>) -> f32 {
    let alpha = globals.sunrise.a;
    if alpha <= 0.001 { return 0.0; }
    // sin(sunAngle) < 0 flips the fan to the other horizon; sun.z = -sin(sunAngle).
    let flip = select(0.0, 180.0, -globals.sun.z < 0.0);
    let theta = radians(flip + 90.0);
    let c = cos(theta);
    let s = sin(theta);
    var best = 0.0;
    let apex = rotate_x90(vec3(-100.0 * s, 100.0 * c, 0.0));
    for (var i = 0; i < 16; i++) {
        let a0 = f32(i) * 2.0 * PI / 16.0;
        let a1 = f32(i + 1) * 2.0 * PI / 16.0;
        let l0 = vec3(sin(a0) * 120.0, cos(a0) * 120.0, -cos(a0) * 40.0 * alpha);
        let l1 = vec3(sin(a1) * 120.0, cos(a1) * 120.0, -cos(a1) * 40.0 * alpha);
        let r0 = rotate_x90(vec3(l0.x * c - l0.y * s, l0.x * s + l0.y * c, l0.z));
        let r1 = rotate_x90(vec3(l1.x * c - l1.y * s, l1.x * s + l1.y * c, l1.z));
        // Moller-Trumbore from the origin along d.
        let e1 = r0 - apex;
        let e2 = r1 - apex;
        let pv = cross(d, e2);
        let det = dot(e1, pv);
        if abs(det) < 1e-6 { continue; }
        let tv = -apex;
        let u = dot(tv, pv) / det;
        let qv = cross(tv, e1);
        let v = dot(d, qv) / det;
        let t = dot(e2, qv) / det;
        if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0 {
            best = max(best, 1.0 - u - v);
        }
    }
    return best;
}
// The sky's colour along the view ray `d` (a shader pack's sky stage gets it as its input:
// gfx/native_sky.rs builds that stage from this section).
fn sky_along_ray(d: vec3<f32>) -> vec3<f32> {
    var color = globals.fog_color.rgb;
    let dist = disc_distance(d, 16.0);
    if dist.x >= 0.0 {
        color = sky_fog(globals.sky_color.rgb, dist);
    }
    let w = sunrise_weight(d);
    if w > 0.0 {
        color = mix(color, globals.sunrise.rgb, w * globals.sunrise.a);
    }
    return color;
}
@fragment fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    return vec4(scene_rgb(sky_along_ray(view_ray(in.ndc))), 1.0);
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
struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) spherical: f32,
    @location(1) cylindrical: f32,
    @location(2) color: vec4<f32>,
}
@vertex fn vs_line(@builtin(vertex_index) vertex: u32,
    @location(0) start: vec3<f32>, @location(1) end: vec3<f32>,
    @location(2) argb: vec4<f32>, @location(3) width: f32) -> LineOut {
    let position = select(start, end, vertex >= 2u);
    var out: LineOut;
    out.clip = outline_clip(vertex, start, end, width);
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
@vertex fn vs_rain(@builtin(vertex_index) vertex: u32, @builtin(instance_index) id: u32)
    -> RainOut {
    var out: RainOut;
    out.clip = vec4(2.0, 2.0, 2.0, 1.0);
    out.uv = vec2(0.0);
    out.colour = vec3(0.0);
    out.alpha = 0.0;
    out.splash = 0.0;
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
    let column = rain_cell(vec2<i32>(floor(xz)));
    if column == 0u || !rain_falls(column) { return out; }
    // It stops on the first block that stops motion (vanilla's rain heightmap), a water's top a
    // little under it, and leaves a circle there for a moment.
    let ground = rain_top(column) - select(0.0, 0.11, rain_surface(column) == 10u);
    var head = vec3(xz.x, y, xz.y);
    var splash = -1.0;
    if y < ground {
        let since = (ground - y) / speed;
        if !near || since > RAIN_SPLASH { return out; }
        splash = since / RAIN_SPLASH;
        let at = start - lean * (ground - camera.y);
        head = vec3(at.x, ground, at.y);
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
    if splash >= 0.0 {
        // The circle it leaves where it lands, flat on the ground or the water and spreading:
        // a quad lying there, its ring drawn by fs_rain; larger drops leave wider ones.
        let extent = RAIN_CIRCLE * (0.6 + 0.2 * diameter);
        let at = rel + vec3(corner.x * extent, 0.006, (corner.y * 2.0 - 1.0) * extent);
        out.clip = globals.view_proj * vec4(at, 1.0);
        out.uv = vec2(corner.x, corner.y * 2.0 - 1.0);
        out.splash = 1.0 + splash;
        // As faint as the ring is thin against the pixels it covers.
        out.alpha = 0.45 * fade * min(1.0, extent * 0.08 / pixel);
        return out;
    }
    // A streak: its path over the exposure, a thin quad turned to the eye about it, a pixel wide
    // at least. A drop covers a streak's pixel for its diameter over the streak's length of the
    // exposure (Garg and Nayar 2006), less when it is thinner than the pixel.
    let streak = speed * RAIN_EXPOSURE + 0.01;
    let down = normalize(vec3(wind.x, -speed, wind.y));
    let tail = rel - down * streak;
    let across = normalize(cross(down, (rel + tail) * 0.5) + vec3(0.0, 1e-5, 0.0));
    let width = max(diameter * 0.001, pixel);
    let opacity = clamp(60.0 * diameter * 0.001 / streak, 0.0, 0.6)
        * sqrt(diameter * 0.001 / width);
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
