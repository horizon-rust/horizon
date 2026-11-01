// The visibility test of the far tile traversal (gfx/lod_gpu.rs): called for
// every node whose box is in the frustum. `lo` and `hi` are the node's box
// relative to the camera, so a test against the depth of the last frame
// (stage 5's HiZ) projects them with `view.view_proj`. Returns true to leave
// the node, and everything under it, out. Resources it reads are bound in
// group 1 (`Visibility::layout`). This one sees everything.
fn node_hidden(lo: vec3<f32>, hi: vec3<f32>) -> bool {
    return false;
}
