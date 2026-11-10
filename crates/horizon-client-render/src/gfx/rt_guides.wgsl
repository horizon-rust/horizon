
// DLSS Ray Reconstruction's guides (gfx/rt_guides.rs, docs/rt-dlss-rr.md): appended to the hardware
// kernel (shadow_rays.rs `source(true, ..)`, then this) only when `rt_guides::guides_wanted()`, as
// a module of its own, so the kernel the game runs otherwise is the very text it was. It reads the
// kernel's group 0 as the kernel binds it for the view (the same bind group: `rt`, the depth
// prepass, the structures, the arena, the sprites, the grid, the rain map) and writes its own
// group 1, a texel a depth pixel (the render size: the screen's prepass, VR's at the rays' size).
//
// What each pixel gets, at the surface the depth prepass shows (the solid terrain):
// - a short ray across that surface (`hw_hit` from just before it to just past it: its exact
//   triangle, alpha-tested, and its texel), whose albedo (linear, the texel times the vertex
//   tint) and geometric face normal are taken; where it misses or meets something at another
//   distance than the depth's (geometry outside the structures, far terrain), the depth's own
//   normal (`surface_normal`) and the voxel grid's map colour of the block (else mid grey);
// - roughness 1 and F0 0.04 (the kernel reads no LabPBR specular map), except where the kernel's
//   reflections mirror the rain's puddles (roughness 0.05) and wet films (0.2), F0 0.02 (water);
// - the specular albedo, the preintegrated environment BRDF (`env_brdf_approx`) of that F0 and
//   roughness at the pixel's N.V, not F0;
// - the camera's motion: where the surface was on last frame's screen less where it is (pixels,
//   x right, y down), from the depth and last frame's matrix (`rt.prev_view_proj`, `rt.motion`)
//   as the kernel's own reprojection takes it; the sky by its direction alone.
// Entities, the hand and translucent surfaces are not in the depth prepass: they get the terrain
// behind them, and entities no motion of their own (docs/rt-dlss-rr.md, "Guides").
@group(1) @binding(0) var guide_diffuse: texture_storage_2d<rgba8unorm, write>;
@group(1) @binding(1) var guide_specular: texture_storage_2d<rgba8unorm, write>;
// World-space unit normal (xyz, toward the camera), roughness in w (`DLSS.Roughness.Mode` packed).
@group(1) @binding(2) var guide_normals: texture_storage_2d<rgba16float, write>;
// Render pixels from this frame's (unjittered) position to last frame's: previous - current.
@group(1) @binding(3) var guide_motion: texture_storage_2d<rg16float, write>;
// `HORIZON_RT_GUIDES_SHOW`'s picture of one guide, for the screen (1x1 and unwritten otherwise).
@group(1) @binding(4) var guide_show: texture_storage_2d<rgba8unorm, write>;
// Which guide `guide_show` pictures (rt_guides.rs `Show`): 0 none, 1 the diffuse albedo, 2 the
// normals, 3 the roughness, 4 the motion, 5 the specular albedo, 6 where each guide came from.
override GUIDES_SHOW: u32 = 0u;
// The F0 of a dielectric (Remix's and Unreal's default) and of water.
const GUIDE_F0: f32 = 0.04;
const GUIDE_WATER_F0: f32 = 0.02;
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
fn guide_picture(diffuse: vec3<f32>, specular: vec3<f32>, n: vec3<f32>, roughness: f32,
    motion: vec2<f32>, source: vec3<f32>) -> vec4<f32> {
    switch GUIDES_SHOW {
        case 1u: { return vec4(pow(diffuse, vec3(1.0 / 2.2)), 1.0); }
        case 2u: { return vec4(n * 0.5 + 0.5, 1.0); }
        case 3u: { return vec4(vec3(roughness), 1.0); }
        // 16 pixels a frame either way fill the range; grey is still.
        case 4u: { return vec4(clamp(vec2(0.5) + motion / 32.0, vec2(0.0), vec2(1.0)), 0.5, 1.0); }
        case 5u: { return vec4(pow(specular, vec3(1.0 / 2.2)), 1.0); }
        default: { return vec4(source, 1.0); }
    }
}
@compute @workgroup_size(8, 8) fn cs_guides(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if f32(p.x) >= rt.size.x || f32(p.y) >= rt.size.y { return; }
    let centre = vec2<f32>(p) + 0.5;
    let d = textureLoad(depth, p, 0);
    var diffuse = vec3(0.0);
    var specular = vec3(0.0);
    var roughness = 1.0;
    var motion = vec2(0.0);
    var n = vec3(0.0, 1.0, 0.0);
    // Black: the sky; green: the ray's triangle; red: the depth's fallback; yellow: a wet override.
    var source = vec3(0.0);
    if d <= 0.0 {
        // Reverse depth: 0 is the sky, which moves with the camera's turn alone.
        let size = rt.size.xy;
        let ndc = vec2(centre.x / size.x * 2.0 - 1.0, 1.0 - centre.y / size.y * 2.0);
        let c = rt.inverse_view_proj * vec4(ndc, 1e-7, 1.0);
        let dir = normalize(c.xyz / c.w);
        n = -dir;
        let last = guide_last_pixel(vec4(dir, 0.0));
        if last.z > 0.5 { motion = last.xy - centre; }
    } else {
        let pos = surface_at(p);
        let dist = length(pos);
        let ray = pos / max(dist, 1e-6);
        // Across the depth's surface only: nothing between the camera and it (the player's own
        // body, the view model) is met, and the depth's precision falls with distance.
        let margin = 0.05 + 0.006 * dist;
        let start = max(dist - margin, 0.0);
        let h = hw_hit(ray * start, ray, dist + margin - start);
        let tolerance = 0.03 + 0.004 * dist;
        if h.t >= 0.0 && abs(start + h.t - dist) <= tolerance {
            n = h.normal;
            diffuse = h.albedo;
            source = vec3(0.0, 1.0, 0.0);
        } else {
            n = surface_normal(p, pos);
            let cell = vec3<i32>(floor(pos - n * 0.01 + rt.camera.xyz + rt.fraction.xyz));
            let map = map_albedo(cell);
            diffuse = select(vec3(0.214), map, map.x >= 0.0);
            source = vec3(1.0, 0.0, 0.0);
        }
        var f0 = GUIDE_F0;
        // The rain's puddles and wet films, which the kernel's reflections mirror
        // (shadow_rays_hw.wgsl `hw_puddle`, under the same conditions).
        let level = max(rt.rain.z, rt.rain_sun.y);
        let wet = max(rt.rain.y, rt.rain_sun.x);
        if rt.mode.x > 0.5 && (SKIP & 8u) == 0u && (level > 0.0 || wet > 0.2) {
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
        specular = env_brdf_approx(vec3(f0), roughness, clamp(dot(n, -ray), 0.0, 1.0));
        let last = guide_last_pixel(vec4(pos + rt.motion.xyz, 1.0));
        if last.z > 0.5 { motion = last.xy - centre; }
    }
    diffuse = clamp(finite3(diffuse), vec3(0.0), vec3(1.0));
    n = finite3(n);
    motion = vec2(finite(motion.x), finite(motion.y));
    textureStore(guide_diffuse, p, vec4(diffuse, 1.0));
    textureStore(guide_specular, p, vec4(specular, 1.0));
    textureStore(guide_normals, p, vec4(n, roughness));
    textureStore(guide_motion, p, vec4(motion, 0.0, 0.0));
    if GUIDES_SHOW != 0u {
        textureStore(guide_show, p, guide_picture(diffuse, specular, n, roughness, motion, source));
    }
}
