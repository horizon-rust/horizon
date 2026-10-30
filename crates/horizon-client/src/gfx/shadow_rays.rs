//! Ray-traced shadows' and ambient occlusion's rays (docs/raytracing.md): after the world pass, a
//! compute pass traces, for each pixel of the view's depth, a ray toward the sun or the moon and
//! six short ones around the surface, through the voxel grid (`voxels`), and writes how much light
//! reaches into a mask of the depth's size. The tone mapping scales each
//! pixel by it, over the share of its light the solid terrain keeps in the shadow (the scene's
//! alpha, terrain.wgsl `sun_share`). Rays don't depend on what was drawn over the pixel, and the
//! window and every VR eye run the same pass on their own depth.

use super::upload::Uploads;
use glam::{DVec3, Mat4};

/// The views kept at once: the window (0), two eyes, two quad-view insets.
const KEPT: usize = 5;

/// Where the rays go, a view's choice: on the screen one a pixel (quality first); in VR one for
/// 2x2 pixels where the eyes look, about the density KRVR streams to the headset, and one for 8x8
/// away from it (shared by a 4x4 block of mask texels, shadow_rays.wgsl).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Foveation {
    /// Depth pixels a mask texel stands for, on each side.
    pub scale: u32,
    /// Where the eyes look (camera-relative, unit), and the cosine of the sharp zone's half
    /// angle; `None`: all of the view is sharp.
    pub gaze: Option<([f32; 3], f32)>,
}

/// The sharp zone's half angle, as VRS's (horizon-xr `foveation.rs`): a 12-degree gaze step on
/// each side stays sharp.
pub const SHARP_DEGREES: f32 = 25.0;

impl Foveation {
    /// The screen's: one ray a pixel, or `HORIZON_RT_SCALE`'s.
    pub fn screen() -> Self {
        static SCALE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        let scale = *SCALE.get_or_init(|| {
            std::env::var("HORIZON_RT_SCALE")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v| (1..=8).contains(v))
                .unwrap_or(1)
        });
        Self { scale, gaze: None }
    }

    /// A VR eye looking along `gaze`; `None` for a quad view's inset, sharp all over.
    pub fn eye(gaze: Option<glam::Vec3>) -> Self {
        // `HORIZON_RT_SHARP` (degrees) moves the sharp zone's edge, to measure what the foveation
        // saves: 0 traces sparsely everywhere, 180 sharp everywhere.
        static SHARP: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
        let sharp = *SHARP.get_or_init(|| {
            std::env::var("HORIZON_RT_SHARP")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v| (0.0..=180.0).contains(v))
                .unwrap_or(SHARP_DEGREES)
        });
        Self {
            scale: 2,
            gaze: gaze.map(|g| {
                (
                    g.normalize_or(glam::Vec3::NEG_Z).to_array(),
                    sharp.to_radians().cos(),
                )
            }),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    inverse_view_proj: [[f32; 4]; 4],
    light: [f32; 4],
    camera: [f32; 4],
    fraction: [f32; 4],
    grid: [f32; 4],
    size: [f32; 4],
    fovea: [f32; 4],
    mode: [f32; 4],
    prev_view_proj: [[f32; 4]; 4],
    motion: [f32; 4],
    sun: [f32; 4],
    frame: [f32; 4],
    sun_light: [f32; 4],
    water: [f32; 4],
    hw_offset: [f32; 4],
    held: [f32; 4],
    moving: [[f32; 4]; MOVING_LIGHTS],
}

/// The moving lights the rays light the terrain with at most (`Rays::moving`).
pub const MOVING_LIGHTS: usize = 8;

/// This frame's rays (`Renderer::ray_traced_shadows`): toward the light and the strength (0: no
/// shadows); the camera's block (and the Y above the grid's highest block) and its place in it;
/// the grid's first block (x, z), width and budget; the ambient occlusion's strength (0: off).
#[derive(Clone, Copy, Default)]
pub struct Rays {
    pub light: [f32; 4],
    pub camera: [f32; 4],
    pub fraction: [f32; 4],
    pub grid: [f32; 4],
    pub ambient: f32,
    /// The sun's shadows are traced (else its light only depends on the faces' facing).
    pub shadows: bool,
    /// RT lighting: the mask also holds each pixel's distance (a), for the terrain to read it the
    /// next frame (terrain.wgsl `rt_visibility`).
    pub lighting: bool,
    /// RT lighting: toward the real sun, w 1 when the sky table (`atmosphere`) holds its sky.
    pub sun: [f32; 4],
    /// RT lighting: the light traced toward (the sun's or the moon's), for the bounced light.
    pub sun_light: [f32; 4],
    /// The water's sprite layers and the time, for the caustics.
    pub water: [f32; 4],
    /// The camera's offset from the hardware rays' instances' anchor (`HwRays::offset`).
    pub hw_offset: [f32; 4],
    /// RT lighting's moving lights (a held torch, a burning mob, a dropped lantern: Remix's held
    /// light and its dynamic lights): camera-relative position and light level, the first
    /// `moving_count` of them.
    pub moving: [[f32; 4]; MOVING_LIGHTS],
    pub moving_count: usize,
    /// The Eye of Aldhani (`aldhani`): the seconds into it (below 0 none) and a full moon's light
    /// overhead, its meteors' light's unit.
    pub aldhani: [f32; 2],
}

impl Rays {
    /// Whether the pass has anything to trace.
    pub fn any(&self) -> bool {
        self.lighting || (self.shadows && self.light[3] > 0.0) || self.ambient > 0.0
    }
}

/// A view's last mask, for the terrain to light the next frame with (RT lighting): what it was
/// seen through and from.
pub struct History<'a> {
    pub mask: &'a wgpu::TextureView,
    pub view_proj: Mat4,
    pub camera: DVec3,
}

/// A view's two masks, written in turn: one is traced while the terrain reads the other.
struct ViewMasks {
    view: usize,
    size: [u32; 2],
    masks: [wgpu::TextureView; 2],
    /// RT lighting's sky and bounced light, accumulated, one a mask.
    blocks: [wgpu::TextureView; 2],
    /// RT lighting's light sources' direct light and faces (this frame's), the denoiser's half-way
    /// texture, and its output one a mask: what the terrain reads, and the last one the bounce
    /// rays read.
    direct: [wgpu::TextureView; 2],
    scratch: wgpu::TextureView,
    /// RT lighting's sun through what tints it, one a mask.
    sun: [wgpu::TextureView; 2],
    /// RT lighting's ReSTIR DI reservoirs, one a mask.
    reservoirs: [wgpu::TextureView; 2],
    /// RT lighting's ReSTIR GI reservoirs (the sample's point and weight; its light, count and
    /// normal), one a mask.
    gi: [[wgpu::TextureView; 2]; 2],
    /// RT lighting's fast history (ReLAX's), one a mask.
    fast: [wgpu::TextureView; 2],
    /// RT lighting's water reflections and refractions (Remix's primary surface replacement).
    reflection: wgpu::TextureView,
    refraction: wgpu::TextureView,
    light: [wgpu::TextureView; 2],
    /// The mask the next pass writes.
    next: usize,
    /// The last pass's view projection and camera.
    last: Option<(Mat4, DVec3)>,
}

/// RT lighting's hybrid's (`hw_rays`) for the shadow rays: the terrain's top-level structure,
/// the vertex arena's first page and the first sprite array, for their alpha test.
pub struct Hardware<'a> {
    pub tlas: &'a wgpu::Tlas,
    pub arena: &'a wgpu::Buffer,
    pub sprites: &'a wgpu::TextureView,
    /// Where each instance's quads are (`hw_rays::HwRays::instances`).
    pub instances: &'a wgpu::Buffer,
    /// The entities' and the view model's vertices (`hw_rays::HwRays::dynamics`) and the shared
    /// atlas their texels are in, for the alpha test and the hits' colour.
    pub dynamics: &'a wgpu::Buffer,
    pub atlas: &'a wgpu::TextureView,
}

/// The hybrid's part of the shader (`hw_rays`): the sun's and the light sources' shadow rays
/// against the drawn terrain, alpha-tested; without hardware rays, `hw_light` says to use the
/// voxel grid's.
const HARDWARE: &str = concat!(
    include_str!("shadow_rays_hw.wgsl"),
    include_str!("shadow_rays_psr.wgsl")
);
const NO_HARDWARE: &str = "
const HW: bool = false;
fn hw_light(pos: vec3<f32>, l: vec3<f32>, reach: f32) -> vec3<f32> { return vec3(-1.0); }
fn hw_hit(pos: vec3<f32>, l: vec3<f32>, reach: f32) -> HwHit {
    return HwHit(-1.0, vec3(0.0), vec3(0.0), vec3(1.0));
}
fn hw_reflect(p: vec2<i32>) -> vec4<f32> { return vec4(0.0); }
fn hw_refract(p: vec2<i32>) -> vec4<f32> { return vec4(0.0); }
";

/// The pass's shader, with the hybrid's hardware rays or without (`ShadowRays::new`).
pub fn source(hardware: bool, split: u32) -> String {
    if hardware {
        format!(
            "enable wgpu_ray_query;\nconst SPRITE_SPLIT: u32 = {split}u;\n{}{HARDWARE}",
            include_str!("shadow_rays.wgsl")
        )
    } else {
        format!("{}{NO_HARDWARE}", include_str!("shadow_rays.wgsl"))
    }
}

/// RT lighting's radiance cache's cells (a power of two).
const CACHE_CELLS: usize = 1 << 20;

/// The denoiser's passes' tap spacings (denoise.wgsl), the last adding the direct light.
const DENOISE_STEPS: [i32; 4] = [1, 2, 4, 8];

pub struct ShadowRays {
    /// RT lighting's denoiser (denoise.wgsl): its pipeline and layout, a uniform a pass.
    denoise: wgpu::ComputePipeline,
    denoise_layout: wgpu::BindGroupLayout,
    denoise_params: [wgpu::Buffer; 4],
    /// RT lighting's radiance cache (shadow_rays.wgsl `CacheCell`, radiance_cache.wgsl), shared by
    /// every view, and its resolve's pipeline and group.
    cache: wgpu::Buffer,
    resolve: wgpu::ComputePipeline,
    resolve_group: wgpu::BindGroup,
    /// Whether the pipeline traces hardware rays (`Hardware` is then bound).
    hardware: bool,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    uniform: wgpu::Buffer,
    /// The Eye of Aldhani's field (`aldhani::Aldhani::field`), its meteors the rays light with.
    aldhani: wgpu::Buffer,
    views: Vec<ViewMasks>,
    /// Passes run, for the rays' directions to change.
    frame: u32,
}

impl ShadowRays {
    /// The pass, with the hybrid's hardware rays when `hardware` (sprite layers past `split` go to
    /// the next arrays, which the alpha test takes as opaque); `aldhani` the Eye of Aldhani's field.
    pub fn new(device: &wgpu::Device, hardware: bool, split: u32, aldhani: &wgpu::Buffer) -> Self {
        let source = source(hardware, split);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Ray-traced shadows"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let grid = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D3,
            multisampled: false,
        };
        let output = wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba16Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let last = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let cache_binding = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let storage = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ray-traced shadows"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(2, grid),
                entry(3, grid),
                entry(4, grid),
                entry(5, grid),
                entry(6, output),
                entry(7, storage),
                entry(8, storage),
                entry(9, output),
                entry(
                    10,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(11, last),
                entry(12, last),
                entry(13, grid),
                entry(14, storage),
                entry(18, output),
                entry(19, last),
                entry(20, last),
                entry(21, output),
                entry(22, last),
                entry(
                    24,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
                entry(
                    25,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(26, cache_binding),
                entry(
                    27,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
                entry(
                    28,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
                entry(29, last),
                entry(
                    30,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(31, output),
                entry(32, last),
                entry(33, output),
                entry(34, output),
                entry(38, cache_binding),
                entry(39, storage),
            ]
            .into_iter()
            .chain(
                hardware
                    .then(|| {
                        [
                            entry(
                                15,
                                wgpu::BindingType::AccelerationStructure {
                                    vertex_return: false,
                                },
                            ),
                            entry(16, storage),
                            entry(23, storage),
                            entry(36, storage),
                            entry(
                                17,
                                wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2Array,
                                    multisampled: false,
                                },
                            ),
                            entry(
                                37,
                                wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                            ),
                        ]
                    })
                    .into_iter()
                    .flatten(),
            )
            .collect::<Vec<_>>(),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Ray-traced shadows"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Ray-traced shadows"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_shadow"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &[
                    (
                        "DEBUG",
                        std::env::var("HORIZON_RT_DEBUG")
                            .ok()
                            .and_then(|v| v.parse::<u32>().ok())
                            .filter(|v| *v >= 3)
                            .map_or(0.0, f64::from),
                    ),
                    (
                        "SKIP",
                        std::env::var("HORIZON_RT_SKIP")
                            .ok()
                            .and_then(|v| v.parse::<u32>().ok())
                            .map_or(0.0, f64::from),
                    ),
                ],
                ..Default::default()
            },
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ray-traced shadows"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let denoise_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RT lighting's denoiser"),
            source: wgpu::ShaderSource::Wgsl(include_str!("denoise.wgsl").into()),
        });
        let read = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let denoise_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT lighting's denoiser"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(1, read),
                entry(2, read),
                entry(3, read),
                entry(4, output),
                entry(5, read),
                entry(6, read),
            ],
        });
        let denoise = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("RT lighting's denoiser"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("RT lighting's denoiser"),
                    bind_group_layouts: &[Some(&denoise_layout)],
                    ..Default::default()
                }),
            ),
            module: &denoise_shader,
            entry_point: Some("cs_denoise"),
            compilation_options: Default::default(),
            cache: None,
        });
        let denoise_params = std::array::from_fn(|i| {
            use wgpu::util::DeviceExt;
            let last = i32::from(i == DENOISE_STEPS.len() - 1);
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("RT lighting's denoiser"),
                contents: bytemuck::cast_slice(&[DENOISE_STEPS[i], last, 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        });
        // RT lighting's radiance cache: 2^20 cells of 36 bytes, every light word starting as "none".
        let cache = {
            use wgpu::util::DeviceExt;
            let mut cells = vec![0u32; CACHE_CELLS * 9];
            for cell in cells.chunks_mut(9) {
                cell[5..8].fill(u32::MAX);
            }
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("RT lighting's radiance cache"),
                contents: bytemuck::cast_slice(&cells),
                usage: wgpu::BufferUsages::STORAGE,
            })
        };
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT lighting's radiance cache"),
            entries: &[entry(0, cache_binding)],
        });
        let resolve = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("RT lighting's radiance cache"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("RT lighting's radiance cache"),
                    bind_group_layouts: &[Some(&resolve_layout)],
                    ..Default::default()
                }),
            ),
            module: &device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("RT lighting's radiance cache"),
                source: wgpu::ShaderSource::Wgsl(include_str!("radiance_cache.wgsl").into()),
            }),
            entry_point: Some("cs_resolve"),
            compilation_options: Default::default(),
            cache: None,
        });
        let resolve_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RT lighting's radiance cache"),
            layout: &resolve_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cache.as_entire_binding(),
            }],
        });
        Self {
            cache,
            resolve,
            resolve_group,
            denoise,
            denoise_layout,
            denoise_params,
            hardware,
            layout,
            pipeline,
            uniform,
            aldhani: aldhani.clone(),
            views: Vec::new(),
            frame: 0,
        }
    }

    /// The mask view `view` (0 the window, then the VR views) traced last.
    pub fn history(&self, view: usize) -> Option<History<'_>> {
        let v = self.views.iter().find(|v| v.view == view)?;
        let (view_proj, camera) = v.last?;
        Some(History {
            mask: &v.masks[1 - v.next],
            view_proj,
            camera,
        })
    }

    fn masks(&mut self, device: &wgpu::Device, view: usize, size: [u32; 2]) -> &mut ViewMasks {
        let i = match self.views.iter().position(|v| v.view == view) {
            Some(i) if self.views[i].size == size => i,
            found => {
                if let Some(i) = found {
                    self.views.remove(i);
                } else if self.views.len() == KEPT {
                    self.views.remove(0);
                }
                let mask = || Self::mask(device, size);
                self.views.push(ViewMasks {
                    view,
                    size,
                    masks: [mask(), mask()],
                    blocks: [mask(), mask()],
                    direct: [mask(), mask()],
                    sun: [mask(), mask()],
                    reservoirs: [0, 1].map(|_| Self::reservoir(device, size)),
                    gi: [0, 1].map(|_| {
                        [
                            Self::texture(device, size, wgpu::TextureFormat::Rgba32Float),
                            Self::reservoir(device, size),
                        ]
                    }),
                    scratch: mask(),
                    fast: [mask(), mask()],
                    reflection: mask(),
                    refraction: mask(),
                    light: [mask(), mask()],
                    next: 0,
                    last: None,
                });
                self.views.len() - 1
            }
        };
        &mut self.views[i]
    }

    fn reservoir(device: &wgpu::Device, size: [u32; 2]) -> wgpu::TextureView {
        Self::texture(device, size, wgpu::TextureFormat::Rgba32Uint)
    }

    fn texture(
        device: &wgpu::Device,
        size: [u32; 2],
        format: wgpu::TextureFormat,
    ) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("RT lighting's light reservoirs"),
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
            .create_view(&Default::default())
    }

    fn mask(device: &wgpu::Device, size: [u32; 2]) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Ray-traced light mask"),
                size: wgpu::Extent3d {
                    width: size[0].max(1),
                    height: size[1].max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }

    /// Traces view `view` (0 the window, then the VR views), whose depth is `depth` (`size`, seen
    /// through `view_proj` from `camera`), as `foveation` says, and returns its mask and the light
    /// sources' light and the sun through what tints it (RT lighting). `grid` is the voxel grid's views (`Voxels::views`), `lights`
    /// its light sources and cells (`Voxels::light_buffers`).
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        view: usize,
        depth: &wgpu::TextureView,
        size: [u32; 2],
        view_proj: Mat4,
        camera: DVec3,
        rays: &Rays,
        foveation: Foveation,
        grid: [&wgpu::TextureView; 5],
        lights: [&wgpu::Buffer; 3],
        visibility: &wgpu::Buffer,
        sky: &wgpu::TextureView,
        hardware: Option<Hardware<'_>>,
    ) -> [wgpu::TextureView; 5] {
        let scale = foveation.scale;
        let mask_size = size.map(|v| v.div_ceil(scale));
        let masks = self.masks(device, view, mask_size);
        let mask = masks.masks[masks.next].clone();
        let blocks = masks.blocks[masks.next].clone();
        let last_mask = masks.masks[1 - masks.next].clone();
        let last_blocks = masks.blocks[1 - masks.next].clone();
        let direct = masks.direct[masks.next].clone();
        let last_direct = masks.direct[1 - masks.next].clone();
        let sun = masks.sun[masks.next].clone();
        let last_sun = masks.sun[1 - masks.next].clone();
        let reservoirs = masks.reservoirs[masks.next].clone();
        let last_reservoirs = masks.reservoirs[1 - masks.next].clone();
        let gi = masks.gi[masks.next].clone();
        let last_gi = masks.gi[1 - masks.next].clone();
        let fast = masks.fast[masks.next].clone();
        let last_fast = masks.fast[1 - masks.next].clone();
        let reflection = masks.reflection.clone();
        let refraction = masks.refraction.clone();
        let scratch = masks.scratch.clone();
        let light = masks.light[masks.next].clone();
        let last_light = masks.light[1 - masks.next].clone();
        let last = masks.last.replace((view_proj, camera));
        masks.next = 1 - masks.next;
        self.frame = self.frame.wrapping_add(1);
        let uniform = Uniform {
            inverse_view_proj: view_proj.inverse().to_cols_array_2d(),
            light: rays.light,
            camera: rays.camera,
            fraction: rays.fraction,
            grid: rays.grid,
            size: [size[0] as f32, size[1] as f32, scale as f32, rays.ambient],
            fovea: match foveation.gaze {
                Some(([x, y, z], cos)) => [x, y, z, cos],
                None => [0.0, 0.0, 0.0, -2.0],
            },
            mode: [
                f32::from(u8::from(rays.lighting)),
                f32::from(u8::from(rays.shadows)),
                0.0,
                0.0,
            ],
            prev_view_proj: last.map_or([[0.0; 4]; 4], |l| l.0.to_cols_array_2d()),
            motion: match last {
                Some((_, last_camera)) => {
                    let m = (camera - last_camera).as_vec3();
                    [m.x, m.y, m.z, 1.0]
                }
                None => [0.0; 4],
            },
            sun: rays.sun,
            frame: [(self.frame % 65536) as f32, 0.0, 0.0, 0.0],
            sun_light: rays.sun_light,
            water: rays.water,
            hw_offset: rays.hw_offset,
            held: [
                rays.moving_count.min(MOVING_LIGHTS) as f32,
                rays.aldhani[0],
                rays.aldhani[1],
                0.0,
            ],
            moving: rays.moving,
        };
        uploads.buffer(encoder, &self.uniform, 0, bytemuck::bytes_of(&uniform));
        let view = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        // Without the hybrid's this frame (no section yet), a pipeline that traces hardware rays
        // has nothing to bind: the pass waits.
        if self.hardware && hardware.is_none() {
            return [mask, blocks, sun, reflection, refraction];
        }
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniform.as_entire_binding(),
            },
            view(1, depth),
            view(2, grid[0]),
            view(3, grid[1]),
            view(4, grid[2]),
            view(5, grid[3]),
            view(6, &mask),
            wgpu::BindGroupEntry {
                binding: 7,
                resource: lights[0].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: lights[1].as_entire_binding(),
            },
            view(9, &blocks),
            view(10, sky),
            view(11, &last_mask),
            view(12, &last_blocks),
            view(13, grid[4]),
            wgpu::BindGroupEntry {
                binding: 14,
                resource: lights[2].as_entire_binding(),
            },
            view(18, &direct),
            view(19, &last_light),
            view(20, &last_direct),
            view(21, &sun),
            view(22, &last_sun),
            view(24, &reservoirs),
            view(25, &last_reservoirs),
            wgpu::BindGroupEntry {
                binding: 26,
                resource: self.cache.as_entire_binding(),
            },
            view(27, &gi[0]),
            view(28, &gi[1]),
            view(29, &last_gi[0]),
            view(30, &last_gi[1]),
            view(31, &fast),
            view(32, &last_fast),
            view(33, &reflection),
            view(34, &refraction),
            wgpu::BindGroupEntry {
                binding: 38,
                resource: visibility.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 39,
                resource: self.aldhani.as_entire_binding(),
            },
        ];
        if let (true, Some(hw)) = (self.hardware, &hardware) {
            entries.extend([
                wgpu::BindGroupEntry {
                    binding: 15,
                    resource: wgpu::BindingResource::AccelerationStructure(hw.tlas),
                },
                wgpu::BindGroupEntry {
                    binding: 16,
                    resource: hw.arena.as_entire_binding(),
                },
                view(17, hw.sprites),
                wgpu::BindGroupEntry {
                    binding: 23,
                    resource: hw.instances.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 36,
                    resource: hw.dynamics.as_entire_binding(),
                },
                view(37, hw.atlas),
            ]);
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Ray-traced shadows"),
            layout: &self.layout,
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Ray-traced shadows"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(mask_size[0].div_ceil(8), mask_size[1].div_ceil(8), 1);
        drop(pass);
        // One kernel traces the sun, the direct lights, the bounces and the mirrors at once.
        super::gpu_passes::stamp(encoder, "rays: trace (sun, DI, GI, PSR)");
        if !rays.lighting {
            return [mask, blocks, sun, reflection, refraction];
        }
        // The radiance cache: this pass's sums become its cells' light for the next views.
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("RT lighting's radiance cache"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.resolve);
            pass.set_bind_group(0, &self.resolve_group, &[]);
            pass.dispatch_workgroups((CACHE_CELLS as u32).div_ceil(64), 1, 1);
        }
        super::gpu_passes::stamp(encoder, "rays: radiance cache resolve");
        // RT lighting's denoiser: the sky and bounced light from `blocks` to `light` by way of
        // `scratch`, a pass a tap spacing, the direct light added by the last.
        let passes = [
            (&blocks, &scratch),
            (&scratch, &light),
            (&light, &scratch),
            (&scratch, &light),
        ];
        for ((from, to), params) in passes.into_iter().zip(&self.denoise_params) {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RT lighting's denoiser"),
                layout: &self.denoise_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    view(1, from),
                    view(2, &mask),
                    view(3, &direct),
                    view(4, to),
                    view(5, &blocks),
                    view(6, &sun),
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("RT lighting's denoiser"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.denoise);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(mask_size[0].div_ceil(8), mask_size[1].div_ceil(8), 1);
            drop(pass);
            super::gpu_passes::stamp(encoder, "rays: denoiser (4 passes)");
        }
        [mask, light, sun, reflection, refraction]
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_radiance_cache_cell_is_nine_words() {
        // The buffer is made with nine words a cell (`CACHE_CELLS * 9`), as both shaders lay it out.
        for source in [
            include_str!("radiance_cache.wgsl"),
            &super::source(false, 2048),
        ] {
            let module = naga::front::wgsl::parse_str(source).expect("the shader");
            let mut layouter = naga::proc::Layouter::default();
            layouter.update(module.to_ctx()).expect("its layout");
            let (cell, _) = module
                .types
                .iter()
                .find(|(_, t)| t.name.as_deref() == Some("CacheCell"))
                .expect("the cache's cells");
            assert_eq!(layouter[cell].size, 36);
        }
    }

    #[test]
    fn the_shaders_map_colours_are_the_blocks() {
        let source = include_str!("shadow_rays.wgsl");
        let body = source
            .split("const MAP_COLORS: array<u32, 62> = array<u32, 62>(")
            .nth(1)
            .and_then(|b| b.split(");").next())
            .expect("the palette");
        let colours: Vec<u32> = body
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.trim_end_matches('u').parse().expect("a colour"))
            .collect();
        assert_eq!(colours, crate::blocks::MAP_COLORS);
    }
}
