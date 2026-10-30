//! Relief from LabPBR resource packs (the Parallax setting, off by default): terrain drawn with
//! the block sprites' normal maps (`gfx/materials.rs`). Each fragment marches the ray from the
//! camera into the map's height (alpha: 255 the surface, 0 a quarter block deep) texel by
//! texel, so 16x packs keep their pixels: it stops on a texel's top, lit by the map's normal,
//! or on the wall between two texels. The colour is then the sprite's at that point, shaded
//! with vanilla's directional face shade for the new normal, times the map's ambient occlusion
//! (blue) and a little darker the deeper it is (`PARALLAX_CAVITY`, which a ray-traced ambient
//! occlusion would replace). Translucent terrain gets the normal and the occlusion, without
//! the march.
//!
//! Against aliasing, the march runs only where a texel of the map covers about a pixel or more
//! (from the texture coordinate's derivatives): the relief fades out as texels shrink below a
//! pixel, with distance and at grazing angles, and the normal and the occlusion are then read
//! from the map's mip level for that size, averaged as data. A picture drawn at a lower
//! resolution (the resolution scale, a foveated view's periphery) reaches that size sooner, so
//! it marches less.
//!
//! With RT lighting (terrain.wgsl `rt_shade`) the relief's surface is lit as a LabPBR material
//! instead: the albedo, the map's normal and occlusion, and from the specular maps (`_s`) the
//! roughness, the reflectance or metal, the subsurface scattering and the emission, under the
//! rays' sun, sky and light sources.
//!
//! The light through thin materials is this file's own (`parallax_transmission`): the sun's, on a
//! face turned away from it, as much as the specular map's blue says (65 and up) or, where it says
//! nothing, as much as `SSS_THIN` for leaves, plants and the like (`gfx/transmission.rs`), none
//! for thick blocks. Its colour is the pack's `_sss` map (RTX Remix's backlight tint) where the
//! sprite has one, the albedo's twice (filtered by the leaf on the way in and out) where not.
//!
//! The pipelines are vanilla's terrain (terrain.wgsl's section, prefixed as for the material
//! stage, `native_material.rs`) with entries of their own; they read the terrain's group 0,
//! the flat draws' group 1 where those run, and the normal, specular and subsurface maps and
//! the thin layers at group [`GROUP`].

use super::materials::MapArrays;
use super::native_material::{NativeMaterial, TerrainShape};
use super::wgsl_prefix::{TERRAIN, prefixed, section};

/// The bind group of the normal, specular and subsurface maps and the thin layers.
pub const GROUP: u32 = 2;
/// Block sampled textures terrain.wgsl's group 0 binds (8 sprite arrays, the lightmap, RT
/// lighting's two masks, its sky and its sun's tint).
const TERRAIN_TEXTURES: u32 = 14;

const PREFIX: &str = "horizon_terrain_";

/// The entries, after the prefixed terrain and the normal map declarations.
const ENTRIES: &str = r"
// How deep the relief goes (1: LabPBR's quarter block) and where it fades out, in blocks.
override PARALLAX_DEPTH: f32 = 1.0;
override PARALLAX_DISTANCE: f32 = 24.0;
// How much darker the bottom of the relief is (0 leaves occlusion to the map and the light).
override PARALLAX_CAVITY: f32 = 0.25;
// At most this many texels crossed per fragment.
const PARALLAX_STEPS: u32 = 48u;
const PARALLAX_NEUTRAL: vec4<f32> = vec4(0.5, 0.5, 1.0, 1.0);
// No specular map: rough, a dielectric's 4 % (10 / 255 in LabPBR), no emission.
const SPECULAR_NEUTRAL: vec4<f32> = vec4(0.0, 0.0392, 0.0, 1.0);
// No subsurface map: no tint (alpha 0, a map's own texels read 1).
const SUBSURFACE_NEUTRAL: vec4<f32> = vec4(0.0);
// The subsurface scattering a thin block (leaves, plants) has where the specular map says none.
override SSS_THIN: f32 = 0.7;
// How much of the sun on a thin material's far side comes out of the near one, as in
// terrain.wgsl `rt_shade`.
const SSS_ENERGY: f32 = 0.35;

struct ParallaxOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) color: vec4<f32>,
    @location(3) spherical: f32,
    @location(4) cylindrical: f32,
    @location(5) @interpolate(flat) emission: u32,
    @location(6) position: vec3<f32>,
    // The sky and block light levels, for ray-traced light (terrain.wgsl `sun_share`).
    @location(7) light: vec2<f32>,
    // For RT lighting: the face's tint.
    @location(8) tint: vec3<f32>,
}
fn parallax_vertex(position: vec3<f32>, uv: vec2<f32>, layer: u32, color: vec4<f32>,
    light: vec2<u32>, origin: vec3<f32>) -> ParallaxOut {
    let t = horizon_terrain_terrain_vertex(position, uv, layer, color, light, origin);
    return ParallaxOut(t.clip, t.uv, t.layer, t.color, t.spherical, t.cylindrical, t.emission,
        position + origin, t.light, t.tint);
}
@vertex fn parallax_vs(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>,
    @location(5) origin: vec4<f32>) -> ParallaxOut {
    return parallax_vertex(position, horizon_terrain_terrain_uv(uv, origin.w), layer, color,
        light, origin.xyz);
}
@vertex fn parallax_vs_flat(
    @builtin(vertex_index) vertex: u32,
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>
) -> ParallaxOut {
    let origin = horizon_terrain_slot_origins[horizon_terrain_block_slots[vertex >> 8u]];
    return parallax_vertex(position, horizon_terrain_terrain_uv(uv, origin.w), layer, color,
        light, origin.xyz);
}

// The sprite at `uv` with the surface's own derivatives (the march moves `uv` per fragment).
fn parallax_sprite(uv: vec2<f32>, layer: u32, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    let l = i32(layer % horizon_terrain_SPRITE_SPLIT);
    let s = horizon_terrain_sprite_sampler;
    switch layer / horizon_terrain_SPRITE_SPLIT {
        case 0u: { return textureSampleGrad(horizon_terrain_sprites, s, uv, l, dx, dy); }
        case 1u: { return textureSampleGrad(horizon_terrain_sprites_1, s, uv, l, dx, dy); }
        case 2u: { return textureSampleGrad(horizon_terrain_sprites_2, s, uv, l, dx, dy); }
        case 3u: { return textureSampleGrad(horizon_terrain_sprites_3, s, uv, l, dx, dy); }
        case 4u: { return textureSampleGrad(horizon_terrain_sprites_4, s, uv, l, dx, dy); }
        case 5u: { return textureSampleGrad(horizon_terrain_sprites_5, s, uv, l, dx, dy); }
        case 6u: { return textureSampleGrad(horizon_terrain_sprites_6, s, uv, l, dx, dy); }
        default: { return textureSampleGrad(horizon_terrain_sprites_7, s, uv, l, dx, dy); }
    }
}

// Where a sprite layer's normal map is: its array (0xffff: none), its layer there and its
// edge in texels.
struct ParallaxSlot {
    array: u32,
    layer: i32,
    size: f32,
}
fn parallax_slot(layer: u32) -> ParallaxSlot {
    let entry = parallax_index[layer];
    if entry == 0xffffffffu {
        return ParallaxSlot(0xffffu, 0, 1.0);
    }
    let array = entry >> 16u;
    return ParallaxSlot(array, i32(entry & 0xffffu), f32(parallax_size(array)));
}
// The same for the sprite's specular map.
fn specular_slot(layer: u32) -> ParallaxSlot {
    let entry = specular_index[layer];
    if entry == 0xffffffffu {
        return ParallaxSlot(0xffffu, 0, 1.0);
    }
    let array = entry >> 16u;
    return ParallaxSlot(array, i32(entry & 0xffffu), f32(specular_size(array)));
}
// The same for the sprite's subsurface map.
fn subsurface_slot(layer: u32) -> ParallaxSlot {
    let entry = subsurface_index[layer];
    if entry == 0xffffffffu {
        return ParallaxSlot(0xffffu, 0, 1.0);
    }
    let array = entry >> 16u;
    return ParallaxSlot(array, i32(entry & 0xffffu), f32(subsurface_size(array)));
}
// The backlight tint at `uv` (the texel's colour, alpha 1; alpha 0 where the sprite has no map),
// read as `specular_at` reads.
fn subsurface_at(layer: u32, uv: vec2<f32>, footprint: f32) -> vec4<f32> {
    let slot = subsurface_slot(layer);
    if slot.array == 0xffffu { return SUBSURFACE_NEUTRAL; }
    let lod = log2(max(footprint * slot.size, 1e-6));
    let level = clamp(i32(round(lod)), 0, i32(log2(slot.size)));
    let size = max(slot.size / exp2(f32(level)), 1.0);
    let cell = floor(uv * size);
    let texel = subsurface_load(slot, vec2<i32>(cell - floor(cell / size) * size), level);
    return vec4(texel.rgb, 1.0);
}
// The specular texel at `uv` for a footprint of `footprint` (the texture coordinate's change a
// pixel), read at the map's mip level for that size.
fn specular_at(layer: u32, uv: vec2<f32>, footprint: f32) -> vec4<f32> {
    let slot = specular_slot(layer);
    if slot.array == 0xffffu { return SPECULAR_NEUTRAL; }
    let lod = log2(max(footprint * slot.size, 1e-6));
    let level = clamp(i32(round(lod)), 0, i32(log2(slot.size)));
    let size = max(slot.size / exp2(f32(level)), 1.0);
    let cell = floor(uv * size);
    return specular_load(slot, vec2<i32>(cell - floor(cell / size) * size), level);
}
// Texel `cell` (wrapped into the map) of mip `level` (0 the map's own size).
fn parallax_texel(slot: ParallaxSlot, cell: vec2<f32>, level: i32) -> vec4<f32> {
    let size = max(slot.size / exp2(f32(level)), 1.0);
    return parallax_load(slot, vec2<i32>(cell - floor(cell / size) * size), level);
}
// How far below the surface texel `cell` of the map lies, in blocks.
fn parallax_depth(slot: ParallaxSlot, cell: vec2<f32>) -> f32 {
    return (1.0 - parallax_texel(slot, cell, 0).a) * 0.25 * PARALLAX_DEPTH;
}
// Vanilla's directional shade of a face facing `n` (up 1, north and south 0.8, east and west
// 0.6, down 0.5), blended for normals between the axes.
fn parallax_shade(n: vec3<f32>) -> f32 {
    let w = n * n;
    return w.x * 0.6 + w.z * 0.8 + w.y * select(0.5, 1.0, n.y > 0.0);
}

struct ParallaxHit {
    uv: vec2<f32>,
    // Tangent space: x along u, y along v (LabPBR's down), z out of the surface.
    normal: vec3<f32>,
    ao: f32,
    depth: f32,
}
// Where the view ray `ray` (tangent space, z < 0 into the surface) meets the height map from
// `uv`, crossing whole texels.
fn parallax_march(slot: ParallaxSlot, uv: vec2<f32>, ray: vec3<f32>, strength: f32)
    -> ParallaxHit {
    let size = slot.size;
    let deepest = 0.25 * PARALLAX_DEPTH * strength;
    // Texels per block of depth along the ray.
    let d = ray.xy / max(-ray.z, 0.05) * size;
    let p = uv * size;
    var cell = floor(p);
    let step = sign(d);
    let inverse = select(vec2(1e9), 1.0 / abs(d), abs(d) > vec2(1e-6));
    // Depth along the ray at which it leaves the cell on each axis.
    var exit = select(vec2(1e9), (cell + max(step, vec2(0.0)) - p) / d, abs(d) > vec2(1e-6));
    var t = 0.0;
    var wall = vec2(0.0);
    var hit = false;
    for (var i = 0u; i < PARALLAX_STEPS; i++) {
        let h = parallax_depth(slot, cell) * strength;
        let leave = min(min(exit.x, exit.y), deepest);
        if h <= leave {
            if h >= t {
                t = h;
                wall = vec2(0.0);
            }
            hit = true;
            break;
        }
        if leave >= deepest {
            t = deepest;
            break;
        }
        if exit.x < exit.y {
            t = exit.x;
            cell.x += step.x;
            exit.x += inverse.x;
            wall = vec2(-step.x, 0.0);
        } else {
            t = exit.y;
            cell.y += step.y;
            exit.y += inverse.y;
            wall = vec2(0.0, -step.y);
        }
    }
    let texel = parallax_texel(slot, cell, 0);
    var normal = parallax_unpack(texel);
    if hit && any(wall != vec2(0.0)) {
        // A wall turns as the relief fades, so it never flips from one pixel to the next.
        normal = normalize(mix(normal, vec3(wall, 0.0), strength));
    }
    return ParallaxHit(uv + d / size * t, normal, texel.b, t / max(deepest, 1e-6));
}
// The relief's own shadow (POM self-shadowing): from where the view ray landed (`uv`, `depth`
// blocks into the map) toward the light `l` (tangent space, z out of the surface), the share of
// it the height map lets through: none where a texel stands above the ray on its way out, so the
// sunward wall of a groove the low sun can't reach into stays dark.
fn parallax_self_shadow(slot: ParallaxSlot, uv: vec2<f32>, depth: f32, l: vec3<f32>,
    strength: f32) -> f32 {
    if depth <= 1e-4 || l.z <= 0.0 { return 1.0; }
    let slope = l.xy / max(l.z, 0.05);
    var lit = 1.0;
    for (var i = 1u; i <= 8u; i++) {
        let up = depth * f32(i) / 8.0;
        let at = (uv + slope * up) * slot.size;
        let h = parallax_depth(slot, floor(at)) * strength;
        // The map stands above the ray there: a soft edge over a sixteenth of the depth.
        lit = min(lit, smoothstep(-depth / 16.0, 0.0, h - (depth - up)));
    }
    return lit;
}
fn parallax_unpack(texel: vec4<f32>) -> vec3<f32> {
    let xy = texel.rg * 2.0 - 1.0;
    return vec3(xy, sqrt(max(1.0 - dot(xy, xy), 0.0)));
}

// The relief's surface at a fragment: the sprite's texel where the view ray landed (untinted),
// the face's normal and the map's (world space), the map's occlusion, where it landed and the
// texture coordinate's change a pixel.
struct ParallaxSurface {
    texel: vec4<f32>,
    geo: vec3<f32>,
    normal: vec3<f32>,
    ao: f32,
    uv: vec2<f32>,
    footprint: f32,
    mapped: bool,
}
fn parallax_surface(in: ParallaxOut, march: bool) -> ParallaxSurface {
    // Derivatives first, in uniform control flow.
    let dx = dpdx(in.uv);
    let dy = dpdy(in.uv);
    let dp1 = dpdx(in.position);
    let dp2 = dpdy(in.position);
    var n = normalize(cross(dp1, dp2));
    if dot(n, in.position) > 0.0 { n = -n; }
    // The cotangent frame: the world directions of +u and +v on the face.
    let p2 = cross(dp2, n);
    let p1 = cross(n, dp1);
    let tu = normalize(p2 * dx.x + p1 * dy.x);
    let tv = normalize(p2 * dx.y + p1 * dy.y);
    let layer = in.layer & 0x7fffu;
    let slot = parallax_slot(layer);
    let texel = parallax_sprite(in.uv, layer, dx, dy);
    let footprint = max(length(dx), length(dy));
    if slot.array == 0xffffu {
        return ParallaxSurface(texel, n, n, 1.0, in.uv, footprint, false);
    }
    // log2 of the map's texels per pixel: 0 where a texel covers one pixel.
    let lod = log2(max(max(length(dx), length(dy)) * slot.size, 1e-6));
    let distance = length(in.position);
    let near = 1.0 - smoothstep(PARALLAX_DISTANCE * 0.75, PARALLAX_DISTANCE, distance);
    let strength = select(0.0, near * (1.0 - smoothstep(-0.5, 0.5, lod)), march);
    let view = in.position / max(distance, 1e-6);
    var hit: ParallaxHit;
    if strength > 0.001 {
        hit = parallax_march(slot, in.uv, vec3(dot(view, tu), dot(view, tv), dot(view, n)),
            strength);
    } else {
        let level = clamp(i32(round(lod)), 0, i32(log2(slot.size)));
        let texel = parallax_texel(slot, floor(in.uv * slot.size / exp2(f32(level))), level);
        hit = ParallaxHit(in.uv, parallax_unpack(texel), texel.b, 0.0);
    }
    let world = normalize(hit.normal.x * tu + hit.normal.y * tv + hit.normal.z * n);
    let ao = hit.ao * (1.0 - PARALLAX_CAVITY * hit.depth);
    if strength > 0.001 {
        let l = horizon_terrain_globals.rt_light.xyz;
        let deepest = 0.25 * PARALLAX_DEPTH * strength;
        horizon_terrain_rt_relief_sun = parallax_self_shadow(slot, hit.uv, hit.depth * deepest,
            vec3(dot(l, tu), dot(l, tv), dot(l, n)), strength);
    }
    // The sprite where the ray landed, when it moved.
    var landed = texel;
    if strength > 0.001 {
        landed = parallax_sprite(hit.uv, layer, dx, dy);
    }
    return ParallaxSurface(landed, n, world, ao, hit.uv, footprint, true);
}
// Vanilla's light: the surface's texel shaded for the map's normal, times its occlusion.
fn parallax_color(in: ParallaxOut, march: bool) -> vec4<f32> {
    let s = parallax_surface(in, march);
    let albedo = s.texel * in.color;
    if !s.mapped { return albedo; }
    let shade = parallax_shade(s.normal) / max(parallax_shade(s.geo), 0.05);
    return vec4(albedo.rgb * clamp(shade, 0.0, 1.5) * s.ao, albedo.a);
}
// RT lighting: whether this fragment is lit by the rays (as terrain.wgsl's own).
fn parallax_rt(in: ParallaxOut) -> bool {
    return horizon_terrain_globals.rt_mode.x > 0.5 && horizon_terrain_linear_on()
        && in.emission == 0u;
}
// RT lighting: the sun's light through a thin material (`intensity`, 0..1: how much it scatters)
// in the colour `tint` (linear), added to what `rt_material_lit` gives for the material without
// its own subsurface term (terrain.wgsl's `rt_shade` has no tint to give it). It is lit where the
// face turns away from the sun, as `rt_shade`'s is, and fogged as `rt_material_lit` fogs: the
// air's extinction and, past the world's edge, vanilla's fog (the light being linear, both only
// scale it).
fn parallax_transmission(in: ParallaxOut, geo: vec3<f32>, tint: vec3<f32>, intensity: f32)
    -> vec3<f32> {
    if intensity <= 0.0 { return vec3(0.0); }
    let g = horizon_terrain_globals;
    let l = g.rt_light.xyz;
    let v = horizon_terrain_rt_visibility(in.position, in.clip.xy);
    let sun = g.rt_sun_light.rgb * g.rt_light.w * v.x
        * horizon_terrain_rt_sun_colour(in.clip.xy, v.x);
    let gate = smoothstep(0.0, 0.15, dot(geo, l));
    let through = sun * tint * (intensity * SSS_ENERGY * max(dot(-geo, l), 0.0) * (1.0 - gate));
    let air = vec3(5.802e-3 + 4.44e-3, 13.558e-3 + 4.44e-3, 33.1e-3 + 4.44e-3);
    let extinction = exp(-air * (length(in.position) * 0.001 * g.rt_mode.w));
    let f = horizon_terrain_fog_amount(in.spherical, in.cylindrical) * g.fog_color.a;
    return through * extinction * (1.0 - f);
}
// RT lighting: the surface as a LabPBR material, lit and fogged (linear).
fn parallax_rt_lit(in: ParallaxOut, s: ParallaxSurface) -> vec3<f32> {
    let layer = in.layer & 0x7fffu;
    let albedo = horizon_terrain_decode_gamma(s.texel.rgb * in.tint);
    let specular = specular_at(layer, s.uv, s.footprint);
    var m = horizon_terrain_rt_labpbr(albedo, s.normal, s.ao, specular);
    // The map's word on the subsurface scattering wins; where it has none, thin blocks scatter
    // and thick ones do not.
    let thin = thin_layers[layer] != 0u;
    let intensity = select(select(0.0, SSS_THIN, thin), m.subsurface, m.subsurface > 0.0);
    // The pack's tint, with the face's biome colour as a hue only (the map is as bright as the
    // light through the leaf), or the albedo's twice.
    let sss = subsurface_at(layer, s.uv, s.footprint);
    let hue = in.tint / max(max(in.tint.r, in.tint.g), max(in.tint.b, 1e-3));
    let tint = select(albedo * albedo, horizon_terrain_decode_gamma(sss.rgb * hue), sss.a > 0.5);
    m.subsurface = 0.0;
    return horizon_terrain_rt_material_lit(m, s.geo, in.position, in.clip.xy, in.light,
        in.spherical, in.cylindrical) + parallax_transmission(in, s.geo, tint, intensity);
}
@fragment fn parallax_fs_solid(in: ParallaxOut) -> @location(0) vec4<f32> {
    // Lava: its own look, not the pack's texture (terrain.wgsl `rt_lava`).
    if horizon_terrain_rt_lava_on(in.layer) {
        let n = horizon_terrain_face_normal(in.position);
        return vec4(horizon_terrain_rt_lava(in.position, n, in.clip.xy, in.light, in.spherical,
            in.cylindrical), 1.0);
    }
    if parallax_rt(in) {
        let s = parallax_surface(in, true);
        if s.texel.a < 0.5 && (in.layer & 0x8000u) == 0u { discard; }
        return vec4(parallax_rt_lit(in, s), 1.0);
    }
    // RT lighting: a light-emitting block lit as any other, its burning texels shining
    // (terrain.wgsl `rt_burning`).
    if horizon_terrain_globals.rt_mode.x > 0.5 && horizon_terrain_linear_on() {
        let s = parallax_surface(in, true);
        if s.texel.a < 0.5 && (in.layer & 0x8000u) == 0u { discard; }
        let lit = parallax_rt_lit(in, s);
        let color = s.texel * in.color;
        let glow = horizon_terrain_scene_color(vec4(color.rgb, 0.0),
            horizon_terrain_emission_gain(in.emission), in.spherical, in.cylindrical).rgb;
        return vec4(mix(lit, glow, horizon_terrain_rt_burning(s.texel.rgb)), 1.0);
    }
    let color = parallax_color(in, true);
    if color.a < 0.5 && (in.layer & 0x8000u) == 0u { discard; }
    let emissive = select(1.0, 0.0, in.emission > 0u);
    var out = horizon_terrain_scene_color(vec4(color.rgb, emissive),
        horizon_terrain_emission_gain(in.emission), in.spherical, in.cylindrical);
    // Ray-traced shadows and ambient occlusion scale this share of the light over the pack's own
    // ambient occlusion, already in `color` (terrain.wgsl `sun_share`).
    if horizon_terrain_linear_on() {
        out.a = 0.5 * horizon_terrain_sun_share(in.light, in.emission);
    }
    return out;
}
@fragment fn parallax_fs_translucent(in: ParallaxOut) -> @location(0) vec4<f32> {
    if parallax_rt(in) {
        // Water: its own look, not the pack's texture (terrain.wgsl `rt_water_at`).
        if horizon_terrain_rt_is_water(in.layer) {
            let n = horizon_terrain_face_normal(in.position);
            let w = horizon_terrain_rt_water_at(in.position, in.clip.xy, in.light, in.spherical,
                in.cylindrical, n, in.tint);
            return vec4(w.added / w.alpha, w.alpha);
        }
        let s = parallax_surface(in, false);
        if s.texel.a < 0.01 { discard; }
        return vec4(parallax_rt_lit(in, s), s.texel.a);
    }
    let color = parallax_color(in, false);
    if color.a < 0.01 { discard; }
    return horizon_terrain_scene_color(color, horizon_terrain_emission_gain(in.emission),
        in.spherical, in.cylindrical);
}
";

/// The declarations of `arrays` maps of a kind (`kind`: `parallax` the normal maps, `specular`,
/// `subsurface`), from binding `first`: the index, the arrays, their loader (`neutral` without a
/// map) and their sizes.
fn maps(kind: &str, first: u32, arrays: u32, neutral: &str) -> String {
    let mut out =
        format!("@group({GROUP}) @binding({first}) var<storage, read> {kind}_index: array<u32>;\n");
    for i in 0..arrays {
        out.push_str(&format!(
            "@group({GROUP}) @binding({}) var {kind}_maps_{i}: texture_2d_array<f32>;\n",
            first + 1 + i
        ));
    }
    out.push_str(&format!(
        "fn {kind}_load(slot: ParallaxSlot, texel: vec2<i32>, level: i32) -> vec4<f32> {{\n    \
         switch slot.array {{\n"
    ));
    for i in 0..arrays {
        out.push_str(&format!(
            "        case {i}u: {{ return textureLoad({kind}_maps_{i}, texel, slot.layer, level); }}\n"
        ));
    }
    out.push_str(&format!(
        "        default: {{ return {neutral}; }}\n    }}\n}}\n\
         fn {kind}_size(array: u32) -> u32 {{\n    switch array {{\n"
    ));
    for i in 0..arrays {
        out.push_str(&format!(
            "        case {i}u: {{ return textureDimensions({kind}_maps_{i}).x; }}\n"
        ));
    }
    out.push_str("        default: { return 1u; }\n    }\n}\n");
    out
}

/// The first binding of each kind of map (normal, specular, subsurface) for that many arrays of
/// each, and after them the thin layers' binding.
fn bindings([normals, speculars, subsurfaces]: [u32; 3]) -> ([u32; 3], u32) {
    let specular = normals + 1;
    let subsurface = specular + speculars + 1;
    ([0, specular, subsurface], subsurface + subsurfaces + 1)
}

/// The parallax terrain shader for that many normal, specular and subsurface map arrays.
fn shader_source(arrays: [u32; 3]) -> String {
    let terrain = prefixed(
        section(TERRAIN, "struct Globals {", "// Sky (SkyRenderer)"),
        PREFIX,
    );
    let ([normal, specular, subsurface], thin) = bindings(arrays);
    format!(
        "{terrain}{}{}{}\
         @group({GROUP}) @binding({thin}) var<storage, read> thin_layers: array<u32>;\n{ENTRIES}",
        maps("parallax", normal, arrays[0], "PARALLAX_NEUTRAL"),
        maps("specular", specular, arrays[1], "SPECULAR_NEUTRAL"),
        maps("subsurface", subsurface, arrays[2], "SUBSURFACE_NEUTRAL")
    )
}

/// The parallax pipelines, and the bind group of the maps they read.
pub struct Parallax {
    pub pipelines: NativeMaterial,
    pub group: wgpu::BindGroup,
}

impl Parallax {
    /// The pipelines over the normal, specular and subsurface `maps` and the `thin` layers; an
    /// error when the device cannot bind that many textures in a fragment shader. Where only the
    /// subsurface maps are too many, they are left out (a pack's tint is a nicety).
    pub fn new(
        device: &wgpu::Device,
        shape: &TerrainShape<'_>,
        maps: [&MapArrays; 3],
        thin: &wgpu::Buffer,
    ) -> Result<Self, String> {
        let limit = device.limits().max_sampled_textures_per_shader_stage;
        let mut counts = maps.map(|m| m.textures.len() as u32);
        let none;
        let mut maps = maps;
        if TERRAIN_TEXTURES + counts.iter().sum::<u32>() > limit
            && TERRAIN_TEXTURES + counts[0] + counts[1] <= limit
        {
            log::warn!(
                "Horizon: no subsurface tints: {} map arrays need more textures per shader than \
                 the device binds ({limit})",
                counts[2]
            );
            none = MapArrays::empty(device);
            maps[2] = &none;
            counts[2] = 0;
        }
        let needed = TERRAIN_TEXTURES + counts.iter().sum::<u32>();
        if needed > limit {
            return Err(format!(
                "{} normal and {} specular map arrays need {needed} textures per shader, the \
                 device binds {limit}",
                counts[0], counts[1]
            ));
        }
        let text = shader_source(counts);
        let module = naga::front::wgsl::parse_str(&text).map_err(|e| e.emit_to_string(&text))?;
        // Each kind's index and arrays from its first binding, then the thin layers.
        let (firsts, thin_binding) = bindings(counts);
        let kinds: Vec<(u32, &MapArrays)> = firsts.into_iter().zip(maps).collect();
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let mut entries = Vec::new();
        let mut views = Vec::new();
        for (first, maps) in &kinds {
            entries.push(storage(*first));
            for (i, t) in maps.textures.iter().enumerate() {
                entries.push(wgpu::BindGroupLayoutEntry {
                    binding: first + 1 + i as u32,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                });
                views.push((
                    first + 1 + i as u32,
                    t.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2Array),
                        ..Default::default()
                    }),
                ));
            }
        }
        entries.push(storage(thin_binding));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Parallax maps"),
            entries: &entries,
        });
        let mut bindings: Vec<_> = kinds
            .iter()
            .map(|(first, maps)| wgpu::BindGroupEntry {
                binding: *first,
                resource: maps.index.as_entire_binding(),
            })
            .collect();
        bindings.push(wgpu::BindGroupEntry {
            binding: thin_binding,
            resource: thin.as_entire_binding(),
        });
        bindings.extend(views.iter().map(|(binding, view)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: wgpu::BindingResource::TextureView(view),
        }));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Parallax maps"),
            layout: &layout,
            entries: &bindings,
        });
        let pipelines = NativeMaterial::build(
            device,
            shape,
            "Parallax terrain",
            module,
            Vec::new(),
            (GROUP, &layout),
            ["parallax_vs", "parallax_vs_flat"],
            ["parallax_fs_solid", "parallax_fs_translucent"],
        )?;
        Ok(Self { pipelines, group })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_validates_for_no_one_and_several_arrays() {
        for arrays in [
            [0, 0, 0],
            [1, 0, 0],
            [3, 2, 0],
            [3, 2, 1],
            [1, 1, 4],
            [7, 7, 7],
        ] {
            let text = shader_source(arrays);
            let module = naga::front::wgsl::parse_str(&text)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&text)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&text)));
            let entries: Vec<&str> = module
                .entry_points
                .iter()
                .map(|e| e.name.as_str())
                .collect();
            for entry in [
                "parallax_vs",
                "parallax_vs_flat",
                "parallax_fs_solid",
                "parallax_fs_translucent",
            ] {
                assert!(entries.contains(&entry), "{entry}");
            }
        }
    }

    /// Each map kind's index and arrays and the thin layers take consecutive bindings of the
    /// group, as `Parallax::new` lays them out.
    #[test]
    fn the_maps_and_the_thin_layers_take_consecutive_bindings() {
        for arrays in [[0, 0, 0], [2, 0, 1], [3, 2, 5]] {
            let text = shader_source(arrays);
            let marker = format!("@group({GROUP}) @binding(");
            let mut found: Vec<u32> = text
                .match_indices(&marker)
                .map(|(at, _)| {
                    let rest = &text[at + marker.len()..];
                    rest[..rest.find(')').unwrap()].parse().unwrap()
                })
                .collect();
            found.sort_unstable();
            let total = arrays.iter().sum::<u32>() + 4;
            assert_eq!(found, (0..total).collect::<Vec<_>>(), "{arrays:?}");
            assert_eq!(bindings(arrays).1, total - 1);
        }
    }
}
