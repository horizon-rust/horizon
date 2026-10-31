
// RT lighting's hybrid (gfx/hw_rays.rs): the drawn terrain in hardware acceleration structures,
// for the sun's and the light sources' shadow rays to meet its exact triangles, alpha-tested.
// Appended to shadow_rays.wgsl when the GPU traces rays (`enable wgpu_ray_query;` before it).
const HW: bool = true;
@group(0) @binding(15) var terrain: acceleration_structure;
// The vertex arena's pages (gfx/arena.rs), the first `ARENA_PAGES` (the missing ones bound to the
// first): `TerrainVertex` six words each, position, then the texture coordinate (two halves,
// `UV_STEPS` a unit), then the layer and the light.
@group(0) @binding(16) var<storage, read> arena: array<u32>;
@group(0) @binding(40) var<storage, read> arena1: array<u32>;
@group(0) @binding(41) var<storage, read> arena2: array<u32>;
@group(0) @binding(42) var<storage, read> arena3: array<u32>;
@group(0) @binding(43) var<storage, read> arena4: array<u32>;
@group(0) @binding(44) var<storage, read> arena5: array<u32>;
@group(0) @binding(45) var<storage, read> arena6: array<u32>;
@group(0) @binding(46) var<storage, read> arena7: array<u32>;
const ARENA_PAGES: u32 = 8u;
// Word `k` of arena vertex `v`: its page in bits 28 to 30 (`hw_quad`), its vertex in that page below.
fn arena_at(v: u32, k: u32) -> u32 {
    let i = (v & 0x0fffffffu) * 6u + k;
    switch v >> 28u {
        case 0u: { return arena[i]; }
        case 1u: { return arena1[i]; }
        case 2u: { return arena2[i]; }
        case 3u: { return arena3[i]; }
        case 4u: { return arena4[i]; }
        case 5u: { return arena5[i]; }
        case 6u: { return arena6[i]; }
        default: { return arena7[i]; }
    }
}
// The first sprite array, for the alpha test.
@group(0) @binding(17) var sprites: texture_2d_array<f32>;
const UV_STEPS: f32 = 32768.0;
// Arena vertex `v`'s texture coordinate.
fn hw_uv(v: u32) -> vec2<f32> {
    let w = arena_at(v, 3u);
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
@group(0) @binding(36) var<storage, read> dynamics: array<u32>;
@group(0) @binding(37) var entity_atlas: texture_2d<f32>;
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
// The quad candidate hit `c` is on: its first arena vertex (`arena_at`), or none (`u32` max) on a
// page past those bound or on none (moving geometry, a structure whose vertices went).
fn hw_quad(c: RayIntersection, translucent: bool) -> u32 {
    let at = instances[c.instance_custom_data];
    if at.x >= ARENA_PAGES { return 0xffffffffu; }
    return at.x << 28u | (select(at.y, at.z, translucent) + (c.primitive_index / 2u) * 4u);
}
// The texel candidate hit `c` meets on the quad from arena vertex `base`, and its sprite layer
// (past the first array: none, -1).
fn hw_texel(c: RayIntersection, base: u32) -> vec4<f32> {
    let layer = arena_at(base, 4u) & 0x7fffu;
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
// Whether candidate hit `c` (a solid quad) meets an opaque texel. Opaque leaves, quads on arena
// pages past those bound and sprites past the first array are taken as opaque.
fn hw_opaque(c: RayIntersection) -> bool {
    // An entity's texel as entity.wgsl keeps it (it discards what is under a tenth opaque).
    let entity = hw_entity_quad(c);
    if entity != NO_QUAD { return hw_entity_texel(c, entity).a >= 0.1; }
    let base = hw_quad(c, false);
    if base == 0xffffffffu { return true; }
    if (arena_at(base, 4u) & 0x8000u) != 0u { return true; }
    let texel = hw_texel(c, base);
    return texel.a < 0.0 || texel.a >= 0.5;
}
// What candidate hit `c` (a translucent quad: stained glass, water, ice) lets through: the light
// filtered by its colour (its texel times its tint, in light), as much as it is opaque.
fn hw_tint(c: RayIntersection, pos: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let base = hw_quad(c, true);
    if base == 0xffffffffu { return vec3(1.0); }
    // Water: the ray starts under it (`c.t` is the length crossed to reach its surface, so the
    // depth is that times the ray's rise). The sun's light came down through the surface bent by
    // Snell's law, along a steeper line than `l`: the waves' caustics at the floor point (what
    // those waves focus on it, a network continuous across blocks), the light lost on the way
    // along that longer path in Remix's medium (and the biome's colour), and what the surface
    // reflected of it (Fresnel). A ray that grazes the surface or goes down gets no caustics.
    let layer = f32(arena_at(base, 4u) & 0x7fffu);
    if layer == rt.water.x || layer == rt.water.y {
        let w = arena_at(base, 5u);
        let tint = vec3(f32(w & 255u), f32((w >> 8u) & 255u), f32((w >> 16u) & 255u)) / 255.0;
        let absorption = WATER_ABSORPTION + WATER_PURE_ABSORPTION + WATER_TINT_ABSORPTION * (1.0 - pow(tint, vec3(2.2)));
        if l.y < 0.02 { return exp(-absorption * c.t); }
        let depth = c.t * l.y;
        let bent = refract(-l, vec3(0.0, 1.0, 0.0), 1.0 / WATER_INDEX);
        let floor_at = pos + rt.camera.xyz + rt.fraction.xyz;
        let footprint = length(pos) * 1.5 / rt.size.y;
        // How open the water lies, in its colour's alpha (mesher.rs `with_sea`).
        let focus = water_caustics(floor_at.xz, depth, l, rt.water.z, footprint,
            f32(w >> 28u) / 15.0);
        let fresnel = 0.02 + 0.98 * pow(1.0 - l.y, 5.0);
        return focus * (1.0 - fresnel) * exp(-absorption * depth / max(-bent.y, 0.1));
    }
    // Glass, and every other translucent block: each face it crosses is a wall (`glass_wall`).
    let p0 = hw_position(base);
    let f = cross(hw_position(base + 1u) - p0, hw_position(base + 2u) - p0);
    if dot(f, f) < 1e-12 { return vec3(1.0); }
    return glass_wall(hw_glass(c, base), abs(dot(normalize(f), l)));
}
// RT lighting's glass, Remix's translucent material as RTX Remix for Minecraft gives the block
// atlas (its `BlockTextureManager`): a dielectric of index 1.52 whose transmittance is 0.97 over
// 0.35 block (`transmittanceMeasurementDistance`), times the texel's colour for stained glass,
// panes and the other translucent blocks (the atlas is their transmittance texture); the plain
// glass (`rt.frame.y`, its block's and its pane edge's sprites packed as the lava's) keeps the grey
// 0.97 under a diffuse layer of its texel, as opaque as the texel's alpha (`useDiffuseLayer`).
// Over a whole block that colour would leave red glass black (3 % of red light through), so the
// glass is thin-walled (Remix's `thinWalled`), the measurement distance shared between its two
// walls: a block or a pane seen square on shows its texture's colour, deeper and darker as the ray
// slants through the walls. The same in the glass's shader (terrain.wgsl `rt_glass_at`) and the
// froxels' rays.
const GLASS_INDEX: f32 = 1.52;
const GLASS_TRANSMITTANCE: f32 = 0.97;
const GLASS_DISTANCE: f32 = 0.35;
const GLASS_WALL: f32 = 0.175;
// What a wall of glass `g` (`hw_glass`) lets through of a ray meeting it at `cosine` to its
// normal: what Fresnel does not reflect, Beer-Lambert over the wall's thickness along the ray bent
// inside it (Snell's law), less what the diffuse layer stops.
fn glass_wall(g: vec4<f32>, cosine: f32) -> vec3<f32> {
    let c = clamp(cosine, 0.0, 1.0);
    let slant = sqrt(1.0 - (1.0 - c * c) / (GLASS_INDEX * GLASS_INDEX));
    return (1.0 - glass_fresnel(c)) * (1.0 - g.a) * exp(-g.rgb * GLASS_WALL / slant);
}
// A dielectric's reflectance at index 1.52 (Schlick's, from F0 = ((n - 1) / (n + 1))^2), the
// cosine of the angle on the air's side.
fn glass_fresnel(cosine: f32) -> f32 {
    return 0.0426 + 0.9574 * pow(1.0 - clamp(cosine, 0.0, 1.0), 5.0);
}
fn hw_glass_plain(layer: f32) -> bool {
    let packed = rt.frame.y;
    if packed < 0.0 { return false; }
    let pane = floor(packed / 4096.0);
    return layer == packed - pane * 4096.0 || layer == pane;
}
// The glass of the translucent quad from arena vertex `base` that hit `c` meets: its attenuation
// coefficient a block (rgb, -ln of its transmittance over the measurement distance) and its
// diffuse layer's opacity (a).
fn hw_glass(c: RayIntersection, base: u32) -> vec4<f32> {
    let clear = vec3(-log(GLASS_TRANSMITTANCE) / GLASS_DISTANCE);
    let texel = hw_texel(c, base);
    if texel.a < 0.0 { return vec4(clear, 0.0); }
    if hw_glass_plain(f32(arena_at(base, 4u) & 0x7fffu)) {
        return vec4(clear, clamp(texel.a, 0.0, 1.0));
    }
    let w = arena_at(base, 5u);
    let tint = vec3(f32(w & 255u), f32((w >> 8u) & 255u), f32((w >> 16u) & 255u)) / 255.0;
    let colour = pow(max(texel.rgb * tint, vec3(0.0)), vec3(2.2));
    return vec4(-log(max(colour * GLASS_TRANSMITTANCE, vec3(1e-3))) / GLASS_DISTANCE, 0.0);
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
// RT lighting's water (terrain.wgsl `rt_water`, shadow_rays_hw.wgsl `hw_tint` and `hw_reflect`): the
// surface's waves at world `p` (x, z) and `t` seconds, a sum of sixteen plane waves of every
// length from a long swell (6 m) down to capillary ripples (21 cm), their directions turning by
// the golden angle so no two line up, each travelling at its water-wave speed (gravity and surface
// tension). Every wave is the same slope's worth of sine whatever its length, capped so the
// finest ones keep their curvature (what focuses light) without roughening the glints: the height,
// its slopes and its curvature (the second derivatives along x, xz and z) are exact sums, the
// same in the shader that shows the water, the one that traces what it reflects and refracts, and
// the one that traces the sun's light through it (these three copies are identical).
struct WaterSurface {
    height: f32,
    slope: vec2<f32>,
    curvature: vec3<f32>,
}
// One plane wave: `d` its direction (x, z), its wavenumber (rad/m) and its slope's amplitude; `w`
// its angular frequency and its phase. The sixteen below are the law above written out, the
// directions turning 2.39996 rad (the golden angle) a wave, the wavenumbers 1.04 rad/m growing
// by 1.25 a wave (6 m to 21 cm), the slope min(1 / k, 0.06 min(1, k / 2.4)), the frequency
// 0.6 sqrt(9.81 k + 7.3e-5 k^3) (gravity and surface tension, blocks being metres; slowed to
// 0.6, a calmer sea than dispersion alone gives), the phase 2.1 a wave. Written out, no shader has to work them out: each costs a sine and a cosine.
fn water_wave(p: vec2<f32>, t: f32, footprint: f32, d: vec4<f32>, w: vec2<f32>) -> WaterSurface {
    // A ripple finer than about a pixel only shimmers: it fades out, and costs nothing then.
    let fade = 1.0 - smoothstep(1.5, 4.0, d.z * footprint);
    if fade <= 0.0 { return WaterSurface(0.0, vec2(0.0), vec3(0.0)); }
    let slope = d.w * fade;
    let phase = d.z * dot(d.xy, p) - w.x * t + w.y;
    let s = sin(phase);
    let dir = d.xy;
    return WaterSurface(slope / d.z * s, slope * cos(phase) * dir,
        -slope * d.z * s * vec3(dir.x * dir.x, dir.x * dir.y, dir.y * dir.y));
}
fn water_sum(a: WaterSurface, b: WaterSurface) -> WaterSurface {
    return WaterSurface(a.height + b.height, a.slope + b.slope, a.curvature + b.curvature);
}
// The sea state varies over the water, as wind and fetch make it vary on a real lake: three gains
// on the waves' slopes, smooth over the swell's 230 m, the wind sea's 70 m and the ripples' 23 m
// (the cat's paws, gusts drifting with the wind), about a mean of 1 (0.4 to 1.6, 0.15 to 1.85), from
// a value noise of the world position (no period: a lake 16 km across never shows the same water
// twice). The gains change slowly enough for their own slopes to be neglected.
fn water_hash(c: vec2<i32>) -> f32 {
    var h = (u32(c.x) * 0x8da6b343u) ^ (u32(c.y) * 0xd8163841u);
    h = (h ^ (h >> 15u)) * 0x2c1b3c6du;
    h = (h ^ (h >> 12u)) * 0x297a2d39u;
    return f32((h ^ (h >> 15u)) & 0xffffu) / 65535.0;
}
fn water_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    return mix(mix(water_hash(c), water_hash(c + vec2(1, 0)), u.x),
        mix(water_hash(c + vec2(0, 1)), water_hash(c + vec2(1, 1)), u.x), u.y);
}
// `sea` is how open the water lies (terrain.wgsl `TerrainOut::sea`: 0 a swamp's still water, 0.6 a
// lake's, 1 the open sea's): the swell and the wind sea grow with it, still water hardly moves.
fn water_sea(q: vec2<f32>, t: f32, sea: f32) -> vec3<f32> {
    let still = mix(vec3(0.1, 0.25, 0.4), vec3(1.0), smoothstep(0.0, 0.6, sea));
    let open = mix(vec3(1.0), vec3(1.8, 1.3, 1.1), smoothstep(0.6, 1.0, sea));
    let swell = water_noise(q / 233.0 + vec2(0.003, 0.001) * t);
    let wind = water_noise(q / 71.0 + vec2(17.3, 5.1) + vec2(0.006, -0.004) * t);
    let gusts = water_noise(q / 23.0 + vec2(-3.7, 11.9) + vec2(0.045, 0.02) * t);
    // The wind's gains contrast more than the swell's: calm slicks beside rippled patches.
    let patches = smoothstep(vec2(0.15), vec2(0.85), vec2(wind, gusts));
    return vec3(0.4 + 1.2 * swell, 0.15 + 1.7 * patches) * still * open;
}
fn water_surface(q: vec2<f32>, t: f32, footprint: f32, sea: f32) -> WaterSurface {
    var s = WaterSurface(0.0, vec2(0.0), vec3(0.0));
    // The crests are not straight lines: a slow swirl bends the coordinates every wave is read at,
    // and a noise over a hundred metres and thirty turns their lines on the scale of a lake, so
    // they never run parallel far (its own slopes are neglected: it turns them by a few degrees).
    let swirl = 0.6 * vec2(sin(0.31 * q.y + 0.23 * t + 1.3), sin(0.27 * q.x - 0.19 * t));
    let bend = 12.0 * (vec2(water_noise(q / 97.0 + vec2(31.1, 7.7)),
        water_noise(q / 97.0 + vec2(-12.4, 23.5))) - 0.5)
        + 3.6 * (vec2(water_noise(q / 29.0 + vec2(5.3, -19.2)),
        water_noise(q / 29.0 + vec2(40.6, 2.9))) - 0.5);
    let p = q + swirl + bend;
    // The sea state there: the swell's, the wind sea's and the ripples' gains (`water_sea`).
    let g = water_sea(q, t, sea);
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.92106, 0.38942, 1.0400, 0.02600 * g.x), vec2(1.9165, 0.0)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.94221, 0.33503, 1.3000, 0.03250 * g.x), vec2(2.1427, 2.1)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.46845, -0.88349, 1.6250, 0.04063 * g.x), vec2(2.3956, 4.2)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.25138, 0.96789, 2.0312, 0.05078 * g.x), vec2(2.6784, 6.3)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.83916, -0.54389, 2.5391, 0.06000 * g.x), vec2(2.9946, 8.4)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.98616, -0.16580, 3.1738, 0.06000 * g.y), vec2(3.3481, 10.5)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.61516, 0.78840, 3.9673, 0.06000 * g.y), vec2(3.7433, 12.6)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.07896, -0.99688, 4.9591, 0.06000 * g.y), vec2(4.1853, 14.7)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.73160, 0.68173, 6.1989, 0.06000 * g.y), vec2(4.6796, 16.8)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.99996, -0.00849, 7.7486, 0.06000 * g.y), vec2(5.2323, 18.9)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.74308, -0.66921, 9.6858, 0.06000 * g.y), vec2(5.8506, 21.0)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.09587, 0.99539, 12.1072, 0.06000 * g.z), vec2(6.5425, 23.1)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.60169, -0.79873, 15.1340, 0.06000 * g.z), vec2(7.3170, 25.2)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.98320, 0.18252, 18.9175, 0.05286 * g.z), vec2(8.1846, 27.3)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(-0.84827, 0.52956, 23.6469, 0.04229 * g.z), vec2(9.1574, 29.4)));
    s = water_sum(s, water_wave(p, t, footprint, vec4(0.26778, -0.96348, 29.5586, 0.03383 * g.z), vec2(10.2503, 31.5)));
    return s;
}
// The height and the slopes along x and z, for the shaders that only look at the surface.
fn water_waves(p: vec2<f32>, t: f32, footprint: f32, sea: f32) -> vec3<f32> {
    let w = water_surface(p, t, footprint, sea);
    return vec3(w.height, w.slope);
}
// Water's index of refraction at red (656 nm), green (550 nm) and blue (450 nm): the shorter the
// wavelength, the slower the light, so the more the surface bends it (measured values; a Cauchy
// fit n = 1.3199 + 3298 / lambda^2 is 0.003 low across the board, its spread is the same).
const WATER_INDEX_RGB: vec3<f32> = vec3(1.3311, 1.3337, 1.3378);
// Where a sun ray (`l` toward the sun) goes after the surface bends it by Snell's law (`index`
// the water's, for the colour in question) at a surface with `slope`: its horizontal travel per
// metre of depth.
fn water_bent(slope: vec2<f32>, l: vec3<f32>, index: f32) -> vec2<f32> {
    let n = normalize(vec3(-slope.x, 1.0, -slope.y));
    let d = refract(-l, n, 1.0 / index);
    return d.xz / max(-d.y, 1e-3);
}
// The area ratio of the surface-to-floor map of one colour: the surface's curvature through
// Snell's law's own derivative at `slope` for water of `index`, over `depth` metres.
fn water_area(slope: vec2<f32>, curvature: vec3<f32>, depth: f32, l: vec3<f32>, index: f32)
    -> f32 {
    let r = water_bent(slope, l, index);
    let e = 0.01;
    let to_slope = mat2x2<f32>((water_bent(slope + vec2(e, 0.0), l, index) - r) / e,
        (water_bent(slope + vec2(0.0, e), l, index) - r) / e);
    let hessian = mat2x2<f32>(vec2(curvature.x, curvature.y), vec2(curvature.y, curvature.z));
    return determinant(mat2x2<f32>(vec2(1.0, 0.0), vec2(0.0, 1.0)) + depth * (to_slope * hessian));
}
// What the soft area ratio averages to over the waves at `depth` (measured over many sea states,
// and sun heights): one sample of it by the surface point a flat surface would send the ray from
// weighs the folds' piles by the surface's area and not the floor's, so past a metre it runs high
// before the washing-out of deeper water brings it down. Divided out, the light stays the sun's
// on average (a floor, and the froxels' air above it, would else turn white).
fn water_mean(d: f32) -> f32 {
    return 1.0 + 0.03 * clamp(d, 0.0, 1.0) + 0.57 * clamp(d - 1.0, 0.0, 1.0)
        + 0.65 * clamp(d - 2.0, 0.0, 1.0) + 0.15 * clamp(d - 3.0, 0.0, 1.0)
        - 0.3 * clamp(d - 4.0, 0.0, 1.0) - 0.4 * clamp(d - 5.0, 0.0, 1.0)
        - 0.275 * clamp(d - 6.0, 0.0, 2.0) - 0.175 * clamp(d - 8.0, 0.0, 2.0)
        - 0.0875 * clamp(d - 10.0, 0.0, 4.0);
}
// The sunlight a point of the floor `depth` metres under the surface, at world `p` (x, z), gets
// relative to a flat surface (1 on average) at `t` seconds, for red, green and blue, `l` the
// unit direction to the sun and `footprint` the world size of the pixel looking at it. The
// light's ray is followed up through the waves: the surface point a flat surface would send the
// ray from (the waves' own bending moves it by centimetres, small against the network's scale),
// then the Jacobian of the surface-to-floor map there, the surface's curvature through Snell's
// law's own derivative (the first-order approximation of the focusing every caustic renderer
// makes): the energy a bundle of rays carries is conserved, so the floor's light is the inverse
// of its area ratio. Where the map folds, that area goes to zero and the light piles up in a thin
// bright line: the caustic network, continuous across blocks since it is a function of the world
// position alone, finer the shallower the water (the light's reach is short, only the finest
// ripples focus it). The colours do not cross the water alike (dispersion): each has its own
// index, so its own refracted rays and area ratio over the same waves, and the lines where they
// pile up part by a fraction of their width, red's on the outside: the rainbow edge of a real
// caustic. The sun's disc and the water's scattering wash it out with depth, and far from the
// eye it blurs to its mean, rather than shimmer.
fn water_caustics(p: vec2<f32>, depth: f32, l: vec3<f32>, t: f32, footprint: f32, sea: f32)
    -> vec3<f32> {
    // Deep down the network is washed out and finer than a pixel: nothing to compute.
    if depth > 14.0 { return vec3(1.0); }
    let x = p - depth * water_bent(vec2(0.0), l, WATER_INDEX_RGB.y);
    let w = water_surface(x, t, footprint, sea);
    let det = vec3(water_area(w.slope, w.curvature, depth, l, WATER_INDEX_RGB.x),
        water_area(w.slope, w.curvature, depth, l, WATER_INDEX_RGB.y),
        water_area(w.slope, w.curvature, depth, l, WATER_INDEX_RGB.z));
    // Soft at the fold (the sun's disc has a size, the line a width), never infinite.
    let eps = 0.018 + 0.005 * depth;
    let gathered = 1.0 / sqrt(det * det + eps * eps) / water_mean(depth);
    let keeps = exp(-0.07 * depth) * (1.0 - smoothstep(0.06, 0.25, footprint));
    return mix(vec3(1.0), gathered, keeps);
}
// Remix's water medium (terrain.wgsl `WATER_ABSORPTION`): its extinction a block (a metre).
const WATER_ABSORPTION: vec3<f32> = vec3(0.025541, 0.005268, 0.005827);
const WATER_TINT_ABSORPTION: f32 = 0.06;
// What real water adds to Remix's clear medium (terrain.wgsl `WATER_PURE_ABSORPTION`).
const WATER_PURE_ABSORPTION: vec3<f32> = vec3(0.30, 0.045, 0.012);
// The nearest opaque triangle from camera-relative `pos` along `l` (unit) within `reach`, as
// Remix's bounce rays meet it: its distance (-1 none), its face's normal toward the ray, its
// texel's colour times the vertex tint in light (the grid's map colour past the first arena page
// or sprite array), and what the translucent triangles before it let through; the light
// sources' light by its block light level in `hw_level`.
fn hw_hit(pos: vec3<f32>, l: vec3<f32>, reach: f32) -> HwHit {
    hw_level = vec3(0.0);
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
        // The light sources' light by the quad's block light level (the light's high byte, 16
        // a level), the steps from the light it stands for squared (terrain.wgsl `rt_shade`'s law).
        let block = f32((arena_at(base, 4u) >> 24u) & 255u) / 16.0;
        let steps = max(15.0 - block, 0.0);
        hw_level = select(vec3(0.0), HW_BLOCK_LIGHT / ((1.0 + steps) * (1.0 + steps)), block > 0.0);
        if texel.a >= 0.0 {
            let w = arena_at(base, 5u);
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
// The light sources' light on the surface `hw_hit` last met by its block light level (terrain.wgsl
// `BLOCK_LIGHT` over the square of the steps from the light), which what the primary surface
// replacement sees takes where the radiance cache holds less (`hw_seen_light`): the cache's cell,
// coarser, averages the light near a lamp with the dark around it.
var<private> hw_level: vec3<f32>;
const HW_BLOCK_LIGHT: vec3<f32> = vec3(3.0, 1.9, 0.9);
// The light other than the sun's on a surface the primary surface replacement sees (`hw_hit`'s,
// at camera-relative `seen`, normal `n`): the radiance cache's, or the open sky's guess, at least
// its block light level's (`hw_level`); on a light source itself and what touches it (a
// lantern, a torch, a lamp, in `lights`: within a block of its point), at least that source's
// light at point-blank range as the pixels' own rays give it (`light_path`, no shadow ray so
// close), which the cache's coarse cell averages away: it shines as seen directly.
fn hw_seen_light(seen: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let lifted = seen + n * 0.02;
    let last = cached_on(seen, n);
    var light = max(select(open_light(lifted, n, seen), last.rgb, last.a >= 0.0), hw_level);
    let local = lifted + rt.camera.xyz + rt.fraction.xyz - vec3(rt.grid.x, -64.0, rt.grid.y);
    let words = light_cell(local);
    for (var k = 0; k < 8; k++) {
        let i = cell_light(words, k);
        if i == 0u { continue; }
        if distance(lights[i - 1u].at.xyz, local) < 1.0 {
            light = max(light, light_path(i, local, n, vec3(0.0)).light);
        }
    }
    return light;
}
// Arena vertex `v`'s section-relative position.
fn hw_position(v: u32) -> vec3<f32> {
    return bitcast<vec3<f32>>(vec3(arena_at(v, 0u), arena_at(v, 1u), arena_at(v, 2u)));
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
    water_at = vec4(0.0, 0.0, 0.0, -1.0);
    glass_at = vec4(0.0, 0.0, 0.0, -1.0);
    if hit.kind == RAY_QUERY_INTERSECTION_NONE {
        // No water or glass in front: maybe a puddle on the solid terrain.
        if solid { return hw_puddle(far, ray); }
        return vec4(0.0);
    }
    let at = ray * hit.t;
    // The waves on the water's top, as its shader's (terrain.wgsl `rt_water`).
    var n = vec3(0.0, 1.0, 0.0);
    let base = hw_quad(hit, true);
    let p0 = hw_position(base);
    let f = cross(hw_position(base + 1u) - p0, hw_position(base + 2u) - p0);
    if dot(f, f) > 1e-12 { n = normalize(f); }
    if dot(n, ray) > 0.0 { n = -n; }
    let layer = f32(arena_at(base, 4u) & 0x7fffu);
    if n.y > 0.5 && (layer == rt.water.x || layer == rt.water.y) {
        let world = at + rt.camera.xyz + rt.fraction.xyz;
        // The pixel's footprint stretched by the slant (terrain.wgsl `water_footprint`).
        let w = water_waves(world.xz, rt.water.z,
            hit.t * 1.5 / rt.size.y / max(abs(n.y * ray.y), 0.03), f32(arena_at(base, 5u) >> 28u) / 15.0);
        // And the rain's ripples, as the water's shader has them (terrain.wgsl `rt_rain_ripples`).
        let r = hw_rain_ripples(world, hit.t);
        n = normalize(vec3(-w.y - r.x, 1.0, -w.z - r.y));
    }
    // Under the water, looking up at its top: the waves seen from below.
    if n.y < -0.5 && (layer == rt.water.x || layer == rt.water.y) {
        let world = at + rt.camera.xyz + rt.fraction.xyz;
        // The pixel's footprint stretched by the slant (terrain.wgsl `water_footprint`).
        let w = water_waves(world.xz, rt.water.z,
            hit.t * 1.5 / rt.size.y / max(abs(n.y * ray.y), 0.03), f32(arena_at(base, 5u) >> 28u) / 15.0);
        n = -normalize(vec3(-w.y, 1.0, -w.z));
    }
    // Its top (seen from above or below) refracts; its sides blend as before. Glass refracts on
    // every face (shadow_rays_psr.wgsl `hw_refract_glass`).
    if abs(n.y) > 0.5 && (layer == rt.water.x || layer == rt.water.y) { water_at = vec4(n, hit.t); }
    if layer != rt.water.x && layer != rt.water.y {
        glass_at = vec4(n, hit.t);
        glass_sigma = hw_glass(hit, base).rgb;
    }
    var r = reflect(ray, n);
    if dot(r, n) < 0.0 { r = normalize(r - n * 2.0 * dot(r, n)); }
    return hw_mirror(at + n * 0.02, r);
}
// What a mirror at camera-relative `start` reflects along `r`: the surface met, lit by the sun's
// shadow ray and the radiance cache (or the last frame's screen) there; a 0 at the sky.
fn hw_mirror(start: vec3<f32>, r: vec3<f32>) -> vec4<f32> {
    let h = hw_hit(start, r, 512.0);
    if h.t < 0.0 { return vec4(0.0); }
    let seen = start + r * h.t;
    let light = sun_on(seen + h.normal * 0.02, h.normal) + hw_seen_light(seen, h.normal);
    return vec4(h.through * h.albedo * light, 1.0);
}
// The rain's ripples (slopes) on the water's top at world `world`, `t` blocks from the camera,
// where the rain falls on it (terrain.wgsl `rt_rain_ripples`).
fn hw_rain_ripples(world: vec3<f32>, t: f32) -> vec2<f32> {
    if rt.rain.x <= 0.0 { return vec2(0.0); }
    let w = rain_cell(vec2<i32>(floor(world.xz)));
    if w == 0u || !rain_falls(w) || rain_surface(w) != 10u || world.y < rain_height(w) - 1.2 {
        return vec2(0.0);
    }
    return rain_ripples(world.xz, rt.water.z, rt.rain.x, t * 1.5 / rt.size.y, 0.2, 1.0) * RAIN_RING;
}
// A puddle or a wet film on the solid terrain at camera-relative `at` (terrain.wgsl `rt_wet`; a
// puddle as deep as it can be there: the pack's relief unknown, the sunlit puddles' level or the
// shade's): its flat water, a puddle's rippled while it rains, reflects what the ray off it meets;
// a 0 where there is neither or the reflection reaches the sky.
fn hw_puddle(at: vec3<f32>, ray: vec3<f32>) -> vec4<f32> {
    let level = max(rt.rain.z, rt.rain_sun.y);
    if level <= 0.0 && max(rt.rain.y, rt.rain_sun.x) <= 0.2 { return vec4(0.0); }
    let world = at + rt.camera.xyz + rt.fraction.xyz;
    var slope = vec2(0.0);
    if rain_puddle(world, level, 0.8) > 0.0 {
        slope = rain_ripples(world.xz, rt.water.z, rt.rain.x, length(at) * 1.5 / rt.size.y, 0.2,
            1.0) * RAIN_RING;
    } else if max(rt.rain.y, rt.rain_sun.x) <= 0.2 || !rain_film(world) {
        return vec4(0.0);
    }
    let n = normalize(vec3(-slope.x, 1.0, -slope.y));
    var r = reflect(ray, n);
    r.y = abs(r.y);
    return hw_mirror(at + vec3(0.0, 0.02, 0.0), r);
}
