// Bloom (gfx/bloom.rs): a chain of halved copies of the scene's light, each blurred as it goes
// down (the 13 taps of Jimenez's "Next Generation Post Processing in Call of Duty: Advanced
// Warfare"), then blended back up through a 3x3 tent. The bloom's chain keeps the light over a
// threshold (Remix's), which the grading pass adds to the scene; the other chain, with none, is
// measured for the exposure and the local tone mapping.
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_clamp: sampler;

// The scene's light and its ray-traced shadows' mask, for the first level only (the layout of
// tonemap.wgsl's `source` and `mask`).
@group(1) @binding(0) var scene: texture_2d<f32>;
@group(1) @binding(1) var mask: texture_2d<f32>;

// The sun's disc is tens of thousands of times paper white: its glare spreads it as the eye's does
// (the TAA keeps its pixels from shimmering); under the half floats' range.
const CEILING: f32 = 30000.0;

fn shadowed(c: vec4<f32>, p: vec2<f32>) -> vec3<f32> {
    let m = textureDimensions(mask);
    if m.x <= 1u || c.a > 0.75 { return c.rgb; }
    let at = vec2<i32>(p * vec2<f32>(m) / vec2<f32>(textureDimensions(scene)));
    let light = textureLoad(mask, at, 0).rg;
    let share = 2.0 * c.a;
    return c.rgb * (share * light.g + (1.0 - share) * light.r);
}

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2(f32(i32(i & 1u) * 4 - 1), f32(i32(i >> 1u) * 4 - 1));
    return vec4(p, 0.0, 1.0);
}

// gfx/tonemap.rs `Grading::params` (x: dt, y, z: the exposure's limits, w: unused; then the
// bloom's burn intensity and luminance threshold, in paper white's units) and the exposure
// (x: the multiplier), for the first level's threshold.
struct Params {
    exposure: vec4<f32>,
    bloom: vec4<f32>,
};
@group(2) @binding(0) var<uniform> params: Params;
@group(2) @binding(1) var<uniform> exposure: vec4<f32>;

// The bloom's own chain keeps only the light above the threshold, measured after the exposure
// (Remix's luminanceThreshold); the exposure's chain keeps all of it.
override BRIGHT: bool = false;
// The glow's first level steadied against the rays' noise (`fs_first`): on unless
// `HORIZON_BLOOM_STABLE=0`, which keeps the hard threshold and the plain mean.
override STABLE: bool = true;
// The most a pixel gives the glow over the threshold, in paper whites after the exposure (0: no
// limit, `HORIZON_BLOOM_MOST`). The sun's disc is tens of thousands of paper whites: unbounded,
// the chain's eight levels spread it into a white veil tens of degrees wide over the sky and
// whatever stands before it (in VR, where the sun is nearly always in view, over half the
// picture), and the disc's light times its pixels grew with the resolution. Bounded, the disc
// keeps a halo a few times its size; the sky's own glare (terrain.wgsl `rt_glare`) is the
// aureole around it. Lamps, lava and fire are far below it.
override GLOW_MOST: f32 = 64.0;

// Half the scene's size: each texel is the mean of the 2x2 pixels it covers, shadows applied.
@fragment fn fs_first(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(scene));
    let base = vec2<i32>(p.xy) * 2;
    var sum = vec3(0.0);
    var weights = 0.0;
    for (var k = 0; k < 4; k++) {
        let at = min(base + vec2(k & 1, k >> 1), size - 1);
        let c = textureLoad(scene, at, 0);
        var light = min(max(shadowed(c, vec2<f32>(at) + 0.5), vec3(0.0)), vec3(CEILING));
        var weight = 1.0;
        if BRIGHT {
            // The light above the threshold, a pixel's colour scaled by the share of its
            // luminance that is over.
            let luma = max(dot(light, vec3(0.2126, 0.7152, 0.0722)) * exposure.x, 1e-4);
            var over = max(luma - params.bloom.y, 0.0);
            if STABLE {
                // A soft knee half the threshold wide (a quadratic into the line): a surface
                // whose light wavers about the threshold (the rays' noise, a wave's glint) fades
                // into the glow, where the hard edge switched it on and off.
                let knee = 0.5 * params.bloom.y;
                let soft = clamp(luma - params.bloom.y + knee, 0.0, 2.0 * knee);
                over = max(over, soft * soft / max(4.0 * knee, 1e-4));
            }
            if GLOW_MOST > 0.0 { over = min(over, GLOW_MOST); }
            light *= over / luma;
            if STABLE {
                // Karis' average (Jimenez 2014, the first level only): a pixel counts for less
                // the brighter it is, so one hot pixel among dark ones (a firefly, a glint a
                // pixel wide the jitter catches one frame in a few) no longer lights a whole
                // patch of the glow on and off. A bright area keeps its light: its pixels weigh
                // the same.
                weight = 1.0 / (1.0 + over);
            }
        }
        sum += light * weight;
        weights += weight;
    }
    return vec4(sum / weights, 1.0);
}

@fragment fn fs_down(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(source));
    let t = 1.0 / dims;
    let uv = p.xy * 2.0 / dims;
    let a = textureSampleLevel(source, linear_clamp, uv + vec2(-2.0, -2.0) * t, 0.0).rgb;
    let b = textureSampleLevel(source, linear_clamp, uv + vec2(0.0, -2.0) * t, 0.0).rgb;
    let c = textureSampleLevel(source, linear_clamp, uv + vec2(2.0, -2.0) * t, 0.0).rgb;
    let d = textureSampleLevel(source, linear_clamp, uv + vec2(-2.0, 0.0) * t, 0.0).rgb;
    let e = textureSampleLevel(source, linear_clamp, uv, 0.0).rgb;
    let f = textureSampleLevel(source, linear_clamp, uv + vec2(2.0, 0.0) * t, 0.0).rgb;
    let g = textureSampleLevel(source, linear_clamp, uv + vec2(-2.0, 2.0) * t, 0.0).rgb;
    let h = textureSampleLevel(source, linear_clamp, uv + vec2(0.0, 2.0) * t, 0.0).rgb;
    let i = textureSampleLevel(source, linear_clamp, uv + vec2(2.0, 2.0) * t, 0.0).rgb;
    let j = textureSampleLevel(source, linear_clamp, uv + vec2(-1.0, -1.0) * t, 0.0).rgb;
    let k = textureSampleLevel(source, linear_clamp, uv + vec2(1.0, -1.0) * t, 0.0).rgb;
    let l = textureSampleLevel(source, linear_clamp, uv + vec2(-1.0, 1.0) * t, 0.0).rgb;
    let m = textureSampleLevel(source, linear_clamp, uv + vec2(1.0, 1.0) * t, 0.0).rgb;
    let light = e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625
        + (j + k + l + m) * 0.125;
    return vec4(light, 1.0);
}

// The tent of the smaller level, drawn over the one it is drawn into with a blend constant
// (gfx/bloom.rs): the sum of all the levels' weights stays 1.
@fragment fn fs_up(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(source));
    let t = 1.0 / dims;
    let uv = p.xy / (dims * 2.0);
    var light = textureSampleLevel(source, linear_clamp, uv, 0.0).rgb * 4.0;
    light += textureSampleLevel(source, linear_clamp, uv + vec2(-t.x, 0.0), 0.0).rgb * 2.0;
    light += textureSampleLevel(source, linear_clamp, uv + vec2(t.x, 0.0), 0.0).rgb * 2.0;
    light += textureSampleLevel(source, linear_clamp, uv + vec2(0.0, -t.y), 0.0).rgb * 2.0;
    light += textureSampleLevel(source, linear_clamp, uv + vec2(0.0, t.y), 0.0).rgb * 2.0;
    light += textureSampleLevel(source, linear_clamp, uv + vec2(-t.x, -t.y), 0.0).rgb;
    light += textureSampleLevel(source, linear_clamp, uv + vec2(t.x, -t.y), 0.0).rgb;
    light += textureSampleLevel(source, linear_clamp, uv + vec2(-t.x, t.y), 0.0).rgb;
    light += textureSampleLevel(source, linear_clamp, uv + vec2(t.x, t.y), 0.0).rgb;
    return vec4(light / 16.0, 1.0);
}
