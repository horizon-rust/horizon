// Auto-exposure (gfx/exposure.rs), as RTX Remix's: the scene's mean log luminance, centre-weighted,
// adapted over time into the multiplier the grading pass applies. One workgroup reads a small
// mip of the bloom chain (the scene's light, 16 pixels to a texel), so the mean is already
// filtered and nothing flickers with a single bright pixel.
@group(0) @binding(0) var levels: texture_2d<f32>;
// gfx/tonemap.rs `Grading::params`, written every frame.
struct Params {
    // Seconds since the exposure was last adapted.
    dt: f32,
    // The exposure's limits, in stops (Remix: -2.5 and +1.5; a situation lowers the top).
    ev_max: f32,
    ev_min: f32,
    pad: f32,
};
@group(0) @binding(1) var<uniform> params: Params;
// x: the multiplier, y: the adapted mean log2 luminance, z: 1 once the first frame has set y,
// w: the exposure the scene is adapting to, in stops.
@group(0) @binding(2) var<storage, read_write> state: vec4<f32>;

// The mean log2 luminance that maps to an exposure of 1: what a noon scene averages (Remix aims
// at a mid grey in its own units; ours is paper white's, so the key is measured, not copied).
override KEY: f32 = -2.6;
// Remix's adaptation speed, 10: the exposure closes 1 - exp(-10 dt) of its gap each frame, about
// 0.1 s to settle, up or down.
override SPEED: f32 = 10.0;
// Remix's centre metering size: how much more the middle of the view counts than its edge.
override CENTRE: f32 = 0.75;

const WORKGROUP: u32 = 256u;
var<workgroup> sums: array<vec2<f32>, WORKGROUP>;

@compute @workgroup_size(WORKGROUP) fn cs_exposure(@builtin(local_invocation_index) i: u32) {
    let size = textureDimensions(levels);
    let dims = vec2<f32>(size);
    var acc = vec2(0.0);
    for (var t = i; t < size.x * size.y; t += WORKGROUP) {
        let at = vec2(t % size.x, t / size.x);
        let c = textureLoad(levels, vec2<i32>(at), 0).rgb;
        let luma = dot(c, vec3(0.2126, 0.7152, 0.0722));
        // Light and dark beyond what a display tells apart say nothing about the exposure.
        let log_luma = log2(clamp(luma, 0.0002, 16.0));
        let d = (vec2<f32>(at) + 0.5) / dims - 0.5;
        let weight = (1.0 - CENTRE) + CENTRE * exp(-dot(d, d) / (2.0 * 0.25 * 0.25));
        acc += vec2(log_luma * weight, weight);
    }
    sums[i] = acc;
    workgroupBarrier();
    for (var stride = WORKGROUP / 2u; stride > 0u; stride >>= 1u) {
        if i < stride { sums[i] += sums[i + stride]; }
        workgroupBarrier();
    }
    if i != 0u { return; }
    let mean = sums[0].x / max(sums[0].y, 1e-6);
    var adapted = mean;
    if state.z > 0.5 {
        adapted = state.y + (mean - state.y) * (1.0 - exp(-params.dt * SPEED));
    }
    let ev = clamp(KEY - adapted, params.ev_min, params.ev_max);
    state = vec4(exp2(ev), adapted, 1.0, ev);
}
