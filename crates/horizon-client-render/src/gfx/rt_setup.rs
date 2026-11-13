//! RT lighting's pipelines made with the renderer (`Renderer::new`): the depth prepass the rays
//! start from, and the translucent terrain blending with two sources (gfx.rs may not grow).

use super::*;

/// RT lighting's depth prepass pipelines over the terrain layout (`Renderer::rt_prepass`): its
/// draws' indirect arguments, the solid terrain's depth with and without the alpha test, the
/// translucent terrain's cover, and both eyes' at once where the device has multiview.
pub(super) fn prepass_pipelines(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    bindings: &wgpu::BindGroupLayout,
    terrain_buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    sprite_constants: &[(&str, f64)],
) -> (
    wgpu::Buffer,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    Option<EyeDepths>,
) {
    let tri = wgpu::PrimitiveTopology::TriangleList;
    let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    };
    // RT lighting's depth prepass: the solid terrain into a depth of its own, colour-less;
    // with the alpha test (`fs_depth`), or for the quads whose sprites have no transparent
    // texel without a fragment shader (`prepass_split`).
    let depth_prepass = |layout: &wgpu::PipelineLayout,
                         module: &wgpu::ShaderModule,
                         vertex: &str,
                         multiview_mask: Option<std::num::NonZeroU32>,
                         alpha_test: bool| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(if alpha_test {
                "Terrain depth for the rays"
            } else {
                "Opaque terrain depth for the rays"
            }),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some(vertex),
                compilation_options: Default::default(),
                buffers: terrain_buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: tri,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: alpha_test.then(|| wgpu::FragmentState {
                module,
                entry_point: Some("fs_depth"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: sprite_constants,
                    ..Default::default()
                },
                targets: &[],
            }),
            multiview_mask,
            cache: None,
        })
    };
    let prepass_indirect = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Prepass draws"),
        size: 20,
        usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let solid_depth = depth_prepass(layout, shader, "vs_terrain", None, true);
    let opaque_depth = depth_prepass(layout, shader, "vs_terrain", None, false);
    // The translucent terrain's cover over the prepass's depth (`draw_cover`): its quads as the
    // world draws them (back faces culled), tested against the depth without writing it.
    let cover_prepass = |layout: &wgpu::PipelineLayout,
                         module: &wgpu::ShaderModule,
                         vertex: &str,
                         multiview_mask: Option<std::num::NonZeroU32>| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Translucent terrain's cover for the rays"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some(vertex),
                compilation_options: Default::default(),
                buffers: terrain_buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: tri,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_cover"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COVER_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask,
            cache: None,
        })
    };
    let rt_cover = cover_prepass(layout, shader, "vs_terrain", None);
    // VR: both eyes' prepass in one multiview pass, where the device draws two views at once.
    let eye_depths = device
        .features()
        .contains(wgpu::Features::MULTIVIEW)
        .then(|| {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Horizon terrain, two views"),
                source: wgpu::ShaderSource::Wgsl(terrain_views().into()),
            });
            let views = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Both eyes' matrices"),
                entries: &[entry(
                    2,
                    wgpu::ShaderStages::VERTEX,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                )],
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Horizon terrain layout, two views"),
                bind_group_layouts: &[Some(bindings), Some(&views)],
                ..Default::default()
            });
            let matrices = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Both eyes' matrices"),
                size: 128,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Both eyes' matrices"),
                layout: &views,
                entries: &[wgpu::BindGroupEntry {
                    binding: 2,
                    resource: matrices.as_entire_binding(),
                }],
            });
            EyeDepths {
                pipeline: depth_prepass(
                    &layout,
                    &shader,
                    "vs_terrain_views",
                    std::num::NonZeroU32::new(0b11),
                    true,
                ),
                opaque: depth_prepass(
                    &layout,
                    &shader,
                    "vs_terrain_views",
                    std::num::NonZeroU32::new(0b11),
                    false,
                ),
                matrices,
                group,
                layers: None,
                next: None,
                cover: cover_prepass(
                    &layout,
                    &shader,
                    "vs_terrain_views",
                    std::num::NonZeroU32::new(0b11),
                ),
                covers: None,
                next_cover: None,
            }
        });
    (
        prepass_indirect,
        solid_depth,
        opaque_depth,
        rt_cover,
        eye_depths,
    )
}
/// Where the GPU blends with two sources: the translucent terrain tints what shows through it
/// (RT lighting's water), its own module, terrain.wgsl with the entry that writes both.
pub(super) fn dual_source_translucent(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    terrain_buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    sprite_constants: &[(&str, f64)],
    world_format: wgpu::TextureFormat,
    alpha_blend: wgpu::BlendState,
    translucent: wgpu::RenderPipeline,
) -> wgpu::RenderPipeline {
    let tri = wgpu::PrimitiveTopology::TriangleList;
    if device
        .features()
        .contains(wgpu::Features::DUAL_SOURCE_BLENDING)
    {
        let dual = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon terrain, dual-source"),
            source: wgpu::ShaderSource::Wgsl(dual_source_terrain().into()),
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Terrain translucent, dual-source"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &dual,
                entry_point: Some("vs_terrain"),
                compilation_options: Default::default(),
                buffers: terrain_buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: tri,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &dual,
                entry_point: Some("fs_translucent_dual"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: sprite_constants,
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format: world_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::Src1,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: alpha_blend.alpha,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    } else {
        translucent
    }
}

/// RT lighting's textures bound where it has none of its own (`Renderer::new`): its prepass's
/// depth, the sun's tint through glass, and its last light.
pub(super) fn placeholders(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::TextureView, wgpu::TextureView, wgpu::TextureView) {
    // RT lighting's depth prepass where none ran: nothing behind the water.
    let rt_depth_unused = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("No RT depth"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default());
    // RT lighting's sun through tinting glass where none was traced: all of it.
    let rt_sun_unused = {
        use wgpu::util::DeviceExt;
        let one = [0x3c00u16; 4];
        device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("No RT sun tint"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                bytemuck::cast_slice(&one),
            )
            .create_view(&Default::default())
    };
    // Bound in place of RT lighting's last light while there is none.
    let rt_history_unused = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("No RT light"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default());
    (rt_depth_unused, rt_sun_unused, rt_history_unused)
}

/// RT lighting's sky pipelines over the terrain layout (`Renderer::new`, `pipeline` its maker):
/// the sky, the real stars and the planets with their instances, and the three again after the
/// solid terrain (`SkyLast`) when `HORIZON_SKY_LAST` asks.
pub(super) fn sky_pipelines(
    device: &wgpu::Device,
    pipeline: &impl Fn(
        &str,
        &str,
        &str,
        &[Option<wgpu::VertexBufferLayout<'_>>],
        wgpu::PrimitiveTopology,
        Option<wgpu::Face>,
        bool,
        bool,
        Option<wgpu::BlendState>,
    ) -> wgpu::RenderPipeline,
) -> (
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::Buffer,
    wgpu::RenderPipeline,
    wgpu::Buffer,
    Option<(
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
    )>,
) {
    let tri = wgpu::PrimitiveTopology::TriangleList;
    let rt_sky = pipeline(
        "RT lighting's sky",
        "vs_sky",
        "fs_rt_sky",
        &[],
        tri,
        None,
        false,
        false,
        None,
    );
    // RT lighting's stars: the real ones (`sky_stars`), a quad each, added to the sky.
    let rt_star_pipeline = pipeline(
        "RT lighting's stars",
        "vs_rt_star",
        "fs_rt_star",
        &[Some(sky_stars::LAYOUT)],
        tri,
        None,
        false,
        false,
        Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        }),
    );
    let rt_star_buffer = sky_stars::buffer(device);
    // The planets (`sky_planets`): the stars' quads and light, their own instances.
    let rt_planet_pipeline = pipeline(
        "RT lighting's planets",
        "vs_rt_planet",
        "fs_rt_star",
        &[Some(sky_planets::LAYOUT)],
        tri,
        None,
        false,
        false,
        Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        }),
    );
    let rt_planet_buffer = sky_planets::buffer(device);
    let additive = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };
    let sky_last_pipelines = sky_last().then(|| {
        (
            pipeline(
                "RT lighting's sky, after the terrain",
                "vs_sky",
                "fs_rt_sky",
                &[],
                tri,
                None,
                false,
                true,
                None,
            ),
            pipeline(
                "RT lighting's stars, after the terrain",
                "vs_rt_star",
                "fs_rt_star",
                &[Some(sky_stars::LAYOUT)],
                tri,
                None,
                false,
                true,
                Some(additive),
            ),
            pipeline(
                "RT lighting's planets, after the terrain",
                "vs_rt_planet",
                "fs_rt_star",
                &[Some(sky_planets::LAYOUT)],
                tri,
                None,
                false,
                true,
                Some(additive),
            ),
        )
    });
    (
        rt_sky,
        rt_star_pipeline,
        rt_star_buffer,
        rt_planet_pipeline,
        rt_planet_buffer,
        sky_last_pipelines,
    )
}
