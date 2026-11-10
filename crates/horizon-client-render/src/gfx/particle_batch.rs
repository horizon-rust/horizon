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

pub(super) struct CompactParticles {
    pipeline: wgpu::RenderPipeline,
    hdr_mark_pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    capacity: u64,
    vertices: Vec<ParticleVertex>,
}

impl CompactParticles {
    pub(super) fn new(
        kept: &super::reuse::On<'_>,
        device: &wgpu::Device,
        (shader, layout): (&wgpu::ShaderModule, &wgpu::PipelineLayout),
        (format, bias): (wgpu::TextureFormat, wgpu::DepthBiasState),
    ) -> Self {
        let vertex = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ParticleVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBUTES,
        })];
        let make = |label, depth_write, blend, fragment, write_mask| {
            kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_particle"),
                    compilation_options: Default::default(),
                    buffers: &vertex,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias,
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(fragment),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[(
                            "HDR_WRITE_LIGHT",
                            f64::from(u8::from(format == super::LINEAR_FRAME)),
                        )],
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make(
            "Compact translucent particles",
            true,
            wgpu::BlendState::ALPHA_BLENDING,
            "fs_entity",
            wgpu::ColorWrites::ALL,
        );
        let hdr_mark_pipeline = make(
            "Compact HDR particle mark",
            false,
            wgpu::BlendState::REPLACE,
            "fs_hdr_mark",
            wgpu::ColorWrites::ALPHA,
        );
        let capacity = 4;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Compact translucent particles"),
            size: capacity * std::mem::size_of::<ParticleVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            hdr_mark_pipeline,
            buffer,
            capacity,
            vertices: Vec::new(),
        }
    }

    /// Preserve the offsets of every other entity material in the shared buffer.
    pub(super) fn upload(
        &mut self,
        (device, uploads, encoder): (
            &wgpu::Device,
            &mut super::upload::Uploads,
            &mut wgpu::CommandEncoder,
        ),
        (entity_buffer, entities): (&wgpu::Buffer, &[Vertex]),
        (posed, particle_base, particle_quads): (u32, u32, u32),
        eligible: bool,
    ) -> bool {
        let first = (particle_base * 4 - posed) as usize;
        let end = first + particle_quads as usize * 4;
        let compact =
            particle_quads >= 256 && eligible && prepare(&entities[first..end], &mut self.vertices);
        if !compact {
            self.vertices.clear();
        }
        if entities.is_empty() {
            return compact;
        }
        let stride = std::mem::size_of::<Vertex>() as u64;
        if compact {
            if first > 0 {
                uploads.buffer(
                    encoder,
                    entity_buffer,
                    u64::from(posed) * stride,
                    bytemuck::cast_slice(&entities[..first]),
                );
            }
            if end < entities.len() {
                uploads.buffer(
                    encoder,
                    entity_buffer,
                    (u64::from(posed) + end as u64) * stride,
                    bytemuck::cast_slice(&entities[end..]),
                );
            }
            let vertices = self.vertices.len() as u64;
            if vertices > self.capacity {
                self.capacity = vertices.next_power_of_two();
                self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Compact translucent particles"),
                    size: self.capacity * std::mem::size_of::<ParticleVertex>() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            uploads.buffer(
                encoder,
                &self.buffer,
                0,
                bytemuck::cast_slice(&self.vertices),
            );
        } else {
            uploads.buffer(
                encoder,
                entity_buffer,
                u64::from(posed) * stride,
                bytemuck::cast_slice(entities),
            );
        }
        compact
    }
}

impl super::Renderer {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_particles<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        compact: bool,
        quads: u32,
        original_first: i32,
        entity_pipeline: &'a wgpu::RenderPipeline,
        warp_group: Option<&'a wgpu::BindGroup>,
        entity_mark_pipeline: &'a wgpu::RenderPipeline,
    ) {
        pass.set_pipeline(if compact {
            &self.particles.pipeline
        } else {
            entity_pipeline
        });
        if let Some(group) = warp_group {
            pass.set_bind_group(2, group, &[]);
        }
        pass.set_bind_group(0, &self.entity_group, &[]);
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_vertex_buffer(
            0,
            if compact {
                self.particles.buffer.slice(..)
            } else {
                self.entity_buffer.slice(..)
            },
        );
        let first = if compact { 0 } else { original_first };
        pass.draw_indexed(0..quads * 6, first, 0..1);
        if super::light_marks(self.output.frame.format, self.hdr.is_some()) {
            pass.set_pipeline(if compact {
                &self.particles.hdr_mark_pipeline
            } else {
                entity_mark_pipeline
            });
            pass.draw_indexed(0..quads * 6, first, 0..1);
        }
        pass.set_bind_group(0, &self.group, &[]);
    }
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
