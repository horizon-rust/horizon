// RT lighting's cloud mesh (gfx/rt_cloud_mesh.rs), shaded as a soft translucent surface: the
// sun's light wrapped around the form, light through the thin edges, the sky from above and
// below, and the air toward the horizon. Positions are from the camera: each tile's origin is a
// row of `tiles`, picked by the instance.

const PI: f32 = 3.14159265;
const TILES: u32 = 256u;
// How far (blocks) the cloud fades into the sky's haze, and where it is gone.
const HAZE: f32 = 14000.0;
const GONE_FROM: f32 = 5000.0;
const GONE_TO: f32 = 9500.0;

struct Draw {
    view_proj: mat4x4<f32>,
    // xyz toward the light (the sun's, or the moon's) and w the Earth's curvature (1 / its
    // diameter; 0 off), rgb its colour in paper whites, xyz toward the sun (for the sky table).
    light: vec4<f32>,
    light_color: vec4<f32>,
    sun: vec4<f32>,
    tiles: array<vec4<f32>, 256>,
}
@group(0) @binding(0) var<uniform> draw: Draw;
@group(0) @binding(1) var sky_view: texture_2d<f32>;
@group(0) @binding(2) var sky_sampler: sampler;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

@vertex fn vs_cloud(@builtin(instance_index) tile: u32, @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>) -> Out {
    var out: Out;
    var rel = draw.tiles[min(tile, TILES - 1u)].xyz + position;
    // The slab bends with the Earth from the eye, as the distant terrain (terrain.wgsl `earth_drop`).
    rel.y -= dot(rel.xz, rel.xz) * draw.light.w;
    out.clip = draw.view_proj * vec4(rel, 1.0);
    out.rel = rel;
    out.normal = normal;
    return out;
}

// The sky table toward `d` (unit), as terrain.wgsl `rt_sky`.
fn sky_at(d: vec3<f32>) -> vec3<f32> {
    let flat = d.xz / max(length(d.xz), 1e-5);
    let sun = draw.sun.xz / max(length(draw.sun.xz), 1e-5);
    let azimuth = acos(clamp(dot(flat, sun), -1.0, 1.0)) / PI;
    let e = asin(clamp(d.y, -1.0, 1.0)) / 1.5707963;
    let v = 0.5 + 0.5 * sign(e) * sqrt(abs(e));
    return textureSampleLevel(sky_view, sky_sampler, vec2(azimuth, v), 0.0).rgb;
}

fn hg(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * c, 1e-3), 1.5));
}

@fragment fn fs_cloud(in: Out) -> @location(0) vec4<f32> {
    let distance = length(in.rel);
    let d = in.rel / max(distance, 1e-3);
    let n = normalize(in.normal);
    let l = draw.light.xyz;
    let sun = draw.light_color.rgb;
    let facing = saturate(dot(n, -d));
    // The sun wrapped around the form (a soft terminator), and the light that went through the
    // cloud toward the eye: strong at the edges toward the sun (the silver lining).
    let ndl = dot(n, l);
    let wrapped = saturate((ndl + 0.45) / 1.45);
    let c = dot(d, l);
    let through = (1.0 - facing) * (1.0 - facing) * min(hg(c, 0.8) * PI, 6.0) * 0.5;
    // The sky: its zenith light on top, less below, the ground's a little warm light under.
    let up = saturate(n.y * 0.5 + 0.5);
    let sky = mix(sky_at(vec3(0.0, 1.0, 0.0)) * 0.6, sky_at(normalize(vec3(n.x, 0.6, n.z))), up);
    let ground = sun * max(l.y, 0.0) * vec3(0.12, 0.13, 0.08) * (1.0 - up);
    // Thin edges are lit through; the thick middle is the white of a diffuse surface.
    let albedo = 0.95;
    var color = albedo * (sun * (wrapped * 1.15 + through) + sky * 1.1) + ground;
    // The air between: toward the horizon the cloud takes the sky's colour and fades.
    let haze = 1.0 - exp(-distance / HAZE);
    color = mix(color, sky_at(d), haze);
    // Soft: the edges, where the surface turns from the eye, are most transparent.
    let alpha = pow(smoothstep(0.0, 0.55, facing), 0.8) * 0.96
        * (1.0 - smoothstep(GONE_FROM, GONE_TO, distance));
    return vec4(max(color, vec3(0.0)), alpha);
}
