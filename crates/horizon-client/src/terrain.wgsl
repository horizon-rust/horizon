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
    // x: sky fog end, y: cloud fog end, z: dark disc flag
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
@vertex fn vs_terrain(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>,
    @location(5) origin: vec4<f32>) -> TerrainOut {
    var out = terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin.xyz);
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
    var out = terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin.xyz);
    out.fade = terrain_fade(origin.w);
    return out;
}
fn terrain_vertex(position: vec3<f32>, uv: vec2<f32>, layer: u32, color: vec4<f32>,
    light: vec2<u32>, origin: vec3<f32>) -> TerrainOut {
    let pos = position + origin;
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
// RT lighting's sky toward `d` (unit): the atmosphere's light (gfx/atmosphere.rs `sky_view`).
fn rt_sky(d: vec3<f32>) -> vec3<f32> {
    let flat = d.xz / max(length(d.xz), 1e-5);
    let sun = globals.rt_sun.xz / max(length(globals.rt_sun.xz), 1e-5);
    let azimuth = acos(clamp(dot(flat, sun), -1.0, 1.0)) / 3.14159265;
    let e = asin(clamp(d.y, -1.0, 1.0)) / 1.5707963;
    let v = 0.5 + 0.5 * sign(e) * sqrt(abs(e));
    return textureSampleLevel(rt_sky_view, lightmap_sampler, vec2(azimuth, v), 0.0).rgb;
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
    let tint = vec3(1.0, 0.93, 0.85);
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
// The material `m` (on a face of normal `geo`, camera-relative `pos`) lit, before the fog; `light`
// its sky and block light levels (0..1), `v` the rays' visibility, `traced` the light sources'.
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
    let sky_level = pow(clamp(light.x * 16.0 / 15.0, 0.0, 1.0), 2.0);
    let dome = select(mix(0.6, 1.0, n.y), mix(0.6, 0.25, -n.y), n.y < 0.0);
    let seen = select(sky_level * dome, v.z, v.z >= 0.0);
    let open = v.y * m.ao;
    let steps = max(15.0 - light.y * 16.0, 0.0);
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
    return sun * (direct + through) + highlight + diffuse * (1.0 - ambient_f) * around + reflected
        + m.albedo * m.emission * MATERIAL_EMISSION;
}
// RT lighting: material `m`'s linear colour, lit by the model and fogged. The fog scatters the
// light it gets: the sky's, as much as the surface sees of it (a sealed room's air carries none).
fn rt_material_lit(m: RtMaterial, geo: vec3<f32>, pos: vec3<f32>, pixel: vec2<f32>,
    light: vec2<f32>, spherical: f32, cylindrical: f32) -> vec3<f32> {
    let v = rt_visibility(pos, pixel);
    rt_mirror = rt_reflection(pixel, pos);
    let lit = rt_shade(m, geo, pos, light, v, rt_traced(pixel), rt_sun_colour(pixel, v.x));
    let f = rt_fade(spherical, cylindrical, pos) * globals.fog_color.a;
    let sky = select(pow(clamp(light.x * 16.0 / 15.0, 0.0, 1.0), 2.0), v.z, v.z >= 0.0);
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
fn water_surface(q: vec2<f32>, t: f32, footprint: f32) -> WaterSurface {
    var s = WaterSurface(0.0, vec2(0.0), vec3(0.0));
    // The crests are not straight lines: a slow swirl bends the coordinates every wave is read at
    // (its own slopes are neglected: the bend is gentle, 20 % at most).
    let p = q + 0.6 * vec2(sin(0.31 * q.y + 0.23 * t + 1.3), sin(0.27 * q.x - 0.19 * t));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.92106, 0.38942, 1.0400, 0.02600), vec2(1.9165, 0.0)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.94221, 0.33503, 1.3000, 0.03250), vec2(2.1427, 2.1)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.46845, -0.88349, 1.6250, 0.04063), vec2(2.3956, 4.2)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.25138, 0.96789, 2.0312, 0.05078), vec2(2.6784, 6.3)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.83916, -0.54389, 2.5391, 0.06000), vec2(2.9946, 8.4)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.98616, -0.16580, 3.1738, 0.06000), vec2(3.3481, 10.5)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.61516, 0.78840, 3.9673, 0.06000), vec2(3.7433, 12.6)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.07896, -0.99688, 4.9591, 0.06000), vec2(4.1853, 14.7)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.73160, 0.68173, 6.1989, 0.06000), vec2(4.6796, 16.8)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.99996, -0.00849, 7.7486, 0.06000), vec2(5.2323, 18.9)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.74308, -0.66921, 9.6858, 0.06000), vec2(5.8506, 21.0)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.09587, 0.99539, 12.1072, 0.06000), vec2(6.5425, 23.1)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.60169, -0.79873, 15.1340, 0.06000), vec2(7.3170, 25.2)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.98320, 0.18252, 18.9175, 0.05286), vec2(8.1846, 27.3)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.84827, 0.52956, 23.6469, 0.04229), vec2(9.1574, 29.4)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.26778, -0.96348, 29.5586, 0.03383), vec2(10.2503, 31.5)));
    return s;
}
// The height and the slopes along x and z, for the shaders that only look at the surface.
fn water_waves(p: vec2<f32>, t: f32, footprint: f32) -> vec3<f32> {
    let w = water_surface(p, t, footprint);
    return vec3(w.height, w.slope);
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
    if face.y > 0.5 {
        let w = water_waves(world.xz, globals.rt_water.z, length(in.pos) * 1.5 / globals.rt_motion.y);
        n = normalize(vec3(-w.y, 1.0, -w.z));
    }
    let distance = length(in.pos);
    let ray = in.pos / max(distance, 1e-4);
    let view = -ray;
    let nv = max(dot(n, view), 1e-3);
    let fresnel = 0.02 + 0.98 * pow(1.0 - nv, 5.0);
    let v = rt_visibility(in.pos, in.clip);
    let sky_seen = select(pow(clamp(in.light.x * 16.0 / 15.0, 0.0, 1.0), 2.0), max(v.z, 0.0),
        v.z >= 0.0);
    let sun = globals.rt_sun_light.rgb * globals.rt_light.w * v.x;
    // Reflected: the sky that way, and the sun's glint (GGX, a calm surface).
    var reflected = rt_sky(reflect(ray, n)) * max(sky_seen, 0.15 * v.x);
    if globals.rt_motion.w > 1.5 {
        let traced = rt_reflection(in.clip, in.pos);
        if traced.a > 0.5 { reflected = traced.rgb; }
        // HORIZON_RT_DEBUG=9: the traced reflection alone (magenta: none, the sky's).
        if globals.rt_mode.y > 2.5 {
            return WaterLight(select(vec3(1.0, 0.0, 1.0), traced.rgb, traced.a > 0.5), vec3(0.0),
                1.0);
        }
    }
    let l = globals.rt_light.xyz;
    let h = normalize(l + view);
    let nl = max(dot(n, l), 0.0);
    let a2 = 0.0004;
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
    var added = fresnel * reflected + (1.0 - fresnel) * glow;
    var shows = (1.0 - fresnel) * through;
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
// RT lighting: a fragment without maps, its albedo (`albedo`, gamma) lit and fogged.
fn rt_lit(albedo: vec3<f32>, in: TerrainOut, n: vec3<f32>) -> vec3<f32> {
    return rt_material_lit(rt_plain(albedo, n), n, in.pos, in.clip.xy, in.light, in.spherical,
        in.cylindrical);
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
@fragment fn fs_translucent(in: TerrainOut) -> @location(0) vec4<f32> {
    let n = face_normal(in.pos);
    let texel = sprite(in.uv, in.layer & 0x7fffu);
    let color = texel * in.color;
    if color.a < 0.01 || faded_out(in.fade, in.clip.xy) { discard; }
    // RT lighting: water its own way (`rt_water`), glass by the same model as the terrain (the rays
    // saw what is behind it or the surface).
    if globals.rt_mode.x > 0.5 && linear_on() && in.emission == 0u {
        if rt_is_water(in.layer) {
            let w = rt_water(in, n);
            return vec4(w.added / w.alpha, w.alpha);
        }
        return vec4(rt_lit(texel.rgb * in.tint, in, n), color.a);
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
// (gfx/atmosphere.rs) and the real sun's disc, in light.
@fragment fn fs_rt_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let d = view_ray(in.ndc);
    let sky = rt_sky(d);
    let pixel = length(fwidth(d));
    return vec4(sky + rt_sun_disc(d) + rt_moon_disc(d) + rt_glare(d) + rt_milky_way(d, sky)
        + rt_meteor(d, sky, pixel), 1.0);
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
    let brightness = globals.rt_stars.w;
    if brightness <= 0.0 || d.y <= 0.0 { return out; }
    let sky = rt_sky(d);
    let glare = exp(-dot(sky, vec3(0.2126, 0.7152, 0.0722)) * 60.0);
    // Through the air: airmass over the zenith's, more of the blue lost (the stars redden low).
    let airmass = 1.0 / max(d.y + 0.025, 0.03);
    let air = exp(-vec3(0.12, 0.18, 0.3) * airmass);
    let twinkle = 1.0 + 0.3 * sin(globals.time.x * 24000.0 * 1.7 + f32(id) * 1.618)
        * clamp(airmass * 0.2, 0.1, 1.0);
    // Paper whites at the quad's middle: a magnitude 0 star 20 (the eye adapted to the night),
    // each magnitude 2.5 times dimmer.
    let flux = 20.0 * pow(10.0, -0.4 * magnitude);
    out.light = star_colour(kelvin) * (flux * brightness * glare * twinkle) * air;
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
// The Milky Way, where it is: a faint, uneven band along the galactic plane (its north pole at
// right ascension 192.86 degrees, declination 27.13 degrees), brighter toward its centre in
// Sagittarius; the sky's light outshines it like the stars.
fn rt_milky_way(d: vec3<f32>, sky: vec3<f32>) -> vec3<f32> {
    let brightness = globals.rt_stars.w;
    if brightness <= 0.0 || d.y <= 0.0 { return vec3(0.0); }
    let m = globals.rt_star_frame;
    let e = vec3(dot(d, m[0].xyz), dot(d, m[1].xyz), dot(d, m[2].xyz));
    let pole = vec3(-0.8676, -0.1981, 0.4560);
    let centre = vec3(-0.0548, -0.8734, -0.4838);
    let b = dot(e, pole);
    let toward = max(dot(e, centre), 0.0);
    let grain = rt_milky_noise(e * 9.0) * 0.6 + rt_milky_noise(e * 27.0) * 0.4;
    let band = exp(-b * b / 0.012) * (0.35 + grain) * (1.0 + 2.5 * toward * toward * toward);
    let glare = exp(-dot(sky, vec3(0.2126, 0.7152, 0.0722)) * 60.0);
    let air = exp(-0.2 / max(d.y + 0.025, 0.03));
    return vec3(0.8, 0.85, 1.0) * (0.006 * band * brightness * glare * air);
}
fn rt_milky_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    var v = 0.0;
    for (var k = 0u; k < 8u; k++) {
        let o = vec3(f32(k & 1u), f32((k >> 1u) & 1u), f32(k >> 2u));
        let w = select(1.0 - u, u, o > vec3(0.5));
        let c = vec3<i32>(i + o);
        var h = u32(c.x) * 73856093u ^ u32(c.y) * 19349663u ^ u32(c.z) * 83492791u;
        h = (h ^ (h >> 13u)) * 1274126177u;
        v += w.x * w.y * w.z * f32(h >> 8u) / 16777216.0;
    }
    return v;
}
// A hash of `n` to 32 random bits (a PCG round), for the meteors' draws.
fn meteor_hash(n: u32) -> u32 {
    let state = n * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}
fn meteor_rand(n: u32) -> f32 {
    return f32(meteor_hash(n) >> 8u) / 16777216.0;
}
// A shooting star: the time is cut into slots of `rt_stars.y` seconds, each holding a meteor with
// the chance `rt_stars.z`, its place in the slot, its sky, its heading, its length and its
// magnitude all drawn from the slot's number, so no frame carries any state and a quiet sky costs
// one branch. It is a streak on a great circle of the sky, a head that runs ahead and a tail that
// fades behind it, the whole fading out over a few tenths of a second; mostly heading down (they
// fall), reddened and dimmed by the air toward the horizon, outshone by the sky's own light like
// the stars (`sky`), and none by day.
fn rt_meteor(d: vec3<f32>, sky: vec3<f32>, pixel: f32) -> vec3<f32> {
    let period = globals.rt_stars.y;
    if period <= 0.0 || globals.rt_stars.w <= 0.0 || d.y <= 0.0 { return vec3(0.0); }
    let slot = u32(globals.rt_water.z / period);
    let seed = slot * 16u + 1u;
    if meteor_rand(seed) > globals.rt_stars.z { return vec3(0.0); }
    // Its life: 0.3 to 0.9 s, started somewhere in the slot where it still ends inside it.
    let life = 0.3 + 0.6 * meteor_rand(seed + 1u);
    let start = max(period - life, 0.0) * meteor_rand(seed + 2u);
    let age = (globals.rt_water.z - f32(slot) * period - start) / life;
    if age < 0.0 || age > 1.0 { return vec3(0.0); }
    // Where it starts (25 to 70 degrees up) and which way it goes along its great circle: toward
    // `u`, in the tangent plane, from straight down to nearly sideways, never up.
    let azimuth = 6.2831853 * meteor_rand(seed + 3u);
    let elevation = radians(25.0 + 45.0 * meteor_rand(seed + 4u));
    let a = vec3(cos(elevation) * cos(azimuth), sin(elevation), cos(elevation) * sin(azimuth));
    let east = normalize(cross(vec3(0.0, 1.0, 0.0), a));
    let up = cross(a, east);
    let heading = -(0.1 + 0.8 * meteor_rand(seed + 5u)) * PI;
    let u = east * cos(heading) + up * sin(heading);
    let span = radians(8.0 + 14.0 * meteor_rand(seed + 6u));
    let trail = span * (0.3 + 0.3 * meteor_rand(seed + 7u));
    // Its place on its circle: across it by `dot(d, n)`, along it by the angle `phi` from the start.
    let n = cross(a, u);
    let x = dot(d, a);
    if x <= 0.0 { return vec3(0.0); }
    let phi = atan2(dot(d, u), x);
    let behind = (span * age - phi) / trail;
    if behind < -0.02 || behind > 1.0 { return vec3(0.0); }
    // A hair across (a pixel or two), a tail that thins to nothing, a head that ends sharply.
    let width = max(0.0012, pixel * 1.2);
    let across = dot(d, n) / width;
    let along = pow(1.0 - clamp(behind, 0.0, 1.0), 1.7) * smoothstep(-0.02, 0.0, behind);
    let fade = (1.0 - age) * smoothstep(0.0, 0.08, age);
    // Magnitude -3 to 1 at its brightest: paper whites like the stars' (20 at magnitude 0), the
    // streak being spread over many pixels, its head is worth more.
    let magnitude = -3.0 + 4.0 * meteor_rand(seed + 8u);
    let flux = 60.0 * pow(10.0, -0.4 * magnitude);
    let airmass = 1.0 / max(d.y + 0.025, 0.03);
    let air = exp(-vec3(0.12, 0.18, 0.3) * airmass);
    let glare = exp(-dot(sky, vec3(0.2126, 0.7152, 0.0722)) * 60.0);
    // Mostly white, some green (magnesium) or orange (sodium).
    let tints = array<vec3<f32>, 4>(vec3(1.0, 1.0, 1.0), vec3(0.85, 1.0, 0.9), vec3(1.0, 0.8, 0.55),
        vec3(0.9, 0.95, 1.0));
    let tint = tints[meteor_hash(seed + 9u) & 3u];
    return tint * (flux * along * fade * exp(-across * across) * glare * globals.rt_stars.w) * air;
}
