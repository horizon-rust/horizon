//! Heat haze for RT lighting (docs/raytracing.md, "Heat haze and fire"): the air above a flame
//! shimmers, the scene behind it refracted by a column of rising, turbulent hot air.
//!
//! The flames are the light sources of the flame blocks (`mesher.rs` `lights`, which tags each
//! with its kind in the level's high nibble; `Voxels::flames` lists those near the camera). Each
//! one is a small column of air above its flame, and the pass draws no geometry of it but a
//! screen rectangle around the column's projection, in two passes on the linear scene, after the
//! temporal resolve (so the shimmer is not smeared by the history):
//!
//!  1. `fs_distort` writes into a scratch target, inside the rectangles only, the scene sampled
//!     at an offset: a noise rising with the time, scaled by how deep the pixel's ray goes into
//!     the column (strongest just above the flame, gone with height, across the column's width
//!     and with distance), computed where the pixel's ray meets the plane through the column's
//!     middle, so the haze has a size in blocks whatever the view. What is in front of that
//!     plane (the depth buffer says) is not distorted, nor drawn from: the offset reads the
//!     depth too, so a pillar in front never leaks into the haze behind it;
//!  2. `fs_copy` writes the scratch back over the scene, the same rectangles.
//!
//! With no flame in view nothing is recorded; with some, the cost is the rectangles' pixels,
//! twice (a torch's is some 30 x 60 pixels). One code path for the window and the eyes of a
//! headset: the inputs are a view's own matrices and targets.

use super::upload::Uploads;
use glam::{DVec3, Mat4};

/// Flames farther than this from the camera give no haze (it fades from 14 blocks out).
pub const RANGE: f64 = 24.0;
/// At most this many columns a view, the nearest ones.
pub const MAX_FLAMES: usize = 32;

/// `HORIZON_HAZE=0` leaves the heat haze off, for comparison.
fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("HORIZON_HAZE").is_ok_and(|v| v == "0"))
}

/// One flame in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flame {
    /// Where it burns, in blocks.
    pub at: [f64; 3],
    /// Which of the kinds below.
    pub kind: u8,
}

impl Flame {
    pub const TORCH: u8 = 1;
    pub const FIRE: u8 = 2;
    pub const CAMPFIRE: u8 = 3;
    pub const CANDLE: u8 = 4;
    pub const LANTERN: u8 = 5;

    /// The column of air above a flame of this kind: how far above its light point it starts, its
    /// radius, its height (all in blocks) and how far it bends the light at most (in blocks, in
    /// the plane across the view). A torch's is half a block tall, a campfire's or a fire's a
    /// few, a candle's tiny.
    fn column(kind: u8) -> (f32, f32, f32, f32) {
        match kind {
            Self::FIRE => (0.3, 0.8, 3.0, 0.20),
            Self::CAMPFIRE => (0.25, 0.75, 3.0, 0.20),
            Self::CANDLE => (0.12, 0.07, 0.25, 0.02),
            Self::LANTERN => (0.15, 0.22, 0.6, 0.05),
            _ => (0.1, 0.2, 0.7, 0.06),
        }
    }
}

/// The flames among a section's light sources (`mesher.rs` `lights`), `sections` as `Voxels`
/// keeps them, within `range` of `camera`: nearest first, at most `MAX_FLAMES`.
pub fn nearby<'a>(
    sections: impl Iterator<Item = (&'a (i32, i32, i32), &'a Vec<[u8; 10]>)>,
    camera: DVec3,
    range: f64,
) -> Vec<Flame> {
    let mut found: Vec<(f64, Flame)> = Vec::new();
    for (&(x, sy, z), lights) in sections {
        // The section's centre, to leave out the far ones before looking at their lights.
        let centre = DVec3::new(
            f64::from(x * 16 + 8),
            f64::from(crate::chunk::MIN_Y + sy * 16 + 8),
            f64::from(z * 16 + 8),
        );
        if (centre - camera).length() > range + 14.0 {
            continue;
        }
        for &[lx, ly, lz, level, _, _, _, ax, ay, az] in lights {
            let kind = level >> 4;
            if kind == 0 {
                continue;
            }
            // Where in the block it burns (`mesher.rs` `light_at`), or its middle.
            let within = if ax == 255 {
                [0.5; 3]
            } else {
                [ax, ay, az].map(|v| f64::from(v) / 32.0)
            };
            let at = [
                f64::from(x * 16 + i32::from(lx)) + within[0],
                f64::from(crate::chunk::MIN_Y + sy * 16 + i32::from(ly)) + within[1],
                f64::from(z * 16 + i32::from(lz)) + within[2],
            ];
            let d = DVec3::from(at) - camera;
            if d.length_squared() <= range * range {
                found.push((d.length_squared(), Flame { at, kind }));
            }
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.truncate(MAX_FLAMES);
    found.into_iter().map(|(_, f)| f).collect()
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    view_proj: [[f32; 4]; 4],
    inverse_view_proj: [[f32; 4]; 4],
    /// The scene's width and height in pixels, and their inverses.
    size: [f32; 4],
    /// x: the time in seconds; y, z: the depth buffer's view depth as 1 / (y * depth + z).
    params: [f32; 4],
}

/// One column for the GPU, `base` relative to the camera.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Column {
    /// xyz: where the column starts; w: its radius.
    base: [f32; 4],
    /// x: its height; y: how far it bends the light; z: a number for this flame's noise alone.
    shape: [f32; 4],
}

/// A view's scratch target, of the scene's size.
struct Scratch {
    size: [u32; 2],
    view: wgpu::TextureView,
}

pub struct Haze {
    distort_layout: wgpu::BindGroupLayout,
    copy_layout: wgpu::BindGroupLayout,
    distort: wgpu::RenderPipeline,
    copy: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    columns: wgpu::Buffer,
    /// Each view's scratch target, by view index, made when a flame first shows in it.
    scratch: Vec<Option<Scratch>>,
}

/// What `Haze::run` takes.
pub struct Inputs<'a> {
    /// The view (0 the window, as `Taa`'s).
    pub view: usize,
    /// The scene (linear light, `tonemap::SCENE_FORMAT`) the haze is drawn over, its depth
    /// (reversed) and their size.
    pub color: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub size: [u32; 2],
    /// The view's projection without the jitter, relative to `camera`.
    pub view_proj: Mat4,
    pub camera: DVec3,
    /// The time in seconds.
    pub time: f32,
    pub flames: &'a [Flame],
}

impl Haze {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon heat haze"),
            source: wgpu::ShaderSource::Wgsl(include_str!("haze.wgsl").into()),
        });
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let texture = |sample_type| wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let uniform = entry(
            0,
            both,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        );
        let distort_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon heat haze"),
            entries: &[
                uniform,
                entry(
                    1,
                    fragment,
                    texture(wgpu::TextureSampleType::Float { filterable: true }),
                ),
                entry(2, fragment, texture(wgpu::TextureSampleType::Depth)),
                entry(
                    3,
                    fragment,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let copy_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon heat haze copy"),
            entries: &[
                uniform,
                entry(
                    4,
                    fragment,
                    texture(wgpu::TextureSampleType::Float { filterable: false }),
                ),
            ],
        });
        let pipeline = |label, layout: &wgpu::BindGroupLayout, fragment_entry| {
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
                    entry_point: Some("vs_haze"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Column>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment_entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: super::tonemap::SCENE_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            distort: pipeline("Horizon heat haze", &distort_layout, "fs_distort"),
            copy: pipeline("Horizon heat haze copy", &copy_layout, "fs_copy"),
            distort_layout,
            copy_layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Horizon heat haze"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Horizon heat haze"),
                size: std::mem::size_of::<Uniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            columns: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Horizon heat haze columns"),
                size: (MAX_FLAMES * std::mem::size_of::<Column>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            scratch: Vec::new(),
        }
    }

    /// Frees the scratch targets (RT lighting is off, or the title is up).
    pub fn release(&mut self) {
        self.scratch.clear();
    }

    /// Draws the haze of `inputs.flames` over `inputs.color`; records nothing with no flame.
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        inputs: Inputs,
    ) {
        if !enabled() || inputs.flames.is_empty() {
            return;
        }
        let columns: Vec<Column> = inputs
            .flames
            .iter()
            .take(MAX_FLAMES)
            .map(|flame| {
                let (lift, radius, height, bend) = Flame::column(flame.kind);
                let at = (DVec3::from(flame.at) - inputs.camera).as_vec3();
                // A number from the flame's own block, so that neighbours do not shimmer alike.
                let seed = (flame.at.map(|v| v.floor() as i32 as u32))
                    .iter()
                    .fold(0x9e37_79b9u32, |h, &v| {
                        (h ^ v).wrapping_mul(0x85eb_ca6b).rotate_left(13)
                    });
                Column {
                    base: [at.x, at.y + lift, at.z, radius],
                    shape: [height, bend, (seed >> 8) as f32 / (1u32 << 24) as f32, 0.0],
                }
            })
            .collect();
        let slot = &mut self.scratch;
        if slot.len() <= inputs.view {
            slot.resize_with(inputs.view + 1, || None);
        }
        if slot[inputs.view]
            .as_ref()
            .is_none_or(|s| s.size != inputs.size)
        {
            let view = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Horizon heat haze scratch"),
                    size: wgpu::Extent3d {
                        width: inputs.size[0].max(1),
                        height: inputs.size[1].max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: super::tonemap::SCENE_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            slot[inputs.view] = Some(Scratch {
                size: inputs.size,
                view,
            });
        }
        let scratch = &slot[inputs.view].as_ref().expect("made above").view;
        let [width, height] = inputs.size.map(|v| v.max(1) as f32);
        // The depth buffer's value to a view depth: the inverse projection's w row only holds
        // the depth and the constant (a perspective projection's), as TAA's `unproject` reads it.
        let inverse = inputs.view_proj.inverse();
        let uniform = Uniform {
            view_proj: inputs.view_proj.to_cols_array_2d(),
            inverse_view_proj: inverse.to_cols_array_2d(),
            size: [width, height, 1.0 / width, 1.0 / height],
            params: [inputs.time, inverse.col(2).w, inverse.col(3).w, 0.0],
        };
        uploads.buffer(encoder, &self.uniform, 0, bytemuck::bytes_of(&uniform));
        uploads.buffer(encoder, &self.columns, 0, bytemuck::cast_slice(&columns));
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let uniform_entry = entry(0, self.uniform.as_entire_binding());
        let distort_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon heat haze"),
            layout: &self.distort_layout,
            entries: &[
                uniform_entry.clone(),
                entry(1, wgpu::BindingResource::TextureView(inputs.color)),
                entry(2, wgpu::BindingResource::TextureView(inputs.depth)),
                entry(3, wgpu::BindingResource::Sampler(&self.sampler)),
            ],
        });
        let copy_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon heat haze copy"),
            layout: &self.copy_layout,
            entries: &[
                uniform_entry,
                entry(4, wgpu::BindingResource::TextureView(scratch)),
            ],
        });
        let count = columns.len() as u32;
        let mut pass = |label, target: &wgpu::TextureView, pipeline, group| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    // The scene is kept outside the rectangles; the scratch's own contents
                    // outside them are never read.
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.set_vertex_buffer(0, self.columns.slice(..));
            pass.draw(0..4, 0..count);
        };
        pass("Horizon heat haze", scratch, &self.distort, &distort_group);
        pass(
            "Horizon heat haze copy",
            inputs.color,
            &self.copy,
            &copy_group,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sections(
        lights: Vec<[u8; 10]>,
    ) -> std::collections::HashMap<(i32, i32, i32), Vec<[u8; 10]>> {
        std::collections::HashMap::from([((0, 20, 0), lights)])
    }

    #[test]
    fn only_the_tagged_lights_near_the_camera_are_flames() {
        let y = (20 * 16 + crate::chunk::MIN_Y) as f64;
        // A torch (kind 1, level 14) at its stick's top, a lamp (no kind) and a far campfire.
        let map = sections(vec![
            [3, 0, 4, 14 | Flame::TORCH << 4, 255, 135, 0, 16, 22, 16],
            [5, 0, 5, 15, 255, 205, 130, 255, 255, 255],
            [
                15,
                15,
                15,
                15 | Flame::CAMPFIRE << 4,
                255,
                135,
                0,
                255,
                255,
                255,
            ],
        ]);
        let near = nearby(map.iter(), DVec3::new(2.0, y, 2.0), 10.0);
        assert_eq!(near.len(), 1, "{near:?}");
        assert_eq!(near[0].kind, Flame::TORCH);
        assert_eq!(near[0].at, [3.5, y + 22.0 / 32.0, 4.5]);
        let all = nearby(map.iter(), DVec3::new(2.0, y, 2.0), 30.0);
        assert_eq!(all.len(), 2);
        assert!(all[0].kind == Flame::TORCH && all[1].kind == Flame::CAMPFIRE);
    }
}
