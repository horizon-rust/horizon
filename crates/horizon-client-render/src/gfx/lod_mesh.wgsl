// Far tiles by meshlets (gfx/lod_mesh.rs), appended to terrain.wgsl after the
// `enable wgpu_mesh_shader;` line. The traversal (lod_gpu.wgsl) lists, for
// each tile it keeps, groups of up to 32 of the tile's meshlets: one task
// workgroup each, one thread per meshlet, which leaves out the meshlets that
// are outside the frustum or face away from the eye (as the traversal's runs
// of side buckets do for whole buckets) and hands the others to as many mesh
// workgroups: one meshlet each, a thread per vertex, running terrain.wgsl's
// `terrain_vertex` on the quad vertices it reads from the arena page.
// Fragments are terrain.wgsl's fs_solid.

struct View {
    planes: array<vec4<f32>, 6>,
    view_proj: mat4x4<f32>,
    cam_int: vec4<i32>,
    cam_frac: vec4<f32>,
    // xyz: the eye, camera-relative; w: 1 when quads facing away are left out.
    eye: vec4<f32>,
    misc: vec4<f32>,
    times: vec4<i32>,
}
struct Meshlet {
    first: u32,
    count: u32,
    side: u32,
    plane: f32,
    bmin: vec3<f32>,
    // The first vertex, in the page the vertices are bound from.
    base: u32,
    bmax: vec3<f32>,
    pad: u32,
}
@group(1) @binding(0) var<uniform> view: View;
// Per tile: its camera-relative origin and fade (the draw path's instance).
@group(1) @binding(1) var<storage, read> tile_origins: array<vec4<f32>>;
// Per task workgroup: x the tile, y its first meshlet, z how many (up to 32).
@group(1) @binding(2) var<storage, read> items: array<vec4<u32>>;
@group(1) @binding(3) var<storage, read> meshlets: array<Meshlet>;
// The page's vertices, six words each (mesher.rs's `TerrainVertex`).
@group(1) @binding(4) var<storage, read> page_vertices: array<u32>;
// x: this page's first item.
@group(1) @binding(5) var<uniform> page: vec4<u32>;

struct Payload {
    tile: u32,
    count: u32,
    ids: array<u32, 32>,
}
var<task_payload> payload: Payload;
var<workgroup> kept: atomic<u32>;

fn outside_view(lo: vec3<f32>, hi: vec3<f32>) -> bool {
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

// Whether any of the meshlet can be seen: in the frustum, and, when quads
// facing away are left out, not wholly turned from the eye (Sides::visible_from).
fn meshlet_visible(m: Meshlet, origin: vec3<f32>) -> bool {
    if outside_view(origin + m.bmin, origin + m.bmax) {
        return false;
    }
    let axis = BUCKET_AXIS[m.side];
    if view.eye.w < 0.5 || axis == 3u {
        return true;
    }
    let eye = view.eye.xyz - origin;
    if BUCKET_POSITIVE[m.side] == 1u {
        return eye[axis] > m.plane - SIDE_MARGIN;
    }
    return eye[axis] < m.plane + SIDE_MARGIN;
}

@task @payload(payload) @workgroup_size(32)
fn ts_main(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) -> @builtin(mesh_task_size) vec3<u32> {
    let item = items[page.x + group.x];
    if lane == 0u {
        atomicStore(&kept, 0u);
        payload.tile = item.x;
    }
    workgroupBarrier();
    if lane < item.z {
        let id = item.y + lane;
        if meshlet_visible(meshlets[id], tile_origins[item.x].xyz) {
            let slot = atomicAdd(&kept, 1u);
            payload.ids[slot] = id;
        }
    }
    workgroupBarrier();
    let n = atomicLoad(&kept);
    if lane == 0u {
        payload.count = n;
    }
    return vec3(n, 1u, 1u);
}

struct MeshPrimitive {
    @builtin(triangle_indices) indices: vec3<u32>,
}
struct MeshOutput {
    @builtin(vertices) vertices: array<TerrainOut, 64>,
    @builtin(primitives) primitives: array<MeshPrimitive, 32>,
    @builtin(vertex_count) vertex_count: u32,
    @builtin(primitive_count) primitive_count: u32,
}
var<workgroup> mesh_output: MeshOutput;

@mesh(mesh_output) @payload(payload) @workgroup_size(64)
fn ms_main(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let m = meshlets[payload.ids[group.x]];
    if lane == 0u {
        mesh_output.vertex_count = m.count * 4u;
        mesh_output.primitive_count = m.count * 2u;
    }
    workgroupBarrier();
    if lane < m.count * 4u {
        // terrain.wgsl's vs_terrain for the vertex's six words: position, the UV, the sprite
        // layer and light (sky, block) packed in a word, the colour as four unorm bytes.
        let at = (m.base + lane) * 6u;
        let position = vec3(
            bitcast<f32>(page_vertices[at]),
            bitcast<f32>(page_vertices[at + 1u]),
            bitcast<f32>(page_vertices[at + 2u]));
        let uv = page_vertices[at + 3u];
        let layer_light = page_vertices[at + 4u];
        let origin = tile_origins[payload.tile];
        mesh_output.vertices[lane] = terrain_vertex(
            position,
            terrain_uv(vec2(uv & 0xffffu, uv >> 16u), origin.w),
            layer_light & 0xffffu,
            unpack4x8unorm(page_vertices[at + 5u]),
            vec2((layer_light >> 16u) & 0xffu, (layer_light >> 24u) & 0xffu),
            origin.xyz,
            origin.w);
    }
    if lane < m.count * 2u {
        // Quad q's corners 0 1 2 and 0 2 3, as the index buffer of the draw path.
        let first = (lane / 2u) * 4u;
        if lane % 2u == 0u {
            mesh_output.primitives[lane].indices = vec3(first, first + 1u, first + 2u);
        } else {
            mesh_output.primitives[lane].indices = vec3(first, first + 2u, first + 3u);
        }
    }
}
