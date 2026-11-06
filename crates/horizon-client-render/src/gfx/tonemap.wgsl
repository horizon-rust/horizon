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
// x: the exposure's multiplier.
@group(1) @binding(2) var<uniform> exposure: vec4<f32>;
// gfx/tonemap.rs `Grading::params`.
struct Params {
    // dt, the exposure's top and bottom in stops (exposure.wgsl); w 1 for the eye's night vision.
    exposure: vec4<f32>,
    // The bloom's burn intensity and luminance threshold (bloom.wgsl).
    bloom: vec4<f32>,
    // Local tone mapping: exposure (relative to Remix's 0.21405728), shadows, highlights, sigma.
    local: vec4<f32>,
    // Its exposure preference offset.
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
// mesopic range between them on a log scale. A torch's or a lamp's light stays in colour.
fn night_vision(c: vec3<f32>) -> vec3<f32> {
    if params.exposure.w < 0.5 { return c; }
    let y = dot(c, vec3(0.2126, 0.7152, 0.0722));
    let cones = smoothstep(log(0.002), log(0.03), log(max(y, 1e-7)));
    if cones >= 1.0 { return c; }
    // The rods' response in linear sRGB (the CIE scotopic curve folded onto the primaries), as a
    // grey with the slight blue it is seen with.
    let rods = dot(c, vec3(0.05, 0.55, 0.40));
    let tint = vec3(0.80, 0.90, 1.08);
    let grey = tint * (rods / dot(tint, vec3(0.2126, 0.7152, 0.0722)));
    return mix(grey, c, cones);
}
@fragment fn fs_grade(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(source, vec2<i32>(p.xy), 0);
    let uv = p.xy / vec2<f32>(textureDimensions(source));
    let bloom = textureSampleLevel(glow, glow_sampler, uv, 0.0).rgb;
    let lit = night_vision(max(shadowed(c, p.xy), vec3(0.0)));
    var light = (lit + bloom * params.bloom.x) * exposure.x;
    let near = textureSampleLevel(local_light, glow_sampler, uv, 0.0).rgb * exposure.x;
    light = local_tone(light, dot(near, vec3(0.2126, 0.7152, 0.0722)));
    return vec4(light * params.local.x, 1.0);
}

@fragment fn fs_decode(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(source, vec2<i32>(p.xy), 0);
    return vec4(pow(max(c.rgb, vec3(0.0)), vec3(2.2)), c.a);
}
