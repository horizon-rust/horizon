//! DLSS Ray Reconstruction's guide buffers (docs/rt-dlss-rr.md, "Guides"): for each depth pixel of
//! a view, the diffuse albedo, the specular albedo, the normal with the roughness, and the motion
//! vector, written by a compute pass (`rt_guides.wgsl`, `cs_guides`) appended to the hardware
//! kernel's shader and run with the kernel's own bind group for the view: right after the kernel
//! on the RT depth prepass, or, for DLSS-RR's view, later on the world pass's own jittered depth
//! (`ShadowRays::run_guides`). The forward terrain is untouched: each guide comes from a short
//! hardware ray across the surface the depth shows (`hw_hit`), with water and glass found by a
//! second one, and the depth's normal where both miss.
//!
//! Nothing of it exists (no shader, no pipeline, no textures, no dispatch) unless
//! [`guides_wanted`]: `HORIZON_RT_GUIDES=1`, or DLSS-RR wanted (`HORIZON_RT_DLSS_RR=1`, read here
//! directly so it builds without the `dlss-rr` feature). `HORIZON_RT_GUIDES_SHOW` pictures one of
//! them on the window ([`Show`]), to look at them without DLSS.

use super::shadow_rays;

/// The views kept at once, as the kernel keeps them (`shadow_rays::KEPT`).
const KEPT: usize = 5;

/// Whether the guides are made: `HORIZON_RT_GUIDES=1` or `HORIZON_RT_DLSS_RR=1`, or a picture of
/// one asked for (`HORIZON_RT_GUIDES_SHOW`).
pub fn guides_wanted() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = |name: &str| std::env::var(name).is_ok_and(|v| v == "1");
        on("HORIZON_RT_GUIDES") || on("HORIZON_RT_DLSS_RR") || show() != Show::None
    })
}

/// Which guide `HORIZON_RT_GUIDES_SHOW` pictures on the window, over the frame and its HUD
/// (rt_guides.wgsl `GUIDES_SHOW`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    None = 0,
    /// The diffuse albedo, gamma-encoded for the screen.
    Albedo = 1,
    /// The world-space normal, `n * 0.5 + 0.5` (up is green).
    Normal = 2,
    /// The roughness, black 0 to white 1.
    Roughness = 3,
    /// The motion, red right and green down from grey, 16 pixels a frame to full.
    Motion = 4,
    /// The specular albedo, gamma-encoded.
    Specular = 5,
    /// Where each pixel's guides came from: green the hardware ray's triangle, red the depth's
    /// fallback, yellow a puddle or a wet film, blue water or glass, black the sky.
    Source = 6,
    /// The depth's distance, white near to black far: which depth the guides were made from.
    Depth = 7,
}

impl Show {
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "albedo" | "diffuse" => Self::Albedo,
            "normal" | "normals" => Self::Normal,
            "roughness" => Self::Roughness,
            "motion" => Self::Motion,
            "specular" => Self::Specular,
            "source" => Self::Source,
            "depth" => Self::Depth,
            _ => Self::None,
        }
    }
}

/// `HORIZON_RT_GUIDES_SHOW`: `albedo`, `normal`, `roughness`, `motion`, `specular`, `source` or
/// `depth`.
pub fn show() -> Show {
    static SHOW: std::sync::OnceLock<Show> = std::sync::OnceLock::new();
    *SHOW.get_or_init(|| {
        std::env::var("HORIZON_RT_GUIDES_SHOW").map_or(Show::None, |v| Show::parse(&v))
    })
}

/// The kernel's shader with the guides' pass after it.
pub fn source(split: u32) -> String {
    shadow_rays::source(true, split) + include_str!("rt_guides.wgsl")
}

pub const DIFFUSE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const SPECULAR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const NORMALS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const SHOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// rt_guides.wgsl `GuideParams`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    jitter: [f32; 4],
}

/// One view's guides, at its depth's size (the render size), for DLSS-RR's evaluation
/// (horizon-rtx `dlss_rr::Inputs`).
#[derive(Clone)]
pub struct Guides {
    pub size: [u32; 2],
    /// Linear diffuse albedo (rgb).
    pub diffuse_albedo: wgpu::TextureView,
    /// The preintegrated specular albedo (rgb), not F0.
    pub specular_albedo: wgpu::TextureView,
    /// World-space unit normal (xyz, toward the camera), roughness in w (`roughness_packed`).
    pub normals: wgpu::TextureView,
    /// Render pixels from this frame's position to last frame's (previous - current), x right,
    /// y down, unjittered (the jitter is taken out): `mv_scale` [1, 1], `mv_jittered` false.
    pub motion: wgpu::TextureView,
    /// `HORIZON_RT_GUIDES_SHOW`'s picture (1x1 and unwritten without it).
    show: wgpu::Texture,
}

struct ViewGuides {
    view: usize,
    guides: Guides,
    show: wgpu::TextureView,
    params: wgpu::Buffer,
}

pub struct RtGuides {
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    views: Vec<ViewGuides>,
    /// `guide_objects`' stand-in without the entities' own motion.
    no_objects: wgpu::TextureView,
    /// The picture's blit onto the window, by the target's format.
    blitter: Option<(wgpu::TextureFormat, wgpu::util::TextureBlitter)>,
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    skip: u32,
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("RT guides"),
        layout: Some(layout),
        module: shader,
        entry_point: Some("cs_guides"),
        compilation_options: wgpu::PipelineCompilationOptions {
            constants: &[
                ("GUIDES_SHOW", f64::from(show() as u32)),
                ("SKIP", f64::from(skip)),
            ],
            ..Default::default()
        },
        cache: None,
    })
}

impl RtGuides {
    /// The pass over the kernel's bind group layout `kernel` (group 0), with the kernel's `skip`.
    pub fn new(
        device: &wgpu::Device,
        kernel: &wgpu::BindGroupLayout,
        split: u32,
        skip: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RT guides"),
            source: wgpu::ShaderSource::Wgsl(source(split).into()),
        });
        let storage = |binding, format| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT guides"),
            entries: &[
                storage(0, DIFFUSE_FORMAT),
                storage(1, SPECULAR_FORMAT),
                storage(2, NORMALS_FORMAT),
                storage(3, MOTION_FORMAT),
                storage(4, SHOW_FORMAT),
                texture(5, wgpu::TextureSampleType::Depth),
                texture(6, wgpu::TextureSampleType::Float { filterable: false }),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RT guides"),
            bind_group_layouts: &[Some(kernel), Some(&layout)],
            ..Default::default()
        });
        let pipeline = pipeline(device, &pipeline_layout, &shader, skip);
        Self {
            shader,
            pipeline_layout,
            pipeline,
            layout,
            views: Vec::new(),
            no_objects: Self::texture(
                device,
                [1, 1],
                super::frame_gen::motion::FORMAT,
                "RT guides: no entity motion",
            )
            .create_view(&Default::default()),
            blitter: None,
        }
    }

    /// The pass again with the kernel's new `skip` (`ShadowRays::set_skip`).
    pub fn set_skip(&mut self, device: &wgpu::Device, skip: u32) {
        self.pipeline = pipeline(device, &self.pipeline_layout, &self.shader, skip);
    }

    fn texture(
        device: &wgpu::Device,
        size: [u32; 2],
        format: wgpu::TextureFormat,
        label: &str,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    }

    fn view_guides(&mut self, device: &wgpu::Device, view: usize, size: [u32; 2]) -> usize {
        match self.views.iter().position(|v| v.view == view) {
            Some(i) if self.views[i].guides.size == size => return i,
            Some(i) => {
                self.views.remove(i);
            }
            None if self.views.len() == KEPT => {
                self.views.remove(0);
            }
            None => {}
        }
        let made = |format, label| Self::texture(device, size, format, label);
        let show_size = if show() == Show::None { [1, 1] } else { size };
        let guides = Guides {
            size,
            diffuse_albedo: made(DIFFUSE_FORMAT, "RT guide: diffuse albedo")
                .create_view(&Default::default()),
            specular_albedo: made(SPECULAR_FORMAT, "RT guide: specular albedo")
                .create_view(&Default::default()),
            normals: made(NORMALS_FORMAT, "RT guide: normals and roughness")
                .create_view(&Default::default()),
            motion: made(MOTION_FORMAT, "RT guide: motion vectors")
                .create_view(&Default::default()),
            show: Self::texture(device, show_size, SHOW_FORMAT, "RT guide: picture"),
        };
        let show = guides.show.create_view(&Default::default());
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RT guides"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.views.push(ViewGuides {
            view,
            guides,
            show,
            params,
        });
        self.views.len() - 1
    }

    /// Writes view `view`'s guides from `depth` of `size` (the RT prepass, or the world's own
    /// depth, whose image moved by `jitter` pixels right and down), with the kernel's bind group
    /// `kernel` as the kernel ran it for this view (its uniform still the view's in `encoder`).
    /// `objects`: the entities' own motion over that depth (frame_gen `ObjectMotion`).
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        view: usize,
        kernel: &wgpu::BindGroup,
        depth: &wgpu::TextureView,
        size: [u32; 2],
        jitter: [f32; 2],
        objects: Option<&wgpu::TextureView>,
    ) {
        let i = self.view_guides(device, view, size);
        let v = &self.views[i];
        let params = Params {
            jitter: [
                jitter[0],
                jitter[1],
                f32::from(u8::from(objects.is_some())),
                0.0,
            ],
        };
        uploads.buffer(encoder, &v.params, 0, bytemuck::bytes_of(&params));
        let entry = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RT guides"),
            layout: &self.layout,
            entries: &[
                entry(0, &v.guides.diffuse_albedo),
                entry(1, &v.guides.specular_albedo),
                entry(2, &v.guides.normals),
                entry(3, &v.guides.motion),
                entry(4, &v.show),
                entry(5, depth),
                entry(6, objects.unwrap_or(&self.no_objects)),
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: v.params.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("RT guides"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, kernel, &[]);
        pass.set_bind_group(1, &group, &[]);
        pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1);
        drop(pass);
        super::gpu_passes::stamp(encoder, "rays: DLSS-RR guides");
    }

    /// View `view`'s guides, once written.
    pub fn guides(&self, view: usize) -> Option<&Guides> {
        self.views
            .iter()
            .find(|v| v.view == view)
            .map(|v| &v.guides)
    }

    /// Draws `HORIZON_RT_GUIDES_SHOW`'s picture of view `view` over all of `target` (stretched to
    /// it); nothing without one.
    pub fn draw_shown(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        view: usize,
        target: &wgpu::Texture,
    ) {
        if show() == Show::None {
            return;
        }
        let Some(source) = self
            .guides(view)
            .map(|g| g.show.create_view(&Default::default()))
        else {
            return;
        };
        // The frame's passes write the window through its non-sRGB view (`readback::draw_onto`):
        // the picture's values go there as they are, already encoded for the screen.
        let format = target.format().remove_srgb_suffix();
        let target = target.create_view(&wgpu::TextureViewDescriptor {
            format: Some(format),
            ..Default::default()
        });
        if self.blitter.as_ref().is_none_or(|b| b.0 != format) {
            self.blitter = Some((format, wgpu::util::TextureBlitter::new(device, format)));
        }
        if let Some((_, blitter)) = &self.blitter {
            blitter.copy(device, encoder, &source, &target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validated(source: &str) -> (naga::Module, naga::valid::ModuleInfo) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("the guides: {}", e.emit_to_string(source)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("the guides: {e:?}"));
        (module, info)
    }

    #[test]
    fn the_guides_pass_builds_for_every_picture_and_skip() {
        // The kernel with the guides after it parses, validates, and writes SPIR-V (what Vulkan
        // runs) for `cs_guides` with the constants `pipeline` sets, for each picture.
        let source = source(2048);
        let (module, info) = validated(&source);
        for (show, skip) in [
            (0, 64),
            (1, 0),
            (2, 64),
            (3, 64),
            (4, 64),
            (5, 64),
            (6, u32::MAX),
            (7, 64),
        ] {
            let mut constants = naga::back::PipelineConstants::default();
            constants.insert("GUIDES_SHOW".to_string(), f64::from(show));
            constants.insert("SKIP".to_string(), f64::from(skip));
            let (m, i) = naga::back::pipeline_constants::process_overrides(
                &module,
                &info,
                Some((naga::ShaderStage::Compute, "cs_guides")),
                &constants,
            )
            .unwrap_or_else(|e| panic!("the guides' constants, show {show}: {e}"));
            naga::back::spv::write_vec(
                &m,
                &i,
                &naga::back::spv::Options::default(),
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: naga::ShaderStage::Compute,
                    entry_point: "cs_guides".to_string(),
                }),
            )
            .unwrap_or_else(|e| panic!("the guides in SPIR-V, show {show}: {e}"));
        }
    }

    #[test]
    fn the_guides_write_their_own_group_in_the_formats_the_textures_have() {
        // Group 1 is the guides' alone (the kernel's layout is group 0): each storage binding in
        // the format `RtGuides` makes its texture in (`DIFFUSE_FORMAT`..`SHOW_FORMAT`), then the
        // depth, the entities' motion and the uniform `RtGuides::new`'s layout declares: wgpu
        // refuses a mismatch only on the GPU.
        let (module, _) = validated(&source(2048));
        let mut found = Vec::new();
        for (_, var) in module.global_variables.iter() {
            let Some(binding) = &var.binding else {
                continue;
            };
            if binding.group != 1 {
                continue;
            }
            match &module.types[var.ty].inner {
                naga::TypeInner::Image {
                    class: naga::ImageClass::Storage { format, .. },
                    ..
                } => found.push((binding.binding, *format)),
                naga::TypeInner::Image {
                    class: naga::ImageClass::Depth { multi: false },
                    dim: naga::ImageDimension::D2,
                    ..
                } => assert_eq!(binding.binding, 5, "the depth"),
                naga::TypeInner::Image {
                    class:
                        naga::ImageClass::Sampled {
                            kind: naga::ScalarKind::Float,
                            multi: false,
                        },
                    dim: naga::ImageDimension::D2,
                    ..
                } => assert_eq!(binding.binding, 6, "the entities' motion"),
                naga::TypeInner::Struct { span, .. } => {
                    assert_eq!(binding.binding, 7, "the uniform");
                    assert_eq!(*span as usize, std::mem::size_of::<Params>());
                }
                other => panic!("unexpected binding in group 1: {other:?}"),
            }
        }
        found.sort_by_key(|(b, _)| *b);
        use naga::StorageFormat as F;
        assert_eq!(
            found,
            [
                (0, F::Rgba8Unorm),
                (1, F::Rgba8Unorm),
                (2, F::Rgba16Float),
                (3, F::Rg16Float),
                (4, F::Rgba8Unorm),
            ]
        );
    }

    #[test]
    fn the_picture_names_parse() {
        assert_eq!(Show::parse("albedo"), Show::Albedo);
        assert_eq!(Show::parse(" Normal "), Show::Normal);
        assert_eq!(Show::parse("roughness"), Show::Roughness);
        assert_eq!(Show::parse("motion"), Show::Motion);
        assert_eq!(Show::parse("specular"), Show::Specular);
        assert_eq!(Show::parse("source"), Show::Source);
        assert_eq!(Show::parse("depth"), Show::Depth);
        assert_eq!(Show::parse("nonsense"), Show::None);
    }
}
