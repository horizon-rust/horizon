// Linear Light (gfx/tonemap.rs): the scene, drawn in linear light into a float target, to the
// screen's gamma-space 8-bit colour; and the way back for a shader pack's 8-bit output.
@group(0) @binding(0) var source: texture_2d<f32>;
// Ray-traced light (gfx/shadow_rays.rs): r how much sun reaches each pixel of the world, g how
// much of the light around (ambient occlusion), at the world's size; 1x1 without them.
@group(0) @binding(1) var mask: texture_2d<f32>;

// The scene's light with ray-traced light: on solid terrain its alpha is half the share of the light
// it keeps in the shadow (terrain.wgsl `sun_share`), 1 elsewhere. That share, the light around, is
// scaled by the ambient occlusion, the rest, the sun's, by the shadow.
// `HORIZON_RT_DEBUG`: 1 shows the sun's mask, 2 the share, 6 the ambient occlusion.
override DEBUG: u32 = 0u;
fn shadowed(c: vec4<f32>, p: vec2<f32>) -> vec3<f32> {
    let m = textureDimensions(mask);
    if m.x <= 1u || c.a > 0.75 { return c.rgb; }
    let at = vec2<i32>(p * vec2<f32>(m) / vec2<f32>(textureDimensions(source)));
    let light = textureLoad(mask, at, 0).rg;
    let share = 2.0 * c.a;
    if DEBUG == 1u { return vec3(light.r); }
    if DEBUG == 2u { return vec3(share); }
    if DEBUG == 6u { return vec3(light.g); }
    return c.rgb * (share * light.g + (1.0 - share) * light.r);
}

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2(f32(i32(i & 1u) * 4 - 1), f32(i32(i >> 1u) * 4 - 1));
    return vec4(p, 0.0, 1.0);
}

// Below the knee the curve is the identity, so a scene with nothing brighter than 90 % of paper
// white comes out as vanilla draws it; above, a shoulder rolls the light off toward white.
const KNEE: f32 = 0.9;
fn shoulder(x: f32) -> f32 {
    if x <= KNEE { return x; }
    return KNEE + (1.0 - KNEE) * (1.0 - exp(-(x - KNEE) / (1.0 - KNEE)));
}
// Half the shoulder on the brightest channel with the others scaled with it (bright lava stays
// orange), half on each channel (the brightest light goes to white, as a camera's does: the sun
// and the glowstone's core).
fn tone_sdr(rgb: vec3<f32>) -> vec3<f32> {
    let c = max(rgb, vec3(0.0));
    let m = max(c.r, max(c.g, c.b));
    if m <= KNEE { return c; }
    let hue = c * (shoulder(m) / m);
    let white = vec3(shoulder(c.r), shoulder(c.g), shoulder(c.b));
    return mix(hue, white, 0.5);
}

// The alpha passes through: HDR output (hdr.wgsl) reads its emission mark.
@fragment fn fs_tonemap(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(source, vec2<i32>(p.xy), 0);
    return vec4(pow(tone_sdr(shadowed(c, p.xy)), vec3(1.0 / 2.2)), c.a);
}

// The curve of RT lighting's grading: ACES, Stephen Hill's fit of the RRT and ODT (RTX Remix's
// non-legacy ACES), applied to the graded light (`fs_grade`) in place of the shoulder above.
// `ACES_WHITE` is the input that paper white gets: the exposure brings a scene's mean to about
// 0.17 (exposure.wgsl `KEY`), which the fit maps to a middle grey at 1, and `fs_grade` scales its
// exposure so that Remix's 0.214 is that scale.
const ACES_IN = mat3x3<f32>(
    vec3(0.59719, 0.07600, 0.02840),
    vec3(0.35458, 0.90834, 0.13383),
    vec3(0.04823, 0.01566, 0.83777),
);
const ACES_OUT = mat3x3<f32>(
    vec3(1.60475, -0.10208, -0.00327),
    vec3(-0.53108, 1.10813, -0.07276),
    vec3(-0.07367, -0.00605, 1.07602),
);
const ACES_WHITE: f32 = 1.0;
fn aces(light: vec3<f32>) -> vec3<f32> {
    let v = ACES_IN * (max(light, vec3(0.0)) * ACES_WHITE);
    let fit = (v * (v + 0.0245786) - 0.000090537) / (v * (0.983729 * v + 0.4329510) + 0.238081);
    return clamp(ACES_OUT * fit, vec3(0.0), vec3(1.0));
}
@fragment fn fs_tonemap_aces(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(source, vec2<i32>(p.xy), 0);
    return vec4(pow(aces(c.rgb), vec3(1.0 / 2.2)), 1.0);
}

// RT lighting's grading (gfx/tonemap.rs `Post`), before the curve: the scene with its shadows,
// plus the bloom (bloom.wgsl, thresholded, times its burn intensity), times the auto-exposure
// (exposure.wgsl), then a local tone mapping. It draws into a float target the curve reads with
// no mask left to apply (alpha 1), and HDR output reads in place of the scene (hdr.rs).
@group(1) @binding(0) var glow: texture_2d<f32>;
@group(1) @binding(1) var glow_sampler: sampler;
// exposure.wgsl's state: x the multiplier, y the adapted mean log2 luminance, z 1 once it is
// measured (0 with a fixed exposure).
@group(1) @binding(2) var<uniform> exposure: vec4<f32>;
// gfx/tonemap.rs `Grading::params`.
struct Params {
    // dt, the exposure's top and bottom in stops (exposure.wgsl); w the eye's night vision: 0 off,
    // 1 by the eye's adaptation, 2 by each pixel's light (`night_vision`).
    exposure: vec4<f32>,
    // The bloom's burn intensity and luminance threshold (bloom.wgsl).
    bloom: vec4<f32>,
    // Local tone mapping: exposure (relative to Remix's 0.21405728), shadows, highlights, sigma.
    local: vec4<f32>,
    // Its exposure preference offset; y the night vision's shift in stops.
    local_offset: vec4<f32>,
};
@group(1) @binding(3) var<uniform> params: Params;
// The scene's light averaged over about 8 pixels, its local adaptation level.
@group(1) @binding(4) var local_light: texture_2d<f32>;

// Remix's local tone mapping fuses three exposures of the image by how well exposed each is, a
// Gaussian of the pixel's stops around a preferred level (exposurePreferenceSigma and Offset):
// the light as it is, lifted by `shadows` and lowered by `highlights`. This one fuses them on
// the local average, which gives the ratio the pixel is scaled by, so its own detail stays.
const PREFERRED_STOPS: f32 = -1.5;
fn local_tone(light: vec3<f32>, average: f32) -> vec3<f32> {
    let stops = vec3(log2(params.local.y), 0.0, -log2(params.local.z));
    let at = log2(max(average, 1e-4)) + stops - (PREFERRED_STOPS + params.local_offset.x);
    let w = exp(-at * at / (2.0 * params.local.w * params.local.w));
    let fused = dot(w, exp2(stops)) / max(w.x + w.y + w.z, 1e-6);
    return light * fused;
}

// The eye's night vision (the Purkinje shift): in dim light its rods take over from its cones,
// they see no colour and see blue-green best (their sensitivity peaks at 507 nm), so a dim scene
// fades to a cool grey, before the exposure lifts it. The scene's light in paper whites, a sunlit
// white face 1: colour down to a moonlit field's (0.03), grey under a starlit one's (0.002), the
// mesopic range between them on a log scale.
// Which light decides it (`params.exposure.w`): 1, the eye's adaptation, the auto-exposure's
// adapted mean (exposure.wgsl `state.y`), so the whole view greys together as the eye adapts to
// the dark, and the same in both eyes of a headset (they share one meter); 2, each pixel's own
// light (`HORIZON_NIGHT_VISION=pixel`, the old way: a dark corner greyed under a noon sun).
// With the eye's, a pixel bright enough for the cones on its own (a torch, a lamp, a lit window
// seen from the night) keeps its colour, as a red light stays red in the dark.
const MOON_FIELD: f32 = 0.03;
const STAR_FIELD: f32 = 0.002;
// What a scene's mean is of its lit white face: the exposure's key (exposure.wgsl `KEY`, 2^-2.6).
const MEAN_OF_WHITE: f32 = 0.165;
// A pixel's own cones: from twice a moonlit white face to half a sunlit one.
const CONES_FROM: f32 = 0.06;
const CONES_FULL: f32 = 0.5;
fn rods_grey(c: vec3<f32>) -> vec3<f32> {
    // The rods' response in linear sRGB (the CIE scotopic curve folded onto the primaries), as a
    // grey with the slight blue it is seen with.
    let rods = dot(c, vec3(0.05, 0.55, 0.40));
    let tint = vec3(0.80, 0.90, 1.08);
    return tint * (rods / dot(tint, vec3(0.2126, 0.7152, 0.0722)));
}
fn night_vision(c: vec3<f32>) -> vec3<f32> {
    let mode = params.exposure.w;
    if mode < 0.5 { return c; }
    let y = dot(c, vec3(0.2126, 0.7152, 0.0722));
    var cones: f32;
    // A fixed exposure (or its first frame) has no adapted mean: the pixel's light.
    if mode > 1.5 || exposure.z < 0.5 {
        cones = smoothstep(log(STAR_FIELD), log(MOON_FIELD), log(max(y, 1e-7)));
    } else {
        // `HORIZON_NIGHT_VISION_EV` moves the eye's range up (greys sooner) or down, in stops.
        let shift = params.local_offset.y;
        let eye = exposure.y - shift;
        let lo = log2(STAR_FIELD * MEAN_OF_WHITE);
        let hi = log2(MOON_FIELD * MEAN_OF_WHITE);
        let adapted = smoothstep(lo, hi, eye);
        let own = smoothstep(log(CONES_FROM), log(CONES_FULL), log(max(y, 1e-7)));
        cones = max(adapted, own);
    }
    if cones >= 1.0 { return c; }
    return mix(rods_grey(c), c, cones);
}
fn grade(p: vec2<f32>) -> vec3<f32> {
    let c = textureLoad(source, vec2<i32>(p), 0);
    let uv = p / vec2<f32>(textureDimensions(source));
    let bloom = textureSampleLevel(glow, glow_sampler, uv, 0.0).rgb;
    let lit = night_vision(max(shadowed(c, p), vec3(0.0)));
    var light = (lit + bloom * params.bloom.x) * exposure.x;
    let near = textureSampleLevel(local_light, glow_sampler, uv, 0.0).rgb * exposure.x;
    light = local_tone(light, dot(near, vec3(0.2126, 0.7152, 0.0722)));
    return light * params.local.x;
}
@fragment fn fs_grade(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4(grade(p.xy), 1.0);
}
// The vignette over the finished frame (gfx/tonemap.rs `Vignette`), as vanilla draws it
// (RenderPipelines.VIGNETTE blends ZERO, ONE_MINUS_SRC_COLOR over the frame): the sprite stretched
// over the view, read with its own sampling (screen.wgsl `atlas_uv`).
struct VignetteParams {
    // The sprite's atlas rectangle (u0, v0, u1, v1).
    rect: vec4<f32>,
    // Its colour: how dark it makes the frame.
    color: vec4<f32>,
    // x: 1 to draw it; y: 1 for clamp-to-edge, else repeat.
    mode: vec4<f32>,
}
@group(2) @binding(0) var vignette_atlas: texture_2d<f32>;
@group(2) @binding(1) var vignette_sampler: sampler;
@group(2) @binding(2) var<uniform> vignette: VignetteParams;
// What the frame is multiplied by at `uv` (0..1 across the view, from its top left).
fn vignette_at(uv: vec2<f32>) -> vec3<f32> {
    if vignette.mode.x < 0.5 { return vec3(1.0); }
    let rect = vignette.rect;
    var at = rect.xy + fract(uv) * (rect.zw - rect.xy);
    if vignette.mode.y > 0.5 {
        let half = 0.5 / vec2<f32>(textureDimensions(vignette_atlas));
        at = clamp(rect.xy + uv * (rect.zw - rect.xy), rect.xy + half, rect.zw - half);
    }
    let c = textureSampleLevel(vignette_atlas, vignette_sampler, at, 0.0) * vignette.color;
    return vec3(1.0) - c.rgb;
}
// The grading and the curve in one pass, when nothing reads the graded light (gfx/tonemap.rs
// `run`'s `keep_graded`): as `fs_grade` then `fs_tonemap_aces`, without the float target between;
// the vignette over it.
@fragment fn fs_grade_aces(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = p.xy / vec2<f32>(textureDimensions(source));
    return vec4(pow(aces(grade(p.xy)), vec3(1.0 / 2.2)) * vignette_at(uv), 1.0);
}

@fragment fn fs_decode(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(source, vec2<i32>(p.xy), 0);
    return vec4(pow(max(c.rgb, vec3(0.0)), vec3(2.2)), c.a);
}
