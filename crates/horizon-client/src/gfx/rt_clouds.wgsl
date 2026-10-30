// RT lighting's volumetric clouds (gfx/rt_clouds.rs): cumulus in a slab 700 to 1300 blocks up (one
// block a metre), marched at a fraction of the view's resolution, lit by the real sun (Beer's law
// toward it, several octaves of multiple scattering, a dual-lobe Henyey-Greenstein phase for the
// silver lining), by the sky from below and above, and blended toward the sky's own light with
// distance. Three passes: `cs_march` (this frame's jittered ray), `cs_resolve` (the history,
// reprojected by view direction and clamped to the neighbours) and `fs_composite` over the scene
// (the scene times the transmittance, plus the light), where the depth shows the sky.

const PI: f32 = 3.14159265;
// The slab, in metres, and where the noise tiles repeat: the camera and the wind are wrapped at
// WRAP, a multiple of every tile's size, so single precision holds far from the origin.
const BASE: f32 = 700.0;
const TOP: f32 = 1300.0;
const WRAP: f32 = 32768.0;
const SHAPE_XZ: f32 = 4096.0;
const SHAPE_Y: f32 = 3072.0;
const DETAIL: f32 = 512.0;
const COVERAGE: f32 = 16384.0;
// The extinction at density 1 (per metre), and how far the march and the haze go.
const SIGMA: f32 = 0.05;
const MAX_DISTANCE: f32 = 45000.0;
const HAZE: f32 = 14000.0;
const STEPS: i32 = 40;
const LIGHT_STEPS: i32 = 6;
// The sun's light scattered by the cloud, against Lambert's (a white surface facing the sun is 1).
const SUN_GAIN: f32 = 1.4;
const AMBIENT_GAIN: f32 = 1.2;

struct Clouds {
    inverse_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    // xyz toward the light (the sun's, or the moon's once it has set), w unused.
    light: vec4<f32>,
    // rgb the light's colour in paper whites (Globals.rt_sun_light).
    light_color: vec4<f32>,
    // xyz toward the sun, for the sky table's azimuth.
    sun: vec4<f32>,
    // xyz the camera's place (x and z wrapped at WRAP, y real), w the weather's thickening (0..1).
    camera: vec4<f32>,
    // xy the wind's drift (wrapped), z the frame, w the new frame's share of the history.
    wind: vec4<f32>,
    // xyz the camera's move since the history's frame, w 1 when there is one.
    motion: vec4<f32>,
    // The view's size, the cloud target's size.
    size: vec4<f32>,
    // xy the jitter inside a cloud texel (-0.5..0.5), z the texels' scale on the view's.
    jitter: vec4<f32>,
}
@group(0) @binding(0) var<uniform> clouds: Clouds;
@group(0) @binding(1) var depth: texture_depth_2d;

// The march's bindings.
@group(0) @binding(2) var shape_noise: texture_3d<f32>;
@group(0) @binding(3) var detail_noise: texture_3d<f32>;
@group(0) @binding(4) var coverage_noise: texture_2d<f32>;
@group(0) @binding(5) var repeat_sampler: sampler;
@group(0) @binding(6) var sky_view: texture_2d<f32>;
@group(0) @binding(7) var clamp_sampler: sampler;
@group(0) @binding(8) var raw_out: texture_storage_2d<rgba16float, write>;

// The sky table toward `d` (unit), as terrain.wgsl `rt_sky`.
fn sky_at(d: vec3<f32>) -> vec3<f32> {
    let flat = d.xz / max(length(d.xz), 1e-5);
    let sun = clouds.sun.xz / max(length(clouds.sun.xz), 1e-5);
    let azimuth = acos(clamp(dot(flat, sun), -1.0, 1.0)) / PI;
    let e = asin(clamp(d.y, -1.0, 1.0)) / 1.5707963;
    let v = 0.5 + 0.5 * sign(e) * sqrt(abs(e));
    return textureSampleLevel(sky_view, clamp_sampler, vec2(azimuth, v), 0.0).rgb;
}

// The camera-relative ray through the view's `uv` (0..1): its start on the near plane, its
// direction toward a point almost at the far one (so an eye's offset in VR is in the start).
fn ray_at(uv: vec2<f32>) -> array<vec3<f32>, 2> {
    let ndc = vec2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let near = clouds.inverse_view_proj * vec4(ndc, 1.0, 1.0);
    let far = clouds.inverse_view_proj * vec4(ndc, 0.0005, 1.0);
    let start = near.xyz / near.w;
    return array<vec3<f32>, 2>(start, normalize(far.xyz / far.w - start));
}

fn remap(v: f32, lo: f32, hi: f32) -> f32 {
    return (v - lo) / (hi - lo);
}

// The weather map at `xz` (world, wrapped): the share of the sky covered (x), and a second noise
// for the clouds' height (y).
fn coverage_at(xz: vec2<f32>) -> vec2<f32> {
    let c = textureSampleLevel(coverage_noise, repeat_sampler, xz / COVERAGE, 0.0).rg;
    let thick = clouds.camera.w;
    // The coverage curve: gaps between the cumulus, more clouds, wider and taller, in bad weather.
    return vec2(saturate(remap(c.x, 0.58 - 0.2 * thick, 0.88 - 0.3 * thick)), c.y);
}

// The cloud's density at `p` (world, x and z wrapped and drifted) in the slab at height share
// `h` (0..1), with the weather map's `cover`; `fine` adds the high-frequency erosion.
fn density(p: vec3<f32>, h: f32, cover: vec2<f32>, fine: bool) -> f32 {
    // Cumulus: a flat base, a rounded top as high as the weather map says.
    let top = mix(0.3, 1.0, cover.x) * mix(0.8, 1.0, cover.y);
    let profile = smoothstep(0.0, 0.08, h) * (1.0 - smoothstep(top * 0.55, top, h));
    if profile <= 0.0 { return 0.0; }
    // Wind shears the clouds a little with height.
    let q = p + vec3(h * 120.0, 0.0, h * 60.0);
    let s = textureSampleLevel(shape_noise, repeat_sampler, vec3(q.x / SHAPE_XZ, q.y / SHAPE_Y, q.z / SHAPE_XZ), 0.0);
    let fbm = s.g * 0.625 + s.b * 0.25 + s.a * 0.125;
    var base = saturate(remap(s.r, fbm - 1.0, 1.0)) * profile;
    base = saturate(remap(base, 1.0 - cover.x * 0.95, 1.0)) * cover.x;
    if base <= 0.0 { return 0.0; }
    if !fine { return base; }
    let d = textureSampleLevel(detail_noise, repeat_sampler, q / DETAIL, 0.0);
    let erosion = d.r * 0.625 + d.g * 0.25 + d.b * 0.125;
    // Wispy at the base, billowy higher up.
    let modifier = mix(erosion, 1.0 - erosion, saturate(h * 6.0));
    return saturate(remap(base, modifier * 0.4, 1.0));
}

// The Henyey-Greenstein phase (integrating to 1 over the sphere) for the cosine `c` and `g`.
fn hg(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * c, 1e-3), 1.5));
}
// The dual lobe: a strong forward one (the silver lining) and a weak backward one.
fn phase(c: f32, spread: f32) -> f32 {
    return mix(hg(c, 0.82 * spread), hg(c, -0.3 * spread), 0.22);
}

// The light's optical depth from `p` toward the light, a few steps growing along the way.
fn light_depth(p: vec3<f32>, jitter: f32) -> f32 {
    let l = clouds.light.xyz;
    var depth_sum = 0.0;
    var t = 0.0;
    var len = 18.0;
    for (var i = 0; i < LIGHT_STEPS; i++) {
        let s = len * (1.0 + 0.4 * jitter);
        let q = p + l * (t + s * 0.5);
        let y = q.y;
        let h = (y - BASE) / (TOP - BASE);
        t += s;
        len *= 2.0;
        if h < 0.0 || h > 1.0 { continue; }
        let cover = coverage_at(q.xz);
        if cover.x <= 0.0 { continue; }
        depth_sum += density(q, h, cover, i < 2) * s;
    }
    return depth_sum * SIGMA;
}

// A noise by pixel and frame (interleaved gradient noise).
fn gradient_noise(pixel: vec2<f32>, frame: f32) -> f32 {
    let q = pixel + 5.588238 * (frame % 64.0);
    return fract(52.9829189 * fract(dot(q, vec2(0.06711056, 0.00583715))));
}

// Light through the slab along the ray from the camera at `origin_y` toward `d`: its start and
// end distances (end < start: none).
fn slab(origin_y: f32, d: vec3<f32>) -> vec2<f32> {
    var near = 0.0;
    var far = MAX_DISTANCE;
    if abs(d.y) < 1e-5 {
        if origin_y < BASE || origin_y > TOP { return vec2(1.0, 0.0); }
        return vec2(near, far);
    }
    let a = (BASE - origin_y) / d.y;
    let b = (TOP - origin_y) / d.y;
    near = max(near, min(a, b));
    far = min(far, max(a, b));
    return vec2(near, far);
}

// This frame's clouds along the view ray: rgb the light they send, a their transmittance.
fn march(origin: vec3<f32>, d: vec3<f32>, noise: f32) -> vec4<f32> {
    let range = slab(clouds.camera.y + origin.y, d);
    if range.y <= range.x { return vec4(0.0, 0.0, 0.0, 1.0); }
    let start = vec3(clouds.camera.x + origin.x + clouds.wind.x, clouds.camera.y + origin.y,
        clouds.camera.z + origin.z + clouds.wind.y);
    let step_length = (range.y - range.x) / f32(STEPS);
    let c = dot(d, clouds.light.xyz);
    let sun_color = clouds.light_color.rgb;
    // The sky from above, and the light the ground sends up (its colour a bit green and brown).
    let up_sky = sky_at(vec3(0.0, 1.0, 0.0)) * 0.6 + sky_at(normalize(vec3(d.x, 0.35, d.z))) * 0.4;
    let ground = sun_color * max(clouds.light.y, 0.0) * vec3(0.12, 0.13, 0.08);
    var light = vec3(0.0);
    var transmittance = 1.0;
    var weighted = 0.0;
    var weight = 0.0;
    var optical = 0.0;
    for (var i = 0; i < STEPS; i++) {
        let t = range.x + (f32(i) + noise) * step_length;
        let q = start + d * t;
        let h = (q.y - BASE) / (TOP - BASE);
        let cover = coverage_at(q.xz);
        if cover.x <= 0.0 { continue; }
        let rho = density(q, h, cover, true);
        if rho <= 0.002 { continue; }
        let od = rho * SIGMA * step_length;
        let step_transmittance = exp(-od);
        // The light reaching the sample, through three octaves of scattering: each weaker, with
        // less extinction and a rounder phase, which lifts the shadowed side of a thick cloud.
        let tau = light_depth(q, noise);
        var sun = 0.0;
        var a = 1.0;
        var b = 1.0;
        var spread = 1.0;
        for (var k = 0; k < 3; k++) {
            sun += a * exp(-tau * b) * phase(c, spread);
            a *= 0.5;
            b *= 0.5;
            spread *= 0.5;
        }
        // The powder effect: a cloud's edge (little cloud seen through yet) is darker than its
        // inside, except toward the sun.
        let powder = mix(1.0, 1.0 - exp(-2.0 * (optical + od)), 0.6 * (1.0 - saturate(c)));
        // The sky reaches a cloud's top more than its belly, and a thick cloud less.
        let sky_share = mix(0.3, 1.0, pow(saturate(h), 0.6)) * exp(-tau * 0.04);
        let scattered = sun_color * (sun * powder * PI * SUN_GAIN)
            + up_sky * (sky_share * AMBIENT_GAIN)
            + ground * (1.0 - saturate(h)) * (1.0 - saturate(h));
        let absorbed = transmittance * (1.0 - step_transmittance);
        light += absorbed * scattered;
        weighted += absorbed * t;
        weight += absorbed;
        optical += od;
        transmittance *= step_transmittance;
        if transmittance < 0.01 { break; }
    }
    if weight <= 0.0 { return vec4(0.0, 0.0, 0.0, 1.0); }
    // Aerial perspective: toward the horizon a cloud takes the sky's colour (the air between)
    // and fades out.
    let distance = weighted / weight;
    let haze = 1.0 - exp(-distance / HAZE);
    let gone = smoothstep(18000.0, MAX_DISTANCE, distance);
    let opacity = 1.0 - transmittance;
    let tinted = mix(light, sky_at(d) * opacity, haze);
    return vec4(tinted * (1.0 - gone), 1.0 - opacity * (1.0 - gone));
}

@compute @workgroup_size(8, 8) fn cs_march(@builtin(global_invocation_id) id: vec3<u32>) {
    let low = vec2<u32>(clouds.size.zw);
    if id.x >= low.x || id.y >= low.y { return; }
    let scale = clouds.jitter.z;
    let full = vec2<i32>(clouds.size.xy);
    let center = (vec2<f32>(id.xy) + 0.5) * scale;
    // Under solid terrain all around, nothing to see: the texel and its neighbours' footprints
    // (the composite blends between them) are all covered.
    var open = false;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let at = clamp(vec2<i32>(center + vec2(f32(x), f32(y)) * scale), vec2(0), full - 1);
            if textureLoad(depth, at, 0) <= 0.0 { open = true; }
        }
    }
    if !open {
        textureStore(raw_out, vec2<i32>(id.xy), vec4(0.0, 0.0, 0.0, 1.0));
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5 + clouds.jitter.xy) * scale / clouds.size.xy;
    let ray = ray_at(uv);
    let noise = gradient_noise(vec2<f32>(id.xy), clouds.wind.z);
    textureStore(raw_out, vec2<i32>(id.xy), march(ray[0], ray[1], noise));
}

// The resolve pass's bindings (its own layout, so it can read what the march wrote).
@group(0) @binding(10) var raw_in: texture_2d<f32>;
@group(0) @binding(11) var history: texture_2d<f32>;
@group(0) @binding(12) var history_sampler: sampler;
@group(0) @binding(13) var resolved: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8) fn cs_resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    let low = vec2<u32>(clouds.size.zw);
    if id.x >= low.x || id.y >= low.y { return; }
    let p = vec2<i32>(id.xy);
    let top = vec2<i32>(low) - 1;
    let current = textureLoad(raw_in, p, 0);
    var lo = current;
    var hi = current;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let n = textureLoad(raw_in, clamp(p + vec2(x, y), vec2(0), top), 0);
            lo = min(lo, n);
            hi = max(hi, n);
        }
    }
    let scale = clouds.jitter.z;
    let uv = (vec2<f32>(id.xy) + 0.5) * scale / clouds.size.xy;
    let ray = ray_at(uv);
    // A far point on the pixel's ray, seen from the last frame's camera.
    let moved = ray[0] + ray[1] * 20000.0 + clouds.motion.xyz;
    let clip = clouds.prev_view_proj * vec4(moved, 1.0);
    var out = current;
    if clouds.motion.w > 0.5 && clip.w > 0.0 {
        let ndc = clip.xy / clip.w;
        let prev_uv = vec2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if all(prev_uv > vec2(0.0)) && all(prev_uv < vec2(1.0)) {
            let at = prev_uv * clouds.size.xy / (clouds.size.zw * scale);
            // A little margin around the neighbours: the clouds are soft, the jitter moves them.
            let margin = (hi - lo) * 0.25 + 0.01;
            let past = clamp(textureSampleLevel(history, history_sampler, at, 0.0), lo - margin, hi + margin);
            out = mix(past, current, clouds.wind.w);
        }
    }
    textureStore(resolved, p, out);
}

// The composite pass: a triangle over the view, the clouds' light and transmittance (blended as
// the scene times the one plus the other) wherever the depth is the sky's.
@group(0) @binding(14) var cloud_light: texture_2d<f32>;
@group(0) @binding(15) var cloud_sampler: sampler;

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment fn fs_composite(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(pos.xy);
    if textureLoad(depth, at, 0) > 0.0 { discard; }
    let uv = pos.xy / (clouds.size.zw * clouds.jitter.z);
    let c = textureSampleLevel(cloud_light, cloud_sampler, uv, 0.0);
    return vec4(max(c.rgb, vec3(0.0)), clamp(c.a, 0.0, 1.0));
}
