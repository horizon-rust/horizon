//! A Hi-Z depth pyramid of the world pass's depth, for the occlusion test in
//! `gpu_culling`'s `cull` pass.
//!
//! The `cull` pass runs before the frame draws anything, so the only depth it
//! can test against is the one the last frame left: the pyramid is built after
//! the world pass and read by the next frame's `cull`. A section that came into
//! sight since is tested against depth that does not know about it, which can
//! only mean an occluder the pyramid has not seen yet -- it hides less, never
//! more, except where the camera moved enough that last frame's depth no longer
//! covers this frame's view. `gpu_culling`'s own near-box bail (`near_or_big`)
//! leaves out the boxes a moving camera is closest to, which is where that
//! would show.
//!
//! Level 0 is half the depth's size and no level is padded to a power of two;
//! hiz.wgsl says why that is safe and what it saves.

/// Level 0 below this many texels a side: no pyramid, and no test. Under it the
/// whole thing is cheaper to skip than to set up.
const MIN_SIDE: u32 = 32;

/// Whether frames build a pyramid: `HORIZON_HIZ=0` stops them, which also
/// stops the world passes keeping their depth (`gfx.rs`'s `keep_depth`), so
/// the two can be measured against each other in one build.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("HORIZON_HIZ").as_deref() != Ok("0"))
}

/// hiz.wgsl's `Shape`: the depth's size, level 0's, and the sizes of the level
/// a reduction reads and the one it writes.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Shape {
    depth: [u32; 2],
    level0: [u32; 2],
    src: [u32; 2],
    dst: [u32; 2],
}

/// A uniform bound at a dynamic offset takes a multiple of 256 bytes, so each
/// level's `Shape` takes that much of `HiZ::shapes`.
const SHAPE_STRIDE: u64 = 256;

pub struct HiZ {
    seed: wgpu::RenderPipeline,
    reduce: wgpu::ComputePipeline,
    shape_layout: wgpu::BindGroupLayout,
    level_layout: wgpu::BindGroupLayout,
    /// One `Shape` a level, `SHAPE_STRIDE` apart.
    shapes: wgpu::Buffer,
    shape_capacity: u64,
    texture: Option<Pyramid>,
    /// `HORIZON_HIZ_DUMP`: copy the depth and every level to the CPU and print
    /// what each holds, for working out why a level reads as it does.
    dump: Option<Dump>,
}

struct Pyramid {
    texture: wgpu::Texture,
    /// Each level's size, level 0 first.
    sizes: Vec<[u32; 2]>,
    /// One view a level, for the passes that write them.
    mips: Vec<wgpu::TextureView>,
    /// Every level, for `cull` to sample.
    all: wgpu::TextureView,
    /// Group 0 of both pipelines: the shapes, and the depth the seed reads.
    shape_group: wgpu::BindGroup,
    /// The depth view `shape_group` holds, so a new one rebuilds it.
    seed_depth: wgpu::TextureView,
    /// Group 1, one a reduction: level n read, level n + 1 written.
    reduce_groups: Vec<wgpu::BindGroup>,
    /// What the depth it was seeded from measured, so a resize rebuilds.
    depth_size: (u32, u32),
}

/// One level's readback, in flight.
struct Dump {
    /// What is still to be copied this sweep; it ends when this empties.
    todo: Vec<u32>,
    buffer: wgpu::Buffer,
    /// What `buffer` holds and its size, once a copy is encoded.
    pending: Option<(u32, [u32; 2])>,
    described: bool,
}

impl HiZ {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Hi-Z"),
            source: wgpu::ShaderSource::Wgsl(include_str!("hiz.wgsl").into()),
        });
        let shape_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Hi-Z shape"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    // Unfilterable float, not `Depth`: see hiz.wgsl's `depth`.
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let level_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Hi-Z levels"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let both = [Some(&shape_layout), Some(&level_layout)];
        let reduce_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Hi-Z reduce"),
            bind_group_layouts: &both,
            ..Default::default()
        });
        let reduce = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Hi-Z reduce"),
            layout: Some(&reduce_layout),
            module: &module,
            entry_point: Some("reduce"),
            compilation_options: Default::default(),
            cache: None,
        });
        let only_shape = [Some(&shape_layout)];
        let seed_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Hi-Z seed"),
            bind_group_layouts: &only_shape,
            ..Default::default()
        });
        let seed = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Hi-Z seed"),
            layout: Some(&seed_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("seed_vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("seed_fragment"),
                targets: &[Some(wgpu::TextureFormat::R32Float.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: Default::default(),
            cache: None,
        });
        Self {
            seed,
            reduce,
            shape_layout,
            level_layout,
            shapes: shape_buffer(device, 16),
            shape_capacity: 16,
            texture: None,
            dump: std::env::var_os("HORIZON_HIZ_DUMP").is_some().then(|| Dump {
                todo: Vec::new(),
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Hi-Z dump"),
                    size: DUMP_BYTES,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                pending: None,
                described: false,
            }),
        }
    }

    /// Every level, for the `cull` pass's bind group; `None` until a frame has
    /// built one.
    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.texture.as_ref().map(|p| &p.all)
    }

    /// What `cull` needs to read it: the depth it was seeded from and how many
    /// levels the pyramid holds. Zero levels means no pyramid, and no test;
    /// level 0's size follows from the depth's, so it is not sent.
    pub fn shape_for_shader(&self) -> [u32; 4] {
        self.texture.as_ref().map_or([0; 4], |p| {
            [p.depth_size.0, p.depth_size.1, p.sizes.len() as u32, 0]
        })
    }

    /// Each level's size, level 0 first: half the depth, rounded up, then
    /// halving, rounded down, to one texel. Empty for a depth too small to be
    /// worth a pyramid.
    fn sizes_for(width: u32, height: u32) -> Vec<[u32; 2]> {
        let mut size = [width.div_ceil(2), height.div_ceil(2)];
        if size[0].min(size[1]) < MIN_SIDE {
            return Vec::new();
        }
        let mut sizes = vec![size];
        while size != [1, 1] {
            size = size.map(|s| (s >> 1).max(1));
            sizes.push(size);
        }
        sizes
    }

    /// Rebuilds the pyramid, its views and its bind groups when the depth has
    /// changed size or view, so that `build` only has to encode.
    fn ensure(&mut self, device: &wgpu::Device, depth: &wgpu::TextureView, w: u32, h: u32) {
        let built = self
            .texture
            .as_ref()
            .is_some_and(|p| p.depth_size == (w, h) && p.seed_depth == *depth);
        if built {
            return;
        }
        let sizes = Self::sizes_for(w, h);
        if sizes.is_empty() {
            self.texture = None;
            return;
        }
        let levels = sizes.len() as u32;
        if u64::from(levels) > self.shape_capacity {
            self.shape_capacity = u64::from(levels).next_power_of_two();
            self.shapes = shape_buffer(device, self.shape_capacity);
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Hi-Z pyramid"),
            size: wgpu::Extent3d {
                width: sizes[0][0],
                height: sizes[0][1],
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mips: Vec<_> = (0..levels)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("Hi-Z level"),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let all = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Hi-Z"),
            ..Default::default()
        });
        let shape_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Hi-Z shape"),
            layout: &self.shape_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.shapes,
                        offset: 0,
                        size: std::num::NonZeroU64::new(size_of::<Shape>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
            ],
        });
        let reduce_groups = (1..levels as usize)
            .map(|level| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Hi-Z levels"),
                    layout: &self.level_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&mips[level - 1]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&mips[level]),
                        },
                    ],
                })
            })
            .collect();
        self.texture = Some(Pyramid {
            texture,
            sizes,
            mips,
            all,
            shape_group,
            seed_depth: depth.clone(),
            reduce_groups,
            depth_size: (w, h),
        });
    }

    /// Builds the pyramid from `depth`, which must be the world pass's depth
    /// for the frame that has just drawn.
    pub fn build(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        self.ensure(device, depth, width, height);
        let Some(p) = &self.texture else { return };
        // One `Shape` a level: the seed's names the depth it reduces, each
        // reduction's the level below it and its own.
        let mut shapes = vec![0u8; SHAPE_STRIDE as usize * p.sizes.len()];
        for (level, size) in p.sizes.iter().enumerate() {
            let shape = Shape {
                depth: [width, height],
                level0: p.sizes[0],
                src: if level == 0 {
                    [width, height]
                } else {
                    p.sizes[level - 1]
                },
                dst: *size,
            };
            let at = level * SHAPE_STRIDE as usize;
            shapes[at..at + size_of::<Shape>()].copy_from_slice(bytemuck::bytes_of(&shape));
        }
        queue.write_buffer(&self.shapes, 0, &shapes);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Hi-Z seed"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &p.mips[0],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // The near plane, which a `min` absorbs: the triangle
                        // covers every texel, so this is never read.
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.seed);
            pass.set_bind_group(0, &p.shape_group, &[0]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Hi-Z"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.reduce);
            for (level, group) in p.reduce_groups.iter().enumerate() {
                let [w, h] = p.sizes[level + 1];
                let offset = ((level as u64 + 1) * SHAPE_STRIDE) as u32;
                pass.set_bind_group(0, &p.shape_group, &[offset]);
                pass.set_bind_group(1, group, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            }
        }
        self.dump(device, encoder, depth);
    }

    /// `HORIZON_HIZ_DUMP`: read the depth and one level back a frame and print
    /// what each holds, so a level of zeros can be told from a depth of zeros.
    /// The copy encoded here is read on the next frame, once the queue has run
    /// it.
    fn dump(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
    ) {
        let Some(levels) = self.texture.as_ref().map(|p| p.sizes.len() as u32) else {
            return;
        };
        let Some(dump) = &mut self.dump else { return };
        if !dump.described {
            dump.described = true;
            let t = depth.texture();
            println!(
                "hi-z depth texture {}x{} fmt {:?} mips {} samples {} usage {:?}",
                t.width(),
                t.height(),
                t.format(),
                t.mip_level_count(),
                t.sample_count(),
                t.usage(),
            );
        }
        if let Some((level, [w, h])) = dump.pending.take() {
            let slice = dump.buffer.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| ());
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            if let Ok(bytes) = slice.get_mapped_range() {
                let row = dump_row(w) as usize / 4;
                let words: &[f32] = bytemuck::cast_slice(&bytes);
                let (mut lo, mut hi, mut zeros, mut sum) = (f32::INFINITY, f32::MIN, 0u64, 0f64);
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let v = words[y * row + x];
                        lo = lo.min(v);
                        hi = hi.max(v);
                        sum += f64::from(v);
                        zeros += u64::from(v == 0.0);
                    }
                }
                let n = u64::from(w) * u64::from(h);
                let what = if level == DEPTH {
                    "DEPTH   ".to_string()
                } else {
                    format!("level {level:2}")
                };
                println!(
                    "hi-z {what} {w:5}x{h:<5} min {lo:.4e} max {hi:.4e} mean {:.4e} zero {:5.1}%",
                    sum / n as f64,
                    100.0 * zeros as f64 / n as f64,
                );
            }
            dump.buffer.unmap();
        }
        if dump.todo.is_empty() {
            // The depth first, then the levels, so a sweep reads them in order.
            dump.todo = (0..levels).rev().collect();
            dump.todo.push(DEPTH);
        }
        let Some(level) = dump.todo.pop() else { return };
        let (texture, mip, [w, h]) = if level == DEPTH {
            let t = depth.texture();
            (t, 0, [t.width(), t.height()])
        } else {
            let Some(p) = &self.texture else { return };
            (&p.texture, level, p.sizes[level as usize])
        };
        if u64::from(dump_row(w)) * u64::from(h) > DUMP_BYTES {
            return;
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: mip,
                origin: wgpu::Origin3d::ZERO,
                aspect: if level == DEPTH {
                    wgpu::TextureAspect::DepthOnly
                } else {
                    wgpu::TextureAspect::All
                },
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &dump.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(dump_row(w)),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        if let Some(dump) = &mut self.dump {
            dump.pending = Some((level, [w, h]));
        }
    }
}

fn shape_buffer(device: &wgpu::Device, levels: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Hi-Z shapes"),
        size: SHAPE_STRIDE * levels,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// `Dump::todo`'s marker for the depth texture itself.
const DEPTH: u32 = u32::MAX;

/// The dump buffer: enough for a 4K depth read as 32-bit texels.
const DUMP_BYTES: u64 = 3840 * 2160 * 4;

/// A level's row pitch in the dump buffer: four bytes a texel, padded to the
/// 256 a texture copy wants.
fn dump_row(texels: u32) -> u32 {
    (texels * 4).next_multiple_of(256)
}

#[cfg(test)]
mod tests {
    use super::{HiZ, MIN_SIDE};

    /// Level 0 is half the depth, and every level below it halves to one texel.
    #[test]
    fn levels_halve_from_half_the_depth_down_to_one_texel() {
        for (w, h) in [
            (1920, 1080),
            (1280, 720),
            (2560, 1440),
            (3840, 2160),
            (800, 600),
            (1365, 767),
        ] {
            let sizes = HiZ::sizes_for(w, h);
            assert_eq!(sizes[0], [w.div_ceil(2), h.div_ceil(2)], "{w}x{h} level 0");
            for pair in sizes.windows(2) {
                let (above, below) = (pair[0], pair[1]);
                assert_eq!(
                    below,
                    above.map(|s| (s >> 1).max(1)),
                    "{w}x{h}: {above:?} -> {below:?}"
                );
            }
            assert_eq!(*sizes.last().expect("levels"), [1, 1], "{w}x{h} ends at 1");
        }
    }

    /// Half-resolution levels of the depth's own shape cost a fraction of the
    /// full-resolution power-of-two square this replaced, which is the whole
    /// reason for the odd sizes hiz.wgsl has to handle.
    #[test]
    fn half_resolution_levels_cost_a_fraction_of_a_padded_square() {
        let texels: u64 = HiZ::sizes_for(1920, 1080)
            .iter()
            .map(|[w, h]| u64::from(*w) * u64::from(*h))
            .sum();
        let square: u64 = (0..12).map(|l| (2048u64 >> l) * (2048u64 >> l)).sum();
        assert!(
            texels * 8 < square,
            "{texels} texels is not under an eighth of {square}"
        );
    }

    /// A window too small to be worth a pyramid gets none, and `cull` then has
    /// no test to run rather than a one-texel one.
    #[test]
    fn a_tiny_window_gets_no_pyramid() {
        assert!(HiZ::sizes_for(16, 9).is_empty());
        // One texel under the floor on either axis is still no pyramid.
        assert!(HiZ::sizes_for(MIN_SIDE * 2, MIN_SIDE * 2 - 2).is_empty());
        assert!(HiZ::sizes_for(MIN_SIDE * 2 - 2, MIN_SIDE * 2).is_empty());
        // The smallest that does get one, with an odd level 0 to reduce.
        assert_eq!(HiZ::sizes_for(MIN_SIDE * 2 + 2, MIN_SIDE * 2 + 1)[0], [
            MIN_SIDE + 1,
            MIN_SIDE + 1
        ]);
    }
}
