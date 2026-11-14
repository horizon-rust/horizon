// Blocks, as Horizon's terrain shader draws them with HDR on: vanilla's colour, and on a light
// source the texels that shine lifted by their tier's gain, free of the shading and the light
// level that would darken them.
fn material(input: MaterialInput) -> Surface {
    let texel = gamma(input.albedo.rgb);
    // Vanilla's shading, and coloured lights added to it under the vertex colour.
    let shade = min(gamma(input.shading) + gamma(input.tint.rgb) * gamma(input.colored_light), vec3<f32>(1.0));
    var color = texel * shade;
    var mark = 1.0;
    let tier = tier_of(input.emission);
    let y = brightness(texel);
    if frame.hdr == 1u && tier > 0u && !input.translucent && y >= input.emission_threshold {
        mark = hdr_mark(tier, y);
        let full = gamma(frame.full_light);
        color *= hdr_unshade(shade, hdr_lift(tier, y), max(full.r, max(full.g, full.b)));
    }
    return Surface(vec4<f32>(lit(color, mark), input.albedo.a * input.tint.a));
}
