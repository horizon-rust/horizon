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
    // x: the first slice's far end in metres, y: the last slice's, z: log(y / x), w: the plain
    // glass's sprite layers, packed (froxels_hw.wgsl `hw_glass`).
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

// The sun's light scattered toward the view looking along `dir`, at camera-relative `pos`.
fn sun_scatter(pos: vec3<f32>, dir: vec3<f32>, rnd: vec3<f32>) -> vec3<f32> {
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
    return PI * hg(dot(dir, l), fx.sigma_t.w) * fx.sun_light.rgb * seen;
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
    var fresh = sun_scatter(pos, dir, rnd2);
    fresh += ambient_scatter(pos, rnd.zx);
    fresh += lights_scatter(pos, dir, rnd2.zxy);
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

@compute @workgroup_size(8, 8, 1)
fn cs_integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec3<u32>(fx.dims.xyz);
    if any(id.xy >= dims.xy) { return; }
    let st = max(fx.sigma_t.rgb, vec3(1e-6));
    let ss = st * fx.albedo.rgb;
    var scattered = vec3(0.0);
    var through = vec3(1.0);
    var last = 0.0;
    for (var z = 0u; z < dims.z; z++) {
        let r = slice_r(f32(z + 1u) / fx.dims.z);
        let dz = r - last;
        last = r;
        let j = textureLoad(source_in, vec3<i32>(vec3(id.xy, z)), 0).rgb;
        let step_t = exp(-st * dz);
        // The source is constant over the slice: its in-scatter to the slice's near end, exactly.
        scattered += through * j * ss * (1.0 - step_t) / st;
        through *= step_t;
        textureStore(integrated_out, vec3<i32>(vec3(id.xy, z)), vec4(scattered, luminance(through)));
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
    let d = textureLoad(depth, texel, 0);
    if d <= 0.0 { return fx.depth.y; }
    let ndc = vec2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let c = fx.inverse_view_proj * vec4(ndc, d, 1.0);
    return min(length(c.xyz / c.w - fx.eye.xyz), fx.depth.y);
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
// The integrated froxels at screen `uv` and distance `r`: the in-scatter and the luminance of the
// transmittance (slice `z` holds the value at its far end: coordinate `(z + 0.5) / n`, the near
// end of the first slice being nothing and clear).
fn froxel_at(uv: vec2<f32>, r: f32) -> vec4<f32> {
    let n = fx.dims.z;
    let t = slice_t(r);
    let w = clamp(t - 0.5 / n, 0.5 / n, 1.0 - 0.5 / n);
    var v = integrated_cubic(uv, w);
    if t < 1.0 / n { v = mix(vec4(0.0, 0.0, 0.0, 1.0), v, t * n); }
    return v;
}
// The light the medium adds in front of the pixel, added to the scene.
@fragment
fn fs_scatter(in: Screen) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(in.position.xy);
    return vec4(froxel_at(in.uv, pixel_distance(texel, in.uv)).rgb, 0.0);
}
// What the medium lets through of the scene behind it (Beer-Lambert, exact for the homogeneous
// medium), multiplied into the scene.
@fragment
fn fs_transmit(in: Screen) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(in.position.xy);
    let r = pixel_distance(texel, in.uv);
    return vec4(exp(-fx.sigma_t.rgb * r), 1.0);
}
