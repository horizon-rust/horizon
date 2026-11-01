//! The occlusion test of far terrain tiles, on a depth pyramid.
//!
//! Far tiles are few and big, and boxes drawn against the depth (as
//! `cull.rs` tests sections) cost fill rate in proportion to the screen they
//! cover, which for a tile kilometres wide is most of it. This test costs the
//! same for every box. After phase 1 of occlusion culling, whose depth holds
//! the near sections and the far tiles the last test saw, a compute pass
//! reduces that depth to a pyramid, each texel the farthest depth of the
//! pixels under it (`hiz.wgsl`: `from_depth`, `down`), and a second runs one
//! thread per tile: the eight corners of its box give the pixels it covers
//! and the nearest depth it reaches, and a tile whose nearest depth is behind
//! the farthest the pyramid holds over those pixels is hidden. It is tested
//! at the level of the pyramid where its pixels take two texels at most.
//!
//! The answer goes where `cull.rs` collects its own: a tile that is seen
//! stores the frame's stamp in the test's result and gives its phase 2 draws
//! an instance, so it is drawn in the second pass; the results read back
//! later say which tiles the next frame draws in phase 1. A box is tested
//! against the depth of phase 1 only, which is farther than or as far as the
//! final depth, so a tile is never left out where it would show.
use glam::Vec3;
use std::num::NonZeroU64;

const GROUP: u32 = 64;
/// Threads of a pyramid pass in each direction.
const TILE: u32 = 8;
/// The most levels a pyramid has (a 2^16 pixel side).
const MAX_LEVELS: u32 = 16;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    stamp: u32,
    first: u32,
    count: u32,
    levels: u32,
    size: [f32; 2],
    padding: [u32; 2],
}

/// The depth's reduction: level `i` has `max(1, base >> i)` texels a side,
/// base being half the depth's size, so a level 0 texel covers 2x2 pixels.
struct Pyramid {
    /// The depth's size in pixels.
    size: [u32; 2],
    levels: u32,
    all: wgpu::TextureView,
    level_0: wgpu::TextureView,
    /// Level 0 from the depth: the depth it was bound to, and the group.
    from_depth: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    /// Level `i + 1` from level `i`, and the workgroups it takes.
    downs: Vec<(wgpu::BindGroup, [u32; 2])>,
    /// The workgroups level 0 takes.
    first_groups: [u32; 2],
}

pub struct Hiz {
    from_depth: wgpu::ComputePipeline,
    down: wgpu::ComputePipeline,
    test: wgpu::ComputePipeline,
    depth_layout: wgpu::BindGroupLayout,
    down_layout: wgpu::BindGroupLayout,
    test_layout: wgpu::BindGroupLayout,
    pyramid: Option<Pyramid>,
    /// This frame's tests: visible index, first phase 2 draw, phase 2 draws.
    tests: Vec<[u32; 4]>,
    /// Two per test: the low and the high corner of its box.
    boxes: Vec<[f32; 4]>,
    tests_buffer: wgpu::Buffer,
    boxes_buffer: wgpu::Buffer,
    capacity: u64,
    params: wgpu::Buffer,
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

const STORAGE_USAGE: wgpu::BufferUsages =
    wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_DST);

/// A pyramid of an `R32Float` level `base >> i` a side: its level count, and
/// each level's size.
fn level_sizes(depth: [u32; 2]) -> Vec<[u32; 2]> {
    let base = depth.map(|side| (side / 2).max(1));
    let levels = (base[0].max(base[1]).ilog2() + 1).min(MAX_LEVELS);
    (0..levels)
        .map(|i| base.map(|side| (side >> i).max(1)))
        .collect()
}

fn groups(size: [u32; 2]) -> [u32; 2] {
    size.map(|side| side.div_ceil(TILE))
}

impl Hiz {
    pub fn new(device: &wgpu::Device) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let storage = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let texture = |sample_type| wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let float = wgpu::TextureSampleType::Float { filterable: false };
        let level_out = wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::R32Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let depth_layout = layout(
            "Far terrain depth pyramid, level 0",
            &[
                entry(0, texture(wgpu::TextureSampleType::Depth)),
                entry(2, level_out),
            ],
        );
        let down_layout = layout(
            "Far terrain depth pyramid",
            &[entry(1, texture(float)), entry(2, level_out)],
        );
        let test_layout = layout(
            "Far terrain occlusion test",
            &[
                entry(3, storage(true)),
                entry(4, storage(true)),
                entry(5, storage(false)),
                entry(6, storage(false)),
                entry(7, storage(true)),
                entry(8, uniform),
                entry(9, uniform),
                entry(10, texture(float)),
            ],
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Far terrain occlusion"),
            source: wgpu::ShaderSource::Wgsl(include_str!("hiz.wgsl").into()),
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
                module: &module,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            from_depth: pipeline(
                "Far terrain depth pyramid, level 0",
                &depth_layout,
                "from_depth",
            ),
            down: pipeline("Far terrain depth pyramid", &down_layout, "down"),
            test: pipeline("Far terrain occlusion test", &test_layout, "test"),
            depth_layout,
            down_layout,
            test_layout,
            pyramid: None,
            tests: Vec::new(),
            boxes: Vec::new(),
            tests_buffer: buffer(device, "Far tile tests", 16, STORAGE_USAGE),
            boxes_buffer: buffer(device, "Far tile boxes", 32, STORAGE_USAGE),
            capacity: 1,
            params: buffer(
                device,
                "Far tile test parameters",
                std::mem::size_of::<Params>() as u64,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
        }
    }

    /// Start a frame: no tests.
    pub fn clear(&mut self) {
        self.tests.clear();
        self.boxes.clear();
    }

    /// How many tiles this frame tests.
    pub fn len(&self) -> usize {
        self.tests.len()
    }

    /// A tile to test: `test` is its visible index, its first phase 2 draw
    /// and how many it has there (0 for a tile phase 1 drew), `bounds` its
    /// box relative to its origin.
    pub fn push(&mut self, test: [u32; 3], bounds: [Vec3; 2]) {
        self.tests.push([test[0], test[1], test[2], 0]);
        self.boxes.push(bounds[0].extend(0.0).to_array());
        self.boxes.push(bounds[1].extend(0.0).to_array());
    }

    fn make_pyramid(&self, device: &wgpu::Device, depth: [u32; 2]) -> Pyramid {
        let sizes = level_sizes(depth);
        let levels = sizes.len() as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Far terrain depth pyramid"),
            size: wgpu::Extent3d {
                width: sizes[0][0],
                height: sizes[0][1],
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let level = |i: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("Far terrain depth pyramid level"),
                base_mip_level: i,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let views: Vec<_> = (0..levels).map(level).collect();
        let downs = (1..levels as usize)
            .map(|i| {
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Far terrain depth pyramid"),
                    layout: &self.down_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&views[i - 1]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&views[i]),
                        },
                    ],
                });
                (group, groups(sizes[i]))
            })
            .collect();
        Pyramid {
            size: depth,
            levels,
            all: texture.create_view(&Default::default()),
            level_0: views[0].clone(),
            from_depth: None,
            downs,
            first_groups: groups(sizes[0]),
        }
    }

    /// Before the passes: stage this frame's tests and parameters, and bind
    /// the pyramid to the world's `depth`, `size` pixels. The first test's
    /// result is `first`, and a test that sees its box stores `stamp`.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        size: [u32; 2],
        stamp: u32,
        first: u32,
    ) {
        let count = self.tests.len() as u64;
        if count == 0 {
            return;
        }
        let size = size.map(|side| side.max(1));
        if self.pyramid.as_ref().is_none_or(|p| p.size != size) {
            self.pyramid = Some(self.make_pyramid(device, size));
        }
        let pyramid = self.pyramid.as_mut().expect("made above");
        if pyramid
            .from_depth
            .as_ref()
            .is_none_or(|(bound, _)| bound != depth)
        {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Far terrain depth pyramid, level 0"),
                layout: &self.depth_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&pyramid.level_0),
                    },
                ],
            });
            pyramid.from_depth = Some((depth.clone(), group));
        }
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            self.tests_buffer = buffer(device, "Far tile tests", self.capacity * 16, STORAGE_USAGE);
            self.boxes_buffer = buffer(device, "Far tile boxes", self.capacity * 32, STORAGE_USAGE);
        }
        uploads.buffer(
            encoder,
            &self.tests_buffer,
            0,
            bytemuck::cast_slice(&self.tests),
        );
        uploads.buffer(
            encoder,
            &self.boxes_buffer,
            0,
            bytemuck::cast_slice(&self.boxes),
        );
        let params = Params {
            stamp,
            first,
            count: count as u32,
            levels: pyramid.levels,
            size: size.map(|side| side as f32),
            padding: [0; 2],
        };
        uploads.buffer(encoder, &self.params, 0, bytemuck::bytes_of(&params));
    }

    /// Between the phases: reduce the depth, then test the tiles. `results`
    /// and `phase2` are `cull.rs`'s, `origins` the frame's camera-relative
    /// origins and `globals` its uniforms (the view projection leads them).
    pub fn run(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        results: &wgpu::Buffer,
        phase2: &wgpu::Buffer,
        origins: &wgpu::Buffer,
        globals: &wgpu::Buffer,
    ) {
        let Some(pyramid) = &self.pyramid else {
            return;
        };
        let Some((_, from_depth)) = &pyramid.from_depth else {
            return;
        };
        if self.tests.is_empty() {
            return;
        }
        // One pass a level: each reads the one before it.
        let mut level = |label: &'static str,
                         pipeline: &wgpu::ComputePipeline,
                         group: &wgpu::BindGroup,
                         [x, y]: [u32; 2]| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(x, y, 1);
        };
        level(
            "Far terrain depth pyramid, level 0",
            &self.from_depth,
            from_depth,
            pyramid.first_groups,
        );
        for (group, size) in &pyramid.downs {
            level("Far terrain depth pyramid", &self.down, group, *size);
        }
        fn whole(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            }
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Far terrain occlusion test"),
            layout: &self.test_layout,
            entries: &[
                whole(3, &self.tests_buffer),
                whole(4, &self.boxes_buffer),
                whole(5, results),
                whole(6, phase2),
                whole(7, origins),
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: globals,
                        offset: 0,
                        size: NonZeroU64::new(64),
                    }),
                },
                whole(9, &self.params),
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&pyramid.all),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Far terrain occlusion test"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.test);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups((self.tests.len() as u32).div_ceil(GROUP), 1, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaders_validate() {
        let module = naga::front::wgsl::parse_str(include_str!("hiz.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn the_pyramid_halves_the_depth_down_to_one_texel() {
        let sizes = level_sizes([1920, 1080]);
        assert_eq!(sizes[0], [960, 540]);
        assert_eq!(sizes[1], [480, 270]);
        assert_eq!(*sizes.last().unwrap(), [1, 1]);
        assert_eq!(sizes.len(), 10);
        // A side of one pixel, or an odd one.
        assert_eq!(level_sizes([1, 1]), [[1, 1]]);
        assert_eq!(level_sizes([3, 1]), [[1, 1]]);
        assert_eq!(level_sizes([5, 7]).last(), Some(&[1, 1]));
        // The mip chain wgpu makes of a texture of the first level's size
        // has as many levels and sides as these.
        for depth in [[1920, 1080], [2561, 1441], [854, 480], [7, 3]] {
            let sizes = level_sizes(depth);
            let first = sizes[0];
            assert_eq!(
                sizes.len() as u32,
                first[0].max(first[1]).ilog2() + 1,
                "{depth:?}"
            );
            for (i, size) in sizes.iter().enumerate() {
                assert_eq!(*size, first.map(|side| (side >> i).max(1)));
            }
        }
    }

    #[test]
    fn a_params_block_matches_the_shader() {
        // Four words, a vec2<f32> (8-aligned) and a vec2<u32>.
        assert_eq!(std::mem::size_of::<Params>(), 32);
        assert_eq!(std::mem::offset_of!(Params, size), 16);
        assert_eq!(std::mem::offset_of!(Params, padding), 24);
    }
}
