# Horizon HDR

Horizon's own HDR look as a shader pack on the public API (docs/shaders.md,
[shaders-reference-0.2.md](../../../../docs/api/shaders-reference-0.2.md)): light sources shine past paper white, each
by its kind of light, and a tone curve fits that light to the display. It is the look the game draws itself with HDR
on and no pack (Look: Horizon), and the example to copy from: a pack that wants that look under its own starts here.
Like Golden Hour it comes with the SDK, not with the game: the game never picks it by itself, and nothing changes
until a player puts the folder in `shaderpacks/` and picks it. With HDR off it draws vanilla's picture.

`hdr_look.wgsl` is the look: its numbers (the brightness a texel lifts at, each tier's gain, the scene's peak) and
its maths, with no name of the renderer in it. There is one copy of it. The renderer's own shaders start with this
very file (`horizon-client-render`'s `gfx/wgsl_prefix.rs`) and the pack lists it as its shared source, so a change to
the look changes both. The stages only call it:

- `material.wgsl`, blocks: vanilla's colour, `albedo` under `shading`. On a light source (`emission`) the texels as
  bright as `emission_threshold` take their tier's mark and, as they lift, leave their face's shading behind, up to
  `frame.full_light`.
- `entity.wgsl`, entities and block entities: vanilla's colour, and what `light_source` names lifted by its tier.
- `tonemap.wgsl`: the look's tone curve, from the scene's peak to the display's.
- `lift.wgsl`: what the first two share. The look works on colours in vanilla's gamma, a stage on linear light.

Check it with `horizon-shader check crates/horizon-shader-api/packs/horizon_hdr`.

## What it can't draw yet

The frame identity test (`tools/test_frame_identity_parity.py`, `look_gates`) holds the pack to the look the game
draws itself, for blocks, entities and block entities: within a step of 255 in a screenshot and two of 1023 in an
HDR shot, the rounding of the pack's route (more 2.2 powers per pixel, a decode and an encode around `[tonemap]`).
What it can't reach waits for the API:

- **The sun's disc, the moon, stars, lightning, additive particles and beams** reach no stage. Under a pack they stay
  Horizon's, at paper white, until the `[glow]` stage.
- **Items and particles** aren't drawn through `[entity]` yet (#14583, steps 3 and 4), so what shines among them (a
  torch's flame, a glowing item) isn't lifted.
- **Light sources in fog, and in a section that is fading in.** Horizon lifts a pixel after both, by what they left
  of it; a stage runs before them. Near the camera there is no difference. The two agree once the look lifts before
  fog (docs/design/hdr-linear-light.md, step 7e).
- **The enchantment glint on something that shines** is added after a stage, and so after the lift; Horizon lifts
  it with the pixel.
- **The back faces of a model that shines and is drawn from both sides** (an End crystal's glass). Horizon lifts
  them by their front's brightness; a stage is handed the shade of the face it draws.
- **Resource packs' emissive overlays** (OptiFine's `_e` textures), which Horizon lays over a block and lifts: a
  material stage isn't handed them.
- **Native mods' coloured lights** are added at each corner of a face by Horizon, and at each pixel by a stage.
- **A screenshot's light above white.** Horizon maps a frame into SDR by its own rule: by the brightest channel with
  its look, so the hue stays, and channel by channel under a pack. `[tonemap]` runs for the display only so far. The
  HDR shot beside the screenshot is the pack's.
- **The lift's 8-bit steps.** The look rounds a pixel's mark to 8 bits, and the pack's rounding puts a few texels on
  the other side of a step (step 7g takes the steps out).
- **What any running pack turns off**: anti-aliasing (TAA, FXAA, TAA upscaling), Improved Transparency and frame
  generation.
