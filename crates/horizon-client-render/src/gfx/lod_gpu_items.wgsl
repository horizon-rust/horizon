// The traversal for the mesh shader path (gfx/lod_mesh.rs): a tile kept is
// listed as groups of up to 32 meshlets for the task shader, in its page's
// share of `items` and the page's task dispatch (`task_args`: x workgroups,
// then 1 and 1), instead of as indexed draws. A page's share starts where
// `state` says its draws would (the shader path has none).
const MESH_PATH: bool = true;
@group(0) @binding(7) var<storage, read_write> task_args: array<atomic<u32>>;
@group(0) @binding(8) var<storage, read_write> items: array<vec4<u32>>;

fn push_items(ti: u32, page: u32) {
    let first = tiles[ti].meshlet_first;
    let total = tiles[ti].meshlet_count;
    let base = atomicLoad(&state[PAGES + u32(view.times.z) + page]);
    for (var g = 0u; g * 32u < total; g++) {
        let slot = atomicAdd(&task_args[page * 4u], 1u);
        items[base + slot] = vec4(ti, first + g * 32u, min(32u, total - g * 32u), 0u);
    }
}
