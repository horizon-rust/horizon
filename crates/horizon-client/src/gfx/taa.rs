//! Temporal anti-aliasing for RT lighting (docs/raytracing.md, "Anti-aliasing"): what NVIDIA's RTX
//! Remix leaves to DLSS, written from published techniques (Karis 2014, "High Quality Temporal
//! Supersampling"; Salvi's variance clipping; a Halton(2, 3) jitter), none of NVIDIA's code.
//!
//! The world is drawn with a sub-pixel jitter in its projection, `PHASES` positions that repeat
//! (`jitter`, `jittered`). `Taa::resolve` then blends the drawn scene into a history of the resolved
//! frames (`taa.wgsl`): the history reprojected by depth and the camera's motion (the world is
//! still; what moves ghosts, bounded by the clipping to the neighbourhood's colour box), sampled
//! with Catmull-Rom, weighing a new frame 1/16 once a pixel's history is full and 1 where it was
//! just uncovered. The scene's linear light is what is resolved, before the exposure, the bloom
//! and the tone mapping read it, so they see the image that stays.
//!
//! **The rays use the jittered matrices too.** The depth prepass and the world are drawn with the
//! same jittered `view_proj`, which `shadow_rays` takes to rebuild each pixel's position from the
//! depth and to reproject into its history: both are exact for the jittered sample that was
//! drawn, so nothing in the rays' pass (the accumulation, the ReSTIR reuse, the denoiser) needs
//! the jitter as a number, and the light a pixel reads is the light at the sample the pixel
//! shows. The price is that the rays' history, reprojected onto a pixel grid that moves by a
//! fraction of a pixel each frame, is resampled bilinearly (a little soft light, no softer
//! albedo). The resolve itself unprojects the pixel's centre with the clean matrices: a still
//! camera maps each pixel onto itself.
//!
//! The first-person hand and the HUD are drawn after the resolve, unjittered, and are not
//! anti-aliased (as before). One history per view, as the rays have (the window, each VR eye).

use super::upload::Uploads;
use glam::{DVec3, Mat4, Vec3};

/// Jitter positions before the sequence repeats: Halton(2, 3) over 16 frames.
const PHASES: u32 = 16;
/// The camera moving farther than this in a frame is a cut (a teleport, a new world): the history
/// is forgotten.
const CUT_DISTANCE: f64 = 8.0;
/// The scene's format (`tonemap::SCENE_FORMAT`) and the aux's: the view depth and the frames
/// accumulated, half precision (a depth is compared within 4 %).
const AUX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// `HORIZON_TAA=0` leaves the anti-aliasing off, for comparison; on with RT lighting otherwise.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("HORIZON_TAA").is_ok_and(|v| v == "0"))
}

/// The `index`th number of the Halton sequence in `base`, in [0, 1).
fn halton(mut index: u32, base: u32) -> f32 {
    let (mut result, mut fraction) = (0.0, 1.0);
    while index > 0 {
        fraction /= base as f32;
        result += fraction * (index % base) as f32;
        index /= base;
    }
    result
}

/// The jitter of frame `frame`, in pixels within [-0.5, 0.5): how far the image moves right and
/// down. Halton's index starts at 1 (its first number, 0, is no jitter at all).
pub fn jitter(frame: u32) -> [f32; 2] {
    let index = frame % PHASES + 1;
    [halton(index, 2) - 0.5, halton(index, 3) - 0.5]
}

/// `view_proj` with its image moved by `jitter` pixels on a target of `size`: a shear of clip space
/// (x and y gain a multiple of w), so a point lands exactly that far from where it did.
pub fn jittered(view_proj: Mat4, jitter: [f32; 2], size: [u32; 2]) -> Mat4 {
    let x = 2.0 * jitter[0] / size[0].max(1) as f32;
    let y = -2.0 * jitter[1] / size[1].max(1) as f32;
    Mat4::from_cols_array(&[
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, 0.0, 1.0,
    ]) * view_proj
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    inverse_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    motion: [f32; 4],
    size: [f32; 4],
}

/// One view's resolved frames: two of them, as the resolve reads the last and writes the next.
struct History {
    size: [u32; 2],
    color: [wgpu::TextureView; 2],
    aux: [wgpu::TextureView; 2],
    /// Which of the two holds the last resolved frame.
    last: usize,
    /// Frames resolved since the history began: where the jitter is in its sequence.
    frame: u32,
    /// The clean view-projection and camera of the last resolved frame.
    camera: Option<(Mat4, DVec3)>,
}

impl History {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let texture = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
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
                .create_view(&Default::default())
        };
        let colors = |label| {
            [
                texture(label, super::tonemap::SCENE_FORMAT),
                texture(label, super::tonemap::SCENE_FORMAT),
            ]
        };
        let auxes = |label| [texture(label, AUX_FORMAT), texture(label, AUX_FORMAT)];
        Self {
            size,
            color: colors("TAA history"),
            aux: auxes("TAA history aux"),
            last: 0,
            frame: 0,
            camera: None,
        }
    }
}

/// What `Taa::resolve` takes.
pub struct Inputs<'a> {
    /// The view (0 the window, as `shadow_rays`' views).
    pub view: usize,
    /// The world drawn, which the resolve writes over, and its depth, both of `size`.
    pub color: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub size: [u32; 2],
    /// This frame's view-projection without the jitter, and the camera it is relative to.
    pub view_proj: Mat4,
    pub camera: DVec3,
}

pub struct Taa {
    resolve_layout: wgpu::BindGroupLayout,
    blit_layout: wgpu::BindGroupLayout,
    resolve: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    /// Each view's history, by view index.
    views: Vec<Option<History>>,
}

impl Taa {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon TAA"),
            source: wgpu::ShaderSource::Wgsl(include_str!("taa.wgsl").into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let texture = |sample_type| wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let loaded = texture(wgpu::TextureSampleType::Float { filterable: false });
        let filtered = texture(wgpu::TextureSampleType::Float { filterable: true });
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon TAA resolve"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(1, loaded),
                entry(2, texture(wgpu::TextureSampleType::Depth)),
                entry(3, filtered),
                entry(4, loaded),
                entry(
                    5,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon TAA blit"),
            entries: &[entry(6, loaded)],
        });
        let pipeline = |label,
                        layout: &wgpu::BindGroupLayout,
                        fragment,
                        targets: &[Option<wgpu::ColorTargetState>]| {
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
                    entry_point: Some("vs_taa"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let target = |format| {
            Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        Self {
            resolve: pipeline(
                "Horizon TAA resolve",
                &resolve_layout,
                "fs_resolve",
                &[target(super::tonemap::SCENE_FORMAT), target(AUX_FORMAT)],
            ),
            blit: pipeline(
                "Horizon TAA blit",
                &blit_layout,
                "fs_blit",
                &[target(super::tonemap::SCENE_FORMAT)],
            ),
            resolve_layout,
            blit_layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Horizon TAA"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Horizon TAA"),
                size: std::mem::size_of::<Uniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            views: Vec::new(),
        }
    }

    /// The jitter this view's next frame is drawn with, in pixels.
    pub fn jitter(&self, view: usize) -> [f32; 2] {
        // `HORIZON_TAA_JITTER=0` draws the world without it (the resolve still runs), to tell what
        // the jitter does from what the resolve does.
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !*ON.get_or_init(|| !std::env::var("HORIZON_TAA_JITTER").is_ok_and(|v| v == "0")) {
            return [0.0; 2];
        }
        let frame = self
            .views
            .get(view)
            .and_then(Option::as_ref)
            .map_or(0, |h| h.frame);
        jitter(frame)
    }

    /// The last resolved frame of view `view` (linear light, before the exposure), while its
    /// history runs: what the water's and the glass's screen-space reflections read
    /// (terrain.wgsl `rt_screen_reflection`).
    pub fn last_color(&self, view: usize) -> Option<wgpu::TextureView> {
        let history = self.views.get(view)?.as_ref()?;
        history.camera?;
        Some(history.color[history.last].clone())
    }
    /// Frees the histories (RT lighting is off, or the title is up): 120 MB at 3440x1440.
    pub fn release(&mut self) {
        self.views.clear();
    }

    /// Resolves `inputs.color` in place: the frame blended into the view's history, written over
    /// the scene for what reads it next.
    pub fn resolve(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        inputs: Inputs,
    ) {
        if self.views.len() <= inputs.view {
            self.views.resize_with(inputs.view + 1, || None);
        }
        let slot = &mut self.views[inputs.view];
        if slot.as_ref().is_none_or(|h| h.size != inputs.size) {
            *slot = Some(History::new(device, inputs.size));
        }
        let history = slot.as_mut().expect("the history was just made");
        // A new view, a resolution change or a cut forgets the past.
        let (previous, forget) = match history.camera {
            Some((view_proj, camera)) if (inputs.camera - camera).length() <= CUT_DISTANCE => {
                (view_proj, false)
            }
            _ => (inputs.view_proj, true),
        };
        let motion = match history.camera {
            Some((_, camera)) if !forget => (inputs.camera - camera).as_vec3(),
            _ => Vec3::ZERO,
        };
        let [width, height] = inputs.size.map(|v| v.max(1) as f32);
        let uniform = Uniform {
            inverse_view_proj: inputs.view_proj.inverse().to_cols_array_2d(),
            prev_view_proj: previous.to_cols_array_2d(),
            motion: [motion.x, motion.y, motion.z, f32::from(u8::from(forget))],
            size: [width, height, 1.0 / width, 1.0 / height],
        };
        uploads.buffer(encoder, &self.uniform, 0, bytemuck::bytes_of(&uniform));
        let (last, next) = (history.last, 1 - history.last);
        fn view(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            }
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon TAA resolve"),
            layout: &self.resolve_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                view(1, inputs.color),
                view(2, inputs.depth),
                view(3, &history.color[last]),
                view(4, &history.aux[last]),
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let attachment = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Horizon TAA resolve"),
                color_attachments: &[
                    attachment(&history.color[next]),
                    attachment(&history.aux[next]),
                ],
                ..Default::default()
            });
            pass.set_pipeline(&self.resolve);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon TAA blit"),
            layout: &self.blit_layout,
            entries: &[view(6, &history.color[next])],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Horizon TAA blit"),
                color_attachments: &[attachment(inputs.color)],
                ..Default::default()
            });
            pass.set_pipeline(&self.blit);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        history.last = next;
        history.frame = history.frame.wrapping_add(1);
        history.camera = Some((inputs.view_proj, inputs.camera));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    #[test]
    fn halton_is_the_radical_inverse() {
        let base2: Vec<f32> = (1..=4).map(|i| halton(i, 2)).collect();
        assert_eq!(base2, [0.5, 0.25, 0.75, 0.125]);
        let third: Vec<f32> = (1..=3).map(|i| halton(i, 3)).collect();
        assert!((third[0] - 1.0 / 3.0).abs() < 1e-6);
        assert!((third[1] - 2.0 / 3.0).abs() < 1e-6);
        assert!((third[2] - 1.0 / 9.0).abs() < 1e-6);
    }

    #[test]
    fn the_jitter_stays_within_a_pixel_and_repeats() {
        let mut seen = std::collections::HashSet::new();
        for frame in 0..PHASES {
            let [x, y] = jitter(frame);
            assert!((-0.5..0.5).contains(&x) && (-0.5..0.5).contains(&y));
            assert!(seen.insert((x.to_bits(), y.to_bits())));
            assert_eq!(jitter(frame), jitter(frame + PHASES));
        }
    }

    #[test]
    fn a_jittered_projection_moves_the_image_by_the_jitter() {
        let size = [3440, 1440];
        let view_proj =
            Mat4::perspective_rh(1.2, 3440.0 / 1440.0, 2048.0, 0.05) * Mat4::from_rotation_y(0.3);
        let point = Vec4::new(3.0, 1.0, -20.0, 1.0);
        let pixel = |m: Mat4| {
            let c = m * point;
            [
                (c.x / c.w * 0.5 + 0.5) * size[0] as f32,
                (0.5 - c.y / c.w * 0.5) * size[1] as f32,
            ]
        };
        let (still, moved) = (
            pixel(view_proj),
            pixel(jittered(view_proj, [0.25, -0.4], size)),
        );
        assert!(
            (moved[0] - still[0] - 0.25).abs() < 1e-3,
            "{still:?} {moved:?}"
        );
        assert!(
            (moved[1] - still[1] + 0.4).abs() < 1e-3,
            "{still:?} {moved:?}"
        );
        // The depth the rasteriser tests is the point's own.
        let depth = |m: Mat4| (m * point).z / (m * point).w;
        assert!((depth(view_proj) - depth(jittered(view_proj, [0.25, -0.4], size))).abs() < 1e-7);
    }
}
