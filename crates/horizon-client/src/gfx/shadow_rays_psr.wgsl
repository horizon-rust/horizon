// RT lighting's primary surface replacement through water (Remix's PSR, gfx/shadow_rays.rs):
// appended with the hybrid's hardware rays. Where `hw_reflect` met the water, the ray from the
// camera bends into it by Snell's law (water's index 1.33; from under it, out into the air, and
// back by total internal reflection past the critical angle), meets what lies beyond and lights
// it as the reflection does; the water's shader shows it through its absorption over the distance
// the ray ran in it, so what lies under the waves is seen bent by them, and from below, the sky
// through Snell's window.
const WATER_INDEX: f32 = 1.33;
fn hw_refract(p: vec2<i32>) -> vec4<f32> {
    if water_at.w < 0.0 { return vec4(0.0); }
    let size = rt.size.xy;
    let ndc = vec2((f32(p.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (f32(p.y) + 0.5) / size.y * 2.0);
    let c = rt.inverse_view_proj * vec4(ndc, 1e-7, 1.0);
    let ray = normalize(c.xyz / c.w);
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
