// RT lighting's volumetric clouds (gfx/rt_clouds.rs), rebuilt after the way the well-known
// real-time cloud renderers do it (Schneider's Nubis for Horizon Zero Dawn, Hillaire's Frostbite
// clouds, the Minecraft packs that follow them): written from those talks' ideas, no code taken.
//
// - Two layers. Cumulus in a slab 800 to 2200 blocks up (one block a metre), their shape a
//   Perlin-Worley noise cut by a weather map (the coverage, and the cloud's type from flat
//   stratocumulus to towering congestus, which sets how high it grows), eroded at its edges by a
//   finer Worley noise (wispy under, billowy on top). Above, at 8 km, a thin sheet of cirrus,
//   streaked along the wind: a lookup or two a pixel, but it gives a clear sky its depth and the
//   sunsets their colour.
// - The march skips the empty air in long steps with the coarse shape alone and steps back into a
//   cloud in short ones, shorter near the eye (a step about 1 % of the distance).
// - The light at a sample: the sun's through the cloud toward it (six steps growing in length,
//   Beer's law), in four octaves of multiple scattering (each weaker, less absorbed and less
//   forward: Wrenninge's approximation) with a dual-lobe phase (the silver lining, a little back
//   scatter); the sky's from above and the ground's from below, each through its own short march
//   (dark bellies, bright tops); the powder effect on the edges away from the sun. Integrated in
//   the energy-conserving way (Hillaire 2016).
// - The air between: toward the horizon a cloud takes the sky's colour and fades.
//
// Two ways to draw them:
// - The dome (default): the clouds are hundreds of metres away or more, so both eyes of a headset
//   see them alike (6 cm apart, under a hundredth of a degree), and a frame looks much like the
//   last. The clouds are marched once a frame into a map of the sky's directions (`cs_dome_march`,
//   one texel in N*N a frame, in turn), reprojected and accumulated there (`cs_dome_resolve`: the
//   camera's move and the wind's, by each texel's distance), and every view only looks the map up
//   (`fs_dome_composite`). The map's rows are finer toward the horizon, where clouds are far and
//   small.
// - Per view (`cs_march`, `cs_resolve`, `fs_composite`): marched at a fraction of the view's
//   resolution; for a camera near or inside the slab, where the dome's parallax breaks.

const PI: f32 = 3.14159265;
// The cumulus slab and the cirrus sheet, in metres, and where the noise tiles repeat: the camera and
// the wind are wrapped at WRAP, a multiple of every tile's size, so single precision holds far
// from the origin.
const BASE: f32 = 800.0;
const TOP: f32 = 2200.0;
const CIRRUS: f32 = 8000.0;
// The altocumulus' thin layer: small cells, a mackerel sky.
const ALTO_BASE: f32 = 3800.0;
const ALTO_STEPS = 24;
const ALTO_TOP: f32 = 3950.0;
const WRAP: f32 = 32768.0;
const SHAPE_XZ: f32 = 4096.0;
const SHAPE_Y: f32 = 4096.0;
const DETAIL: f32 = 512.0;
const COVERAGE: f32 = 16384.0;
// The extinction at density 1 (per metre), and how far the march and the haze go.
const SIGMA: f32 = 0.06;
const ALBEDO: f32 = 0.99;
const HAZE: f32 = 22000.0;
const ITERATIONS: i32 = 288;
// The light the octaves leave out (the higher orders of scattering, which make a thick cloud's
// lit side about as bright as a white surface facing the sun, and its belly a light grey).
const SUN_GAIN: f32 = 2.0;
const AMBIENT_GAIN: f32 = 1.8;
const LIGHT_STEPS: i32 = 6;
// The lowest elevation the dome holds (radians): the slab bends below the horizon far away when the
// Earth's curvature is on.
const E_MIN: f32 = -0.0523599;

struct Clouds {
    inverse_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    // xyz toward the light (the sun's, or the moon's once it has set), w the Earth's curvature
    // (1 / its diameter; 0 off).
    light: vec4<f32>,
    // rgb the light's colour in paper whites (Globals.rt_sun_light), w the cirrus's amount (0..1).
    light_color: vec4<f32>,
    // xyz toward the sun, for the sky table's azimuth; w the day's coverage bias (-1..1).
    sun: vec4<f32>,
    // xyz the camera's place (x and z wrapped at WRAP, y real), w the weather's thickening (0..1).
    camera: vec4<f32>,
    // xy the wind's drift (wrapped), z the frame, w the new frame's share of the history.
    wind: vec4<f32>,
    // xyz the move to add to a camera-relative point to see it from the history's camera (the
    // camera's move and the wind's), w 1 when there is a history.
    motion: vec4<f32>,
    // The view's size (the dome's), the march target's size.
    size: vec4<f32>,
    // xy the jitter inside a march texel (-0.5..0.5), z the texels' scale on the view's (the dome:
    // its checkerboard's side), w the dome's texel this frame marches in each square (x + y * z).
    jitter: vec4<f32>,
    // x the altocumulus' amount (0..1), y how far the cumulus reach (blocks); the shadow map: z
    // its texels' side in blocks.
    layers: vec4<f32>,
    // xy the altocumulus' drift, zw the cirrus' (wrapped; they move faster than the cumulus).
    drifts: vec4<f32>,
}
@group(0) @binding(0) var<uniform> clouds: Clouds;
@group(0) @binding(1) var depth: texture_2d<f32>; // a depth view read as unfilterable float, which GLSL can translate

// The march's bindings.
@group(0) @binding(2) var shape_noise: texture_3d<f32>;
@group(0) @binding(3) var detail_noise: texture_3d<f32>;
@group(0) @binding(4) var coverage_noise: texture_2d<f32>;
@group(0) @binding(5) var repeat_sampler: sampler;
@group(0) @binding(6) var sky_view: texture_2d<f32>;
@group(0) @binding(7) var clamp_sampler: sampler;
@group(0) @binding(8) var raw_out: texture_storage_2d<rgba16float, write>;
// The dome's march also writes each texel's distance (metres; 0 for no cloud).
@group(0) @binding(9) var raw_distance_out: texture_storage_2d<r32float, write>;

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

// The dome's map: u the azimuth, v the elevation from E_MIN to the zenith, its rows finer toward
// the horizon (the elevation's share is (v^2 + v) / 2: three times finer there than overhead).
fn dome_dir(uv: vec2<f32>) -> vec3<f32> {
    let azimuth = uv.x * 2.0 * PI;
    let s = clamp(uv.y, 0.0, 1.0);
    let e = E_MIN + (0.5 * PI - E_MIN) * (s * s + s) * 0.5;
    return vec3(cos(e) * cos(azimuth), sin(e), cos(e) * sin(azimuth));
}

fn dome_uv(d: vec3<f32>) -> vec2<f32> {
    let u = fract(atan2(d.z, d.x) / (2.0 * PI) + 1.0);
    let f = (asin(clamp(d.y, -1.0, 1.0)) - E_MIN) / (0.5 * PI - E_MIN);
    let v = (sqrt(1.0 + 8.0 * max(f, 0.0)) - 1.0) * 0.5;
    return vec2(u, select(v, -0.01, f < 0.0));
}

// How far the cumulus reach (the Cloud Distance setting, 40 km on Auto), and the haze's scale:
// the air between washes the far clouds out as it would, however far they are drawn.
fn reach() -> f32 {
    return clouds.layers.y;
}

fn haze_scale() -> f32 {
    return HAZE;
}

fn remap(v: f32, lo: f32, hi: f32) -> f32 {
    return (v - lo) / (hi - lo);
}

// The weather map at `xz` (world, wrapped and drifted): x how much of the sky the cumulus cover
// there, y their type (0 a flat stratocumulus, 1 a towering congestus).
fn weather_at(xz: vec2<f32>) -> vec2<f32> {
    let c = textureSampleLevel(coverage_noise, repeat_sampler, xz / COVERAGE, 0.0).rg;
    // Fields of cloud and wide clear stretches between, tens of kilometres across: the cumulus
    // gather, more and taller where the air rises, instead of an even carpet.
    let field = textureSampleLevel(coverage_noise, repeat_sampler, xz / 98304.0 + vec2(0.29, 0.71), 0.0).g;
    let rising = smoothstep(0.3, 0.7, field);
    let thick = clouds.camera.w;
    let bias = clouds.sun.w;
    // Gaps between the cumulus on a fair day, more of them, and taller, on a cloudier one or in
    // the rain.
    let lo = 0.56 - 0.14 * bias - 0.22 * thick + 0.16 * (1.0 - thick) * (0.5 - rising) * 2.0;
    let cover = saturate(remap(c.x, lo, lo + 0.3));
    let kind = saturate(c.y * 1.5 - 0.55 + 0.45 * rising + 0.25 * bias + 0.6 * thick);
    return vec2(cover, kind);
}

// How much of a cloud of type `kind` there is at height share `h` (0..1) of the slab: a flat
// base, a rounded top, higher for a taller type.
fn profile(h: f32, kind: f32) -> f32 {
    let top = mix(0.35, 1.0, kind);
    // A flat base and a dome: full most of the way up, rounding over near the top (a ramp from
    // 40 % of it cut the clouds into loaves and pyramids).
    let x = saturate(h / top);
    return smoothstep(0.0, 0.1, h) * sqrt(saturate(1.0 - x * x * x));
}

// How much of the finest erosion octave the march's samples get: 1 near the eye, 0 past 5 km,
// where it is finer than a texel (set by `march` for each step).
var<private> near_detail: f32 = 0.0;

// The cloud's density at `p` (world, x and z wrapped and drifted) in the slab at height share
// `h` (0..1), with the weather map's `w`; `fine` adds the high-frequency erosion.
fn density(p: vec3<f32>, h: f32, w: vec2<f32>, fine: bool) -> f32 {
    let shape_profile = profile(h, w.y);
    if shape_profile <= 0.0 || w.x <= 0.0 { return 0.0; }
    // Wind shears the clouds a little with height: their tops lead.
    let q = p + vec3(h * 160.0, 0.0, h * 70.0);
    let s = textureSampleLevel(shape_noise, repeat_sampler,
        vec3(q.x / SHAPE_XZ, q.y / SHAPE_Y, q.z / SHAPE_XZ), 0.0);
    let fbm = s.g * 0.625 + s.b * 0.25 + s.a * 0.125;
    var base = saturate(remap(s.r, fbm - 1.0, 1.0)) * shape_profile;
    base = saturate(remap(base, 1.0 - w.x, 1.0)) * w.x;
    if base <= 0.0 || !fine { return base; }
    // The erosion: a finer Worley noise, its lookup bent by the shape's own (a cheap curl), wispy
    // at the base and billowy higher up.
    let bent = q / DETAIL + (s.gba - 0.5) * 0.35;
    let d = textureSampleLevel(detail_noise, repeat_sampler, bent, 0.0);
    var erosion = d.r * 0.625 + d.g * 0.25 + d.b * 0.125;
    if near_detail > 0.0 {
        // Up close, an octave four times finer (eight-block billows).
        let e = textureSampleLevel(detail_noise, repeat_sampler, bent * 4.0 + 0.31, 0.0);
        erosion = mix(erosion, erosion * 0.7 + (e.r * 0.6 + e.g * 0.4) * 0.3, near_detail);
    }
    let modifier = mix(erosion, 1.0 - erosion, saturate(h * 8.0));
    return saturate(remap(base, modifier * 0.45, 1.0));
}

// The density at `q` wherever it is (0 outside the slab), coarse unless `fine`.
fn density_at(q: vec3<f32>, fine: bool) -> f32 {
    let h = (q.y - BASE) / (TOP - BASE);
    if h <= 0.0 || h >= 1.0 { return 0.0; }
    return density(q, h, weather_at(q.xz), fine);
}

// The Henyey-Greenstein phase (integrating to 1 over the sphere) for the cosine `c` and `g`.
fn hg(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * c, 1e-4), 1.5));
}
// The dual lobe: a strong forward one (the silver lining) and a weak backward one; `spread`
// rounds both for the higher scattering orders.
fn phase(c: f32, spread: f32) -> f32 {
    return mix(hg(c, 0.8 * spread), hg(c, -0.25 * spread), 0.2);
}

// The optical depth from `p` toward `dir`: `steps` steps of `first` metres, each twice the last,
// the first two with the erosion.
fn optical_depth(p: vec3<f32>, dir: vec3<f32>, first: f32, steps: i32, jitter: f32) -> f32 {
    var sum = 0.0;
    var t = 0.0;
    var len = first;
    for (var i = 0; i < steps; i++) {
        let s = len * (0.75 + 0.5 * jitter);
        let q = p + dir * (t + s * 0.5);
        t += s;
        len *= 2.0;
        sum += density_at(q, i < 2) * s;
    }
    return sum * SIGMA;
}

// A noise by pixel and frame (interleaved gradient noise).
fn gradient_noise(pixel: vec2<f32>, frame: f32) -> f32 {
    let q = pixel + 5.588238 * (frame % 64.0);
    return fract(52.9829189 * fract(dot(q, vec2(0.06711056, 0.00583715))));
}

// Where a ray rising `slope` a metre and `bend` a square metre (the Earth's curvature) has risen
// `rise`: the two distances (x > y: never).
fn rise_at(rise: f32, slope: f32, bend: f32) -> vec2<f32> {
    let disc = slope * slope + 4.0 * bend * rise;
    if disc < 0.0 { return vec2(1e30, -1e30); }
    let q = -0.5 * (slope + select(-1.0, 1.0, slope >= 0.0) * sqrt(disc));
    if abs(q) < 1e-20 { return vec2(-1.0, 1.0) * sqrt(max(rise / bend, 0.0)); }
    let a = q / bend;
    let b = -rise / q;
    return vec2(min(a, b), max(a, b));
}

// Light through the slab along the ray from the camera at `origin_y` toward `d`: its start and
// end distances (end < start: none). Under the Earth's curvature (`clouds.light.w`) the slab
// bends down away from the eye, as the ray rises from it by its distance's square: the first
// stretch of the ray between its heights.
fn slab(origin_y: f32, d: vec3<f32>) -> vec2<f32> {
    let bend = clouds.light.w * dot(d.xz, d.xz);
    if bend > 1e-12 {
        let below_top = rise_at(TOP - origin_y, d.y, bend);
        if below_top.x > below_top.y { return vec2(1.0, 0.0); }
        // Under the base inside below_top: the ray is in the slab before or after it.
        let under = rise_at(BASE - origin_y, d.y, bend);
        var range = below_top;
        if under.x <= under.y {
            range = select(vec2(under.y, below_top.y), vec2(below_top.x, under.x), under.x > 0.0);
        }
        return vec2(max(range.x, 0.0), min(range.y, reach()));
    }
    var near = 0.0;
    var far = reach();
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

// The clouds along a ray: rgb the light they send, a their transmittance, and the distance their
// light comes from (its transmittance-weighted average; 0 for none).
struct Marched {
    light: vec4<f32>,
    distance: f32,
}

// The cirrus' density (0..1) at world `xz` (wrapped): fibres along the wind, the sheet's
// coordinates turned to it and stretched along it, the large patches from the weather map, the
// fibres from two lookups of the Worley octaves.
fn cirrus_density(xz: vec2<f32>) -> f32 {
    let amount = clouds.light_color.w;
    if amount <= 0.0 { return 0.0; }
    let p = xz + clouds.drifts.zw;
    let along = normalize(vec2(0.92, 0.38));
    var r = vec2(dot(p, along), dot(p, vec2(-along.y, along.x)));
    let large = textureSampleLevel(coverage_noise, repeat_sampler, r / vec2(40000.0, 14000.0) + 0.37, 0.0);
    // The fibres bend with the large patches.
    r += (large.g - 0.5) * vec2(0.0, 1200.0);
    let a = textureSampleLevel(shape_noise, repeat_sampler, vec3(r.x / 24000.0, 0.41, r.y / 1500.0), 0.0);
    let b = textureSampleLevel(shape_noise, repeat_sampler, vec3(r.x / 9000.0, 0.73, r.y / 600.0), 0.0);
    let fine = a.g * 0.55 + b.b * 0.3 + b.a * 0.15;
    let cover = saturate(remap(large.r, 0.8 - 0.55 * amount, 1.05)) * amount;
    return smoothstep(0.0, 0.6, saturate(remap(fine, 1.0 - cover, 1.0))) * cover;
}

// The altocumulus' density at `p` (world, wrapped): rounded cells a few hundred metres across in
// patches, in a thin lens of a layer.
fn alto_density(p: vec3<f32>) -> f32 {
    let amount = clouds.layers.x;
    let h = (p.y - ALTO_BASE) / (ALTO_TOP - ALTO_BASE);
    if amount <= 0.0 || h <= 0.0 || h >= 1.0 { return 0.0; }
    let q = p + vec3(clouds.drifts.x, 0.0, clouds.drifts.y);
    let patches = textureSampleLevel(coverage_noise, repeat_sampler, q.xz / 26000.0 + vec2(0.61, 0.13), 0.0).g;
    let cover = saturate(remap(patches, 0.75 - 0.55 * amount, 0.95)) * amount;
    if cover <= 0.0 { return 0.0; }
    // The cells: the shape's Worley octaves, small; each rounded top and bottom.
    let s = textureSampleLevel(shape_noise, repeat_sampler, vec3(q.x / 3400.0, q.y / 1800.0, q.z / 3400.0), 0.0);
    let cells = s.b * 0.7 + s.a * 0.3;
    // Each cell a flattened dome: a strong one fills more of the layer's height, a weak one
    // only its middle (straight-sided cells looked like pillars seen edge on, near the horizon).
    let r = (h - 0.45) / 0.55;
    let base = saturate(remap(cells - r * r * 0.7, 1.0 - cover, 1.0));
    let d = textureSampleLevel(detail_noise, repeat_sampler, q / 300.0, 0.0);
    let eroded = saturate(remap(base, (d.r * 0.6 + d.g * 0.4) * 0.35, 1.0));
    // No faint veil: below a floor the cell is clear air, so its edge is the cell's round
    // outline and not the patch's (whose bilinear texels drew straight-edged sheets).
    return smoothstep(0.15, 0.4, eroded) * eroded * 0.6;
}

// The altocumulus along the camera-relative ray from `origin` toward `d`, before what is behind.
fn altocumulus(origin: vec3<f32>, d: vec3<f32>, noise: f32, behind: Marched) -> Marched {
    let origin_y = clouds.camera.y + origin.y;
    if clouds.layers.x <= 0.0 || origin_y >= ALTO_BASE { return behind; }
    let bend = clouds.light.w * dot(d.xz, d.xz);
    var range = vec2(0.0);
    if bend > 1e-12 {
        range = vec2(rise_at(ALTO_BASE - origin_y, d.y, bend).y, rise_at(ALTO_TOP - origin_y, d.y, bend).y);
    } else if d.y > 1e-4 {
        range = vec2(ALTO_BASE - origin_y, ALTO_TOP - origin_y) / d.y;
    } else {
        return behind;
    }
    range.y = min(range.y, range.x + 2500.0);
    let cut = max(70000.0, reach());
    if range.x <= 0.0 || range.x > cut { return behind; }
    let start = vec3(clouds.camera.x + origin.x, origin_y, clouds.camera.z + origin.z);
    let l = clouds.light.xyz;
    let c = dot(d, l);
    let up_sky = sky_at(vec3(0.0, 1.0, 0.0)) * 0.7 + sky_at(normalize(vec3(d.x, 0.3, d.z))) * 0.3;
    // Steps of about 100 blocks: a ray grazing the layer crosses many cells, and a few long
    // steps hit them only now and then (a grainy veil between the clouds).
    let steps = i32(clamp(ceil((range.y - range.x) / 100.0), 8.0, f32(ALTO_STEPS)));
    let step_length = (range.y - range.x) / f32(steps);
    var light = vec3(0.0);
    var transmittance = 1.0;
    for (var i = 0; i < steps; i++) {
        // Each step jittered on its own, so no slices line up across neighbouring pixels.
        let jitter = fract(noise + f32(i) * 0.618034);
        let t = range.x + (f32(i) + jitter) * step_length;
        let q = start + d * t + vec3(0.0, bend * t * t, 0.0);
        let rho = alto_density(q);
        if rho <= 0.003 { continue; }
        let od = rho * SIGMA * step_length;
        let step_transmittance = exp(-od);
        // Thin: two steps toward the light, the rest of the layer above toward the sky.
        let tau = (alto_density(q + l * 40.0) * 80.0 + alto_density(q + l * 160.0) * 160.0) * SIGMA;
        let above = saturate((ALTO_TOP - q.y) / (ALTO_TOP - ALTO_BASE));
        let tau_sky = rho * SIGMA * above * (ALTO_TOP - ALTO_BASE) * 0.5;
        let sun = (exp(-tau) * phase(c, 1.0) + 0.5 * exp(-tau * 0.4) * phase(c, 0.6)) * PI * SUN_GAIN;
        let scattered = clouds.light_color.rgb * sun + up_sky * (AMBIENT_GAIN * (exp(-tau_sky) * 0.7 + 0.3));
        light += transmittance * (1.0 - step_transmittance) * scattered * ALBEDO;
        transmittance *= step_transmittance;
        if transmittance < 0.02 { break; }
    }
    if transmittance >= 0.999 { return behind; }
    let distance = range.x;
    let haze = 1.0 - exp(-distance / (haze_scale() * 1.8));
    let gone = smoothstep(cut * 0.57, cut, distance);
    let opacity = (1.0 - transmittance) * (1.0 - gone);
    let tinted = mix(light * (1.0 - gone), sky_at(d) * opacity, haze);
    var out: Marched;
    out.light = vec4(tinted + behind.light.rgb * (1.0 - opacity), (1.0 - opacity) * behind.light.a);
    out.distance = distance;
    return out;
}

// The cirrus sheet along the ray from the camera at `origin_y` toward `d`, behind the cumulus.
fn cirrus(origin: vec3<f32>, d: vec3<f32>) -> Marched {
    var none: Marched;
    none.light = vec4(0.0, 0.0, 0.0, 1.0);
    none.distance = 0.0;
    let amount = clouds.light_color.w;
    let origin_y = clouds.camera.y + origin.y;
    if amount <= 0.0 || origin_y >= CIRRUS { return none; }
    let bend = clouds.light.w * dot(d.xz, d.xz);
    var t = 1e30;
    if bend > 1e-12 {
        t = rise_at(CIRRUS - origin_y, d.y, bend).y;
    } else if d.y > 1e-4 {
        t = (CIRRUS - origin_y) / d.y;
    }
    let cut = max(90000.0, reach() * 1.4);
    if t <= 0.0 || t > cut { return none; }
    let dens = cirrus_density(vec2(clouds.camera.x + origin.x, clouds.camera.z + origin.z) + d.xz * t);
    if dens <= 0.0 { return none; }
    // A thin sheet: its optical depth along the ray grows as the ray grazes it.
    let slant = clamp(t / max(CIRRUS - origin_y, 1.0), 1.0, 6.0);
    let tau = dens * 0.5 * slant;
    let opacity = 1.0 - exp(-tau);
    // Ice crystals: a sharp forward lobe, the sky's light from all round.
    let c = dot(d, clouds.light.xyz);
    let sun = clouds.light_color.rgb * (PI * SUN_GAIN * mix(hg(c, 0.7), hg(c, -0.15), 0.35));
    let sky = sky_at(vec3(0.0, 1.0, 0.0)) * 0.7 + sky_at(normalize(vec3(d.x, 0.2, d.z))) * 0.3;
    var light = (sun + sky) * ALBEDO * opacity;
    // The air between, much more of it toward the horizon.
    let haze = 1.0 - exp(-t / (haze_scale() * 2.5));
    light = mix(light, sky_at(d) * opacity, haze);
    let gone = smoothstep(cut * 0.61, cut, t);
    var out: Marched;
    out.light = vec4(light * (1.0 - gone), 1.0 - opacity * (1.0 - gone));
    out.distance = t;
    return out;
}

// The cumulus along the camera-relative ray from `origin` toward `d`, then the cirrus behind.
fn march(origin: vec3<f32>, d: vec3<f32>, noise: f32) -> Marched {
    var out: Marched;
    out.light = vec4(0.0, 0.0, 0.0, 1.0);
    out.distance = 0.0;
    let range = slab(clouds.camera.y + origin.y, d);
    let behind = altocumulus(origin, d, noise, cirrus(origin, d));
    if range.y <= range.x { return behind; }
    let start = vec3(clouds.camera.x + origin.x + clouds.wind.x, clouds.camera.y + origin.y,
        clouds.camera.z + origin.z + clouds.wind.y);
    let l = clouds.light.xyz;
    let c = dot(d, l);
    let sun_color = clouds.light_color.rgb;
    // The sky's light from above (its zenith and the band toward the ray), and the ground's from
    // below: a little green and brown, lit by the sun.
    let up_sky = sky_at(vec3(0.0, 1.0, 0.0)) * 0.6 + sky_at(normalize(vec3(d.x, 0.35, d.z))) * 0.4;
    let ground = sun_color * max(l.y, 0.0) * vec3(0.18, 0.19, 0.13);
    let bend = clouds.light.w * dot(d.xz, d.xz);
    var light = vec3(0.0);
    var transmittance = 1.0;
    var weighted = 0.0;
    var weight = 0.0;
    var optical = 0.0;
    var fine_mode = false;
    var empty = 0;
    var t = range.x;
    // The first step's jitter (the accumulation averages it out).
    var started = false;
    for (var i = 0; i < ITERATIONS; i++) {
        // A step about 1 % of the distance in the cloud, three times that through clear air.
        let fine_step = clamp(25.0 + t * 0.008, 25.0, 320.0);
        let coarse_step = fine_step * 3.0;
        if !started {
            t += noise * coarse_step;
            started = true;
        }
        if t > range.y { break; }
        // The point's height over the curved Earth: the ray's, plus how far it bends away under.
        let q = start + d * t + vec3(0.0, bend * t * t, 0.0);
        let h = (q.y - BASE) / (TOP - BASE);
        let w = weather_at(q.xz);
        if w.x <= 0.0 || h <= 0.0 || h >= 1.0 {
            fine_mode = false;
            t += coarse_step;
            continue;
        }
        if !fine_mode {
            if density(q, h, w, false) > 0.0 {
                // Into a cloud: back a step, and on in short ones.
                fine_mode = true;
                empty = 0;
                t = max(t - coarse_step * 0.7, range.x);
            } else {
                t += coarse_step;
            }
            continue;
        }
        near_detail = 1.0 - smoothstep(2000.0, 5000.0, t);
        let raw = density(q, h, w, true);
        near_detail = 0.0;
        // No dust: the thinnest wisps fade to clear air (a scattering of them, hit by one sample
        // in a few, drew grainy veils darker than the sky between the clouds).
        let rho = raw * smoothstep(0.0, 0.06, raw);
        if rho <= 0.003 {
            empty++;
            if empty >= 6 { fine_mode = false; }
            t += fine_step;
            continue;
        }
        empty = 0;
        let od = rho * SIGMA * fine_step;
        let step_transmittance = exp(-od);
        // The light reaching the sample from the sun, the sky and the ground.
        let tau_light = optical_depth(q, l, 30.0, LIGHT_STEPS, noise);
        let tau_sky = optical_depth(q, vec3(0.0, 1.0, 0.0), 60.0, 2, noise);
        let tau_ground = optical_depth(q, vec3(0.0, -1.0, 0.0), 80.0, 1, noise);
        // Octaves of multiple scattering: each weaker, absorbed less, rounder.
        var sun = 0.0;
        var ambient = 0.0;
        var below = 0.0;
        var a = 1.0;
        var b = 1.0;
        var spread = 1.0;
        for (var k = 0; k < 4; k++) {
            sun += a * exp(-tau_light * b) * phase(c, spread);
            ambient += a * exp(-tau_sky * b);
            below += a * exp(-tau_ground * b);
            a *= 0.5;
            b *= 0.4;
            spread *= 0.6;
        }
        // The powder effect: a thin cloud's edge facing away from the sun is darker than its
        // inside (little light scattered toward the eye there yet).
        let powder = mix(1.0, 1.0 - exp(-2.0 * (optical + od) - 0.5 * rho * SIGMA * 300.0),
            0.5 * saturate(1.0 - c));
        // Isotropic light from a hemisphere (the sky, the ground) scatters half its radiance.
        let scattered = sun_color * (sun * powder * PI * SUN_GAIN)
            + up_sky * (ambient * 0.5 * AMBIENT_GAIN)
            + ground * (below * 0.5 * AMBIENT_GAIN);
        let absorbed = transmittance * (1.0 - step_transmittance);
        light += absorbed * scattered * ALBEDO;
        weighted += absorbed * t;
        weight += absorbed;
        optical += od;
        transmittance *= step_transmittance;
        t += fine_step;
        if transmittance < 0.01 { break; }
    }
    if weight <= 0.0 { return behind; }
    // Aerial perspective: toward the horizon a cloud takes the sky's colour (the air between)
    // and fades out.
    let distance = weighted / weight;
    let haze = 1.0 - exp(-distance / haze_scale());
    let gone = smoothstep(reach() * 0.65, reach(), distance);
    let opacity = (1.0 - transmittance) * (1.0 - gone);
    let tinted = mix(light * (1.0 - gone), sky_at(d) * opacity, haze);
    // The cirrus behind, through what the cumulus let pass.
    out.light = vec4(tinted + behind.light.rgb * (1.0 - opacity), (1.0 - opacity) * behind.light.a);
    out.distance = distance;
    return out;
}

// ---- The dome ----

// The dome's texel this frame marches in each square of the checkerboard, and the march target's
// texel `id`'s place in the dome.
fn dome_texel(id: vec2<u32>) -> vec2<u32> {
    let n = u32(clouds.jitter.z);
    let k = u32(clouds.jitter.w);
    return id * n + vec2(k % n, k / n);
}

@compute @workgroup_size(8, 8) fn cs_dome_march(@builtin(global_invocation_id) id: vec3<u32>) {
    let low = vec2<u32>(clouds.size.zw);
    if id.x >= low.x || id.y >= low.y { return; }
    let texel = dome_texel(id.xy);
    let uv = (vec2<f32>(texel) + 0.5 + clouds.jitter.xy * 0.5) / clouds.size.xy;
    let d = dome_dir(uv);
    let noise = gradient_noise(vec2<f32>(texel), clouds.wind.z);
    let m = march(vec3(0.0), d, noise);
    textureStore(raw_out, vec2<i32>(id.xy), m.light);
    textureStore(raw_distance_out, vec2<i32>(id.xy), vec4(m.distance, 0.0, 0.0, 0.0));
}

// The dome's resolve: this frame's marched texels into the history, the rest reprojected.
@group(0) @binding(16) var dome_raw: texture_2d<f32>;
@group(0) @binding(17) var dome_raw_distance: texture_2d<f32>;
@group(0) @binding(18) var dome_history: texture_2d<f32>;
// x the distance (km; 0 for none), y how many marches the history holds.
@group(0) @binding(19) var dome_history_data: texture_2d<f32>;
@group(0) @binding(20) var dome_sampler: sampler;
@group(0) @binding(21) var dome_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(22) var dome_data_out: texture_storage_2d<rgba16float, write>;

// The most marches a texel's history averages.
const DOME_HISTORY: f32 = 10.0;

@compute @workgroup_size(8, 8) fn cs_dome_resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<u32>(clouds.size.xy);
    if id.x >= size.x || id.y >= size.y { return; }
    let p = vec2<i32>(id.xy);
    let n = i32(clouds.jitter.z);
    let k = i32(clouds.jitter.w);
    let low = vec2<i32>(clouds.size.zw);
    let cell = min(p / n, low - 1);
    let fresh = all(p % n == vec2(k % n, k / n));
    let current = textureLoad(dome_raw, cell, 0);
    let current_distance = textureLoad(dome_raw_distance, cell, 0).r * 0.001;
    // The neighbours marched this frame bound what the history may hold (no ghosts).
    var lo = current;
    var hi = current;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let c = textureLoad(dome_raw, clamp(cell + vec2(x, y), vec2(0), low - 1), 0);
            lo = min(lo, c);
            hi = max(hi, c);
        }
    }
    var out = current;
    var out_distance = current_distance;
    // A texel only filled from its square's march (no history yet) takes its own march whole
    // when its turn comes.
    var age = select(0.0, 1.0, fresh);
    if clouds.motion.w > 0.5 {
        // The texel's direction seen from the history's camera, at the distance it held.
        let uv = (vec2<f32>(p) + 0.5) / clouds.size.xy;
        let d = dome_dir(uv);
        let held = textureLoad(dome_history_data, p, 0).r;
        let distance = select(6.0, held, held > 0.0) * 1000.0;
        let moved = normalize(d * distance + clouds.motion.xyz);
        // As the difference of two mappings, so the trigonometry's rounding (some drivers' asin
        // is off by a tenth of a row) cancels instead of sliding the history down each frame.
        var shift = dome_uv(moved) - dome_uv(d);
        shift.x -= round(shift.x);
        let at = uv + shift;
        if at.y >= 0.0 && at.y <= 1.0 {
            var past = textureSampleLevel(dome_history, dome_sampler, at, 0.0);
            let data = textureSampleLevel(dome_history_data, dome_sampler, at, 0.0);
            // The neighbours bound the history only as far as it moved: a texel's own history,
            // still, is better than its square's few samples this frame (which miss the small
            // altocumulus cells and clamped them away, in grainy veils and streaks).
            let moved_by = shift * clouds.size.xy;
            let trust = saturate(length(moved_by) - 0.5);
            let margin = (hi - lo) * 0.3 + 0.02;
            let bounded = mix(past, clamp(past, lo - margin, hi + margin), trust);
            age = data.g;
            // A history the neighbours disagree with starts again (mostly).
            if any(abs(bounded - past) > vec4(0.05)) { age = min(age, 2.0); }
            past = bounded;
            if fresh {
                age = min(age + 1.0, DOME_HISTORY);
                out = mix(past, current, 1.0 / age);
                out_distance = mix(data.r, current_distance, 1.0 / age);
            } else {
                out = past;
                out_distance = data.r;
            }
        }
    }
    textureStore(dome_out, p, out);
    textureStore(dome_data_out, p, vec4(out_distance, age, 0.0, 0.0));
}

// A view's look at the dome: wherever its depth shows the sky.
@group(0) @binding(23) var dome_light: texture_2d<f32>;

@fragment fn fs_dome_composite(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(pos.xy);
    if textureLoad(depth, at, 0).r > 0.0 { discard; }
    let d = ray_at(pos.xy / clouds.size.xy)[1];
    let uv = dome_uv(d);
    if uv.y < 0.0 { discard; }
    let c = textureSampleLevel(dome_light, dome_sampler, uv, 0.0);
    return vec4(max(c.rgb, vec3(0.0)), clamp(c.a, 0.0, 1.0));
}

// ---- The shadow map ----

// The light's transmittance through the clouds by where its ray crosses the cumulus' base: a
// map wrapping every SHADOW_SIDE blocks of world x and z (terrain.wgsl `rt_cloud_shadow` reads it),
// one texel in each checkerboard square a frame (`dome_texel`), the march's output its texture.
const SHADOW_SIDE: f32 = 4096.0;

fn wrap_near(x: vec2<f32>, side: f32) -> vec2<f32> {
    return x - side * floor(x / side + 0.5);
}

@compute @workgroup_size(8, 8) fn cs_shadow(@builtin(global_invocation_id) id: vec3<u32>) {
    let low = vec2<u32>(clouds.size.zw);
    if id.x >= low.x || id.y >= low.y { return; }
    let texel = dome_texel(id.xy);
    let l = clouds.light.xyz;
    var out = vec4(1.0, 0.0, 0.0, 1.0);
    if l.y > 0.03 {
        let camera = clouds.camera.xyz;
        let across = l.xz / l.y;
        // The texel stands for the place nearest to where the camera's own column meets the base.
        let center = camera.xz + across * (BASE - camera.y);
        let q = center + wrap_near((vec2<f32>(texel) + 0.5) * clouds.layers.z - center, SHADOW_SIDE);
        let length = min((TOP - BASE) / l.y, 12000.0);
        let s = length / 20.0;
        let wind = vec3(clouds.wind.x, 0.0, clouds.wind.y);
        var tau = 0.0;
        for (var i = 0; i < 20; i++) {
            let p = vec3(q.x, BASE, q.y) + l * (s * (f32(i) + 0.5)) + wind;
            tau += density_at(p, true) * s;
        }
        tau *= SIGMA;
        // The layers above, each at its middle.
        let mid = 0.5 * (ALTO_BASE + ALTO_TOP);
        let a = q + across * (mid - BASE);
        tau += alto_density(vec3(a.x, mid, a.y)) * SIGMA * min((ALTO_TOP - ALTO_BASE) / l.y, 3000.0);
        tau += cirrus_density(q + across * (CIRRUS - BASE)) * 0.5 * min(1.0 / l.y, 6.0);
        // A thick cloud still lets a little light through, scattered forward.
        var t = mix(exp(-tau), exp(-tau * 0.25), 0.25);
        // The shadows fade as the light reaches the horizon (they would stretch for kilometres).
        t = mix(1.0, t, smoothstep(0.03, 0.12, l.y));
        out = vec4(t, tau, 0.0, 1.0);
    }
    textureStore(raw_out, vec2<i32>(texel), out);
}

// ---- Per view ----

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
            if textureLoad(depth, at, 0).r <= 0.0 { open = true; }
        }
    }
    if !open {
        textureStore(raw_out, vec2<i32>(id.xy), vec4(0.0, 0.0, 0.0, 1.0));
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5 + clouds.jitter.xy) * scale / clouds.size.xy;
    let ray = ray_at(uv);
    let noise = gradient_noise(vec2<f32>(id.xy), clouds.wind.z);
    textureStore(raw_out, vec2<i32>(id.xy), march(ray[0], ray[1], noise).light);
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
    if textureLoad(depth, at, 0).r > 0.0 { discard; }
    let uv = pos.xy / (clouds.size.zw * clouds.jitter.z);
    let c = textureSampleLevel(cloud_light, cloud_sampler, uv, 0.0);
    return vec4(max(c.rgb, vec3(0.0)), clamp(c.a, 0.0, 1.0));
}
