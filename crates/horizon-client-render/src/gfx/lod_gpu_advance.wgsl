// Between two levels of the far tile traversal (gfx/lod_gpu.rs): the next
// level's nodes are all queued, so its dispatch size is known, and the half
// of the queue just walked is free for the level after it.
@group(0) @binding(0) var<storage, read_write> state: array<atomic<u32>>;
@group(0) @binding(1) var<storage, read_write> args: array<u32>;

@compute @workgroup_size(1)
fn advance() {
    let walked = atomicAdd(&state[2], 1u);
    let here = walked & 1u;
    let next = atomicLoad(&state[1u - here]);
    atomicStore(&state[here], 0u);
    args[0] = (next + 63u) / 64u;
    args[1] = 1u;
    args[2] = 1u;
}
