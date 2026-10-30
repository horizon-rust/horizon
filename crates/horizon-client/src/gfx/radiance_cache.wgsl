// RT lighting's radiance cache's resolve (gfx/shadow_rays.rs, shadow_rays.wgsl `CacheCell`): once a
// frame, each cell's sums since the last join its light, an average over its frames up to
// `CACHE_FRAMES` (SHaRC's `accumulationFrameNum` as Remix sets it), and a cell nobody added to for
// `CACHE_STALE` frames (its `staleFrameNumMax`) is emptied for others. `age`: the frames since it
// was last added to (low 16 bits) and the frames its light averages (high 16 bits).
struct CacheCell {
    key: atomic<u32>,
    r: atomic<u32>,
    g: atomic<u32>,
    b: atomic<u32>,
    count: atomic<u32>,
    light: array<u32, 3>,
    age: u32,
}
@group(0) @binding(0) var<storage, read_write> cache: array<CacheCell>;
const CACHE_SCALE: f32 = 1024.0;
const CACHE_STALE: u32 = 1024u;
const CACHE_FRAMES: u32 = 1000u;
@compute @workgroup_size(64) fn cs_resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= arrayLength(&cache) { return; }
    if atomicLoad(&cache[i].key) == 0u { return; }
    let count = atomicExchange(&cache[i].count, 0u);
    let sum = vec3(f32(atomicExchange(&cache[i].r, 0u)), f32(atomicExchange(&cache[i].g, 0u)),
        f32(atomicExchange(&cache[i].b, 0u))) / CACHE_SCALE;
    if count == 0u {
        cache[i].age += 1u;
        if (cache[i].age & 0xffffu) > CACHE_STALE {
            atomicStore(&cache[i].key, 0u);
            cache[i].light = array<u32, 3>(0xffffffffu, 0xffffffffu, 0xffffffffu);
            cache[i].age = 0u;
        }
        return;
    }
    let now = sum / f32(count);
    var light = now;
    let held = cache[i].light;
    var frames = 1u;
    if held[0] != 0xffffffffu {
        let last = vec3(bitcast<f32>(held[0]), bitcast<f32>(held[1]), bitcast<f32>(held[2]));
        frames = min((cache[i].age >> 16u) + 1u, CACHE_FRAMES);
        light = mix(last, now, 1.0 / f32(frames));
    }
    cache[i].light = array<u32, 3>(bitcast<u32>(light.r), bitcast<u32>(light.g), bitcast<u32>(light.b));
    cache[i].age = frames << 16u;
}
