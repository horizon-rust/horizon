//! Bloom for Linear Light with RT lighting (`tonemap`, bloom.wgsl): the scene's light halved
//! over and over, blurred as it goes (Jimenez's 13 taps), then blended back up level by level.
//! Remix's bloom keeps the light over a luminance threshold and adds the glow to the scene at
//! its burn intensity, so the sun's disc, lava and lamps glow; the same chain, with no threshold,
//! is the exposure's meter and the local tone mapping's average.

/// The chain's format: light above paper white, no alpha.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// How much of each level's light comes from the smaller ones when blending up: 0 would keep
/// the first level alone, 1 the smallest.
const SPREAD: f64 = 0.6;

/// The pipelines, made once.
pub struct Bloom {
    /// A level's source: its view and the sampler.
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    first: wgpu::RenderPipeline,
    /// The first level keeping the light over the threshold (bloom.wgsl `BRIGHT`).
    first_bright: wgpu::RenderPipeline,
    /// The parameters and the exposure, for the first level.
    params_layout: wgpu::BindGroupLayout,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
}

/// The chain of one scene size.
pub struct Chain {
    /// The first level's view and each level's.
    levels: Vec<wgpu::TextureView>,
    /// Level `i + 1` drawn from level `i`.
    down: Vec<wgpu::BindGroup>,
    /// Level `i` blended from level `i + 1`.
    up: Vec<wgpu::BindGroup>,
}

impl Bloom {
    /// `scene_layout`: the scene's and the shadow mask's (`Tonemap::layout`).
    pub fn new(device: &wgpu::Device, scene_layout: &wgpu::BindGroupLayout) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon bloom"),
            source: wgpu::ShaderSource::Wgsl(include_str!("bloom.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon bloom"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Horizon bloom"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let levels_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon bloom"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        // The first level reads the scene (group 1) and the post-processing's parameters and
        // exposure (group 2), the others the level before.
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon bloom's parameters"),
            entries: &[uniform(0), uniform(1)],
        });
        let first_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon bloom's first level"),
            bind_group_layouts: &[None, Some(scene_layout), Some(&params_layout)],
            ..Default::default()
        });
        let pipeline = |entry: &str, layout, blend, constants: &[(&str, f64)]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // The level drawn into keeps `1 - constant` of what it had.
        let mix = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Constant,
                dst_factor: wgpu::BlendFactor::OneMinusConstant,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let bright = pipeline("fs_first", &first_layout, None, &[("BRIGHT", 1.0)]);
        Self {
            first_bright: bright,
            params_layout,
            first: pipeline("fs_first", &first_layout, None, &[]),
            down: pipeline("fs_down", &levels_layout, None, &[]),
            up: pipeline("fs_up", &levels_layout, Some(mix), &[]),
            layout,
            sampler,
        }
    }

    /// The layout of the first level's parameters group (`Grading::params`, the exposure).
    pub fn params_layout(&self) -> &wgpu::BindGroupLayout {
        &self.params_layout
    }

    /// The chain for a scene of `size`, of at most `levels` levels and none smaller than about
    /// `2 << spare` pixels on the short side.
    pub fn chain(&self, device: &wgpu::Device, size: [u32; 2], levels: u32, spare: u32) -> Chain {
        let half = [(size[0] / 2).max(1), (size[1] / 2).max(1)];
        let fit = half[0].min(half[1]).max(1).ilog2().saturating_sub(spare).max(1);
        let count = fit.min(levels);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Horizon bloom"),
            size: wgpu::Extent3d {
                width: half[0],
                height: half[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let levels: Vec<_> = (0..count)
            .map(|mip| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("Horizon bloom level"),
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let group = |source: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Horizon bloom"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        };
        Chain {
            down: levels[..levels.len() - 1].iter().map(group).collect(),
            up: levels[1..].iter().map(group).collect(),
            levels,
        }
    }

    fn pass(
        encoder: &mut wgpu::CommandEncoder,
        label: &str,
        pipeline: &wgpu::RenderPipeline,
        groups: [Option<&wgpu::BindGroup>; 3],
        target: &wgpu::TextureView,
        blend: Option<f64>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: if blend.is_some() {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        for (i, group) in groups.into_iter().enumerate() {
            pass.set_bind_group(i as u32, group, &[]);
        }
        if let Some(constant) = blend {
            pass.set_blend_constant(wgpu::Color {
                r: constant,
                g: constant,
                b: constant,
                a: constant,
            });
        }
        pass.draw(0..3, 0..1);
    }

    /// The scene's light (`scene`, with its mask: tonemap.wgsl's group) halved level by level;
    /// `bright` keeps only what is over the bloom's threshold (`params`: `params_layout`'s group).
    pub fn down(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        chain: &Chain,
        scene: &wgpu::BindGroup,
        params: &wgpu::BindGroup,
        bright: bool,
    ) {
        Self::pass(
            encoder,
            "Bloom, first level",
            if bright { &self.first_bright } else { &self.first },
            [None, Some(scene), Some(params)],
            &chain.levels[0],
            None,
        );
        for (i, group) in chain.down.iter().enumerate() {
            Self::pass(
                encoder,
                "Bloom, down",
                &self.down,
                [Some(group), None, None],
                &chain.levels[i + 1],
                None,
            );
        }
    }

    /// Each level mixed with the smaller ones', the first level left holding the glow.
    pub fn up(&self, encoder: &mut wgpu::CommandEncoder, chain: &Chain) {
        for (i, group) in chain.up.iter().enumerate().rev() {
            Self::pass(
                encoder,
                "Bloom, up",
                &self.up,
                [Some(group), None, None],
                &chain.levels[i],
                Some(SPREAD),
            );
        }
    }
}

impl Chain {
    /// The first level: the glow, once `Bloom::up` has run, the scene's light halved before.
    pub fn glow(&self) -> &wgpu::TextureView {
        &self.levels[0]
    }

    /// The level the exposure is measured on, about 16 pixels of the scene to a texel.
    pub fn meter(&self) -> &wgpu::TextureView {
        self.level(3)
    }

    /// Level `i` (the last if there are fewer): the scene's light averaged over `2 << i` pixels.
    pub fn level(&self, i: usize) -> &wgpu::TextureView {
        &self.levels[i.min(self.levels.len() - 1)]
    }
}
