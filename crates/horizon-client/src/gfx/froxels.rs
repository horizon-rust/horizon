//! Froxel volumetrics, as NVIDIA's RTX Remix has them (docs/raytracing.md, "Next: the state of the
//! art"): a homogeneous medium keyed to where the camera is (water, cave, rain, thunder, open air),
//! lit by the sun, the sky and the light sources in a grid of frustum voxels, per view.
//!
//! The grid: `x` by `y` froxels across the view (one per `TILE` pixels), `SLICES` slices along each
//! ray, exponential from `NEAR` to `FAR` blocks (1 block = 1 metre, Remix's `rtx.sceneScale` 0.01 over
//! centimetre units). Each frame, for every view (the window, each VR eye), `Froxels::run`:
//!
//! 1. *injects* (`cs_inject`): for each froxel a point drawn inside it (a 3D R-sequence shifted per
//!    froxel, so the history supersamples the froxel), the light the medium scatters toward the
//!    camera: the sun (a shadow ray, tinted by stained glass and water, times the Henyey-Greenstein
//!    phase), the sky (a ray toward it, times the sky's mean light) and one light source drawn by
//!    importance (a shadow ray), through hardware ray queries against the terrain's TLAS (`hw_rays`)
//!    where the GPU has them, else the voxel grid's DDA; then *blends* (`cs_blend`) that sample with
//!    the last frame's froxels reprojected in world space (the history `MAX_ACCUMULATION` frames
//!    long, `SETTLING_ACCUMULATION` for `SETTLING_FRAMES` after the medium changes or the view is
//!    new), the history shortened where its froxel moved a lot and clamped to the neighbourhood of
//!    the new samples;
//! 2. *integrates* (`cs_integrate`): front to back along each column, the in-scatter up to every
//!    slice (the transmittance is Beer-Lambert's, closed form for a homogeneous medium);
//! 3. *applies* (two full-screen draws onto the world's colour, before the tone mapping): the scene
//!    times the transmittance at the pixel's distance, plus the in-scatter there (read through a
//!    cubic B-spline across the screen).
//!
//! What the main session fills: `CameraMedium` (where the camera is) through `Renderer::
//! set_volumetrics`; `CameraMedium::medium` turns it into the medium.

use super::shadow_rays::{Hardware, Rays};
use super::upload::Uploads;
use glam::{DVec3, Mat4, Vec3};

/// Pixels a froxel spans on each side (Remix's `froxelGridResolutionScale` is 16).
pub const TILE: u32 = 16;
/// Slices along the rays.
pub const SLICES: u32 = 64;
/// Where the first slice ends and the last slice ends, in blocks (`froxelMaxDistanceMeters` 256).
pub const NEAR: f32 = 0.5;
pub const FAR: f32 = 256.0;
/// How many frames the history holds at most (Remix's default 128), and while settling
/// (`maxAccumulationFrames` 8), for how many frames after the medium changes or the view is new.
pub const MAX_ACCUMULATION: f32 = 128.0;
pub const SETTLING_ACCUMULATION: f32 = 8.0;
pub const SETTLING_FRAMES: u32 = 8;

/// A medium: how much light it absorbs and scatters, and where it scatters it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Medium {
    /// The extinction a block (a metre), per colour channel: `-ln(transmittance) / distance`.
    pub sigma_t: Vec3,
    /// The share of the extinction that scatters (Remix's `singleScatteringAlbedo`).
    pub albedo: Vec3,
    /// The phase's anisotropy (Henyey-Greenstein's g, -1 back to 1 forward).
    pub anisotropy: f32,
}

impl Medium {
    /// The medium that lets `transmittance` of the light through over `distance` blocks (Remix's
    /// `transmittanceColor` over `transmittanceMeasurementDistanceMeters`).
    pub fn measured(transmittance: Vec3, distance: f32, albedo: Vec3, anisotropy: f32) -> Self {
        Self {
            sigma_t: Vec3::new(
                -transmittance.x.clamp(1e-4, 1.0).ln(),
                -transmittance.y.clamp(1e-4, 1.0).ln(),
                -transmittance.z.clamp(1e-4, 1.0).ln(),
            ) / distance,
            albedo,
            anisotropy,
        }
    }

    /// Remix's water: `transmittanceColor` (0.6, 0.9, 0.89) over 20 m, `singleScatteringAlbedo`
    /// (0.206, 0.999, 0.903), anisotropy 0.4.
    pub fn water() -> Self {
        Self::measured(
            Vec3::new(0.6, 0.9, 0.89),
            20.0,
            Vec3::new(0.206, 0.999, 0.903),
            0.4,
        )
    }

    /// Remix's cave: albedo 0.9, transmittance 0.85 over 20 m, anisotropy -0.2 (`enableFogRemap`
    /// false: the distances are the real ones).
    pub fn cave() -> Self {
        Self::measured(Vec3::splat(0.85), 20.0, Vec3::splat(0.9), -0.2)
    }

    /// Remix's rain: transmittance 0.996 over 10 m. Its albedo and anisotropy aren't in the settings
    /// the task gave: water droplets scatter, forward a little (0.95, 0.3).
    pub fn rain() -> Self {
        Self::measured(Vec3::splat(0.996), 10.0, Vec3::splat(0.95), 0.3)
    }

    /// Remix's thunder: transmittance 0.97 over 10 m (the same albedo and anisotropy as the rain's,
    /// a little darker: 0.8).
    pub fn thunder() -> Self {
        Self::measured(Vec3::splat(0.97), 10.0, Vec3::splat(0.8), 0.3)
    }

    /// The open air with rain of `rate` mm/h falling through it: the air's medium plus the drops'
    /// own extinction (`rain::extinction`, Marshall-Palmer sizes; Remix's rain, 0.996 over 10 m,
    /// is about 1.5 mm/h of it and its thunder, 0.97, about 25 mm/h), scattering as the rain's
    /// medium does, a downpour's a little darker as Remix's thunder.
    pub fn falling(rate: f32) -> Self {
        let air = Self::air();
        let drops = crate::rain::extinction(rate);
        if drops <= 0.0 {
            return air;
        }
        let storm = (rate / crate::rain::THUNDER_MM_H).clamp(0.0, 1.0);
        let rain = Self::rain().mix(Self::thunder(), storm);
        let share = drops / (drops + air.sigma_t.x);
        Self {
            sigma_t: air.sigma_t + Vec3::splat(drops),
            albedo: air.albedo.lerp(rain.albedo, share),
            anisotropy: air.anisotropy + (rain.anisotropy - air.anisotropy) * share,
        }
    }

    /// The open air by day, a thin default (Remix's is the runtime's, its own atmosphere being off):
    /// 0.95 of the light through 100 m (about 88 % over the 256 m the grid reaches), no
    /// absorption, scattering forward (g 0.7) so the sun's shafts show past shadows and toward it
    /// without a haze over the rest of the view.
    pub fn air() -> Self {
        Self::measured(Vec3::splat(0.95), 100.0, Vec3::splat(1.0), 0.7)
    }

    /// Between two media (the extinction and the albedo linearly, `t` 0: `self`).
    pub fn mix(self, other: Self, t: f32) -> Self {
        Self {
            sigma_t: self.sigma_t.lerp(other.sigma_t, t),
            albedo: self.albedo.lerp(other.albedo, t),
            anisotropy: self.anisotropy + (other.anisotropy - self.anisotropy) * t,
        }
    }

    /// A medium by name (`HORIZON_FROXEL_MEDIUM`: water, cave, rain, thunder or air), for testing.
    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "water" => Self::water(),
            "cave" => Self::cave(),
            "rain" => Self::rain(),
            "thunder" => Self::thunder(),
            "air" => Self::air(),
            _ => return None,
        })
    }

    /// Whether the grid's history is stale after a change from `other`: the phase or the albedo
    /// moved, or the extinction by a quarter (a weather fading in moves it smoothly and never does).
    fn differs(&self, other: &Self) -> bool {
        let sigma = (self.sigma_t - other.sigma_t).abs().max_element();
        sigma > 0.25 * other.sigma_t.max_element().max(1e-4)
            || (self.albedo - other.albedo).abs().max_element() > 0.05
            || (self.anisotropy - other.anisotropy).abs() > 0.05
    }
}

/// Where the camera is, for the medium the froxels fill (Remix switches its media by it). The main
/// session fills it every frame (`Renderer::set_volumetrics`); what `Frame` already says is merged
/// in by the renderer (the camera under water, the rain and thunder levels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraMedium {
    /// The camera's eye is in water.
    pub in_water: bool,
    /// The sky light level at the camera's block, 0 to 15 (the world's stored light, not dimmed by
    /// the night): 15 under the open sky, down to 0 deep in a cave.
    pub sky_light: f32,
    /// The rain and thunder levels, 0 to 1 (`Frame::weather`'s, as `pack.rain`, `pack.thunder`).
    pub rain: f32,
    pub thunder: f32,
}

impl Default for CameraMedium {
    /// Open air under the sky, in clear weather.
    fn default() -> Self {
        Self {
            in_water: false,
            sky_light: 15.0,
            rain: 0.0,
            thunder: 0.0,
        }
    }
}

impl CameraMedium {
    /// The medium for the camera's place: water in water; else the cave's where the sky light is
    /// low (all of it at level 3 and below, none from 9 up, a smooth change between); else the open
    /// air, with the rain's drops in it as it falls harder (`Medium::falling`).
    pub fn medium(&self) -> Medium {
        if self.in_water {
            return Medium::water();
        }
        let weather = Medium::falling(crate::rain::rate_mm_h(self.rain, self.thunder));
        let open = ((self.sky_light - 3.0) / 6.0).clamp(0.0, 1.0);
        let open = open * open * (3.0 - 2.0 * open);
        Medium::cave().mix(weather, open)
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    inverse_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    light: [f32; 4],
    camera: [f32; 4],
    fraction: [f32; 4],
    grid: [f32; 4],
    sun: [f32; 4],
    sun_light: [f32; 4],
    hw_offset: [f32; 4],
    water: [f32; 4],
    motion: [f32; 4],
    dims: [f32; 4],
    depth: [f32; 4],
    sigma_t: [f32; 4],
    albedo: [f32; 4],
    history: [f32; 4],
    size: [f32; 4],
    eye: [f32; 4],
    prev_eye: [f32; 4],
    jitter: [f32; 4],
}

/// Where a view's rays start, camera-relative: the point whose clip x, y and w are all 0 (the view
/// bobbing moves it off the camera). Float64, the projection's reversed depth being ill-conditioned.
fn eye_of(view_proj: Mat4) -> Vec3 {
    let e = view_proj.as_dmat4().inverse() * glam::DVec4::new(0.0, 0.0, 1.0, 0.0);
    if e.w.abs() < 1e-12 {
        return Vec3::ZERO;
    }
    (e.truncate() / e.w).as_vec3()
}

/// Frame `n`'s offset within a froxel: the 3D R-sequence (Roberts), whose points spread evenly in
/// the unit cube however many are taken.
fn jitter(n: u32) -> [f32; 4] {
    // The plastic number, whose powers' reciprocals step the three axes.
    const G: f64 = 1.220_744_084_605_759_5;
    let f = f64::from(n);
    let axis = |k: i32| (0.5 + f * G.powi(-k)).fract() as f32;
    [axis(1), axis(2), axis(3), 0.0]
}

/// A view's froxels: two grids of the source (the last frame's is the history) and the integrated
/// one.
struct ViewFroxels {
    view: usize,
    dims: [u32; 3],
    source: [wgpu::TextureView; 2],
    /// The samples drawn this frame, before they meet the history.
    fresh: wgpu::TextureView,
    integrated: wgpu::TextureView,
    /// The grid written next.
    next: usize,
    /// The last frame's view projection and camera, for the reprojection.
    last: Option<(Mat4, DVec3)>,
    /// The medium the history was made in, and the frames of settling left.
    medium: Option<Medium>,
    settling: u32,
}

fn texture_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

/// The views kept at once: the window, two eyes, two quad-view insets.
const KEPT: usize = 5;

/// The pass's shader, with the hardware rays (`froxels_hw.wgsl` in place of the stub between the
/// `HARDWARE` markers) or without (`sprite_split`: the sprite layers past it are taken as opaque).
pub fn source(hardware: bool, sprite_split: u32) -> String {
    let text = include_str!("froxels.wgsl");
    if !hardware {
        return text.to_string();
    }
    let begin = text
        .find("// HARDWARE:BEGIN")
        .expect("the hardware section");
    let end = text.find("// HARDWARE:END").expect("the hardware section");
    format!(
        "enable wgpu_ray_query;\nconst SPRITE_SPLIT: u32 = {sprite_split}u;\n{}{}{}",
        &text[..begin],
        include_str!("froxels_hw.wgsl"),
        &text[end..]
    )
}

pub struct Froxels {
    /// The volumetrics are drawn (`Renderer::set_volumetrics`, or `HORIZON_FROXELS=1`).
    pub enabled: bool,
    /// Where the camera is (`Renderer::set_volumetrics`).
    pub camera: CameraMedium,
    /// A medium forced for testing (`HORIZON_FROXEL_MEDIUM`).
    forced: Option<Medium>,
    hardware: bool,
    inject: wgpu::ComputePipeline,
    inject_layout: wgpu::BindGroupLayout,
    blend: wgpu::ComputePipeline,
    blend_layout: wgpu::BindGroupLayout,
    integrate: wgpu::ComputePipeline,
    integrate_layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
    apply_layout: wgpu::BindGroupLayout,
    /// The composite's pipelines (the scene times the transmittance, then the in-scatter added), for
    /// the colour format they were made for.
    apply: Option<(wgpu::TextureFormat, [wgpu::RenderPipeline; 2])>,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    views: Vec<ViewFroxels>,
    frame: u32,
}

/// What `Froxels::run` draws from, for one view.
pub struct Inputs<'a> {
    /// The view (0 the window, an eye's index in VR): each keeps its own history.
    pub view: usize,
    /// The world's depth (reversed, 0 the sky) of `size`, and the colour it was drawn into.
    pub depth: &'a wgpu::TextureView,
    pub color: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    pub size: [u32; 2],
    pub view_proj: Mat4,
    pub camera: DVec3,
    /// This frame's rays: the sun's direction and light, the grid's place, the hybrid's offset
    /// (`Renderer::ray_traced_shadows`).
    pub rays: &'a Rays,
    /// The voxel grid (`voxels.views()`), the light sources (`voxels.light_buffers()`), the sky
    /// (`atmosphere.view`) and the hybrid's structures (`Renderer::hw_bindings`).
    pub grid: [&'a wgpu::TextureView; 5],
    pub lights: [&'a wgpu::Buffer; 3],
    pub sky: &'a wgpu::TextureView,
    pub hardware: Option<Hardware<'a>>,
}

impl Froxels {
    pub fn new(device: &wgpu::Device, hardware: bool, sprite_split: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Froxel volumetrics"),
            source: wgpu::ShaderSource::Wgsl(source(hardware, sprite_split).into()),
        });
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let compute = wgpu::ShaderStages::COMPUTE;
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let grid = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D3,
            multisampled: false,
        };
        let storage = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let froxels = |filterable| wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D3,
            multisampled: false,
        };
        let output = wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba16Float,
            view_dimension: wgpu::TextureViewDimension::D3,
        };
        let sampler = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let mut inject_entries = vec![
            entry(0, compute, uniform),
            entry(2, compute, grid),
            entry(3, compute, grid),
            entry(4, compute, grid),
            entry(5, compute, grid),
            entry(7, compute, storage),
            entry(8, compute, storage),
            entry(
                10,
                compute,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            ),
            entry(22, compute, output),
        ];
        if hardware {
            inject_entries.extend([
                entry(
                    15,
                    compute,
                    wgpu::BindingType::AccelerationStructure {
                        vertex_return: false,
                    },
                ),
                entry(16, compute, storage),
                entry(23, compute, storage),
                entry(
                    17,
                    compute,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                ),
            ]);
        }
        let inject_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Froxel volumetrics' inject"),
            entries: &inject_entries,
        });
        let blend_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Froxel volumetrics' blend"),
            entries: &[
                entry(0, compute, uniform),
                entry(11, compute, froxels(true)),
                entry(12, compute, sampler),
                entry(13, compute, output),
                entry(21, compute, froxels(false)),
            ],
        });
        let integrate_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Froxel volumetrics' integrate"),
            entries: &[
                entry(0, compute, uniform),
                entry(19, compute, froxels(false)),
                entry(14, compute, output),
            ],
        });
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let apply_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Froxel volumetrics' composite"),
            entries: &[
                entry(0, fragment, uniform),
                entry(
                    1,
                    fragment,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(12, fragment, sampler),
                entry(20, fragment, froxels(true)),
            ],
        });
        let pipeline = |label, layout: &wgpu::BindGroupLayout, entry_point| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(
                    &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(label),
                        bind_group_layouts: &[Some(layout)],
                        ..Default::default()
                    }),
                ),
                module: &shader,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let inject = pipeline("Froxel volumetrics' inject", &inject_layout, "cs_inject");
        let blend = pipeline("Froxel volumetrics' blend", &blend_layout, "cs_blend");
        let integrate = pipeline(
            "Froxel volumetrics' integrate",
            &integrate_layout,
            "cs_integrate",
        );
        let forced = std::env::var("HORIZON_FROXEL_MEDIUM")
            .ok()
            .and_then(|name| Medium::named(name.trim()));
        Self {
            enabled: std::env::var("HORIZON_FROXELS").is_ok_and(|v| v != "0"),
            camera: CameraMedium::default(),
            forced,
            hardware,
            inject,
            inject_layout,
            blend,
            blend_layout,
            integrate,
            integrate_layout,
            shader,
            apply_layout,
            apply: None,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Froxel volumetrics"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Froxel volumetrics"),
                size: std::mem::size_of::<Uniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            views: Vec::new(),
            frame: 0,
        }
    }

    /// The medium this frame: the forced one, else the camera's place.
    pub fn medium(&self) -> Medium {
        self.forced.unwrap_or_else(|| self.camera.medium())
    }

    /// The composite's two pipelines for a colour target of `format`: the scene multiplied by the
    /// transmittance, then the in-scatter added. The scene's alpha (the terrain's sun share, read by
    /// the tone mapping) is left alone.
    fn apply_pipelines(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &[wgpu::RenderPipeline; 2] {
        if self.apply.as_ref().is_none_or(|(f, _)| *f != format) {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Froxel volumetrics' composite"),
                bind_group_layouts: &[Some(&self.apply_layout)],
                ..Default::default()
            });
            let keep_alpha = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            };
            let make = |label, entry_point, color: wgpu::BlendComponent| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &self.shader,
                        entry_point: Some("vs_apply"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.shader,
                        entry_point: Some(entry_point),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: Some(wgpu::BlendState {
                                color,
                                alpha: keep_alpha,
                            }),
                            write_mask: wgpu::ColorWrites::COLOR,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
            let pipelines = [
                make(
                    "Froxel volumetrics' transmittance",
                    "fs_transmit",
                    wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::Src,
                        operation: wgpu::BlendOperation::Add,
                    },
                ),
                make(
                    "Froxel volumetrics' in-scatter",
                    "fs_scatter",
                    wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                ),
            ];
            self.apply = Some((format, pipelines));
        }
        &self.apply.as_ref().expect("just made").1
    }

    /// The view's froxels, made (and its history dropped) when its size changed.
    fn view_froxels(&mut self, device: &wgpu::Device, view: usize, size: [u32; 2]) -> usize {
        let dims = [
            size[0].div_ceil(TILE).max(1),
            size[1].div_ceil(TILE).max(1),
            SLICES,
        ];
        if let Some(i) = self.views.iter().position(|v| v.view == view) {
            if self.views[i].dims == dims {
                return i;
            }
            self.views.remove(i);
        }
        if self.views.len() >= KEPT {
            self.views.remove(0);
        }
        let texture = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: dims[0],
                        height: dims[1],
                        depth_or_array_layers: dims[2],
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        self.views.push(ViewFroxels {
            view,
            dims,
            source: [
                texture("Froxel volumetrics' source"),
                texture("Froxel volumetrics' source"),
            ],
            fresh: texture("Froxel volumetrics' samples"),
            integrated: texture("Froxel volumetrics' integral"),
            next: 0,
            last: None,
            medium: None,
            settling: SETTLING_FRAMES,
        });
        self.views.len() - 1
    }

    /// One view's froxels this frame, inject, integrate and composite, onto `inputs.color` (the
    /// world drawn, `inputs.depth` its depth). Call once per view (the window, each VR eye) after
    /// the world pass and before the tone mapping.
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        inputs: Inputs<'_>,
    ) {
        // A pipeline that traces hardware rays has nothing to bind before the hybrid's first
        // structures exist.
        if self.hardware && inputs.hardware.is_none() {
            return;
        }
        let medium = self.medium();
        self.frame = self.frame.wrapping_add(1);
        let frame = self.frame;
        let index = self.view_froxels(device, inputs.view, inputs.size);
        let state = &mut self.views[index];
        let dims = state.dims;
        if state.medium.is_some_and(|m| m.differs(&medium)) {
            state.settling = SETTLING_FRAMES;
        }
        state.medium = Some(medium);
        let cap = if state.settling > 0 {
            SETTLING_ACCUMULATION
        } else {
            MAX_ACCUMULATION
        };
        state.settling = state.settling.saturating_sub(1);
        let last = state.last.replace((inputs.view_proj, inputs.camera));
        let from = state.source[1 - state.next].clone();
        let to = state.source[state.next].clone();
        state.next = 1 - state.next;
        let fresh = state.fresh.clone();
        let integrated = state.integrated.clone();
        let rays = inputs.rays;
        let eye = eye_of(inputs.view_proj);
        let prev_eye = last.map_or(eye, |l| eye_of(l.0));
        let uniform = Uniform {
            inverse_view_proj: inputs.view_proj.inverse().to_cols_array_2d(),
            prev_view_proj: last.map_or([[0.0; 4]; 4], |l| l.0.to_cols_array_2d()),
            light: rays.light,
            camera: rays.camera,
            fraction: rays.fraction,
            grid: rays.grid,
            sun: rays.sun,
            sun_light: rays.sun_light,
            hw_offset: rays.hw_offset,
            water: rays.water,
            motion: match last {
                Some((_, last_camera)) => {
                    let m = (inputs.camera - last_camera).as_vec3();
                    [m.x, m.y, m.z, 1.0]
                }
                None => [0.0; 4],
            },
            dims: [
                dims[0] as f32,
                dims[1] as f32,
                dims[2] as f32,
                (frame % 65536) as f32,
            ],
            depth: [NEAR, FAR, (FAR / NEAR).ln(), 0.0],
            sigma_t: [
                medium.sigma_t.x,
                medium.sigma_t.y,
                medium.sigma_t.z,
                medium.anisotropy,
            ],
            albedo: [medium.albedo.x, medium.albedo.y, medium.albedo.z, 1.0],
            history: [cap, 1.0, 0.0, 0.0],
            size: [inputs.size[0] as f32, inputs.size[1] as f32, 0.0, 0.0],
            eye: [eye.x, eye.y, eye.z, 0.0],
            prev_eye: [prev_eye.x, prev_eye.y, prev_eye.z, 0.0],
            jitter: jitter(frame),
        };
        uploads.buffer(encoder, &self.uniform, 0, bytemuck::bytes_of(&uniform));
        let (view, buffer) = (texture_entry, buffer_entry);
        let mut entries = vec![
            buffer(0, &self.uniform),
            view(2, inputs.grid[0]),
            view(3, inputs.grid[1]),
            view(4, inputs.grid[2]),
            view(5, inputs.grid[3]),
            buffer(7, inputs.lights[0]),
            buffer(8, inputs.lights[1]),
            view(10, inputs.sky),
            view(22, &fresh),
        ];
        if let (true, Some(hw)) = (self.hardware, &inputs.hardware) {
            entries.extend([
                wgpu::BindGroupEntry {
                    binding: 15,
                    resource: wgpu::BindingResource::AccelerationStructure(hw.tlas),
                },
                buffer(16, hw.arena),
                view(17, hw.sprites),
                buffer(23, hw.instances),
            ]);
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Froxel volumetrics' inject"),
            layout: &self.inject_layout,
            entries: &entries,
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Froxel volumetrics' inject"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.inject);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(
                dims[0].div_ceil(4),
                dims[1].div_ceil(4),
                dims[2].div_ceil(4),
            );
        }
        super::gpu_passes::stamp(encoder, "froxels: inject (lights, shadows)");
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Froxel volumetrics' blend"),
            layout: &self.blend_layout,
            entries: &[
                buffer(0, &self.uniform),
                view(11, &from),
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                view(13, &to),
                view(21, &fresh),
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Froxel volumetrics' blend"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.blend);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(
                dims[0].div_ceil(4),
                dims[1].div_ceil(4),
                dims[2].div_ceil(4),
            );
        }
        super::gpu_passes::stamp(encoder, "froxels: blend (temporal)");
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Froxel volumetrics' integrate"),
            layout: &self.integrate_layout,
            entries: &[
                buffer(0, &self.uniform),
                view(19, &to),
                view(14, &integrated),
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Froxel volumetrics' integrate"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.integrate);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(dims[0].div_ceil(8), dims[1].div_ceil(8), 1);
        }
        super::gpu_passes::stamp(encoder, "froxels: integrate");
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Froxel volumetrics' composite"),
            layout: &self.apply_layout,
            entries: &[
                buffer(0, &self.uniform),
                view(1, inputs.depth),
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                view(20, &integrated),
            ],
        });
        let pipelines = self.apply_pipelines(device, inputs.color_format).clone();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Froxel volumetrics' composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: inputs.color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_bind_group(0, &group, &[]);
        for pipeline in &pipelines {
            pass.set_pipeline(pipeline);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extinction_is_the_measured_transmittance() {
        // sigma_t = -ln(T) / d: the light left over the measured distance is the colour's.
        let water = Medium::water();
        for (sigma, t) in water.sigma_t.to_array().into_iter().zip([0.6, 0.9, 0.89]) {
            assert!(((-sigma * 20.0).exp() - t).abs() < 1e-5);
        }
        let rain = Medium::rain();
        assert!(((-rain.sigma_t.x * 10.0).exp() - 0.996).abs() < 1e-5);
        let air = Medium::air();
        assert!(((-air.sigma_t.y * 100.0).exp() - 0.95).abs() < 1e-5);
    }

    #[test]
    fn the_camera_picks_the_medium() {
        let open = CameraMedium::default();
        assert_eq!(open.medium(), Medium::air());
        let water = CameraMedium {
            in_water: true,
            ..open
        };
        assert_eq!(water.medium(), Medium::water());
        let cave = CameraMedium {
            sky_light: 0.0,
            ..open
        };
        assert_eq!(cave.medium(), Medium::cave());
        // Between the cave and the open: in between.
        let mouth = CameraMedium {
            sky_light: 6.0,
            ..open
        }
        .medium();
        assert!(
            mouth.sigma_t.x > Medium::air().sigma_t.x && mouth.sigma_t.x < Medium::cave().sigma_t.x
        );
        let storm = CameraMedium {
            rain: 1.0,
            thunder: 1.0,
            ..open
        };
        let rain = CameraMedium { rain: 1.0, ..open };
        let air = Medium::air().sigma_t.x;
        // The drops' extinction over the air's, a storm's beyond Remix's thunder.
        assert!(rain.medium().sigma_t.x > air + Medium::rain().sigma_t.x);
        assert!(storm.medium().sigma_t.x > air + Medium::thunder().sigma_t.x);
    }

    #[test]
    fn a_medium_change_unsettles_the_history() {
        assert!(Medium::water().differs(&Medium::air()));
        assert!(Medium::cave().differs(&Medium::air()));
        let a = Medium::air();
        assert!(!a.differs(&a.mix(Medium::rain(), 0.01)));
    }

    #[test]
    fn the_uniform_matches_the_shader() {
        for hardware in [false, true] {
            let text = source(hardware, 2048);
            let module = naga::front::wgsl::parse_str(&text)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&text)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
            let mut layouter = naga::proc::Layouter::default();
            layouter.update(module.to_ctx()).expect("its layout");
            let (fx, _) = module
                .types
                .iter()
                .find(|(_, t)| t.name.as_deref() == Some("Fx"))
                .expect("the uniform");
            assert_eq!(layouter[fx].size as usize, std::mem::size_of::<Uniform>());
        }
    }
}
