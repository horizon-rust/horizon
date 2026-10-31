// RT lighting's primary surface replacement through water (Remix's PSR, gfx/shadow_rays.rs):
// appended with the hybrid's hardware rays. Where `hw_reflect` met the water, the ray from the
// camera bends into it by Snell's law (water's index 1.33; from under it, out into the air, and
// back by total internal reflection past the critical angle), meets what lies beyond and lights
// it as the reflection does; the water's shader shows it through its absorption over the distance
// the ray ran in it, so what lies under the waves is seen bent by them, and from below, the sky
// through Snell's window.
const WATER_INDEX: f32 = 1.33;
fn hw_refract(p: vec2<i32>) -> vec4<f32> {
    if water_at.w < 0.0 && glass_at.w < 0.0 { return vec4(0.0); }
    let size = rt.size.xy;
    let ndc = vec2((f32(p.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (f32(p.y) + 0.5) / size.y * 2.0);
    let c = rt.inverse_view_proj * vec4(ndc, 1e-7, 1.0);
    let ray = normalize(c.xyz / c.w);
    if glass_at.w >= 0.0 { return hw_refract_glass(ray); }
    let n = water_at.xyz;
    let at = ray * water_at.w;
    // From above (the normal up), into the water; from below, out of it.
    let eta = select(WATER_INDEX, 1.0 / WATER_INDEX, n.y > 0.0);
    var bent = refract(ray, n, eta);
    let inside = n.y > 0.0;
    if dot(bent, bent) < 1e-6 {
        // Total internal reflection: back down into the water.
        bent = reflect(ray, n);
    }
    let start = at - n * 0.02;
    let h = hw_hit(start, bent, 256.0);
    if h.t < 0.0 {
        // Through the surface to the sky (from below), or into deep water.
        if inside { return vec4(0.0, 0.0, 0.0, 64.0); }
        return vec4(sky_at(bent) * h.through, 0.01);
    }
    let seen = start + bent * h.t;
    let lifted = seen + h.normal * 0.02;
    var light = sun_on(lifted, h.normal);
    let last = cached_on(seen, h.normal);
    light += select(open_light(lifted, h.normal, seen), last.rgb, last.a >= 0.0);
    // From above, the distance in the water; from below, the ray is already out of it.
    return vec4(h.through * h.albedo * light, select(0.01, max(h.t, 0.01), inside));
}
// The same through glass (shadow_rays_hw.wgsl `hw_glass`): where `hw_reflect` met it, its face
// toward the camera (xyz) and distance (w, below 0 none), and its attenuation a block.
var<private> glass_at: vec4<f32>;
var<private> glass_sigma: vec3<f32>;
// A face the ray from camera-relative `pos` along `l` meets first within `reach`: its distance (-1
// none), its quad's normal (as wound: outward) and whether it is a solid one (the ray then meets
// the terrain there, not the glass's far side).
struct HwFace {
    t: f32,
    normal: vec3<f32>,
    solid: bool,
}
fn hw_face(pos: vec3<f32>, l: vec3<f32>, reach: f32) -> HwFace {
    var q: ray_query;
    rayQueryInitialize(&q, terrain, RayDesc(0x0u, NO_CLOUDS, 0.0, reach, pos + rt.hw_offset.xyz, l));
    while rayQueryProceed(&q) {
        let c = rayQueryGetCandidateIntersection(&q);
        if c.geometry_index == instances[c.instance_custom_data].w {
            if hw_quad(c, true) != NO_QUAD { rayQueryConfirmIntersection(&q); }
        } else if hw_opaque(c) {
            rayQueryConfirmIntersection(&q);
        }
    }
    let hit = rayQueryGetCommittedIntersection(&q);
    if hit.kind == RAY_QUERY_INTERSECTION_NONE { return HwFace(-1.0, vec3(0.0), false); }
    if hit.geometry_index != instances[hit.instance_custom_data].w {
        return HwFace(hit.t, -l, true);
    }
    let base = hw_quad(hit, true);
    let p0 = hw_position(base);
    let f = cross(hw_position(base + 1u) - p0, hw_position(base + 2u) - p0);
    return HwFace(hit.t, select(-l, normalize(f), dot(f, f) > 1e-12), false);
}
// Remix's primary surface replacement through glass: the ray from the camera bends into it by
// Snell's law (index 1.52) at the face `hw_reflect` met, runs inside to the face it leaves by,
// bounces off that face back in by total internal reflection past the critical angle (up to three
// times: the light caught along a pane's length, a window's edges seen at a slant), and bends out
// through it; then meets what lies beyond, lit as the reflection is. That light comes back through
// the walls it crossed (shadow_rays_hw.wgsl `glass_wall`: each one's colour over its thickness
// along the bent ray, deeper where the ray slants; the far face's Fresnel, the near face's being
// the glass's shader's): a window shifts what is behind it by its thickness. Where the glass lies
// against a solid block the ray meets that block inside it. Added to it, what the glass itself
// scatters toward the eye of the sun on the face the sun lights (`GLASS_SCATTER` of what its
// colour takes out, through that colour): stained glass glows in its colour with the sun on it or
// behind it, clear glass hardly at all, black glass not at all. rgb that
// light, a the length run inside; 0 where the ray stays caught in the glass or meets no face
// within 32 blocks (the glass's shader then blends what was drawn behind it).
// How much of the light its pigment takes out of the light crossing glass that it scatters
// instead: Horizon's own, where Remix's translucent material has none.
const GLASS_SCATTER: f32 = 0.25;
fn hw_refract_glass(ray: vec3<f32>) -> vec4<f32> {
    let n = glass_at.xyz;
    let g = vec4(glass_sigma, 0.0);
    let entry = ray * glass_at.w;
    var pos = entry - n * 0.002;
    var dir = refract(ray, n, 1.0 / GLASS_INDEX);
    // The near wall (its Fresnel is the shader's): its colour over its thickness along the bent ray.
    var absorbed = exp(-g.rgb * GLASS_WALL / max(-dot(dir, n), 0.05));
    var inside = 0.0;
    var out = false;
    for (var i = 0u; i < 4u; i++) {
        let f = hw_face(pos, dir, 32.0);
        if f.t < 0.0 { break; }
        inside += f.t;
        if f.solid {
            pos += dir * max(f.t - 0.01, 0.0);
            out = true;
            break;
        }
        pos += dir * f.t;
        // The face's normal against the ray (the far side's points along it).
        let m = select(f.normal, -f.normal, dot(f.normal, dir) > 0.0);
        let bent = refract(dir, m, GLASS_INDEX);
        if dot(bent, bent) < 1e-6 {
            // Reflected back in off the wall, through its colour there and back.
            let w = exp(-g.rgb * GLASS_WALL / max(-dot(dir, m), 0.05));
            absorbed *= w * w;
            dir = reflect(dir, m);
            pos += m * 0.002;
            continue;
        }
        absorbed *= glass_wall(g, -dot(bent, m));
        dir = bent;
        pos -= m * 0.004;
        out = true;
        break;
    }
    if !out { return vec4(0.0); }
    // The sun on the near face, or on the far one (the glass lying against no block there); clear
    // glass, which scatters next to nothing, spares the ray.
    let taken = 1.0 - dot(absorbed, vec3(0.2126, 0.7152, 0.0722));
    var lit = vec3(0.0);
    if taken >= 0.1 && rt.light.w > 0.0 {
        if dot(n, rt.light.xyz) > 0.0 {
            lit = sun_on(entry + n * 0.02, n);
        } else if dot(pos - entry, n) < 0.0 {
            lit = sun_on(pos - n * 0.02, -n);
        }
    }
    let glow = lit * absorbed * GLASS_SCATTER * taken;
    let h = hw_hit(pos, dir, 256.0);
    if h.t < 0.0 { return vec4(sky_at(dir) * h.through * absorbed + glow, max(inside, 0.01)); }
    let seen = pos + dir * h.t;
    let lifted = seen + h.normal * 0.02;
    var light = sun_on(lifted, h.normal);
    let last = cached_on(seen, h.normal);
    light += select(open_light(lifted, h.normal, seen), last.rgb, last.a >= 0.0);
    return vec4(absorbed * h.through * h.albedo * light + glow, max(inside, 0.01));
}
