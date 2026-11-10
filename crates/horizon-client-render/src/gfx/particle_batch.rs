//! Compact GPU input for ordinary translucent particle quads.
//! The shader supplies the normal and glint UV, which are zero for these quads.
//! Other entity materials and custom particle attributes retain their full format.
use super::Vertex;

pub(super) const ATTRIBUTES: [wgpu::VertexAttribute; 5] =
    wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4];

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct ParticleVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    layer: u32,
    color: u32,
    light: u32,
}

impl From<&Vertex> for ParticleVertex {
    fn from(v: &Vertex) -> Self {
        Self {
            pos: v.pos,
            uv: v.uv,
            layer: v.layer,
            color: v.color,
            light: v.light,
        }
    }
}

/// Convert only when every omitted attribute is zero. Modded particles with
/// custom normals or glint UVs fall back to the original 44-byte vertex path.
pub(super) fn prepare(vertices: &[Vertex], output: &mut Vec<ParticleVertex>) -> bool {
    output.clear();
    if !vertices
        .iter()
        .all(|v| v.normal == [0; 4] && v.glint_uv == [0.0; 2])
    {
        return false;
    }
    output.extend(vertices.iter().map(ParticleVertex::from));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_particles_keep_every_variable_attribute() {
        let v = Vertex {
            pos: [1.25, -0.3, 13.0],
            uv: [0.15, 0.8],
            layer: 15 | (3 << 16),
            color: 0xaabbccdd,
            light: 0x12345678,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        };
        assert_eq!(std::mem::size_of::<ParticleVertex>(), 32);
        assert_eq!(std::mem::size_of::<Vertex>(), 44);
        let mut packed = Vec::new();
        assert!(prepare(&[v], &mut packed));
        assert_eq!(
            bytemuck::bytes_of(&packed[0]),
            &bytemuck::bytes_of(&v)[..32]
        );

        let changed = Vertex {
            normal: [0, 0, 0, 1],
            ..v
        };
        assert!(!prepare(&[v, changed], &mut packed));
        assert!(packed.is_empty());
        let changed = Vertex {
            glint_uv: [0.5, 0.0],
            ..v
        };
        assert!(!prepare(&[changed], &mut packed));
        assert!(packed.is_empty());
    }
}
