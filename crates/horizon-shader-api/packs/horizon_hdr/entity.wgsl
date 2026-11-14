// Entities and block entities, as Horizon's entity shader draws them with HDR on: vanilla's
// colour (the reference's own `entity`, with coloured lights added to the light), and what
// shines lifted by its tier's gain. They keep their shading: it is their own, not a face's.
fn entity(input: EntityInput) -> Surface {
    let base = gamma(input.albedo.rgb * input.tint.rgb * input.shade);
    let flashed = mix(base, gamma(input.overlay.rgb), input.overlay.a);
    let light = min(gamma(input.lightmap) + gamma(input.colored_light), vec3<f32>(1.0));
    var mark = 1.0;
    let tier = tier_of(input.light_source);
    if frame.hdr == 1u && tier > 0u && input.albedo.a >= 0.1 {
        // A layer that fades (the warden's) shines by what is left of it.
        mark = hdr_mark(tier, brightness(base) * input.tint.a);
    }
    return Surface(vec4<f32>(lit(flashed * light, mark), input.albedo.a * input.tint.a));
}
