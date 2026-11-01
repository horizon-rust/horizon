// Impostors (impostor.rs): far entities as one camera-facing quad each, textured from
// the page array of baked tiles, lit as the sky lights an entity in the open and
// fogged as entity.wgsl fogs it. The quads are camera-relative and write depth, so
// terrain in front of them (the far tiles too) hides them.
struct Globals {
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    // x, y: spherical fog start and end; z, w: cylindrical.
    fog: vec4<f32>,
}
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(3) var lightmap: texture_2d<f32>;
@group(0) @binding(4) var lightmap_sampler: sampler;
@group(1) @binding(0) var tiles: texture_2d_array<f32>;
@group(1) @binding(1) var tile_sampler: sampler;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) page: u32,
    @location(2) spherical: f32,
    @location(3) cylindrical: f32,
}
@vertex fn vs_impostor(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) page: u32) -> Out {
    var out: Out;
    out.clip = globals.view_proj * vec4(position, 1.0);
    out.uv = uv;
    out.page = page;
    out.spherical = length(position);
    out.cylindrical = max(length(position.xz), abs(position.y));
    return out;
}
fn linear_fog(d: f32, start: f32, end: f32) -> f32 {
    if d <= start { return 0.0; }
    if d >= end { return 1.0; }
    return (d - start) / (end - start);
}
@fragment fn fs_impostor(in: Out) -> @location(0) vec4<f32> {
    let texel = textureSample(tiles, tile_sampler, in.uv, i32(in.page));
    // The tiles' alpha is cut when baked; the mips average it, so a thin limb keeps
    // showing at a distance while the filtered edge of a whole one stays crisp.
    if texel.a < 0.35 { discard; }
    // Full sky light, no block light: the lightmap's top row, so the time of day and
    // the dimension tint it as they tint the entities beside it.
    let light = textureSampleLevel(lightmap, lightmap_sampler, vec2(0.5 / 16.0, 15.5 / 16.0), 0.0);
    let color = texel.rgb * light.rgb;
    let f = max(linear_fog(in.spherical, globals.fog.x, globals.fog.y),
                linear_fog(in.cylindrical, globals.fog.z, globals.fog.w));
    return vec4(mix(color, globals.fog_color.rgb, f * globals.fog_color.a), 1.0);
}
