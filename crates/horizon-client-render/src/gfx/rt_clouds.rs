//! RT lighting's volumetric clouds (docs/raytracing.md, gap 6): cumulus in a slab 800 to 2200
//! blocks up and a sheet of cirrus at 8 km, in place of vanilla's flat clouds, after the way the
//! well-known real-time cloud renderers do it (Nubis, Frostbite, the Minecraft packs that follow
//! them; `rt_clouds.wgsl` says what is taken from which). The sun and moon's light, the direction
//! and the sky's table are RT lighting's own (`atmosphere`); the clouds are drawn over the world's
//! sky before the tone mapping: the scene times the clouds' transmittance, plus the light they send.
//!
//! Two ways to draw them ([`Mode`]):
//! - The dome (the default): marched once a frame into a map of the sky's directions around the
//!   camera, a sixteenth of its texels a frame in turn, reprojected and accumulated there; every
//!   view (the window, both eyes of a headset) only looks the map up. Clouds are far enough that
//!   both eyes see them alike, so a headset pays for one march, not two. A camera within 150 blocks
//!   of the slab's base draws them per view instead.
//! - Per view: each view marches at a fraction of its resolution into a small texture,
//!   reprojected and blended with the last frame's by view direction.
//!
//! The clouds' shape is noise made once on the CPU: a Perlin-Worley 64^3 with three Worley octaves
//! beside it for the shape, a Worley 32^3 for the erosion, and a 2D weather map of low-frequency
//! noise for where the clouds are and of which type. They tile, so wind is a drift of the lookups.
//! Each day's weather differs: more or fewer cumulus, taller or flatter, more or less cirrus.

use super::tonemap::SCENE_FORMAT;
use super::upload::Uploads;
use glam::{DVec3, Mat4, Vec3};

/// The views kept at once, as `shadow_rays`: the window, two eyes, two quad-view insets.
const KEPT: usize = 5;

/// The noise textures' sides (the shape's, the detail's, the weather map's).
pub(super) const SHAPE: u32 = 64;
pub(super) const DETAIL: u32 = 32;
pub(super) const WEATHER: u32 = 256;

/// The distance the noise and the camera wrap at (rt_clouds.wgsl `WRAP`): the weather map tiles
/// every 16 km, the shape every 4 km, so a drift modulo this one does not show.
pub(super) const WRAP: f64 = 32768.0;
/// The wind's speed in blocks a second (x, z): a slow drift, a little faster than real cumulus
/// so that it shows within a minute.
pub(super) const WIND: [f64; 2] = [6.0, 2.5];
/// This frame's share of the history (about the last ten frames' average).
const BLEND: f32 = 0.1;
/// The cumulus slab's base (rt_clouds.wgsl `BASE`): a camera less than `DOME_CLEARANCE` under it
/// draws the clouds per view.
const BASE: f64 = 800.0;
const DOME_CLEARANCE: f64 = 150.0;
/// A camera moving farther than this in a frame starts the dome's history again.
const TELEPORT: f32 = 1500.0;
/// The length of a Minecraft day in seconds: the weather changes from one to the next.
const DAY: f64 = 1200.0;

/// How many pixels of the view a cloud texel is on a side (`HORIZON_RT_CLOUD_SCALE`), per view.
fn scale() -> u32 {
    static SCALE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| {
        std::env::var("HORIZON_RT_CLOUD_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v| (1..=8).contains(v))
            .unwrap_or(4)
    })
}

/// The dome's rows (`HORIZON_RT_CLOUD_DOME`, 256 to 4096; its columns four times as many) and the
/// side of its checkerboard (`HORIZON_RT_CLOUD_CHECKER`, 1, 2 or 4: one texel in its square a
/// frame is marched).
fn dome_shape() -> ([u32; 2], u32) {
    static SHAPE: std::sync::OnceLock<([u32; 2], u32)> = std::sync::OnceLock::new();
    *SHAPE.get_or_init(|| {
        let rows = std::env::var("HORIZON_RT_CLOUD_DOME")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| (256..=4096).contains(v))
            .unwrap_or(1024);
        let checker = std::env::var("HORIZON_RT_CLOUD_CHECKER")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| [1, 2, 4].contains(v))
            .unwrap_or(4);
        ([rows * 4, rows], checker)
    })
}

/// The order the checkerboard's texels are marched in (x + y * side), spread out as a Bayer
/// matrix's.
fn checker_order(side: u32) -> &'static [u32] {
    match side {
        4 => &[0, 10, 2, 8, 5, 15, 7, 13, 1, 11, 3, 9, 4, 14, 6, 12],
        2 => &[0, 3, 1, 2],
        _ => &[0],
    }
}

/// How RT lighting draws clouds (`HORIZON_RT_CLOUDS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Vanilla's flat clouds (`0`).
    Off,
    /// A mesh, as RTX Remix does (`rt_cloud_mesh`; `mesh`, or bit 2048 of `HORIZON_RT_SKIP`):
    /// the clouds before the dome, and the only ones the rays' shadows meet.
    Mesh,
    /// The volume, marched per view (`march`).
    March,
    /// The volume, marched into the dome all views share: the default.
    Dome,
}

pub fn mode() -> Mode {
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        let skip = std::env::var("HORIZON_RT_SKIP")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        match std::env::var("HORIZON_RT_CLOUDS").as_deref() {
            Ok("0") => Mode::Off,
            Ok("mesh") => Mode::Mesh,
            Ok("march") => Mode::March,
            Ok("dome") => Mode::Dome,
            _ if skip & 2048 != 0 => Mode::Mesh,
            _ => Mode::Dome,
        }
    })
}

/// Whether a camera at `camera` can look at the dome (else the clouds are drawn per view).
pub fn dome_sees(camera: DVec3) -> bool {
    camera.y < BASE - DOME_CLEARANCE
}

/// The weather map's tile, in blocks (rt_clouds.wgsl `COVERAGE`).
pub(super) const COVERAGE_TILE: f32 = 16384.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    inverse_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    light: [f32; 4],
    light_color: [f32; 4],
    sun: [f32; 4],
    camera: [f32; 4],
    wind: [f32; 4],
    motion: [f32; 4],
    size: [f32; 4],
    jitter: [f32; 4],
    layers: [f32; 4],
    drifts: [f32; 4],
}

/// A frame's light and weather for the clouds.
pub struct Params {
    /// Toward the light the clouds are lit by (the sun's, or the moon's once the sun has set).
    pub light: Vec3,
    /// That light's colour, in paper whites (`Globals::rt_sun_light`).
    pub light_color: [f32; 3],
    /// Toward the sun, which the sky table's azimuth is from.
    pub sun: Vec3,
    /// The time in seconds, for the wind and the day's weather.
    pub time: f64,
    /// 0 for fair weather, 1 for the thickest overcast (the rain).
    pub thick: f32,
    /// The Earth's curvature the slab bends by: 1 / its diameter, 0 off.
    pub curvature: f32,
}

/// The day's weather at `time` (seconds): the cumulus' coverage bias (-1 fewer and flatter, 1 more
/// and taller), the cirrus' amount and the altocumulus' (0..1), drifting from one day's to the
/// next over the day.
pub fn weather(time: f64) -> (f32, f32, f32) {
    let day = (time / DAY).floor();
    let into = ((time / DAY) - day) as f32;
    let of = |day: f64| {
        let h = hash(day as i32, 7, 3, 101);
        let bias = unit(h) * 2.0 - 1.0;
        // Many days a little cirrus, some a lot, some none; altocumulus one day in three or so.
        let cirrus = (unit(h.rotate_left(13) ^ 0x51ed_270b) * 1.4 - 0.2).clamp(0.0, 1.0);
        let alto = (unit(h.rotate_left(23) ^ 0x2545_f491) * 2.2 - 1.2).clamp(0.0, 1.0);
        (bias, cirrus, alto)
    };
    let (a, b) = (of(day), of(day + 1.0));
    let t = into * into * (3.0 - 2.0 * into);
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

/// How fast the altocumulus and the cirrus drift against the cumulus (winds aloft are stronger).
const ALTO_WIND: f64 = 1.6;
const CIRRUS_WIND: f64 = 2.5;

/// The uniform's rows the march reads (`Uniform`'s light, light_color, sun, camera, the wind's
/// drift, layers and drifts).
struct Rows {
    light: [f32; 4],
    light_color: [f32; 4],
    sun: [f32; 4],
    camera: [f32; 4],
    drift: [f32; 2],
    layers: [f32; 4],
    drifts: [f32; 4],
}

impl Params {
    /// The uniform's rows the march reads, for a camera at `camera`.
    fn rows(&self, camera: DVec3) -> Rows {
        let (bias, cirrus, alto) = weather(self.time);
        #[cfg(test)]
        let (cirrus, alto) = match std::env::var("HORIZON_CLOUD_STILL_WITHOUT").as_deref() {
            Ok("cirrus") => (0.0, alto),
            Ok("alto") => (cirrus, 0.0),
            _ => (cirrus, alto),
        };
        let drift = |speed: f64| (self.time * speed).rem_euclid(WRAP) as f32;
        Rows {
            light: [self.light.x, self.light.y, self.light.z, self.curvature],
            light_color: [
                self.light_color[0],
                self.light_color[1],
                self.light_color[2],
                cirrus,
            ],
            sun: [self.sun.x, self.sun.y, self.sun.z, bias],
            camera: [
                camera.x.rem_euclid(WRAP) as f32,
                camera.y as f32,
                camera.z.rem_euclid(WRAP) as f32,
                self.thick,
            ],
            drift: [drift(WIND[0]), drift(WIND[1])],
            layers: [alto, 0.0, SHADOW_TEXEL, 0.0],
            drifts: [
                drift(WIND[0] * ALTO_WIND),
                drift(WIND[1] * ALTO_WIND),
                drift(WIND[0] * CIRRUS_WIND),
                drift(WIND[1] * CIRRUS_WIND),
            ],
        }
    }
}

/// The clouds' shadow map's side in texels, and a texel's in blocks: it wraps every 4096 blocks
/// (terrain.wgsl `CLOUD_SHADOW_SIDE`, the period `Globals::rt_camera` wraps at).
const SHADOW_SIZE: u32 = 1024;
const SHADOW_TEXEL: f32 = 4.0;

/// The clouds' shadows the terrain reads (terrain.wgsl `rt_cloud_shadow`): the light's
/// transmittance through the clouds by where its ray crosses the cumulus' base, x and z modulo
/// 4096 blocks, 4 blocks a texel, made by [`RtClouds::update_shadows`] a sixteenth at a time; white
/// (1x1) where the clouds are not volumes.
pub struct Shadows {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    /// The last update's camera and time, and how many updates it has made.
    last: Option<(DVec3, f64)>,
    frame: u32,
}

impl Shadows {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let volume = matches!(mode(), Mode::Dome | Mode::March);
        let side = if volume { SHADOW_SIZE } else { 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Cloud shadows"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // White until the first update: 1.0 in half floats is 0x3c00.
        let one = 0x3c00u16.to_le_bytes();
        let white: Vec<u8> = (0..side * side * 4).flat_map(|_| one).collect();
        queue.write_texture(
            texture.as_image_copy(),
            &white,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * 8),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
        );
        Self {
            view: texture.create_view(&Default::default()),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Cloud shadows"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            last: None,
            frame: 0,
        }
    }
}

/// A view's textures: this frame's march, and the history, written in turn.
struct ViewClouds {
    view: usize,
    size: [u32; 2],
    low: [u32; 2],
    raw: wgpu::TextureView,
    history: [wgpu::TextureView; 2],
    /// The history the next frame writes.
    next: usize,
    /// The last frame's view projection and camera.
    last: Option<(Mat4, DVec3)>,
}

/// The dome's textures: this frame's march (the light, the distance), and the history (the light;
/// the distance and how many marches it holds), written in turn.
struct Dome {
    size: [u32; 2],
    low: [u32; 2],
    checker: u32,
    raw: wgpu::TextureView,
    raw_distance: wgpu::TextureView,
    history: [wgpu::TextureView; 2],
    data: [wgpu::TextureView; 2],
    /// The history the next frame writes (the other one is the newest).
    next: usize,
    /// The last update's camera and time.
    last: Option<(DVec3, f64)>,
    frame: u32,
}

pub struct RtClouds {
    march: wgpu::ComputePipeline,
    march_layout: wgpu::BindGroupLayout,
    resolve: wgpu::ComputePipeline,
    resolve_layout: wgpu::BindGroupLayout,
    composite: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    dome_march: wgpu::ComputePipeline,
    dome_resolve: wgpu::ComputePipeline,
    dome_resolve_layout: wgpu::BindGroupLayout,
    dome_composite: wgpu::RenderPipeline,
    dome_composite_layout: wgpu::BindGroupLayout,
    shadow: wgpu::ComputePipeline,
    uniform: wgpu::Buffer,
    dome_uniform: wgpu::Buffer,
    shadow_uniform: wgpu::Buffer,
    shape: wgpu::TextureView,
    detail: wgpu::TextureView,
    weather: wgpu::TextureView,
    repeat: wgpu::Sampler,
    clamp: wgpu::Sampler,
    /// Around the dome's azimuth, clamped at its ends.
    around: wgpu::Sampler,
    /// The per-view march's stand-in for the dome's distance target.
    no_distance: wgpu::TextureView,
    views: Vec<ViewClouds>,
    dome: Option<Dome>,
    frame: u32,
}

impl RtClouds {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let noise = Noise::shared();
        let texture = |label, dimension, size: [u32; 3], format, data: &[u8]| {
            use wgpu::util::DeviceExt;
            device
                .create_texture_with_data(
                    queue,
                    &wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: size[2],
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension,
                        format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    data,
                )
                .create_view(&Default::default())
        };
        let shape = texture(
            "Cloud shape noise",
            wgpu::TextureDimension::D3,
            [SHAPE; 3],
            wgpu::TextureFormat::Rgba8Unorm,
            &noise.shape,
        );
        let detail = texture(
            "Cloud detail noise",
            wgpu::TextureDimension::D3,
            [DETAIL; 3],
            wgpu::TextureFormat::Rgba8Unorm,
            &noise.detail,
        );
        let weather = texture(
            "Cloud weather map",
            wgpu::TextureDimension::D2,
            [WEATHER, WEATHER, 1],
            wgpu::TextureFormat::Rg8Unorm,
            &noise.weather,
        );
        let sampler = |u, v| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Clouds"),
                address_mode_u: u,
                address_mode_v: v,
                address_mode_w: u,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            })
        };
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RT lighting's clouds"),
            source: wgpu::ShaderSource::Wgsl(include_str!("rt_clouds.wgsl").into()),
        });
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let compute = wgpu::ShaderStages::COMPUTE;
        let uniform_entry = entry(
            0,
            compute | wgpu::ShaderStages::FRAGMENT,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        );
        let depth = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let filterable = |dimension| wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: dimension,
            multisampled: false,
        };
        let unfiltered = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let sampler_binding = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let storage = |format| wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let output = storage(wgpu::TextureFormat::Rgba16Float);
        let march_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT lighting's clouds, march"),
            entries: &[
                uniform_entry,
                entry(1, compute, depth),
                entry(2, compute, filterable(wgpu::TextureViewDimension::D3)),
                entry(3, compute, filterable(wgpu::TextureViewDimension::D3)),
                entry(4, compute, filterable(wgpu::TextureViewDimension::D2)),
                entry(5, compute, sampler_binding),
                entry(6, compute, filterable(wgpu::TextureViewDimension::D2)),
                entry(7, compute, sampler_binding),
                entry(8, compute, output),
                entry(9, compute, storage(wgpu::TextureFormat::R32Float)),
            ],
        });
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT lighting's clouds, resolve"),
            entries: &[
                uniform_entry,
                entry(10, compute, filterable(wgpu::TextureViewDimension::D2)),
                entry(11, compute, filterable(wgpu::TextureViewDimension::D2)),
                entry(12, compute, sampler_binding),
                entry(13, compute, output),
            ],
        });
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT lighting's clouds, composite"),
            entries: &[
                uniform_entry,
                entry(1, fragment, depth),
                entry(14, fragment, filterable(wgpu::TextureViewDimension::D2)),
                entry(15, fragment, sampler_binding),
            ],
        });
        let dome_resolve_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("RT lighting's clouds, dome resolve"),
                entries: &[
                    uniform_entry,
                    entry(16, compute, filterable(wgpu::TextureViewDimension::D2)),
                    entry(17, compute, unfiltered),
                    entry(18, compute, filterable(wgpu::TextureViewDimension::D2)),
                    entry(19, compute, filterable(wgpu::TextureViewDimension::D2)),
                    entry(20, compute, sampler_binding),
                    entry(21, compute, output),
                    entry(22, compute, output),
                ],
            });
        let dome_composite_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("RT lighting's clouds, dome composite"),
                entries: &[
                    uniform_entry,
                    entry(1, fragment, depth),
                    entry(20, fragment, sampler_binding),
                    entry(23, fragment, filterable(wgpu::TextureViewDimension::D2)),
                ],
            });
        let compute_pipeline = |label, layout: &wgpu::BindGroupLayout, entry_point| {
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
        let march = compute_pipeline("RT lighting's clouds, march", &march_layout, "cs_march");
        let resolve = compute_pipeline(
            "RT lighting's clouds, resolve",
            &resolve_layout,
            "cs_resolve",
        );
        let dome_march = compute_pipeline(
            "RT lighting's clouds, dome march",
            &march_layout,
            "cs_dome_march",
        );
        let shadow = compute_pipeline("RT lighting's clouds, shadows", &march_layout, "cs_shadow");
        let dome_resolve = compute_pipeline(
            "RT lighting's clouds, dome resolve",
            &dome_resolve_layout,
            "cs_dome_resolve",
        );
        // The scene times the transmittance (the fragment's alpha) plus the light; the scene's own
        // alpha (the tone mapping's share of the sun) is left alone.
        let composite_pipeline = |label, layout: &wgpu::BindGroupLayout, entry_point| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(
                    &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(label),
                        bind_group_layouts: &[Some(layout)],
                        ..Default::default()
                    }),
                ),
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
                    entry_point: Some(entry_point),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: SCENE_FORMAT,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::SrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent::OVER,
                        }),
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let composite = composite_pipeline(
            "RT lighting's clouds, composite",
            &composite_layout,
            "fs_composite",
        );
        let dome_composite = composite_pipeline(
            "RT lighting's clouds, dome composite",
            &dome_composite_layout,
            "fs_dome_composite",
        );
        let uniform_buffer = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<Uniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let no_distance = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Clouds (no distance)"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        use wgpu::AddressMode::{ClampToEdge, Repeat};
        Self {
            march,
            march_layout,
            resolve,
            resolve_layout,
            composite,
            composite_layout,
            dome_march,
            dome_resolve,
            dome_resolve_layout,
            dome_composite,
            dome_composite_layout,
            shadow,
            uniform: uniform_buffer("RT lighting's clouds"),
            dome_uniform: uniform_buffer("RT lighting's clouds, dome"),
            shadow_uniform: uniform_buffer("RT lighting's clouds, shadows"),
            shape,
            detail,
            weather,
            repeat: sampler(Repeat, Repeat),
            clamp: sampler(ClampToEdge, ClampToEdge),
            around: sampler(Repeat, ClampToEdge),
            no_distance,
            views: Vec::new(),
            dome: None,
            frame: 0,
        }
    }

    fn textures(&mut self, device: &wgpu::Device, view: usize, size: [u32; 2]) -> &mut ViewClouds {
        let i = match self.views.iter().position(|v| v.view == view) {
            Some(i) if self.views[i].size == size => i,
            found => {
                if let Some(i) = found {
                    self.views.remove(i);
                } else if self.views.len() == KEPT {
                    self.views.remove(0);
                }
                let low = size.map(|v| v.div_ceil(scale()).max(1));
                self.views.push(ViewClouds {
                    view,
                    size,
                    low,
                    raw: target(device, low, wgpu::TextureFormat::Rgba16Float),
                    history: [
                        target(device, low, wgpu::TextureFormat::Rgba16Float),
                        target(device, low, wgpu::TextureFormat::Rgba16Float),
                    ],
                    next: 0,
                    last: None,
                });
                self.views.len() - 1
            }
        };
        &mut self.views[i]
    }

    /// The march's bind group, writing `raw` (and `distance`, the dome's).
    fn march_group(
        &self,
        device: &wgpu::Device,
        uniform: &wgpu::Buffer,
        depth: &wgpu::TextureView,
        sky: &wgpu::TextureView,
        raw: &wgpu::TextureView,
        distance: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RT lighting's clouds, march"),
            layout: &self.march_layout,
            entries: &[
                buffer_entry(0, uniform),
                tex(1, depth),
                tex(2, &self.shape),
                tex(3, &self.detail),
                tex(4, &self.weather),
                sampler_entry(5, &self.repeat),
                tex(6, sky),
                sampler_entry(7, &self.clamp),
                tex(8, raw),
                tex(9, distance),
            ],
        })
    }

    /// Draws view `view`'s clouds (0 the window, then the VR views) onto `target`, the scene of
    /// `size`, where `depth` shows the sky, marched for this view alone; `view_proj` and `camera`
    /// are the view's, `sky` the atmosphere's table.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        view: usize,
        depth: &wgpu::TextureView,
        target: &wgpu::TextureView,
        size: [u32; 2],
        view_proj: Mat4,
        camera: DVec3,
        params: &Params,
        sky: &wgpu::TextureView,
    ) {
        self.frame = self.frame.wrapping_add(1);
        let frame = self.frame;
        let textures = self.textures(device, view, size);
        let low = textures.low;
        let last = textures.last.replace((view_proj, camera));
        let (written, past) = (textures.next, 1 - textures.next);
        textures.next = past;
        let raw = textures.raw.clone();
        let (history, previous) = (
            textures.history[written].clone(),
            textures.history[past].clone(),
        );
        let rows = params.rows(camera);
        let drift = rows.drift;
        let uniform = Uniform {
            inverse_view_proj: view_proj.inverse().to_cols_array_2d(),
            prev_view_proj: last.map_or([[0.0; 4]; 4], |l| l.0.to_cols_array_2d()),
            light: rows.light,
            light_color: rows.light_color,
            sun: rows.sun,
            camera: rows.camera,
            layers: rows.layers,
            drifts: rows.drifts,
            wind: [drift[0], drift[1], (frame % 4096) as f32, BLEND],
            motion: match last {
                Some((_, last_camera)) => {
                    let m = (last_camera - camera).as_vec3();
                    // Camera-relative points are the last frame's plus this move.
                    [-m.x, -m.y, -m.z, 1.0]
                }
                None => [0.0; 4],
            },
            size: [size[0] as f32, size[1] as f32, low[0] as f32, low[1] as f32],
            jitter: [
                halton(frame % 16 + 1, 2) - 0.5,
                halton(frame % 16 + 1, 3) - 0.5,
                scale() as f32,
                0.0,
            ],
        };
        uploads.buffer(encoder, &self.uniform, 0, bytemuck::bytes_of(&uniform));
        let groups = [
            self.march_group(device, &self.uniform, depth, sky, &raw, &self.no_distance),
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RT lighting's clouds, resolve"),
                layout: &self.resolve_layout,
                entries: &[
                    buffer_entry(0, &self.uniform),
                    tex(10, &raw),
                    tex(11, &previous),
                    sampler_entry(12, &self.clamp),
                    tex(13, &history),
                ],
            }),
        ];
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("RT lighting's clouds"),
                timestamp_writes: None,
            });
            for (pipeline, group) in [&self.march, &self.resolve].into_iter().zip(&groups) {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(low[0].div_ceil(8), low[1].div_ceil(8), 1);
            }
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RT lighting's clouds, composite"),
            layout: &self.composite_layout,
            entries: &[
                buffer_entry(0, &self.uniform),
                tex(1, depth),
                tex(14, &history),
                sampler_entry(15, &self.clamp),
            ],
        });
        composite_pass(encoder, target, &self.composite, &group);
    }

    /// Marches this frame's share of the dome around `camera` and folds it into the dome's history:
    /// once a frame, by the first view (`depth` is any depth texture: the march does not read it).
    #[allow(clippy::too_many_arguments)]
    pub fn update_dome(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        camera: DVec3,
        params: &Params,
        sky: &wgpu::TextureView,
    ) {
        let (size, checker) = dome_shape();
        let dome = self
            .dome
            .get_or_insert_with(|| Dome::new(device, size, checker));
        dome.frame = dome.frame.wrapping_add(1);
        let frame = dome.frame;
        let order = checker_order(dome.checker);
        let slot = order[frame as usize % order.len()];
        let last = dome.last.replace((camera, params.time));
        let (written, past) = (dome.next, 1 - dome.next);
        dome.next = past;
        let motion = last.and_then(|(last_camera, last_time)| {
            // A cloud seen now at a camera-relative point was where the wind had not yet moved it
            // (rt_clouds.wgsl: the lookups drift by the wind, the clouds the other way).
            let elapsed = params.time - last_time;
            let wind = DVec3::new(WIND[0] * elapsed, 0.0, WIND[1] * elapsed);
            let m = (camera - last_camera + wind).as_vec3();
            (m.length() < TELEPORT && elapsed.abs() < 5.0).then_some([m.x, m.y, m.z, 1.0])
        });
        let rows = params.rows(camera);
        let drift = rows.drift;
        let uniform = Uniform {
            inverse_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            prev_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            light: rows.light,
            light_color: rows.light_color,
            sun: rows.sun,
            camera: rows.camera,
            layers: rows.layers,
            drifts: rows.drifts,
            wind: [drift[0], drift[1], (frame % 4096) as f32, 0.0],
            motion: motion.unwrap_or([0.0; 4]),
            size: [
                dome.size[0] as f32,
                dome.size[1] as f32,
                dome.low[0] as f32,
                dome.low[1] as f32,
            ],
            jitter: [
                halton(frame % 16 + 1, 2) - 0.5,
                halton(frame % 16 + 1, 3) - 0.5,
                dome.checker as f32,
                slot as f32,
            ],
        };
        uploads.buffer(encoder, &self.dome_uniform, 0, bytemuck::bytes_of(&uniform));
        let dome = self.dome.as_ref().expect("made above");
        let march = self.march_group(
            device,
            &self.dome_uniform,
            depth,
            sky,
            &dome.raw,
            &dome.raw_distance,
        );
        let resolve = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RT lighting's clouds, dome resolve"),
            layout: &self.dome_resolve_layout,
            entries: &[
                buffer_entry(0, &self.dome_uniform),
                tex(16, &dome.raw),
                tex(17, &dome.raw_distance),
                tex(18, &dome.history[past]),
                tex(19, &dome.data[past]),
                sampler_entry(20, &self.around),
                tex(21, &dome.history[written]),
                tex(22, &dome.data[written]),
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("RT lighting's clouds, dome"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.dome_march);
        pass.set_bind_group(0, &march, &[]);
        pass.dispatch_workgroups(dome.low[0].div_ceil(8), dome.low[1].div_ceil(8), 1);
        pass.set_pipeline(&self.dome_resolve);
        pass.set_bind_group(0, &resolve, &[]);
        pass.dispatch_workgroups(dome.size[0].div_ceil(8), dome.size[1].div_ceil(8), 1);
    }

    /// Marches this frame's sixteenth of the clouds' shadow map `shadows` around `camera` (all of
    /// it after a jump): once a frame, by the first view (`depth` as [`Self::update_dome`]'s).
    #[allow(clippy::too_many_arguments)]
    pub fn update_shadows(
        &self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        camera: DVec3,
        params: &Params,
        sky: &wgpu::TextureView,
        shadows: &mut Shadows,
    ) {
        if shadows.view.texture().width() < 2 {
            return;
        }
        shadows.frame = shadows.frame.wrapping_add(1);
        let last = shadows.last.replace((camera, params.time));
        let whole = last.is_none_or(|(c, t)| {
            (c - camera).length() > f64::from(TELEPORT) || (params.time - t).abs() > 5.0
        });
        let checker = if whole { 1 } else { 4 };
        let order = checker_order(checker);
        let slot = order[shadows.frame as usize % order.len()];
        let low = SHADOW_SIZE / checker;
        let rows = params.rows(camera);
        let uniform = Uniform {
            inverse_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            prev_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            light: rows.light,
            light_color: rows.light_color,
            sun: rows.sun,
            camera: rows.camera,
            wind: [rows.drift[0], rows.drift[1], 0.0, 0.0],
            motion: [0.0; 4],
            size: [
                SHADOW_SIZE as f32,
                SHADOW_SIZE as f32,
                low as f32,
                low as f32,
            ],
            jitter: [0.0, 0.0, checker as f32, slot as f32],
            layers: rows.layers,
            drifts: rows.drifts,
        };
        uploads.buffer(
            encoder,
            &self.shadow_uniform,
            0,
            bytemuck::bytes_of(&uniform),
        );
        let group = self.march_group(
            device,
            &self.shadow_uniform,
            depth,
            sky,
            &shadows.view,
            &self.no_distance,
        );
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("RT lighting's clouds, shadows"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.shadow);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(low.div_ceil(8), low.div_ceil(8), 1);
    }

    /// Draws the dome's clouds onto `target`, the scene of `size` whose `depth` shows the sky, seen
    /// through `view_proj` (camera-relative). Nothing before the first [`Self::update_dome`].
    #[allow(clippy::too_many_arguments)]
    pub fn draw_dome(
        &self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        target: &wgpu::TextureView,
        size: [u32; 2],
        view_proj: Mat4,
    ) {
        let Some(dome) = &self.dome else {
            return;
        };
        let uniform = Uniform {
            inverse_view_proj: view_proj.inverse().to_cols_array_2d(),
            size: [size[0] as f32, size[1] as f32, 0.0, 0.0],
            ..bytemuck::Zeroable::zeroed()
        };
        uploads.buffer(encoder, &self.uniform, 0, bytemuck::bytes_of(&uniform));
        // The newest history: the one the last update wrote.
        let newest = &dome.history[1 - dome.next];
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RT lighting's clouds, dome composite"),
            layout: &self.dome_composite_layout,
            entries: &[
                buffer_entry(0, &self.uniform),
                tex(1, depth),
                sampler_entry(20, &self.around),
                tex(23, newest),
            ],
        });
        composite_pass(encoder, target, &self.dome_composite, &group);
    }
}

impl Dome {
    fn new(device: &wgpu::Device, size: [u32; 2], checker: u32) -> Self {
        let low = size.map(|v| v.div_ceil(checker));
        let light = || target(device, size, wgpu::TextureFormat::Rgba16Float);
        Self {
            size,
            low,
            checker,
            raw: target(device, low, wgpu::TextureFormat::Rgba16Float),
            raw_distance: target(device, low, wgpu::TextureFormat::R32Float),
            history: [light(), light()],
            data: [light(), light()],
            next: 0,
            last: None,
            frame: 0,
        }
    }
}

/// A texture the passes write and read, of `size` and `format`.
fn target(device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Clouds"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn tex(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn sampler_entry(binding: u32, sampler: &wgpu::Sampler) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::Sampler(sampler),
    }
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

/// The composite's full-view triangle over `target` (kept: the clouds blend over it).
fn composite_pass(
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("RT lighting's clouds, composite"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}

/// The `index`th element of the Halton sequence in `base`, in 0..1.
fn halton(mut index: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0, 0.0);
    while index > 0 {
        f /= base as f32;
        r += f * (index % base) as f32;
        index /= base;
    }
    r
}

/// The clouds' noise, 8 bits a channel: the shape (r: Perlin-Worley, gba: Worley at 4, 8 and 16
/// cells), the detail (rgb: Worley at 4, 8 and 16 cells) and the weather map (r: where the clouds
/// are, g: their height).
pub(super) struct Noise {
    pub(super) shape: Vec<u8>,
    pub(super) detail: Vec<u8>,
    pub(super) weather: Vec<u8>,
}

fn hash(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f)
        ^ seed.wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^ (h >> 15)
}

fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / (1 << 24) as f32
}

/// Perlin's gradient noise at `p` (in cells), repeating every `period` cells, about -0.7..0.7.
fn perlin(p: Vec3, period: i32, seed: u32) -> f32 {
    let cell = p.floor();
    let f = p - cell;
    let c = cell.as_ivec3();
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v, w) = (fade(f.x), fade(f.y), fade(f.z));
    let gradient = |i: i32, j: i32, k: i32, d: Vec3| {
        let h = hash(
            (c.x + i).rem_euclid(period),
            (c.y + j).rem_euclid(period),
            (c.z + k).rem_euclid(period),
            seed,
        );
        let g = Vec3::new(
            unit(h) * 2.0 - 1.0,
            unit(h.rotate_left(11) ^ 0x5bd1_e995) * 2.0 - 1.0,
            unit(h.rotate_left(21) ^ 0x1b87_3593) * 2.0 - 1.0,
        )
        .normalize_or(Vec3::Y);
        g.dot(d)
    };
    let corner =
        |i: i32, j: i32, k: i32| gradient(i, j, k, f - Vec3::new(i as f32, j as f32, k as f32));
    let x00 = corner(0, 0, 0) + u * (corner(1, 0, 0) - corner(0, 0, 0));
    let x10 = corner(0, 1, 0) + u * (corner(1, 1, 0) - corner(0, 1, 0));
    let x01 = corner(0, 0, 1) + u * (corner(1, 0, 1) - corner(0, 0, 1));
    let x11 = corner(0, 1, 1) + u * (corner(1, 1, 1) - corner(0, 1, 1));
    let y0 = x00 + v * (x10 - x00);
    let y1 = x01 + v * (x11 - x01);
    y0 + w * (y1 - y0)
}

/// Worley's noise at `p` (in cells), repeating every `cells`: 1 at a feature point, falling to 0
/// a cell away (billowy).
fn worley(p: Vec3, cells: i32, seed: u32) -> f32 {
    let cell = p.floor();
    let c = cell.as_ivec3();
    let mut nearest = 2.0f32;
    for k in -1..=1 {
        for j in -1..=1 {
            for i in -1..=1 {
                let n = c + glam::IVec3::new(i, j, k);
                let h = hash(
                    n.x.rem_euclid(cells),
                    n.y.rem_euclid(cells),
                    n.z.rem_euclid(cells),
                    seed,
                );
                let feature = n.as_vec3()
                    + Vec3::new(
                        unit(h),
                        unit(h.rotate_left(10) ^ 0x68e3_1da4),
                        unit(h.rotate_left(20) ^ 0xb529_7a4d),
                    );
                nearest = nearest.min((feature - p).length_squared());
            }
        }
    }
    1.0 - nearest.sqrt().min(1.0)
}

/// Fractal Perlin at `p` (in the unit cube), `octaves` of it from `cells` a side.
fn perlin_fbm(p: Vec3, cells: i32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amplitude, mut norm) = (0.0, 1.0, 0.0);
    for o in 0..octaves {
        let n = cells << o;
        sum += perlin(p * n as f32, n, seed + o) * amplitude;
        norm += amplitude;
        amplitude *= 0.5;
    }
    sum / norm
}

/// Stretches `values` over 0..1 (the 0.5th to the 99.5th percentile, so a few outliers do not
/// squash the rest).
fn stretch(values: &mut [f32]) {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let lo = sorted[sorted.len() / 200];
    let hi = sorted[sorted.len() - 1 - sorted.len() / 200];
    for v in values {
        *v = ((*v - lo) / (hi - lo).max(1e-6)).clamp(0.0, 1.0);
    }
}

fn bytes(values: &[f32]) -> impl Iterator<Item = u8> + '_ {
    values.iter().map(|v| (v * 255.0 + 0.5) as u8)
}

/// Runs `f` for every index of `0..count` on every core, collecting what it returns in order.
fn parallel<T: Send>(count: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let per = count.div_ceil(threads);
    std::thread::scope(|scope| {
        let f = &f;
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                scope.spawn(move || {
                    (t * per..((t + 1) * per).min(count))
                        .map(f)
                        .collect::<Vec<T>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("noise thread"))
            .collect()
    })
}

impl Noise {
    /// The noise, made once and shared (the mesh's worker and the march's textures read it).
    pub(super) fn shared() -> std::sync::Arc<Self> {
        static NOISE: std::sync::OnceLock<std::sync::Arc<Noise>> = std::sync::OnceLock::new();
        NOISE
            .get_or_init(|| std::sync::Arc::new(Self::generate()))
            .clone()
    }

    fn generate() -> Self {
        // The shape: a channel at a time, stretched to use all of 0..1.
        let n = SHAPE as usize;
        let at = |i: usize| {
            let (x, y, z) = (i % n, (i / n) % n, i / (n * n));
            (Vec3::new(x as f32, y as f32, z as f32) + 0.5) / n as f32
        };
        let pw = parallel(n * n * n, |i| {
            let p = at(i);
            let perlin = perlin_fbm(p, 4, 4, 1) * 0.5 + 0.5;
            let worley = worley(p * 4.0, 4, 11);
            // Perlin-Worley: the Perlin noise's bumps between the Worley cells' billows.
            worley + perlin * (1.0 - worley)
        });
        let worleys: Vec<Vec<f32>> = [4, 8, 16]
            .into_iter()
            .map(|cells| {
                parallel(n * n * n, |i| {
                    worley(at(i) * cells as f32, cells, 20 + cells as u32)
                })
            })
            .collect();
        let mut channels = vec![
            pw,
            worleys[0].clone(),
            worleys[1].clone(),
            worleys[2].clone(),
        ];
        for c in &mut channels {
            stretch(c);
        }
        let shape = (0..n * n * n)
            .flat_map(|i| (0..4).map(move |c| (c, i)))
            .map(|(c, i)| (channels[c][i] * 255.0 + 0.5) as u8)
            .collect();
        // The detail: Worley at 4, 8 and 16 cells of a 32 texel tile.
        let d = DETAIL as usize;
        let detail_at = |i: usize| {
            let (x, y, z) = (i % d, (i / d) % d, i / (d * d));
            (Vec3::new(x as f32, y as f32, z as f32) + 0.5) / d as f32
        };
        let mut detail_channels: Vec<Vec<f32>> = [4, 8, 16]
            .into_iter()
            .map(|cells| {
                parallel(d * d * d, |i| {
                    worley(detail_at(i) * cells as f32, cells, 40 + cells as u32)
                })
            })
            .collect();
        for c in &mut detail_channels {
            stretch(c);
        }
        let detail = (0..d * d * d)
            .flat_map(|i| {
                [
                    detail_channels[0][i],
                    detail_channels[1][i],
                    detail_channels[2][i],
                    1.0,
                ]
            })
            .map(|v| (v * 255.0 + 0.5) as u8)
            .collect();
        // The weather map: coverage from five octaves of Perlin from 4 cells, the height from
        // another noise.
        let w = WEATHER as usize;
        let slice = |seed: u32, cells: i32, octaves: u32| {
            let mut v = parallel(w * w, |i| {
                let p = Vec3::new((i % w) as f32 + 0.5, 0.5, (i / w) as f32 + 0.5) / w as f32;
                perlin_fbm(p, cells, octaves, seed)
            });
            stretch(&mut v);
            v
        };
        let (cover, height) = (slice(70, 6, 4), slice(90, 6, 3));
        let weather = bytes(&cover)
            .zip(bytes(&height))
            .flat_map(|(a, b)| [a, b])
            .collect();
        Self {
            shape,
            detail,
            weather,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_noise_tiles_and_uses_its_range() {
        // A lookup one tile away is the same: the noise wraps.
        let p = Vec3::new(0.3, 0.6, 0.1);
        let a = perlin_fbm(p, 4, 3, 1);
        let b = perlin_fbm(p + Vec3::X, 4, 3, 1);
        assert!((a - b).abs() < 1e-4, "{a} {b}");
        let a = worley(p * 4.0, 4, 11);
        let b = worley(p * 4.0 + Vec3::new(4.0, 4.0, 0.0), 4, 11);
        assert!((a - b).abs() < 1e-4, "{a} {b}");
        let weather = Noise::generate().weather;
        let cover: Vec<u8> = weather.iter().step_by(2).copied().collect();
        let share = cover.iter().filter(|v| **v > 128).count() as f32 / cover.len() as f32;
        assert!((0.3..0.7).contains(&share), "{share}");
    }

    #[test]
    fn the_dome_shape_and_the_weather_stay_in_range() {
        for side in [1, 2, 4] {
            let mut seen: Vec<u32> = checker_order(side).to_vec();
            seen.sort_unstable();
            assert_eq!(seen, (0..side * side).collect::<Vec<_>>());
        }
        for i in 0..2000 {
            let (bias, cirrus, alto) = weather(i as f64 * 37.3);
            assert!((-1.0..=1.0).contains(&bias) && (0.0..=1.0).contains(&cirrus));
            assert!((0.0..=1.0).contains(&alto));
        }
        // A day's weather drifts into the next's without a jump.
        let (a, _, _) = weather(DAY * 5.0 - 0.01);
        let (b, _, _) = weather(DAY * 5.0 + 0.01);
        assert!((a - b).abs() < 1e-3, "{a} {b}");
    }

    /// Renders the dome on whatever adapter there is (lavapipe will do) for a few skies and writes
    /// them as pictures into `HORIZON_CLOUD_STILLS` (a folder), to judge the clouds' look without
    /// the game: `HORIZON_CLOUD_STILLS=/tmp/stills cargo test -p horizon-client --lib
    /// rt_clouds::tests::stills -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn stills() {
        let Ok(folder) = std::env::var("HORIZON_CLOUD_STILLS") else {
            return;
        };
        let folder = std::path::PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let mut clouds = RtClouds::new(&device, &queue);
        let mut atmosphere = super::super::atmosphere::Atmosphere::new(&device);
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: None,
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
        let only: Option<String> = std::env::var("HORIZON_CLOUD_STILL").ok();
        // (name, sun elevation and azimuth in degrees, the view's yaw and pitch, the time, rain)
        // A day of much cirrus and few cumulus, for the sheet.
        let cirrus_day = (0..400)
            .map(|d| f64::from(d) * DAY + DAY * 0.5)
            .find(|t| {
                let (bias, cirrus, _) = weather(*t);
                cirrus > 0.85 && bias < -0.3
            })
            .unwrap_or(0.0);
        let alto_day = (0..400)
            .map(|d| f64::from(d) * DAY + DAY * 0.5)
            .find(|t| {
                let (bias, _, alto) = weather(*t);
                alto > 0.85 && bias < 0.0
            })
            .unwrap_or(0.0);
        let mut shadows = Shadows::new(&device, &queue);
        let skies: [(&str, f32, f32, f32, f32, f64, f32); 8] = [
            ("noon", 60.0, 30.0, 0.0, 12.0, 200.0, 0.0),
            ("afternoon_toward_sun", 25.0, 0.0, 0.0, 10.0, 1500.0, 0.0),
            ("afternoon_away", 25.0, 0.0, 180.0, 12.0, 1500.0, 0.0),
            ("sunset", 3.0, 0.0, 20.0, 6.0, 2900.0, 0.0),
            ("overhead", 45.0, 120.0, 0.0, 70.0, 4200.0, 0.0),
            ("rain", 40.0, 30.0, 90.0, 10.0, 600.0, 0.8),
            ("cirrus", 35.0, 200.0, 0.0, 25.0, cirrus_day, 0.0),
            ("altocumulus", 30.0, 160.0, 0.0, 30.0, alto_day, 0.0),
        ];
        for (name, elevation, azimuth, yaw, pitch, time, rain) in skies {
            if only.as_deref().is_some_and(|o| o != name) {
                continue;
            }
            println!("{name}: weather {:?}", weather(time));
            let (e, a) = (elevation.to_radians(), azimuth.to_radians());
            let sun = Vec3::new(e.cos() * a.cos(), e.sin(), e.cos() * a.sin());
            let overcast = if rain > 0.0 {
                super::super::atmosphere::overcast(rain, 0.0, 1.0)
            } else {
                glam::Vec4::ZERO
            };
            atmosphere.update(&queue, sun, 1.0, overcast);
            let light = atmosphere.sunlight(sun) * super::super::atmosphere::PAPER_WHITE;
            let light = if rain > 0.0 { light * 0.3 } else { light };
            let params = |frame: u32| Params {
                light: sun,
                light_color: light.to_array(),
                sun,
                time: time + f64::from(frame) / 90.0,
                thick: rain,
                curvature: 0.5 / 6_371_000.0,
            };
            let camera = DVec3::new(1000.0, 90.0, -400.0);
            clouds.dome = None;
            let frames = 48;
            for frame in 0..frames {
                let mut uploads = Uploads::new(&device);
                let mut encoder = device.create_command_encoder(&Default::default());
                clouds.update_dome(
                    &device,
                    &mut uploads,
                    &mut encoder,
                    &depth,
                    camera,
                    &params(frame),
                    &atmosphere.view,
                );
                uploads.finish(&encoder);
                queue.submit([encoder.finish()]);
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            }
            if name == "noon" {
                // The shadow map, whole (the first update), as the ground sees it from above.
                let mut uploads = Uploads::new(&device);
                let mut encoder = device.create_command_encoder(&Default::default());
                shadows.last = None;
                clouds.update_shadows(
                    &device,
                    &mut uploads,
                    &mut encoder,
                    &depth,
                    camera,
                    &params(0),
                    &atmosphere.view,
                    &mut shadows,
                );
                uploads.finish(&encoder);
                queue.submit([encoder.finish()]);
                let side = SHADOW_SIZE;
                let map = read_rgba16f(&device, &queue, &shadows.view, [side, side]);
                let grey: Vec<[f32; 3]> = map.iter().map(|t| [t[0] * 0.9; 3]).collect();
                write_png(&folder.join("shadows.png"), &grey, [side, side]);
            }
            let dome = clouds.dome.as_ref().unwrap();
            let size = dome.size;
            let texels = read_rgba16f(&device, &queue, &dome.history[1 - dome.next], size);
            let sky = read_rgba16f(
                &device,
                &queue,
                &atmosphere.view,
                super::super::atmosphere::SIZE,
            );
            let picture = perspective(&texels, size, &sky, sun, yaw, pitch);
            write_png(&folder.join(format!("{name}.png")), &picture, [960, 540]);
            // The dome itself, for the mapping.
            if name == "noon" {
                let flat: Vec<[f32; 3]> = (0..size[0] * size[1] / 4)
                    .map(|i| {
                        let (x, y) = (i % (size[0] / 2) * 2, i / (size[0] / 2) * 2);
                        let t = texels[(y * size[0] + x) as usize];
                        [t[0] + t[3] * 0.3, t[1] + t[3] * 0.45, t[2] + t[3] * 0.8]
                    })
                    .collect();
                write_png(&folder.join("dome.png"), &flat, [size[0] / 2, size[1] / 2]);
            }
        }
    }

    fn half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exponent = i32::from((bits >> 10) & 0x1f);
        let mantissa = f32::from(bits & 0x3ff);
        sign * match exponent {
            0 => mantissa / 1024.0 * 2f32.powi(-14),
            31 => f32::INFINITY,
            e => (1.0 + mantissa / 1024.0) * 2f32.powi(e - 15),
        }
    }

    fn read_rgba16f(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        size: [u32; 2],
    ) -> Vec<[f32; 4]> {
        let texture = view.texture();
        let row = (size[0] * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(size[1]),
                },
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap();
        let mut out = Vec::with_capacity((size[0] * size[1]) as usize);
        for y in 0..size[1] {
            for x in 0..size[0] {
                let at = (y * row + x * 8) as usize;
                out.push(std::array::from_fn(|c| {
                    half(u16::from_le_bytes([data[at + c * 2], data[at + c * 2 + 1]]))
                }));
            }
        }
        out
    }

    /// rt_clouds.wgsl `dome_uv`.
    fn dome_uv(d: Vec3) -> [f32; 2] {
        let e_min = -0.052_359_9f32;
        let u = (d.z.atan2(d.x) / std::f32::consts::TAU + 1.0).fract();
        let f = (d.y.clamp(-1.0, 1.0).asin() - e_min) / (std::f32::consts::FRAC_PI_2 - e_min);
        [u, ((1.0 + 8.0 * f.max(0.0)).sqrt() - 1.0) * 0.5]
    }

    fn bilinear(texels: &[[f32; 4]], size: [u32; 2], uv: [f32; 2]) -> [f32; 4] {
        let x = uv[0] * size[0] as f32 - 0.5;
        let y = (uv[1] * size[1] as f32 - 0.5).clamp(0.0, size[1] as f32 - 1.0);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |i: f32, j: f32| {
            let i = (i as i32).rem_euclid(size[0] as i32) as u32;
            let j = (j as i32).clamp(0, size[1] as i32 - 1) as u32;
            texels[(j * size[0] + i) as usize]
        };
        let (a, b, c, d) = (
            at(x0, y0),
            at(x0 + 1.0, y0),
            at(x0, y0 + 1.0),
            at(x0 + 1.0, y0 + 1.0),
        );
        std::array::from_fn(|k| {
            (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy
        })
    }

    /// The sky table toward `d` (rt_clouds.wgsl `sky_at`).
    fn sky_at(sky: &[[f32; 4]], sun: Vec3, d: Vec3) -> Vec3 {
        let size = super::super::atmosphere::SIZE;
        let flat = glam::Vec2::new(d.x, d.z).normalize_or_zero();
        let s = glam::Vec2::new(sun.x, sun.z).normalize_or_zero();
        let azimuth = flat.dot(s).clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
        let e = d.y.clamp(-1.0, 1.0).asin() / std::f32::consts::FRAC_PI_2;
        let v = 0.5 + 0.5 * e.signum() * e.abs().sqrt();
        let x = (azimuth * size[0] as f32 - 0.5).clamp(0.0, size[0] as f32 - 1.001);
        let y = (v * size[1] as f32 - 0.5).clamp(0.0, size[1] as f32 - 1.001);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |i: f32, j: f32| {
            let t = sky[(j as u32 * size[0] + i as u32) as usize];
            Vec3::new(t[0], t[1], t[2])
        };
        let top = at(x0, y0).lerp(at(x0 + 1.0, y0), fx);
        let bottom = at(x0, y0 + 1.0).lerp(at(x0 + 1.0, y0 + 1.0), fx);
        top.lerp(bottom, fy)
    }

    /// A 960x540 picture, 80 degrees across, looking toward `yaw` (degrees from +x toward +z) and
    /// `pitch` up: the sky table under the dome's clouds, in linear light.
    fn perspective(
        texels: &[[f32; 4]],
        size: [u32; 2],
        sky: &[[f32; 4]],
        sun: Vec3,
        yaw: f32,
        pitch: f32,
    ) -> Vec<[f32; 3]> {
        let (w, h) = (960u32, 540u32);
        let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
        let forward = Vec3::new(
            pitch.cos() * yaw.cos(),
            pitch.sin(),
            pitch.cos() * yaw.sin(),
        );
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward);
        let tan = (40f32).to_radians().tan();
        let mut out = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let sx = ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0) * tan;
                let sy = (1.0 - (y as f32 + 0.5) / h as f32 * 2.0) * tan * h as f32 / w as f32;
                let d = (forward + right * sx + up * sy).normalize();
                let mut color = sky_at(sky, sun, d);
                if d.dot(sun) > 0.99996 {
                    color += Vec3::splat(40.0);
                }
                if d.y < -0.02 {
                    color = Vec3::new(0.05, 0.06, 0.04);
                } else {
                    let c = bilinear(texels, size, dome_uv(d));
                    color = color * c[3] + Vec3::new(c[0], c[1], c[2]);
                }
                out.push(color.to_array());
            }
        }
        out
    }

    fn write_png(path: &std::path::Path, pixels: &[[f32; 3]], size: [u32; 2]) {
        // A plain filmic curve and sRGB, about the game's exposure for a day sky.
        let curve = |v: f32| {
            let v = v.max(0.0) * 0.6;
            let a = (v * (2.51 * v + 0.03)) / (v * (2.43 * v + 0.59) + 0.14);
            let a = a.clamp(0.0, 1.0);
            let s = if a <= 0.003_130_8 {
                a * 12.92
            } else {
                1.055 * a.powf(1.0 / 2.4) - 0.055
            };
            (s * 255.0 + 0.5) as u8
        };
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| p.map(curve)).collect();
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&bytes)
            .unwrap();
    }

    #[test]
    fn the_jitter_stays_in_the_texel() {
        for i in 1..=16 {
            for base in [2, 3] {
                let h = halton(i, base);
                assert!((0.0..1.0).contains(&h));
            }
        }
    }
}
