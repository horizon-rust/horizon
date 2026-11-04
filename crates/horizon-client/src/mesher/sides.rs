//! A mesh's solid quads sorted by the side they face, so that the renderer
//! can leave out the quads turned away from the eye before the GPU shades
//! their vertices.
//!
//! A quad in the plane `x = p` whose corners go counter-clockwise seen from
//! +X faces +X. From an eye at `x < p` it is seen from behind, and at
//! `x = p` edge on; the solid pipeline culls back faces, so it draws nothing
//! from there. A bucket of such quads draws nothing at all while the eye is
//! at or below the lowest of their planes, and likewise for the other five
//! axis directions. The seventh bucket holds every other quad (sloped,
//! rotated, degenerate, or whose two triangles wind opposite ways): it is
//! drawn from everywhere.
//!
//! The buckets lie one after another in the mesh, in an order chosen so that
//! the buckets an eye can see are usually one or two runs of quads: one draw
//! each. Inside a bucket the quads whose sprite has no transparent texel
//! come first, then the cutouts (leaves, plants, fences, iron bars, grass's
//! overlays), each kind in model order: coplanar quads facing one way (an
//! overlay, a cutout, on its opaque base) still draw in that order. RT
//! lighting's hybrid puts each bucket's opaque part in an opaque geometry of
//! the section's structure (`Sides::geometries`, gfx/hw_rays.rs), which the
//! hardware confirms without asking the shader's alpha test.
use super::{OPAQUE_LAYER, TerrainVertex};

/// The buckets in mesh order: +X, +Z, other, +Y, -Z, -X, -Y. Seen from
/// above (+Y, the commonest case), each pair of horizontal sides with a
/// common corner is then one run, or two.
const BUCKETS: [Bucket; 7] = [
    Bucket::Side(0, true),
    Bucket::Side(2, true),
    Bucket::Other,
    Bucket::Side(1, true),
    Bucket::Side(2, false),
    Bucket::Side(0, false),
    Bucket::Side(1, false),
];
const OTHER: usize = 2;
/// How far, in blocks, the eye must be behind a bucket's planes before the
/// bucket is left out. The renderer computes both the eye and the section
/// origins in `f32`, with errors far below this.
const MARGIN: f32 = 1.0 / 16.0;

#[derive(Clone, Copy)]
enum Bucket {
    /// Quads in a plane across `axis` facing its positive (`true`) or
    /// negative direction.
    Side(usize, bool),
    Other,
}

/// Where each bucket of a mesh's solid quads ends, and the planes that tell
/// whether it can face the eye.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sides {
    /// The solid quads up to the end of each bucket, in `BUCKETS` order.
    ends: [u32; 7],
    /// Per bucket facing a positive direction, its lowest plane; facing a
    /// negative one, its highest (unused for the other bucket).
    planes: [f32; 7],
    /// The opaque quads at the start of each bucket (the cutouts follow).
    opaque: [u32; 7],
}

impl Sides {
    /// `quads` solid quads in one bucket drawn from everywhere: for meshes
    /// that were not sorted.
    pub fn unsorted(quads: u32) -> Self {
        let mut ends = [0; 7];
        ends[OTHER..].fill(quads);
        Self {
            ends,
            planes: [0.0; 7],
            opaque: [0; 7],
        }
    }

    /// The number of solid quads the buckets hold.
    pub fn quads(&self) -> u32 {
        self.ends[6]
    }

    /// The solid quads in runs of one kind, in mesh order: `(first, count,
    /// opaque)`, opaque runs holding quads whose sprites have no transparent
    /// texel (or forceOpaque leaves), the others the cutouts. Empty parts are
    /// left out and neighbours of one kind merged, so a section of stone and
    /// grass is one or two runs; at most 14 (7 buckets, each opaque then cutout).
    pub fn geometries(&self) -> Vec<(u32, u32, bool)> {
        let mut out: Vec<(u32, u32, bool)> = Vec::with_capacity(4);
        let mut start = 0;
        for (&end, &opaque) in self.ends.iter().zip(&self.opaque) {
            let parts = [
                (start, opaque, true),
                (start + opaque, end - start - opaque, false),
            ];
            for (first, count, kind) in parts {
                if count == 0 {
                    continue;
                }
                match out.last_mut() {
                    Some(last) if last.2 == kind => last.1 += count,
                    _ => out.push((first, count, kind)),
                }
            }
            start = end;
        }
        out
    }

    /// Calls `draw(first, count)` for each run of quads an eye at `eye`
    /// (relative to the mesh's origin) can see the front of, nearest the
    /// start first. Empty buckets do not break a run.
    pub fn visible(&self, eye: [f32; 3], mut draw: impl FnMut(u32, u32)) {
        let mut start = 0;
        let mut run: Option<u32> = None;
        for (i, bucket) in BUCKETS.iter().enumerate() {
            let end = self.ends[i];
            if end == start {
                continue;
            }
            let seen = match *bucket {
                Bucket::Side(axis, true) => eye[axis] > self.planes[i] - MARGIN,
                Bucket::Side(axis, false) => eye[axis] < self.planes[i] + MARGIN,
                Bucket::Other => true,
            };
            match (seen, run) {
                (true, None) => run = Some(start),
                (false, Some(first)) => {
                    draw(first, start - first);
                    run = None;
                }
                _ => {}
            }
            start = end;
        }
        if let Some(first) = run {
            draw(first, start - first);
        }
    }
}

/// The bucket of a quad: the axis plane it lies in and the side its
/// triangles (corners 0, 1, 2 and 0, 2, 3, as the renderer draws them) face.
fn bucket(quad: &[TerrainVertex; 4]) -> usize {
    let p = quad.map(|v| v.pos);
    for axis in 0..3 {
        if p.iter().any(|v| v[axis] != p[0][axis]) {
            continue;
        }
        // The normal's component along `axis`, twice each triangle's area.
        let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
        let area = |a: usize, b: usize, c: usize| {
            (p[b][u] - p[a][u]) * (p[c][w] - p[a][w]) - (p[b][w] - p[a][w]) * (p[c][u] - p[a][u])
        };
        let (first, second) = (area(0, 1, 2), area(0, 2, 3));
        let positive = first >= 0.0 && second >= 0.0;
        let negative = first <= 0.0 && second <= 0.0;
        if positive == negative {
            // Opposite windings, or no area at all.
            return OTHER;
        }
        return BUCKETS
            .iter()
            .position(|b| matches!(*b, Bucket::Side(a, s) if a == axis && s == positive))
            .expect("every side has a bucket");
    }
    OTHER
}

/// Whether a quad's sprite has no transparent texel (`opaque_layers`, by
/// sprite layer: `Sprites::opaque`), or it is a forceOpaque leaf
/// (`OPAQUE_LAYER`): the hybrid's alpha test (shadow_rays_hw.wgsl
/// `hw_opaque`) would confirm every hit on it.
fn opaque(quad: &[TerrainVertex; 4], opaque_layers: &[bool]) -> bool {
    let layer = quad[0].layer;
    layer & OPAQUE_LAYER != 0
        || opaque_layers
            .get(usize::from(layer))
            .copied()
            .unwrap_or(false)
}

/// Reorders `solid`'s quads by bucket, and inside each the opaque ones
/// (`opaque_layers`, by sprite layer; empty: none) before the cutouts, keeping
/// the model order of each kind.
pub fn sort(solid: &mut Vec<TerrainVertex>, opaque_layers: &[bool]) -> Sides {
    let (quads, rest) = solid.as_chunks::<4>();
    debug_assert!(rest.is_empty(), "solid vertices come in quads");
    // A quad's group: twice its bucket, plus one for a cutout.
    let groups: Vec<u8> = quads
        .iter()
        .map(|q| (bucket(q) * 2 + usize::from(!opaque(q, opaque_layers))) as u8)
        .collect();
    let mut counts = [0u32; 7];
    let mut planes = [0f32; 7];
    let mut in_group = [0u32; 14];
    for (quad, &g) in quads.iter().zip(&groups) {
        in_group[usize::from(g)] += 1;
        let b = usize::from(g / 2);
        if let Bucket::Side(axis, positive) = BUCKETS[b] {
            let p = quad[0].pos[axis];
            planes[b] = match (counts[b], positive) {
                (0, _) => p,
                (_, true) => planes[b].min(p),
                (_, false) => planes[b].max(p),
            };
        }
        counts[b] += 1;
    }
    let mut ends = [0u32; 7];
    let mut total = 0;
    for (end, count) in ends.iter_mut().zip(counts) {
        total += count;
        *end = total;
    }
    let opaque: [u32; 7] = std::array::from_fn(|b| in_group[b * 2]);
    if in_group.iter().filter(|&&c| c > 0).count() > 1 {
        let mut next: [u32; 14] = std::array::from_fn(|g| {
            let b = g / 2;
            ends[b] - counts[b] + if g % 2 == 1 { opaque[b] } else { 0 }
        });
        let mut sorted = vec![quads[0]; quads.len()];
        for (quad, &g) in quads.iter().zip(&groups) {
            let slot = &mut next[usize::from(g)];
            sorted[*slot as usize] = *quad;
            *slot += 1;
        }
        *solid = sorted.into_flattened();
    }
    Sides {
        ends,
        planes,
        opaque,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(corners: [[f32; 3]; 4], layer: u16) -> [TerrainVertex; 4] {
        corners.map(|p| TerrainVertex::new(p, [0.0; 2], layer, 0, 0))
    }

    /// A unit block's six faces at `origin`, wound counter-clockwise seen
    /// from outside, as block models are.
    fn cube(origin: [f32; 3], layer: u16) -> Vec<TerrainVertex> {
        let [x, y, z] = origin;
        let (a, b, c) = (x + 1.0, y + 1.0, z + 1.0);
        [
            [[a, y, z], [a, b, z], [a, b, c], [a, y, c]], // +X
            [[x, y, z], [x, y, c], [x, b, c], [x, b, z]], // -X
            [[x, b, z], [x, b, c], [a, b, c], [a, b, z]], // +Y
            [[x, y, z], [a, y, z], [a, y, c], [x, y, c]], // -Y
            [[x, y, c], [a, y, c], [a, b, c], [x, b, c]], // +Z
            [[x, y, z], [x, b, z], [a, b, z], [a, y, z]], // -Z
        ]
        .into_iter()
        .flat_map(|q| quad(q, layer))
        .collect()
    }

    fn runs(sides: &Sides, eye: [f32; 3]) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        sides.visible(eye, |first, count| out.push((first, count)));
        out
    }

    /// The quads of `solid` whose triangles face `eye`, by their layer.
    fn front_facing(solid: &[TerrainVertex], eye: [f32; 3]) -> Vec<u16> {
        let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        solid
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|q| {
                [[0, 1, 2], [0, 2, 3]].iter().any(|t| {
                    let (e1, e2) = (sub(q[t[1]].pos, q[t[0]].pos), sub(q[t[2]].pos, q[t[0]].pos));
                    let n = [
                        e1[1] * e2[2] - e1[2] * e2[1],
                        e1[2] * e2[0] - e1[0] * e2[2],
                        e1[0] * e2[1] - e1[1] * e2[0],
                    ];
                    let to_eye = sub(eye, q[t[0]].pos);
                    n[0] * to_eye[0] + n[1] * to_eye[1] + n[2] * to_eye[2] > 0.0
                })
            })
            .map(|q| q[0].layer)
            .collect()
    }

    #[test]
    fn cube_faces_go_to_their_buckets_in_order() {
        let mut solid = cube([3.0, 4.0, 5.0], 0);
        for (i, v) in solid.iter_mut().enumerate() {
            v.layer = (i / 4) as u16;
        }
        let sides = sort(&mut solid, &[]);
        assert_eq!(sides.ends, [1, 2, 2, 3, 4, 5, 6]);
        let layers: Vec<u16> = solid.iter().step_by(4).map(|v| v.layer).collect();
        // +X, +Z, +Y, -Z, -X, -Y.
        assert_eq!(layers, [0, 4, 2, 5, 1, 3]);
        assert_eq!(sides.planes[0], 4.0); // +X at x = 4
        assert_eq!(sides.planes[5], 3.0); // -X at x = 3
        assert_eq!(sides.planes[3], 5.0); // +Y at y = 5
    }

    #[test]
    fn the_drawn_runs_hold_every_quad_that_faces_the_eye() {
        // As the workers sort (no opaque layers), and with every other layer opaque.
        let opaque: Vec<bool> = (0..64).map(|l| l % 2 == 0).collect();
        for table in [&[][..], &opaque] {
            every_quad_facing_the_eye_is_drawn(table);
        }
    }

    fn every_quad_facing_the_eye_is_drawn(opaque_layers: &[bool]) {
        // Blocks across a section, with a sloped quad and a bowtie.
        let mut solid = Vec::new();
        let mut layer = 0;
        for origin in [
            [0.0, 0.0, 0.0],
            [7.0, 3.0, 12.0],
            [15.0, 15.0, 15.0],
            [2.0, 9.0, 4.0],
        ] {
            for q in cube(origin, 0).chunks(4) {
                solid.extend(q.iter().map(|v| TerrainVertex { layer, ..*v }));
                layer += 1;
            }
        }
        let slope = [
            [1.0, 1.0, 1.0],
            [2.0, 1.0, 1.0],
            [2.0, 2.0, 2.0],
            [1.0, 2.0, 2.0],
        ];
        let bowtie = [
            [5.0, 1.0, 1.0],
            [5.0, 2.0, 2.0],
            [5.0, 1.0, 2.0],
            [5.0, 2.0, 1.0],
        ];
        for corners in [slope, bowtie] {
            solid.extend(quad(corners, layer));
            layer += 1;
        }
        let sides = sort(&mut solid, opaque_layers);
        assert_eq!(sides.quads(), u32::from(layer));
        let eyes = [-20.0f32, -0.5, 0.0, 3.5, 8.0, 15.99, 16.0, 30.0];
        for &x in &eyes {
            for &y in &eyes {
                for &z in &eyes {
                    let eye = [x, y, z];
                    let mut drawn = Vec::new();
                    for (first, count) in runs(&sides, eye) {
                        drawn.extend(
                            solid[first as usize * 4..(first + count) as usize * 4]
                                .iter()
                                .step_by(4)
                                .map(|v| v.layer),
                        );
                    }
                    for layer in front_facing(&solid, eye) {
                        assert!(drawn.contains(&layer), "{eye:?} misses quad {layer}");
                    }
                }
            }
        }
    }

    /// Whether the quad from vertex `v` is opaque in `table` (`opaque`).
    fn is_opaque(solid: &[TerrainVertex], quad: u32, table: &[bool]) -> bool {
        let v = quad as usize * 4;
        opaque(&[solid[v], solid[v + 1], solid[v + 2], solid[v + 3]], table)
    }

    #[test]
    fn each_bucket_draws_its_opaque_quads_before_its_cutouts() {
        // Stone (layer 0, opaque), then a grass block: its opaque sides (1) with their cutout
        // overlays (2) on the same planes after them in model order, its top (3, opaque); a
        // leaf block (4, cutout) and a forceOpaque one (4 with `OPAQUE_LAYER`).
        let table = [true, true, false, true, false];
        let mut solid = cube([0.0, 0.0, 0.0], 0);
        let grass = cube([2.0, 0.0, 0.0], 1);
        for (i, q) in grass.chunks(4).enumerate() {
            let layer = if i == 2 { 3 } else { 1 };
            solid.extend(q.iter().map(|v| TerrainVertex { layer, ..*v }));
            if i != 2 && i != 3 {
                // The overlay on the sides, coplanar, drawn after its base.
                solid.extend(q.iter().map(|v| TerrainVertex { layer: 2, ..*v }));
            }
        }
        solid.extend(cube([4.0, 0.0, 0.0], 4));
        solid.extend(cube([6.0, 0.0, 0.0], 4 | OPAQUE_LAYER));
        let before: Vec<u16> = solid.iter().step_by(4).map(|v| v.layer).collect();
        let sides = sort(&mut solid, &table);
        assert_eq!(sides.quads() as usize, before.len());
        // Every quad is still there.
        let mut after: Vec<u16> = solid.iter().step_by(4).map(|v| v.layer).collect();
        let mut sorted_before = before.clone();
        sorted_before.sort_unstable();
        after.sort_unstable();
        assert_eq!(after, sorted_before);
        // Per bucket: the opaque ones, then the cutouts, as counted.
        let mut start = 0;
        for b in 0..7 {
            let end = sides.ends[b];
            for q in start..end {
                let first = q < start + sides.opaque[b];
                assert_eq!(is_opaque(&solid, q, &table), first, "bucket {b} quad {q}");
            }
            start = end;
        }
        // Stone's +X face, the grass's +X side, then its overlay: each kind in model order.
        let x: Vec<u16> = solid[..sides.ends[0] as usize * 4]
            .iter()
            .step_by(4)
            .map(|v| v.layer)
            .collect();
        assert_eq!(x, [0, 1, 4 | OPAQUE_LAYER, 2, 4]);
        // A side bucket of the grass's: 3 opaque (stone, grass, forceOpaque leaves), 2 cutout.
        assert_eq!(sides.opaque[0], 3);
        // The top bucket has no cutout but the leaves' top.
        assert_eq!(sides.opaque[3], 3);
        // The geometries: runs of one kind, covering every quad in order, as `opaque` says.
        let runs = sides.geometries();
        let mut next = 0;
        for (i, &(first, count, kind)) in runs.iter().enumerate() {
            assert_eq!(first, next);
            assert!(count > 0);
            if i > 0 {
                assert_ne!(runs[i - 1].2, kind, "neighbours of one kind merge");
            }
            for q in first..first + count {
                assert_eq!(is_opaque(&solid, q, &table), kind);
            }
            next = first + count;
        }
        assert_eq!(next, sides.quads());
        assert!(runs.len() <= 14);
    }

    #[test]
    fn a_mesh_of_one_kind_is_one_geometry() {
        let mut stone = cube([0.0; 3], 0);
        let sides = sort(&mut stone, &[true]);
        assert_eq!(sides.geometries(), [(0, 6, true)]);
        let mut leaves = cube([0.0; 3], 0);
        let sides = sort(&mut leaves, &[false]);
        assert_eq!(sides.geometries(), [(0, 6, false)]);
        assert_eq!(Sides::unsorted(9).geometries(), [(0, 9, false)]);
        assert!(Sides::unsorted(0).geometries().is_empty());
        // One bucket, opaque and cutout mixed: reordered, opaque first.
        let face = [
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
        ];
        let mut solid: Vec<_> = [1, 0, 1, 0]
            .into_iter()
            .flat_map(|l| quad(face, l))
            .collect();
        let sides = sort(&mut solid, &[true, false]);
        let layers: Vec<u16> = solid.iter().step_by(4).map(|v| v.layer).collect();
        assert_eq!(layers, [0, 0, 1, 1]);
        assert_eq!(sides.geometries(), [(0, 2, true), (2, 2, false)]);
    }

    #[test]
    fn runs_merge_across_hidden_and_empty_buckets() {
        let mut solid = cube([0.0; 3], 0);
        let sides = sort(&mut solid, &[]);
        // From above, +X and +Z: +X, +Z, (no other), +Y is one run.
        assert_eq!(runs(&sides, [8.0, 8.0, 8.0]), [(0, 3)]);
        // From below, -X and -Z: -Z, -X, -Y.
        assert_eq!(runs(&sides, [-8.0, -8.0, -8.0]), [(3, 3)]);
        // From above, -X and +Z: +Z, +Y, then -X.
        assert_eq!(runs(&sides, [-8.0, 8.0, 8.0]), [(1, 2), (4, 1)]);
        // Inside the block every face is seen from behind.
        assert!(runs(&sides, [0.5; 3]).is_empty());
        // Level with its top, the eye may see it: the margin keeps it.
        assert_eq!(runs(&sides, [0.5, 1.0, 0.5]), [(2, 1)]);
    }

    #[test]
    fn unsorted_meshes_draw_everything() {
        let sides = Sides::unsorted(9);
        assert_eq!(sides.quads(), 9);
        assert_eq!(runs(&sides, [-100.0; 3]), [(0, 9)]);
        assert!(runs(&Sides::unsorted(0), [0.0; 3]).is_empty());
    }

    #[test]
    fn one_bucket_keeps_the_mesh_as_it_is() {
        let mut solid: Vec<_> = (0..3)
            .flat_map(|i| {
                quad(
                    [
                        [1.0, 0.0, 0.0],
                        [1.0, 1.0, 0.0],
                        [1.0, 1.0, 1.0],
                        [1.0, 0.0, 1.0],
                    ],
                    i,
                )
            })
            .collect();
        let before = solid.clone();
        let sides = sort(&mut solid, &[]);
        assert_eq!(sides.ends, [3; 7]);
        assert_eq!(
            solid.iter().map(|v| v.layer).collect::<Vec<_>>(),
            before.iter().map(|v| v.layer).collect::<Vec<_>>()
        );
    }
}
