//! Impostors on the GPU (impostor.rs has what they are): the page-array texture the baked tiles
//! are copied into, one pipeline, and the quads' vertex buffer. The texture is made with the first
//! tile; nothing exists on a game that never meets a far entity.
use super::upload::Uploads;
use crate::impostor::{self, Draw, TILE, TILE_BYTES, Vertex};

pub(super) struct Impostors {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas: Option<Atlas>,
    vertices: wgpu::Buffer,
    capacity: usize,
    count: u32,
}

struct Atlas {
    texture: wgpu::Texture,
    group: wgpu::BindGroup,
}

impl Impostors {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        entities: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Impostors"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../impostor.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Impostor tiles"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Impostors layout"),
            bind_group_layouts: &[Some(entities), Some(&layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Impostors"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_impostor"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            // Opaque with the alpha cut: they write depth, so far terrain nearer than an
            // impostor hides it and the impostors hide each other in any order.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_impostor"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Impostor tiles"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            atlas: None,
            vertices: Self::buffer(device, 4),
            capacity: 4,
            count: 0,
        }
    }

    fn buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Impostor vertices"),
            size: (capacity * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// The frame's new tiles go into the atlas and its quads into the vertex buffer.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut Uploads,
        draw: &Draw,
    ) {
        self.count = 0;
        if draw.uploads.is_empty() && (draw.vertices.is_empty() || self.atlas.is_none()) {
            return;
        }
        let (layout, sampler) = (&self.layout, &self.sampler);
        let atlas = self
            .atlas
            .get_or_insert_with(|| Atlas::new(device, layout, sampler));
        for tile in draw.uploads {
            if tile.pixels.len() != TILE_BYTES || tile.slot >= impostor::SLOTS {
                continue;
            }
            let (page, tx, ty) = impostor::slot_origin(tile.slot);
            let mut offset = 0;
            for mip in 0..impostor::MIPS as u32 {
                let size = (TILE >> mip) as u32;
                let bytes = (size * size * 4) as usize;
                uploads.texture(
                    encoder,
                    wgpu::TexelCopyTextureInfo {
                        texture: &atlas.texture,
                        mip_level: mip,
                        origin: wgpu::Origin3d {
                            x: (tx * TILE as u32) >> mip,
                            y: (ty * TILE as u32) >> mip,
                            z: page,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    size,
                    size,
                    &tile.pixels[offset..offset + bytes],
                );
                offset += bytes;
            }
        }
        if draw.vertices.is_empty() {
            return;
        }
        if draw.vertices.len() > self.capacity {
            self.capacity = draw.vertices.len().next_power_of_two();
            self.vertices = Self::buffer(device, self.capacity);
        }
        uploads.buffer(
            encoder,
            &self.vertices,
            0,
            bytemuck::cast_slice(draw.vertices),
        );
        self.count = draw.vertices.len() as u32;
    }

    /// The quads, in a pass with the entities' depth: `indices` is the renderer's shared quad indices.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        entities: &wgpu::BindGroup,
        indices: &wgpu::Buffer,
    ) {
        let Some(atlas) = &self.atlas else {
            return;
        };
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, entities, &[]);
        pass.set_bind_group(1, &atlas.group, &[]);
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw_indexed(0..self.count / 4 * 6, 0, 0..1);
    }
}

impl Atlas {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Impostor tiles"),
            size: wgpu::Extent3d {
                width: impostor::PAGE,
                height: impostor::PAGE,
                depth_or_array_layers: impostor::PAGES,
            },
            mip_level_count: impostor::MIPS as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Impostor tiles"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        Self { texture, group }
    }
}
