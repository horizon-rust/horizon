//! A resource pack's `core/text` shader on world text (`RenderPipelines.TEXT_POLYGON_OFFSET`,
//! `TEXT_SEE_THROUGH` and, with improved transparency, `OIT_TEXT_POLYGON_OFFSET`): text displays,
//! name tags and their backgrounds, which vanilla 26.3 draws with that shader too, as `GUI_TEXT`
//! draws the HUD's (`hud/pack_text.rs`). A pack that recolours or reshapes them (a wall drawn on
//! a text display's background) draws as in vanilla. Without one they draw natively
//! (`fs_entity_unmarked` in entity.wgsl, `oit_features.rs`).

use super::oit::{Stage, stage_targets};
use crate::core_shader::{CoreProgram, Glue, other_binding};
use std::cell::RefCell;
use std::sync::Arc;

/// The entity vertices (`mesher::Vertex`) as `POSITION_COLOR_TEX_LIGHTMAP`. Their light bytes are
/// sky then block, each times 16; vanilla's `UV2` is block then sky.
pub(crate) const WORLD_GLUE: Glue = Glue {
    inputs: &[
        ("Position", "vec3"),
        ("Color", "vec4"),
        ("UV0", "vec2"),
        ("UV2", "ivec2"),
    ],
    prelude: "layout(location = 0) in vec3 hz_pos;\n\
              layout(location = 1) in vec2 hz_uv;\n\
              layout(location = 3) in vec4 hz_color;\n\
              layout(location = 4) in uvec4 hz_light;",
    setup: "Position = hz_pos; Color = hz_color; UV0 = hz_uv; UV2 = ivec2(hz_light.y, hz_light.x);",
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
};

/// `TEXT_SEE_THROUGH`'s format, `POSITION_COLOR_TEX`.
pub(crate) const SEE_THROUGH_GLUE: Glue = Glue {
    inputs: &[("Position", "vec3"), ("Color", "vec4"), ("UV0", "vec2")],
    prelude: "layout(location = 0) in vec3 hz_pos;\n\
              layout(location = 1) in vec2 hz_uv;\n\
              layout(location = 3) in vec4 hz_color;",
    setup: "Position = hz_pos; Color = hz_color; UV0 = hz_uv;",
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
};

/// `RenderPipelines`' OIT snippet for `oit.rs`'s targets (two coefficient targets of a rank 2
/// wavelet), then each stage's own defines, in `Stage` order. Device depth runs 0 to 1 here.
pub(crate) const OIT_DEFINES: [&[&str]; 3] = [
    &[
        "OIT",
        "OIT_WAVELET_RANK 2",
        "OIT_COEFF_COUNT 8",
        "OIT_COEFF_ATTACHMENT_COUNT 2",
        "RENDERPEARL_DEPTH_IS_ZERO_TO_ONE",
        "OIT_ALPHA_ONLY",
        "OIT_DEPTH_BOUNDS",
    ],
    &[
        "OIT",
        "OIT_WAVELET_RANK 2",
        "OIT_COEFF_COUNT 8",
        "OIT_COEFF_ATTACHMENT_COUNT 2",
        "RENDERPEARL_DEPTH_IS_ZERO_TO_ONE",
        "OIT_ALPHA_ONLY",
        "OIT_TRANSMITTANCE",
    ],
    &[
        "OIT",
        "OIT_WAVELET_RANK 2",
        "OIT_COEFF_COUNT 8",
        "OIT_COEFF_ATTACHMENT_COUNT 2",
        "RENDERPEARL_DEPTH_IS_ZERO_TO_ONE",
        "OIT_ACCUMULATE",
    ],
];

/// The OIT samplers vanilla gives these pipelines: the culled depth bounds, the coefficients.
pub(crate) const OIT_SAMPLERS: [&str; 3] = ["DepthBoundsSampler", "Coeff0", "Coeff1"];

const BLOCK_OFFSETS: [u64; 4] = [0, 256, 512, 768];
const BLOCK_SIZES: [u64; 4] = [48, 64, 160, 48];

/// A resource pack's `core/text`, translated for each pipeline that draws with it; none draws
/// that text natively.
#[derive(Default)]
pub struct TextShaders {
    /// `GUI_TEXT` (`IS_GUI`): the HUD's and the screens' text.
    pub gui: Option<Arc<CoreProgram>>,
    /// `TEXT_POLYGON_OFFSET`: text displays and name tags.
    pub world: Option<Arc<CoreProgram>>,
    /// `TEXT_SEE_THROUGH` (`IS_SEE_THROUGH`).
    pub see_through: Option<Arc<CoreProgram>>,
    /// `OIT_TEXT_POLYGON_OFFSET`, one per `Stage`.
    pub oit: [Option<Arc<CoreProgram>>; 3],
}

/// What the packs' core shaders read alike this frame: the `Globals` block
/// (`post_chain::Globals::bytes`) and the fog with its sky and clouds ends.
#[derive(Clone, Copy)]
pub(super) struct PackBlocks {
    pub(super) globals: [u8; 48],
    pub(super) fog: [f32; 6],
}

impl super::Renderer {
    /// The resource packs' `core/text` for world text, or the built-in text pipelines.
    pub fn set_world_text_shaders(&mut self, shaders: &TextShaders) {
        self.core_text = CoreText::new(
            &self.device,
            self.output.frame.format,
            shaders.world.as_deref(),
            shaders.see_through.as_deref(),
            shaders.oit.each_ref().map(|s| s.as_deref()),
            &self.entity_atlas,
            (&self.lightmap_view, &self.lightmap_sampler),
        );
    }

    pub(super) fn pack_blocks(
        &self,
        frame: &super::Frame,
        fog: [f32; 4],
        ends: [f32; 4],
    ) -> PackBlocks {
        let globals = crate::post_chain::Globals {
            glint_alpha: frame.glint_alpha,
            ..frame.post_effects.globals
        };
        let size = [self.config.width, self.config.height];
        PackBlocks {
            globals: globals.bytes(size, frame.post_effects.menu_blur_radius as i32),
            fog: [fog[0], fog[1], fog[2], fog[3], ends[0], ends[1]],
        }
    }

    /// The pack's world text blocks: the entities' view and projection (layering 1). Written
    /// through the queue, which applies it before this frame's commands.
    pub(super) fn write_core_text(
        &self,
        pack: &PackBlocks,
        view: &super::View,
        pose: glam::Mat4,
        jitter: glam::Mat4,
        fog_color: [f32; 4],
    ) {
        let Some(core) = &self.core_text else {
            return;
        };
        if let Some(oit) = &self.oit {
            core.prepare_oit(&self.device, oit);
        }
        let data =
            super::core_lines::uniforms(pack.globals, view, pose, jitter, 1.0, fog_color, pack.fog);
        self.queue.write_buffer(&core.uniform, 0, &data);
    }

    /// Group 0 for a text draw: the pack's pipeline over the one set, when the pack has one for
    /// it (`see_through`, else polygon offset) and `allowed`, else the entities' resources.
    pub(super) fn bind_world_text(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        allowed: bool,
        see_through: bool,
    ) {
        let pack = self
            .core_text
            .as_ref()
            .filter(|_| allowed)
            .and_then(|core| {
                let pipeline = if see_through {
                    core.see_through.as_ref()
                } else {
                    core.polygon_offset.as_ref()
                };
                Some((pipeline?, &core.group))
            });
        match pack {
            Some((pipeline, group)) => {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, group, &[]);
            }
            None => pass.set_bind_group(0, &self.entity_group, &[]),
        }
    }
}

/// What an improved transparency stage's text quads draw with: the pack's pipeline and group,
/// the entities' group set again after, or the native pipeline.
pub(super) struct OitText<'a> {
    pub(super) core: Option<&'a CoreText>,
    pub(super) entities: &'a wgpu::BindGroup,
}

impl OitText<'_> {
    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        stage: Stage,
        native: &wgpu::RenderPipeline,
        quads: u32,
        first: u32,
    ) {
        if quads == 0 {
            return;
        }
        let base = (first * 4) as i32;
        match self.core.and_then(|c| c.oit_stage(stage)) {
            Some((pipeline, group)) => {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw_indexed(0..quads * 6, base, 0..1);
                pass.set_bind_group(0, self.entities, &[]);
            }
            None => {
                pass.set_pipeline(native);
                pass.draw_indexed(0..quads * 6, base, 0..1);
            }
        }
    }
}

/// One improved transparency stage: its pipeline, its group's layout and the bindings of the OIT
/// samplers its program reads, and the group made for the targets of a given size.
struct OitStage {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    samplers: Vec<(usize, u32)>,
    group: RefCell<Option<([u32; 2], wgpu::BindGroup)>>,
}

pub(super) struct CoreText {
    pub(super) polygon_offset: Option<wgpu::RenderPipeline>,
    pub(super) see_through: Option<wgpu::RenderPipeline>,
    pub(super) group: wgpu::BindGroup,
    oit: [Option<OitStage>; 3],
    uniform: wgpu::Buffer,
    atlas: wgpu::TextureView,
    glyphs: wgpu::Sampler,
    lightmap: (wgpu::TextureView, wgpu::Sampler),
    texel: wgpu::Sampler,
}

fn buffer(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(BLOCK_SIZES[binding as usize]),
        },
        count: None,
    }
}

fn texture(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler(binding: u32, filtering: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Sampler(if filtering {
            wgpu::SamplerBindingType::Filtering
        } else {
            wgpu::SamplerBindingType::NonFiltering
        }),
        count: None,
    }
}

/// The blocks, then `core_shader::SAMPLERS`' Sampler0 (4, 5) and Sampler2 (8, 9).
fn base_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![
        buffer(0),
        buffer(1),
        buffer(2),
        buffer(3),
        texture(4, true),
        sampler(5, true),
        texture(8, true),
        sampler(9, true),
    ]
}

/// The group entries of `base_entries`.
fn base_resources<'a>(
    uniform: &'a wgpu::Buffer,
    atlas: &'a wgpu::TextureView,
    glyphs: &'a wgpu::Sampler,
    lightmap: &'a (wgpu::TextureView, wgpu::Sampler),
) -> Vec<wgpu::BindGroupEntry<'a>> {
    let mut resources: Vec<wgpu::BindGroupEntry> = (0..4)
        .map(|i| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: uniform,
                offset: BLOCK_OFFSETS[i],
                size: wgpu::BufferSize::new(BLOCK_SIZES[i]),
            }),
        })
        .collect();
    resources.extend([
        wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::TextureView(atlas),
        },
        wgpu::BindGroupEntry {
            binding: 5,
            resource: wgpu::BindingResource::Sampler(glyphs),
        },
        wgpu::BindGroupEntry {
            binding: 8,
            resource: wgpu::BindingResource::TextureView(&lightmap.0),
        },
        wgpu::BindGroupEntry {
            binding: 9,
            resource: wgpu::BindingResource::Sampler(&lightmap.1),
        },
    ]);
    resources
}

impl CoreText {
    /// `world`, `see_through` and `oit` (one per `Stage`) are the pack's programs; `atlas` holds
    /// the glyphs world text samples, `lightmap` what `Sampler2` reads.
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        world: Option<&CoreProgram>,
        see_through: Option<&CoreProgram>,
        oit: [Option<&CoreProgram>; 3],
        atlas: &wgpu::TextureView,
        lightmap: (&wgpu::TextureView, &wgpu::Sampler),
    ) -> Option<Self> {
        if world.is_none() && see_through.is_none() {
            return None;
        }
        // A pack's shader is untrusted: the backend failing to compile it is an internal error.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Resource pack world text"),
            entries: &base_entries(),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Resource pack world text uniforms"),
            size: 1024,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyphs = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Resource pack world text glyphs"),
            ..Default::default()
        });
        let texel = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Resource pack world text OIT targets"),
            ..Default::default()
        });
        let lightmap = (lightmap.0.clone(), lightmap.1.clone());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Resource pack world text"),
            layout: &layout,
            entries: &base_resources(&uniform, atlas, &glyphs, &lightmap),
        });
        let mut text = Self {
            polygon_offset: None,
            see_through: None,
            group,
            oit: [None, None, None],
            uniform,
            atlas: atlas.clone(),
            glyphs,
            lightmap,
            texel,
        };
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::mesher::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        // The built-in text pipelines' state (gfx.rs, oit_features.rs): culled, TEXT_POLYGON_OFFSET
        // depth tested with its bias, TEXT_SEE_THROUGH over everything, the OIT stages without
        // depth writes into their targets.
        let pipeline = |label: &str,
                        program: &CoreProgram,
                        layout: &wgpu::BindGroupLayout,
                        depth: wgpu::DepthStencilState,
                        targets: &[Option<wgpu::ColorTargetState>]| {
            let module = |module: &naga::Module| {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
                })
            };
            let (vertex, fragment) = (module(&program.vertex), module(&program.fragment));
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                ..Default::default()
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &vertex,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(depth),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &fragment,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let depth = |write, compare, bias| wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: Default::default(),
            bias,
        };
        let offset = wgpu::DepthBiasState {
            constant: 10,
            slope_scale: 1.0,
            clamp: 0.0,
        };
        let blended = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        text.polygon_offset = world.map(|program| {
            pipeline(
                "Resource pack polygon offset text",
                program,
                &layout,
                depth(true, wgpu::CompareFunction::GreaterEqual, offset),
                &blended,
            )
        });
        text.see_through = see_through.map(|program| {
            pipeline(
                "Resource pack see-through text",
                program,
                &layout,
                depth(false, wgpu::CompareFunction::Always, Default::default()),
                &blended,
            )
        });
        let stages = [Stage::DepthBounds, Stage::Transmittance, Stage::Accumulate];
        for (stage, program) in stages.into_iter().zip(oit) {
            let Some(program) = program else { continue };
            // The OIT samplers this stage's program reads, at its `other` bindings, and only
            // those: the transmittance stage writes the coefficients its shader names but
            // doesn't read. The targets are read texel by texel; the bounds aren't filterable.
            let reads = |name: &str| {
                let texture = format!("hz_tex_{name}");
                [&program.vertex, &program.fragment].iter().any(|module| {
                    module
                        .global_variables
                        .iter()
                        .any(|(_, g)| g.name.as_deref() == Some(texture.as_str()))
                })
            };
            let samplers: Vec<(usize, u32)> = program
                .other
                .iter()
                .enumerate()
                .filter(|(_, name)| reads(name))
                .filter_map(|(i, name)| {
                    let input = OIT_SAMPLERS.iter().position(|s| s == name)?;
                    Some((input, other_binding(i)))
                })
                .collect();
            let mut entries = base_entries();
            for &(_, binding) in &samplers {
                entries.extend([texture(binding, false), sampler(binding + 1, false)]);
            }
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Resource pack world text OIT"),
                entries: &entries,
            });
            let pipeline = pipeline(
                "Resource pack OIT text",
                program,
                &layout,
                depth(false, wgpu::CompareFunction::GreaterEqual, offset),
                &stage_targets(stage),
            );
            text.oit[stage as usize] = Some(OitStage {
                pipeline,
                layout,
                samplers,
                group: RefCell::new(None),
            });
        }
        // The stages share one transmittance function: all of them the pack's, or none.
        if text.oit.iter().any(Option::is_none) {
            text.oit = [None, None, None];
        }
        let internal = pollster::block_on(internal.pop());
        if let Some(e) = internal.or(pollster::block_on(validation.pop())) {
            log::warn!("the resource pack's world text shader is not used: {e}");
            return None;
        }
        Some(text)
    }

    /// The OIT stages' groups for `oit.rs`'s targets of this frame, made again when they are.
    pub(super) fn prepare_oit(&self, device: &wgpu::Device, oit: &super::oit::Oit) {
        if let Some((size, bounds, coefficients)) = oit.pack_inputs() {
            self.prepare_oit_inputs(device, size, [bounds, coefficients[0], coefficients[1]]);
        }
    }

    /// [`prepare_oit`](Self::prepare_oit) for `OIT_SAMPLERS`' views, made at `size`.
    fn prepare_oit_inputs(
        &self,
        device: &wgpu::Device,
        size: [u32; 2],
        inputs: [&wgpu::TextureView; 3],
    ) {
        for index in 0..3 {
            let Some(stage) = &self.oit[index] else {
                continue;
            };
            if stage
                .group
                .borrow()
                .as_ref()
                .is_some_and(|(made, _)| *made == size)
            {
                continue;
            }
            let mut entries =
                base_resources(&self.uniform, &self.atlas, &self.glyphs, &self.lightmap);
            for &(input, binding) in &stage.samplers {
                entries.extend([
                    wgpu::BindGroupEntry {
                        binding,
                        resource: wgpu::BindingResource::TextureView(inputs[input]),
                    },
                    wgpu::BindGroupEntry {
                        binding: binding + 1,
                        resource: wgpu::BindingResource::Sampler(&self.texel),
                    },
                ]);
            }
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Resource pack world text OIT"),
                layout: &stage.layout,
                entries: &entries,
            });
            *stage.group.borrow_mut() = Some((size, group));
        }
    }

    /// The pack's pipeline and group for an improved transparency stage, once prepared.
    pub(super) fn oit_stage(
        &self,
        stage: Stage,
    ) -> Option<(&wgpu::RenderPipeline, wgpu::BindGroup)> {
        let s = self.oit[stage as usize].as_ref()?;
        Some((&s.pipeline, s.group.borrow().as_ref()?.1.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One quad through each stage's pipeline into `oit.rs`'s target formats, as `Oit::run`
    /// does: the depth bounds, then the coefficients (reading the bounds), then the accumulation
    /// (reading both). A stage that read a target it writes fails the submit's validation.
    fn draw_the_oit_stages(device: &wgpu::Device, queue: &wgpu::Queue, text: CoreText) {
        let size = [8, 8];
        let target = |format, label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bounds = view(&target(wgpu::TextureFormat::Rgba32Float, "bounds"));
        let coefficients = [
            view(&target(wgpu::TextureFormat::Rgba16Float, "coefficients 0")),
            view(&target(wgpu::TextureFormat::Rgba16Float, "coefficients 1")),
        ];
        let accumulated = view(&target(wgpu::TextureFormat::Rgba16Float, "accumulated"));
        let depth = view(&target(wgpu::TextureFormat::Depth32Float, "depth"));
        text.prepare_oit_inputs(device, size, [&bounds, &coefficients[0], &coefficients[1]]);
        let vertices = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: None,
                contents: &vec![0; 6 * std::mem::size_of::<crate::mesher::Vertex>()],
                usage: wgpu::BufferUsages::VERTEX,
            },
        );
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&Default::default());
        let stages: [(Stage, Vec<&wgpu::TextureView>); 3] = [
            (Stage::DepthBounds, vec![&bounds]),
            (
                Stage::Transmittance,
                vec![&coefficients[0], &coefficients[1]],
            ),
            (Stage::Accumulate, vec![&accumulated]),
        ];
        for (stage, targets) in stages {
            let (pipeline, group) = text.oit_stage(stage).expect("the stage's pipeline");
            let attachments: Vec<_> = targets
                .into_iter()
                .map(|view| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })
                })
                .collect();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..6, 0..1);
        }
        queue.submit([encoder.finish()]);
        if let Some(e) = pollster::block_on(validation.pop()) {
            panic!("the OIT stages don't draw: {e}");
        }
    }

    /// A device, with what improved transparency needs when the adapter has it.
    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        let descriptor = wgpu::DeviceDescriptor {
            required_features: super::super::oit::features(&adapter),
            ..Default::default()
        };
        pollster::block_on(adapter.request_device(&descriptor)).ok()
    }

    /// A text display's quad drawn with a pack's shader that writes the `UV2` it gets: the
    /// vertex's light bytes (sky, then block) reach it as vanilla's `UV2` (block, then sky).
    #[test]
    fn a_pack_text_shader_reads_the_light_as_vanilla_does() {
        let Some((device, queue)) = device() else {
            eprintln!("no GPU: skipping");
            return;
        };
        let vsh = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
            layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\n\
            layout(location = 2) in vec2 UV0;\nlayout(location = 3) in ivec2 UV2;\n\
            layout(location = 0) flat out ivec2 light;\n\
            void main() { gl_Position = vec4(Position, 1.0); light = UV2; }\n";
        let fsh = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
            layout(location = 0) flat in ivec2 light;\nlayout(location = 0) out vec4 fragColor;\n\
            void main() { fragColor = vec4(vec2(light) / 255.0, 0.0, 1.0); }\n";
        let program =
            crate::core_shader::translate_core(vsh, fsh, &[], &|_| None, &WORLD_GLUE).unwrap();
        const SIZE: u32 = 64;
        let texture = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let binding = wgpu::TextureUsages::TEXTURE_BINDING;
        let atlas =
            texture(wgpu::TextureFormat::Rgba8Unorm, binding).create_view(&Default::default());
        let sampler = device.create_sampler(&Default::default());
        let text = CoreText::new(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            Some(&program),
            None,
            [None, None, None],
            &atlas,
            (&atlas, &sampler),
        )
        .expect("the pack's pipeline");
        let target = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let color = texture(
            wgpu::TextureFormat::Rgba8Unorm,
            target | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(wgpu::TextureFormat::Depth32Float, target);
        // Sky 3, block 12 (each times 16), on two triangles over the whole target.
        let light = 0x30 | (0xC0 << 8);
        let corner = |x: f32, y: f32| crate::mesher::Vertex {
            pos: [x, y, 0.5],
            uv: [0.0; 2],
            layer: 0,
            color: u32::MAX,
            light,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        };
        let quad = [
            corner(-1.0, -1.0),
            corner(1.0, -1.0),
            corner(1.0, 1.0),
            corner(-1.0, -1.0),
            corner(1.0, 1.0),
            corner(-1.0, 1.0),
        ];
        let vertices = wgpu::util::DeviceExt::create_buffer_init(
            &device,
            &wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&quad),
                usage: wgpu::BufferUsages::VERTEX,
            },
        );
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let color_view = color.create_view(&Default::default());
            let depth_view = depth.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(text.polygon_offset.as_ref().unwrap());
            pass.set_bind_group(0, &text.group, &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..6, 0..1);
        }
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: Some(SIZE),
                },
            },
            color.size(),
        );
        queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixels = read.slice(..).get_mapped_range().unwrap().to_vec();
        let middle = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
        assert_eq!(
            &pixels[middle..middle + 4],
            &[0xC0, 0x30, 0, 0xFF],
            "UV2 is (block, sky)"
        );
    }

    /// Vanilla's own `text.vsh` and `text.fsh` (from the prepared resources) translate for every
    /// world text pipeline, the improved transparency stages included, and make their pipelines.
    #[test]
    fn vanillas_text_shader_makes_every_world_text_pipeline() {
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        let read = |path: &str| std::fs::read_to_string(root.join(path)).ok();
        let (Some(vertex), Some(fragment)) = (
            read("assets/minecraft/shaders/core/text.vsh"),
            read("assets/minecraft/shaders/core/text.fsh"),
        ) else {
            eprintln!(
                "skipping resource-backed world text check: {}",
                root.display()
            );
            return;
        };
        let include = |id: &str| {
            let (namespace, path) = id.split_once(':')?;
            read(&format!("assets/{namespace}/{path}"))
        };
        let translate = |defines: &[&str], glue: &Glue| {
            crate::core_shader::translate_core_with(
                &vertex,
                &fragment,
                defines,
                &include,
                glue,
                &|name| OIT_SAMPLERS.contains(&name),
            )
            .expect("vanilla's text shader translates")
        };
        let world = translate(&[], &WORLD_GLUE);
        let see_through = translate(&["IS_SEE_THROUGH"], &SEE_THROUGH_GLUE);
        let oit = OIT_DEFINES.map(|defines| translate(defines, &WORLD_GLUE));
        let Some((device, queue)) = device() else {
            eprintln!("no GPU: skipping the pipelines");
            return;
        };
        let texture = |format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 4,
                        height: 4,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let atlas = texture(wgpu::TextureFormat::Rgba8Unorm);
        let lightmap = texture(wgpu::TextureFormat::Rgba8Unorm);
        let sampler = device.create_sampler(&Default::default());
        let with_oit = super::super::oit::supported(&device);
        let text = CoreText::new(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            Some(&world),
            Some(&see_through),
            oit.each_ref().map(|p| with_oit.then_some(p)),
            &atlas,
            (&lightmap, &sampler),
        )
        .expect("the pack's world text pipelines");
        assert!(text.polygon_offset.is_some() && text.see_through.is_some());
        if with_oit {
            assert!(text.oit.iter().all(Option::is_some), "every OIT stage");
            // What each stage samples (vanilla's oit.glsl): nothing while it finds the depth
            // bounds, the bounds while it adds transmittance into the coefficients it writes,
            // the bounds and the coefficients while it accumulates.
            let read = |stage: usize| -> Vec<usize> {
                text.oit[stage]
                    .as_ref()
                    .unwrap()
                    .samplers
                    .iter()
                    .map(|s| s.0)
                    .collect()
            };
            assert_eq!(read(0), Vec::<usize>::new());
            assert_eq!(read(1), [0]);
            assert_eq!(read(2), [0, 1, 2]);
            draw_the_oit_stages(&device, &queue, text);
        } else {
            eprintln!("no FLOAT32_BLENDABLE: the OIT stages were translated, not made");
        }
    }
}
