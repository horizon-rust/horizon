//! docs/api/shaders-reference-0.2.md, generated from the stages and inputs this crate defines
//! (`tests/reference.rs` fails when the file drifts).
use crate::check::{MAX_EXPRESSIONS, MAX_TEXTURE_READS};
use crate::inputs::{DISPLACEMENT_INPUT, Frame, RAY_HIT, SURFACE, Struct};
use crate::manifest::{API, MAX_GPU_MS, MAX_OPTIONS, MAX_POST_PASSES};
use crate::package::{MAX_TOTAL, MAX_WGSL};
use crate::stages::{
    GROUP, PBR_NORMALS_BINDING, PBR_SPECULAR_BINDING, RAY_ENTITY_RETAINED_BINDING,
    RAY_ENTITY_STORE_BINDING, RAY_SCENE_BINDING, RAY_SPRITE_ARRAYS, RAY_SPRITES_BINDING, StageKind,
};

fn table(s: &Struct, offsets: Option<&[usize]>) -> String {
    let mut out = format!("### `{}`\n\n{}\n\n", s.name, one_line(s.doc));
    out += if offsets.is_some() {
        "| Field | Type | Offset | Meaning |\n| --- | --- | --- | --- |\n"
    } else {
        "| Field | Type | Meaning |\n| --- | --- | --- |\n"
    };
    for (i, f) in s.fields.iter().enumerate() {
        let offset = offsets.map_or(String::new(), |o| format!(" {} |", o[i]));
        out += &format!(
            "| `{}` | `{}` |{offset} {} |\n",
            f.name,
            f.ty,
            one_line(f.doc)
        );
    }
    out + "\n"
}

fn one_line(doc: &str) -> String {
    doc.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where the reference lives, from the repository's root.
pub const PATH: &str = "docs/api/shaders-reference-0.2.md";

/// The line's rules, after the introduction.
const RULES: &str = "\
**Experimental** ([stability.md](stability.md)): `horizon-shader check` takes 0.2 packs, the game draws them and
the registry lists them, and anything here may change in any release.
The design: [hdr-linear-light.md](../design/hdr-linear-light.md).

## Colour

Every colour a 0.2 stage reads or returns is **linear light**: BT.709 (sRGB) primaries by default, scene-referred,
1.0 is vanilla's white at paper white, more is brighter. A pack may ask for BT.2020 primaries, which hold colours
outside sRGB, with `[frame] primaries` (below). Horizon decodes vanilla's values with a 2.2 power (vanilla's, not
the piecewise sRGB curve), replacing negatives and NaN with 0 first, and encodes a stage's result the same way where
the frame stores gamma (`[frame] blend`, below).

- **Linear:** `MaterialInput` `albedo`, `tint`, `lightmap` and `colored_light`; `VertexInput.tint`; `SkyInput.color`;
  `WeatherInput.color`; `Frame` `sky_color` and `fog_color`; `block_texture`, `block_texture_level` and
  `block_light_color`; `RayHit` `albedo` and `tint`; every colour texture a stage samples (`scene_color`, `previous`,
  `history`, the pack's targets, `opaque_color`, `reflection`, `shadow_color`).
- **Data:** depth, normals, `light` levels, `emission`, every channel of the LabPBR maps, and the pack's
  `[[textures]]`.
- **`[[lights]]` and `[[particle_lights]]` colours** are written in vanilla's gamma, and Horizon decodes
  them: `block_light_color` and `colored_light` are linear. `block_light_colors` holds them as written; read it through
  `block_light_color`.
- **Alpha is coverage only**, never light: brightness is in rgb alone. A post pass's alpha is ignored.

## Room to grow

`Frame`, the stage inputs (`SkyInput`, `VertexInput`, `MaterialInput`, `DisplacementInput`, `WeatherInput`,
`PostInput`, `TonemapInput`, `EntityInput`), `surface_at`'s `SurfaceData` and `trace`'s `RayHit` may gain fields in later
releases. Read their fields and pass them along, but don't construct them: `horizon-shader check` refuses
`PostInput(...)` and the like. `Surface` is yours to construct and keeps its one field; later material outputs come as
helpers.

**New names reach a stage only if the pack asked for them.** A later release puts a new name before your files (a
helper, a struct, a resource, a constant) only under a manifest key the pack sets, or in a stage added later, whose
prelude is its own. A stage a pack has today keeps the names it has today, so a name of your own that is free now
stays yours: an addition doesn't break a pack that didn't ask for it. Names starting with `horizon_` are Horizon's
already, and new fields can't collide, since you don't construct those structs.

## The display

`frame.hdr`, `headroom`, `paper_white`, `peak` and `black_level` describe the display, read again every frame. With HDR
on, where the device keeps the HUD on a layer of its own, a 0.2 pack sees the display's: `hdr` 1, `paper_white` and
`peak` in nits, `headroom` the peak over paper white (at least 1) and `black_level` the display's black in nits.
Otherwise it sees SDR: `hdr` 0, `headroom` 1, `paper_white` and `peak` 80, `black_level` 0, and its picture ends on an
8-bit frame, clipped at white, before the camera overlays and the HUD draw. Horizon lifts the shadows onto
`black_level` after `[tonemap]`: read it, but don't apply it.

## `[frame] blend`

`blend = \"gamma\"` (the default) keeps the frame in vanilla's gamma, values above 1 kept: Horizon encodes the material,
sky and weather stages' results before they are stored and blended, so glass, water, clouds and particles blend as
vanilla's. `blend = \"linear\"` stores linear light, so see-through draws blend as light does: Horizon stores those
stages' results as they are and decodes its own draws (entities, particles, clouds, weather, the stages a pack leaves
out), so a solid surface looks as under `\"gamma\"`. A stage sees the same values either way. Where the device can't
draw a float frame the pack draws on an 8-bit one, in gamma. Under `\"linear\"` resource packs' `core/sky`,
`core/stars` and `core/rendertype_lines` shaders, which write gamma, give way to Horizon's own sky, stars and block
outline.

## `[frame] primaries`

`primaries = \"bt709\"` (the default) is the rule above. With `primaries = \"bt2020\"` every colour a stage reads or
returns is in BT.2020 primaries instead (D65 white, 1.0 still vanilla's white at paper white, values never negative).
It needs `blend = \"linear\"`, since a change of primaries is only right in linear light. `frame.primaries` says which
space the pack's colours are in, and `frame.luminance` holds that space's luminance weights: compute luminance as
`dot(color, frame.luminance)` and it is right in either. `\"bt2020\"` passes the check; the game doesn't serve it yet.

";

/// `[ray_tracing]`, at the end of the post stage.
fn ray_tracing() -> String {
    let mut out = String::from(
        "### Ray tracing\n\n\
         With `[ray_tracing]` in the manifest (it has no keys, and needs a `[[post]]` pass, a `[material]` or an `[entity]`\n\
         stage) the post stage's prelude also has `RayHit` and the functions and constants below, so do the material and\n\
         entity stages' where their files name one of the four trace functions, and the `Frame` of every stage of the pack\n\
         has `has_ray_tracing` and `ray_distance`. A pack without the key has none of them, and a material or entity stage\n\
         that names no trace function is built as without it.\n\n\
         - `fn trace(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32) -> RayHit`: the nearest solid surface\n  \
         from `origin` (camera-relative, in blocks, as `view_position` gives it) along the unit vector `direction`, within\n  \
         `max_distance` blocks.\n\
         - `fn trace_any(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32) -> bool`: whether any solid surface\n  \
         is in the way: a shadow ray, which stops at the first it finds and reads nothing of it.\n\
         - `const RAY_SOLID: u32`, `const RAY_TRANSLUCENT: u32`: the scene's two layers, bits to OR together. Solid\n  \
         terrain, with cut-outs (leaves, plants) and lava; and what the translucent pass draws: water, glass, stained\n  \
         glass, ice, slime and honey blocks, any block whose texture has partly see-through texels.\n\
         - `const RAY_ENTITY: u32`: a third layer, only in a pack whose files name it, anywhere outside a comment (dead\n  \
         code too): the entities within the scene's reach, in view or not (behind the camera, hidden), which are mobs,\n  \
         players, armour stands, item frames, vehicles, projectiles, falling blocks, block displays, the block entities\n  \
         at rest that the block entity cache keeps (on in every graphics preset: closed chests, shulker boxes, bells,\n  \
         decorated pots, banners, within vanilla's 64 blocks) in view or not, and the other block entities the camera\n  \
         draws (a chest while it opens). Such a pack's `RayHit` also has `entity`. The camera's own body in first person\n  \
         is in it too, but a ray that starts within a quarter block of the camera (`dot(origin, origin) <= 0.0625`)\n  \
         passes it, since it starts inside it. Not in it: particles, text, the hand, translucent items, additive glows,\n  \
         beams and portals, and other block entities out of view. `trace` and `trace_any` never read it.\n\
         - `fn trace_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> RayHit` and\n  \
         `fn trace_any_layers(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, layers: u32) -> bool`: `trace`\n  \
         and `trace_any` over the layers `layers` names; `trace(o, d, m)` is `trace_layers(o, d, m, RAY_SOLID)`. With no\n  \
         layer named every ray misses.\n\
         - `fn hit_normal_map(hit: RayHit) -> vec4<f32>`, `fn hit_specular_map(hit: RayHit) -> vec4<f32>`: the resource\n  \
         packs' LabPBR maps at the hit, its sprite's texel at `hit.uv` at full size, as `block_normal_map_level(uv, 0.0)`\n  \
         and `block_specular_map_level(uv, 0.0)` read a drawn surface's; their defaults without maps, and on a miss.\n\
         - In the post and entity stages, `fn block_pbr_maps() -> bool`: whether the resource packs ship LabPBR maps, as\n  \
         the material stage's.\n\n\
         A ray starts at `origin` itself, so move a point off its own surface before tracing from it. What stops it is what\n\
         the surface's own pass draws. In the solid layer it passes through the clear texels of a cut-out texture (leaves,\n\
         plants), as terrain's alpha test does: a texel stops it from alpha 0.5 up. In the translucent layer every texel\n\
         stops it, however faint, except a clear one (alpha below 0.01), which the translucent pass doesn't draw either;\n\
         the hit's `albedo.a` is the texel's alpha and its `translucent` is true. In the entity layer a texel stops it where\n\
         vanilla's entity alpha test keeps the pixel, `(texel * tint).a` 0.1 or more (on the dying dragon's dissolve,\n\
         vanilla's mask decides), from either side of the face. A ray over several layers stops at the nearest surface of\n\
         any: to go on through a translucent one, trace again from just behind the hit. The scene is Horizon's: the\n\
         terrain around the camera, out to `frame.ray_distance` at least, without particles, clouds or distant terrain,\n\
         and with entities only in `RAY_ENTITY`. While\n\
         the game doesn't trace (`frame.has_ray_tracing` is 0) the pack is built and runs all the same, and every ray\n\
         misses. Your files name no ray query of their own (`ray_query`, `acceleration_structure`, `RayDesc`,\n\
         `RayIntersection`, a `rayQuery` function, `enable wgpu_ray_query`): the check refuses them, and an `output`\n\
         named as one of these functions.\n\n\
         In the material stage only `material` traces: `displacement` and the vertex stage's `vertex`, whose files share\n\
         its module, run in vertex shaders, and the check refuses a pack where either reaches a trace function.\n\
         `material` runs for every terrain fragment drawn, so a ray costs more there than in a post pass: keep its rays for\n\
         translucent surfaces (`input.translucent`), which leave no surface for a post pass to light.\n\n\
         In the entity stage `entity` traces as `material` does, in the world's colour pass only (the shadow map's\n\
         entities don't run the stage). A ray it casts with `RAY_ENTITY` in its layers can hit the entity it starts on:\n\
         move its origin off the surface, as from terrain.\n\n",
    );
    out += &table(&RAY_HIT, None);
    out += &format!(
        "Bindings {RAY_SCENE_BINDING} to {} of group {GROUP} hold the scene while the game traces ({RAY_ENTITY_STORE_BINDING} to\n\
         {RAY_ENTITY_RETAINED_BINDING} the entity layer, for a pack that names it), and in a post pass bindings {PBR_NORMALS_BINDING} and {PBR_SPECULAR_BINDING} the LabPBR maps (the\n\
         material and entity stages have them there already). They are Horizon's, as their `horizon_` names say, and change with the\n\
         renderer.\n\n",
        RAY_SPRITES_BINDING + RAY_SPRITE_ARRAYS - 1
    );
    out
}

/// The entity stage's notes, after its input: what it returns, and vanilla's own.
const ENTITY_NOTES: &str = "\
`entity` lights what is no block terrain: `input.kind` says which. It returns the same `Surface` as `material` (rgb
linear light, 0 or more and unbounded; alpha its coverage), so a pack writes its lighting once and calls it from both
stages. Around it Horizon keeps vanilla's order: the alpha test on the returned alpha, then the enchantment glint
added on top, then fog. A pack without `[entity]` is drawn as before, and sees none of this stage's names; the rule
for new names holds for the stage as for the others.

**What the game draws through the stage so far: entities and block entities** (`kind` 0 and 1). Until a later
release brings them in, Horizon lights the rest itself, exactly as without the stage: dropped and framed items and
what a block or a display entity shows of one (`kind` 2), what flies as its item (a snowball, a potion, a fireball,
an eye of ender), firework rockets, particles (`kind` 4), the fire on a burning entity, a guardian's beam, a lead's
rope, name tags and text, and the first-person hand. Some draws stay Horizon's for good: beacon beams, the beams of
End crystals, End portals and gateways, breaking cracks, energy swirls, lightning and other additive glows.

An item an entity holds or wears as its model (a sword in a hand, a block on a head) reaches the stage with the
entity, unless its texture has see-through pixels (stained glass, ice, a slime block): that one is drawn with the
translucent items, and stays Horizon's until items draw through the stage. A block drawn as an entity (a falling
block, primed TNT, a block display) or as a block entity (a block a piston moves) reaches the stage too. These have
vanilla's shade already in `tint`, and `shade` 1: their vertices carry the two as one colour so far. A sign's board
and a bed are block terrain, the material stage's.

Vanilla's own, the picture a pack without the stage gets:

```wgsl
fn entity(input: EntityInput) -> Surface {
    let base = input.albedo.rgb * input.tint.rgb * input.shade;
    // Vanilla lays the flash over the surface in its gamma (the 2.2 power), before the light.
    let gamma = vec3<f32>(1.0 / 2.2);
    let flashed = pow(mix(pow(base, gamma), pow(input.overlay.rgb, gamma), input.overlay.a), vec3<f32>(2.2));
    return Surface(vec4<f32>(flashed * input.lightmap, input.albedo.a * input.tint.a));
}
```

Without a flash (`overlay.a` 0) that is `albedo * tint * shade * lightmap`. In linear light alone,
`mix(base, input.overlay.rgb, input.overlay.a) * input.lightmap` is the same flash a little brighter. The shade and
the light act on rgb only. `lightmap` holds no coloured light: where `colored_light` is above 0, Horizon's own draw
adds it to the light colour, and the expression above leaves it out.

";

/// The reference.
pub fn markdown() -> String {
    let mut out = format!(
        "# Shader API reference ({API})\n\n\
         Generated from `crates/horizon-shader-api` (`HORIZON_WRITE_DOCS=1 cargo test -p horizon-shader-api --test reference`\n\
         rewrites it; the test fails when it drifts). The API's design and pack format: [shaders.md](shaders.md).\n\n"
    );
    out += RULES;
    out += "A stage is a function your WGSL defines, with the signature below; Horizon puts the prelude (these structs, the\n\
            resources and helpers) before your files and calls the function from its own pass. Your files declare no\n\
            bindings and no entry points, and no names starting with `horizon_`.\n\n\
            ## Stages\n\n\
            | Stage | In `horizon-shaders.toml` | Your function |\n| --- | --- | --- |\n";
    for kind in StageKind::ALL {
        let key = match kind {
            StageKind::Post => "[[post]]",
            StageKind::Sky => "[sky]",
            StageKind::Material => "[material]",
            StageKind::Vertex => "[vertex]",
            StageKind::Weather => "[weather]",
            StageKind::Tonemap => "[tonemap]",
            StageKind::Entity => "[entity]",
        };
        out += &format!("| {} | `{key}` | `{}` |\n", kind.name(), kind.signature());
    }
    out += "\n";
    for kind in StageKind::ALL {
        out += &format!("## {}\n\n`{}`\n\n", kind.name(), kind.signature());
        out += &table(kind.input(), None);
        if kind == StageKind::Entity {
            out += ENTITY_NOTES;
        }
        if kind == StageKind::Material {
            out += &table(&SURFACE, None);
            out += "Under `[displacement]` the stage also defines \
                    `fn displacement(input: DisplacementInput) -> f32`, called for each corner of the grid the \
                    faces near the camera are cut into; the corner moves along the face's normal by the depth it \
                    gives at the point's height less the depth at the texture's mean height, pinned at the face's \
                    edges and easing out over the last quarter of the distance.\n\n";
            out += &table(&DISPLACEMENT_INPUT, None);
        }
        out += &format!(
            "Resources (bind group {GROUP}):\n\n| Binding | Declaration | Meaning |\n| --- | --- | --- |\n"
        );
        for r in kind.resources() {
            out += &format!("| {} | `{}` | {} |\n", r.binding, r.ty, r.doc);
        }
        out += "\n";
        if kind == StageKind::Tonemap {
            out += "It is a pure mapping from the scene's light to the output its `frame` describes, run once per pixel of\n\
                    each output, after the post chain and before the camera overlays and the HUD. Today the display is the\n\
                    only output; Horizon may add others for the same frame, such as an SDR screenshot or icon, where `frame`'s\n\
                    display fields describe SDR (`hdr` 0, `headroom` 1, `paper_white` and `peak` 80, `black_level` 0). Its\n\
                    only output is the colour it returns, so nothing carries from one call to the next. A pack without it gets\n\
                    Horizon's: a clip at white in SDR, and in HDR the curve Horizon uses without a pack, which keeps light up\n\
                    to 60% of the peak as it is and rolls it off into the peak above. Either runs once: Horizon doesn't map a pack's\n\
                    result again. Horizon holds the result to 0 to `frame.headroom`, then lifts the shadows onto\n\
                    `frame.black_level`.\n\n";
            continue;
        }
        if kind != StageKind::Sky {
            out += "Helpers:\n\n";
        }
        if kind == StageKind::Post {
            out += "- `fn depth_at(uv: vec2<f32>) -> f32`: the world's depth at `uv`.\n\
                    - `fn view_position(uv: vec2<f32>, depth: f32) -> vec3<f32>`: the camera-relative position, in blocks, of\n  \
                    the point at `uv` and `depth`.\n\
                    - `fn surface_at(texel: vec2<i32>) -> SurfaceData`: the solid terrain's surface at a pixel, as the \
                    material stage's `set_surface` left it: `present` (false where the sky, an entity, water, glass or a \
                    particle shows instead), `normal` (world axes), `smoothness` and `data`.\n";
        }
        if kind == StageKind::Material {
            out += "- `fn block_texture(uv: vec2<f32>) -> vec4<f32>`: the surface's block texture at `uv`, anywhere on \
                    the sprite, without the tint.\n\
                    - `fn block_texture_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`: the same at mip level `lod`.\n\
                    - `fn block_texture_size() -> vec2<f32>`: its size in texels.\n\
                    - `fn block_texture_lod() -> f32`: the mip level `block_texture` reads at this point.\n";
            out += "- `fn block_pbr_maps() -> bool`: whether the resource packs ship LabPBR maps (`_n`, `_s`).\n\
                    - `fn block_normal_map(uv: vec2<f32>) -> vec4<f32>`: the surface's LabPBR normal map at `uv` (rg the \
                    normal along `input.tangent` and down the texture, b ambient occlusion, a height); (0.5, 0.5, 1, 1) \
                    without one.\n\
                    - `fn block_specular_map(uv: vec2<f32>) -> vec4<f32>`: its LabPBR specular map (r smoothness, g F0 or a \
                    metal, b porosity or subsurface, a emission, 1 none); (0, 0, 0, 0) without one.\n\
                    - `fn block_normal_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`, \
                    `fn block_specular_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`: the same at the block texture's mip level `lod`.\n\
                    - `fn block_pbr_size() -> vec2<f32>`: the LabPBR maps' size in texels.\n\
                    - `fn set_surface(normal: vec3<f32>, smoothness: f32, data: f32)`: what the post stage's \
                    `surface_at` reads for this point of solid terrain (`data` the pack's own; the face's normal and 0, 0 unset).\n";
        }
        if kind == StageKind::Entity {
            out += "- `fn block_light_color(position: vec3<f32>) -> vec4<f32>`: the colour of the block light at the \
                    camera-relative `position` (rgb) and how strongly it reaches it (a), as the material stage's.\n";
        }
        if kind != StageKind::Sky {
            out += "- `fn shadow_position(position: vec3<f32>) -> vec3<f32>`: where the camera-relative point `position` is\n  \
                    stored in the shadow map, for `textureSampleCompareLevel`.\n\
                    - `fn shadow_distort(xy: vec2<f32>) -> vec2<f32>`: the map's distortion of a point at shadow clip `xy`.\n\n";
        }
        if kind == StageKind::Post {
            out += "A `[[post]]` entry with `output = \"<name>\"` writes a target of the pack's own, at its `scale` of the\n\
                    world's size (1, 0.5 or 0.25), instead of the next pass's `previous`; the prelude of every pass declares\n\
                    `var <name>: texture_2d<f32>` (group 3 from binding 32), which holds last frame's until written again (a\n\
                    pass reads a blank one in place of the target it writes).\n\n";
            out += &ray_tracing();
        }
    }
    out += "## The frame\n\n";
    out += &table(&Frame::WGSL, Some(Frame::OFFSETS));
    out += "## Anti-aliasing\n\n\
            While a pack's `[[post]]` chain draws the frame, the player's anti-aliasing settings do not run: `taa`, `fxaa`\n\
            and `upscaling` in `horizon-graphics.txt` have no effect, and the frame's projection is never jittered, because the\n\
            last post pass draws the picture itself and Horizon's own resolve, which does TAA, FXAA and TAA upscaling, is the\n\
            step it replaces. The world is still rendered at `resolutionScale` before the pack reads it.\n\n\
            A pack that wants smooth edges draws them itself, in a post pass: a spatial filter (FXAA, SMAA and the like) works on\n\
            `previous` as it is. A temporal pass cannot do what TAA does here, even though `history` and\n\
            `frame.previous_view_projection` give it the last frame: nothing moves the camera by a fraction of a texel from one\n\
            frame to the next, so a still camera gives it the same picture twice and there is no sub-pixel detail to gather.\n\n\
            Without a pack the settings apply as the player set them.\n\n";
    out += &format!(
        "## Options\n\n\
         Each `[[options]]` entry sets the WGSL `override` of the same name when the pipelines are built:\n\n\
         | `type` | Keys | The `override` |\n| --- | --- | --- |\n\
         | `bool` | `default` | `override <id>: bool` |\n\
         | `float` | `default`, `min`, `max`, `step` | `override <id>: f32` |\n\
         | `int` | `default`, `min`, `max` | `override <id>: i32` |\n\
         | `choice` | `values`, `default` (one of them) | `override <id>: u32`, the value's index |\n\n\
         ## Limits\n\n\
         - {MAX_POST_PASSES} post passes, {MAX_OPTIONS} options, `budget.gpu_ms` up to {MAX_GPU_MS}.\n\
         - {} KiB per WGSL file, {} MiB per pack.\n\
         - Per stage: {MAX_EXPRESSIONS} naga expressions over all its functions, {MAX_TEXTURE_READS} texture reads (samples,\n  \
         gathers and loads, the prelude's included).\n\n\
         ## The check\n\n\
         A pack runs only after it passes `horizon_shader_api::check` (the game when it loads a pack, the launcher before it\n\
         installs one, the registry's CI): each stage is parsed and validated by naga, checked against its signature and\n\
         the rules above, and translated for Vulkan (SPIR-V), Metal and DirectX 12 (HLSL) with its options at their\n\
         defaults, minimums and maximums. Every option must be an `override` of its type in some stage; an `override`\n\
         that is no option needs a default.\n",
        MAX_WGSL >> 10,
        MAX_TOTAL >> 20
    );
    out
}
