// Froxel volumetrics (gfx/froxels.rs, docs/raytracing.md "Next: ... RTX Remix"): a homogeneous
// medium around the camera in a grid of frustum voxels (x, y: the view's screen; z: the distance
// along the ray, exponential slices to `far`). Three passes:
//   cs_inject    per froxel, the light scattered toward the camera at a point of the froxel drawn
//                this frame (a low-discrepancy sequence, so the history supersamples the froxel): the
//                sun (a shadow ray, tinted by stained glass and water), the sky's ambient (a ray toward
//                the sky), and one light source drawn by importance, each through a shadow ray.
//   cs_blend     per froxel, that sample blended with the last frame's froxels reprojected (a long
//                history, 8 frames while settling), the history shortened where the froxel's content
//                moved a lot and held within the range the new samples' neighbourhood shows.
//   cs_integrate per screen column, front to back: the in-scatter and (in alpha) the transmittance
//                up to each slice.
//   fs_scatter / fs_transmit  the composite, `scene * T + in-scatter` at each pixel's depth, the
//                in-scatter read through a cubic B-spline across the screen (a trilinear read shows
//                the 16 pixel grid as kinked, blocky stripes in the halo).
// Units are those of terrain.wgsl's diffuse light: radiance times pi (Lambert's 1/pi left out), so
// the sun's light in `sun_light` scatters as `pi * HG * E` and an isotropic sky as its mean.
struct Fx {
    // Clip space to camera-relative world space; the last frame's view's world to clip space.
    inverse_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    // xyz toward the light (the sun's or the moon's), w its strength (0: none).
    light: vec4<f32>,
    // xyz: the camera's block, w: the Y above the grid's highest block.
    camera: vec4<f32>,
    // xyz: the camera within its block.
    fraction: vec4<f32>,
    // x, y: the grid's first block's x and z, z: its width, w: the DDA's budget.
    grid: vec4<f32>,
    // xyz toward the real sun (the sky table's frame).
    sun: vec4<f32>,
    // rgb: the light traced toward, in paper whites; w its disc's angular radius.
    sun_light: vec4<f32>,
    // The camera's offset from the hardware rays' instances' anchor (gfx/hw_rays.rs).
    hw_offset: vec4<f32>,
    // The water's still and flowing sprite layers (-1 none) and the time.
    water: vec4<f32>,
    // xyz: the camera's move since the last frame of this view, w 1 when that frame's froxels can
    // be reprojected.
    motion: vec4<f32>,
    // The grid's size (x, y, z), w this frame's number.
    dims: vec4<f32>,
    // x: the first slice's far end in metres, y: the last slice's, z: log(y / x), w: unused.
    depth: vec4<f32>,
    // rgb: the medium's extinction a metre (-ln(transmittance) / distance), w: the phase's
    // anisotropy (Henyey-Greenstein g).
    sigma_t: vec4<f32>,
    // rgb: the medium's single scattering albedo, w: the sky's ambient light in the medium.
    albedo: vec4<f32>,
    // x: the history's longest length in frames, y: the light sources' scale.
    history: vec4<f32>,
    // The depth's size in pixels.
    size: vec4<f32>,
    // xyz: where the view's rays start (camera-relative; the view bobbing moves it off the camera),
    // this frame's and the last frame's.
    eye: vec4<f32>,
    prev_eye: vec4<f32>,
    // xyz: this frame's offset within a froxel before the froxel's own shift (a 3D R-sequence).
    jitter: vec4<f32>,
    // The natural fogs (gfx/natural_fog.rs). The relief map's layers: xy the camera's block less
    // the near layer's origin (x, z), zw the far layer's; and x, y their texels' sizes' inverse,
    // z, w whether each is made.
    map: vec4<f32>,
    map_scale: vec4<f32>,
    // x: the valley fog's density scale (0: none), y: the steam fog's, z: the air's humidity, w: how
    // far the valley fog has lifted (0..1).
    fog: vec4<f32>,
    // x: how far it has formed (0..1), y: the water's temperature less the air's, z: the sun's
    // warmth (0..1).
    fog2: vec4<f32>,
    // How far the valley fog's noise and the steam fog's have drifted (the wind, the steam's
    // rising): whole blocks (xyz), and the fraction of a block.
    valley_drift: vec4<f32>,
    valley_drift_f: vec4<f32>,
    steam_drift: vec4<f32>,
    steam_drift_f: vec4<f32>,
    // The far slices past `depth.y`, the fogs' only: x the last one's far end, y log(x / depth.y),
    // z how many, w 1 when the fogs are on.
    far: vec4<f32>,
}
@group(0) @binding(0) var<uniform> fx: Fx;
@group(0) @binding(1) var depth: texture_depth_2d;
// The voxel grid (gfx/voxels.rs): 512 bits an 8x8x8 brick in four texels along x, the solid blocks
// and the cutout ones; one texel a brick (bit 0 any block, bit 1 cutout ones); one a 32x32x32 brick.
@group(0) @binding(2) var solid_masks: texture_3d<u32>;
@group(0) @binding(3) var cutout_masks: texture_3d<u32>;
@group(0) @binding(4) var bricks: texture_3d<u32>;
@group(0) @binding(5) var supers: texture_3d<u32>;
// The light sources in range (gfx/voxels.rs `light_cells`): the centre relative to the grid's first
// block (y from the world's bottom) and the reach; the colour (gamma) and strength.
struct Light {
    at: vec4<f32>,
    colour: vec4<f32>,
}
@group(0) @binding(7) var<storage, read> lights: array<Light>;
// For each 8-block cell of the grid (x, then z, then y), the eight lights lighting it most
// (indices + 1, 0 none, 16 bits each, two to a word).
@group(0) @binding(8) var<storage, read> cells: array<vec4<u32>>;
// The sky (gfx/atmosphere.rs): its radiance by azimuth from the sun and elevation.
@group(0) @binding(10) var sky_view: texture_2d<f32>;
// The last frame's froxels (rgb: the source, a: how many frames it holds), the sample drawn this
// frame, the froxels written, the integrated ones, and what the composite reads.
@group(0) @binding(11) var history: texture_3d<f32>;
@group(0) @binding(12) var linear_sampler: sampler;
@group(0) @binding(13) var source_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(21) var fresh_in: texture_3d<f32>;
@group(0) @binding(14) var integrated_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(22) var fresh_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(19) var source_in: texture_3d<f32>;
@group(0) @binding(20) var integrated: texture_3d<f32>;
// The natural fogs' relief map (gfx/natural_fog.rs `finish`): four layers of 256 x 256 texels, the
// near layer's shape (ground, surface, the pool's top, the valley floor) and climate (humid, calm
// water's share, the air's cold in degrees, 0), then the far layer's.
@group(0) @binding(24) var<storage, read> fog_map: array<vec4<f32>>;

const PI: f32 = 3.14159265;
// Light sources' brightness and the rays' budget for them, as shadow_rays.wgsl's.
const BLOCK_LIGHT: f32 = 3.0;
const LIGHT_BUDGET: i32 = 16;

// The distance (metres, blocks) at slice coordinate `t` (0..1), and the coordinate of a distance.
fn slice_r(t: f32) -> f32 {
    return fx.depth.x * exp(t * fx.depth.z);
}
fn slice_t(r: f32) -> f32 {
    return clamp(log(max(r, fx.depth.x) / fx.depth.x) / fx.depth.z, 0.0, 1.0);
}
// The unit direction of the view's screen position `uv` (0..1, y down), from the eye the view's rays
// start at: the point unprojected at mid depth lies a decimetre ahead of the eye, so the eye's
// offset from the camera (the view bobbing's) has to come off it, or every step would swing the rays.
fn ray_dir(uv: vec2<f32>) -> vec3<f32> {
    let ndc = vec2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let c = fx.inverse_view_proj * vec4(ndc, 0.5, 1.0);
    return normalize(c.xyz / c.w - fx.eye.xyz);
}
// Henyey-Greenstein's phase function (it integrates to 1 over the sphere), `c` the cosine between
// the light's travel and the view's (1: scattering forward, toward the light when looking at it).
fn hg(c: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * c;
    return (1.0 - g * g) / (4.0 * PI * pow(max(d, 1e-4), 1.5));
}
fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}
// Three random numbers (PCG3D, Jarzynski and Olano), 0..1.
fn random3(v_in: vec3<u32>) -> vec3<f32> {
    var v = v_in * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return vec3<f32>(v >> vec3(8u)) / 16777216.0;
}

// ---- The voxel grid's rays (shadow_rays.wgsl `ray`, without the blocks' shapes), used where the
// GPU traces no hardware rays. ----
fn in_grid(v: vec3<i32>) -> bool {
    let g = fx.grid;
    let w = i32(g.z);
    let o = vec2<i32>(i32(g.x), i32(g.y));
    return v.y < 320 && v.x >= o.x && v.x < o.x + w && v.z >= o.y && v.z < o.y + w;
}
fn brick_slot(v: vec3<i32>, bits: u32) -> vec3<i32> {
    let w = i32(fx.grid.z) >> bits;
    let b = vec3<i32>(v.x >> bits, (v.y + 64) >> bits, v.z >> bits);
    return vec3<i32>(((b.x % w) + w) % w, b.y, ((b.z % w) + w) % w);
}
fn brick_exit(world: vec3<i32>, bits: u32, o: vec3<f32>, l: vec3<f32>) -> f32 {
    let size = f32(1u << bits);
    let low = vec3<f32>(((world + vec3(0, 64, 0)) >> vec3(bits)) << vec3(bits))
        - vec3(0.0, 64.0, 0.0) - fx.camera.xyz;
    let far = select(low, low + size, l > vec3(0.0));
    let exit = select((far - o) / l, vec3(1e30), abs(l) < vec3(1e-6));
    return min(min(exit.x, exit.y), exit.z);
}
fn mask_bit(masks: texture_3d<u32>, slot: vec3<i32>, l: vec3<i32>) -> bool {
    let k = l.z * 2 + (l.y >> 2u);
    let word = textureLoad(masks, vec3(slot.x * 4 + (k >> 2u), slot.y, slot.z), 0)[k & 3];
    return ((word >> u32((l.y & 3) * 8 + l.x)) & 1u) != 0u;
}
// The light the ray from camera-relative `pos` along `l` lets through before it leaves the grid,
// rises above its highest block, spends `budget_in` or goes `reach`: a solid block stops it, leaves
// let half through each.
fn dda_light(pos: vec3<f32>, l: vec3<f32>, budget_in: i32, reach: f32) -> f32 {
    let o = pos + fx.fraction.xyz;
    let camera = vec3<i32>(fx.camera.xyz);
    let start = vec3<i32>(floor(o)) + camera;
    let step = vec3<i32>(sign(l));
    let delta = 1.0 / max(abs(l), vec3(1e-6));
    let top = fx.camera.w - fx.camera.y;
    var light = 1.0;
    var t = 0.0;
    var budget = budget_in;
    while budget > 0 && t < reach {
        let p = o + l * t;
        if l.y > 0.0 && p.y >= top { break; }
        let cell = vec3<i32>(floor(p));
        let world = cell + camera;
        if !in_grid(world) { break; }
        if world.y < -64 { return 0.0; }
        if textureLoad(supers, brick_slot(world, 5u), 0).r == 0u {
            t = max(brick_exit(world, 5u, o, l), t) + 1e-3;
            budget -= 1;
            continue;
        }
        let corner = (world >> vec3(3u)) << vec3(3u);
        let leave = brick_exit(world, 3u, o, l);
        let slot = brick_slot(world, 3u);
        let kinds = textureLoad(bricks, slot, 0).r;
        if kinds != 0u {
            let leaves = (kinds & 2u) != 0u;
            let into = select(floor(p) + 1.0 - p, p - floor(p), l < vec3(0.0));
            var next = t + into * delta;
            var local = world - corner;
            var entered = t;
            for (var i = 0; i < 25; i++) {
                if any(local < vec3(0)) || any(local > vec3(7)) { break; }
                if entered >= reach { return light; }
                let block = local + corner;
                let own = all(block == start);
                let solid = !own && mask_bit(solid_masks, slot, local);
                let cut = !own && !solid && leaves && mask_bit(cutout_masks, slot, local);
                if solid { return 0.0; }
                if cut {
                    light *= 0.5;
                    if light < 0.1 { return 0.0; }
                }
                if next.x <= next.y && next.x <= next.z {
                    local.x += step.x;
                    entered = next.x;
                    next.x += delta.x;
                } else if next.y <= next.z {
                    local.y += step.y;
                    entered = next.y;
                    next.y += delta.y;
                } else {
                    local.z += step.z;
                    entered = next.z;
                    next.z += delta.z;
                }
            }
            budget -= 4;
        } else {
            budget -= 1;
        }
        t = max(leave, t) + 1e-3;
    }
    if budget <= 0 && t < reach { return 0.0; }
    return light;
}
// HARDWARE:BEGIN
const HW: bool = false;
fn hw_light(pos: vec3<f32>, l: vec3<f32>, reach: f32) -> vec3<f32> { return vec3(-1.0); }
// HARDWARE:END
// The light that reaches camera-relative `pos` along `l` (unit) for `reach`, in colour: the
// hybrid's exact triangles where the GPU traces rays, the voxel grid's otherwise.
fn visibility(pos: vec3<f32>, l: vec3<f32>, reach: f32) -> vec3<f32> {
    if HW { return hw_light(pos, l, reach); }
    return vec3(dda_light(pos, l, i32(fx.grid.w), reach));
}

// ---- The sky ----
fn sky_texel(at: vec2<i32>, top: vec2<i32>) -> vec3<f32> {
    return textureLoad(sky_view, clamp(at, vec2(0), top), 0).rgb;
}
// The sky toward `d` (unit), from the sky table, bilinear (shadow_rays.wgsl `sky_at`).
fn sky_at(d: vec3<f32>) -> vec3<f32> {
    let flat = d.xz / max(length(d.xz), 1e-5);
    let sun = fx.sun.xz / max(length(fx.sun.xz), 1e-5);
    let azimuth = acos(clamp(dot(flat, sun), -1.0, 1.0)) / 3.14159265;
    let e = asin(clamp(d.y, -1.0, 1.0)) / 1.5707963;
    let size = vec2<f32>(textureDimensions(sky_view));
    let p = vec2(azimuth, 0.5 + 0.5 * sign(e) * sqrt(abs(e))) * size - 0.5;
    let base = vec2<i32>(floor(p));
    let f = p - floor(p);
    let top = vec2<i32>(size) - 1;
    let low = mix(sky_texel(base, top), sky_texel(base + vec2(1, 0), top), f.x);
    let high = mix(sky_texel(base + vec2(0, 1), top), sky_texel(base + vec2(1, 1), top), f.x);
    return mix(low, high, f.y);
}
// The sky's mean light over the sphere: what an isotropic medium scatters of it.
fn sky_mean() -> vec3<f32> {
    return (sky_at(vec3(0.0, 1.0, 0.0)) * 2.0 + sky_at(vec3(1.0, 0.1, 0.0))
        + sky_at(vec3(-1.0, 0.1, 0.0)) + sky_at(normalize(vec3(0.0, 0.1, 1.0)))
        + sky_at(normalize(vec3(0.0, 0.1, -1.0))) + sky_at(vec3(0.0, -1.0, 0.0))) / 7.0;
}

// ---- The light sources (shadow_rays.wgsl `light_path`, cheaper: toward the block's middle) ----
fn light_cell(local: vec3<f32>) -> vec4<u32> {
    let cell = vec3<i32>(floor(local / 8.0));
    let count = vec3<i32>(i32(fx.grid.z) / 8, 48, i32(fx.grid.z) / 8);
    if any(cell < vec3(0)) || any(cell >= count) { return vec4(0u); }
    return cells[(cell.y * count.z + cell.z) * count.x + cell.x];
}
fn cell_light(words: vec4<u32>, i: i32) -> u32 {
    return (words[i / 2] >> (u32(i % 2) * 16u)) & 0xffffu;
}
// Light source `index` (+ 1)'s point, for a point at grid-local `local`: the light it gives there,
// unshadowed (physical falloff eased to nothing at its reach), and the direction toward it and
// its distance (w).
struct LightTo {
    light: vec3<f32>,
    dir: vec4<f32>,
}
fn light_to(index: u32, local: vec3<f32>) -> LightTo {
    let light = lights[index - 1u];
    let middle = floor(light.at.xyz) + vec3(0.5, 0.55, 0.5);
    let to = middle - local;
    let d = length(to);
    let reach = light.at.w;
    if d >= reach { return LightTo(vec3(0.0), vec4(0.0, 1.0, 0.0, 0.0)); }
    let window = 1.0 - (d / reach) * (d / reach);
    let gain = light.colour.w * window * window / ((1.0 + d) * (1.0 + d));
    return LightTo(pow(light.colour.rgb, vec3(2.2)) * (BLOCK_LIGHT * gain),
        vec4(to / max(d, 1e-4), d));
}

// ---- The natural fogs (gfx/natural_fog.rs) ----
const MAP_SIZE: i32 = 256;
// The valley fog's densest extinction a block (a fog some 80 blocks deep in sight), the steam
// fog's, their scattering albedo, and the steam fog's most height above the water.
const VALLEY_DENSITY: f32 = 0.025;
const STEAM_DENSITY: f32 = 0.3;
const FOG_ALBEDO: f32 = 0.98;
const STEAM_TOP: f32 = 5.0;

fn fog_on() -> bool {
    return fx.far.w > 0.5;
}
fn map_texel(layer: i32, t: vec2<i32>) -> vec4<f32> {
    let c = clamp(t, vec2(0), vec2(MAP_SIZE - 1));
    return fog_map[(layer * MAP_SIZE + c.y) * MAP_SIZE + c.x];
}
// A layer's shape and climate at texel coordinate `p` (texel centres at .5), bilinear.
struct FogTexel {
    shape: vec4<f32>,
    climate: vec4<f32>,
}
fn map_bilinear(l: i32, b: vec2<i32>, f: vec2<f32>) -> vec4<f32> {
    return mix(mix(map_texel(l, b), map_texel(l, b + vec2(1, 0)), f.x),
        mix(map_texel(l, b + vec2(0, 1)), map_texel(l, b + vec2(1, 1)), f.x), f.y);
}
fn map_layer(layer: i32, p_in: vec2<f32>) -> FogTexel {
    let p = p_in - 0.5;
    let b = vec2<i32>(floor(p));
    let f = p - floor(p);
    return FogTexel(map_bilinear(layer * 2, b, f), map_bilinear(layer * 2 + 1, b, f));
}
// The relief map at camera-relative `xz`: the near layer within it, faded into the far one over
// its last 24 texels.
fn fog_map_at(xz: vec2<f32>) -> FogTexel {
    let rel = xz + fx.fraction.xz;
    let near = (fx.map.xy + rel) * fx.map_scale.x;
    let edge = min(min(near.x, near.y), f32(MAP_SIZE) - max(near.x, near.y));
    let w = clamp((edge - 4.0) / 24.0, 0.0, 1.0) * fx.map_scale.z;
    if w >= 1.0 { return map_layer(0, near); }
    let far = map_layer(1, (fx.map.zw + rel) * fx.map_scale.y);
    if w <= 0.0 { return far; }
    let n = map_layer(0, near);
    return FogTexel(mix(far.shape, n.shape, w), mix(far.climate, n.climate, w));
}
// A number 0..1 a lattice point (PCG3D's first output).
fn hash_cell(c: vec3<i32>) -> f32 {
    var v = bitcast<vec3<u32>>(c) * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3(16u);
    v.x += v.y * v.z;
    return f32(v.x >> 8u) / 16777216.0;
}
// Value noise 0..1 on a lattice 2^k blocks wide, at the world point `base + off` (`base` whole
// blocks; `off` blocks): the lattice cell and the place within it come from the whole blocks'
// bits, so nothing loses precision far from the world's origin.
fn noise_at(base: vec3<i32>, off: vec3<f32>, k: u32) -> f32 {
    let fo = floor(off);
    let block = base + vec3<i32>(fo);
    let size = f32(1u << k);
    let mask = vec3((1 << k) - 1);
    let f = (vec3<f32>(block & mask) + off - fo) / size;
    let cell = block >> vec3(k);
    let s = f * f * (3.0 - 2.0 * f);
    let a = mix(hash_cell(cell), hash_cell(cell + vec3(1, 0, 0)), s.x);
    let b = mix(hash_cell(cell + vec3(0, 1, 0)), hash_cell(cell + vec3(1, 1, 0)), s.x);
    let c = mix(hash_cell(cell + vec3(0, 0, 1)), hash_cell(cell + vec3(1, 0, 1)), s.x);
    let d = mix(hash_cell(cell + vec3(0, 1, 1)), hash_cell(cell + vec3(1, 1, 1)), s.x);
    return mix(mix(a, b, s.y), mix(c, d, s.y), s.z);
}
// The noise's lattice turned about the vertical (an axis-aligned lattice shows its rows as a grid,
// a pattern repeating across the fog): xz times the integer matrix [[a, b], [-b, a]], exact on the
// whole blocks, so its cells are 2^k / sqrt(a² + b²) blocks wide in the world.
fn turn(i: u32) -> vec2<i32> {
    if i == 0u { return vec2(3, 4); }
    if i == 1u { return vec2(4, -3); }
    return vec2(5, 12);
}
// That noise as seen by samples `spacing` blocks apart: faded to its mean as the samples grow
// sparser than half its cells (sampled sparser, it would alias into a moiré far away), and not
// computed once gone.
fn noise_lod(base: vec3<i32>, off: vec3<f32>, k: u32, t: u32, spacing: f32) -> f32 {
    let m = turn(t);
    let size = f32(1u << k) / length(vec2<f32>(m));
    let w = 1.0 - smoothstep(0.25 * size, 0.6 * size, spacing);
    if w <= 0.0 { return 0.5; }
    let b = vec3(m.x * base.x + m.y * base.z, base.y, m.x * base.z - m.y * base.x);
    let o = vec3(f32(m.x) * off.x + f32(m.y) * off.z, off.y, f32(m.x) * off.z - f32(m.y) * off.x);
    return mix(0.5, noise_at(b, o, k), w);
}
// The fogs at a camera-relative point, sampled `spacing` blocks apart (the noise's finer octaves
// faded out where they would alias): the valley fog's and the steam fog's extinctions, and the
// valley pool's top there (world y).
struct FogAt {
    valley: f32,
    steam: f32,
    top: f32,
}
fn fog_at(pos: vec3<f32>, spacing: f32) -> FogAt {
    var out = FogAt(0.0, 0.0, -1e4);
    let m = fog_map_at(pos.xz);
    let y = fx.camera.y + fx.fraction.y + pos.y;
    let ground = m.shape.x;
    // Nothing under the ground (a cave beneath a valley stays clear).
    if y < ground - 1.5 { return out; }
    let local = pos + fx.fraction.xyz;
    let camera = vec3<i32>(fx.camera.xyz);
    if fx.fog.x > 0.0 {
        // The pool: from the valley floor, as deep as it has formed, its top rippled by the noise.
        let top = m.shape.w + (m.shape.z - m.shape.w) * (0.35 + 0.65 * fx.fog2.x);
        out.top = top;
        let humid = smoothstep(0.6, 0.95, fx.fog.z + 0.3 * (m.climate.x - 0.5));
        if humid > 0.0 && y < top + 4.0 {
            let base = camera + vec3<i32>(fx.valley_drift.xyz);
            let off = local + fx.valley_drift_f.xyz;
            // Cells about 26 and 6 blocks wide, the patches about 315.
            let n = 0.6 * noise_lod(base, off, 7u, 0u, spacing) + 0.4 * noise_lod(base, off, 5u, 1u, spacing);
            let patches = noise_lod(base, off, 12u, 2u, spacing);
            let top_n = top + (n - 0.5) * 6.0;
            // Lifting: its bottom rises toward its top as the sun warms the ground.
            let bottom = mix(ground - 2.0, top, fx.fog.w * 0.85);
            // A shallow pool (ground fog over a plain) fades out over most of its depth, a deep
            // one has a sharp top.
            let edge = clamp(0.6 * (top - m.shape.w), 3.0, 8.0);
            let body = smoothstep(top_n, top_n - edge, y)
                * smoothstep(bottom, bottom + max(3.0, (top - bottom) * 0.4), y);
            // Tearing: the noise's low parts clear first.
            let threshold = mix(0.3, 0.7, fx.fog.w);
            let tear = clamp((mix(n, patches, 0.45) - threshold) * 3.0, 0.0, 1.0);
            out.valley = fx.fog.x * VALLEY_DENSITY * humid * body * tear;
        }
    }
    let water = m.climate.y;
    let h = y - m.shape.y;
    if fx.fog.y > 0.0 && water > 0.05 && h > -0.5 && h < STEAM_TOP {
        // Colder air over warmer water (a cold biome's air colder still), the sun's warmth
        // burning it off.
        let cold = smoothstep(2.0, 9.0, fx.fog2.y - 0.5 * m.climate.z) * (1.0 - 0.7 * fx.fog2.z);
        if cold > 0.0 {
            let base = camera + vec3<i32>(fx.steam_drift.xyz);
            var off = local + fx.steam_drift_f.xyz;
            // Wisps: the noise's lattice bent by a coarser noise, rising and drifting.
            // Cells about 6, 3 and 2.5 blocks wide.
            let warp = noise_lod(base, off, 5u, 0u, spacing) - 0.5;
            off += vec3(warp * 3.0, 0.0, -warp * 3.0);
            let n = 0.6 * noise_lod(base, off, 4u, 1u, spacing) + 0.4 * noise_lod(base, off, 5u, 2u, spacing);
            let profile = exp(-max(h, 0.0) / 1.3) * smoothstep(-0.5, 0.1, h);
            let wisp = clamp(n * 2.0 - 0.6 + 0.35 * exp(-max(h, 0.0)), 0.0, 1.0);
            out.steam = fx.fog.y * STEAM_DENSITY * water * cold * profile * wisp;
        }
    }
    return out;
}
// The fogs' phase: water droplets scatter mostly forward (the sun's glow through them), a little
// back.
fn fog_phase(c: f32) -> f32 {
    return mix(hg(c, 0.8), hg(c, -0.2), 0.2);
}
// The sun's light a fog scatters toward the view (`c` the cosine to the sun), `sun_t` of it
// reaching the point through the fog: the single scattering by the droplets' phase, and the
// multiple scattering of a thick fog (dimmed far less with depth, its phase broadened: the octaves
// of Wrenninge's approximation, folded into one).
fn fog_sun(c: f32, sun_t: f32) -> f32 {
    return fog_phase(c) * sun_t + 0.6 * hg(c, 0.3) * pow(sun_t, 0.2);
}
// The fogs' optical depth from `pos` toward `l` until above the pool's top (`top`, world y) or
// `reach` blocks, in four steps (jittered by `rnd`).
fn fog_depth_toward(pos: vec3<f32>, l: vec3<f32>, top: f32, reach: f32, rnd: f32) -> f32 {
    let y = fx.camera.y + fx.fraction.y + pos.y;
    let up = max(top + 3.0 - y, STEAM_TOP);
    let len = min(up / max(l.y, 0.06), reach);
    var tau = 0.0;
    for (var i = 0; i < 4; i++) {
        let f = fog_at(pos + l * (len * (f32(i) + rnd) / 4.0), len / 4.0);
        tau += (f.valley + f.steam) * len / 4.0;
    }
    return tau;
}
// The sun's light from it reaching camera-relative `pos`, before the phase (`pi * E * seen`).
fn sun_scatter(pos: vec3<f32>, rnd: vec3<f32>) -> vec3<f32> {
    let l = fx.light.xyz;
    if fx.light.w <= 0.0 { return vec3(0.0); }
    // A point of the disc: the shafts' edges soften as the sun's own shadows do.
    let t = normalize(select(cross(l, vec3(0.0, 1.0, 0.0)), cross(l, vec3(1.0, 0.0, 0.0)),
        abs(l.y) > 0.9));
    let b = cross(l, t);
    let r = sqrt(rnd.x) * fx.sun_light.w;
    let a = 2.0 * PI * rnd.y;
    let d = normalize(l + (t * cos(a) + b * sin(a)) * r);
    let seen = visibility(pos, d, 4096.0);
    return PI * fx.sun_light.rgb * seen;
}
// The sky's light in the medium, reached from above: a cosine-weighted ray toward the sky.
fn ambient_scatter(pos: vec3<f32>, rnd: vec2<f32>) -> vec3<f32> {
    let r = sqrt(rnd.x);
    let a = 2.0 * PI * rnd.y;
    let d = normalize(vec3(r * cos(a), sqrt(max(1.0 - rnd.x, 1e-3)), r * sin(a)));
    let seen = visibility(pos, d, 4096.0);
    return sky_mean() * fx.albedo.w * seen;
}
// One light source of the froxel's cell, drawn by its light there (resampled importance
// sampling with one sample: the sum of the weights over the chosen one's), through a shadow ray.
fn lights_scatter(pos: vec3<f32>, dir: vec3<f32>, rnd: vec3<f32>) -> vec3<f32> {
    let local = pos + fx.camera.xyz + fx.fraction.xyz - vec3(fx.grid.x, -64.0, fx.grid.y);
    let words = light_cell(local);
    var total = 0.0;
    var chosen = 0u;
    var chosen_weight = 0.0;
    for (var k = 0; k < 8; k++) {
        let index = cell_light(words, k);
        if index == 0u { continue; }
        let w = luminance(light_to(index, local).light);
        if w <= 0.0 { continue; }
        total += w;
        let pick = fract(rnd.z * 7919.0 + f32(k) * 0.618034 + rnd.x);
        if pick * total < w {
            chosen = index;
            chosen_weight = w;
        }
    }
    if chosen == 0u { return vec3(0.0); }
    let to = light_to(chosen, local);
    let seen = visibility(pos, to.dir.xyz, max(to.dir.w - 0.6, 0.0));
    return PI * hg(dot(dir, to.dir.xyz), fx.sigma_t.w) * to.light * (total / chosen_weight)
        * seen * fx.history.y;
}

@compute @workgroup_size(4, 4, 4)
fn cs_inject(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec3<u32>(fx.dims.xyz);
    if any(id >= dims) { return; }
    let rnd = random3(vec3(id.x + u32(fx.dims.w) * 16411u, id.y * 3571u, id.z + 977u * u32(fx.dims.w)));
    let rnd2 = random3(id * 7919u + vec3(u32(fx.dims.w) * 31u, 17u, 5u));
    let size = fx.dims.xyz;
    // The point drawn within the froxel this frame: the frame's R3 offset shifted by the froxel's own
    // random number (a Cranley-Patterson rotation), so each froxel's samples over the frames cover
    // it evenly, far sooner than random ones, and the grid's aliasing leaves no pattern across it.
    let shift = random3(id * 2654435761u + vec3(101u, 7u, 13u));
    let at = (vec3<f32>(id) + fract(fx.jitter.xyz + shift)) / size;
    let dir = ray_dir(at.xy);
    let pos = fx.eye.xyz + dir * slice_r(at.z);
    let c = dot(dir, fx.light.xyz);
    let sun = sun_scatter(pos, rnd2);
    let ambient = ambient_scatter(pos, rnd.zx);
    let lamps = lights_scatter(pos, dir, rnd2.zxy);
    var fresh = sun * hg(c, fx.sigma_t.w) + ambient + lamps;
    if fog_on() {
        // In the fogs: the sun's light dimmed by the fog toward it (their top lit, their depths in
        // their own shade), with a share of multiple scattering (dimmed less, scattered evenly);
        // the sky's dimmed by the fog above. The air and the fog each scatter by their own phase,
        // mixed by the fog's mean over the froxel's slice along its centre ray, the very fog the
        // integration dims it by (at the point drawn alone, a froxel across the fog's edge would
        // light its fog as clear air, or the reverse, and show the slices as bands).
        let centre = (vec3<f32>(id) + 0.5) / size;
        let r_c = slice_r(centre.z);
        let width = r_c * length(ray_dir((vec2<f32>(id.xy) + vec2(1.5, 0.5)) / size.xy) - ray_dir(centre.xy));
        let f = fog_at(fx.eye.xyz + ray_dir(centre.xy) * r_c, width);
        let mean = fog_slice(ray_dir(centre.xy), slice_r(f32(id.z) / size.z), slice_r((f32(id.z) + 1.0) / size.z));
        var sun_t = 1.0;
        var up_t = 1.0;
        if mean > 0.0 || fx.camera.y + fx.fraction.y + pos.y < f.top + 3.0 {
            if fx.light.w > 0.0 {
                sun_t = exp(-fog_depth_toward(pos, fx.light.xyz, f.top, 400.0, rnd.y));
            }
            up_t = exp(-fog_depth_toward(pos, vec3(0.0, 1.0, 0.0), f.top, 64.0, rnd.z));
        }
        let air = sun * hg(c, fx.sigma_t.w) * sun_t + ambient * sqrt(up_t) + lamps;
        let fog = sun * fog_sun(c, sun_t) + ambient * (0.7 + 0.3 * sqrt(up_t)) + lamps;
        let sa = luminance(fx.sigma_t.rgb * fx.albedo.rgb);
        let sf = mean * FOG_ALBEDO;
        fresh = (air * sa + fog * sf) / max(sa + sf, 1e-8);
    }
    textureStore(fresh_out, vec3<i32>(id), vec4(fresh, 1.0));
}

// The most the history keeps of its length where its froxel moved far, and the froxels' move (the
// grid's own units) it starts to shorten and is shortest at.
const MOVED_KEEPS: f32 = 0.1;
const MOVED_FROM: f32 = 0.25;
const MOVED_TO: f32 = 1.75;

@compute @workgroup_size(4, 4, 4)
fn cs_blend(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec3<u32>(fx.dims.xyz);
    if any(id >= dims) { return; }
    let size = fx.dims.xyz;
    let top = vec3<i32>(dims) - 1;
    let here_id = vec3<i32>(id);
    var fresh = textureLoad(fresh_in, here_id, 0).rgb;
    // This frame's samples around the froxel: their mean and spread say what the light there is now.
    var sum = vec3(0.0);
    var sum_squares = vec3(0.0);
    for (var dz = -1; dz <= 1; dz++) {
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let c = textureLoad(fresh_in, clamp(here_id + vec3(dx, dy, dz), vec3(0), top), 0).rgb;
                sum += c;
                sum_squares += c * c;
            }
        }
    }
    let mean = sum / 27.0;
    let spread = sqrt(max(sum_squares / 27.0 - mean * mean, vec3(0.0)));
    // Frame to frame: where this froxel's centre stood in the last frame's grid, in world space from
    // the eye each frame's rays start at.
    var old = vec4(0.0);
    var moved = 0.0;
    if fx.motion.w > 0.5 {
        let centre = (vec3<f32>(id) + 0.5) / size;
        let here = fx.eye.xyz + ray_dir(centre.xy) * slice_r(centre.z);
        let before = here + fx.motion.xyz;
        let clip = fx.prev_view_proj * vec4(before, 1.0);
        let r = length(before - fx.prev_eye.xyz);
        if clip.w > 0.0 {
            let ndc = clip.xy / clip.w;
            let uv = vec3(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, slice_t(r));
            if all(uv >= vec3(0.0)) && all(uv <= vec3(1.0)) && r <= fx.depth.y {
                old = textureSampleLevel(history, linear_sampler, uv, 0.0);
                moved = length((uv - centre) * size);
            }
        }
    }
    var count = old.a;
    // Where the froxel's content moved a lot, the history has been resampled blurred and its shafts
    // lag: it keeps a shorter length there, and is held within what the neighbourhood now shows
    // (loosely when the froxel stayed, whose long history the shadows' noise would spoil).
    let shake = smoothstep(MOVED_FROM, MOVED_TO, moved);
    count *= mix(1.0, MOVED_KEEPS, shake);
    let band = mix(6.0, 1.5, shake) * spread + 0.15 * mean + vec3(1e-4);
    old = vec4(clamp(old.rgb, mean - band, mean + band), old.a);
    // A froxel the last frame's grid had no light for (new at the view's edge) takes its
    // neighbourhood's mean rather than one noisy sample.
    if count <= 0.0 { fresh = mean; }
    let n = min(count + 1.0, fx.history.x);
    // Bright outliers (a ray through a gap) are held to what the history shows, beyond a few
    // times its light, so one lucky ray doesn't flash.
    let limit = max(luminance(old.rgb) * 6.0, 0.5);
    let lum = luminance(fresh);
    if count > 0.0 && lum > limit { fresh *= limit / lum; }
    let out = mix(old.rgb, fresh, 1.0 / n);
    textureStore(source_out, here_id, vec4(out, n));
}

// The fogs' mean extinction over the slice from `near` to `far` along `dir`: a point every two
// blocks or so (2 to 6), so a thin layer seen edge-on doesn't band with the slices.
fn fog_slice(dir: vec3<f32>, near: f32, far: f32) -> f32 {
    let n = clamp(i32(ceil((far - near) * 0.5)), 2, 6);
    var sum = 0.0;
    for (var i = 0; i < n; i++) {
        let f = fog_at(fx.eye.xyz + dir * mix(near, far, (f32(i) + 0.5) / f32(n)), (far - near) / f32(n));
        sum += f.valley + f.steam;
    }
    return sum / f32(n);
}
// The far slices' distance at coordinate `t` (0..1) past `depth.y`, and back.
fn far_r(t: f32) -> f32 {
    return fx.depth.y * exp(t * fx.far.y);
}
fn far_t(r: f32) -> f32 {
    return clamp(log(max(r, fx.depth.y) / fx.depth.y) / fx.far.y, 0.0, 1.0);
}

@compute @workgroup_size(8, 8, 1)
fn cs_integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec3<u32>(fx.dims.xyz);
    if any(id.xy >= dims.xy) { return; }
    let st = max(fx.sigma_t.rgb, vec3(1e-6));
    let ss = st * fx.albedo.rgb;
    let fogs = fog_on();
    let dir = ray_dir((vec2<f32>(id.xy) + 0.5) / fx.dims.xy);
    var scattered = vec3(0.0);
    var through = vec3(1.0);
    // What the fogs alone let through (grey): the composite multiplies it into the scene beside
    // the medium's closed form.
    var fog_through = 1.0;
    var last = 0.0;
    for (var z = 0u; z < dims.z; z++) {
        let r = slice_r(f32(z + 1u) / fx.dims.z);
        let dz = r - last;
        var fog = 0.0;
        if fogs { fog = fog_slice(dir, last, r); }
        last = r;
        let j = textureLoad(source_in, vec3<i32>(vec3(id.xy, z)), 0).rgb;
        let t = st + fog;
        let step_t = exp(-t * dz);
        // The source is constant over the slice: its in-scatter to the slice's near end, exactly.
        scattered += through * j * (ss + fog * FOG_ALBEDO) * (1.0 - step_t) / t;
        through *= step_t;
        fog_through *= exp(-fog * dz);
        textureStore(integrated_out, vec3<i32>(vec3(id.xy, z)), vec4(scattered, fog_through));
    }
    // Past the froxels, the fogs alone out to the far terrain, lit without rays: the sun through
    // the fog up to the pool's top, and the sky.
    let far = u32(fx.far.z);
    let c = dot(dir, fx.light.xyz);
    // The angle between this column's ray and the next one's (a far sample's width per block away).
    let spread = length(ray_dir((vec2<f32>(id.xy) + vec2(1.5, 0.5)) / fx.dims.xy) - dir);
    var sky = vec3(0.0);
    if fogs { sky = sky_mean() * fx.albedo.w * 0.9; }
    for (var z = 0u; z < far; z++) {
        if fogs {
            let r = far_r(f32(z + 1u) / fx.far.z);
            let dz = r - last;
            var fog = 0.0;
            var j = vec3(0.0);
            for (var i = 0; i < 4; i++) {
                let pos = fx.eye.xyz + dir * mix(last, r, (f32(i) + 0.5) / 4.0);
                let f = fog_at(pos, max(dz / 4.0, r * spread));
                let d = f.valley + f.steam;
                if d > 0.0 {
                    let y = fx.camera.y + fx.fraction.y + pos.y;
                    let sun_t = exp(-d * clamp((f.top + 2.0 - y) / max(fx.light.y, 0.06), 0.0, 400.0));
                    var sun = vec3(0.0);
                    if fx.light.w > 0.0 {
                        sun = PI * fx.sun_light.rgb * fog_sun(c, sun_t);
                    }
                    j += d * (sun + sky);
                    fog += d;
                }
            }
            last = r;
            if fog > 0.0 {
                j /= fog;
                fog *= 0.25;
                let step_t = exp(-fog * dz);
                scattered += through * j * FOG_ALBEDO * (1.0 - step_t);
                through *= step_t;
                fog_through *= step_t;
            }
        }
        textureStore(integrated_out, vec3<i32>(vec3(id.xy, dims.z + z)), vec4(scattered, fog_through));
    }
}

struct Screen {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex
fn vs_apply(@builtin(vertex_index) i: u32) -> Screen {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return Screen(vec4(p * 2.0 - 1.0, 0.0, 1.0), vec2(p.x, 1.0 - p.y));
}
// The distance along the ray (from the eye) of the pixel at `texel` of the depth (the far end for
// the sky).
fn pixel_distance(texel: vec2<i32>, uv: vec2<f32>) -> f32 {
    let end = select(fx.depth.y, fx.far.x, fog_on());
    let d = textureLoad(depth, texel, 0);
    if d <= 0.0 { return end; }
    let ndc = vec2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let c = fx.inverse_view_proj * vec4(ndc, d, 1.0);
    return min(length(c.xyz / c.w - fx.eye.xyz), end);
}
// The integrated froxels at screen `uv` and slice coordinate `w`, read through a cubic B-spline
// across the screen (each axis' four taps folded into two bilinear fetches, whose weights come from
// the spline's), linear in depth: smooth to the second derivative, where a plain trilinear read has
// the grid's cell edges show as kinks.
fn integrated_cubic(uv: vec2<f32>, w: f32) -> vec4<f32> {
    let size = fx.dims.xy;
    let p = uv * size - 0.5;
    let base = floor(p);
    let f = p - base;
    let f2 = f * f;
    let f3 = f2 * f;
    let w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
    let w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
    let w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
    let w3 = f3 / 6.0;
    let g0 = w0 + w1;
    let g1 = w2 + w3;
    let h0 = (base - 0.5 + w1 / g0) / size;
    let h1 = (base + 1.5 + w3 / g1) / size;
    let a = textureSampleLevel(integrated, linear_sampler, vec3(h0.x, h0.y, w), 0.0);
    let b = textureSampleLevel(integrated, linear_sampler, vec3(h1.x, h0.y, w), 0.0);
    let c = textureSampleLevel(integrated, linear_sampler, vec3(h0.x, h1.y, w), 0.0);
    let d = textureSampleLevel(integrated, linear_sampler, vec3(h1.x, h1.y, w), 0.0);
    return g0.y * (g0.x * a + g1.x * b) + g1.y * (g0.x * c + g1.x * d);
}
// A noise by pixel and frame (interleaved gradient noise), -0.5..0.5.
fn pixel_dither(pixel: vec2<f32>) -> f32 {
    let q = pixel + 5.588238 * (fx.dims.w % 64.0);
    return fract(52.9829189 * fract(dot(q, vec2(0.06711056, 0.00583715)))) - 0.5;
}
// The integrated froxels at screen `uv` and distance `r`: the in-scatter and the luminance of the
// transmittance (slice `z` holds the value at its far end: coordinate `(z + 0.5) / n`, the near
// end of the first slice being nothing and clear), read `dither` slices off (the pixel's noise:
// read in between, the slices' ends show as arcs around the eye where the fog changes within
// them; dithered, the anti-aliasing's history smooths them out).
fn froxel_at(uv: vec2<f32>, r: f32, dither: f32) -> vec4<f32> {
    let n = fx.dims.z;
    // The integrated grid holds the froxels' slices, then the far ones past `depth.y`.
    let total = n + fx.far.z;
    let t = slice_t(r);
    var slice = t * n;
    if r > fx.depth.y { slice = n + far_t(r) * fx.far.z; }
    let w = clamp(slice - 0.5 + dither, 0.5, total - 0.5) / total;
    var v = integrated_cubic(uv, w);
    if t < 1.0 / n { v = mix(vec4(0.0, 0.0, 0.0, 1.0), v, t * n); }
    return v;
}
// The light the medium adds in front of the pixel, added to the scene.
@fragment
fn fs_scatter(in: Screen) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(in.position.xy);
    let r = pixel_distance(texel, in.uv);
    let dither = pixel_dither(in.position.xy);
    let here = froxel_at(in.uv, r, dither);
    var light = here.rgb;
    // The steam fog's reflection in the water under it: the mirrored ray crosses the same thin
    // layer again, so the water reflects (Fresnel's share) the light the layer scattered along the
    // view's last stretch through it.
    if fog_on() && fx.fog.y > 0.0 && r < fx.depth.y {
        let dir = ray_dir(in.uv);
        let pos = fx.eye.xyz + dir * r;
        let m = fog_map_at(pos.xz);
        let y = fx.camera.y + fx.fraction.y + pos.y;
        if dir.y < -0.01 && m.climate.y > 0.5 && abs(y - m.shape.y) < 0.4 {
            let before = froxel_at(in.uv, max(r - STEAM_TOP / -dir.y, 0.0), dither);
            let layer = max(here.rgb - before.rgb, vec3(0.0)) / max(before.a, 0.05);
            let fresnel = 0.02 + 0.98 * pow(1.0 + dir.y, 5.0);
            light += layer * fresnel * here.a;
        }
    }
    return vec4(light, 0.0);
}
// What the medium lets through of the scene behind it (Beer-Lambert, exact for the homogeneous
// medium, out to the froxels' reach), times what the fogs let through (integrated), multiplied
// into the scene.
@fragment
fn fs_transmit(in: Screen) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(in.position.xy);
    let r = pixel_distance(texel, in.uv);
    var t = exp(-fx.sigma_t.rgb * min(r, fx.depth.y));
    if fog_on() { t *= froxel_at(in.uv, r, pixel_dither(in.position.xy)).a; }
    return vec4(t, 1.0);
}
