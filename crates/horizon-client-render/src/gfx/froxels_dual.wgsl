// The froxels' composite in one pass with dual-source blending (froxels.rs, where the GPU has it):
// the medium's light added and what it lets through of the scene, from one march of the fogs instead
// of one for each (`fs_transmit`, `fs_scatter`). Appended to froxels.wgsl after
// `enable dual_source_blending;`.
struct ApplyOut {
    @location(0) @blend_src(0) added: vec4<f32>,
    @location(0) @blend_src(1) through: vec4<f32>,
}
@fragment
fn fs_apply_dual(in: Screen) -> ApplyOut {
    let m = medium(in);
    return ApplyOut(vec4(m.light, 0.0), vec4(medium_through(m.r, m.through), 1.0));
}
