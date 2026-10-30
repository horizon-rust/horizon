
// RT lighting's translucent terrain with dual-source blending (gfx.rs, where the GPU has it): the
// light a fragment adds, and what of the frame behind shows through it in colour, so water tints
// what lies under it (red absorbed first) instead of only dimming it. Appended to terrain.wgsl
// after `enable dual_source_blending;`.
struct DualOut {
    @location(0) @blend_src(0) added: vec4<f32>,
    @location(0) @blend_src(1) through: vec4<f32>,
}
@fragment fn fs_translucent_dual(in: TerrainOut) -> DualOut {
    let n = face_normal(in.pos);
    let texel = sprite(in.uv, in.layer & 0x7fffu);
    let color = texel * in.color;
    var out: DualOut;
    if color.a < 0.01 || faded_out(in.fade, in.clip.xy) { discard; }
    if globals.rt_mode.x > 0.5 && linear_on() && in.emission == 0u && rt_is_water(in.layer) {
        let w = rt_water(in, n);
        out.added = vec4(w.added, w.alpha);
        out.through = vec4(w.through, 1.0 - w.alpha);
        return out;
    }
    var c = scene_color(color, emission_gain(in.emission), in.spherical, in.cylindrical);
    if globals.rt_mode.x > 0.5 && linear_on() && in.emission == 0u {
        c = vec4(rt_lit(texel.rgb * in.tint, in, n), color.a);
    }
    out.added = vec4(c.rgb * c.a, c.a);
    out.through = vec4(1.0 - c.a);
    return out;
}
