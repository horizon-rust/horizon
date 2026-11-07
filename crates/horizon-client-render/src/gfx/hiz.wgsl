// Hi-Z (gfx/hiz.rs): a depth pyramid of the world pass's depth, each level the
// farthest depth over the four texels below it.
//
// The depth is reversed (gfx.rs clears to 0.0 and compares `GreaterEqual`), so
// 1.0 is the near plane and the farthest depth is the SMALLEST value: every
// level is a `min`. A region's minimum is its weakest occluder, which is what
// an occlusion test must compare against -- a box is hidden only where even the
// weakest thing in front of it is nearer than the box's nearest corner.
//
// Level 0 is HALF the depth's size, each texel the farthest of the four depth
// texels under it. An occlusion test reads four texels of whichever level is
// wide enough to cover a box's footprint, so a texel to the pixel buys it
// nothing, and half the size is a quarter of the pyramid to write and read.
//
// No level is padded out to a power of two. A level's size is the one below it
// shifted down, floor, so an odd level leaves a row or column over; the
// reduction that reads an odd level covers that leftover as a third texel
// rather than dropping it. Dropping a texel could only RAISE a minimum, which
// would claim a nearer occluder than the frame holds and hide a section that is
// in sight; covering one more can only lower it, which hides less. Reads off
// the end of a level take 1.0, the near plane, which is the largest depth there
// is and so cannot change a `min`.

struct Shape {
    /// The depth's size, and level 0's.
    depth: vec2<u32>,
    level0: vec2<u32>,
    /// Which level is being reduced, and its size.
    src: vec2<u32>,
    dst: vec2<u32>,
}
@group(0) @binding(0) var<uniform> shape: Shape;

// The world pass's depth, bound as an unfilterable float rather than as a
// depth texture, for the reason cull.wgsl gives at its own `depth`: naga's
// GLSL backend cannot `textureLoad` a depth texture, and silently reads zero
// on every texel. The texel is the same value either way.
@group(0) @binding(1) var depth: texture_2d<f32>;

/// A depth texel, or the near plane off the end.
fn depth_at(p: vec2<u32>) -> f32 {
    if any(p >= shape.depth) {
        return 1.0;
    }
    return textureLoad(depth, p, 0).r;
}

// Level 0: the farthest of each 2x2 of the depth.
//
// A render pass, not a compute one, to match cull.rs's working read of the same
// view: a fullscreen triangle into level 0. Only the reductions below are
// compute.
@vertex
fn seed_vertex(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    // The triangle that covers the clip square.
    let xy = vec2(f32((vertex << 1u) & 2u) * 2.0 - 1.0, f32(vertex & 2u) * 2.0 - 1.0);
    return vec4(xy, 0.0, 1.0);
}

@fragment
fn seed_fragment(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let base = vec2<u32>(at.xy) * 2u;
    let far = min(
        min(depth_at(base), depth_at(base + vec2(1u, 0u))),
        min(depth_at(base + vec2(0u, 1u)), depth_at(base + vec2(1u, 1u))),
    );
    return vec4(far, 0.0, 0.0, 0.0);
}

@group(1) @binding(0) var src: texture_2d<f32>;
@group(1) @binding(1) var dst: texture_storage_2d<r32float, write>;

/// A texel of the level being reduced, or the near plane off its end.
fn src_at(p: vec2<u32>) -> f32 {
    if any(p >= shape.src) {
        return 1.0;
    }
    return textureLoad(src, p, 0).x;
}

// Level n + 1: the farthest of each 2x2 of level n, and of the row or column an
// odd level n leaves over.
@compute @workgroup_size(8, 8)
fn reduce(@builtin(global_invocation_id) gid: vec3<u32>) {
    if any(gid.xy >= shape.dst) {
        return;
    }
    let base = gid.xy * 2u;
    // One texel more along an odd axis, for the last texel of this level only:
    // nothing of level n goes unread.
    let over = vec2<u32>(
        select(0u, 1u, (shape.src.x & 1u) != 0u && gid.x + 1u == shape.dst.x),
        select(0u, 1u, (shape.src.y & 1u) != 0u && gid.y + 1u == shape.dst.y),
    );
    var far = 1.0;
    for (var y = 0u; y <= 1u + over.y; y++) {
        for (var x = 0u; x <= 1u + over.x; x++) {
            far = min(far, src_at(base + vec2(x, y)));
        }
    }
    textureStore(dst, gid.xy, vec4(far, 0.0, 0.0, 0.0));
}
