// VR's depth prepass for both eyes in one pass (multiview, gfx.rs `EyeDepths`): the terrain's
// vertex with each view's matrix, the first eye's in layer 0 and the second's in layer 1.
@group(1) @binding(2) var<uniform> eye_view_projs: array<mat4x4<f32>, 2>;
@vertex fn vs_terrain_views(
    @builtin(view_index) view: u32,
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>,
    @location(5) origin: vec4<f32>) -> TerrainOut {
    var out = terrain_vertex(position, terrain_uv(uv, origin.w), layer, color, light, origin);
    out.clip = eye_view_projs[view] * vec4(out.pos, 1.0);
    out.fade = terrain_fade(origin.w);
    return out;
}
