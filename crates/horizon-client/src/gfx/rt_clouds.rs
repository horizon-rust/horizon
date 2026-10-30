//! RT lighting's volumetric clouds (docs/raytracing.md, gap 6): cumulus in a slab 700 to 1300
//! blocks up, in place of vanilla's flat ones. Each view (the window, every VR eye) marches rays
//! through the slab at a fraction of its resolution into a small texture (`rt_clouds.wgsl`),
//! reprojects and blends the last frame's by view direction (the rays' start jittered a frame at
//! a time), and draws the result over the world's sky, before the tone mapping: the scene times
//! the clouds' transmittance, plus the light they send. The sun and moon's light, the direction
//! and the sky's table are RT lighting's own (`atmosphere`).
//!
//! The clouds' shape is noise made once on the CPU: a Perlin-Worley 64^3 with three Worley octaves
//! beside it for the shape, a Worley 32^3 for the erosion, and a 2D weather map of low-frequency
//! noise for where the clouds are and how tall. They tile, so wind is a drift of the lookups.

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

/// How many pixels of the view a cloud texel is on a side (`HORIZON_RT_CLOUD_SCALE`).
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

/// How RT lighting draws clouds (`HORIZON_RT_CLOUDS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Vanilla's flat clouds (`0`).
    Off,
    /// A mesh, as RTX Remix does (`rt_cloud_mesh`): the default.
    Mesh,
    /// The raymarched volume of this module (`march`).
    March,
}

pub fn mode() -> Mode {
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("HORIZON_RT_CLOUDS").as_deref() {
        Ok("0") => Mode::Off,
        Ok("march") => Mode::March,
        _ => Mode::Mesh,
    })
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
}

/// A frame's light and weather for the clouds.
pub struct Params {
    /// Toward the light the clouds are lit by (the sun's, or the moon's once the sun has set).
    pub light: Vec3,
    /// That light's colour, in paper whites (`Globals::rt_sun_light`).
    pub light_color: [f32; 3],
    /// Toward the sun, which the sky table's azimuth is from.
    pub sun: Vec3,
    /// The time in seconds, for the wind.
    pub time: f64,
    /// 0 for fair weather, 1 for the thickest overcast.
    pub thick: f32,
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

pub struct RtClouds {
    march: wgpu::ComputePipeline,
    march_layout: wgpu::BindGroupLayout,
    resolve: wgpu::ComputePipeline,
    resolve_layout: wgpu::BindGroupLayout,
    composite: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    shape: wgpu::TextureView,
    detail: wgpu::TextureView,
    weather: wgpu::TextureView,
    repeat: wgpu::Sampler,
    clamp: wgpu::Sampler,
    views: Vec<ViewClouds>,
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
        let sampler = |mode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Clouds"),
                address_mode_u: mode,
                address_mode_v: mode,
                address_mode_w: mode,
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
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let filterable = |dimension| wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: dimension,
            multisampled: false,
        };
        let sampler_binding = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let output = wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba16Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
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
        // The scene times the transmittance (the fragment's alpha) plus the light; the scene's own
        // alpha (the tone mapping's share of the sun) is left alone.
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("RT lighting's clouds, composite"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("RT lighting's clouds, composite"),
                    bind_group_layouts: &[Some(&composite_layout)],
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
                entry_point: Some("fs_composite"),
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
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RT lighting's clouds"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            march,
            march_layout,
            resolve,
            resolve_layout,
            composite,
            composite_layout,
            uniform,
            shape,
            detail,
            weather,
            repeat: sampler(wgpu::AddressMode::Repeat),
            clamp: sampler(wgpu::AddressMode::ClampToEdge),
            views: Vec::new(),
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
                let texture = || {
                    device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some("Clouds"),
                            size: wgpu::Extent3d {
                                width: low[0],
                                height: low[1],
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba16Float,
                            usage: wgpu::TextureUsages::STORAGE_BINDING
                                | wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        })
                        .create_view(&Default::default())
                };
                self.views.push(ViewClouds {
                    view,
                    size,
                    low,
                    raw: texture(),
                    history: [texture(), texture()],
                    next: 0,
                    last: None,
                });
                self.views.len() - 1
            }
        };
        &mut self.views[i]
    }

    /// Draws view `view`'s clouds (0 the window, then the VR views) onto `target`, the scene of
    /// `size`, where `depth` shows the sky; `view_proj` and `camera` are the view's, `sky` the
    /// atmosphere's table.
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
        let drift = |speed: f64| (params.time * speed).rem_euclid(WRAP) as f32;
        let uniform = Uniform {
            inverse_view_proj: view_proj.inverse().to_cols_array_2d(),
            prev_view_proj: last.map_or([[0.0; 4]; 4], |l| l.0.to_cols_array_2d()),
            light: [params.light.x, params.light.y, params.light.z, 0.0],
            light_color: [
                params.light_color[0],
                params.light_color[1],
                params.light_color[2],
                0.0,
            ],
            sun: [params.sun.x, params.sun.y, params.sun.z, 0.0],
            camera: [
                camera.x.rem_euclid(WRAP) as f32,
                camera.y as f32,
                camera.z.rem_euclid(WRAP) as f32,
                params.thick,
            ],
            wind: [drift(WIND[0]), drift(WIND[1]), (frame % 4096) as f32, BLEND],
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
        let tex = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let sampler = |binding, sampler| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Sampler(sampler),
        };
        let uniform_entry = wgpu::BindGroupEntry {
            binding: 0,
            resource: self.uniform.as_entire_binding(),
        };
        let groups = [
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RT lighting's clouds, march"),
                layout: &self.march_layout,
                entries: &[
                    uniform_entry.clone(),
                    tex(1, depth),
                    tex(2, &self.shape),
                    tex(3, &self.detail),
                    tex(4, &self.weather),
                    sampler(5, &self.repeat),
                    tex(6, sky),
                    sampler(7, &self.clamp),
                    tex(8, &raw),
                ],
            }),
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RT lighting's clouds, resolve"),
                layout: &self.resolve_layout,
                entries: &[
                    uniform_entry.clone(),
                    tex(10, &raw),
                    tex(11, &previous),
                    sampler(12, &self.clamp),
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
                uniform_entry,
                tex(1, depth),
                tex(14, &history),
                sampler(15, &self.clamp),
            ],
        });
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
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
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
    fn the_jitter_stays_in_the_texel() {
        for i in 1..=16 {
            for base in [2, 3] {
                let h = halton(i, base);
                assert!((0.0..1.0).contains(&h));
            }
        }
    }
}
