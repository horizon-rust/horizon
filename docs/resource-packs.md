# Resource packs: what works

Java Edition resource packs load in Horizon: put the folder or `.zip` in `resourcepacks/`, pick it on the Resource Packs
screen (the same screen as vanilla's) or in game, and every loader reloads from it. This page lists everything a
26.3 resource pack can change and whether Horizon honours it, with the proof.

## How it works

`packs.rs` finds the packs (`FolderRepositorySource`: folders and zips, plus the built-in Programmer Art and High
Contrast, which only the asset index holds: preparation copies them, `horizon-prepare packs` repairs older resources),
reads `pack.mcmeta` (format range, overlays, filters) and builds one overlay directory that layers the
selected packs over the prepared vanilla resources, later packs winning. Files vanilla reads as a resource *stack*
(language files, `sounds.json`, font definitions, atlas definitions) are merged layer by layer as vanilla merges them.
Every loader then reads that one tree; `game/packs.rs` (`reload_packs`) reloads all of them in game, as
`Minecraft.reloadResourcePacks` does. Unchanged selections reuse the overlay ([reuse-pack-overlays.md](reuse-pack-overlays.md)).

Vanilla 26.3's reload listeners (`Minecraft.<init>`, `Gui`, `Hud`): languages, textures, shaders, sounds, atlases,
fonts, grass / foliage / dry foliage colour maps, models (block states, block models, item definitions), equipment
assets, paletted textures, entity and block entity renderers, particles, the game renderer (post effects), the level
renderer, clouds, GPU warn list, regional compliance, splashes and waypoint styles. Every row below comes from one
of them.

## The table

**Works**: Horizon reads it from the pack and draws or plays it like vanilla. **Partly**: some of it.
**No**: ignored (the vanilla version stays). Proof: a parity harness in `tools/` (compares with the pinned 26.3
client), a unit test, or the loader reading the overlay (vanilla's files go through the same loader, and the
harness named checks that loader against vanilla).

| Feature | Files | State | Proof |
| --- | --- | --- | --- |
| Pack metadata: description, icon, `min_format`/`max_format`, old `supported_formats`, compatibility warning | `pack.mcmeta`, `pack.png` | works | `test_pack_screen_parity`, `test_pack_selection_parity` |
| Overlays per format | `pack.mcmeta` `overlays` | works | `test_resource_pack_parity` (an overlay for 26.3 applied, an older one not) |
| Filters | `pack.mcmeta` `filter` | works | `test_resource_pack_parity` (`horizon-client-core/src/pattern.rs` regex subset) |
| Folder and zip packs, order, built-in packs | `resourcepacks/` | works | `test_resource_pack_parity`, `test_pack_selection_parity` |
| Server resource packs: the prompt, the server list's Server Resource Packs setting, download (250 MiB, SHA-1, `downloads/` keeps 20), download toasts, a progress screen while the packs reload, applied on top of the player's packs, removed on leaving | the server's `resource_pack_push`/`_pop` | works | `tools/server_pack_e2e.py` (against vanilla's server and Horizon's), unit tests in `server_packs.rs`; `test_server_resource_pack_parity` for Horizon's server |
| Other namespaces | `assets/<namespace>/` | works | `test_resource_pack_parity` |
| Block states: variants, multipart, weights, rotations, uvlock | `blockstates/` | works | `test_blockstate_model_parity`, `test_model_choice_parity`, `test_blockstate_transform_parity` |
| Block models: parents, elements, rotations, faces, cullface, tint, shade, light emission, ambient occlusion | `models/block/` | works | `test_model_resolution_parity`, `test_model_baking_parity`, `test_section_mesh_parity` |
| Item definitions and item models (all model types, display transforms, tints) | `items/`, `models/item/` | works | `test_item_model_parity` |
| Entity models | (code in vanilla) | n/a | entity models aren't resource files in 26.3 (`test_entity_model_parity` checks the code ones) |
| Textures, any resolution (16x to 512x) | `textures/` | works | atlases stitched from the overlay; Faithful 32x, Default HD 128x and FPBR measured in [reuse-pack-overlays.md](reuse-pack-overlays.md) |
| Texture animation: frames, frame time, frame size | `*.png.mcmeta` `animation` | works | `test_sprite_animation_parity` |
| Texture animation: `interpolate` | `*.png.mcmeta` `animation` | **no** | frames step instead of blending (vanilla's own prismarine, magma, stems, lit furnaces and campfires, command blocks, sculk too): fix in progress |
| LabPBR 1.3 material maps: normal, material AO, height; smoothness, F0 and metals, porosity, subsurface scattering, emission | `*_n.png`, `*_s.png`, `optifine/texture.properties` | partly | loaded beside the block sprites; the Parallax setting (off by default) draws the normal maps: relief, shading, material AO. The specular map waits for the shader rework ([LabPBR maps](#labpbr-maps)); `labpbr.rs`, `sprites.rs`, `gfx/materials.rs`, `gfx/parallax.rs` tests |
| Connected textures: `method=ctm` (47 tiles), `connect` (`block`, `tile`, `state`), `faces`, `innerSeams`, `matchBlocks` with properties, `matchTiles`, `<default>`/`<skip>` tiles | `optifine/ctm/**/*.properties` | works | `ctm.rs` tests (the tile table checked against SPBR's 47 glass tiles) |
| Connected textures: other methods (`ctm_compact`, `horizontal`, `vertical`, `horizontal+vertical`, `top`, `random`, `repeat`, `fixed`, `overlay*`), `biomes`, `heights`, `name`, `weights`, `symmetry`, numeric ids | `optifine/ctm/` | **no** | a rule with another method is skipped with a warning ([Connected textures](#connected-textures)) |
| Emissive overlays | `optifine/emissive.properties`, `*_e.png` | **no** | not read (a LabPBR pack's emission is in `_s` instead) |
| Mipmap strategy, alpha cutoff bias | `*.png.mcmeta` `texture` | works | `test_mipmap_parity` |
| `blur` / `clamp` | `*.png.mcmeta` `texture` | partly | vanilla's own files (vignette, pumpkin blur, shadow, glint, nausea) are drawn with their vanilla sampling; a pack that changes the flags is ignored |
| Atlases and sprite sources: `directory`, `single`, `filter`, `unstitch`, `paletted_permutations` | `atlases/` | works | `test_resource_pack_parity` (merged source lists), `atlas_sources.rs` tests |
| GUI sprites | `textures/gui/sprites/` | works | `test_hud_parity`, `test_screen_layout_parity` |
| GUI sprite scaling: `nine_slice` border | `*.png.mcmeta` `gui` | works | `ui.rs` reads the border; vanilla's buttons, sliders and frames use it |
| GUI sprite scaling: `width`/`height`, `stretch_inner`, `tile`, `stretch` | `*.png.mcmeta` `gui` | partly | the border is taken in texels, so HD GUI sprites get thin borders; `stretch_inner` (vanilla's tooltip frame) and `tile` aren't read |
| Fonts: `bitmap`, `space`, `unihex`, `reference` | `font/*.json`, `textures/font/` | works | `test_resource_pack_parity` (merged provider lists) |
| Fonts: `ttf` | `font/*.json` | **no** | no TrueType rasteriser yet |
| Fonts picked by text components (`"font": "..."`, custom fonts) | `font/` | partly | the default font and the enchanting table's `alt` glyphs are drawn; other fonts named by a text component use the default |
| Languages, translations | `lang/*.json` | works | `test_resource_pack_parity` (key by key, as `ClientLanguage`) |
| New languages | `pack.mcmeta` `language` | works | `lang.rs` reads every pack's `language` section into the language list |
| Sounds and sound events (`replace`, weights, streams, event references) | `sounds.json`, `sounds/` | works | `test_resource_pack_parity`, `test_sound_event_parity`, `test_sound_engine_parity` |
| Splashes | `texts/splashes.txt` | works | `title.rs` reads it from the overlay |
| Credits, end poem | `texts/credits.json`, `end.txt`, `postcredits.txt` | **no** | Horizon has no credits screen yet (Options → *Credits & Attribution* opens nothing) |
| Grass, foliage and dry foliage colours | `textures/colormap/` | works | `biome.rs` reads the overlay; `test_section_mesh_parity` checks the tints |
| Entity textures and variants | `textures/entity/` | works | `test_entity_model_parity`, loaded from the overlay |
| Block entities: chests, signs, beds, banners, shields, shulker boxes, decorated pots | `atlases/`, `textures/entity/` | works | `test_block_entity_render_parity` |
| Villager hats | `*.png.mcmeta` `villager` | **no** | the type layer's hat is always drawn; vanilla hides it under a profession hat marked `full`/`partial` |
| Equipment (armour, horse/llama/wolf/nautilus armour, wings, dyeable layers) | `equipment/`, `textures/entity/equipment/` | works | `game/armor_stands.rs`, `mobs/render.rs` read the overlay |
| Armour trims on items | `atlases/items.json`, `textures/trims/items/` | works | `paletted_permutations` (`atlas_sources.rs` tests) |
| Armour trims on worn armour | `textures/trims/entity/`, `textures/palettes/`, `palette` metadata | **no** | worn trims aren't drawn yet (`PalettedTextureManager`) |
| Particles | `particles/`, `textures/particle/` | works | `test_particle_providers_parity` |
| Sky: sun, moon phases, stars, end sky, clouds, rain and snow | `atlases/celestials.json`, `textures/environment/` | works | `test_environment_render_parity`, `test_cloud_render_parity`, `test_weather_render_parity` |
| Title panorama | `textures/gui/title/background/` | works | `gfx/panorama.rs` loads it from the overlay |
| Screen overlays: vignette, pumpkin, powder snow, nausea, underwater, world border | `textures/misc/` | works | `test_world_border_render_parity`, `test_hud_parity`; loaded from the overlay |
| Enchantment glint | `textures/misc/enchanted_glint_*` | works | `item_render/gui.rs`, `entity.wgsl` |
| Maps: background, decorations | `textures/map/`, `atlases/map_decorations.json` | works | loaded from the overlay |
| Paintings, mob effect icons | `atlases/paintings.json`, `textures/mob_effect/` | works | loaded from the overlay |
| Locator bar styles | `waypoint_style/` | **no** | the locator bar isn't drawn yet |
| Post effects (spectating a creeper, spider or enderman; glowing outlines) | `post_effect/`, `shaders/post/` | **no** | Horizon's effects are WGSL; a pack's GLSL post chain isn't run |
| Core shaders | `shaders/core/`, `shaders/include/` | **no** | Horizon draws with WGSL; a pack's GLSL core shaders aren't run |
| GPU warn list, regional compliance | `gpu_warnlist.json`, `regional_compliancies.json` | n/a | OpenGL driver warnings and play-time notices for some regions; not used |

Summary: 49 rows; 33 work, 4 partly, 10 don't (interpolated animation, TrueType fonts, credits and end poem,
villager hats, trims on worn armour, locator bar styles, post effects, core shaders, connected-texture methods
other than `ctm`, emissive overlays), 2 don't apply. Except the shaders and the OptiFine formats (LabPBR,
connected textures, emissive overlays), the missing pieces are missing vanilla features too, and each vanilla
parity fix covers the pack case as well.

## Gaps and their work

| Gap | Size | Where |
| --- | --- | --- |
| Animation `interpolate` | small | fix PR (`gfx.rs` `animate_sprites`) |
| GUI scaling `width`/`height`, `stretch_inner`, `tile`, `stretch` | small | next fix PR (`ui.rs`) |
| Villager hats | small | [#334](https://github.com/ludovic111/horizon/issues/334) |
| `blur`/`clamp` from packs | small | [#335](https://github.com/ludovic111/horizon/issues/335) |
| Text component fonts, TrueType fonts | medium | [#336](https://github.com/ludovic111/horizon/issues/336) |
| Trims on worn armour (paletted textures) | medium | [#337](https://github.com/ludovic111/horizon/issues/337) |
| Credits and end poem screen | medium | [#338](https://github.com/ludovic111/horizon/issues/338) |
| Locator bar | medium | [#339](https://github.com/ludovic111/horizon/issues/339) |
| Post effects and core shaders | large | none yet |
| A renderer stage that draws the LabPBR maps | large | the shader rework ([shaders.md](shaders.md#labpbr-materials)) |
| Connected-texture methods other than `ctm` | medium | `ctm.rs` |
| Emissive overlays (`optifine/emissive.properties`) | small | none yet |

## LabPBR maps

A pack's `<texture>_n.png` and `<texture>_s.png` beside a block texture are read as LabPBR 1.3
(shaderlabs.org's *LabPBR Material Standard*), whatever `optifine/texture.properties` declares (the game logs the
format it declares). `sprites.rs` keeps each map at its own size (a power of two, up to 512): maps are grouped by size, so
SPBR's 16x maps stay 16x even though two 64x textures make the block sprite arrays 64x, and a texture without a
map takes no room. An index gives, for each sprite layer, where its map is (`labpbr::MapSet`); a sprite
without one reads neutral texels (`labpbr::NEUTRAL`: flat, unoccluded, rough, F0 0, no emission). Maps are
sampled with the sprite's own texture coordinate, so they line up with it whatever their size. Mip levels
are averaged as data, not colours (`mipmap::Strategy::Data`), and an animated texture's maps show their frames
with the texture's own (a map of fewer frames repeats them). `gfx/materials.rs` keeps them on the GPU as one
texture array per size (per 2048 layers) and an index buffer (`array << 16 | layer` by sprite layer,
`Renderer::material_arrays`), the lookup a ray-traced stage can use too, and animates them with the sprites. `labpbr::decode` is
what a texel means (predefined metals 230 to 237 with their `n` and `k`, 238 to 255 the albedo as F0, porosity
0 to 64, subsurface scattering 65 to 255, emission 0 to 254).

They are only made when a pack has maps. Options > Horizon Settings > Parallax (off by default, as it changes
vanilla's look; saved as `parallax` in Horizon's graphics settings) draws terrain with the normal maps
(`gfx/parallax.rs`):

- **Parallax occlusion mapping**: each fragment follows the view ray into the height (alpha) texel by
  texel, up to 48 texels, so a 16x pack keeps its pixels: the ray stops on a texel's top, lit by the map's
  normal, or on the wall between two texels, lit as that wall. Against aliasing it runs only where a texel of
  the map covers about a pixel or more (from the texture coordinate's derivatives): the relief fades out as
  texels shrink below a pixel, with distance and at grazing angles (and beyond 18 to 24 blocks in any case),
  and the normal and occlusion are then read from the map's mip level for that size. A lower resolution
  (the resolution scale, a foveated view's periphery) gets there sooner, so it marches less. Depth is not
  written (the silhouette stays the face's).
- **Normal shading**: vanilla's directional face shade (up 1, north and south 0.8, east and west 0.6, down 0.5)
  recomputed for the map's normal, relative to the face's, so the light stays vanilla's.
- **Ambient occlusion**: the map's material AO (blue) on top of vanilla's smooth lighting, and a little darker
  the deeper the ray went (`PARALLAX_CAVITY`, at most a quarter): a stand-in for the occlusion a ray-traced
  AO will compute, kept apart so it can be turned off then.
- Translucent terrain (water, glass) gets the normal shading and the AO, without the march. A shader pack's
  material stage, when one runs, takes precedence.

The specular map (reflections, emission, subsurface scattering, porosity) is not drawn yet: it needs real
lighting, which comes with the shader rework. Item, entity and armour maps are not read (block sprites only).

Cost with SPBR 22.5, measured on 2026-10-27 (RTX 4090, Vulkan, 1280x720 offscreen, `horizon-ctl bench 10`
on a test scene of stone, bricks, glass, glowstone and logs at 4 to 8 blocks):

| | GPU time per frame (median) | Video memory |
| --- | --- | --- |
| Vanilla | 0.36 ms | |
| SPBR, Parallax off | 0.35 ms | +5.6 MiB of maps (1 929 normal and 2 088 specular maps) |
| SPBR, Parallax on | 0.52 ms | the same |

- **Memory**: 5.6 MiB with each map at its own size (the game logs it: `LabPBR maps: ...`). The first version
  stored every map at the sprite arrays' size, 64x because of two 64x textures in SPBR: 91.2 MiB. The block
  sprites still pay that factor (every sprite layer is the largest texture's size); the same grouping by
  size would save it there too. It is the same on the desktop and in VR: it does not depend on the picture's
  size.
- **Parallax**: about +0.16 ms for 0.9 megapixels of mostly near terrain, about 0.2 ms per megapixel. The cost
  is per terrain pixel where texels are at least a pixel: one texel load per texel the ray crosses (1 on a
  flat texel, at most 48 at grazing angles); elsewhere one load. In VR (Apple Vision Pro through KRVR,
  4096x3288 per eye, 27 megapixels for both eyes at 90 Hz), scaling that measure would be a few milliseconds
  on the same GPU, less where the foveated views draw at a lower resolution, since their texels reach a pixel
  sooner: not measured in VR yet.
- Animated maps add a compute dispatch per changed tick, as animated sprites do; without the Parallax setting
  nothing samples the maps.

## Connected textures

SPBR's `optifine/ctm` has 35 rules, all `method=ctm` with `connect=block`: glass, tinted glass, the 16 stained
glasses and their panes (`faces=sides`), 18 sets of 47 tiles with their own `_n`/`_s` maps. That is what
`ctm.rs` draws: the mesher asks it for each face, which looks at the eight neighbours in the face's plane (the
face's texture coordinates give which way is up and right) and picks one of the 47 tiles of OptiFine's
template. Rules naming sprites (`matchTiles`) are tried before rules naming blocks, in file order; a rule with
`matchBlocks` and no `matchTiles` changes the faces drawn with the block's particle sprite, so a pane's thin
edges keep their sprite. `connect=tile` is approximated by "drawn by the same rule". Checked in game with SPBR: a 4x4 glass wall draws as one pane with a single outer frame.

Missing for packs that use more of the format (for example rotrBLOCKS or Pixlli, not checked against them):
the other methods (`ctm_compact`, `horizontal`, `vertical`, `horizontal+vertical`, `top`, `random` with
`weights` and `symmetry`, `repeat`, `fixed`, the `overlay` family), the `biomes`, `heights` and `name`
conditions, faces rotated by an `axis` property, numeric block ids, and connected textures on items. Each
is a rule shape in `ctm.rs` and a branch in `Ctm::tile`.

## Testing a pack

`tools/test_resource_pack_parity.py` builds small packs (folder and zip, overlays, a filter, textures, languages,
sounds, fonts, atlases), and a Java oracle answers what vanilla's loaders read from them; Horizon's loaders must
give the same answers. Never commit third-party packs: build test packs in the harness or the test.
