// The Eye of Aldhani (gfx/aldhani.rs, HORIZON_METEORS=aldhani, the control socket's `aldhani`): a
// storm of thousands of meteors at once out of one radiant on the horizon, a few minutes long. Each
// frame this pass makes the field from the event's clock, the same for every view (the screen and
// both eyes): every meteor burning, its streak's two ends and colour for the sky (terrain.wgsl
// `aldhani_streaks`), binned into tiles of the sky so a pixel only looks at those crossing its own,
// and its light toward the terrain with a running sum of their light (`cdf`), for the rays to draw
// one by its share without a loop over all (shadow_rays.wgsl `aldhani_light`).
// ALDHANI-BEGIN: the Eye of Aldhani's field, the same text in gfx/aldhani.wgsl (which makes it),
// terrain.wgsl (the sky draws it) and gfx/shadow_rays.wgsl (the terrain is lit by it); a test keeps
// them equal.
// The slots a frame looks back over (one meteor at most each), the sky's tiles a side (a square
// folded over the upper half of the sky, `aldhani_tile`) and the streaks a tile holds at most.
const ALDHANI_METEORS: u32 = 32768u;
const ALDHANI_TILES: u32 = 64u;
const ALDHANI_TILE_CAP: u32 = 256u;
// How far from its line a streak's light reaches at most, in radians (its glow fades to nothing
// there): a streak is binned into every tile within it.
const ALDHANI_REACH: f32 = 0.012;
// The event's seconds: the storm builds over a minute, holds, and dies out over the last minute.
const ALDHANI_EVENT: f32 = 300.0;
// Where the meteors come from: a fixed direction of the world, due south eight degrees under the
// horizon, far behind it, so the streaks rise out of the haze beyond the mountains and fan out
// upward and outward over the whole sky.
const ALDHANI_RADIANT: vec3<f32> = vec3<f32>(0.0, -0.1392, 0.99027);
// The light the field gives a face turned to it at the storm's height, in full moons (the moon's
// light overhead, `rt.held.z`), and the glow of the sky around the radiant at its brightest, in
// paper whites (radiance; Lambert's 1/pi in the units, as the sky table's).
const ALDHANI_MOONS: f32 = 16.0;
const ALDHANI_GLOW: f32 = 0.2;
// A meteor of the field: toward its streak's tail (xyz) and its core's width in pixels (w); toward
// its head (xyz) and its seed (w, for the sparkles); the light of its core at the head (rgb, paper
// whites) and its head's glow (w); the light it gives the terrain (rgb, luminance 1) and its share
// (w, the cdf's weight; 0: not burning).
struct AldhaniMeteor {
    tail: vec4<f32>,
    head: vec4<f32>,
    colour: vec4<f32>,
    light: vec4<f32>,
}
// x: the meteors' shares summed, y: the meteors burning, z: those drawn in the sky; the shares'
// running sum (`cdf[i]` the sum up to meteor i); each streak's great circle (its normal, xyz, 0
// for a point, and its angle, w), for the sky to pass a streak by from one read; and the meteors.
struct AldhaniField {
    total: vec4<f32>,
    cdf: array<f32, ALDHANI_METEORS>,
    lines: array<vec4<f32>, ALDHANI_METEORS>,
    meteors: array<AldhaniMeteor, ALDHANI_METEORS>,
}
// A hash of `n` to [0, 1) (a PCG round, as terrain.wgsl's `meteor_hash`).
fn aldhani_rand(n: u32) -> f32 {
    let state = n * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return f32(((word >> 22u) ^ word) >> 8u) / 16777216.0;
}
// How strong the storm is `t` seconds into the event (0 to 1; 0 before and after it).
fn aldhani_envelope(t: f32) -> f32 {
    return smoothstep(0.0, 60.0, t) * (1.0 - smoothstep(ALDHANI_EVENT - 60.0, ALDHANI_EVENT, t));
}
// The sky tile direction `d` falls in: the upper half of the sky folded flat (an octahedron's top,
// turned to a square), ALDHANI_TILES a side, the horizon on the square's edges.
fn aldhani_tile(d: vec3<f32>) -> u32 {
    let up = vec3(d.x, max(d.y, 0.0), d.z);
    let p = up.xz / (abs(up.x) + up.y + abs(up.z) + 1e-6);
    let q = vec2(p.x + p.y, p.x - p.y) * 0.5 + 0.5;
    let c = min(vec2<u32>(max(q, vec2(0.0)) * f32(ALDHANI_TILES)), vec2(ALDHANI_TILES - 1u));
    return c.y * ALDHANI_TILES + c.x;
}
// The sky's own light toward `d` during the event (`t` seconds in), in paper whites: the glow the
// thousands of meteors too far and faint to tell apart make, a pale cyan sky brightest toward the
// radiant and pinkish grey on its west side, and the eye itself, a bright glow on the horizon
// around the radiant, whitish cyan on its east side and pink-white on its west. ALDHANI_PALETTE:
// the colours are the show's (Andor, "The Eye"), no spectrum is known for it.
fn aldhani_glow(d: vec3<f32>, t: f32) -> vec3<f32> {
    let strength = aldhani_envelope(t);
    if strength <= 0.0 { return vec3(0.0); }
    let c = clamp(dot(d, ALDHANI_RADIANT), -1.0, 1.0);
    let theta = acos(c);
    let up = max(d.y, 0.0);
    let east = d.x / max(sin(theta), 1e-3);
    // The sky: the faint meteors everywhere, thicker toward the radiant's half of the sky.
    let tint = mix(vec3(0.2, 0.95, 0.8), vec3(1.0, 0.55, 0.75), 0.75 * smoothstep(-0.2, 0.9, -east));
    let sky = tint * (3.5 * (0.35 + 0.65 * (0.5 + 0.5 * c)) * (0.55 + 0.45 * exp(-up * 2.0)));
    // The eye: a few degrees of white-hot core in a wide glow, its sides two colours.
    let side = mix(vec3(0.1, 0.95, 0.8), vec3(1.0, 0.5, 0.78), smoothstep(-0.7, 0.7, -east));
    let core = exp(-theta * theta / (2.0 * 0.08 * 0.08));
    let wide = exp(-theta / 0.25) * (0.4 + 0.6 * exp(-up / 0.15));
    let eye = side * (8.0 * wide) + mix(side, vec3(1.0), 0.6) * (60.0 * core);
    // Under the horizon the ground hides it (the rays see the ground there).
    let above = smoothstep(-0.06, 0.0, d.y);
    return (sky + eye) * (ALDHANI_GLOW * strength * above);
}
// ALDHANI-END
struct Params {
    // x: the seconds since the event began, y: a slot's seconds.
    time: vec4<f32>,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> field: AldhaniField;
// Each tile's count, then its list of meteors (ALDHANI_TILE_CAP a tile).
@group(0) @binding(2) var<storage, read_write> tiles: array<atomic<u32>>;
// The colours of the show's frame, in light (ALDHANI_PALETTE), by patches of sky rather than
// meteor by meteor (neighbours there differ by some 10 degrees of hue), richer than the frame:
// the cold ones from green through turquoise and cyan to blue, drifting slowly across the sky,
// and the warm ones, gold, orange and rose, a few everywhere and many on the west (the right,
// facing the radiant). `around` and `theta` are where it lights up about the radiant (0 east, pi
// west), `u` and `v` its own draws.
fn palette(around: f32, theta: f32, u: f32, v: f32) -> vec3<f32> {
    let drift = 0.5 + 0.35 * sin(1.9 * around + 4.0 * theta + 1.0) + 0.25 * sin(3.7 * around - 6.0 * theta);
    let h = clamp(drift + 0.2 * (u - 0.5), 0.0, 1.0);
    let green = vec3(0.25, 1.0, 0.3);
    let turquoise = vec3(0.0, 1.0, 0.6);
    let cyan = vec3(0.0, 0.75, 1.0);
    let blue = vec3(0.25, 0.4, 1.0);
    var cold = mix(green, turquoise, smoothstep(0.0, 0.3, h));
    cold = mix(cold, cyan, smoothstep(0.35, 0.65, h));
    cold = mix(cold, blue, smoothstep(0.7, 1.0, h));
    let west = smoothstep(0.2, 1.0, -cos(around));
    if v < 0.08 + 0.5 * west * west {
        let warm = 0.5 + 0.5 * sin(6.0 * theta + 2.0 * around);
        let gold = vec3(1.0, 0.72, 0.12);
        let orange = vec3(1.0, 0.35, 0.1);
        let rose = vec3(1.0, 0.25, 0.6);
        return mix(mix(gold, orange, smoothstep(0.0, 0.5, warm)), rose, smoothstep(0.5, 1.0, warm));
    }
    return cold;
}
const EARTH_KM: f32 = 6371.0;
// The meteor of slot `slot` (slots of `period` seconds from the event's start), `t` seconds into
// the event. A meteoroid falls along a straight line away from the radiant, as a shower's do (the
// perspective of parallel lines makes the fan: short near the radiant, long overhead), lit high,
// at 130 to 160 km (the Eye's are fast, large ones), its speed 35 to 70 km/s, its path 20 to 80 km; it lights up where it is, its glow
// stays behind its head for a fifth to a half second (its streak), and its light follows the
// classical single-body curve along the path (a rise to a peak late on, an abrupt end), its
// brightness a population index's (many faint, a few bright). Many start near the radiant (the
// eye), more than a thousand km away along the horizon: short, foreshortened, dimmed by the
// distance and the air's long path, the lowest lost in the haze over the horizon; overhead they
// are near, long and bright.
fn aldhani_make(slot: u32, t: f32, period: f32) -> AldhaniMeteor {
    var o = AldhaniMeteor(vec4(0.0), vec4(0.0), vec4(0.0), vec4(0.0));
    let seed = slot * 16u + 5u;
    let start = (f32(slot) + aldhani_rand(seed)) * period;
    // As many as the storm's strength at the meteor's own start says.
    if aldhani_rand(seed + 1u) > aldhani_envelope(start) { return o; }
    let age = t - start;
    let speed = 35.0 + 35.0 * aldhani_rand(seed + 2u);
    let path = 20.0 + 60.0 * aldhani_rand(seed + 3u);
    let lasts = path / speed;
    let persist = 0.2 + 0.3 * aldhani_rand(seed + 4u);
    if age < 0.0 || age > lasts + persist { return o; }
    // Where it lights up: its angle from the radiant, most near it, and its side, above the horizon.
    let theta = radians(8.5 + 81.5 * pow(aldhani_rand(seed + 5u), 2.4));
    let around = 3.14159265 * aldhani_rand(seed + 6u);
    let east = normalize(cross(vec3(0.0, 1.0, 0.0), ALDHANI_RADIANT));
    let lift = cross(ALDHANI_RADIANT, east);
    let a = normalize(cos(theta) * ALDHANI_RADIANT + sin(theta) * (cos(around) * east + sin(around) * lift));
    if a.y < 0.01 { return o; }
    // Its distance to the height it lights up at, over the round Earth.
    let height = 130.0 + 30.0 * aldhani_rand(seed + 7u);
    let r = EARTH_KM + height;
    let reach = sqrt(r * r - EARTH_KM * EARTH_KM * (1.0 - a.y * a.y)) - EARTH_KM * a.y;
    let w = -ALDHANI_RADIANT;
    // Its head (where it burns now, or where it ended) and its glow's tail behind it.
    let head_t = min(age, lasts);
    let tail_t = clamp(age - persist, 0.0, head_t);
    let head = normalize(reach * a + (speed * head_t) * w);
    let tail = normalize(reach * a + (speed * tail_t) * w);
    // Its light along its path, and its glow fading once it has burnt out.
    let x = head_t / lasts;
    let peak = 0.55 + 0.3 * aldhani_rand(seed + 8u);
    let rise = pow(x / peak, 1.6);
    let fall = pow(max(1.0 - (x - peak) / (1.0 - peak), 0.0), 0.8);
    let curve = select(rise, fall, x > peak);
    let fade = exp(-max(age - lasts, 0.0) / (0.3 * persist));
    // Its brightness from the population index 2.2 (times the faintest), at most a fireball's.
    let power = min(pow(max(aldhani_rand(seed + 9u), 1e-4), -1.17), 120.0);
    // Through the air toward it (grey: the show's colours stay): a low one is dimmed by its
    // airmass, and the haze over the horizon swallows the lowest streaks.
    let air = exp(-0.15 / max(head.y + 0.025, 0.03)) * smoothstep(-0.01, 0.06, head.y);
    let colour = palette(around, theta, aldhani_rand(seed + 10u), aldhani_rand(seed + 11u));
    // The sky's streak: faint ones still seen (the light compressed), dimmer far away.
    let shown = pow(power, 0.6) * (0.3 + 0.7 * curve) * fade * air * pow(160.0 / reach, 0.9);
    o.tail = vec4(tail, 0.8 + 0.2 * log2(power + 1.0));
    o.head = vec4(head, f32(seed & 65535u));
    o.colour = vec4(colour * (1.6 * shown), clamp(power / 30.0, 0.0, 1.0) * curve * fade * air);
    // Its light on the terrain: by its power and the inverse square of its distance; its colour
    // half the sky's teal (thousands of them light the ground together, the teal ones the most).
    let lit = mix(colour, vec3(0.0, 1.0, 0.55), 0.4);
    o.light = vec4(lit / dot(lit, vec3(0.2126, 0.7152, 0.0722)), power * curve * fade * air * (160.0 / reach) * (160.0 / reach));
    return o;
}
// Meteor `k` in tile `tile`, unless `seen` (the `count` tiles it is in already) holds it.
fn bin_into(k: u32, tile: u32, seen: ptr<function, array<u32, 48>>, count: ptr<function, u32>) {
    for (var s = 0u; s < *count; s++) {
        if (*seen)[s] == tile { return; }
    }
    if *count >= 48u { return; }
    (*seen)[*count] = tile;
    *count += 1u;
    let n = atomicAdd(&tiles[tile], 1u);
    if n < ALDHANI_TILE_CAP {
        atomicStore(&tiles[ALDHANI_TILES * ALDHANI_TILES + tile * ALDHANI_TILE_CAP + n], k);
    }
}
// Meteor `k` in every tile its streak's light reaches: along it and on either side by
// ALDHANI_REACH, and as far past its two ends, each tile once.
fn bin(k: u32, m: AldhaniMeteor) {
    let a = m.tail.xyz;
    let b = m.head.xyz;
    let span = acos(clamp(dot(a, b), -1.0, 1.0));
    let normal = cross(a, b);
    let side = select(normalize(cross(b, vec3(0.0, 1.0, 0.0))), normalize(normal), length(normal) > 1e-6);
    let ahead = cross(side, b);
    let steps = min(u32(ceil(span / 0.015)) + 1u, 48u);
    var seen: array<u32, 48>;
    var count = 0u;
    for (var i = 0u; i <= steps; i++) {
        let along = normalize(mix(a, b, f32(i) / f32(steps)));
        for (var j = 0; j < 3; j++) {
            bin_into(k, aldhani_tile(normalize(along + side * (f32(j - 1) * ALDHANI_REACH))), &seen, &count);
        }
    }
    // Past the ends: the tail behind, the head ahead.
    for (var j = 0; j < 3; j++) {
        let off = side * (f32(j - 1) * ALDHANI_REACH);
        bin_into(k, aldhani_tile(normalize(b + ahead * ALDHANI_REACH + off)), &seen, &count);
        bin_into(k, aldhani_tile(normalize(a - ahead * ALDHANI_REACH + off)), &seen, &count);
    }
}
// The field: thread k the slot k before the latest, with no state from frame to frame.
@compute @workgroup_size(256) fn cs_aldhani(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.x;
    if k >= ALDHANI_METEORS { return; }
    var m = AldhaniMeteor(vec4(0.0), vec4(0.0), vec4(0.0), vec4(0.0));
    let t = params.time.x;
    let period = params.time.y;
    let slot = i32(floor(t / period)) - i32(k);
    if t >= 0.0 && period > 0.0 && slot >= 0 {
        m = aldhani_make(u32(slot), t, period);
    }
    field.meteors[k] = m;
    let normal = cross(m.tail.xyz, m.head.xyz);
    let s = length(normal);
    field.lines[k] = vec4(select(vec3(0.0), normal / s, s > 1e-7), atan2(s, dot(m.tail.xyz, m.head.xyz)));
    if max(m.colour.r, max(m.colour.g, m.colour.b)) > 0.0 { bin(k, m); }
}
// The shares' running sum (one workgroup: 128 meteors a thread, then a scan of the threads' sums)
// and the counts.
var<workgroup> sums: array<f32, 256>;
var<workgroup> burning: atomic<u32>;
var<workgroup> drawn: atomic<u32>;
@compute @workgroup_size(256) fn cs_aldhani_sum(@builtin(local_invocation_index) i: u32) {
    let per = ALDHANI_METEORS / 256u;
    var sum = 0.0;
    var lit = 0u;
    var shown = 0u;
    for (var j = 0u; j < per; j++) {
        let m = field.meteors[i * per + j];
        sum += m.light.w;
        lit += select(0u, 1u, m.light.w > 0.0);
        shown += select(0u, 1u, max(m.colour.r, max(m.colour.g, m.colour.b)) > 0.0);
    }
    atomicAdd(&burning, lit);
    atomicAdd(&drawn, shown);
    sums[i] = sum;
    workgroupBarrier();
    for (var step = 1u; step < 256u; step *= 2u) {
        var v = sums[i];
        if i >= step { v += sums[i - step]; }
        workgroupBarrier();
        sums[i] = v;
        workgroupBarrier();
    }
    var run = 0.0;
    if i > 0u { run = sums[i - 1u]; }
    for (var j = 0u; j < per; j++) {
        run += field.meteors[i * per + j].light.w;
        field.cdf[i * per + j] = run;
    }
    if i == 255u {
        field.total = vec4(sums[255], f32(atomicLoad(&burning)), f32(atomicLoad(&drawn)), 0.0);
    }
}
