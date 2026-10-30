//! What a section's blocks are to the rays (gfx/voxels.rs, docs/raytracing.md): nothing, a solid
//! block, or a cutout one (leaves) that lets part of the light through. The mesher works it out
//! with the section's mesh, so a changed block reaches the rays when its section is meshed again.

/// Light passes.
pub const EMPTY: u8 = 0;
/// A full opaque block, or a block with a full face (slabs, stairs): it stops the light.
pub const SOLID: u8 = 1;
/// Leaves: the light passes in part.
pub const CUTOUT: u8 = 2;

/// A 16x16x16 section's blocks, two bits each.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Occupancy {
    #[default]
    Empty,
    Solid,
    /// Index `(z * 16 + y) * 16 + x`, four blocks a byte, the lowest bits first.
    Mixed(Box<[u8; 1024]>),
}

fn index(x: usize, y: usize, z: usize) -> usize {
    (z * 16 + y) * 16 + x
}

impl Occupancy {
    /// The section with block `(x, y, z)` (each 0..16) of kind `kind(x, y, z)`.
    pub fn from_fn(mut kind: impl FnMut(usize, usize, usize) -> u8) -> Self {
        let mut bits = Box::new([0u8; 1024]);
        let (mut any, mut all_solid) = (false, true);
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    let k = kind(x, y, z) & 3;
                    any |= k != EMPTY;
                    all_solid &= k == SOLID;
                    let i = index(x, y, z);
                    bits[i / 4] |= k << ((i % 4) * 2);
                }
            }
        }
        match (any, all_solid) {
            (false, _) => Self::Empty,
            (true, true) => Self::Solid,
            _ => Self::Mixed(bits),
        }
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> u8 {
        match self {
            Self::Empty => EMPTY,
            Self::Solid => SOLID,
            Self::Mixed(bits) => {
                let i = index(x, y, z);
                (bits[i / 4] >> ((i % 4) * 2)) & 3
            }
        }
    }

    /// What each 8x8x8 eighth of the section holds, index `(z * 2 + y) * 2 + x` of the eighth:
    /// bit 0 any block, bit 1 cutout ones, so the rays cross empty ones in one step and read the
    /// cutout mask only where there are leaves (gfx/voxels.rs).
    pub fn bricks(&self) -> [u8; 8] {
        match self {
            Self::Empty => [0; 8],
            Self::Solid => [1; 8],
            Self::Mixed(_) => {
                let mut out = [0u8; 8];
                for z in 0..16 {
                    for y in 0..16 {
                        for x in 0..16 {
                            let kind = self.get(x, y, z);
                            if kind != EMPTY {
                                out[((z / 8) * 2 + y / 8) * 2 + x / 8] |=
                                    1 | if kind == CUTOUT { 2 } else { 0 };
                            }
                        }
                    }
                }
                out
            }
        }
    }

    /// The section's eight bricks as bit masks (gfx/voxels.rs): for the solid blocks and for the
    /// cutout ones, 512 bits a brick in four `Rgba32Uint` texels. Word `k` of a brick holds its
    /// local rows `z = k / 2`, `y = (k % 2) * 4..+4`, bit `(y % 4) * 8 + x`. Laid out as the
    /// texture's 8x2x2 texels (two bricks of four texels a row), x fastest.
    pub fn brick_masks(&self) -> [[u32; 128]; 2] {
        let mut out = [[0u32; 128]; 2];
        if *self == Self::Empty {
            return out;
        }
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    let kind = self.get(x, y, z);
                    if kind == EMPTY {
                        continue;
                    }
                    let (b, l) = ([x / 8, y / 8, z / 8], [x % 8, y % 8, z % 8]);
                    let k = l[2] * 2 + l[1] / 4;
                    let texel = (b[2] * 2 + b[1]) * 8 + b[0] * 4 + k / 4;
                    let word = texel * 4 + k % 4;
                    let mask = usize::from(kind == CUTOUT);
                    out[mask][word] |= 1 << ((l[1] % 4) * 8 + l[0]);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_sections_take_no_room_and_mixed_ones_round_trip() {
        assert_eq!(Occupancy::from_fn(|_, _, _| EMPTY), Occupancy::Empty);
        assert_eq!(Occupancy::from_fn(|_, _, _| SOLID), Occupancy::Solid);
        let kind = |x: usize, y: usize, z: usize| ((x + 2 * y + 3 * z) % 3) as u8;
        let o = Occupancy::from_fn(kind);
        assert_eq!(o.bricks(), [3; 8]);
        let corner = Occupancy::from_fn(|x, y, z| u8::from(x == 15 && y < 8 && z == 0));
        assert_eq!(corner.bricks(), [0, 1, 0, 0, 0, 0, 0, 0]);
        // Block (15, 5, 0): brick x 1, local (7, 5, 0): word 1 (texel 4, its y), bit 15.
        let one = Occupancy::from_fn(|x, y, z| u8::from((x, y, z) == (15, 5, 0)));
        let masks = one.brick_masks();
        assert_eq!(masks[0][4 * 4 + 1], 1 << 15);
        assert_eq!(masks[0].iter().map(|w| w.count_ones()).sum::<u32>(), 1);
        assert_eq!(masks[1], [0; 128]);
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    assert_eq!(o.get(x, y, z), kind(x, y, z));
                }
            }
        }
    }
}

/// A section's blocks as RT lighting's rays see them past solid and cutout (gfx/voxels.rs): each
/// block's map colour (`blocks::MAP_COLORS` id, 0 none, six bits) for the light bouncing off it,
/// and its shape (`shape_id`, 0 the whole block), `shape << 6 | colour`. Run-length encoded
/// (runs of a value along x, then y, then z), so the many resident sections take little room
/// until they come into the grid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Albedo(Vec<(u16, u8)>);

impl Albedo {
    /// The values `f(x, y, z)` gives (each 0..16).
    pub fn from_fn(mut f: impl FnMut(usize, usize, usize) -> u16) -> Self {
        let mut runs: Vec<(u16, u8)> = Vec::new();
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    let v = f(x, y, z);
                    match runs.last_mut() {
                        Some((last, n)) if *last == v && *n < 255 => *n += 1,
                        _ => runs.push((v, 1)),
                    }
                }
            }
        }
        if runs.iter().all(|(v, _)| *v == 0) {
            runs.clear();
        }
        Self(runs)
    }

    /// Whether no block has a colour or a shape.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The 4096 blocks as the grid's `R16Uint` texels (little-endian), x fastest, then y, then z.
    pub fn decode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8192);
        for &(v, n) in &self.0 {
            for _ in 0..n {
                out.extend(v.to_le_bytes());
            }
        }
        out.resize(8192, 0);
        out
    }
}

/// A block's shape to RT lighting's rays: 8x8x8 cells, bit `(z * 8 + y) * 8 + x` of the sixteen
/// words, and whether light passes its cells in part (leaves, plants).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shape {
    pub cells: [u32; 16],
    pub cutout: bool,
}

/// At most this many shapes (ids 1..): past them a block is taken whole, or as nothing when
/// light passes it in part.
pub const SHAPES: usize = 1024;

/// The shapes met so far, by id (0 the whole block), shared by every mesher and read by the
/// renderer as it grows.
struct ShapeTable {
    ids: std::collections::HashMap<Shape, u16>,
    shapes: Vec<Shape>,
}

fn shape_table() -> &'static std::sync::Mutex<ShapeTable> {
    static TABLE: std::sync::OnceLock<std::sync::Mutex<ShapeTable>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::sync::Mutex::new(ShapeTable {
            ids: Default::default(),
            shapes: vec![Shape {
                cells: [u32::MAX; 16],
                cutout: false,
            }],
        })
    })
}

/// `shape`'s id; `None` when there are already `SHAPES`.
pub fn shape_id(shape: Shape) -> Option<u16> {
    let mut table = shape_table().lock().expect("the shape table");
    if let Some(&id) = table.ids.get(&shape) {
        return Some(id);
    }
    if table.shapes.len() >= SHAPES {
        return None;
    }
    let id = table.shapes.len() as u16;
    table.shapes.push(shape);
    table.ids.insert(shape, id);
    Some(id)
}

/// The shapes by id, for the GPU (gfx/voxels.rs): the sixteen words of cells, then 1 when
/// cutout and three words of padding.
pub fn shapes() -> Vec<[u32; 20]> {
    let table = shape_table().lock().expect("the shape table");
    table
        .shapes
        .iter()
        .map(|s| {
            let mut out = [0u32; 20];
            out[..16].copy_from_slice(&s.cells);
            out[16] = u32::from(s.cutout);
            out
        })
        .collect()
}

/// Sets cell `(x, y, z)` (each 0..8) in `cells`.
pub fn set_cell(cells: &mut [u32; 16], x: usize, y: usize, z: usize) {
    let bit = (z * 8 + y) * 8 + x;
    cells[bit / 32] |= 1 << (bit % 32);
}

/// The cells `boxes` (block space, min xyz then max xyz) fill: those a quarter of whose volume a
/// box covers, for blocks drawn without a model's faces (chests, signs).
pub fn cells_of(boxes: &[[f64; 6]]) -> [u32; 16] {
    let mut cells = [0u32; 16];
    for z in 0..8 {
        for y in 0..8 {
            for x in 0..8 {
                let low = [x as f64 / 8.0, y as f64 / 8.0, z as f64 / 8.0];
                let covered = boxes.iter().any(|b| {
                    (0..3)
                        .map(|a| ((b[a + 3]).min(low[a] + 0.125) - b[a].max(low[a])).max(0.0) * 8.0)
                        .product::<f64>()
                        >= 0.25 - 1e-9
                });
                if covered {
                    set_cell(&mut cells, x, y, z);
                }
            }
        }
    }
    cells
}

#[cfg(test)]
mod albedo_tests {
    use super::Albedo;

    #[test]
    fn colours_survive_their_runs() {
        let f = |x: usize, y: usize, z: usize| {
            if y < 4 {
                1
            } else if x == z {
                0x0d07
            } else {
                0
            }
        };
        let a = Albedo::from_fn(f);
        let decoded = a.decode();
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    let i = ((z * 16 + y) * 16 + x) * 2;
                    let v = u16::from(decoded[i]) | u16::from(decoded[i + 1]) << 8;
                    assert_eq!(v, f(x, y, z));
                }
            }
        }
        // Rows of the diagonal take three runs each, the solid quarter one a row at most.
        assert!(a.0.len() < 700);
        assert!(Albedo::from_fn(|_, _, _| 0).is_empty());
    }

    #[test]
    fn shapes_fill_the_cells_their_boxes_cover() {
        use super::cells_of;
        let count = |c: [u32; 16]| c.iter().map(|w| w.count_ones()).sum::<u32>();
        assert_eq!(cells_of(&[[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]), [u32::MAX; 16]);
        // A bottom slab: the lower four layers.
        assert_eq!(count(cells_of(&[[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]])), 256);
        // A fence post (6..10 sixteenths): two cells across.
        assert_eq!(
            count(cells_of(&[[0.375, 0.0, 0.375, 0.625, 1.0, 0.625]])),
            32
        );
        // An iron bar (7..9 sixteenths) fills a cell each side of its middle.
        assert_eq!(
            count(cells_of(&[[0.4375, 0.0, 0.0, 0.5625, 1.0, 1.0]])),
            128
        );
    }
}
