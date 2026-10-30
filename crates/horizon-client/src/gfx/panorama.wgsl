// CubeMap's position-only cube and panorama.vsh/fsh: sample its untransformed direction.
struct Projection { view_proj: mat4x4<f32> }
@group(0) @binding(0) var<uniform> projection: Projection;
@group(0) @binding(1) var panorama: texture_cube<f32>;
@group(0) @binding(2) var panorama_sampler: sampler;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) direction: vec3<f32>,
}
@vertex fn vs_panorama(@builtin(vertex_index) index: u32) -> Out {
    let corners = array<vec3<f32>, 8>(
        vec3(-1.0, -1.0, -1.0), vec3(1.0, -1.0, -1.0),
        vec3(1.0, 1.0, -1.0), vec3(-1.0, 1.0, -1.0),
        vec3(-1.0, -1.0, 1.0), vec3(1.0, -1.0, 1.0),
        vec3(1.0, 1.0, 1.0), vec3(-1.0, 1.0, 1.0),
    );
    let indices = array<u32, 36>(
        4, 7, 6, 4, 6, 5, 5, 6, 2, 5, 2, 1,
        1, 2, 3, 1, 3, 0, 0, 3, 7, 0, 7, 4,
        7, 3, 2, 7, 2, 6, 0, 4, 5, 0, 5, 1,
    );
    let position = corners[indices[index]];
    var out: Out;
    out.clip = projection.view_proj * vec4(position, 1.0);
    out.direction = position;
    return out;
}
// Linear Light (gfx/tonemap.rs): the picture decoded to linear light for the float scene.
override LINEAR: bool = false;
fn scene(c: vec4<f32>) -> vec4<f32> {
    if LINEAR { return vec4(pow(max(c.rgb, vec3(0.0)), vec3(2.2)), c.a); }
    return c;
}
@fragment fn fs_panorama(in: Out) -> @location(0) vec4<f32> {
    return scene(textureSample(panorama, panorama_sampler, in.direction));
}

// VR's title backdrop (`Panorama::prepare_eye`): one eye's rays from where it is, met with a
// floor under the head and a sphere around it, sample the cube towards the point they meet
// from the sphere's centre. Both eyes see a point of the scene in the same texel from two
// places, so it has depth and stays where it is as the head turns.
struct Backdrop {
    // Clip space to the backdrop's space (stage metres from the sphere's centre).
    inverse_view_proj: mat4x4<f32>,
    // The eye there, and the floor's height below the centre.
    eye: vec3<f32>,
    floor: f32,
    // x: the sphere's radius.
    params: vec4<f32>,
}
@group(0) @binding(3) var<uniform> backdrop: Backdrop;
struct BackdropOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}
@vertex fn vs_backdrop(@builtin(vertex_index) index: u32) -> BackdropOut {
    let ndc = vec2(f32(index & 1u) * 4.0 - 1.0, f32(index >> 1u) * 4.0 - 1.0);
    var out: BackdropOut;
    out.clip = vec4(ndc, 0.0, 1.0);
    out.ndc = ndc;
    return out;
}
@fragment fn fs_backdrop(in: BackdropOut) -> @location(0) vec4<f32> {
    let on = backdrop.inverse_view_proj * vec4(in.ndc, 0.5, 1.0);
    let ray = normalize(on.xyz / on.w - backdrop.eye);
    let e = backdrop.eye;
    // From inside the sphere one root is ahead.
    let b = dot(e, ray);
    let c = dot(e, e) - backdrop.params.x * backdrop.params.x;
    var t = -b + sqrt(max(b * b - c, 0.0));
    if ray.y < 0.0 {
        let to_floor = (-backdrop.floor - e.y) / ray.y;
        if to_floor > 0.0 {
            t = min(t, to_floor);
        }
    }
    let at = e + ray * t;
    // The cube's space is vanilla's view turned by PI about x (`projection`): y down, ahead +z.
    return scene(textureSample(panorama, panorama_sampler, vec3(at.x, -at.y, -at.z)));
}
