//! Far-terrain quads in 16 bytes, for vertex pulling.
//!
//! A merged face of a far tile (`mesh.rs`: `emit_quad`) is four
//! `TerrainVertex` of 24 bytes each, 96 bytes, though all four share one
//! layer, light and colour and the rest follows from the face's rectangle.
//! Packed, it is one `FarQuad` of 16 bytes, six times smaller; a vertex
//! shader that pulls its corner from a storage buffer of them (index / 4 the
//! quad, index % 4 the corner) makes the same vertices again. The mesher's
//! output stays the source: `pack` converts a finished mesh, and `unpack` is
//! what the shader does, so the two are tested against each other.
//!
//! The packing follows from what far tiles hold. Horizontal coordinates are
//! whole blocks, and for tiles of level 4 and up whole chunks (16 blocks:
//! cells are 16 blocks wide and up). Heights are whole blocks, or a block
//! less 1/9 where a fluid surface lies below its cell's top. Faces wider
//! than `MAX_FACE` carry the UV scale `uv_scale` derives from their size. A
//! quad that does not fit (a fraction, a coordinate past 2048 chunks, four
//! vertices that are not a face) has no packed form, and `pack` of a mesh
//! with one returns none: that tile stays in the vertex format.
//!
//! Words, low bit first:
//! - 0: colour.
//! - 1: layer (16), light (16).
//! - 2: u0 (11), width - 1 (11), plane bits 0 to 9 (10).
//! - 3: v0 (12), height (12), plane bits 10 and 11 (2), face (3, the
//!   `Direction::ALL` index), chunks (1), height 0 lowered (1), height 1
//!   lowered (1).
//!
//! `u` and `v` are the face's two axes as `quad_vertices` takes them. For a
//! top or bottom face `u` is x, `v` is z and the plane is a height; for the
//! others `u` is the horizontal axis along the face (x, or z for west and
//! east), `v` is the height and the plane is the horizontal coordinate the
//! face stands at. A height is its block plus 2048 (so below zero fits).
#![cfg_attr(not(test), allow(dead_code))]
use super::{Rect, quad_vertices, uv_scale};
use crate::mesher::TerrainVertex;
use horizon_assets::model::Direction;

/// What a far-terrain face of this tile costs packed, and as vertices.
pub(super) const PACKED_BYTES: usize = std::mem::size_of::<FarQuad>();
pub(super) const VERTEX_BYTES: usize = 4 * std::mem::size_of::<TerrainVertex>();

/// Added to a block height so heights below zero fit.
const BIAS: i32 = 2048;
/// What a fluid's surface lies below its cell's top, in blocks (`mesh.rs`).
const LOWERED: f32 = 1.0 / 9.0;
/// Blocks in a chunk: the unit of a tile's coordinates when all are whole ones.
const CHUNK: f32 = 16.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct FarQuad {
    words: [u32; 4],
}

/// A height as its block plus `BIAS` and whether it lies 1/9 below that block.
fn height(y: f32) -> Option<(u32, bool)> {
    let block = y.ceil();
    let lowered = y != block;
    if lowered && y != block - LOWERED {
        return None;
    }
    if !(-(BIAS as f32)..BIAS as f32).contains(&block) {
        return None;
    }
    Some(((block as i32 + BIAS) as u32, lowered))
}

fn height_of(code: u32, lowered: bool) -> f32 {
    (code as i32 - BIAS) as f32 - if lowered { LOWERED } else { 0.0 }
}

impl FarQuad {
    /// The packed form of the four vertices one face of a far tile makes,
    /// when it has one that makes them again to the bit.
    pub(super) fn new(quad: &[TerrainVertex; 4]) -> Option<Self> {
        let first = quad[0];
        let paint = |v: &TerrainVertex| (v.layer, v.light, v.color);
        if quad.iter().any(|v| paint(v) != paint(&first)) {
            return None;
        }
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for v in quad {
            for axis in 0..3 {
                min[axis] = min[axis].min(v.pos[axis]);
                max[axis] = max[axis].max(v.pos[axis]);
            }
        }
        // The face's plane is the axis its vertices share; the winding says
        // which of the two directions facing along it.
        let (directions, plane, u, v) = match [0, 1, 2].map(|axis| min[axis] == max[axis]) {
            [false, true, false] => (
                [Direction::Up, Direction::Down],
                min[1],
                [min[0], max[0]],
                [min[2], max[2]],
            ),
            [false, false, true] => (
                [Direction::North, Direction::South],
                min[2],
                [min[0], max[0]],
                [min[1], max[1]],
            ),
            [true, false, false] => (
                [Direction::West, Direction::East],
                min[0],
                [min[2], max[2]],
                [min[1], max[1]],
            ),
            _ => return None,
        };
        directions.into_iter().find_map(|direction| {
            let packed = Self::pack(direction, plane, u, v, first)?;
            let again = packed.vertices();
            again
                .iter()
                .zip(quad)
                .all(|(a, b)| bytemuck::bytes_of(a) == bytemuck::bytes_of(b))
                .then_some(packed)
        })
    }

    fn pack(
        direction: Direction,
        plane: f32,
        u: [f32; 2],
        v: [f32; 2],
        paint: TerrainVertex,
    ) -> Option<Self> {
        let sides = !matches!(direction, Direction::Up | Direction::Down);
        // The horizontal coordinates: whole blocks, or whole chunks if all are.
        let horizontal = if sides {
            [plane, u[0], u[1], plane]
        } else {
            [u[0], u[1], v[0], v[1]]
        };
        if !horizontal.iter().all(|&x| x >= 0.0 && x.fract() == 0.0) {
            return None;
        }
        let chunks = horizontal.iter().all(|&x| x % CHUNK == 0.0);
        let unit = if chunks { CHUNK } else { 1.0 };
        let cell = |x: f32| (x / unit) as u32;
        let u0 = cell(u[0]);
        let width = cell(u[1]).checked_sub(u0)?.checked_sub(1)?;
        let (v0, extent, plane, lowered) = if sides {
            let ((low, low_lowered), (high, high_lowered)) = (height(v[0])?, height(v[1])?);
            (
                low,
                high.checked_sub(low)?,
                cell(plane),
                [low_lowered, high_lowered],
            )
        } else {
            let (code, plane_lowered) = height(plane)?;
            let v0 = cell(v[0]);
            (
                v0,
                cell(v[1]).checked_sub(v0)?,
                code,
                [plane_lowered, false],
            )
        };
        if u0 >= 1 << 11
            || width >= 1 << 11
            || v0 >= 1 << 12
            || extent >= 1 << 12
            || plane >= 1 << 12
        {
            return None;
        }
        let face = Direction::ALL.iter().position(|&d| d == direction)? as u32;
        Some(Self {
            words: [
                paint.color,
                u32::from(paint.layer) | u32::from(paint.light) << 16,
                u0 | width << 11 | (plane & 0x3ff) << 22,
                v0 | extent << 12
                    | (plane >> 10) << 24
                    | face << 26
                    | u32::from(chunks) << 29
                    | u32::from(lowered[0]) << 30
                    | u32::from(lowered[1]) << 31,
            ],
        })
    }

    /// The four vertices, as `emit_quad` writes them (what the vertex shader
    /// builds from the quad).
    pub(super) fn vertices(self) -> [TerrainVertex; 4] {
        let [color, paint, low, high] = self.words;
        let (u0, width) = (low & 0x7ff, ((low >> 11) & 0x7ff) + 1);
        let plane = (low >> 22) | ((high >> 24) & 3) << 10;
        let (v0, extent) = (high & 0xfff, (high >> 12) & 0xfff);
        let direction = Direction::ALL
            .get(((high >> 26) & 7) as usize)
            .copied()
            .unwrap_or(Direction::Up);
        let unit = if (high >> 29) & 1 == 1 { CHUNK } else { 1.0 };
        let lowered = [(high >> 30) & 1 == 1, (high >> 31) & 1 == 1];
        let across = |cell: u32| cell as f32 * unit;
        let (plane, v) = match direction {
            Direction::Up | Direction::Down => (
                height_of(plane, lowered[0]),
                [across(v0), across(v0 + extent)],
            ),
            _ => (
                across(plane),
                [
                    height_of(v0, lowered[0]),
                    height_of(v0 + extent, lowered[1]),
                ],
            ),
        };
        let rect = Rect {
            u0: across(u0),
            v0: v[0],
            u1: across(u0 + width),
            v1: v[1],
        };
        quad_vertices(
            direction,
            plane,
            rect,
            uv_scale(rect),
            paint as u16,
            color,
            (paint >> 16) as u16,
        )
    }
}

/// A tile's vertices (`SectionMesh::solid`) as packed quads, or none when
/// one of its faces has no packed form.
pub(super) fn pack(vertices: &[TerrainVertex]) -> Option<Vec<FarQuad>> {
    let (quads, rest) = vertices.as_chunks::<4>();
    if !rest.is_empty() {
        return None;
    }
    quads.iter().map(FarQuad::new).collect()
}

/// The vertices of packed quads, in order.
pub(super) fn unpack(quads: &[FarQuad]) -> Vec<TerrainVertex> {
    quads.iter().flat_map(|quad| quad.vertices()).collect()
}

#[cfg(test)]
mod tests {
    use super::super::{Material, Paint, mesh_columns};
    use super::*;
    use crate::fast_hash::FxHashSet;
    use crate::lod::{Column, Span, Spans, TileKey};
    use horizon_world::registry::BlockStateId;

    fn same(a: &[TerrainVertex], b: &[TerrainVertex]) -> bool {
        bytemuck::cast_slice::<_, u8>(a) == bytemuck::cast_slice::<_, u8>(b)
    }

    fn face(direction: Direction, plane: f32, rect: Rect) -> [TerrainVertex; 4] {
        quad_vertices(
            direction,
            plane,
            rect,
            uv_scale(rect),
            7,
            0xFFC0_80FF,
            0x30F0,
        )
    }

    /// Sand ground with sea over the low parts, like a coast: the water tops
    /// lie 1/9 below their cells, walls start at them, ground goes down to the
    /// bottom of the world.
    fn coast() -> Vec<Column> {
        let span = |min: i16, max: i16, state: u32, light: u16| Span {
            min,
            max,
            state: BlockStateId(state),
            biome: 0,
            light,
        };
        (0..256)
            .map(|i| {
                let (x, z) = (i % 16, i / 16);
                let top = 55 + ((x * 7 + z * 3) % 11) as i16;
                let mut spans = Spans::new();
                spans.push(span(-64, top, 1, 240 - ((x + z) % 3) as u16 * 16));
                if top < 62 {
                    spans.push(span(top, 62, 2, 240));
                }
                Column {
                    surface: None,
                    spans,
                }
            })
            .collect()
    }
    fn material(span: &Span, _: Direction, _: [i32; 2]) -> Material {
        let water = span.state.0 == 2;
        Material {
            faces: [
                Some(Paint {
                    layer: span.state.0 as u16,
                    light: span.light,
                    color: 0xFF00_0000 | span.light as u32 * 0x0101,
                    translucent: water,
                }),
                None,
            ],
            fluid: u8::from(water),
            opaque: !water,
        }
    }

    #[test]
    fn every_face_of_a_tile_packs_and_unpacks_to_the_bit() {
        let columns = coast();
        for level in [0, 1, 3, 4, 6, 9, 11] {
            let key = TileKey { x: 3, z: -2, level };
            let mesh = mesh_columns(key, &columns, &FxHashSet::default(), None, material);
            for vertices in [&mesh.solid, &mesh.translucent] {
                assert!(!vertices.is_empty(), "level {level}");
                let quads = pack(vertices).unwrap_or_else(|| panic!("level {level} has no form"));
                assert_eq!(quads.len() * 4, vertices.len());
                assert!(same(&unpack(&quads), vertices), "level {level}");
            }
        }
    }

    #[test]
    fn a_far_tile_takes_a_sixth_of_the_memory() {
        assert_eq!((PACKED_BYTES, VERTEX_BYTES), (16, 96));
        let columns = coast();
        let (mut packed, mut vertices) = (0, 0);
        for level in [0, 3, 6, 9] {
            let key = TileKey { x: 0, z: 0, level };
            let mesh = mesh_columns(key, &columns, &FxHashSet::default(), None, material);
            let quads = pack(&mesh.solid).unwrap().len() + pack(&mesh.translucent).unwrap().len();
            let tile = (quads * PACKED_BYTES, quads * VERTEX_BYTES);
            eprintln!(
                "level {level}: {quads} quads, {} bytes as vertices, {} packed",
                tile.1, tile.0
            );
            packed += tile.0;
            vertices += tile.1;
        }
        assert_eq!(packed * 6, vertices);
    }

    #[test]
    fn faces_in_every_direction_keep_their_corners_and_uvs() {
        let flat = Rect {
            u0: 16.0,
            v0: 32.0,
            u1: 96.0,
            v1: 48.0,
        };
        // A wall from below zero up to a fluid's lowered top.
        let wall = Rect {
            u0: 5.0,
            v0: -64.0,
            u1: 9.0,
            v1: 62.0 - 1.0 / 9.0,
        };
        for direction in Direction::ALL {
            let (plane, rect) = match direction {
                Direction::Up | Direction::Down => (62.0 - 1.0 / 9.0, flat),
                _ => (48.0, wall),
            };
            let vertices = face(direction, plane, rect);
            let quad = FarQuad::new(&vertices).unwrap_or_else(|| panic!("{direction:?}"));
            assert!(same(&quad.vertices(), &vertices), "{direction:?}");
        }
    }
    #[test]
    fn faces_without_a_packed_form_stay_vertices() {
        let rect = |u0, v0, u1, v1| Rect { u0, v0, u1, v1 };
        let up = |plane, rect| face(Direction::Up, plane, rect);
        // A fraction of a block, a quarter of a block of height, a coordinate
        // past the 2048 chunks the bits hold, a height past the bias.
        assert!(FarQuad::new(&up(64.0, rect(0.5, 0.0, 2.0, 2.0))).is_none());
        assert!(FarQuad::new(&up(64.25, rect(0.0, 0.0, 2.0, 2.0))).is_none());
        assert!(FarQuad::new(&up(64.0, rect(0.0, 0.0, 16.0 * 4096.0, 16.0))).is_none());
        assert!(FarQuad::new(&up(3000.0, rect(0.0, 0.0, 2.0, 2.0))).is_none());
        // Four vertices of two colours, or not a face at all.
        let mut mixed = up(64.0, rect(0.0, 0.0, 2.0, 2.0));
        mixed[2].color ^= 1;
        assert!(FarQuad::new(&mixed).is_none());
        let mut skew = up(64.0, rect(0.0, 0.0, 2.0, 2.0));
        skew[1].pos[1] += 1.0;
        assert!(FarQuad::new(&skew).is_none());
        let mut vertices = up(64.0, rect(0.0, 0.0, 2.0, 2.0)).to_vec();
        assert!(pack(&vertices).is_some());
        vertices.push(vertices[0]);
        assert!(pack(&vertices).is_none());
    }

    #[test]
    fn chunk_units_reach_the_far_side_of_the_biggest_packed_tile() {
        // A whole level-11 tile (32768 blocks, 2048 chunks) is one face.
        let whole = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 32768.0,
            v1: 32768.0,
        };
        let vertices = face(Direction::Up, 64.0, whole);
        let quad = FarQuad::new(&vertices).expect("a whole tile");
        assert!(same(&quad.vertices(), &vertices));
        // Its far wall stands at the last chunk edge.
        let wall = Rect {
            u0: 0.0,
            v0: -64.0,
            u1: 32768.0,
            v1: 100.0,
        };
        let vertices = face(Direction::East, 32768.0, wall);
        let quad = FarQuad::new(&vertices).expect("the far wall");
        assert!(same(&quad.vertices(), &vertices));
    }
}
