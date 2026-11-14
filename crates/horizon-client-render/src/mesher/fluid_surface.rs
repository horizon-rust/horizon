//! Continuous fluid surfaces, Horizon's "Smooth fluids" (off by default; the
//! vanilla cells are `fluid.rs`). The same faces as vanilla, culled the same
//! way (less the walls a ledge leaves with no height); what differs is where
//! their vertices sit and what they carry:
//!
//! - one height per corner of the block grid, computed from the four cells
//!   around it whichever cell asks, so neighbouring tops always meet;
//! - no step where a sheet pours over a ledge: a corner beside an open cell
//!   with the fluid below it comes down to the lower surface, which the
//!   falling column there lifts to the same height;
//! - corner light averaged over the open cells around it (smooth lighting);
//! - shading from the surface normal at each corner (the height field's
//!   gradient), and blended at the outer vertical edges of columns;
//! - texture coordinates anchored to the world, with the flowing sprite turned
//!   to the nearest axis of the flow, so a sheet flowing one way is one texture.
//!
//! - a top split along its flatter diagonal.
//!
//! See `docs/smooth-fluids.md`.
use super::{
    Info, Padded, SectionMesh, TerrainVertex,
    fluid::{EPSILON, FlowView, MAX_HEIGHT, occluded_by, occluded_by_self},
    lighting, pack_coords, pack_rgb, pi,
};
use horizon_core::{geometry::Direction, position::BlockPos};
use horizon_world::{fluid::FluidKind, fluid_flow::flow_at};

/// Lattice corners per row: x and z from -1 to 17, the corners of the
/// section's cells and one ring more for the normals.
const SPAN: usize = 19;
const UNSET: f32 = f32::NAN;
/// A corner with no cell of the fluid around it.
const NONE: f32 = f32::NEG_INFINITY;

/// The fluid sprite layers (`MeshTables`).
#[derive(Clone, Copy)]
pub(super) struct Sprites {
    pub(super) water_still: u16,
    pub(super) water_flow: u16,
    pub(super) water_overlay: u16,
    pub(super) lava_still: u16,
    pub(super) lava_flow: u16,
}
/// Corner heights of one section, per fluid family (water, lava) and layer,
/// filled as cells ask for them.
pub(super) struct Lattice {
    heights: [Option<Box<[f32]>>; 2],
}
impl Lattice {
    pub(super) fn new() -> Self {
        Self {
            heights: [None, None],
        }
    }
}

/// The cell being meshed and its fluid family.
struct Cells<'a> {
    pad: &'a Padded,
    info: &'a [Info],
    kind: FluidKind,
}
/// What a cell brings to the corners it touches.
#[derive(Clone, Copy, PartialEq)]
enum Sample {
    /// A block fluids treat as solid (`isSolid`): left out.
    Solid,
    /// An open cell: height 0.
    Open,
    /// The fluid with the same fluid above: the corner is full.
    Full,
    /// The fluid, its own height.
    Fluid(f32),
    /// An open cell over the fluid: the surface drops to the cell's floor.
    Drop,
}
impl Cells<'_> {
    fn at(&self, p: [i32; 3]) -> &Info {
        &self.info[self.pad.blocks[pi(p[0], p[1], p[2])] as usize]
    }
    fn same(&self, s: &Info) -> bool {
        s.fluid.kind().same_family(self.kind)
    }
    fn sample(&self, p: [i32; 3]) -> Sample {
        let s = self.at(p);
        if self.same(s) {
            if self.same(self.at([p[0], p[1] + 1, p[2]])) {
                Sample::Full
            } else {
                Sample::Fluid(s.fluid.own_height())
            }
        } else if s.legacy_solid {
            Sample::Solid
        } else if self.same(self.at([p[0], p[1] - 1, p[2]])) {
            Sample::Drop
        } else {
            Sample::Open
        }
    }
    /// The height of lattice corner `(cx, cz)` (the north-west corner of cell
    /// `(cx, cz)`) in layer `y`: vanilla's `calculateAverageHeight` weights
    /// (cells of 0.8 and more count ten times) over all four cells, 1 beside
    /// the fluid above, 0 beside a drop, or NONE without the fluid.
    fn corner(&self, y: i32, cx: i32, cz: i32) -> f32 {
        let samples =
            [[-1, -1], [0, -1], [-1, 0], [0, 0]].map(|[dx, dz]| self.sample([cx + dx, y, cz + dz]));
        if samples.contains(&Sample::Full) {
            return 1.0;
        }
        if !samples.iter().any(|s| matches!(s, Sample::Fluid(_))) {
            return NONE;
        }
        if samples.contains(&Sample::Drop) {
            return 0.0;
        }
        let (mut sum, mut weight) = (0.0f32, 0.0f32);
        for s in samples {
            let (h, w) = match s {
                Sample::Fluid(h) if h >= 0.8 => (h, 10.0),
                Sample::Fluid(h) => (h, 1.0),
                Sample::Open => (0.0, 1.0),
                Sample::Solid | Sample::Full | Sample::Drop => continue,
            };
            sum += h * w;
            weight += w;
        }
        sum / weight
    }
    /// The average of `max(cell, above)` light over the open cells around a
    /// corner (the cell itself when they are all solid).
    fn corner_light(&self, y: i32, cx: i32, cz: i32, own: [i32; 3]) -> u16 {
        let light = |p: [i32; 3]| {
            let above = [p[0], p[1] + 1, p[2]];
            lighting::max_coords(
                lighting::coords(self.pad, self.at(p), p),
                lighting::coords(self.pad, self.at(above), above),
            )
        };
        let (mut low, mut high, mut n) = (0, 0, 0);
        for [dx, dz] in [[-1, -1], [0, -1], [-1, 0], [0, 0]] {
            let p = [cx + dx, y, cz + dz];
            if !self.at(p).solid {
                let c = light(p);
                low += c & 0xFF;
                high += c >> 16 & 0xFF;
                n += 1;
            }
        }
        if n == 0 {
            return pack_coords(light(own));
        }
        pack_coords((low / n) | ((high / n) << 16))
    }
}
impl Lattice {
    /// Corner `(cx, cz)` of layer `y` (0..16), cx and cz in -1..=17.
    fn get(&mut self, cells: &Cells, y: i32, cx: i32, cz: i32) -> f32 {
        let family = usize::from(matches!(
            cells.kind,
            FluidKind::Lava | FluidKind::FlowingLava
        ));
        let grid = self.heights[family].get_or_insert_with(|| vec![UNSET; 16 * SPAN * SPAN].into());
        let i = (y as usize * SPAN + (cz + 1) as usize) * SPAN + (cx + 1) as usize;
        if grid[i].is_nan() {
            grid[i] = cells.corner(y, cx, cz);
        }
        grid[i]
    }
    /// The surface normal at corner `(cx, cz)` from the heights around it
    /// (central differences, one-sided where a neighbour has no fluid).
    fn normal(&mut self, cells: &Cells, y: i32, cx: i32, cz: i32) -> [f32; 3] {
        let h = self.get(cells, y, cx, cz);
        let mut slope = |a: [i32; 2], b: [i32; 2]| {
            let ha = self.get(cells, y, a[0], a[1]);
            let hb = self.get(cells, y, b[0], b[1]);
            match (ha > NONE, hb > NONE) {
                (true, true) => (hb - ha) / 2.0,
                (true, false) => h - ha,
                (false, true) => hb - h,
                (false, false) => 0.0,
            }
        };
        let dx = slope([cx - 1, cz], [cx + 1, cz]);
        let dz = slope([cx, cz - 1], [cx, cz + 1]);
        let n = [-dx, 1.0, -dz];
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        n.map(|c| c / length)
    }
}
/// Directional shade for a normal: the cardinal shades weighted by the squared
/// components, which gives vanilla's values on axis-aligned faces.
fn shade(nether: bool, [x, y, z]: [f32; 3]) -> f32 {
    use horizon_assets::model::Direction as D;
    let vertical = lighting::cardinal(nether, if y >= 0.0 { D::Up } else { D::Down });
    x * x * lighting::cardinal(nether, D::West)
        + y * y * vertical
        + z * z * lighting::cardinal(nether, D::North)
}
/// Shifts UVs that span at most one sprite by whole sprites into `[0, 1]`:
/// world-anchored coordinates tile with the repeating sampler.
fn into_sprite<const N: usize>(values: [f32; N]) -> [f32; N] {
    let low = values.iter().copied().fold(f32::INFINITY, f32::min);
    let shift = (low + 1e-4).floor();
    values.map(|v| v - shift)
}
/// Emits one fluid cell at section-relative `[x, y, z]`, continuous with its
/// neighbours (the culling of `fluid::tesselate`).
#[allow(clippy::too_many_arguments)]
pub(super) fn tesselate(
    info: &[Info],
    sprites: Sprites,
    nether: bool,
    pad: &Padded,
    water_color: u32,
    out: &mut SectionMesh,
    lattice: &mut Lattice,
    [x, y, z]: [i32; 3],
) {
    use horizon_assets::model::Direction as D;
    let me = &info[pad.blocks[pi(x, y, z)] as usize];
    let kind = me.fluid.kind();
    let cells = Cells { pad, info, kind };
    let neighbor = |d: Direction| {
        let [dx, dy, dz] = d.normal();
        cells.at([x + dx, y + dy, z + dz])
    };
    let (down, up) = (neighbor(Direction::Down), neighbor(Direction::Up));
    let should_render = |d: Direction| !cells.same(neighbor(d)) && !occluded_by_self(me, d);
    let render_up = !cells.same(up);
    let render_down =
        should_render(Direction::Down) && !occluded_by(Direction::Down, MAX_HEIGHT, down);
    const SIDES: [Direction; 4] = [
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ];
    let sides = SIDES.map(should_render);
    if !render_up && !render_down && !sides.iter().any(|&s| s) {
        return;
    }
    let water = matches!(kind, FluidKind::Water | FluidKind::FlowingWater);
    let tint = if water {
        water_color & 0xFF_FFFF
    } else {
        0xFF_FFFF
    };
    let emission = if water { 0 } else { 15 };
    let color = |by: f32| {
        let rgb = [16, 8, 0].map(|shift| lighting::scale((tint >> shift & 255) as i32, by) as u32);
        let color = crate::mesher::emitting(pack_rgb(rgb), emission);
        if water {
            crate::mesher::with_sea(color, water_color >> 24)
        } else {
            color
        }
    };
    // Corners as lattice points: NW (x, z), NE (x+1, z), SW (x, z+1), SE (x+1, z+1).
    let (mut nw, mut ne, mut sw, mut se) = (
        lattice.get(&cells, y, x, z),
        lattice.get(&cells, y, x + 1, z),
        lattice.get(&cells, y, x, z + 1),
        lattice.get(&cells, y, x + 1, z + 1),
    );
    let corner_light = |cx: i32, cz: i32| cells.corner_light(y, cx, cz, [x, y, z]);
    let lights = [
        corner_light(x, z),
        corner_light(x + 1, z),
        corner_light(x, z + 1),
        corner_light(x + 1, z + 1),
    ];
    let [l_nw, l_ne, l_sw, l_se] = lights;
    let (still, flow, overlay) = if water {
        (
            sprites.water_still,
            sprites.water_flow,
            Some(sprites.water_overlay),
        )
    } else {
        (sprites.lava_still, sprites.lava_flow, None)
    };
    let target = if water {
        &mut out.translucent
    } else {
        &mut out.solid
    };
    type Corner = ([f32; 3], [f32; 2], u32, u16);
    let mut quad = |v: [Corner; 4], layer: u16, back: bool| {
        let vertex =
            |(pos, uv, color, light): Corner| TerrainVertex::new(pos, uv, layer, color, light);
        target.extend(v.map(vertex));
        if back {
            target.extend([v[0], v[3], v[2], v[1]].map(vertex));
        }
    };
    let (fx, fy, fz) = (x as f32, y as f32, z as f32);
    let bottom = if render_down { EPSILON } else { 0.0 };
    if render_up && !occluded_by(Direction::Up, nw.min(sw).min(se.min(ne)), up) {
        // As vanilla, the sides then meet the lowered top.
        nw -= EPSILON;
        sw -= EPSILON;
        se -= EPSILON;
        ne -= EPSILON;
        let flow_vector = flow_at(
            &mut FlowView { pad, info },
            me.fluid,
            BlockPos::new(x, y, z),
        )
        .unwrap_or_else(|e| match e {});
        // World-anchored UVs at NW, SW, SE, NE: the still sprite once per
        // block, the flowing one at vanilla's half scale, turned to the axis
        // nearest the flow (vanilla's angle for that axis).
        let corners = [
            [fx, fz],
            [fx, fz + 1.0],
            [fx + 1.0, fz + 1.0],
            [fx + 1.0, fz],
        ];
        let (layer, uv) = if flow_vector.x == 0.0 && flow_vector.z == 0.0 {
            (still, corners.map(|[cx, cz]| [cx - fx, cz - fz]))
        } else {
            let (c, s) = if flow_vector.x.abs() >= flow_vector.z.abs() {
                (0.0, if flow_vector.x > 0.0 { -1.0 } else { 1.0 })
            } else {
                (if flow_vector.z > 0.0 { 1.0 } else { -1.0 }, 0.0)
            };
            let u = into_sprite(corners.map(|[cx, cz]| 0.5 * (c * cx + s * cz)));
            let v = into_sprite(corners.map(|[cx, cz]| 0.5 * (c * cz - s * cx)));
            (flow, [0, 1, 2, 3].map(|i| [u[i], v[i]]))
        };
        let shade_at = |lattice: &mut Lattice, cx: i32, cz: i32| {
            color(shade(nether, lattice.normal(&cells, y, cx, cz)))
        };
        let colors = [
            shade_at(lattice, x, z),
            shade_at(lattice, x, z + 1),
            shade_at(lattice, x + 1, z + 1),
            shade_at(lattice, x + 1, z),
        ];
        let backward = (-1..=1).any(|ox| {
            (-1..=1).any(|oz| {
                let s = cells.at([x + ox, y + 1, z + oz]);
                !cells.same(s) && !s.solid
            })
        });
        let top = |i: usize, h: f32, light: u16| -> Corner {
            let [cx, cz] = corners[i];
            ([cx, fy + h, cz], uv[i], colors[i], light)
        };
        let v = [
            top(0, nw, l_nw),
            top(1, sw, l_sw),
            top(2, se, l_se),
            top(3, ne, l_ne),
        ];
        // Split along the flatter diagonal: a single raised corner makes one
        // tilted triangle instead of a fold across the block.
        let v = if (nw - se).abs() <= (sw - ne).abs() {
            v
        } else {
            [v[1], v[2], v[3], v[0]]
        };
        quad(v, layer, backward);
    }
    if render_down {
        let y0 = fy + bottom;
        let c = color(lighting::cardinal(nether, D::Down));
        let light = {
            let p = [x, y - 1, z];
            let above = [x, y, z];
            pack_coords(lighting::max_coords(
                lighting::coords(pad, cells.at(p), p),
                lighting::coords(pad, cells.at(above), above),
            ))
        };
        quad(
            [
                ([fx, y0, fz], [0.0, 0.0], c, light),
                ([fx + 1.0, y0, fz], [1.0, 0.0], c, light),
                ([fx + 1.0, y0, fz + 1.0], [1.0, 1.0], c, light),
                ([fx, y0, fz + 1.0], [0.0, 1.0], c, light),
            ],
            still,
            false,
        );
    }
    // A side whose top came down to its floor (beside a drop) has no area.
    let drawn: [bool; 4] = std::array::from_fn(|i| {
        let d = SIDES[i];
        let (h0, h1) = match d {
            Direction::North => (nw, ne),
            Direction::South => (se, sw),
            Direction::West => (sw, nw),
            _ => (ne, se),
        };
        let h = h0.max(h1);
        sides[i] && h > bottom + EPSILON && !occluded_by(d, h, neighbor(d))
    });
    let up_shade = lighting::cardinal(nether, D::Up);
    let face_shade = |d: Direction| match d {
        Direction::North | Direction::South => lighting::cardinal(nether, D::North),
        _ => lighting::cardinal(nether, D::West),
    };
    for (i, d) in SIDES.into_iter().enumerate() {
        if !drawn[i] {
            continue;
        }
        // Each end: its corner, the side it turns to (index into SIDES), and
        // the coordinate along the face, increasing from end 0 to end 1.
        let (h0, h1, [x0, z0], [x1, z1], turns, light, along) = match d {
            Direction::North => (
                nw,
                ne,
                [fx, fz + EPSILON],
                [fx + 1.0, fz + EPSILON],
                [2, 3],
                [l_nw, l_ne],
                [fx, fx + 1.0],
            ),
            Direction::South => (
                se,
                sw,
                [fx + 1.0, fz + 1.0 - EPSILON],
                [fx, fz + 1.0 - EPSILON],
                [3, 2],
                [l_se, l_sw],
                [-fx - 1.0, -fx],
            ),
            Direction::West => (
                sw,
                nw,
                [fx + EPSILON, fz + 1.0],
                [fx + EPSILON, fz],
                [1, 0],
                [l_sw, l_nw],
                [-fz - 1.0, -fz],
            ),
            _ => (
                ne,
                se,
                [fx + 1.0 - EPSILON, fz],
                [fx + 1.0 - EPSILON, fz + 1.0],
                [0, 1],
                [l_ne, l_se],
                [fz, fz + 1.0],
            ),
        };
        let face_state = neighbor(d);
        let (layer, is_overlay) = match overlay {
            Some(layer) if face_state.overlay => (layer, true),
            _ => (flow, false),
        };
        // An outer vertical edge (the side it turns to is drawn too) takes
        // the normal halfway between the two faces.
        let edge_color = |end: usize| {
            let own = face_shade(d);
            let by = if drawn[turns[end]] {
                (own + face_shade(SIDES[turns[end]])) / 2.0
            } else {
                own
            };
            color(up_shade * by)
        };
        let (c0, c1) = (edge_color(0), edge_color(1));
        // The sprite at vanilla's half scale, anchored to the world: u along
        // the face, v downwards from the floor, which lands on 0.5 or 1.
        let u = into_sprite(along.map(|a| 0.5 * a));
        let floor = -0.5 * fy;
        let vb = floor - (floor.ceil() - 1.0);
        let (v0, v1) = (vb - 0.5 * h0, vb - 0.5 * h1);
        quad(
            [
                ([x0, fy + h0, z0], [u[0], v0], c0, light[0]),
                ([x1, fy + h1, z1], [u[1], v1], c1, light[1]),
                ([x1, fy + bottom, z1], [u[1], vb], c1, light[1]),
                ([x0, fy + bottom, z0], [u[0], vb], c0, light[0]),
            ],
            layer,
            !is_overlay,
        );
    }
}

#[cfg(test)]
mod tests;
