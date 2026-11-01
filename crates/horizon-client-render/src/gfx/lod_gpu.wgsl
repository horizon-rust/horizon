// Far terrain tiles walked on the GPU (gfx/lod_gpu.rs): the nodes of a level
// at a time, each tested against the frustum (and `node_hidden`), the visible
// ones drawn, their children queued for the next level. A tile that is drawn
// writes its draw (as `DrawIndexedIndirectArgs`) into its arena page's share
// of `draws`, and its origin and fade into `instances` at the tile's index,
// which is the draw's first instance: terrain.wgsl's vs_terrain reads it as
// the per-section origin. The walk goes level by level, breadth first, with
// indirect dispatches, and tests each node's projected size.

struct View {
    // Camera-relative frustum planes, as gfx.rs's `frustum`.
    planes: array<vec4<f32>, 6>,
    view_proj: mat4x4<f32>,
    // The camera's block (x, y, z) and the rest of its position.
    cam_int: vec4<i32>,
    cam_frac: vec4<f32>,
    // xyz: the eye, camera-relative; w: 1 when quads facing away are left out.
    eye: vec4<f32>,
    // x: pixels per unit at distance 1 (screen height / 2 / tan(fov / 2)),
    // y: the smallest a node's box may be on the screen, in pixels.
    misc: vec4<f32>,
    // x: milliseconds on the renderer's clock, y: 1 to draw a tile as one
    // run of quads, z: arena pages.
    times: vec4<i32>,
}
struct Node {
    origin: vec2<i32>,
    level: u32,
    tile: u32,
    bmin: vec3<f32>,
    first_child: u32,
    bmax: vec3<f32>,
    child_count: u32,
}
struct Tile {
    base: u32,
    page: u32,
    level: u32,
    shown: i32,
    dying: i32,
    alone: u32,
    ends: array<u32, 7>,
    planes: array<f32, 7>,
    // The tile's meshlets, for the mesh shader path (lod_gpu_items.wgsl).
    meshlet_first: u32,
    meshlet_count: u32,
}
@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> nodes: array<Node>;
@group(0) @binding(2) var<storage, read> tiles: array<Tile>;
// The nodes of the level being walked, then of the next: two halves.
@group(0) @binding(3) var<storage, read_write> queue: array<u32>;
// 0, 1: the two halves' lengths; 2: levels walked; 4..: each arena page's
// draws so far; then each page's first draw in `draws`.
@group(0) @binding(4) var<storage, read_write> state: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> draws: array<u32>;
@group(0) @binding(6) var<storage, read_write> instances: array<vec4<f32>>;

const NONE: u32 = 0xffffffffu;
const NO_TIME: i32 = -2147483647 - 1;
const PAGES: u32 = 4u;

// gfx/lod.rs's `instance`: the UV step, negated past a fading tile's code.
fn fade_code(age: i32, level: u32) -> u32 {
    let faded = min(f32(max(age, 0)) / f32(FADE_MS), 2.0);
    let shown = u32(faded * 64.0) + 1u;
    if shown >= 64u {
        return 0u;
    }
    return 1u + level * 129u + shown;
}
fn fade_out_code(age: i32, level: u32) -> u32 {
    let faded = min(f32(max(age, 0)) / f32(FADE_MS), 2.0);
    return 1u + level * 129u + 64u + min(u32(faded * 64.0) + 1u, 64u);
}

// A tile's origin relative to the camera.
fn relative(origin: vec2<i32>) -> vec3<f32> {
    return vec3(
        f32(origin.x - view.cam_int.x) - view.cam_frac.x,
        f32(-view.cam_int.y) - view.cam_frac.y,
        f32(origin.y - view.cam_int.z) - view.cam_frac.z);
}

// Whether the box is wholly outside a frustum plane: Frustum::intersects.
fn outside(lo: vec3<f32>, hi: vec3<f32>) -> bool {
    for (var i = 0u; i < 6u; i++) {
        let p = view.planes[i];
        let v = vec3(
            select(lo.x, hi.x, p.x >= 0.0),
            select(lo.y, hi.y, p.y >= 0.0),
            select(lo.z, hi.z, p.z >= 0.0));
        if dot(p.xyz, v) + p.w < 0.0 {
            return true;
        }
    }
    return false;
}

// Whether the box's bounding sphere covers fewer pixels across than
// `misc.y`. A box behind the camera plane never counts as small.
fn too_small(lo: vec3<f32>, hi: vec3<f32>) -> bool {
    let center = (lo + hi) * 0.5;
    let radius = length(hi - lo) * 0.5;
    let depth = (view.view_proj * vec4(center, 1.0)).w;
    if depth <= 0.0 {
        return false;
    }
    return 2.0 * radius * view.misc.x / depth < view.misc.y;
}

// Whether quads of bucket `b` can face the eye: Sides::visible_from, whose
// order and margin the constants above came from.
fn bucket_seen(b: u32, eye: vec3<f32>, plane: f32) -> bool {
    let axis = BUCKET_AXIS[b];
    if axis == 3u {
        return true;
    }
    if BUCKET_POSITIVE[b] == 1u {
        return eye[axis] > plane - SIDE_MARGIN;
    }
    return eye[axis] < plane + SIDE_MARGIN;
}

// One draw of `count` quads from quad `first` of tile `ti`.
fn push_run(page: u32, base: u32, ti: u32, first: u32, count: u32) {
    let pages = u32(view.times.z);
    let slot = atomicAdd(&state[PAGES + page], 1u);
    let entry = (atomicLoad(&state[PAGES + pages + page]) + slot) * 5u;
    draws[entry] = count * 6u;
    draws[entry + 1u] = 1u;
    draws[entry + 2u] = 0u;
    draws[entry + 3u] = base + first * 4u;
    draws[entry + 4u] = ti;
}

// Tile `ti`, at `origin` from the camera, drawn as the CPU's `Tile::visible`
// and `facing_runs_from` draw it.
fn emit(ti: u32, origin: vec3<f32>) {
    var t = tiles[ti];
    let now = view.times.x;
    var code = 0u;
    if t.alone != 0u && t.dying != NO_TIME {
        let age = now - t.dying;
        if age >= FADE_MS {
            return;
        }
        code = fade_out_code(age, t.level);
    } else if t.shown == NO_TIME {
        return;
    } else {
        code = fade_code(now - t.shown, t.level);
    }
    let w = select(-(f32(code) + UV_STEP), UV_STEP, code == 0u);
    instances[ti] = vec4(origin, w);
    if MESH_PATH {
        push_items(ti, t.page);
        return;
    }
    if view.eye.w < 0.5 {
        push_run(t.page, t.base, ti, 0u, t.ends[6]);
        return;
    }
    let eye = view.eye.xyz - origin;
    let single = view.times.y != 0;
    var start = 0u;
    var run = NONE;
    var first_seen = NONE;
    var last_end = 0u;
    for (var b = 0u; b < 7u; b++) {
        let end = t.ends[b];
        if end == start {
            continue;
        }
        if bucket_seen(b, eye, t.planes[b]) {
            first_seen = min(first_seen, start);
            last_end = end;
            if run == NONE {
                run = start;
            }
        } else if run != NONE {
            if !single {
                push_run(t.page, t.base, ti, run, start - run);
            }
            run = NONE;
        }
        start = end;
    }
    if single {
        if first_seen != NONE {
            push_run(t.page, t.base, ti, first_seen, last_end - first_seen);
        }
    } else if run != NONE {
        push_run(t.page, t.base, ti, run, start - run);
    }
}

@compute @workgroup_size(64)
fn traverse(@builtin(global_invocation_id) id: vec3<u32>) {
    let level_step = atomicLoad(&state[2]);
    let here = level_step & 1u;
    if id.x >= atomicLoad(&state[here]) {
        return;
    }
    let half_len = arrayLength(&queue) / 2u;
    let n = nodes[queue[here * half_len + id.x]];
    let origin = relative(n.origin);
    let lo = origin + n.bmin;
    let hi = origin + n.bmax;
    if outside(lo, hi) || too_small(lo, hi) || node_hidden(lo, hi) {
        return;
    }
    if n.tile != NONE {
        emit(n.tile, origin);
    }
    let next = 1u - here;
    for (var k = 0u; k < n.child_count; k++) {
        let slot = atomicAdd(&state[next], 1u);
        queue[next * half_len + slot] = n.first_child + k;
    }
}
