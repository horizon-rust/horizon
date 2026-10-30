// Ray-traced shadows' and ambient occlusion's rays (gfx/shadow_rays.rs, docs/raytracing.md): for
// every pixel of a view's depth, how much of the sun's (or the moon's) light reaches the surface it
// shows, and how open it is to the light around, traced through the voxel grid around the camera
// (gfx/voxels.rs). One ray a pixel shown, whatever was drawn over it,
// in the screen's and every VR eye's view alike.
struct Rt {
    // Clip space to camera-relative world space.
    inverse_view_proj: mat4x4<f32>,
    // xyz toward the light, w the shadows' strength.
    light: vec4<f32>,
    // xyz: the camera's block, w: the Y above the grid's highest block.
    camera: vec4<f32>,
    // xyz: the camera within its block.
    fraction: vec4<f32>,
    // x, y: the grid's first block's x and z, z: its width, w: the ray's budget.
    grid: vec4<f32>,
    // xy: the depth's size, z: depth pixels a mask texel side (the rays' scale), w: the ambient
    // occlusion's strength (0: off).
    size: vec4<f32>,
    // Foveated rays (VR): xyz where the eyes look (camera-relative), w the cosine of the sharp
    // zone's half angle; below -1 the whole view is sharp (the screen, the quad views' insets).
    fovea: vec4<f32>,
    // x: RT lighting (the mask also holds the distance), y: the sun's shadows traced.
    mode: vec4<f32>,
    // RT lighting's accumulation over frames: the view's last view projection (clip space from
    // world space relative to its camera) and the camera's move since (xyz, w 1 when the last
    // masks can be read).
    prev_view_proj: mat4x4<f32>,
    motion: vec4<f32>,
    // xyz toward the real sun (the sky table's frame), w 1 when the sky table holds its sky.
    sun: vec4<f32>,
    // x: this frame's number (the rays' directions change with it).
    frame: vec4<f32>,
    // rgb: the light the rays trace toward (the sun's through the atmosphere, or the moon's), in
    // paper whites, for the light bouncing off what the rays hit; w its disc's angular radius.
    sun_light: vec4<f32>,
    // The water's still and flowing sprite layers (-1 none) and the time in seconds, for the
    // caustics (shadow_rays_hw.wgsl `hw_tint`).
    water: vec4<f32>,
    // The camera's offset from the hardware rays' instances' anchor (gfx/hw_rays.rs).
    hw_offset: vec4<f32>,
    // x: how many of `moving` are set.
    held: vec4<f32>,
    // RT lighting's moving lights (a held torch, a burning mob: Remix's held light): camera-relative
    // position and light level (`moving_light`).
    moving: array<vec4<f32>, 8>,
}
@group(0) @binding(0) var<uniform> rt: Rt;
@group(0) @binding(1) var depth: texture_depth_2d;
// 512 bits an 8x8x8 brick in four texels along x, the solid blocks and the cutout ones; one texel
// a brick (bit 0 any block, bit 1 cutout ones); one texel a 32x32x32 brick (1: any block).
@group(0) @binding(2) var solid_masks: texture_3d<u32>;
@group(0) @binding(3) var cutout_masks: texture_3d<u32>;
@group(0) @binding(4) var bricks: texture_3d<u32>;
@group(0) @binding(5) var supers: texture_3d<u32>;
// r: the sun's light reaching the pixel, g: the ambient light (1 open, less occluded).
@group(0) @binding(6) var mask: texture_storage_2d<rgba16float, write>;
// RT lighting's light sources in range (gfx/voxels.rs `light_cells`): the centre relative to the
// grid's first block (y from the world's bottom) and the reach; the colour (gamma) and strength.
struct Light {
    at: vec4<f32>,
    colour: vec4<f32>,
}
@group(0) @binding(7) var<storage, read> lights: array<Light>;
// For each 8-block cell of the grid (x, then z, then y), the eight lights lighting it most
// (indices + 1, 0 none, 16 bits each, two to a word).
@group(0) @binding(8) var<storage, read> cells: array<vec4<u32>>;
// RT lighting's light from the light sources reaching the pixel (rgb, linear, the faces' facing
// in); a below 0 past the grid, where the terrain falls back on the block light level.
@group(0) @binding(9) var block_mask: texture_storage_2d<rgba16float, write>;
// RT lighting's sky (gfx/atmosphere.rs): its radiance by azimuth from the sun and elevation.
@group(0) @binding(10) var sky_view: texture_2d<f32>;
// The view's masks from the last frame, accumulated into.
@group(0) @binding(11) var last_mask: texture_2d<f32>;
@group(0) @binding(12) var last_blocks: texture_2d<f32>;
// RT lighting's light sources' direct light (rgb, sharp, not accumulated nor denoised) and the face
// the pixel shows (a, `face_of`), for the denoiser (denoise.wgsl).
@group(0) @binding(18) var direct_mask: texture_storage_2d<rgba16float, write>;
// The last frame's light other than the sun's, denoised and with the direct light: what a bounce
// ray finds where the last frame showed the surface it hits (`last_light`).
@group(0) @binding(19) var last_light_mask: texture_2d<f32>;
// The last frame's direct light (`direct_mask`), accumulated into: the light sources are areas,
// sampled at a point a frame, and their penumbrae average out.
@group(0) @binding(20) var last_direct: texture_2d<f32>;
// RT lighting's sun reaching the pixel through what tints it (stained glass, water: rgb, 1 all of
// it), accumulated (the disc's penumbra), and the last frame's.
@group(0) @binding(21) var sun_mask: texture_storage_2d<rgba16float, write>;
@group(0) @binding(22) var last_sun: texture_2d<f32>;
// RT lighting's ReSTIR DI reservoirs (`restir_light`), this frame's and the last frame's: the light
// (index + 1), its block, the candidates seen and the weight (the last two as float bits).
@group(0) @binding(24) var reservoirs: texture_storage_2d<rgba32uint, write>;
@group(0) @binding(25) var last_reservoirs: texture_2d<u32>;
// The reservoir the pixel's `restir_light` chose, for `cs_shadow` to store.
var<private> reservoir_out: vec4<u32>;
// RT lighting's ReSTIR GI reservoirs (`restir_gi`), this frame's and the last frame's: the sample's
// point and the weight (a); its light, the candidates seen and its normal, packed (b).
@group(0) @binding(27) var gi_a: texture_storage_2d<rgba32float, write>;
@group(0) @binding(28) var gi_b: texture_storage_2d<rgba32uint, write>;
@group(0) @binding(29) var last_gi_a: texture_2d<f32>;
@group(0) @binding(30) var last_gi_b: texture_2d<u32>;
// RT lighting's fast history (ReLAX's): the indirect and direct light over the last few frames
// (rgb) and its luminance's second moment (a), this frame's and the last frame's, which the slow
// history is clamped to (`accumulate`).
@group(0) @binding(31) var fast: texture_storage_2d<rgba16float, write>;
@group(0) @binding(32) var last_fast: texture_2d<f32>;
var<private> fast_out: vec4<f32>;
// RT lighting's primary surface replacement on water (Remix's PSR, shadow_rays_hw.wgsl
// `hw_reflect`): the light of what the water at the pixel reflects (rgb), a 1 a surface, 0 the
// sky or no water (the water's shader then reflects its own sky).
@group(0) @binding(33) var reflection: texture_storage_2d<rgba16float, write>;
// The same for what the water refracts (shadow_rays_psr.wgsl `hw_refract`): rgb its light, a the
// distance the ray runs in the water (0: none).
@group(0) @binding(34) var refraction: texture_storage_2d<rgba16float, write>;
// Where `hw_reflect` met the water: its normal toward the camera (xyz) and distance (w, below 0
// none), for `hw_refract`.
var<private> water_at: vec4<f32>;
var<private> gi_out_a: vec4<f32>;
var<private> gi_out_b: vec4<u32>;
// RT lighting's radiance cache (SHaRC-like, gfx/radiance_cache.wgsl resolves it): a hash table of
// world-space surface cells, half a block across near the camera and twice as large each doubling
// of the distance, one a face direction. Every pixel adds the light other than the sun's its rays
// found (the sky's, bounced, the light sources') to its cell; a bounce ray reads what the cell it
// hits held last frame. So light keeps bouncing off surfaces the screen no longer shows, the same
// for both eyes, and holds still at two rays a pixel.
struct CacheCell {
    key: atomic<u32>,
    // This frame's sums (fixed point, `CACHE_SCALE` a unit) and how many added to them.
    r: atomic<u32>,
    g: atomic<u32>,
    b: atomic<u32>,
    count: atomic<u32>,
    // What the cell held after the last resolve (f32 bits), and its age in frames unseen.
    light: array<u32, 3>,
    age: u32,
}
@group(0) @binding(26) var<storage, read_write> cache: array<CacheCell>;
const CACHE_SCALE: f32 = 1024.0;
const CACHE_PROBES: u32 = 8u;
// The cell key of world-relative camera-relative `pos` on a face of normal `n` (never 0).
fn cache_key(pos: vec3<f32>, n: vec3<f32>) -> u32 {
    let d = max(length(pos), 1.0);
    let level = u32(clamp(floor(log2(d / 8.0)), 0.0, 9.0));
    let size = 0.5 * exp2(f32(level));
    let world = pos + rt.camera.xyz + rt.fraction.xyz;
    let c = vec3<i32>(floor(world / size));
    var h = u32(c.x) * 73856093u ^ u32(c.y) * 19349663u ^ u32(c.z) * 83492791u;
    h ^= (level * 2654435761u) ^ (u32(face_of(n)) * 2246822519u);
    h = (h ^ (h >> 15u)) * 2246822519u;
    h = (h ^ (h >> 13u)) * 3266489917u;
    h ^= h >> 16u;
    return max(h, 1u);
}
// The slot holding `key`, taken if `insert` and none does; `u32` max when the probes find none.
fn cache_slot(key: u32, insert: bool) -> u32 {
    let mask = arrayLength(&cache) - 1u;
    for (var i = 0u; i < CACHE_PROBES; i++) {
        let slot = (key + i * i) & mask;
        let held = atomicLoad(&cache[slot].key);
        if held == key { return slot; }
        if held == 0u {
            if !insert { return 0xffffffffu; }
            let taken = atomicCompareExchangeWeak(&cache[slot].key, 0u, key);
            if taken.exchanged || taken.old_value == key { return slot; }
        }
    }
    return 0xffffffffu;
}
// A value with its NaNs and infinities as 0 (by their bits: a compiler's fast maths may drop
// `x != x`), as Remix sanitises its buffers: one bad sample would spread through the histories,
// the reservoirs and the cache.
fn finite(x: f32) -> f32 {
    return select(x, 0.0, (bitcast<u32>(x) & 0x7f800000u) == 0x7f800000u);
}
fn finite3(v: vec3<f32>) -> vec3<f32> {
    return vec3(finite(v.x), finite(v.y), finite(v.z));
}
fn finite4(v: vec4<f32>) -> vec4<f32> {
    return vec4(finite(v.x), finite(v.y), finite(v.z), finite(v.w));
}
fn cache_add(pos: vec3<f32>, n: vec3<f32>, light: vec3<f32>) {
    let slot = cache_slot(cache_key(pos, n), true);
    if slot == 0xffffffffu { return; }
    let fixed = vec3<u32>(clamp(finite3(light), vec3(0.0), vec3(64.0)) * CACHE_SCALE);
    atomicAdd(&cache[slot].r, fixed.r);
    atomicAdd(&cache[slot].g, fixed.g);
    atomicAdd(&cache[slot].b, fixed.b);
    atomicAdd(&cache[slot].count, 1u);
}
// What the cell at `pos` (normal `n`) held last frame; a below 0 when it holds nothing yet.
fn cache_light(pos: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    let slot = cache_slot(cache_key(pos, n), false);
    if slot == 0xffffffffu { return vec4(0.0, 0.0, 0.0, -1.0); }
    let held = cache[slot].light;
    if held[0] == 0xffffffffu { return vec4(0.0, 0.0, 0.0, -1.0); }
    return vec4(bitcast<f32>(held[0]), bitcast<f32>(held[1]), bitcast<f32>(held[2]), 1.0);
}

// The face a normal `n` stands for: its largest axis and the sign there, 0..5.
fn face_of(n: vec3<f32>) -> f32 {
    let a = abs(n);
    var axis = 2.0;
    if a.x >= a.y && a.x >= a.z {
        axis = 0.0;
    } else if a.y >= a.z {
        axis = 1.0;
    }
    let s = select(n.z, select(n.y, n.x, axis == 0.0), axis < 2.0);
    return axis * 2.0 + select(0.0, 1.0, s < 0.0);
}
// RT lighting's bounced light and shapes: each block's map colour (the low six bits,
// `MAP_COLORS` ids) and shape (the rest, `shapes` ids, 0 the whole block) (gfx/voxels.rs).
@group(0) @binding(13) var albedo_grid: texture_3d<u32>;
// The blocks' shapes by id (occupancy.rs `shapes`): 8x8x8 cells from their models' opaque
// texels, bit (z * 8 + y) * 8 + x of sixteen words, and whether light passes the cells in part
// (leaves, plants).
struct Shape {
    cells: array<vec4<u32>, 4>,
    flags: vec4<u32>,
}
@group(0) @binding(14) var<storage, read> shapes: array<Shape>;
// Light a cutout shape's cell lets through (a leaf's, a petal's).
const CUTOUT_CELL: f32 = 0.35;
// World block `v`'s texel in the grid's per-block textures.
fn block_slot(v: vec3<i32>) -> vec3<i32> {
    let w = i32(rt.grid.z);
    return vec3(((v.x % w) + w) % w, v.y + 64, ((v.z % w) + w) % w);
}
// A ray (from `o`, relative to the camera's block, along `l`) through world block `block` of shape
// `id`, from where it entered (`entered`) to where it leaves it (`leaving`), cell by cell: the
// light its cells let through, and where a solid cell stopped it (t, below 0 when none did).
fn through_shape(o: vec3<f32>, l: vec3<f32>, block: vec3<i32>, id: u32, entered: f32,
    leaving: f32) -> vec2<f32> {
    let cutout = shapes[id].flags.x != 0u;
    let base = vec3<f32>(block - vec3<i32>(rt.camera.xyz));
    let q = (o + l * (entered + 1e-4) - base) * 8.0;
    var cell = clamp(vec3<i32>(floor(q)), vec3(0), vec3(7));
    let step = vec3<i32>(sign(l));
    let delta = 0.125 / max(abs(l), vec3(1e-6));
    let f = q - floor(q);
    var next = entered + select(1.0 - f, f, l < vec3(0.0)) * delta;
    var at = entered;
    var light = 1.0;
    for (var i = 0; i < 24; i++) {
        if any(cell < vec3(0)) || any(cell > vec3(7)) || at >= leaving { break; }
        let bit = u32((cell.z * 8 + cell.y) * 8 + cell.x);
        let word = shapes[id].cells[bit >> 7u][(bit >> 5u) & 3u];
        if ((word >> (bit & 31u)) & 1u) != 0u {
            if !cutout { return vec2(0.0, at); }
            light *= CUTOUT_CELL;
        }
        if next.x <= next.y && next.x <= next.z {
            cell.x += step.x;
            at = next.x;
            next.x += delta.x;
        } else if next.y <= next.z {
            cell.y += step.y;
            at = next.y;
            next.y += delta.y;
        } else {
            cell.z += step.z;
            at = next.z;
            next.z += delta.z;
        }
    }
    return vec2(light, -1.0);
}
// blocks.rs `MAP_COLORS`: MapColor.col by id (gamma RGB).
const MAP_COLORS: array<u32, 62> = array<u32, 62>(
    0u, 8368696u, 16247203u, 13092807u, 16711680u, 10526975u, 10987431u, 31744u,
    16777215u, 10791096u, 9923917u, 7368816u, 4210943u, 9402184u, 16776437u, 14188339u,
    11685080u, 6724056u, 15066419u, 8375321u, 15892389u, 5000268u, 10066329u, 5013401u,
    8339378u, 3361970u, 6704179u, 6717235u, 10040115u, 1644825u, 16445005u, 6085589u,
    4882687u, 55610u, 8476209u, 7340544u, 13742497u, 10441252u, 9787244u, 7367818u,
    12223780u, 6780213u, 10505550u, 3746083u, 8874850u, 5725276u, 8014168u, 4996700u,
    4993571u, 5001770u, 9321518u, 2430480u, 12398641u, 9715553u, 6035741u, 1474182u,
    3837580u, 5647422u, 1356933u, 6579300u, 14200723u, 8365974u,
);
// `HORIZON_RT_DEBUG` 3, 4 or 5 (with tonemap.wgsl's 1 showing the mask): the depth, the facing, the
// ray alone.
override DEBUG: u32 = 0u;

// Ray-traced shadows (gfx/voxels.rs, docs/raytracing.md): whether world block `v` is in the grid.
fn in_grid(v: vec3<i32>) -> bool {
    let g = rt.grid;
    let w = i32(g.z);
    let o = vec2<i32>(i32(g.x), i32(g.y));
    return v.y < 320 && v.x >= o.x && v.x < o.x + w && v.z >= o.y && v.z < o.y + w;
}
// The grid's texel of the brick of 2^`bits` blocks a side holding world block `v` (in the grid).
fn brick_slot(v: vec3<i32>, bits: u32) -> vec3<i32> {
    let w = i32(rt.grid.z) >> bits;
    let b = vec3<i32>(v.x >> bits, (v.y + 64) >> bits, v.z >> bits);
    return vec3<i32>(((b.x % w) + w) % w, b.y, ((b.z % w) + w) % w);
}
// The ray's parameter where it leaves the brick of 2^`bits` blocks holding world block `world`
// (`o` and the result relative to the camera's block); an axis the ray runs along never ends it.
fn brick_exit(world: vec3<i32>, bits: u32, o: vec3<f32>, l: vec3<f32>) -> f32 {
    let size = f32(1u << bits);
    let low = vec3<f32>(((world + vec3(0, 64, 0)) >> vec3(bits)) << vec3(bits))
        - vec3(0.0, 64.0, 0.0) - rt.camera.xyz;
    let far = select(low, low + size, l > vec3(0.0));
    let exit = select((far - o) / l, vec3(1e30), abs(l) < vec3(1e-6));
    return min(min(exit.x, exit.y), exit.z);
}
// Whether block `l` (local to its brick, 0..8) is set in a brick's 512 bits, four texels of `masks`
// from `slot`: word `l.z * 2 + l.y / 4`, bit `(l.y % 4) * 8 + l.x`.
fn mask_bit(masks: texture_3d<u32>, slot: vec3<i32>, l: vec3<i32>) -> bool {
    let k = l.z * 2 + (l.y >> 2u);
    let word = textureLoad(masks, vec3(slot.x * 4 + (k >> 2u), slot.y, slot.z), 0)[k & 3];
    return ((word >> u32((l.y & 3) * 8 + l.x)) & 1u) != 0u;
}
// How much of the sun's (or the moon's) light reaches camera-relative `pos` on a face of normal
// `n`: a ray toward the light through the grid, an empty brick crossed in one step, the others
// block by block (Amanatides and Woo) in their bit masks; a solid block stops it, leaves let half
// through each, leaving the grid (or rising above its highest block) ends it in the light.
// Coordinates are relative to the camera's block, so they stay small far from the world's origin.
fn sun_visibility(pos: vec3<f32>, n: vec3<f32>) -> f32 {
    let facing = dot(n, rt.light.xyz);
    if facing <= 0.0 { return 0.0; }
    return sun_ray(pos) * smoothstep(0.0, 0.15, facing);
}
// The light the ray from `pos` toward the sun lets through (1 all of it, 0 none), whatever the face.
fn sun_ray(pos: vec3<f32>) -> f32 {
    if HW { return dot(hw_light(pos, rt.light.xyz, 4096.0), vec3(0.2126, 0.7152, 0.0722)); }
    return ray(pos, rt.light.xyz, i32(rt.grid.w), 1e30);
}
// RT lighting: the same toward a point of the sun's (or the moon's) disc, 0.53 degrees across
// (times the Sun Size setting: its angular radius in `sun_light.w`), a new one each frame
// (`random`): accumulated, the shadows' penumbra widens with the distance to what casts them, as
// the real sun's does.
fn sun_disc_ray(pos: vec3<f32>, random: vec2<f32>) -> vec3<f32> {
    let l = rt.light.xyz;
    let t = normalize(select(cross(l, vec3(0.0, 1.0, 0.0)), cross(l, vec3(1.0, 0.0, 0.0)),
        abs(l.y) > 0.9));
    let b = cross(l, t);
    let r = sqrt(random.x) * rt.sun_light.w;
    let a = 6.2831853 * random.y;
    let d = normalize(l + (t * cos(a) + b * sin(a)) * r);
    // The hybrid's exact triangles where the GPU traces rays (shadow_rays_hw.wgsl).
    if HW { return hw_light(pos, d, 4096.0); }
    return vec3(ray(pos, d, i32(rt.grid.w), 1e30));
}
// A pixel's and a frame's random numbers (PCG), 0..1.
fn random2(texel: vec2<i32>, salt: u32) -> vec2<f32> {
    var v = u32(texel.x) * 1973u + u32(texel.y) * 9277u + u32(rt.frame.x) * 26699u + salt * 7919u;
    v = v * 747796405u + 2891336453u;
    var w = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
    w = (w >> 22u) ^ w;
    var u = w * 747796405u + 2891336453u;
    u = ((u >> ((u >> 28u) + 4u)) ^ u) * 277803737u;
    u = (u >> 22u) ^ u;
    return vec2(f32(w >> 8u), f32(u >> 8u)) / 16777216.0;
}
// The light a ray from `pos` along `l` (unit) lets through before it leaves the grid, rises above
// its highest block, spends `budget` (1 an empty brick crossed, 4 a full one) or goes `reach`. The
// block it starts in never stops it: a slab's or a path's top is inside a block the grid holds
// whole (until the grid holds blocks' shapes).
fn ray(pos: vec3<f32>, l: vec3<f32>, budget_in: i32, reach: f32) -> f32 {
    return cast_ray(pos, l, budget_in, reach).light;
}
// What a ray hit: the light it let through, and where it stopped (its parameter and the world
// block) when a block stopped it.
struct RayHit {
    light: f32,
    t: f32,
    cell: vec3<i32>,
    hit: bool,
}
fn cast_ray(pos: vec3<f32>, l: vec3<f32>, budget_in: i32, reach: f32) -> RayHit {
    let o = pos + rt.fraction.xyz;
    let camera = vec3<i32>(rt.camera.xyz);
    let start = vec3<i32>(floor(o)) + camera;
    let step = vec3<i32>(sign(l));
    let delta = 1.0 / max(abs(l), vec3(1e-6));
    let top = rt.camera.w - rt.camera.y;
    var light = 1.0;
    var t = 0.0;
    var budget = budget_in;
    while budget > 0 && t < reach {
        let p = o + l * t;
        if l.y > 0.0 && p.y >= top { break; }
        var cell = vec3<i32>(floor(p));
        let world = cell + camera;
        if !in_grid(world) { break; }
        if world.y < -64 { return RayHit(0.0, t, world, true); }
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
            // Where the ray entered the block `local`.
            var entered = t;
            for (var i = 0; i < 25; i++) {
                if any(local < vec3(0)) || any(local > vec3(7)) { break; }
                if entered >= reach { return RayHit(light, entered, vec3(0), false); }
                let block = local + corner;
                let own = all(block == start);
                let solid = !own && mask_bit(solid_masks, slot, local);
                let cut = !own && !solid && leaves && mask_bit(cutout_masks, slot, local);
                if solid || cut {
                    // A block with a shape (slabs, fences, bars, plants, leaves): its cells.
                    let id = textureLoad(albedo_grid, block_slot(block), 0).r >> 6u;
                    if id == 0u {
                        if solid { return RayHit(0.0, entered, block, true); }
                        light *= 0.5;
                    } else {
                        let leaving = min(next.x, min(next.y, next.z));
                        let crossed = through_shape(o, l, block, id, entered, leaving);
                        if crossed.y >= 0.0 { return RayHit(0.0, crossed.y, block, true); }
                        light *= crossed.x;
                    }
                    if light < 0.1 { return RayHit(0.0, entered, block, true); }
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
    // A ray that spent its budget without getting out (a long cave) is taken as stopped: no sky
    // leaks into caves the rays can't see out of.
    if budget <= 0 && t < reach { return RayHit(0.0, t, vec3(0), false); }
    return RayHit(light, t, vec3(0), false);
}
// The camera-relative position pixel `p` of the depth shows.
// `p` is in the depth's pixels.
fn surface_at(p: vec2<i32>) -> vec3<f32> {
    let size = rt.size.xy;
    let d = textureLoad(depth, clamp(p, vec2(0), vec2<i32>(size) - 1), 0);
    let ndc = vec2((f32(p.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (f32(p.y) + 0.5) / size.y * 2.0);
    let c = rt.inverse_view_proj * vec4(ndc, d, 1.0);
    return c.xyz / c.w;
}
// Whether world block `v` is solid in the grid (none outside it, all below the world).
fn solid_at(v: vec3<i32>) -> bool {
    if !in_grid(v) { return false; }
    if v.y < -64 { return true; }
    let slot = brick_slot(v, 3u);
    if (textureLoad(bricks, slot, 0).r & 1u) == 0u { return false; }
    return mask_bit(solid_masks, slot, v - ((v >> vec3(3u)) << vec3(3u)));
}
// Ambient occlusion's rays: six short rays (AO_LENGTH blocks) fanned around the normal at a fixed
// pattern, so it neither flickers nor needs accumulating; each solid block hit closes its share,
// more the nearer. 1 is open. It counts the corners vanilla's smooth lighting shades too: the rays
// are to replace that shading (docs/raytracing.md).
const AO_LENGTH: f32 = 2.0;
const AO_STEPS: i32 = 5;
fn ambient(pos: vec3<f32>, n: vec3<f32>) -> f32 {
    let t = normalize(select(cross(n, vec3(0.0, 1.0, 0.0)), cross(n, vec3(1.0, 0.0, 0.0)),
        abs(n.y) > 0.9));
    let b = cross(n, t);
    let o = pos + rt.fraction.xyz;
    let camera = vec3<i32>(rt.camera.xyz);
    var closed = 0.0;
    for (var i = 0; i < 6; i++) {
        let a = f32(i) * 1.0471976 + 0.5;
        let d = normalize(n * 0.75 + (t * cos(a) + b * sin(a)) * 0.66);
        for (var s = 1; s <= AO_STEPS; s++) {
            let along = f32(s) / f32(AO_STEPS) * AO_LENGTH;
            if solid_at(vec3<i32>(floor(o + d * along)) + camera) {
                closed += 1.0 - (along - AO_LENGTH / f32(AO_STEPS)) / AO_LENGTH;
                break;
            }
        }
    }
    return 1.0 - rt.size.w * closed / 6.0;
}
// RT lighting's sky toward `d` (unit), from the sky table, bilinear (terrain.wgsl `rt_sky`).
fn sky_at(d: vec3<f32>) -> vec3<f32> {
    let flat = d.xz / max(length(d.xz), 1e-5);
    let sun = rt.sun.xz / max(length(rt.sun.xz), 1e-5);
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
fn sky_texel(at: vec2<i32>, top: vec2<i32>) -> vec3<f32> {
    return textureLoad(sky_view, clamp(at, vec2(0), top), 0).rgb;
}
// RT lighting's sky and bounced light: the light of the sky the surface at `pos` (normal `n`) sees
// and of the blocks around it lit (`bounced`), rgb, diffuse, in paper whites (Lambert's 1/pi in the
// units), and how much of the sky it sees (a), from rays over its hemisphere, cosine weighted, in
// directions new each frame (`texel`'s random numbers): their average over frames is the light
// without the bands a fixed pattern leaves. A ray that gets out brings the sky's light, one that
// hits a block that block's light: a closed tunnel sees no sky, and a torch's light reaches round
// a corner off the wall it lights.
const SKY_BUDGET: i32 = 24;
const SKY_RAYS: i32 = 2;
// RT lighting's global illumination: the light leaving the surface a ray from the pixel hit at
// camera-relative `at`, its colour times the light reaching it there: the sun's (or the moon's)
// through a shadow ray and the rest from the radiance cache (any surface the frames before saw:
// the light keeps bouncing, a bounce more a frame, as SHaRC's), the last frame's screen, or else
// a second traced bounce (with hardware rays) or the sky over the face's open half and the
// brightest light source. Remix path traces up to 4 bounces and ends the paths in its caches.
const BOUNCE_BUDGET: i32 = 24;
// A ray's hit: where (its parameter, below 0 none), the face's normal, the surface's colour in
// light, and what the translucent surfaces on the way let through (the hardware rays'
// `hw_hit`; the voxel grid's `cast_ray` gives its block instead).
struct HwHit {
    t: f32,
    normal: vec3<f32>,
    albedo: vec3<f32>,
    through: vec3<f32>,
}
// The map colour of world block `cell`, in light: the voxel grid's colour of a surface (none: -1).
fn map_albedo(cell: vec3<i32>) -> vec3<f32> {
    let id = textureLoad(albedo_grid, block_slot(cell), 0).r & 63u;
    if id == 0u || id >= 62u { return vec3(-1.0); }
    let c = MAP_COLORS[id];
    return pow(vec3(f32((c >> 16u) & 255u), f32((c >> 8u) & 255u), f32(c & 255u)) / 255.0,
        vec3(2.2));
}
fn bounced(at: vec3<f32>, cell: vec3<i32>) -> vec3<f32> {
    let albedo = map_albedo(cell);
    if albedo.x < 0.0 { return vec3(0.0); }
    return bounce_light(at, hit_normal(at, cell), albedo);
}
// The sun's (or the moon's) light reaching a face at camera-relative `lifted` (normal `n`).
fn sun_on(lifted: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let facing = dot(n, rt.light.xyz);
    if facing <= 0.0 || rt.light.w <= 0.0 { return vec3(0.0); }
    var through = 0.0;
    if HW {
        through = luminance(hw_light(lifted, rt.light.xyz, 4096.0));
    } else {
        through = ray(lifted, rt.light.xyz, BOUNCE_BUDGET, 1e30);
    }
    return rt.sun_light.rgb * (rt.light.w * facing * through);
}
// The rest of the light reaching a face at camera-relative `at`: the radiance cache's, else the
// last frame's screen (w below 0: neither).
fn cached_on(at: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    let cached = cache_light(at, n);
    if cached.a >= 0.0 { return cached; }
    return last_light(at);
}
// The light leaving a surface at camera-relative `at` (normal `n`, colour `albedo`) toward where
// the ray came from; a cache miss traces one more bounce with hardware rays.
fn bounce_light(at: vec3<f32>, n: vec3<f32>, albedo: vec3<f32>) -> vec3<f32> {
    let lifted = at + n * 0.02;
    var light = sun_on(lifted, n);
    let last = cached_on(at, n);
    if last.a >= 0.0 {
        light += last.rgb;
    } else if HW {
        light += second_bounce(lifted, n, vec2<i32>(floor(at.xz * 16.0 + at.y * 7.0)));
    } else {
        light += open_light(lifted, n, at);
    }
    return albedo * light;
}
// Without a cache cell or a further ray: the sky over the face's open half and the brightest
// light source.
fn open_light(lifted: vec3<f32>, n: vec3<f32>, at: vec3<f32>) -> vec3<f32> {
    return sky_at(normalize(n + vec3(0.0, 1.0, 0.0))) * (0.25 + 0.25 * n.y)
        + block_light(lifted, n, 1, vec2<i32>(floor(at.xz * 16.0)));
}
// The light reaching camera-relative `pos` (normal `n`) along one more cosine-weighted ray: the
// sky's, or the surface it meets lit by the sun and its cache cell (or the open sky's guess).
fn second_bounce(pos: vec3<f32>, n: vec3<f32>, salt: vec2<i32>) -> vec3<f32> {
    let d = cosine_dir(n, random2(salt, 3u));
    let h = hw_hit(pos, d, 256.0);
    if h.t < 0.0 { return sky_at(d) * h.through; }
    let at = pos + d * h.t;
    let lifted = at + h.normal * 0.02;
    var light = sun_on(lifted, h.normal);
    let last = cached_on(at, h.normal);
    light += select(open_light(lifted, h.normal, at), last.rgb, last.a >= 0.0);
    return h.through * h.albedo * light;
}
// A cosine-weighted direction about `n` from two random numbers.
fn cosine_dir(n: vec3<f32>, u: vec2<f32>) -> vec3<f32> {
    let t = normalize(select(cross(n, vec3(0.0, 1.0, 0.0)), cross(n, vec3(1.0, 0.0, 0.0)),
        abs(n.y) > 0.9));
    let b = cross(n, t);
    let r = sqrt(u.x);
    let a = 6.2831853 * u.y;
    return normalize((t * cos(a) + b * sin(a)) * r + n * sqrt(max(1.0 - u.x, 0.0)));
}
// The diffuse light other than the sun's the last frame's rays found at camera-relative `at`
// (`last_light_mask`), when the last frame showed that surface; a below 0 when it didn't.
fn last_light(at: vec3<f32>) -> vec4<f32> {
    if rt.motion.w < 0.5 { return vec4(0.0, 0.0, 0.0, -1.0); }
    let q = at + rt.motion.xyz;
    let c = rt.prev_view_proj * vec4(q, 1.0);
    if c.w <= 0.0 { return vec4(0.0, 0.0, 0.0, -1.0); }
    let ndc = c.xy / c.w;
    if any(abs(ndc) > vec2(1.0)) { return vec4(0.0, 0.0, 0.0, -1.0); }
    let size = vec2<f32>(textureDimensions(last_mask));
    let texel = clamp(vec2<i32>(vec2((ndc.x * 0.5 + 0.5) * size.x, (0.5 - ndc.y * 0.5) * size.y)),
        vec2(0), vec2<i32>(size) - 1);
    let d = length(q);
    let shown = textureLoad(last_mask, texel, 0).a;
    if shown < 0.0 || abs(shown - d) > 0.1 + 0.03 * d { return vec4(0.0, 0.0, 0.0, -1.0); }
    let light = textureLoad(last_light_mask, texel, 0);
    if light.a < 0.0 { return vec4(0.0, 0.0, 0.0, -1.0); }
    return vec4(light.rgb, 1.0);
}
fn sky_light(pos: vec3<f32>, n: vec3<f32>, texel: vec2<i32>) -> vec4<f32> {
    let t = normalize(select(cross(n, vec3(0.0, 1.0, 0.0)), cross(n, vec3(1.0, 0.0, 0.0)),
        abs(n.y) > 0.9));
    let b = cross(n, t);
    let random = random2(texel, 1u);
    var light = vec3(0.0);
    var seen = 0.0;
    for (var i = 0; i < SKY_RAYS; i++) {
        let u = fract(random + vec2(f32(i) * 0.618034, f32(i) * 0.754878));
        let r = sqrt(u.x);
        let a = 6.2831853 * u.y;
        let d = normalize((t * cos(a) + b * sin(a)) * r + n * sqrt(max(1.0 - u.x, 0.0)));
        let hit = cast_ray(pos, d, SKY_BUDGET, 1e30);
        light += sky_at(d) * hit.light;
        seen += hit.light;
        if hit.hit {
            light += bounced(pos + d * hit.t, hit.cell);
        }
    }
    return vec4(light, seen) / f32(SKY_RAYS);
}
// RT lighting's indirect light, ReSTIR GI (Ouyang et al. 2021), as RTX Remix uses it: each pixel
// traces one cosine-weighted ray a frame, its sample the point it hits and the light leaving it
// toward the pixel (`bounced`: that block's colour times the sun's and the radiance cache's light
// there), or the sky's where it gets out. The sample is resampled with the last frame's reservoir
// where the surface was and with two neighbours' there (their sample weighed by the solid angle's
// change, the Jacobian, and dropped across a jump in distance or a change of face); a sample taken
// from elsewhere is kept only if this pixel sees it (a visibility ray). The pixel's light: the
// chosen sample's, cosine-weighted, times the reservoir's weight.
struct GiSample {
    // The point (camera-relative; far along the ray for the sky) and its normal, the light it
    // sends back toward the pixel.
    at: vec3<f32>,
    normal: vec3<f32>,
    light: vec3<f32>,
}
// The face a ray hit at camera-relative `at` on block `cell`: the side the point is nearest.
fn hit_normal(at: vec3<f32>, cell: vec3<i32>) -> vec3<f32> {
    let off = at + rt.camera.xyz + rt.fraction.xyz - vec3<f32>(cell) - 0.5;
    let a = abs(off);
    if a.x >= a.y && a.x >= a.z { return vec3(sign(off.x), 0.0, 0.0); }
    if a.y >= a.z { return vec3(0.0, sign(off.y), 0.0); }
    return vec3(0.0, 0.0, sign(off.z));
}
// This frame's sample at camera-relative `pos` (normal `n`), and whether it is the sky's (w 1).
fn gi_sample(pos: vec3<f32>, n: vec3<f32>, texel: vec2<i32>) -> GiSample {
    let d = cosine_dir(n, random2(texel, 1u));
    // The hybrid's exact triangles and their textures where the GPU traces rays, as Remix's.
    if HW {
        let h = hw_hit(pos, d, 4096.0);
        if h.t >= 0.0 {
            let at = pos + d * h.t;
            return GiSample(at, h.normal, h.through * bounce_light(at, h.normal, h.albedo));
        }
        return GiSample(pos + d * 4096.0, -d, sky_at(d) * h.through);
    }
    let hit = cast_ray(pos, d, SKY_BUDGET, 1e30);
    if hit.hit {
        let at = pos + d * hit.t;
        return GiSample(at, hit_normal(at, hit.cell), bounced(at, hit.cell));
    }
    // The sky, as a point far along the ray (its light whatever the direction it is seen from).
    return GiSample(pos + d * 4096.0, -d, sky_at(d) * hit.light);
}
// A sample's target at a pixel at `pos` (normal `n`): its light's luminance by the cosine there.
fn gi_target(s: GiSample, pos: vec3<f32>, n: vec3<f32>) -> f32 {
    return luminance(s.light) * max(dot(n, normalize(s.at - pos)), 0.0);
}
// The last frame's reservoir at texel `at`: its sample (moved to this frame's camera), its weight
// and the candidates it saw; `seen` 0 when there is none.
struct GiReservoir {
    s: GiSample,
    weight: f32,
    seen: f32,
}
fn last_gi(at: vec2<i32>) -> GiReservoir {
    let size = vec2<i32>(textureDimensions(last_gi_a));
    let c = clamp(at, vec2(0), size - 1);
    let a = textureLoad(last_gi_a, c, 0);
    let b = textureLoad(last_gi_b, c, 0);
    let rg = unpack2x16float(b.x);
    let bm = unpack2x16float(b.y);
    let normal = unpack4x8snorm(b.z).xyz;
    // Stored relative to the last frame's camera: this frame's camera moved by `motion`.
    let s = GiSample(a.xyz - rt.motion.xyz, normal, vec3(rg, bm.x));
    return GiReservoir(s, a.w, bm.y);
}
const GI_HISTORY: f32 = 20.0;
fn restir_gi(pos: vec3<f32>, n: vec3<f32>, texel: vec2<i32>, last_at: vec2<i32>, has_last: bool)
    -> vec4<f32> {
    let own = gi_sample(pos, n, texel);
    // The new candidate, drawn by the cosine: its weight its aim over that chance (pi cancels
    // the cosine, Lambert's 1/pi is in the light's units).
    var chosen = own;
    var aim = gi_target(own, pos, n);
    var sum = luminance(own.light) * 3.14159265;
    var seen = 1.0;
    var reused = false;
    var u = random2(texel, 70u);
    if has_last {
        for (var j = 0; j < 3; j++) {
            let jitter = random2(texel, 80u + u32(j));
            let off = select(vec2<i32>((jitter * 2.0 - 1.0) * 8.0), vec2(0), j == 0);
            let q = last_gi(last_at + off);
            if q.seen <= 0.0 || q.weight <= 0.0 { continue; }
            // The same surface: the face it showed and its distance (a floor's pixel doesn't take
            // the wall's sample across the corner).
            if j > 0 {
                let size = vec2<i32>(textureDimensions(last_mask));
                let c = clamp(last_at + off, vec2(0), size - 1);
                let depth = length(pos);
                if abs(face_in(textureLoad(last_direct, c, 0).a) - face_of(n)) > 0.5
                    || abs(textureLoad(last_mask, c, 0).a - depth) > 0.05 * depth + 0.1 {
                    continue;
                }
            }
            // A neighbour a few texels away sees the sample from about the same point: the solid
            // angle's change (the Jacobian) is taken as 1 (a gap from Remix's exact one, which
            // needs the neighbour's own point; the visibility ray below keeps it honest).
            let p = gi_target(q.s, pos, n);
            // The neighbours' history counts for less than the pixel's own (no blotches of one
            // sample spread over a patch).
            let m = min(q.seen, select(GI_HISTORY / 4.0, GI_HISTORY, j == 0));
            let w = p * q.weight * m;
            sum += w;
            seen += m;
            u = fract(u + vec2(0.618034, 0.754878));
            if w > 0.0 && u.x * sum < w {
                chosen = q.s;
                aim = p;
                reused = true;
            }
        }
    }
    var weight = 0.0;
    if aim > 0.0 { weight = sum / (seen * aim); }
    // A sample from elsewhere: this pixel has to see it.
    if reused && aim > 0.0 {
        let to = chosen.at - pos;
        let d = length(to);
        let reach = select(d - 0.05, 1e30, d > 4000.0);
        if ray(pos, to / d, SKY_BUDGET, reach) < 0.5 {
            chosen = own;
            aim = gi_target(own, pos, n);
            weight = select(0.0, 3.14159265 * luminance(own.light) / aim, aim > 0.0);
            seen = 1.0;
        }
    }
    // Half floats past 65504 are infinite: the sample's light kept within them.
    chosen.light = min(finite3(chosen.light), vec3(60000.0));
    weight = min(finite(weight), 1e6);
    gi_out_a = finite4(vec4(chosen.at, weight));
    gi_out_b = vec4(pack2x16float(chosen.light.rg), pack2x16float(vec2(chosen.light.b, seen)),
        pack4x8snorm(vec4(chosen.normal, 0.0)), 0u);
    let cosine = max(dot(n, normalize(chosen.at - pos)), 0.0);
    // The light the chosen sample stands for, in the diffuse light's units (Lambert's 1/pi).
    let light = chosen.light * cosine * weight / 3.14159265;
    let sky = select(0.0, 1.0, length(own.at - pos) > 4000.0);
    return vec4(light, sky);
}
// RT lighting's light sources: the light those of the cell holding camera-relative `pos` (on a face
// of normal `n`) give it, each an area: a full block emitting light (glowstone, lava, a lamp) all
// of its volume, a torch's or a lantern's flame a tenth of a block around its middle. A shadow ray
// a frame toward a point of it (`texel`'s random numbers), stopped where it enters the light's
// block, so the shadows' penumbrae widen with the distance to what casts them once accumulated.
// Physical falloff, the inverse square of the distance to the light's block, eased to nothing at
// the light's reach; through stained glass the light takes its colour (hardware rays).
const BLOCK_LIGHT: f32 = 3.0;
const LIGHT_BUDGET: i32 = 16;
// The cell of the grid holding grid-local `local`, its lights (up to eight, two indices a word) and
// how many there are.
fn light_cell(local: vec3<f32>) -> vec4<u32> {
    let cell = vec3<i32>(floor(local / 8.0));
    let count = vec3<i32>(i32(rt.grid.z) / 8, 48, i32(rt.grid.z) / 8);
    if any(cell < vec3(0)) || any(cell >= count) { return vec4(0u); }
    return cells[(cell.y * count.z + cell.z) * count.x + cell.x];
}
fn cell_light(words: vec4<u32>, i: i32) -> u32 {
    return (words[i / 2] >> (u32(i % 2) * 16u)) & 0xffffu;
}
// Light source `index` (+ 1) at grid-local `local` on a face of normal `n`, toward its point
// `offset` (-1..1 of its size: a torch's flame or a full block's volume): its light, unshadowed,
// and the ray's direction and how far it may go (to where it enters the light's block; w below 0
// when the light can't reach).
struct LightPath {
    light: vec3<f32>,
    dir: vec4<f32>,
}
fn light_path(index: u32, local: vec3<f32>, n: vec3<f32>, offset: vec3<f32>) -> LightPath {
    let none = LightPath(vec3(0.0), vec4(0.0, 1.0, 0.0, -1.0));
    if index == 0u { return none; }
    let light = lights[index - 1u];
    let block = floor(light.at.xyz);
    let world = vec3<i32>(block) + vec3<i32>(i32(rt.grid.x), -64, i32(rt.grid.y));
    let cube = solid_at(world);
    let size = select(0.1, 0.5, cube);
    let middle = block + vec3(0.5, select(0.6, 0.5, cube), 0.5);
    let to = middle + offset * size - local;
    let length_to = length(to);
    let l = to / max(length_to, 1e-4);
    // Where the ray enters the light's block (0 inside it).
    let low = (block - local) / select(l, vec3(1e-6), abs(l) < vec3(1e-6));
    let high = (block + 1.0 - local) / select(l, vec3(1e-6), abs(l) < vec3(1e-6));
    let near = min(low, high);
    let d = max(max(max(near.x, near.y), near.z), 0.0);
    let reach = light.at.w;
    if d >= reach { return none; }
    let facing = select(dot(n, l), 1.0, d < 0.05);
    if facing <= 0.0 { return none; }
    let window = 1.0 - (d / reach) * (d / reach);
    // The distance to the point of the light drawn (a torch's flame stands above its block's
    // floor, so the block it stands on isn't lit as if touching it).
    let gain = light.colour.w * window * window * facing / ((1.0 + length_to) * (1.0 + length_to));
    return LightPath(pow(light.colour.rgb, vec3(2.2)) * (BLOCK_LIGHT * gain), vec4(l, max(d - 0.02, 0.0)));
}
// What the terrain lets through along `path` from camera-relative `pos`.
fn light_through(pos: vec3<f32>, path: LightPath) -> vec3<f32> {
    if path.dir.w < 0.0 { return vec3(0.0); }
    if HW { return hw_light(pos, path.dir.xyz, path.dir.w); }
    return vec3(ray(pos, path.dir.xyz, LIGHT_BUDGET, path.dir.w));
}
fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}
// The light sources' light at camera-relative `pos` (normal `n`) from `most` of its cell's lights
// drawn this frame, each through a shadow ray (the bounce rays' cheap estimate; the pixels' own is
// `restir_light`).
fn block_light(pos: vec3<f32>, n: vec3<f32>, most: i32, texel: vec2<i32>) -> vec3<f32> {
    let local = pos + rt.camera.xyz + rt.fraction.xyz - vec3(rt.grid.x, -64.0, rt.grid.y);
    let words = light_cell(local);
    var all = 0;
    for (var k = 0; k < 8; k++) {
        if cell_light(words, k) != 0u { all = k + 1; }
    }
    if all == 0 { return vec3(0.0); }
    let drawn = min(most, all);
    let offset = i32(random2(texel, 30u).x * f32(all));
    let share = f32(all) / f32(drawn);
    var sum = vec3(0.0);
    for (var j = 0; j < drawn; j++) {
        let i = (offset + j) % all;
        let r = vec3(random2(texel, 10u + u32(i)), random2(texel, 20u + u32(i)).x) * 2.0 - 1.0;
        let path = light_path(cell_light(words, i), local, n, r);
        if luminance(path.light) * share < 1e-3 { continue; }
        sum += path.light * share * light_through(pos, path);
    }
    return sum;
}
// RT lighting's light sources at a pixel, ReSTIR DI (Bitterli et al. 2020): resampled importance
// sampling over the cell's lights (each weighed by its unshadowed light, `light_path` toward its
// middle, no ray), merged with the last frame's reservoirs where this surface was and at two
// neighbours there, so a light a neighbour found carries over and thousands of lights cost a
// shadow ray a pixel. The chosen light's light, through one shadow ray toward a point of it drawn
// this frame, times the reservoir's weight. A reservoir keeps the light's block (its index changes
// when the list is made again) so a stale one is recognised and dropped.
struct Reservoir {
    // The light (index + 1, 0 none), the sum of the weights, the candidates seen, the weight.
    light: u32,
    sum: f32,
    seen: f32,
    weight: f32,
}
// A light's block, packed (grid-local x, y, z: 9 bits each).
fn light_key(index: u32) -> u32 {
    if index == 0u { return 0u; }
    let b = vec3<u32>(floor(lights[index - 1u].at.xyz));
    return (b.x & 511u) | ((b.y & 511u) << 9u) | ((b.z & 511u) << 18u) | (1u << 27u);
}
// The last frame's reservoirs at texel `at`: (light, its block, seen, weight).
fn last_reservoir(at: vec2<i32>) -> vec4<u32> {
    let size = vec2<i32>(textureDimensions(last_reservoirs));
    return textureLoad(last_reservoirs, clamp(at, vec2(0), size - 1), 0);
}
// The most a reservoir carries over from the frames before, in candidates (twenty frames' worth).
const RESERVOIR_HISTORY: f32 = 160.0;
fn restir_light(pos: vec3<f32>, n: vec3<f32>, texel: vec2<i32>, last_at: vec2<i32>,
    has_last: bool) -> vec4<f32> {
    let local = pos + rt.camera.xyz + rt.fraction.xyz - vec3(rt.grid.x, -64.0, rt.grid.y);
    let words = light_cell(local);
    var r = Reservoir(0u, 0.0, 0.0, 0.0);
    var chosen = 0.0;
    var u = random2(texel, 40u);
    // The cell's lights, all of them (eight at most): each candidate's weight its light over the
    // chance of drawing it among them.
    var candidates = 0.0;
    for (var k = 0; k < 8; k++) {
        if cell_light(words, k) != 0u { candidates += 1.0; }
    }
    for (var k = 0; k < 8; k++) {
        let index = cell_light(words, k);
        if index == 0u { continue; }
        let p = luminance(light_path(index, local, n, vec3(0.0)).light);
        let w = p * candidates;
        r.sum += w;
        u = fract(u + vec2(0.618034, 0.754878));
        if w > 0.0 && u.x * r.sum < w {
            r.light = index;
            chosen = p;
        }
    }
    r.seen = candidates;
    // The last frame's, where this surface was and at two neighbours: their light weighed here.
    if has_last {
        for (var j = 0; j < 3; j++) {
            let jitter = random2(texel, 50u + u32(j));
            let off = select(vec2<i32>((jitter * 2.0 - 1.0) * 6.0), vec2(0), j == 0);
            let last = last_reservoir(last_at + off);
            if last.x == 0u || light_key(last.x) != last.y { continue; }
            let p = luminance(light_path(last.x, local, n, vec3(0.0)).light);
            let seen = min(bitcast<f32>(last.z), RESERVOIR_HISTORY / 3.0);
            let w = p * bitcast<f32>(last.w) * seen;
            r.sum += w;
            r.seen += seen;
            u = fract(u + vec2(0.618034, 0.754878));
            if w > 0.0 && u.x * r.sum < w {
                r.light = last.x;
                chosen = p;
            }
        }
    }
    if r.light == 0u || chosen <= 0.0 { return vec4(0.0); }
    r.weight = r.sum / (r.seen * chosen);
    // The light chosen, toward a point of it drawn this frame, through one shadow ray.
    let offset = vec3(random2(texel, 60u), random2(texel, 61u).x) * 2.0 - 1.0;
    let path = light_path(r.light, local, n, offset);
    let light = path.light * light_through(pos, path) * r.weight;
    reservoir_out = vec4(r.light, light_key(r.light), bitcast<u32>(r.seen), bitcast<u32>(r.weight));
    return vec4(light, 1.0);
}
// The moving lights' light at camera-relative `pos` (normal `n`): one of them drawn each frame by
// its unshadowed light (RIS over them all), toward a point of a small sphere around it through
// one shadow ray, weighed by the chance of drawing it. A torch's warm colour; its reach its level
// in blocks, as the block lights'.
const MOVING_COLOUR: vec3<f32> = vec3(1.0, 0.62, 0.3);
fn moving_light(pos: vec3<f32>, n: vec3<f32>, texel: vec2<i32>) -> vec3<f32> {
    let count = min(i32(rt.held.x), 8);
    if count == 0 { return vec3(0.0); }
    var sum = 0.0;
    var chosen = -1;
    var chosen_p = 0.0;
    var u = random2(texel, 70u).x;
    for (var k = 0; k < count; k++) {
        let p = moving_gain(rt.moving[k], pos, n, vec3(0.0));
        sum += p;
        u = fract(u + 0.618034);
        if p > 0.0 && u * sum < p {
            chosen = k;
            chosen_p = p;
        }
    }
    if chosen < 0 { return vec3(0.0); }
    let light = rt.moving[chosen];
    let offset = (vec3(random2(texel, 71u), random2(texel, 72u).x) * 2.0 - 1.0) * 0.15;
    let to = light.xyz + offset - pos;
    let d = length(to);
    let l = to / max(d, 1e-4);
    var through = vec3(0.0);
    if HW {
        through = hw_light(pos, l, max(d - 0.1, 0.0));
    } else {
        through = vec3(ray(pos, l, LIGHT_BUDGET, max(d - 0.1, 0.0)));
    }
    let gain = moving_gain(light, pos, n, offset);
    // The chosen light's own light times the sum over the chance of drawing it (p / sum).
    return pow(MOVING_COLOUR, vec3(2.2)) * BLOCK_LIGHT * through * (gain * sum / chosen_p);
}
// A moving light's unshadowed light at `pos` (normal `n`), as the block lights' fall off.
fn moving_gain(light: vec4<f32>, pos: vec3<f32>, n: vec3<f32>, offset: vec3<f32>) -> f32 {
    let to = light.xyz + offset - pos;
    let d = length(to);
    let reach = light.w;
    if d >= reach || reach <= 0.0 { return 0.0; }
    let facing = dot(n, to / max(d, 1e-4));
    if facing <= 0.0 { return 0.0; }
    let window = 1.0 - (d / reach) * (d / reach);
    return (reach / 15.0) * window * window * facing / ((1.0 + d) * (1.0 + d));
}
// What the rays give the surface depth pixel `p` shows.
struct Traced {
    // The sun's light reaching it and the light around (r, g), and with RT lighting the sky seen
    // (b) and its distance (a). The sky: (1, 1), with RT lighting no distance (-1).
    rays: vec4<f32>,
    // RT lighting's sky and bounced light (rgb), accumulated over frames, a below 0 past the grid
    // (`block_mask`).
    blocks: vec4<f32>,
    // RT lighting's light sources' direct light and the face (`direct_mask`).
    direct: vec4<f32>,
    // RT lighting's sun through what tints it (`sun_mask`).
    sun: vec4<f32>,
    // RT lighting's ReSTIR DI reservoir (`reservoirs`).
    reservoir: vec4<u32>,
}
fn trace(p: vec2<i32>, texel: vec2<i32>) -> Traced {
    let lit_mode = rt.mode.x > 0.5;
    let none = Traced(select(vec4(1.0), vec4(1.0, 1.0, 1.0, -1.0), lit_mode), vec4(0.0, 0.0, 0.0, -1.0),
        vec4(0.0, 0.0, 0.0, -1.0), vec4(1.0), vec4(0u));
    if f32(p.x) >= rt.size.x || f32(p.y) >= rt.size.y { return none; }
    // Reverse depth: 0 is the sky.
    if textureLoad(depth, p, 0) <= 0.0 { return none; }
    let pos = surface_at(p);
    // The surface's normal from its neighbours, on each axis the side nearer in depth (an edge's
    // other side belongs to another surface).
    let r = surface_at(p + vec2(1, 0)) - pos;
    let l = pos - surface_at(p - vec2(1, 0));
    let d = surface_at(p + vec2(0, 1)) - pos;
    let u = pos - surface_at(p - vec2(0, 1));
    let dx = select(l, r, dot(r, r) < dot(l, l));
    let dy = select(u, d, dot(d, d) < dot(u, u));
    var n = normalize(cross(dx, dy));
    if dot(n, pos) > 0.0 { n = -n; }
    if DEBUG == 4u { return Traced(vec4(dot(n, rt.light.xyz)), none.blocks, none.direct, none.sun, none.reservoir); }
    // The rays fade out toward the grid's edge; RT lighting stops short of it (the terrain past it
    // lights itself as vanilla does).
    let reach = 0.5 * rt.grid.z;
    let far = max(abs(pos.x), abs(pos.z));
    let edge = 1.0 - smoothstep(0.75 * reach, reach, far);
    if edge <= 0.0 {
        return Traced(select(none.rays, vec4(1.0, 1.0, -1.0, length(pos)), lit_mode), none.blocks, none.direct, none.sun, none.reservoir);
    }
    // Off the surface by more as the depth's precision falls with distance.
    let lifted = pos + n * (0.02 + 0.002 * length(pos));
    var sun = 1.0;
    if rt.mode.y > 0.5 && rt.light.w > 0.0 {
        let v = sun_visibility(lifted, n);
        if DEBUG == 5u { return Traced(vec4(v), none.blocks, none.direct, none.sun, none.reservoir); }
        sun = mix(1.0, v, rt.light.w * edge);
    }
    // The ambient occlusion's six short rays, but for RT lighting: its sky and bounced light's rays
    // are the occlusion itself.
    var open = 1.0;
    if rt.size.w > 0.0 && !lit_mode {
        open = mix(1.0, ambient(lifted, n), edge);
    }
    if lit_mode {
        // The terrain lights itself with these (terrain.wgsl `rt_lit`): the sun's visibility
        // without the faces' facing (the terrain has its own normal), the light around, and the
        // distance to recognise the surface by.
        var sun_through = vec3(1.0);
        if rt.mode.y > 0.5 && rt.light.w > 0.0 {
            sun_through = mix(vec3(1.0), sun_disc_ray(lifted, random2(texel, 2u)), edge);
        }
        let v = max(sun_through.r, max(sun_through.g, sun_through.b));
        // The sky's and the light sources' light; it fades out toward the grid's edge into the
        // levels' (a).
        // The light sources' light (ReSTIR DI) and the sky's and bounced light (ReSTIR GI), over
        // the reservoirs the last frame left where this surface was.
        let back = restir_last(pos);
        let sky = restir_gi(lifted, n, texel, back.xy, back.z == 1);
        let blocks = vec4(sky.rgb, select(edge, -1.0, edge < 0.5));
        let direct = vec4(restir_light(lifted, n, texel, back.xy, back.z == 1).rgb
            + moving_light(lifted, n, texel), face_of(n));
        // What this pixel found, into the radiance cache for the bounce rays of the frames after.
        cache_add(pos, n, sky.rgb + direct.rgb);
        // The sun mask's alpha: the indirect and direct light's luminance squared, its second
        // moment once accumulated, for the denoiser's variance (denoise.wgsl).
        let moment = luminance(sky.rgb + direct.rgb) * luminance(sky.rgb + direct.rgb);
        return Traced(vec4(v, open, sky.a, length(pos)), blocks, direct, vec4(sun_through, moment),
            reservoir_out);
    }
    return Traced(vec4(sun, open, 0.0, 1.0), none.blocks, none.direct, none.sun, none.reservoir);
}
// Where camera-relative `pos` was in the last frame's masks (xy), and z 1 when it showed it there.
fn restir_last(pos: vec3<f32>) -> vec3<i32> {
    if rt.motion.w < 0.5 { return vec3(0); }
    let q = pos + rt.motion.xyz;
    let c = rt.prev_view_proj * vec4(q, 1.0);
    if c.w <= 0.0 { return vec3(0); }
    let ndc = c.xy / c.w;
    if any(abs(ndc) > vec2(1.0)) { return vec3(0); }
    let size = vec2<f32>(textureDimensions(last_mask));
    let at = vec2<i32>(vec2((ndc.x * 0.5 + 0.5) * size.x, (0.5 - ndc.y * 0.5) * size.y));
    let last = textureLoad(last_mask, clamp(at, vec2(0), vec2<i32>(size) - 1), 0);
    let d = length(q);
    if last.a < 0.0 || abs(last.a - d) > 0.05 + 0.02 * d { return vec3(at, 0); }
    return vec3(at, 1);
}
// The frames a pixel's light averages at most (NRD ReLAX's diffuse `maxAccumulatedFrameNum`, which
// Remix runs when DLSS Ray Reconstruction is off): each frame adds 1/(n+1) of its rays, n the
// frames already in the pixel's history, so a surface just seen converges as fast as its frames
// allow and a long-seen one keeps 30 frames' light.
const MAX_HISTORY: f32 = 30.0;
// The fast history's frames (ReLAX's `maxFastAccumulatedFrameNum`) and how many of its standard
// deviations the slow history may stray from its mean (`historyClampingColorBoxSigmaScale`), so
// a light switched on or off shows within 6 frames, not 30.
const FAST_HISTORY: f32 = 6.0;
const CLAMP_SIGMA: f32 = 2.0;
// The face (0..5) and the history length packed in the direct light's alpha (`face + 8 n`, exact in
// a half float), for the denoiser and the reuse's checks.
fn face_in(a: f32) -> f32 {
    return a - 8.0 * floor(a / 8.0);
}
fn history_in(a: f32) -> f32 {
    return floor(a / 8.0);
}
// RT lighting: this frame's rays at mask texel `texel` added to the last frames' at the same
// surface, reprojected bilinearly (ReLAX's temporal accumulation): the four last texels around
// where the surface was, each kept when it showed the same face at about the same distance; a
// surface none of them showed starts over from this frame's.
fn accumulate(texel: vec2<i32>, v: Traced) -> Traced {
    let now = v.blocks.rgb + v.direct.rgb;
    fast_out = vec4(now, luminance(now) * luminance(now));
    if rt.motion.w < 0.5 || v.rays.a < 0.0 || v.blocks.a < 0.0 { return v; }
    let q = surface_at(pixel_of(texel)) + rt.motion.xyz;
    let c = rt.prev_view_proj * vec4(q, 1.0);
    if c.w <= 0.0 { return v; }
    let ndc = c.xy / c.w;
    if any(abs(ndc) > vec2(1.0)) { return v; }
    let size = vec2<f32>(textureDimensions(last_mask));
    let at = vec2((ndc.x * 0.5 + 0.5) * size.x, (0.5 - ndc.y * 0.5) * size.y) - 0.5;
    let base = vec2<i32>(floor(at));
    let f = at - floor(at);
    let d = length(q);
    let face = face_in(v.direct.a);
    var mask = vec3(0.0);
    var blocks = vec3(0.0);
    var direct = vec3(0.0);
    var sun = vec4(0.0);
    var quick = vec4(0.0);
    var history = 0.0;
    var weight = 0.0;
    for (var i = 0; i < 4; i++) {
        let o = vec2(i & 1, i >> 1u);
        let t = base + o;
        if any(t < vec2(0)) || any(t >= vec2<i32>(size)) { continue; }
        let last = textureLoad(last_mask, t, 0);
        if last.a < 0.0 || abs(last.a - d) > 0.05 + 0.02 * d { continue; }
        let b = textureLoad(last_blocks, t, 0);
        let l = textureLoad(last_direct, t, 0);
        if b.a < 0.0 || abs(face_in(l.a) - face) > 0.5 { continue; }
        let w = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1) + 1e-4;
        mask += last.rgb * w;
        blocks += b.rgb * w;
        direct += l.rgb * w;
        sun += textureLoad(last_sun, t, 0) * w;
        quick += textureLoad(last_fast, t, 0) * w;
        history += history_in(l.a) * w;
        weight += w;
    }
    if weight < 1e-3 { return v; }
    let n = min(floor(history / weight + 0.5) + 1.0, MAX_HISTORY);
    let k = 1.0 / (n + 1.0);
    fast_out = mix(quick / weight, fast_out, max(k, 1.0 / FAST_HISTORY));
    // The slow history clamped to the fast one's mean, give or take its noise: what changed
    // within the last frames overrides what the long average still holds.
    var slow_blocks = blocks / weight;
    var slow_direct = direct / weight;
    let mean = luminance(fast_out.rgb);
    let sigma = sqrt(max(fast_out.a - mean * mean, 0.0));
    let held = luminance(slow_blocks + slow_direct);
    let bound = clamp(held, mean - CLAMP_SIGMA * sigma, mean + CLAMP_SIGMA * sigma);
    if held > 1e-5 && bound != held {
        slow_blocks *= bound / held;
        slow_direct *= bound / held;
    }
    return Traced(vec4(mix(mask / weight, v.rays.rgb, k), v.rays.a),
        vec4(mix(slow_blocks, v.blocks.rgb, k), v.blocks.a),
        vec4(mix(slow_direct, v.direct.rgb, k), face + 8.0 * n),
        mix(sun / weight, v.sun, k),
        v.reservoir);
}
// The depth pixel at the centre of mask texel `texel`.
fn pixel_of(texel: vec2<i32>) -> vec2<i32> {
    let scale = i32(rt.size.z);
    return texel * scale + scale / 2;
}
// Whether mask texel `texel` is in the sharp zone, from its view ray.
fn sharp(texel: vec2<i32>) -> bool {
    if rt.fovea.w < -1.0 { return true; }
    let p = pixel_of(texel);
    let ndc = vec2((f32(p.x) + 0.5) / rt.size.x * 2.0 - 1.0, 1.0 - (f32(p.y) + 0.5) / rt.size.y * 2.0);
    let c = rt.inverse_view_proj * vec4(ndc, 1.0, 1.0);
    return dot(normalize(c.xyz / c.w), rt.fovea.xyz) >= rt.fovea.w;
}
// Away from the sharp zone a 4x4 block of mask texels shares one ray, traced by its first thread.
var<workgroup> shared_rays: array<Traced, 4>;
@compute @workgroup_size(8, 8) fn cs_shadow(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    let texel = vec2<i32>(id.xy);
    let corner = (texel / 4) * 4;
    let block = (local.y / 4u) * 2u + local.x / 4u;
    let fine = sharp(corner + 2);
    var v = Traced(vec4(1.0), vec4(0.0, 0.0, 0.0, -1.0), vec4(0.0, 0.0, 0.0, -1.0), vec4(1.0),
        vec4(0u));
    if fine {
        v = trace(pixel_of(texel), texel);
    } else if all(local.xy % vec2(4u) == vec2(0u)) {
        shared_rays[block] = trace(pixel_of(corner + 2), corner);
    }
    workgroupBarrier();
    if !fine { v = shared_rays[block]; }
    if rt.mode.x > 0.5 { v = accumulate(texel, v); }
    if DEBUG == 3u { v.rays = vec4(textureLoad(depth, pixel_of(texel), 0) * 20.0 + 0.25); }
    if all(vec2<u32>(texel) < textureDimensions(mask)) {
        v.rays = finite4(v.rays);
        v.blocks = finite4(v.blocks);
        v.direct = finite4(v.direct);
        v.sun = finite4(v.sun);
        fast_out = finite4(fast_out);
        textureStore(mask, texel, v.rays);
        if rt.mode.x > 0.5 {
            textureStore(block_mask, texel, v.blocks);
            textureStore(direct_mask, texel, v.direct);
            textureStore(reservoirs, texel, v.reservoir);
            textureStore(gi_a, texel, gi_out_a);
            textureStore(gi_b, texel, gi_out_b);
            textureStore(sun_mask, texel, v.sun);
            textureStore(fast, texel, fast_out);
            textureStore(reflection, texel, finite4(hw_reflect(pixel_of(texel))));
            textureStore(refraction, texel, finite4(hw_refract(pixel_of(texel))));
        }
    }
}
