// The rays' far field, worked out over the terrain's heights (gfx/far_heights.rs) once for every
// view: the screen and each VR eye read it (shadow_rays.wgsl `far_sun`, `far_sky`) instead of
// marching the heights pixel by pixel. A texel stands for 2x2 blocks and its slot is its world
// texel modulo the side, as in the heights; the heights' mip levels are their maxima.
struct Params {
    // xy: the window's first texel (world texel coordinates), z: the highest top (a world Y).
    window: vec4<f32>,
    // xyz: toward the sun or the moon (unit).
    light: vec4<f32>,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var heights: texture_2d<i32>;
// r: the height under which the texel's column is in the shadow of the terrain toward the light
// (a world Y), g: how far the shadow's caster is (blocks, horizontally).
@group(0) @binding(2) var light_out: texture_storage_2d<rg32float, write>;
// How much of the sky each of eight directions leaves the texel's top (`SKY_AZIMUTHS`), four a
// texture.
@group(0) @binding(3) var sky_out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(4) var sky_out2: texture_storage_2d<rgba8unorm, write>;

const SIZE: i32 = 1024;
const LEVELS: i32 = 11;
// Texels from the texel's centre where a march starts: past the texel itself, whose top is its
// highest block's (a sunlit wall's own column would shadow it).
const START: f32 = 1.5;
const STEPS: i32 = 160;

// Slot `slot`'s world texel, and where it lies in the window.
fn texel_of(slot: vec2<i32>) -> vec2<i32> {
    let origin = vec2<i32>(params.window.xy);
    return origin + ((slot - origin) & vec2(SIZE - 1));
}
// The top of level `level`'s texel holding world texel `world`.
fn top(world: vec2<i32>, level: i32) -> f32 {
    let side = SIZE >> u32(level);
    return f32(textureLoad(heights, (world >> vec2(u32(level))) & vec2(side - 1), level).r);
}

// The terrain's shadow toward the light over the texel: the highest of the tops along the light's
// way, each lowered by the light's rise to it (max over d of h(d) - k d, k the rise a texel); a
// point of the column above it sees the light. A hierarchical march over the maxima (Tevs et al.
// 2008): a coarse texel whose top, lowered at its near edge, can't beat the highest yet is crossed
// in one step.
@compute @workgroup_size(8, 8) fn cs_light(@builtin(global_invocation_id) id: vec3<u32>) {
    let slot = vec2<i32>(id.xy);
    if any(slot >= vec2(SIZE)) { return; }
    let l = params.light.xyz;
    let flat = length(l.xz);
    if l.y <= 0.0 || flat < 1e-4 {
        textureStore(light_out, slot, vec4(-128.0, 1.0, 0.0, 0.0));
        return;
    }
    let dir = l.xz / flat;
    let along = select(dir, vec2(1e-6), abs(dir) < vec2(1e-6));
    // Two blocks a texel.
    let rise = min(2.0 * l.y / flat, 1e4);
    let origin = vec2<i32>(params.window.xy);
    let world = texel_of(slot);
    let start = vec2<f32>(world - origin) + 0.5;
    var best = -128.0;
    var at = 1.0;
    var d = START;
    var level = 0;
    for (var i = 0; i < STEPS; i++) {
        let p = start + dir * d;
        if any(p < vec2(0.0)) || any(p >= vec2(f32(SIZE))) { break; }
        // Nothing further can stand higher.
        if params.window.z - rise * d <= best { break; }
        let cell = (origin + vec2<i32>(floor(p))) >> vec2(u32(level));
        let low = vec2<f32>((cell << vec2(u32(level))) - origin);
        let side = select(low, low + f32(1 << u32(level)), along > vec2(0.0));
        let leave = (side - p) / along;
        let next = d + max(min(leave.x, leave.y), 0.0) + 1e-3;
        let bound = top(cell << vec2(u32(level)), level) - rise * d;
        if bound <= best {
            d = next;
            level = min(level + 1, LEVELS - 1);
            continue;
        }
        if level > 0 {
            level -= 1;
            continue;
        }
        best = bound;
        at = d;
        d = next;
    }
    textureStore(light_out, slot, vec4(best, 2.0 * at, 0.0, 0.0));
}

// The sky over the texel's top: toward each of eight directions the horizon's steepest rise, over
// distances growing by half a step from 2 texels to 173 (4 to 345 blocks), each read on the level
// about a quarter of the distance wide (horizon-based occlusion, as HBAO and GTAO find it in a
// depth buffer); a direction whose horizon stands at elevation e leaves cos^2 e of its sky.
const SKY_AZIMUTHS: i32 = 8;
const SKY_SAMPLES: i32 = 12;
@compute @workgroup_size(8, 8) fn cs_sky(@builtin(global_invocation_id) id: vec3<u32>) {
    let slot = vec2<i32>(id.xy);
    if any(slot >= vec2(SIZE)) { return; }
    let origin = vec2<i32>(params.window.xy);
    let world = texel_of(slot);
    let start = vec2<f32>(world - origin) + 0.5;
    let y = top(world, 0);
    var open: array<f32, 8>;
    for (var a = 0; a < SKY_AZIMUTHS; a++) {
        let angle = (f32(a) + 0.5) * 6.2831853 / f32(SKY_AZIMUTHS);
        let dir = vec2(cos(angle), sin(angle));
        var steepest = 0.0;
        var d = 2.0;
        for (var s = 0; s < SKY_SAMPLES; s++) {
            if y + steepest * 2.0 * d > params.window.z { break; }
            let p = start + dir * d;
            if any(p < vec2(0.0)) || any(p >= vec2(f32(SIZE))) { break; }
            let level = clamp(i32(floor(log2(d))) - 2, 0, LEVELS - 1);
            steepest = max(steepest, (top(origin + vec2<i32>(floor(p)), level) - y) / (2.0 * d));
            d *= 1.5;
        }
        open[a] = 1.0 / (1.0 + steepest * steepest);
    }
    textureStore(sky_out, slot, vec4(open[0], open[1], open[2], open[3]));
    textureStore(sky_out2, slot, vec4(open[4], open[5], open[6], open[7]));
}
