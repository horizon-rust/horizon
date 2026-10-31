//! Linear Light (Horizon setting, docs/graphics.md): the world, the hand and the screen effects
//! draw in linear light into a float scene target of the view's size (the window, or each VR
//! eye), and one pass tone maps it onto the screen's 8-bit target, under the HUD. Light sources
//! can go above 1.0 there (lava, the sun), the base the PBR and ray tracing work in
//! (docs/raytracing.md). Without it the frame is drawn as vanilla's, straight to the screen.
//!
//! A shader pack's last pass writes gamma-space 8-bit colour: it goes to `scratch`, which
//! `decode` takes back into the scene before the overlays.
//!
//! With RT lighting (`set_post`) the scene is graded before the curve, as a camera would: the
//! auto-exposure (`exposure`) scales its light to the eye's adaptation and the bloom (`bloom`)
//! lets the brightest of it glow. Both are measured on a chain of halved copies of the scene,
//! and the grading pass writes the result into a float target of its own (`Post`), which the
//! curve reads and HDR output reads in place of the scene (`graded`). Env switches, for A/B:
//! `HORIZON_POST=0` (no grading), `HORIZON_EXPOSURE=0` (the exposure stays 1; a number is
//! a fixed value in stops), `HORIZON_BLOOM=0` (a number is the bloom's burn intensity) and
//! `HORIZON_TONEMAP=shoulder` (today's curve in place of ACES).
//!
//! The grading follows RTX Remix's post-processing and its rtx.conf values (`PostParams`): the
//! auto-exposure of `exposure`, a threshold bloom that is added to the scene (burn intensity),
//! a local tone mapping that fuses three exposures, and the non-legacy ACES curve.

use super::bloom::{self, Bloom};
use super::exposure::{Exposure, Meter};

/// What the post-processing is set by, at run time: Remix's rtx.conf values by default, and a
/// few situations' (`DUSK`, `NIGHT`, `RAIN`, `THUNDER`) for the caller to switch between with
/// `Tonemap::set_params`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostParams {
    /// The auto-exposure's limits, in stops.
    pub ev_min: f32,
    pub ev_max: f32,
    /// The bloom's burn intensity: how much of its glow is added to the scene.
    pub bloom_burn: f32,
    /// The bloom's luminance threshold in Remix's scene units: light below it does not glow.
    /// It is converted with `REMIX_UNITS`.
    pub bloom_threshold: f32,
    /// The local tone mapping's exposure, shadows, highlights, exposure preference sigma and
    /// offset (Remix's `rtx.localtonemap.*`).
    pub exposure: f32,
    pub shadows: f32,
    pub highlights: f32,
    pub sigma: f32,
    pub offset: f32,
}

/// Remix's units of luminance in paper white's: Remix lights a scene in units where a lit surface
/// reads about 200 before its exposure, so the bloom threshold of 250 starts just above white
/// (1.25 paper whites: the sun, lava and lamps glow, a lit wall does not). A measured guess.
const REMIX_UNITS: f32 = 200.0;
/// The local tone mapping exposure at which Remix's own scale is the scene's, 1.
const REMIX_EXPOSURE: f32 = 0.214_057_28;

impl PostParams {
    /// Remix's rtx.conf values.
    pub const REMIX: Self = Self {
        ev_min: -2.5,
        ev_max: 1.5,
        bloom_burn: 0.1,
        bloom_threshold: 250.0,
        exposure: REMIX_EXPOSURE,
        shadows: 1.4,
        highlights: 2.5,
        sigma: 3.5,
        offset: -0.1,
    };
    /// Remix's dusk: the exposure's top lowered to -0.5 stops.
    pub const DUSK: Self = Self {
        ev_max: -0.5,
        ..Self::REMIX
    };
    /// Remix's night: the top at -0.5, a bloom that burns at 1.5 over anything above 1.
    pub const NIGHT: Self = Self {
        ev_max: -0.5,
        bloom_burn: 1.5,
        bloom_threshold: 1.0,
        ..Self::REMIX
    };
    /// Remix's rain: the top at -1, shadows at 1.75.
    pub const RAIN: Self = Self {
        ev_max: -1.0,
        shadows: 1.75,
        ..Self::REMIX
    };
    /// Remix's thunder: the top at -1, shadows at 2.
    pub const THUNDER: Self = Self {
        ev_max: -1.0,
        shadows: 2.0,
        ..Self::REMIX
    };

    /// The uniform the grading shaders read (tonemap.wgsl `Params`), with the time step.
    fn words(&self, dt: f32, burn: Option<f32>) -> [f32; 16] {
        [
            dt,
            self.ev_max,
            self.ev_min,
            0.0,
            burn.unwrap_or(self.bloom_burn),
            self.bloom_threshold / REMIX_UNITS,
            0.0,
            0.0,
            self.exposure / REMIX_EXPOSURE,
            self.shadows,
            self.highlights,
            self.sigma,
            self.offset,
            0.0,
            0.0,
            0.0,
        ]
    }
}

/// The scene target's format.
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Different sizes kept at once: the window, two eyes, two quad-view insets.
const KEPT: usize = 5;
/// Remix's bloom steps, the levels of its chain; the exposure's chain has 6.
const BLOOM_STEPS: u32 = 8;

struct Target {
    size: [u32; 2],
    view: wgpu::TextureView,
    group: wgpu::BindGroup,
    post: Option<Post>,
}

/// A scene size's grading: its chains and the graded light.
struct Post {
    /// The scene's light, unthresholded: the exposure is measured on it and the local tone
    /// mapping reads its average.
    chain: bloom::Chain,
    /// The bloom's: the light over the threshold, blurred.
    glow: bloom::Chain,
    /// The grading pass's target, and the curve's source (the tone mapping layout, no mask).
    graded: wgpu::TextureView,
    /// Its texture, for an F2 screenshot's HDR copy (`graded_texture`).
    graded_texture: wgpu::Texture,
    graded_group: wgpu::BindGroup,
    /// The glow and the exposure, for the grading pass.
    group: wgpu::BindGroup,
}

/// What grading needs, made when RT lighting first draws.
struct Grading {
    bloom: Bloom,
    exposure: Exposure,
    /// One exposure for every scene size: the two eyes, and in quad views the insets and the
    /// wide views, all read it, so the views of one frame never differ in brightness at their
    /// seams. Each size's run measures its own scene and moves it a little.
    meter: Meter,
    /// `PostParams::words`, copied in at each run.
    params: wgpu::Buffer,
    /// The parameters and the exposure for bloom.wgsl's first level.
    first: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
    /// The curve of the graded light: ACES.
    aces: wgpu::RenderPipeline,
}

pub struct Tonemap {
    layout: wgpu::BindGroupLayout,
    /// Bound for the shadow mask without ray-traced shadows (1x1: no shadows, tonemap.wgsl).
    no_mask: wgpu::TextureView,
    tonemap: wgpu::RenderPipeline,
    decode: wgpu::RenderPipeline,
    shader: wgpu::ShaderModule,
    screen: wgpu::TextureFormat,
    scenes: Vec<Target>,
    scratch: Option<Target>,
    /// RT lighting draws: grade the scene (`set_post`).
    post: bool,
    params: PostParams,
    /// `HORIZON_POST_SITUATION` holds `params`.
    held: bool,
    /// The eye's night vision (`set_night_vision`).
    night_vision: bool,
    grading: Option<Grading>,
}

/// `HORIZON_RT_DEBUG=1` or `2` (tonemap.wgsl `DEBUG`), for developing ray-traced shadows; the
/// rays' own views (3, 4, 5, 7: shadow_rays.wgsl) show as the mask (1).
fn debug_constant() -> (&'static str, f64) {
    (
        "DEBUG",
        std::env::var("HORIZON_RT_DEBUG")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .map_or(0.0, |v| {
                f64::from(if (3..=5).contains(&v) || v == 7 { 1 } else { v })
            }),
    )
}

/// The bloom's burn intensity when `HORIZON_BLOOM` sets it (0 for none).
fn bloom_burn() -> Option<f32> {
    std::env::var("HORIZON_BLOOM")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map(|v| v.max(0.0))
}

impl Grading {
    fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        scene_layout: &wgpu::BindGroupLayout,
        screen: wgpu::TextureFormat,
    ) -> Self {
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon grading"),
            entries: &[
                entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    1,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    2,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    3,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    4,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Horizon grading"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon grading"),
            bind_group_layouts: &[Some(scene_layout), Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fs_grade"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_full"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_grade"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[debug_constant()],
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format: SCENE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let exposure = Exposure::new(device);
        let meter = exposure.meter(device);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon post-processing parameters"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bloom = Bloom::new(device, scene_layout);
        let first = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon bloom's parameters"),
            layout: bloom.params_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: meter.state().as_entire_binding(),
                },
            ],
        });
        // ACES reads the graded light, which has no mask left.
        let aces_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon ACES"),
            bind_group_layouts: &[Some(scene_layout)],
            ..Default::default()
        });
        let aces = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fs_tonemap_aces"),
            layout: Some(&aces_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_full"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_tonemap_aces"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: screen,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            bloom,
            meter,
            exposure,
            params,
            first,
            layout,
            sampler,
            pipeline,
            aces,
        }
    }
}

impl Tonemap {
    /// Tone mapping onto targets of `screen`'s format.
    pub fn new(device: &wgpu::Device, screen: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon tone mapping"),
            source: wgpu::ShaderSource::Wgsl(include_str!("tonemap.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon tone mapping"),
            entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }),
        });
        let no_mask = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("No ray-traced light"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon tone mapping"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let debug = [debug_constant()];
        let pipeline = |entry: &str, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
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
                        constants: &debug,
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            tonemap: pipeline("fs_tonemap", screen),
            decode: pipeline("fs_decode", SCENE_FORMAT),
            layout,
            no_mask,
            shader,
            screen,
            scenes: Vec::new(),
            scratch: None,
            post: false,
            // `HORIZON_POST_SITUATION=dusk|night|rain|thunder` holds a situation's values, to
            // look at one whatever the sky (`set_params` then changes nothing).
            held: std::env::var_os("HORIZON_POST_SITUATION").is_some(),
            night_vision: true,
            params: match std::env::var("HORIZON_POST_SITUATION").as_deref() {
                Ok("dusk") => PostParams::DUSK,
                Ok("night") => PostParams::NIGHT,
                Ok("rain") => PostParams::RAIN,
                Ok("thunder") => PostParams::THUNDER,
                _ => PostParams::REMIX,
            },
            grading: None,
        }
    }

    /// Grades the scene before the curve (RT lighting): exposure and bloom, tonemap.rs's header.
    /// The eye's night vision on or off (tonemap.wgsl `night_vision`), from the next frame.
    pub fn set_night_vision(&mut self, on: bool) {
        self.night_vision = on;
    }
    pub fn set_post(&mut self, on: bool) {
        self.post = on && std::env::var("HORIZON_POST").as_deref() != Ok("0");
    }

    /// What the grading is set by (`PostParams::REMIX`, or a situation's), from the next frame.
    pub fn set_params(&mut self, params: PostParams) {
        if !self.held {
            self.params = params;
        }
    }

    /// The graded light of the scene of `size`, drawn by the last `run`: where HDR output reads
    /// the scene when it is graded (its shadows applied: bind `no_mask` with it).
    pub fn graded(&self, size: [u32; 2]) -> Option<&wgpu::TextureView> {
        if !self.post {
            return None;
        }
        let t = self.scenes.iter().find(|t| t.size == size)?;
        t.post.as_ref().map(|p| &p.graded)
    }

    /// The graded light's texture of the scene of `size` (`graded`'s), for a screenshot.
    pub fn graded_texture(&self, size: [u32; 2]) -> Option<&wgpu::Texture> {
        if !self.post {
            return None;
        }
        let t = self.scenes.iter().find(|t| t.size == size)?;
        t.post.as_ref().map(|p| &p.graded_texture)
    }

    /// The grading of the scene at `index` (made on first use).
    fn prepare(&mut self, device: &wgpu::Device, index: usize) {
        let layout = &self.layout;
        let grading = self
            .grading
            .get_or_insert_with(|| Grading::new(device, &self.shader, layout, self.screen));
        if self.scenes[index].post.is_some() {
            return;
        }
        let size = self.scenes[index].size;
        let graded_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Horizon scene (graded)"),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_FORMAT,
            // COPY_SRC: an F2 screenshot keeps the graded light as it is (gfx.rs `copy_graded`).
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let graded = graded_texture.create_view(&Default::default());
        let chain = grading.bloom.chain(device, size, 6, 3);
        let glow = grading.bloom.chain(device, size, BLOOM_STEPS, 1);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon grading"),
            layout: &grading.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(glow.glow()),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: grading.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(chain.level(2)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&grading.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: grading.meter.state().as_entire_binding(),
                },
            ],
        });
        let graded_group = self.group(device, &graded, &self.no_mask);
        self.scenes[index].post = Some(Post {
            chain,
            glow,
            graded,
            graded_texture,
            graded_group,
            group,
        });
    }

    fn target(&self, device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat) -> Target {
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(if format == SCENE_FORMAT {
                    "Horizon scene (linear light)"
                } else {
                    "Horizon shader pack output"
                }),
                size: wgpu::Extent3d {
                    width: size[0].max(1),
                    height: size[1].max(1),
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
            .create_view(&Default::default());
        let group = self.group(device, &view, &self.no_mask);
        Target {
            size,
            view,
            group,
            post: None,
        }
    }

    fn group(
        &self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        mask: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon tone mapping"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(mask),
                },
            ],
        })
    }

    /// The mask bound without ray-traced shadows.
    pub fn no_mask(&self) -> &wgpu::TextureView {
        &self.no_mask
    }

    /// The scene target of `size` (made on first use).
    pub fn scene(&mut self, device: &wgpu::Device, size: [u32; 2]) -> wgpu::TextureView {
        let index = match self.scenes.iter().position(|t| t.size == size) {
            Some(index) => index,
            None => {
                if self.scenes.len() == KEPT {
                    self.scenes.remove(0);
                }
                let t = self.target(device, size, SCENE_FORMAT);
                self.scenes.push(t);
                self.scenes.len() - 1
            }
        };
        if self.post {
            self.prepare(device, index);
        }
        self.scenes[index].view.clone()
    }

    /// The 8-bit target a shader pack's last pass draws into, of `size`.
    pub fn scratch(&mut self, device: &wgpu::Device, size: [u32; 2]) -> wgpu::TextureView {
        if self.scratch.as_ref().is_none_or(|t| t.size != size) {
            self.scratch = Some(self.target(device, size, self.screen));
        }
        self.scratch
            .as_ref()
            .map(|t| t.view.clone())
            .expect("just made")
    }

    fn draw(
        encoder: &mut wgpu::CommandEncoder,
        label: &str,
        pipeline: &wgpu::RenderPipeline,
        groups: &[&wgpu::BindGroup],
        target: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        for (i, group) in groups.iter().enumerate() {
            pass.set_bind_group(i as u32, *group, &[]);
        }
        pass.draw(0..3, 0..1);
    }

    /// Tone maps the scene of `size` onto `target` (the screen's format, the same size), its
    /// solid terrain scaled by the ray-traced shadows' `mask` when there is one. With `set_post`
    /// the scene is graded first (and the mask applied then).
    pub fn run(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        target: &wgpu::TextureView,
        mask: Option<&wgpu::TextureView>,
    ) {
        let Some(t) = self.scenes.iter().find(|t| t.size == size) else {
            return;
        };
        let scene = match mask {
            Some(mask) => &self.group(device, &t.view, mask),
            None => &t.group,
        };
        if let (true, Some(post), Some(grading)) = (self.post, &t.post, &self.grading) {
            // The parameters are copied in with the commands, not written ahead of them: in VR
            // an eye's frame can still be queued when the other's is encoded.
            let mut words = self.params.words(grading.meter.step(), bloom_burn());
            words[3] = f32::from(u8::from(self.night_vision));
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Horizon post-processing parameters"),
                size: 64,
                usage: wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: true,
            });
            staging
                .slice(..)
                .get_mapped_range_mut()
                .expect("mapped at creation")
                .copy_from_slice(bytemuck::cast_slice(&words));
            staging.unmap();
            encoder.copy_buffer_to_buffer(&staging, 0, &grading.params, 0, 64);
            // The exposure is measured on the scene's light halved a few times, then the bloom
            // keeps what is over its threshold, after the exposure it has just adapted.
            grading
                .bloom
                .down(encoder, &post.chain, scene, &grading.first, false);
            super::gpu_passes::stamp(encoder, "exposure: meter (downsample)");
            grading.exposure.adapt(
                device,
                encoder,
                &grading.meter,
                post.chain.meter(),
                &grading.params,
            );
            super::gpu_passes::stamp(encoder, "exposure: adapt");
            grading
                .bloom
                .down(encoder, &post.glow, scene, &grading.first, true);
            grading.bloom.up(encoder, &post.glow);
            super::gpu_passes::stamp(encoder, "bloom");
            Self::draw(
                encoder,
                "Grading",
                &grading.pipeline,
                &[scene, &post.group],
                &post.graded,
            );
            super::gpu_passes::stamp(encoder, "grading (local tone mapping)");
            static SHOULDER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            let shoulder = *SHOULDER
                .get_or_init(|| std::env::var("HORIZON_TONEMAP").as_deref() == Ok("shoulder"));
            Self::draw(
                encoder,
                "Tone mapping",
                if shoulder {
                    &self.tonemap
                } else {
                    &grading.aces
                },
                &[&post.graded_group],
                target,
            );
            super::gpu_passes::stamp(encoder, "tone curve (ACES)");
        } else {
            Self::draw(encoder, "Tone mapping", &self.tonemap, &[scene], target);
        }
    }

    /// Decodes `scratch` (a pack's gamma-space output) into `scene`.
    pub fn decode_scratch(&self, encoder: &mut wgpu::CommandEncoder, scene: &wgpu::TextureView) {
        if let Some(t) = &self.scratch {
            Self::draw(
                encoder,
                "Shader pack output to linear light",
                &self.decode,
                &[&t.group],
                scene,
            );
        }
    }
}
