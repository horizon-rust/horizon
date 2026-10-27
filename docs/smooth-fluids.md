# Smooth fluids

Graphics: Advanced > Quality > Smooth Fluids, saved as `smoothFluids` in `horizon-graphics.txt`. **Off by
default**: off, water and lava are meshed by `mesher/fluid.rs`, the port of 26.3's `FluidRenderer.tesselate`
that `tools/test_section_mesh_parity.py` compares quad for quad with the real client. On, the same cells go
through `mesher/fluid_surface.rs` instead, and the surface of a lake, a lava sea or a cascade reads as one
piece. Changing it rebuilds every loaded section, like vanilla's smooth-lighting option.

## What vanilla does, and where it breaks

`FluidRenderer` draws each fluid cell on its own:

- **Corner heights.** `calculateAverageHeight` averages the cell with its two edge neighbours and, only if
  one of those is a fluid, the diagonal one (cells of 0.8 and more weigh ten times, open cells count as 0,
  `isSolid` blocks not at all). Two cells that touch only at a corner, or that see a different set of cells
  around the corner, compute different heights for the same point: pinches and cracks.
- **Steps.** A sheet that pours off a ledge keeps its own height at its edge, then shows a wall down to its
  floor beside the open cell; the lower sheet starts higher (the falling column there lifts it to the full
  block). That wall is the "step" on lava and water staircases in caves.
- **Flat light.** One light value per cell, `max(cell, above)`: under a cave ceiling the surface is a
  patchwork of block-sized squares.
- **Flat shading.** Every top uses the up shade (1.0) whatever its slope, and every side its cardinal shade
  (0.8 or 0.6): slopes have no relief, and columns have hard corners.
- **Texture per cell.** The flowing sprite is turned by each cell's own flow angle (`Mth.atan2` of
  `FlowingFluid.getFlow`) and centred on the cell: a sheet flowing one way is a grid of tiles at slightly
  different angles. Sides map the sprite from the top of each cell, so a cascade repeats it every block.

Faces between two cells of the same fluid are never drawn in vanilla (`shouldRenderFace`), so there is no
"inner face" to remove: what looks like one inside a sheet is a step wall or a crack.

## What Smooth fluids changes

The faces and their culling are vanilla's, so nothing new is drawn (only the zero-height walls a ledge
leaves are dropped, so slightly fewer quads). The vertices differ:

1. **Shared corners.** A lattice point's height is computed once from the four cells around it, whichever
   cell asks, with vanilla's weights: 1 if one of them has the fluid above, the weighted average of the
   others otherwise. Heights are cached per section and layer (`Lattice`), so a corner costs one
   evaluation.
2. **No step at a ledge.** An open cell with the same fluid below it (the surface goes on lower down) brings
   the corner to its floor, height 0. The lower sheet's corner at that point is 1 (the falling column under
   the sheet's edge has fluid above), the same world height: the upper sheet slopes down into the lower one,
   and the zero-height wall is not drawn.
3. **Smooth light.** Each corner averages `max(cell, above)` over the non-opaque cells around it, as smooth
   lighting does for blocks; the bottom face keeps vanilla's flat light.
4. **Normals.** Each top corner takes the normal of the height field there (central differences of the
   corner heights, one-sided at the edge of the fluid), and its shade is the cardinal shades weighted by the
   squared normal components: exactly 1.0 on flat water, 0.8 or 0.6 on axis-aligned sides, in between on
   slopes. Side faces keep their cardinal shade, blended halfway at an outer vertical edge (both faces of a
   column corner drawn), so columns and waterfalls round their corners.
5. **World-anchored textures.** The still sprite already tiles once per block. The flowing sprite keeps
   vanilla's half scale, but its coordinates come from world positions, turned by the quarter turn nearest
   the flow (vanilla's angle for that axis): cells flowing the same way draw one continuous texture. Sides
   map the sprite downwards from each cell's floor in world space, so a cascade scrolls in one piece. UVs
   stay in `[0, 1]` per quad and rely on the terrain sampler repeating.
6. **Triangulation.** A top is split along its flatter diagonal, so a single raised corner (next to a
   waterfall) bends one triangle instead of folding the whole cell.

It runs on the meshing workers like the rest of the section; nothing is screen-specific, so it works the same
on a flat screen, with HDR, and in VR (both eyes draw the same section meshes). Parallax and LabPBR packs see
ordinary terrain quads; with world-anchored UVs the tangent frame of a flowing sheet is the same from one cell
to the next instead of turning with each cell's angle.

## Cost

`examples/fluid_mesh_bench.rs` meshes, on one thread, every section that holds a fluid face around a point of
a saved world, in both modes, alternating rounds (`--profile fast`, best of 5).

Measured on 2026-10-27 on "Grottes son" (a cave world with lava seas, lava staircases and waterfalls) around
chunk (4, 3), 16-thread desktop, the game closed, `--profile fast`, best of 5 to 9 rounds:

| Area | Sections with fluid | Vanilla | Smooth fluids | Quads (vanilla → smooth) |
|---|---|---|---|---|
| radius 6 (169 columns) | 428 of 2 904 | 150–162 ms, 352–379 µs per section | 164–181 ms, 382–424 µs per section (+9 to +12%) | 331 553 → 330 792 (fluid 65 290 → 64 529) |
| radius 10 (441 columns) | 1 137 of 8 664 | 402 ms, 354 µs per section | 439 ms, 386 µs per section (+9%) | 845 332 → 843 010 (fluid 169 677 → 167 355) |

So about 30–45 µs more per section that holds fluid, about 0.2 µs per fluid quad, and nothing for the others
(85% of the sections here); over all sections it is about +1.5% of meshing CPU. Smooth fluids draws slightly
*fewer* quads, about 1.2% fewer fluid quads, because the walls at ledges are now zero-height and left out.
The GPU and the upload cost nothing more. The CPU cost lands on the meshing workers, not the render thread:
in VR (render thread around 7 ms per frame) it delays a section's rebuild by a few tens of microseconds and
does not lengthen the frame.

A/B against `main` with the option off, same world state, same bench (`fluid_mesh_bench` built on both):
checksums `a7c5f849eed12cf8` (radius 6) and `f4071be67e416130` (radius 10) on both, byte-identical meshes.

## Limits

- **Still and flowing sprites still meet with a seam.** Where a sheet stops flowing the sprite changes
  (vanilla's choice of `water_still` when the flow is zero); blending two sprites needs the shader.
- **Flow is quantized to the four axes.** Diagonal flows pick the X axis on ties; where two neighbouring cells
  flow along different axes the texture still turns (much rarer than vanilla's per-cell angles).
- **Biome tint stays per cell**, after vanilla's biome blend (radius 2 by default), so it is already smooth.
- **The bottom face is vanilla's** (flat light, still sprite).
- **Ledges over a lower lake that the falling column does not lift** (an open cell over water whose corner
  is not raised to 1, e.g. a transient state) bring the upper corner to its floor anyway: a short gap can
  show the block under the ledge.
- **No waves, reflections or refraction**: this is meshing only; shader packs do the rest.
- Not compared with a vanilla oracle (it is not vanilla). The vanilla path is unchanged and still gated by
  `tools/test_section_mesh_parity.py`; the A/B checksum below shows the option off meshes the same bytes as
  `main`.

## Prior work

- **Sodium** (CaffeineMC; [repository](https://github.com/CaffeineMC/sodium), now under the Polyform Shield
  1.0.0 licence, LGPL-3.0 before): its fluid renderer applies its smooth light pipeline to fluids and blends
  biome colours per vertex; recent releases list fixes to "fluid shaping" in an "improved" and a regular
  mode ([0.8.12 notes](https://modrinth.com/mod/sodium/version/XSQwpkzX)). It keeps vanilla's per-cell
  corner rule and per-cell texture rotation. No Sodium code is used here; the licence would not allow it.
- **NoCubes** (Cadiboo, [repository](https://github.com/Cadiboo/NoCubes), LGPL-3.0): "bonus features" that
  work without its smooth terrain: smooth fluid lighting, smooth biome colour transitions and "Natural
  Fluid Textures", which changes how textures flow over the fluid mesh
  ([Modrinth](https://modrinth.com/mod/nocubes)). Ideas only, no code.
- **SmoothWater** (1.10/1.11, deprecated, succeeded by MAGE): smoother water surfaces on old versions.
- **Continuity** is connected textures for blocks and glass (Horizon's `ctm.rs`), not fluids.
- **Shader packs** (Iris/OptiFine: BSL, Complementary...) draw waves, normals and reflections on the water
  surface in the shader, over vanilla's mesh: they hide the texture grid but not the steps or cracks.

Nothing in this repository did it before (`docs/`, branches, mods checked on 2026-10-27).
