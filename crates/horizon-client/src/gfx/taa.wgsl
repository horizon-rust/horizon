// Temporal anti-aliasing (gfx/taa.rs) on the linear HDR scene, before the tone mapping: Karis' "High
// Quality Temporal Supersampling" (2014) with Salvi's variance clipping. The world is drawn with a
// sub-pixel jitter (Halton 2, 3); each frame blends into the last resolved frame, reprojected by
// depth and the camera's motion, so the jitter becomes supersampling and a thin bright streak a
// pixel wide averages into the pixels it covers instead of flickering.
//
//   fs_resolve  the new frame against the history, into the next history and its aux (depth, age);
//   fs_blit     the resolved frame back over the scene, for what reads it next (exposure, bloom, tone
//               mapping).
//
// The scene's alpha is the terrain's sun share, which the tone mapping reads: it is resolved like a
// colour channel.
struct Taa {
    // This frame's matrices without the jitter and the last frame's, both relative to their own
    // camera (the resolve unprojects the pixel centre, so a still camera maps each pixel onto
    // itself whatever the jitter).
    inverse_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    // xyz: this frame's camera minus the last one's, in blocks; w: 1 to forget the history (the
    // view is new, or the camera was cut).
    motion: vec4<f32>,
    // The scene's width and height in pixels, and their inverses.
    size: vec4<f32>,
}
@group(0) @binding(0) var<uniform> taa: Taa;
@group(0) @binding(1) var scene: texture_2d<f32>;
// The world's depth, reversed (1 at the near plane, 0 the sky).
@group(0) @binding(2) var depth: texture_depth_2d;
// The last resolved frame (linear light, alpha as the scene's) and its aux: x the view depth, or
// SKY, y how many frames the pixel has accumulated.
@group(0) @binding(3) var history: texture_2d<f32>;
@group(0) @binding(4) var history_aux: texture_2d<f32>;
@group(0) @binding(5) var linear_sampler: sampler;
// What `fs_blit` copies back.
@group(0) @binding(6) var resolved: texture_2d<f32>;

// What a new sample weighs once the history is full: 1/16, Karis' feedback of 0.9375.
const NEW_WEIGHT: f32 = 0.0625;
const MAX_FRAMES: f32 = 16.0;
// How far the neighbourhood's spread reaches (Salvi's gamma): 1 clips hardest, and the history
// keeps less of what the 3x3 neighbourhood does not show; above it, ghosts last longer.
const GAMMA: f32 = 1.25;
// How far, relative, the depth under the history may be from where the pixel reprojects.
const DEPTH_TOLERANCE: f32 = 0.04;
// What the aux holds for the sky: its depth is the far plane's, which turning the camera moves.
const SKY: f32 = 60000.0;
// The most a colour channel may hold in the history (below half precision's limit).
const MAX_LIGHT: f32 = 60000.0;

struct Screen {
    @builtin(position) position: vec4<f32>,
}
@vertex
fn vs_taa(@builtin(vertex_index) i: u32) -> Screen {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return Screen(vec4(p * 2.0 - 1.0, 0.0, 1.0));
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}
// Karis' reversible tone map: the bright pixels (the sun's glint, an emissive block) no longer
// stretch the neighbourhood's box until nothing is clipped.
fn compress(c: vec3<f32>) -> vec3<f32> {
    return c / (1.0 + luma(c));
}
fn decompress(c: vec3<f32>) -> vec3<f32> {
    return max(c, vec3(0.0)) / (1.0 - min(luma(c), 0.9999));
}
fn to_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3(
        0.25 * c.r + 0.5 * c.g + 0.25 * c.b,
        0.5 * c.r - 0.5 * c.b,
        -0.25 * c.r + 0.5 * c.g - 0.25 * c.b,
    );
}
fn from_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

// The scene at `p`, clamped to the edge; a NaN or infinity from the rays must not enter the history,
// where it would stay.
fn load_scene(p: vec2<i32>) -> vec4<f32> {
    let q = clamp(p, vec2(0), vec2<i32>(taa.size.xy) - 1);
    let c = textureLoad(scene, q, 0);
    return clamp(select(c, vec4(0.0), c != c), vec4(0.0), vec4(MAX_LIGHT));
}
fn load_depth(p: vec2<i32>) -> f32 {
    return textureLoad(depth, clamp(p, vec2(0), vec2<i32>(taa.size.xy) - 1), 0);
}

// Where a pixel is, in blocks from the camera, and its depth along the view axis.
struct Place {
    position: vec3<f32>,
    view_depth: f32,
}
fn unproject(uv: vec2<f32>, d: f32) -> Place {
    let ndc = vec2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let h = taa.inverse_view_proj * vec4(ndc, d, 1.0);
    return Place(h.xyz / h.w, 1.0 / h.w);
}

// The history at `uv` by Catmull-Rom through five bilinear taps (Jimenez's version of the bicubic
// filter): the plain bilinear's blur, repeated every frame, would soften a still image.
fn sample_history(uv: vec2<f32>) -> vec4<f32> {
    let position = uv * taa.size.xy;
    let centre = floor(position - 0.5) + 0.5;
    let f = position - centre;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let t0 = (centre - 1.0) * taa.size.zw;
    let t3 = (centre + 2.0) * taa.size.zw;
    let t12 = (centre + w2 / w12) * taa.size.zw;
    var sum = textureSampleLevel(history, linear_sampler, vec2(t12.x, t0.y), 0.0) * (w12.x * w0.y);
    sum += textureSampleLevel(history, linear_sampler, vec2(t0.x, t12.y), 0.0) * (w0.x * w12.y);
    sum += textureSampleLevel(history, linear_sampler, t12, 0.0) * (w12.x * w12.y);
    sum += textureSampleLevel(history, linear_sampler, vec2(t3.x, t12.y), 0.0) * (w3.x * w12.y);
    sum += textureSampleLevel(history, linear_sampler, vec2(t12.x, t3.y), 0.0) * (w12.x * w3.y);
    let weights = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    let c = sum / weights;
    return clamp(select(c, vec4(0.0), c != c), vec4(0.0), vec4(MAX_LIGHT));
}

// `h` brought onto the box `centre` +- `extent` along the line to the centre (Playdead's clip: the
// colour keeps its hue, where clamping each channel shifts it).
fn clip_box(centre: vec3<f32>, extent: vec3<f32>, h: vec3<f32>) -> vec3<f32> {
    let v = h - centre;
    let a = max(max(abs(v.x / extent.x), abs(v.y / extent.y)), abs(v.z / extent.z));
    if a > 1.0 { return centre + v / a; }
    return h;
}

struct Resolved {
    @location(0) color: vec4<f32>,
    @location(1) aux: vec4<f32>,
}
@fragment
fn fs_resolve(screen: Screen) -> Resolved {
    let texel = vec2<i32>(screen.position.xy);
    var current = vec4(0.0);
    var m1 = vec3(0.0);
    var m2 = vec3(0.0);
    var alpha_low = MAX_LIGHT;
    var alpha_high = 0.0;
    // The closest depth of the 3x3: an edge pixel follows the surface in front of it, so a
    // silhouette does not drag the background's history along.
    var closest = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let at = texel + vec2(dx, dy);
            let s = load_scene(at);
            let y = to_ycocg(compress(s.rgb));
            m1 += y;
            m2 += y * y;
            alpha_low = min(alpha_low, s.a);
            alpha_high = max(alpha_high, s.a);
            if dx == 0 && dy == 0 { current = s; }
            closest = max(closest, load_depth(at));
        }
    }
    let uv = (vec2<f32>(texel) + 0.5) * taa.size.zw;
    let own_depth = load_depth(texel);
    let own = unproject(uv, own_depth);

    // Where the history saw what this pixel shows: the world is still, so the camera's motion is
    // all there is.
    let place = unproject(uv, closest);
    let moved = place.position + taa.motion.xyz;
    let c = taa.prev_view_proj * vec4(moved, 1.0);
    let prev_uv = vec2(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
    var age = 0.0;
    var seen = c.w > 0.0 && taa.motion.w < 0.5 && all(prev_uv >= vec2(0.0)) && all(prev_uv <= vec2(1.0));
    if seen {
        // A disocclusion: none of the four texels around is at the depth the point should have.
        let base = vec2<i32>(floor(prev_uv * taa.size.xy - 0.5));
        var matched = false;
        for (var i = 0; i < 4; i++) {
            let at = clamp(base + vec2(i & 1, i >> 1), vec2(0), vec2<i32>(taa.size.xy) - 1);
            let aux = textureLoad(history_aux, at, 0).xy;
            var same = abs(aux.x - c.w) < DEPTH_TOLERANCE * c.w;
            if closest <= 0.0 { same = aux.x >= SKY; }
            if same {
                matched = true;
                age = max(age, aux.y);
            }
        }
        seen = matched;
    }

    var out_color = current;
    var frames = 1.0;
    if seen {
        let mean = m1 / 9.0;
        let sigma = sqrt(max(m2 / 9.0 - mean * mean, vec3(0.0)));
        // A floor keeps a flat neighbourhood from clipping the history to its noise.
        let extent = max(sigma * GAMMA, vec3(0.004));
        let past = sample_history(prev_uv);
        let kept = decompress(from_ycocg(clip_box(mean, extent, to_ycocg(compress(past.rgb)))));
        let kept_alpha = clamp(past.a, alpha_low, alpha_high);
        frames = min(age + 1.0, MAX_FRAMES);
        let fresh = max(1.0 / frames, NEW_WEIGHT);
        // Karis' luminance weighting: a bright sample counts for less, so a glint a pixel wide
        // that the jitter catches on one frame in eight cannot own the pixel.
        let w_current = fresh / (1.0 + luma(current.rgb));
        let w_kept = (1.0 - fresh) / (1.0 + luma(kept));
        out_color = vec4(
            (current.rgb * w_current + kept * w_kept) / (w_current + w_kept),
            mix(kept_alpha, current.a, fresh),
        );
    }
    var out: Resolved;
    out.color = out_color;
    out.aux = vec4(select(own.view_depth, SKY, own_depth <= 0.0), frames, 0.0, 0.0);
    return out;
}

@fragment
fn fs_blit(screen: Screen) -> @location(0) vec4<f32> {
    return textureLoad(resolved, vec2<i32>(screen.position.xy), 0);
}
