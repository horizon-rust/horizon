
// RT lighting's hybrid (gfx/hw_rays.rs): the drawn terrain in hardware acceleration structures,
// for the sun's and the light sources' shadow rays to meet its exact triangles, alpha-tested.
// Appended to shadow_rays.wgsl when the GPU traces rays (`enable wgpu_ray_query;` before it).
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
// The moving geometry (gfx/hw_rays.rs `finish`): the entities' and the view model's vertices, the
// entity pipeline's `Vertex` (nine words: position, texture coordinate in the shared atlas, layer,
// colour, light, normal), and the atlas. An instance whose row starts with `DYN_ENTITY` holds
// quads from the vertex in its second word; one with `DYN_CLOUD` is a cloud tile (opaque, and
// left out of the bounce rays by their mask: `NO_CLOUDS`).
@group(0) @binding(34) var<storage, read> dynamics: array<u32>;
@group(0) @binding(35) var entity_atlas: texture_2d<f32>;
const DYN_ENTITY: u32 = 0xfffffff0u;
const NO_CLOUDS: u32 = 0xfbu;
const NO_QUAD: u32 = 0xffffffffu;
// Moving vertex `v`'s position, camera-relative as drawn.
fn dyn_position(v: u32) -> vec3<f32> {
    return bitcast<vec3<f32>>(vec3(dynamics[v * 9u], dynamics[v * 9u + 1u], dynamics[v * 9u + 2u]));
}
// The entity or view model quad candidate hit `c` is on: its first vertex in `dynamics`, or none.
fn hw_entity_quad(c: RayIntersection) -> u32 {
    let at = instances[c.instance_custom_data];
    if at.x != DYN_ENTITY { return NO_QUAD; }
    return at.y + (c.primitive_index / 2u) * 4u;
}
// The texel candidate hit `c` meets on the entity quad from vertex `base`, times the vertex colour
// (as entity.wgsl draws it, its alpha too).
fn hw_entity_texel(c: RayIntersection, base: u32) -> vec4<f32> {
    // The quad's triangles: corners 0, 1, 2 and 0, 2, 3 (gfx.rs `quad_indices`).
    let second = (c.primitive_index & 1u) == 1u;
    let b = c.barycentrics;
    let v1 = base + select(1u, 2u, second);
    let v2 = base + select(2u, 3u, second);
    let uv = bitcast<vec2<f32>>(vec2(dynamics[base * 9u + 3u], dynamics[base * 9u + 4u])) * (1.0 - b.x - b.y)
        + bitcast<vec2<f32>>(vec2(dynamics[v1 * 9u + 3u], dynamics[v1 * 9u + 4u])) * b.x
        + bitcast<vec2<f32>>(vec2(dynamics[v2 * 9u + 3u], dynamics[v2 * 9u + 4u])) * b.y;
    let size = vec2<i32>(textureDimensions(entity_atlas));
    let texel = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2(0), size - 1);
    let color = unpack4x8unorm(dynamics[base * 9u + 6u]);
    return textureLoad(entity_atlas, texel, 0) * color;
}
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
    // An entity's texel as entity.wgsl keeps it (it discards what is under a tenth opaque).
    let entity = hw_entity_quad(c);
    if entity != NO_QUAD { return hw_entity_texel(c, entity).a >= 0.1; }
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
    if layer == rt.water.x || layer == rt.water.y {
        let at = pos + l * c.t + rt.camera.xyz + rt.fraction.xyz;
        return water_caustics(at.xz, rt.water.z) * exp(-WATER_ABSORPTION * c.t);
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
    rayQueryInitialize(&q, terrain, RayDesc(0x0u, 0xffu, 0.0, reach, pos + rt.hw_offset.xyz, l));
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
// RT lighting's water (terrain.wgsl `rt_water`, shadow_rays_hw.wgsl `hw_tint`): the surface's
// waves, a few long and short ones crossing, at world `p` (x, z) and `t` seconds: the height and
// its slopes along x and z; and the caustics they focus on what lies under them, from the
// surface's curvature (1 on average, bright where the waves gather the light).
fn water_waves(p: vec2<f32>, t: f32) -> vec3<f32> {
    let dirs = array<vec2<f32>, 4>(vec2(0.96, 0.28), vec2(-0.51, 0.86), vec2(0.66, -0.75),
        vec2(-0.2, -0.98));
    // A calm surface: a slight swell and finer ripples (the slopes stay within a few degrees).
    let ks = vec4(1.3, 2.9, 6.1, 11.7);
    let amps = vec4(0.012, 0.006, 0.003, 0.0012);
    let speeds = vec4(1.1, 1.4, 1.9, 2.6);
    var out = vec3(0.0);
    for (var i = 0; i < 4; i++) {
        let phase = ks[i] * dot(dirs[i], p) - speeds[i] * t;
        out.x += amps[i] * sin(phase);
        let slope = amps[i] * ks[i] * cos(phase);
        out.y += slope * dirs[i].x;
        out.z += slope * dirs[i].y;
    }
    return out;
}
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
// The nearest opaque triangle from camera-relative `pos` along `l` (unit) within `reach`, as
// Remix's bounce rays meet it: its distance (-1 none), its face's normal toward the ray, its
// texel's colour times the vertex tint in light (the grid's map colour past the first arena page
// or sprite array), and what the translucent triangles before it let through.
fn hw_hit(pos: vec3<f32>, l: vec3<f32>, reach: f32) -> HwHit {
    var q: ray_query;
    rayQueryInitialize(&q, terrain, RayDesc(0x0u, NO_CLOUDS, 0.0, reach, pos + rt.hw_offset.xyz, l));
    var through = vec3(1.0);
    while rayQueryProceed(&q) {
        let c = rayQueryGetCandidateIntersection(&q);
        if c.geometry_index == instances[c.instance_custom_data].w {
            through *= hw_tint(c, pos, l);
        } else if hw_opaque(c) {
            rayQueryConfirmIntersection(&q);
        }
    }
    let hit = rayQueryGetCommittedIntersection(&q);
    if hit.kind == RAY_QUERY_INTERSECTION_NONE {
        return HwHit(-1.0, vec3(0.0), vec3(0.0), through);
    }
    let at = pos + l * hit.t;
    let base = hw_quad(hit, false);
    // The triangle's face from the quad's corners (the instances are only moved, not turned).
    var n = -l;
    var albedo = vec3(-1.0);
    let entity = hw_entity_quad(hit);
    if entity != NO_QUAD {
        // An entity or the view model: its quad's face, its texel times its colour in light.
        let p0 = dyn_position(entity);
        let f = cross(dyn_position(entity + 1u) - p0, dyn_position(entity + 2u) - p0);
        if dot(f, f) > 1e-12 { n = normalize(f); }
        albedo = pow(max(hw_entity_texel(hit, entity).rgb, vec3(0.0)), vec3(2.2));
    } else if base != 0xffffffffu {
        let p0 = hw_position(base);
        let f = cross(hw_position(base + 1u) - p0, hw_position(base + 2u) - p0);
        if dot(f, f) > 1e-12 { n = normalize(f); }
        let texel = hw_texel(hit, base);
        if texel.a >= 0.0 {
            let w = arena[base * 6u + 5u];
            let tint = vec3(f32(w & 255u), f32((w >> 8u) & 255u), f32((w >> 16u) & 255u)) / 255.0;
            albedo = pow(max(texel.rgb * tint, vec3(0.0)), vec3(2.2));
        }
    }
    if dot(n, l) > 0.0 { n = -n; }
    if albedo.x < 0.0 {
        let cell = vec3<i32>(floor(at - n * 0.01 + rt.camera.xyz + rt.fraction.xyz));
        albedo = max(map_albedo(cell), vec3(0.0));
    }
    return HwHit(hit.t, n, albedo, through);
}
// Arena vertex `v`'s section-relative position.
fn hw_position(v: u32) -> vec3<f32> {
    return bitcast<vec3<f32>>(vec3(arena[v * 6u], arena[v * 6u + 1u], arena[v * 6u + 2u]));
}
// Remix's primary surface replacement on water and glass: the ray from the camera through depth
// pixel `p` meets the nearest translucent surface in front of the solid terrain (water, stained
// glass, ice), turns off it (off the waves on water's top) and meets what it reflects,
// lit by the sun's shadow ray and the radiance cache (or the last frame's screen) there; rgb that
// light, a 1, or a 0 where the reflection reaches the sky or no water is met.
fn hw_reflect(p: vec2<i32>) -> vec4<f32> {
    let size = rt.size.xy;
    var d = textureLoad(depth, clamp(p, vec2(0), vec2<i32>(size) - 1), 0);
    let solid = d > 0.0;
    d = max(d, 1e-7);
    let ndc = vec2((f32(p.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (f32(p.y) + 0.5) / size.y * 2.0);
    let c = rt.inverse_view_proj * vec4(ndc, d, 1.0);
    let far = c.xyz / c.w;
    let ray = normalize(far);
    let reach = select(4096.0, length(far), solid);
    var q: ray_query;
    rayQueryInitialize(&q, terrain, RayDesc(0x0u, NO_CLOUDS, 0.0, reach, rt.hw_offset.xyz, ray));
    while rayQueryProceed(&q) {
        let h = rayQueryGetCandidateIntersection(&q);
        if h.geometry_index != instances[h.instance_custom_data].w { continue; }
        if hw_quad(h, true) == 0xffffffffu { continue; }
        rayQueryConfirmIntersection(&q);
    }
    let hit = rayQueryGetCommittedIntersection(&q);
    if hit.kind == RAY_QUERY_INTERSECTION_NONE { return vec4(0.0); }
    let at = ray * hit.t;
    // The waves on the water's top, as its shader's (terrain.wgsl `rt_water`).
    var n = vec3(0.0, 1.0, 0.0);
    let base = hw_quad(hit, true);
    let p0 = hw_position(base);
    let f = cross(hw_position(base + 1u) - p0, hw_position(base + 2u) - p0);
    if dot(f, f) > 1e-12 { n = normalize(f); }
    if dot(n, ray) > 0.0 { n = -n; }
    let layer = f32(arena[base * 6u + 4u] & 0x7fffu);
    if n.y > 0.5 && (layer == rt.water.x || layer == rt.water.y) {
        let world = at + rt.camera.xyz + rt.fraction.xyz;
        let w = water_waves(world.xz, rt.water.z);
        n = normalize(vec3(-w.y, 1.0, -w.z));
    }
    var r = reflect(ray, n);
    if dot(r, n) < 0.0 { r = normalize(r - n * 2.0 * dot(r, n)); }
    let start = at + n * 0.02;
    let h = hw_hit(start, r, 512.0);
    if h.t < 0.0 { return vec4(0.0); }
    let seen = start + r * h.t;
    let lifted = seen + h.normal * 0.02;
    var light = sun_on(lifted, h.normal);
    let last = cached_on(seen, h.normal);
    light += select(open_light(lifted, h.normal, seen), last.rgb, last.a >= 0.0);
    return vec4(h.through * h.albedo * light, 1.0);
}
