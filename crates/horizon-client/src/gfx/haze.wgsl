// Heat haze (gfx/haze.rs) on the linear HDR scene, before the tone mapping: the air above a flame
// bends the light. A column of rising noise above each flame refracts the scene behind it.
//
//   vs_haze      a screen rectangle around a column's projection, one instance a flame;
//   fs_distort   into a scratch target, the scene sampled where the column's air sends the ray;
//   fs_copy      the scratch back over the scene.
struct Haze {
    // The view's projection (relative to the camera) and its inverse.
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    // The scene's width and height in pixels, and their inverses.
    size: vec4<f32>,
    // x: the time in seconds; y, z: the depth buffer's view depth is 1 / (y * depth + z).
    params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> haze: Haze;
@group(0) @binding(1) var scene: texture_2d<f32>;
// The world's depth, reversed (1 at the near plane, 0 the sky).
@group(0) @binding(2) var depth: texture_depth_2d;
@group(0) @binding(3) var linear_sampler: sampler;
// What `fs_copy` copies back.
@group(0) @binding(4) var scratch: texture_2d<f32>;

// The column widens with height: the hot air spreads as it rises.
const WIDENING: f32 = 0.9;
// How far a column's noise reaches in blocks, in units of its radius, and how fast it rises, in
// heights a second.
const GRAIN: f32 = 1.4;
const RISE: f32 = 0.9;
// Beyond this many blocks the haze has faded (RANGE in haze.rs is the CPU's cut).
const FADE_FROM: f32 = 14.0;
const FADE_TO: f32 = 24.0;

struct Column {
    @builtin(position) position: vec4<f32>,
    // The column: xyz where it starts (relative to the camera), w its radius; x its height, y how
    // far it bends the light, z its own noise number.
    @location(0) @interpolate(flat) base: vec4<f32>,
    @location(1) @interpolate(flat) shape: vec4<f32>,
}
@vertex
fn vs_haze(@builtin(vertex_index) i: u32, @location(0) base: vec4<f32>, @location(1) shape: vec4<f32>)
    -> Column {
    let radius = base.w;
    // The column's box: its widest (the top's) radius, from a little below its start to its end.
    let reach = radius * (1.0 + WIDENING);
    var low = vec2(1.0);
    var high = vec2(-1.0);
    var in_front = 0u;
    for (var c = 0u; c < 8u; c++) {
        let corner = base.xyz + vec3(
            select(-reach, reach, (c & 1u) != 0u),
            select(-0.5 * radius, shape.x, (c & 2u) != 0u),
            select(-reach, reach, (c & 4u) != 0u));
        let clip = haze.view_proj * vec4(corner, 1.0);
        if clip.w > 0.05 {
            let ndc = clip.xy / clip.w;
            low = min(low, ndc);
            high = max(high, ndc);
            in_front++;
        }
    }
    // A column the camera is inside of (some corners behind) covers the screen; one behind it
    // (all of them) covers nothing.
    if in_front < 8u {
        low = select(vec2(1.0), vec2(-1.0), in_front > 0u);
        high = select(vec2(-1.0), vec2(1.0), in_front > 0u);
    }
    low = clamp(low, vec2(-1.0), vec2(1.0));
    high = clamp(high, vec2(-1.0), vec2(1.0));
    let p = vec2(select(low.x, high.x, (i & 1u) != 0u), select(low.y, high.y, (i & 2u) != 0u));
    return Column(vec4(p, 0.0, 1.0), base, shape);
}

// A lattice point's random number from its integer coordinates (PCG3D, Jarzynski and Olano 2020:
// terrain.wgsl's `lava_hash`).
fn hash(p: vec3<f32>) -> f32 {
    var v = bitcast<vec3<u32>>(vec3<i32>(p)) * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return f32(v.x >> 8u) / 16777216.0;
}
fn noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(hash(i), hash(i + vec3(1.0, 0.0, 0.0)), u.x),
        mix(hash(i + vec3(0.0, 1.0, 0.0)), hash(i + vec3(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(hash(i + vec3(0.0, 0.0, 1.0)), hash(i + vec3(1.0, 0.0, 1.0)), u.x),
        mix(hash(i + vec3(0.0, 1.0, 1.0)), hash(i + vec3(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

fn view_depth(d: f32) -> f32 {
    return 1.0 / max(haze.params.y * d + haze.params.z, 1e-9);
}
fn load_depth(p: vec2<i32>) -> f32 {
    return textureLoad(depth, clamp(p, vec2(0), vec2<i32>(haze.size.xy) - 1), 0);
}

@fragment
fn fs_distort(column: Column) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(column.position.xy);
    let own = textureLoad(scene, pixel, 0);
    let base = column.base.xyz;
    let radius = column.base.w;
    let height = column.shape.x;
    // The plane across the view through the column's middle, and where this pixel's ray meets it.
    let middle = base + vec3(0.0, 0.5 * height, 0.0);
    let toward = vec3(haze.view_proj[0][3], haze.view_proj[1][3], haze.view_proj[2][3]);
    let plane = dot(toward, middle);
    if plane < 0.1 { return own; }
    let ndc = vec2(column.position.x * haze.size.z * 2.0 - 1.0, 1.0 - column.position.y * haze.size.w * 2.0);
    let far = haze.inverse_view_proj * vec4(ndc, 0.5, 1.0);
    let ray = normalize(far.xyz / far.w);
    let at = ray * (plane / dot(toward, ray));

    // How deep into the column: strongest just above the flame, gone with the height, across its
    // width (it widens as it rises), and with the distance. In front of the plane nothing bends.
    let rel = at - base;
    let rise = clamp(rel.y / height, 0.0, 1.0);
    let across = radius * (1.0 + WIDENING * rise);
    let weight = (1.0 - smoothstep(0.3 * across, across, length(rel.xz)))
        * smoothstep(-0.3 * radius, 0.2 * radius, rel.y)
        * pow(1.0 - rise, 1.5)
        * (1.0 - smoothstep(FADE_FROM, FADE_TO, length(middle)))
        * smoothstep(plane - 1.5 * radius, plane - 0.5 * radius, view_depth(load_depth(pixel)));
    if weight < 0.002 { return own; }

    // Two noises rising with the time, in the column's own coordinates (relative to its flame, so
    // they stay precise far from the world's origin), a different slice of them for each flame.
    let seed = column.shape.z;
    let grain = GRAIN / radius;
    let p = (vec3(rel.x, rel.y - haze.params.x * RISE * height, rel.z)) * grain
        + seed * vec3(37.1, 11.3, 23.7);
    let n = vec2(noise(p) - 0.5, noise(p + vec3(31.4, 17.7, 8.3)) - 0.5) * 2.0;
    // Mostly vertical, as the air rises, with some sideways sway.
    let bend = column.shape.y * weight * vec2(0.6 * n.x, n.y);
    // The bend is in the plane across the view: its axes are the projection's.
    let right = normalize(vec3(haze.view_proj[0][0], haze.view_proj[1][0], haze.view_proj[2][0]));
    let up = normalize(vec3(haze.view_proj[0][1], haze.view_proj[1][1], haze.view_proj[2][1]));
    let moved = at + right * bend.x + up * bend.y;
    let a = haze.view_proj * vec4(at, 1.0);
    let b = haze.view_proj * vec4(moved, 1.0);
    let shift = (b.xy / b.w - a.xy / a.w) * vec2(0.5, -0.5) * haze.size.xy;
    let origin = column.position.xy + shift;
    // Nothing in front of the column is pulled into it: where the offset lands on something
    // nearer than the plane, the pixel stays as it is.
    if view_depth(load_depth(vec2<i32>(origin))) < plane - 0.5 * radius { return own; }
    return textureSampleLevel(scene, linear_sampler, origin * haze.size.zw, 0.0);
}

@fragment
fn fs_copy(column: Column) -> @location(0) vec4<f32> {
    return textureLoad(scratch, vec2<i32>(column.position.xy), 0);
}
