// RT lighting's denoiser (gfx/shadow_rays.rs, docs/raytracing.md), after NRD's ReLAX (the
// denoiser Remix runs without DLSS Ray Reconstruction): the light the rays found, the sky's, the
// bounced and the light sources' direct (a few rays a pixel accumulated over up to 30 frames, still
// grainy) smoothed over the surface a pixel shows, an edge-stopping a-trous wavelet filter
// (Dammertz et al.): 3x3 taps spread `step` pixels apart, a pass each for steps 1, 2, 4, 8 and 16
// (as far as four passes of 5x5 reach, for under half the taps), weighted down across a jump in
// distance or a change of face, so light doesn't leak from one surface onto the next, and by the
// difference in light against the pixel's own noise (SVGF, Schied et al. 2017): the variance of the
// accumulated light from its first and second moments, so grainy light smooths out and real changes
// (a contact shadow, the edge of a lit patch) stay.
// Where the history is short (a surface just uncovered, its moments not yet meaningful) the
// variance is taken as large, so the filter blurs wider there (ReLAX's history fix). On the first
// pass a pixel far brighter than every neighbour on its face is brought down to them (ReLAX's
// anti-firefly): one lucky ray toward a bright light would otherwise bloom into a blob.
struct Params {
    // x: the taps' spacing in pixels, y: 1 on the last pass, z: 1 to filter VR's periphery by its
    // blocks (`span`).
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
// The rays' uniform (shadow_rays.wgsl `Rt`) as far as its fovea, for where a block shares a ray.
struct Rt {
    inverse_view_proj: mat4x4<f32>,
    light: vec4<f32>,
    camera: vec4<f32>,
    fraction: vec4<f32>,
    grid: vec4<f32>,
    // x, y: the depth's size, z: its pixels a mask texel stands for on each side.
    size: vec4<f32>,
    // xyz: where the eyes look, w: the cosine of the sharp zone's half angle (below -1: no
    // foveation).
    fovea: vec4<f32>,
    // z: 2 for VR's second eye taking the first eye's denoised light (`taken`).
    mode: vec4<f32>,
}
@group(0) @binding(7) var<uniform> rt: Rt;
// VR's second eye (`rt.mode.z` 2): where the rays took the first eye's light (shadow_rays.wgsl
// `taken_out`), the first eye's texel (x + 1 in the low 16 bits, y in the high ones), 0 where this
// eye traced its own; and the first eye's light as its denoiser left it this frame. A surface's
// diffuse light is the same from either eye: the first pass copies the first eye's there, the
// second into the other texture the passes write, and neither is filtered again (the passes after
// leave it be in both), while the texels this eye traced itself filter as before, the copied light
// around them among their taps.
@group(0) @binding(8) var taken: texture_2d<u32>;
@group(0) @binding(9) var first_light: texture_2d<f32>;
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
// How many times its brightest neighbour's light a pixel may hold before it counts as a firefly.
const FIREFLY: f32 = 4.0;
// The light a tap filters: on the first pass the accumulated indirect light plus the direct.
fn signal(c: vec4<f32>, q: vec2<i32>, first: bool) -> vec3<f32> {
    if first { return usable(c.rgb + max(textureLoad(direct, q, 0).rgb, vec3(0.0))); }
    return usable(c.rgb);
}
// A value with its NaNs and infinities as 0 (by their bits, as shadow_rays.wgsl `finite`).
fn finite(x: f32) -> f32 {
    return select(x, 0.0, (bitcast<u32>(x) & 0x7f800000u) == 0x7f800000u);
}
// Light as the passes hold it: finite, not below 0 and within half floats (a sum past 65504 would
// be stored as infinity, and a tap of infinity weighted 0 is a NaN; light below 0 would darken
// the terrain past its sun): one bad texel would spread through every pass after, and across
// VR's eyes where the second takes the first's light (`taken`).
fn usable(c: vec3<f32>) -> vec3<f32> {
    return clamp(vec3(finite(c.r), finite(c.g), finite(c.b)), vec3(0.0), vec3(65000.0));
}

// The filter's kernel, the linear B-spline's weights.
const KERNEL: array<f32, 3> = array<f32, 3>(0.25, 0.5, 0.25);

// Whether mask texel `p`'s 4x4 block is in VR's sharp zone, as the rays decided it
// (shadow_rays.wgsl `cs_shadow`: `sharp` at the block's centre texel).
fn sharp_block(p: vec2<i32>) -> bool {
    if rt.fovea.w < -1.0 { return true; }
    let scale = i32(rt.size.z);
    let pixel = ((p / 4) * 4 + 2) * scale + scale / 2;
    let ndc = vec2((f32(pixel.x) + 0.5) / rt.size.x * 2.0 - 1.0,
        1.0 - (f32(pixel.y) + 0.5) / rt.size.y * 2.0);
    let c = rt.inverse_view_proj * vec4(ndc, 1.0, 1.0);
    return dot(normalize(c.xyz / c.w), rt.fovea.xyz) >= rt.fovea.w;
}

// `HORIZON_RT_DEBUG=12`: VR's second eye's light, where it takes the first eye's denoised light
// (`rt.mode.z` 2), as what it took. Taken from the first eye: green lit, cyan black though the
// accumulated light there isn't, blue both black. Its own: red lit, yellow black from the
// filter though the accumulated light isn't, magenta both black.
override DEBUG: u32 = 0u;
fn debug_share(p: vec2<i32>, taken_at: u32) -> vec4<f32> {
    let here = textureLoad(input, p, 0);
    let fed = luminance(usable(textureLoad(accumulated, p, 0).rgb
        + max(textureLoad(direct, p, 0).rgb, vec3(0.0)))) > 0.01;
    var lit = luminance(usable(here.rgb)) > 0.01;
    var colour = vec3(4.0, select(0.0, 4.0, fed), select(4.0, 0.0, fed));
    if lit { colour = vec3(4.0, 0.0, 0.0); }
    if taken_at != 0u {
        let t = vec2<i32>(i32(taken_at & 0xffffu) - 1, i32(taken_at >> 16u));
        lit = luminance(usable(textureLoad(first_light, t, 0).rgb)) > 0.01;
        colour = select(vec3(0.0, select(0.0, 4.0, fed), 4.0), vec3(0.0, 4.0, 0.0), lit);
    }
    return vec4(colour, max(here.a, 1.0));
}

// `HORIZON_RT_SHARE_GUARD=1`, a trial: VR's second eye takes the first eye's light only where it
// isn't far darker (under a quarter) than the light accumulated at the texel, the first eye's
// own before its filter; elsewhere it filters the texel as its own. The same in every pass (from
// what no pass writes), so a texel is copied or filtered throughout.
override SHARE_GUARD: bool = false;
fn takes(p: vec2<i32>, taken_at: u32) -> bool {
    if taken_at == 0u { return false; }
    if !SHARE_GUARD { return true; }
    let t = vec2<i32>(i32(taken_at & 0xffffu) - 1, i32(taken_at >> 16u));
    let copied = luminance(usable(textureLoad(first_light, t, 0).rgb));
    let own = luminance(usable(textureLoad(accumulated, p, 0).rgb
        + max(textureLoad(direct, p, 0).rgb, vec3(0.0))));
    return copied >= 0.25 * own;
}

@compute @workgroup_size(8, 8) fn cs_denoise(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(input));
    let p = vec2<i32>(id.xy);
    if any(p >= size) { return; }
    if rt.mode.z > 1.5 {
        let taken_at = textureLoad(taken, p, 0).r;
        if DEBUG == 12u && params.step.y == 1 {
            textureStore(output, p, debug_share(p, taken_at));
            return;
        }
        if takes(p, taken_at) {
            if params.step.x == 1 {
                let t = vec2<i32>(i32(taken_at & 0xffffu) - 1, i32(taken_at >> 16u));
                let l = textureLoad(first_light, t, 0);
                textureStore(output, p, vec4(usable(l.rgb), finite(l.a)));
            } else if params.step.x == 2 {
                textureStore(output, p, textureLoad(input, p, 0));
            }
            return;
        }
    }
    let centre = textureLoad(input, p, 0);
    let first = params.step.x == 1;
    // Away from VR's sharp zone a 4x4 block of texels shares one ray (shadow_rays.wgsl
    // `cs_sparse`): the passes spaced within a block (1 and 2) would blur copies of it. There the
    // filter starts at a block's spacing, its passes of 4, 8 and 16 being the sharp zone's of 1,
    // 2 and 4 counted in blocks (the anti-firefly against the neighbouring blocks, the variance
    // scaled as theirs); the passes before it pass the light on, the direct added on the first.
    // `HORIZON_RT_DENOISE_FOVEA=0` filters the periphery as the sharp zone.
    let span = select(1, 4, params.step.z == 1 && !sharp_block(p));
    if params.step.x < span {
        textureStore(output, p, vec4(signal(centre, p, first), finite(centre.a)));
        return;
    }
    let d0 = textureLoad(mask, p, 0).a;
    let here = textureLoad(direct, p, 0);
    let face = face_in(here.a);
    var light = signal(centre, p, first);
    if params.step.x == span && d0 >= 0.0 && centre.a >= 0.0 {
        var brightest = 0.0;
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                if x == 0 && y == 0 { continue; }
                let q = clamp(p + vec2(x, y) * span, vec2(0), size - 1);
                let c = textureLoad(input, q, 0);
                if c.a < 0.0 || textureLoad(mask, q, 0).a < 0.0 { continue; }
                if abs(face_in(textureLoad(direct, q, 0).a) - face_in(here.a)) > 0.5 { continue; }
                brightest = max(brightest, luminance(signal(c, q, first)));
            }
        }
        let l = luminance(light);
        let cap = FIREFLY * brightest + 1e-3;
        if brightest > 0.0 && l > cap { light *= cap / l; }
    }
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
        let spread = LIGHT_SIGMA * sqrt(variance) / f32(step / span) + 0.02 * mean + 1e-3;
        let l0 = luminance(light);
        var sum = vec3(0.0);
        var weight = 0.0;
        for (var y = 0; y < 3; y++) {
            for (var x = 0; x < 3; x++) {
                let q = clamp(p + vec2(x - 1, y - 1) * step, vec2(0), size - 1);
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
    textureStore(output, p, vec4(usable(light), finite(centre.a)));
}
