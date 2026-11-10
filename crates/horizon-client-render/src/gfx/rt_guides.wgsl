
// DLSS Ray Reconstruction's guides (gfx/rt_guides.rs, docs/rt-dlss-rr.md): appended to the hardware
// kernel (shadow_rays.rs `source(true, ..)`, then this) only when `rt_guides::guides_wanted()`, as
// a module of its own, so the kernel the game runs otherwise is the very text it was. It reads the
// kernel's group 0 as the kernel binds it for the view (the same bind group: `rt`, the structures,
// the arena, the sprites, the grid, the rain map) and its own group 1: the depth it works from, a
// texel a depth pixel, and the guides it writes.
//
// The depth (`guide_depth`) is the RT prepass's (the solid terrain, unjittered) or, on DLSS-RR's
// frames, the world pass's own: jittered as the colour is, with the entities, water and glass in
// it. `guide_params.jitter` says how far that image moved (pixels, right and down), so each pixel
// is read where its sample lies on the unjittered grid `rt`'s matrices describe.
//
// What each pixel gets, at the surface the depth shows:
// - a short ray across that surface (`hw_hit` from just before it to just past it: its exact
//   triangle, alpha-tested, and its texel; the entities' and the view model's quads are in the
//   same structure), whose albedo (linear, the texel times the vertex tint) and geometric face
//   normal are taken;
// - where that ray finds water or glass instead (`guide_cover`), the transparent surface's face
//   normal (its depth is the surface's: the guides stay consistent with it, as NVIDIA's guide asks
//   when no specular motion is given), and albedos and roughness blended between what lies behind
//   it (along the unrefracted ray) and the surface's mirror by its Fresnel term;
// - otherwise (geometry outside the structures, far terrain), the depth's own normal and the
//   voxel grid's map colour of the block (else mid grey);
// - roughness 1 and F0 0.04 (the kernel reads no LabPBR specular map), except where the kernel's
//   reflections mirror the rain's puddles (roughness 0.05) and wet films (0.2), F0 0.02 (water);
// - the specular albedo, the preintegrated environment BRDF (`env_brdf_approx`) of that F0 and
//   roughness at the pixel's N.V, not F0;
// - the motion: where the surface was on last frame's screen less where it is (unjittered pixels,
//   x right, y down), from the depth and last frame's matrix (`rt.prev_view_proj`, `rt.motion`)
//   as the kernel's own reprojection takes it; the sky by its direction alone; an entity's own
//   motion where `guide_objects` has one (frame generation's `ObjectMotion`).
// The sky gets NVIDIA's defaults: diffuse albedo 0.5, specular albedo, normal and roughness 0.
@group(1) @binding(0) var guide_diffuse: texture_storage_2d<rgba8unorm, write>;
@group(1) @binding(1) var guide_specular: texture_storage_2d<rgba8unorm, write>;
// World-space unit normal (xyz, toward the camera), roughness in w (`DLSS.Roughness.Mode` packed).
@group(1) @binding(2) var guide_normals: texture_storage_2d<rgba16float, write>;
// Render pixels from this frame's unjittered position to last frame's: previous - current.
@group(1) @binding(3) var guide_motion: texture_storage_2d<rg16float, write>;
// `HORIZON_RT_GUIDES_SHOW`'s picture of one guide, for the screen (1x1 and unwritten otherwise).
@group(1) @binding(4) var guide_show: texture_storage_2d<rgba8unorm, write>;
// The depth the guides are made from (reversed Z), at the guides' size.
@group(1) @binding(5) var guide_depth: texture_depth_2d;
// The entities' own motion (frame_gen/motion.wgsl: UV toward last frame, z 1 where an entity
// shows), a 1x1 stand-in without it.
@group(1) @binding(6) var guide_objects: texture_2d<f32>;
struct GuideParams {
    // xy: the depth's jitter in pixels (image moved right and down); z: 1 with `guide_objects`.
    jitter: vec4<f32>,
}
@group(1) @binding(7) var<uniform> guide_params: GuideParams;
// Which guide `guide_show` pictures (rt_guides.rs `Show`): 0 none, 1 the diffuse albedo, 2 the
// normals, 3 the roughness, 4 the motion, 5 the specular albedo, 6 where each guide came from,
// 7 the depth's distance.
override GUIDES_SHOW: u32 = 0u;
// The F0 of a dielectric (Remix's and Unreal's default), of water, and of glass.
const GUIDE_F0: f32 = 0.04;
const GUIDE_WATER_F0: f32 = 0.02;
const GUIDE_GLASS_F0: f32 = 0.04;
// What lies behind water or glass is looked for this far past it.
const GUIDE_BEHIND: f32 = 64.0;
// The preintegrated environment BRDF's scale and bias on F0 (Karis 2014, "Physically Based Shading
// on Mobile", after Lazarov 2013): what a split-sum renderer multiplies its prefiltered
// reflection by, the specular albedo DLSS-RR asks for (its guide gives an approximation of the
// same quantity; this is the published one, not NVIDIA's text).
fn env_brdf_approx(f0: vec3<f32>, roughness: f32, n_dot_v: f32) -> vec3<f32> {
    let c0 = vec4(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    let ab = vec2(-1.04, 1.04) * a004 + r.zw;
    return clamp(f0 * ab.x + ab.y, vec3(0.0), vec3(1.0));
}
// Where camera-relative homogeneous point `h` (w 0: a direction, the sky) lay on last frame's
// screen, in pixels, or none (w below 0 there, no last frame).
fn guide_last_pixel(h: vec4<f32>) -> vec3<f32> {
    if rt.motion.w < 0.5 { return vec3(0.0); }
    let c = rt.prev_view_proj * h;
    if c.w <= 1e-6 { return vec3(0.0); }
    let ndc = c.xy / c.w;
    return vec3((ndc.x * 0.5 + 0.5) * rt.size.x, (0.5 - ndc.y * 0.5) * rt.size.y, 1.0);
}
// Unjittered pixel position `q` in normalised device coordinates.
fn guide_ndc(q: vec2<f32>) -> vec2<f32> {
    return vec2(q.x / rt.size.x * 2.0 - 1.0, 1.0 - q.y / rt.size.y * 2.0);
}
// Where depth pixel `p`'s sample lies on the unjittered grid.
fn guide_centre(p: vec2<i32>) -> vec2<f32> {
    return vec2<f32>(p) + 0.5 - guide_params.jitter.xy;
}
// The camera-relative position depth pixel `p` shows (`surface_at` on the guides' depth).
fn guide_surface(p: vec2<i32>) -> vec3<f32> {
    let d = textureLoad(guide_depth, clamp(p, vec2(0), vec2<i32>(rt.size.xy) - 1), 0);
    let c = rt.inverse_view_proj * vec4(guide_ndc(guide_centre(p)), d, 1.0);
    return c.xyz / c.w;
}
// The surface's normal at `p` (at `pos`) from its neighbours (`surface_normal` on the guides'
// depth): on each axis the side nearer in depth.
fn guide_normal(p: vec2<i32>, pos: vec3<f32>) -> vec3<f32> {
    let r = guide_surface(p + vec2(1, 0)) - pos;
    let l = pos - guide_surface(p - vec2(1, 0));
    let d = guide_surface(p + vec2(0, 1)) - pos;
    let u = pos - guide_surface(p - vec2(0, 1));
    let dx = select(l, r, dot(r, r) < dot(l, l));
    let dy = select(u, d, dot(d, d) < dot(u, u));
    let n = normalize(cross(dx, dy));
    return select(n, -n, dot(n, pos) > 0.0);
}
// Whether `guide_cover`'s last surface was water.
var<private> guide_water: bool;
// The water or glass from camera-relative `start` along `ray` within `reach`, as `hw_reflect`
// finds it: its face normal toward the ray and its distance from `start` (w below 0: none, or an
// opaque surface first).
fn guide_cover(start: vec3<f32>, ray: vec3<f32>, reach: f32) -> vec4<f32> {
    var q: ray_query;
    rayQueryInitialize(&q, terrain, RayDesc(0x0u, NO_CLOUDS, 0.0, reach, start + rt.hw_offset.xyz, ray));
    while rayQueryProceed(&q) {
        let h = rayQueryGetCandidateIntersection(&q);
        if h.geometry_index != instances[h.instance_custom_data].at.w { continue; }
        if hw_quad(h, true) == 0xffffffffu { continue; }
        rayQueryConfirmIntersection(&q);
    }
    let hit = rayQueryGetCommittedIntersection(&q);
    if hit.kind == RAY_QUERY_INTERSECTION_NONE
        || hit.geometry_index != instances[hit.instance_custom_data].at.w {
        return vec4(0.0, 0.0, 0.0, -1.0);
    }
    let base = hw_quad(hit, true);
    var n = -ray;
    let p0 = hw_position(base);
    let f = cross(hw_position(base + 1u) - p0, hw_position(base + 2u) - p0);
    if dot(f, f) > 1e-12 { n = normalize(f); }
    if dot(n, ray) > 0.0 { n = -n; }
    let layer = f32(arena_at(base, 4u) & 0x7fffu);
    guide_water = layer == rt.water.x || layer == rt.water.y;
    return vec4(n, hit.t);
}
fn guide_picture(diffuse: vec3<f32>, specular: vec3<f32>, n: vec3<f32>, roughness: f32,
    motion: vec2<f32>, source: vec3<f32>, dist: f32) -> vec4<f32> {
    switch GUIDES_SHOW {
        case 1u: { return vec4(pow(diffuse, vec3(1.0 / 2.2)), 1.0); }
        case 2u: { return vec4(n * 0.5 + 0.5, 1.0); }
        case 3u: { return vec4(vec3(roughness), 1.0); }
        // 16 pixels a frame either way fill the range; grey is still.
        case 4u: { return vec4(clamp(vec2(0.5) + motion / 32.0, vec2(0.0), vec2(1.0)), 0.5, 1.0); }
        case 5u: { return vec4(pow(specular, vec3(1.0 / 2.2)), 1.0); }
        // White near, grey at 32 blocks, black far and at the sky.
        case 7u: { return vec4(vec3(exp(-dist / 46.0)), 1.0); }
        default: { return vec4(source, 1.0); }
    }
}
@compute @workgroup_size(8, 8) fn cs_guides(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if f32(p.x) >= rt.size.x || f32(p.y) >= rt.size.y { return; }
    let centre = guide_centre(p);
    let d = textureLoad(guide_depth, p, 0);
    var diffuse = vec3(0.0);
    var specular = vec3(0.0);
    var roughness = 1.0;
    var motion = vec2(0.0);
    var n = vec3(0.0, 1.0, 0.0);
    var dist = 3.4e38;
    // Black: the sky; green: the ray's triangle; red: the depth's fallback; yellow: a wet
    // override; blue: water or glass over what lies behind.
    var source = vec3(0.0);
    if d <= 0.0 {
        // Reverse depth: 0 is the sky, which moves with the camera's turn alone. NVIDIA's guide's
        // defaults for it: diffuse albedo 0.5, the rest 0.
        let c = rt.inverse_view_proj * vec4(guide_ndc(centre), 1e-7, 1.0);
        let dir = normalize(c.xyz / c.w);
        diffuse = vec3(0.5);
        n = vec3(0.0);
        roughness = 0.0;
        let last = guide_last_pixel(vec4(dir, 0.0));
        if last.z > 0.5 { motion = last.xy - centre; }
    } else {
        let pos = guide_surface(p);
        dist = length(pos);
        let ray = pos / max(dist, 1e-6);
        // Across the depth's surface only: nothing between the camera and it (the player's own
        // body, the view model) is met, and the depth's precision falls with distance.
        let margin = 0.05 + 0.006 * dist;
        let start = max(dist - margin, 0.0);
        let h = hw_hit(ray * start, ray, dist + margin - start);
        let tolerance = 0.03 + 0.004 * dist;
        var f0 = GUIDE_F0;
        // Water or glass at the depth: its face normal, and the Fresnel term its mirror takes.
        var fresnel = -1.0;
        var cover_f0 = 0.0;
        if h.t >= 0.0 && abs(start + h.t - dist) <= tolerance {
            n = h.normal;
            diffuse = h.albedo;
            source = vec3(0.0, 1.0, 0.0);
        } else {
            let cover = guide_cover(ray * start, ray, dist + margin - start);
            if cover.w >= 0.0 && abs(start + cover.w - dist) <= tolerance {
                n = cover.xyz;
                cover_f0 = select(GUIDE_GLASS_F0, GUIDE_WATER_F0, guide_water);
                let nv = clamp(dot(n, -ray), 0.0, 1.0);
                fresnel = cover_f0 + (1.0 - cover_f0) * pow(1.0 - nv, 5.0);
                // What shows through it, along the unrefracted ray, tinted by what else lets
                // it through on the way.
                let behind = hw_hit(pos + ray * 0.02, ray, GUIDE_BEHIND);
                diffuse = select(vec3(0.214), behind.albedo * behind.through, behind.t >= 0.0);
                source = vec3(0.0, 0.4, 1.0);
            } else {
                n = guide_normal(p, pos);
                let cell = vec3<i32>(floor(pos - n * 0.01 + rt.camera.xyz + rt.fraction.xyz));
                let map = map_albedo(cell);
                diffuse = select(vec3(0.214), map, map.x >= 0.0);
                source = vec3(1.0, 0.0, 0.0);
            }
        }
        // The rain's puddles and wet films, which the kernel's reflections mirror
        // (shadow_rays_hw.wgsl `hw_puddle`, under the same conditions).
        let level = max(rt.rain.z, rt.rain_sun.y);
        let wet = max(rt.rain.y, rt.rain_sun.x);
        if fresnel < 0.0 && rt.mode.x > 0.5 && (SKIP & 8u) == 0u && (level > 0.0 || wet > 0.2) {
            let world = pos + rt.camera.xyz + rt.fraction.xyz;
            if rain_puddle(world, level, 0.8) > 0.0 {
                roughness = 0.05;
                f0 = GUIDE_WATER_F0;
                source = vec3(1.0, 1.0, 0.0);
            } else if wet > 0.2 && rain_film(world) {
                roughness = 0.2;
                f0 = GUIDE_WATER_F0;
                source = vec3(1.0, 1.0, 0.0);
            }
        }
        let nv = clamp(dot(n, -ray), 0.0, 1.0);
        specular = env_brdf_approx(vec3(f0), roughness, nv);
        if fresnel >= 0.0 {
            // NVIDIA's guide: where a transparent surface's reflection dominates, blend the
            // albedos and the roughness toward it (the depth and normal stay the surface's). What
            // lies behind comes through by 1 - F, the surface's mirror adds its own.
            diffuse *= 1.0 - fresnel;
            specular = specular * (1.0 - fresnel) + env_brdf_approx(vec3(cover_f0), 0.0, nv);
            roughness = mix(roughness, 0.0, fresnel);
        }
        let last = guide_last_pixel(vec4(pos + rt.motion.xyz, 1.0));
        if last.z > 0.5 { motion = last.xy - centre; }
        // An entity's own motion where it shows (already unjittered, previous - current).
        if guide_params.jitter.z > 0.5 {
            let own = textureLoad(guide_objects, p, 0);
            if own.z > 0.5 { motion = own.xy * rt.size.xy; }
        }
    }
    diffuse = clamp(finite3(diffuse), vec3(0.0), vec3(1.0));
    specular = clamp(finite3(specular), vec3(0.0), vec3(1.0));
    n = finite3(n);
    motion = vec2(finite(motion.x), finite(motion.y));
    textureStore(guide_diffuse, p, vec4(diffuse, 1.0));
    textureStore(guide_specular, p, vec4(specular, 1.0));
    textureStore(guide_normals, p, vec4(n, roughness));
    textureStore(guide_motion, p, vec4(motion, 0.0, 0.0));
    if GUIDES_SHOW != 0u {
        textureStore(guide_show, p, guide_picture(diffuse, specular, n, roughness, motion, source,
            dist));
    }
}
