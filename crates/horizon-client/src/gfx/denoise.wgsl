// RT lighting's denoiser (gfx/shadow_rays.rs, docs/raytracing.md), after NRD's ReLAX (the
// denoiser Remix runs without DLSS Ray Reconstruction): the light the rays found, the sky's, the
// bounced and the light sources' direct (a few rays a pixel accumulated over up to 30 frames, still
// grainy) smoothed over the surface a pixel shows, an edge-stopping a-trous wavelet filter
// (Dammertz et al.): 5x5 taps spread `step` pixels apart, a pass each for steps 1, 2, 4 and 8,
// weighted down across a jump in distance or a change of face, so light doesn't leak from one
// surface onto the next, and by the difference in light against the pixel's own noise (SVGF,
// Schied et al. 2017): the variance of the accumulated light from its first and second moments, so
// grainy light smooths out and real changes (a contact shadow, the edge of a lit patch) stay.
// Where the history is short (a surface just uncovered, its moments not yet meaningful) the
// variance is taken as large, so the filter blurs wider there (ReLAX's history fix).
struct Params {
    // x: the taps' spacing in pixels, y: 1 on the last pass.
    step: vec4<i32>,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var input: texture_2d<f32>;
// The rays' mask: a the surface's distance (below 0 the sky).
@group(0) @binding(2) var mask: texture_2d<f32>;
// The light sources' direct light (rgb), a the face the pixel shows (0..5, its normal's axis and
// sign) plus 8 times the frames in its history (shadow_rays.wgsl `accumulate`).
@group(0) @binding(3) var direct: texture_2d<f32>;
@group(0) @binding(4) var output: texture_storage_2d<rgba16float, write>;
// The accumulated light before any pass, and its second moment (the sun mask's alpha).
@group(0) @binding(5) var accumulated: texture_2d<f32>;
@group(0) @binding(6) var moments: texture_2d<f32>;
// How many standard deviations of the pixel's noise a neighbour's light may differ by.
const LIGHT_SIGMA: f32 = 6.0;
fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}

// The face and the history length from the direct light's alpha (shadow_rays.wgsl `face_in`).
fn face_in(a: f32) -> f32 {
    return a - 8.0 * floor(a / 8.0);
}
// Below this many frames of history the moments say little: the variance is raised instead.
const SHORT_HISTORY: f32 = 4.0;
// The light a tap filters: on the first pass the accumulated indirect light plus the direct.
fn signal(c: vec4<f32>, q: vec2<i32>, first: bool) -> vec3<f32> {
    if first { return c.rgb + max(textureLoad(direct, q, 0).rgb, vec3(0.0)); }
    return c.rgb;
}

// The B3 spline's weights, the filter's kernel.
const KERNEL: array<f32, 5> = array<f32, 5>(0.0625, 0.25, 0.375, 0.25, 0.0625);

@compute @workgroup_size(8, 8) fn cs_denoise(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(input));
    let p = vec2<i32>(id.xy);
    if any(p >= size) { return; }
    let centre = textureLoad(input, p, 0);
    let d0 = textureLoad(mask, p, 0).a;
    let here = textureLoad(direct, p, 0);
    let first = params.step.x == 1;
    let face = face_in(here.a);
    var light = signal(centre, p, first);
    if d0 >= 0.0 && centre.a >= 0.0 {
        let step = params.step.x;
        let tolerance = 0.03 * d0 + 0.05;
        let mean = luminance(textureLoad(accumulated, p, 0).rgb + max(here.rgb, vec3(0.0)));
        var variance = max(textureLoad(moments, p, 0).a - mean * mean, 0.0);
        let history = floor(here.a / 8.0);
        if history < SHORT_HISTORY {
            // As if each missing frame left the light's own size of noise.
            variance = max(variance, mean * mean * (SHORT_HISTORY - history) / SHORT_HISTORY);
        }
        // Coarser passes see less noise (each already averaged some), as SVGF's filtered variance.
        let spread = LIGHT_SIGMA * sqrt(variance) / f32(step) + 0.02 * mean + 1e-3;
        let l0 = luminance(light);
        var sum = vec3(0.0);
        var weight = 0.0;
        for (var y = 0; y < 5; y++) {
            for (var x = 0; x < 5; x++) {
                let q = clamp(p + vec2(x - 2, y - 2) * step, vec2(0), size - 1);
                let c = textureLoad(input, q, 0);
                let d = textureLoad(mask, q, 0).a;
                if c.a < 0.0 || d < 0.0 { continue; }
                if abs(face_in(textureLoad(direct, q, 0).a) - face) > 0.5 { continue; }
                let s = signal(c, q, first);
                let w = KERNEL[x] * KERNEL[y] * exp(-abs(d - d0) / tolerance)
                    * exp(-abs(luminance(s) - l0) / spread);
                sum += s * w;
                weight += w;
            }
        }
        if weight > 1e-5 { light = sum / weight; }
    }
    textureStore(output, p, vec4(light, centre.a));
}
