// Froxel volumetrics' hardware rays (gfx/froxels.rs): the hybrid's drawn terrain (gfx/hw_rays.rs) in
// hardware acceleration structures, for the froxels' shadow rays to meet its exact triangles,
// alpha-tested. The same as shadow_rays_hw.wgsl, duplicated to leave that file alone. Replaces
// froxels.wgsl's HARDWARE section when the GPU traces rays (`enable wgpu_ray_query;` before it).
const HW: bool = true;
@group(0) @binding(15) var terrain: acceleration_structure;
// The vertex arena's first page (gfx/arena.rs): `TerrainVertex` six words each, position, then
// the texture coordinate (two halves, `UV_STEPS` a unit), then the layer and the light.
@group(0) @binding(16) var<storage, read> arena: array<u32>;
// The first sprite array, for the alpha test.
@group(0) @binding(17) var sprites: texture_2d_array<f32>;
const UV_STEPS: f32 = 32768.0;
// Arena vertex `v`'s texture coordinate.
fn hw_uv(v: u32) -> vec2<f32> {
    let w = arena[v * 6u + 3u];
    return vec2(f32(w & 0xffffu), f32(w >> 16u)) / UV_STEPS;
}
// Where each instance's quads are (gfx/hw_rays.rs, by `instance_custom_data`): the arena page, the
// first vertex of its solid quads and of its translucent ones, and which geometry is the
// translucent one (2: none).
@group(0) @binding(23) var<storage, read> instances: array<vec4<u32>>;
// The quad candidate hit `c` is on: its first arena vertex, or none (`u32` max) past the first page.
fn hw_quad(c: RayIntersection, translucent: bool) -> u32 {
    let at = instances[c.instance_custom_data];
    if at.x != 0u { return 0xffffffffu; }
    return select(at.y, at.z, translucent) + (c.primitive_index / 2u) * 4u;
}
// The texel candidate hit `c` meets on the quad from arena vertex `base`, and its sprite layer
// (past the first array: none, -1).
fn hw_texel(c: RayIntersection, base: u32) -> vec4<f32> {
    let layer = arena[base * 6u + 4u] & 0x7fffu;
    if layer >= SPRITE_SPLIT { return vec4(-1.0); }
    // The quad's triangles: corners 0, 1, 2 and 0, 2, 3 (gfx.rs `quad_indices`).
    let second = (c.primitive_index & 1u) == 1u;
    let b = c.barycentrics;
    let uv = hw_uv(base) * (1.0 - b.x - b.y) + hw_uv(base + select(1u, 2u, second)) * b.x
        + hw_uv(base + select(2u, 3u, second)) * b.y;
    let size = vec2<i32>(textureDimensions(sprites).xy);
    let texel = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2(0), size - 1);
    return textureLoad(sprites, texel, i32(layer), 0);
}
// Whether candidate hit `c` (a solid quad) meets an opaque texel. Opaque leaves, quads on later
// arena pages and sprites past the first array are taken as opaque.
fn hw_opaque(c: RayIntersection) -> bool {
    let base = hw_quad(c, false);
    if base == 0xffffffffu { return true; }
    if (arena[base * 6u + 4u] & 0x8000u) != 0u { return true; }
    let texel = hw_texel(c, base);
    return texel.a < 0.0 || texel.a >= 0.5;
}
// What candidate hit `c` (a translucent quad: stained glass, water, ice) lets through: the light
// filtered by its colour (its texel times its tint, in light), as much as it is opaque.
fn hw_tint(c: RayIntersection, pos: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let base = hw_quad(c, true);
    if base == 0xffffffffu { return vec3(1.0); }
    // Water: the caustics its waves focus where the ray crosses its surface, the light lost on
    // the way up through it (the ray starts under it: `c.t` is the depth crossed).
    let layer = f32(arena[base * 6u + 4u] & 0x7fffu);
    if layer == fx.water.x || layer == fx.water.y {
        let at = pos + l * c.t + fx.camera.xyz + fx.fraction.xyz;
        return water_caustics(at.xz, fx.water.z) * exp(-WATER_ABSORPTION * c.t);
    }
    let texel = hw_texel(c, base);
    if texel.a < 0.0 { return vec3(1.0); }
    let w = arena[base * 6u + 5u];
    let tint = vec3(f32(w & 255u), f32((w >> 8u) & 255u), f32((w >> 16u) & 255u)) / 255.0;
    let colour = pow(max(texel.rgb * tint, vec3(0.0)), vec3(2.2));
    return mix(vec3(1.0), colour, clamp(texel.a * 1.5, 0.0, 1.0));
}
// The light the drawn terrain lets through from camera-relative `pos` along `l` (unit) for
// `reach`: none once a triangle's opaque texel is met, otherwise what the translucent ones it
// crosses let through, in colour.
fn hw_light(pos: vec3<f32>, l: vec3<f32>, reach: f32) -> vec3<f32> {
    var q: ray_query;
    // The instances stand around their anchor (gfx/hw_rays.rs): the camera's offset from it.
    rayQueryInitialize(&q, terrain, RayDesc(0x0u, 0xffu, 0.0, reach, pos + fx.hw_offset.xyz, l));
    var through = vec3(1.0);
    while rayQueryProceed(&q) {
        let c = rayQueryGetCandidateIntersection(&q);
        if c.geometry_index == instances[c.instance_custom_data].w {
            through *= hw_tint(c, pos, l);
        } else if hw_opaque(c) {
            rayQueryConfirmIntersection(&q);
            // A shadow ray needs no nearer hit.
            rayQueryTerminate(&q);
        }
    }
    let hit = rayQueryGetCommittedIntersection(&q);
    return select(through, vec3(0.0), hit.kind != RAY_QUERY_INTERSECTION_NONE);
}
// The caustics the waves focus where the light crosses the water's surface, from its curvature (1
// on average, bright where the waves gather the light; shadow_rays_hw.wgsl).
fn water_caustics(p: vec2<f32>, t: f32) -> f32 {
    let dirs = array<vec2<f32>, 4>(vec2(0.96, 0.28), vec2(-0.51, 0.86), vec2(0.66, -0.75),
        vec2(-0.2, -0.98));
    let ks = vec4(1.3, 2.1, 3.7, 6.3) * 1.6;
    let amps = vec4(0.03, 0.018, 0.009, 0.004);
    let speeds = vec4(1.1, 1.4, 1.9, 2.6);
    var curvature = 0.0;
    for (var i = 0; i < 4; i++) {
        let phase = ks[i] * dot(dirs[i], p) - speeds[i] * t;
        curvature += amps[i] * ks[i] * ks[i] * sin(phase);
    }
    // Soft: the light gathered under a crest, spread under a trough, never all of it gone.
    let c = clamp(1.0 + curvature * 2.5, 0.35, 2.2);
    return c * c * 0.85;
}
// Pure water's absorption a block (a metre), red most: deep water turns blue-green.
const WATER_ABSORPTION: vec3<f32> = vec3(0.35, 0.065, 0.025);
