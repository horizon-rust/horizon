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

// Half the scene's size: each texel is the mean of the 2x2 pixels it covers, shadows applied.
@fragment fn fs_first(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(scene));
    let base = vec2<i32>(p.xy) * 2;
    var sum = vec3(0.0);
    for (var k = 0; k < 4; k++) {
        let at = min(base + vec2(k & 1, k >> 1), size - 1);
        let c = textureLoad(scene, at, 0);
        var light = min(max(shadowed(c, vec2<f32>(at) + 0.5), vec3(0.0)), vec3(CEILING));
        if BRIGHT {
            // The light above the threshold, a pixel's colour scaled by the share of its
            // luminance that is over.
            let luma = max(dot(light, vec3(0.2126, 0.7152, 0.0722)) * exposure.x, 1e-4);
            light *= max(luma - params.bloom.y, 0.0) / luma;
        }
        sum += light;
    }
    return vec4(sum * 0.25, 1.0);
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
