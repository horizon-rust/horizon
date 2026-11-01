//! A far tile's solid quads cut into meshlets for the mesh shader path
//! (`lod_mesh.rs`).
//!
//! A meshlet is up to `QUADS` quads of one side bucket (`mesher::Sides`),
//! next to each other in the tile's mesh, with the box that holds them and
//! the plane `Sides::visible_from` compares the eye with: a task shader can
//! leave a meshlet out when it is outside the frustum or its quads face away
//! from the eye, before any of its vertices are shaded. Meshlets lie in the
//! mesh's order, so a tile's are one run, bucket by bucket.
use crate::mesher::{Sides, TerrainVertex, bucket_rule};

/// Quads in a meshlet at most: 64 vertices and 32 triangles, within what
/// every mesh shader implementation outputs per workgroup.
pub const QUADS: u32 = 16;

/// One meshlet, in the layout of `lod_mesh.wgsl`'s `Meshlet`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Meshlet {
    /// First quad in the tile's solid quads, and how many.
    pub first: u32,
    pub count: u32,
    /// The bucket, in `Sides` order.
    pub side: u32,
    /// For a bucket facing a positive direction its lowest plane, for a
    /// negative one its highest (unused for the bucket drawn from everywhere).
    pub plane: f32,
    /// The box, relative to the tile's origin.
    pub min: [f32; 3],
    /// First vertex in the arena page: the tile's base and the quads before
    /// this one's. Set when the GPU's tile list is built (`LodGpu::rebuild`).
    pub base: u32,
    pub max: [f32; 3],
    pub pad: u32,
}

/// The meshlets of `solid` (its quads sorted by `sides`).
pub fn build(solid: &[TerrainVertex], sides: &Sides) -> Vec<Meshlet> {
    let quads = solid.as_chunks::<4>().0;
    let mut meshlets = Vec::with_capacity(quads.len().div_ceil(QUADS as usize) + 7);
    let mut start = 0u32;
    for (side, &end) in sides.ends().iter().enumerate() {
        let end = end.min(quads.len() as u32);
        let rule = bucket_rule(side);
        let mut first = start;
        while first < end {
            let count = QUADS.min(end - first);
            let chunk = &quads[first as usize..(first + count) as usize];
            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            let mut plane: Option<f32> = None;
            for quad in chunk {
                for vertex in quad {
                    for k in 0..3 {
                        min[k] = min[k].min(vertex.pos[k]);
                        max[k] = max[k].max(vertex.pos[k]);
                    }
                }
                if let Some((axis, positive)) = rule {
                    let p = quad[0].pos[axis];
                    plane = Some(match (plane, positive) {
                        (None, _) => p,
                        (Some(q), true) => q.min(p),
                        (Some(q), false) => q.max(p),
                    });
                }
            }
            meshlets.push(Meshlet {
                first,
                count,
                side: side as u32,
                plane: plane.unwrap_or(0.0),
                min,
                base: 0,
                max,
                pad: 0,
            });
            first += count;
        }
        start = end.max(start);
    }
    meshlets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::SectionMesh;

    fn quad(corners: [[f32; 3]; 4]) -> [TerrainVertex; 4] {
        corners.map(|p| TerrainVertex::new(p, [0.0; 2], 0, 0, 0))
    }

    /// `n` unit quads facing +Y at height `y`, side by side along x.
    fn tops(n: usize, y: f32) -> Vec<TerrainVertex> {
        (0..n)
            .flat_map(|i| {
                let x = i as f32;
                quad([
                    [x, y, 0.0],
                    [x, y, 1.0],
                    [x + 1.0, y, 1.0],
                    [x + 1.0, y, 0.0],
                ])
            })
            .collect()
    }

    /// A quad in the plane x = `x` facing +X.
    fn wall(x: f32) -> [TerrainVertex; 4] {
        quad([[x, 0.0, 0.0], [x, 1.0, 0.0], [x, 1.0, 1.0], [x, 0.0, 1.0]])
    }

    #[test]
    fn meshlets_cut_each_bucket_into_runs_of_sixteen_quads() {
        let mut mesh = SectionMesh {
            solid: tops(35, 5.0),
            ..SectionMesh::default()
        };
        mesh.solid.extend(wall(2.0));
        mesh.solid.extend(wall(1.0));
        mesh.sort_solid();
        let meshlets = build(&mesh.solid, &mesh.sides);
        // The two +X walls (bucket 0), then 35 tops (+Y, bucket 3) in 16, 16 and 3.
        let counts: Vec<u32> = meshlets.iter().map(|m| m.count).collect();
        assert_eq!(counts, [2, 16, 16, 3]);
        let mut next = 0;
        for m in &meshlets {
            assert_eq!(m.first, next, "meshlets follow the mesh's order");
            next += m.count;
        }
        assert_eq!(next, 37);
        assert_eq!((meshlets[0].side, meshlets[0].plane), (0, 1.0));
        assert_eq!(meshlets[0].min, [1.0, 0.0, 0.0]);
        let top = &meshlets[1];
        assert_eq!((top.side, top.plane), (3, 5.0));
        assert_eq!(top.min, [0.0, 5.0, 0.0]);
        assert_eq!(top.max, [16.0, 5.0, 1.0]);
        assert_eq!(meshlets[3].max[0], 35.0);
    }

    #[test]
    fn a_mesh_without_quads_has_no_meshlets() {
        assert!(build(&[], &Sides::default()).is_empty());
    }

    #[test]
    fn meshlets_are_the_size_the_shader_reads() {
        assert_eq!(std::mem::size_of::<Meshlet>(), 48);
    }
}
