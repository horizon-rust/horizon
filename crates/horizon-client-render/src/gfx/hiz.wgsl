// A depth pyramid and the occlusion test of far terrain tiles (gfx/hiz.rs).
//
// The world's depth is reversed (1 at the camera, 0 far away and where
// nothing was drawn), so a pyramid texel holds the smallest depth of the
// pixels under it: the farthest surface there. A box whose nearest depth is
// still behind that is hidden.

@group(0) @binding(0) var depth: texture_depth_2d;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var level_out: texture_storage_2d<r32float, write>;

// The last row or column a texel of the next level reduces: the second
// below it, or the third where the level below has an odd side (its last
// texel would be left out of every reduction otherwise).
fn reduce_end(i: u32, count: u32, below: u32) -> u32 {
    let odd_end = i + 1u == count && (below & 1u) == 1u;
    return min(i * 2u + 1u + select(0u, 1u, odd_end), below - 1u);
}

// Level 0: the farthest depth of each 2x2 pixels of the world's depth.
@compute @workgroup_size(8, 8)
fn from_depth(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(level_out);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let below = textureDimensions(depth);
    let end = vec2(reduce_end(id.x, size.x, below.x), reduce_end(id.y, size.y, below.y));
    var farthest = 1.0;
    for (var y = id.y * 2u; y <= end.y; y++) {
        for (var x = id.x * 2u; x <= end.x; x++) {
            farthest = min(farthest, textureLoad(depth, vec2(x, y), 0));
        }
    }
    textureStore(level_out, id.xy, vec4(farthest, 0.0, 0.0, 0.0));
}

// Each next level, from the one below (bound as `source`).
@compute @workgroup_size(8, 8)
fn down(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(level_out);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let below = textureDimensions(source);
    let end = vec2(reduce_end(id.x, size.x, below.x), reduce_end(id.y, size.y, below.y));
    var farthest = 1.0;
    for (var y = id.y * 2u; y <= end.y; y++) {
        for (var x = id.x * 2u; x <= end.x; x++) {
            farthest = min(farthest, textureLoad(source, vec2(x, y), 0).r);
        }
    }
    textureStore(level_out, id.xy, vec4(farthest, 0.0, 0.0, 0.0));
}

// The leading member of terrain.wgsl's Globals.
struct Globals {
    view_proj: mat4x4<f32>,
}
struct Params {
    // This frame's stamp, which a test that sees its box stores.
    stamp: u32,
    // The result slot of the first test.
    first: u32,
    count: u32,
    // The pyramid's levels.
    levels: u32,
    // The depth, in pixels.
    size: vec2<f32>,
    padding: vec2<u32>,
}

// One test per far tile: its visible index (which names its origin), its
// first draw in phase 2 and how many draws it has there (0: phase 1 drew
// it, and the test only tells whether it is still in sight).
@group(0) @binding(3) var<storage, read> tests: array<vec4<u32>>;
// Per test the box's low and high corner, relative to the tile's origin.
@group(0) @binding(4) var<storage, read> boxes: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read_write> results: array<u32>;
// Phase 2's draws, 5 words each, with instance counts of 0 until seen.
@group(0) @binding(6) var<storage, read_write> phase2: array<u32>;
// Camera-relative origin of each visible section and tile.
@group(0) @binding(7) var<storage, read> origins: array<vec4<f32>>;
@group(0) @binding(8) var<uniform> globals: Globals;
@group(0) @binding(9) var<uniform> params: Params;
@group(0) @binding(10) var pyramid: texture_2d<f32>;

// A tile fading in is drawn a hair toward the camera (terrain.wgsl's
// `fade_depth`, up to 0.04%): its box counts as that much nearer.
const NEAR_SLACK: f32 = 1.002;

// Whether the box is behind everything drawn over its pixels. Anything that
// cannot be told is not hidden: a corner behind the camera, a box off the
// screen (frustum culling left it to the clipper).
fn hidden(low: vec3<f32>, high: vec3<f32>) -> bool {
    var rect_low = vec2(1e30);
    var rect_high = vec2(-1e30);
    var nearest = 0.0;
    for (var c = 0u; c < 8u; c++) {
        let corner = vec3(
            select(low.x, high.x, (c & 1u) != 0u),
            select(low.y, high.y, (c & 2u) != 0u),
            select(low.z, high.z, (c & 4u) != 0u),
        );
        let p = globals.view_proj * vec4(corner, 1.0);
        if p.w <= 0.0 {
            return false;
        }
        let ndc = p.xyz / p.w;
        rect_low = min(rect_low, ndc.xy);
        rect_high = max(rect_high, ndc.xy);
        nearest = max(nearest, ndc.z);
    }
    // The pixels it covers, the screen's y down, within the screen.
    let top_left = floor(vec2(rect_low.x * 0.5 + 0.5, 0.5 - rect_high.y * 0.5) * params.size);
    let bottom_right = floor(vec2(rect_high.x * 0.5 + 0.5, 0.5 - rect_low.y * 0.5) * params.size);
    let pixel_low = max(top_left, vec2(0.0));
    let pixel_high = min(bottom_right, params.size - 1.0);
    if any(pixel_high < pixel_low) {
        return false;
    }
    // The level whose texels (2^(level + 1) pixels wide) the box fits in two of.
    let extent = max(pixel_high.x - pixel_low.x, pixel_high.y - pixel_low.y) + 1.0;
    let level = clamp(i32(ceil(log2(extent))) - 1, 0, i32(params.levels) - 1);
    let texel = f32(2u << u32(level));
    let edge = vec2<i32>(textureDimensions(pyramid, level)) - vec2<i32>(1);
    let texel_low = clamp(vec2<i32>(pixel_low / texel), vec2<i32>(0), edge);
    let texel_high = clamp(vec2<i32>(pixel_high / texel), vec2<i32>(0), edge);
    var farthest = 1.0;
    for (var y = texel_low.y; y <= texel_high.y; y++) {
        for (var x = texel_low.x; x <= texel_high.x; x++) {
            farthest = min(farthest, textureLoad(pyramid, vec2(x, y), level).r);
        }
    }
    return nearest * NEAR_SLACK < farthest;
}

@compute @workgroup_size(64)
fn test(@builtin(global_invocation_id) id: vec3<u32>) {
    let t = id.x;
    if t >= params.count {
        return;
    }
    let entry = tests[t];
    let origin = origins[entry.x].xyz;
    if hidden(origin + boxes[t * 2u].xyz, origin + boxes[t * 2u + 1u].xyz) {
        return;
    }
    results[params.first + t] = params.stamp;
    for (var k = 0u; k < entry.z; k++) {
        phase2[(entry.y + k) * 5u + 1u] = 1u;
    }
}
