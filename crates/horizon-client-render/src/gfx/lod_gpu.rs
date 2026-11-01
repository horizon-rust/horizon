//! Far terrain tiles walked and culled on the GPU.
//!
//! The CPU used to test every resident tile against the frustum each frame,
//! sort them, split each into the runs of quads that can face the eye and
//! record a draw for each (`gfx.rs`'s `visible_lods`). Here the tiles live on
//! the GPU as a tree (`tree.rs`) and a compute shader (`lod_gpu.wgsl`) walks
//! it a level at a time with indirect dispatches: a node outside the frustum,
//! hidden (`Visibility`) or too small to see takes everything under it with
//! it, and a tile that stays writes its own indirect draws and its instance
//! (origin and fade, from the clock) into buffers the frame's render pass
//! draws from. The CPU rebuilds the tree only when a tile is uploaded,
//! replaced, released or moved (`Renderer::lod_revision`), not per frame.
//!
//! Draws go out compacted into each arena page's share of the list, so one
//! indirect multi-draw per page: with a count the GPU wrote where the device
//! has `MULTI_DRAW_INDIRECT_COUNT` (Vulkan, DX12), else over the whole share,
//! its unused entries zero, with each tile drawn as one run of quads (a call
//! per entry costs the CPU on Metal, which replays them one by one).
//!
//! What stays on the CPU: tiles with water. Their translucent quads are
//! sorted back to front, which the order the GPU writes draws in cannot
//! give, so `water` names them for the CPU's old walk (solid quads of all
//! tiles are the GPU's). Frames drawn through a water mirror, a shader pack
//! or the title screen use the CPU walk altogether.
//!
//! The walk goes level by level with `dispatchIndirect`, with a frustum
//! test and a projected size test, here as a cull rather than
//! the choice of a level (Horizon's plan picks the tiles on the CPU).
use crate::lod::TileKey;
use glam::{DVec3, Mat4, Vec3, Vec4};
use std::borrow::Cow;
use tree::{Entry, TileRecord};

pub(super) mod tree;

/// Threads per workgroup of `lod_gpu.wgsl`'s `traverse`.
const GROUP: u32 = 64;
/// The state buffer's first words: the lengths of the two halves of the
/// queue, levels walked, and one spare, then each page's draws so far.
const PAGES: u64 = 4;
/// Draws a tile makes at most: its seven buckets of quads give at most four
/// runs the eye can see.
const MOST_RUNS: u32 = 4;
const DRAW_BYTES: u64 = 20;

/// Where a visibility test plugs in (stage 5's HiZ occlusion): WGSL that
/// declares `fn node_hidden(lo: vec3<f32>, hi: vec3<f32>) -> bool` for the
/// traversal to call on each node in the frustum, `lo` and `hi` the node's
/// box relative to the camera (see `lod_visibility.wgsl`), and the layout of
/// the group 1 bind group it reads from, bound with
/// `LodGpu::set_visibility_group`. The default test sees everything.
pub struct Visibility {
    pub wgsl: Cow<'static, str>,
    pub layout: Option<wgpu::BindGroupLayout>,
}

impl Default for Visibility {
    fn default() -> Self {
        Self {
            wgsl: Cow::Borrowed(include_str!("lod_visibility.wgsl")),
            layout: None,
        }
    }
}

/// The smallest a node's bounding sphere may project to, in pixels, before
/// the traversal leaves it out. Raised a little for every frame that runs
/// long and lowered for every short one, between `MIN` and `MAX`: frames
/// the GPU spends past the budget give up the smallest far tiles first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Threshold {
    pixels: f32,
}

impl Default for Threshold {
    fn default() -> Self {
        Self { pixels: Self::MIN }
    }
}

impl Threshold {
    const MIN: f32 = 1.0;
    const MAX: f32 = 8.0;
    const STEP: f32 = 0.05;
    /// A frame at 60 frames per second.
    const BUDGET_MS: f32 = 1000.0 / 60.0;

    pub fn pixels(self) -> f32 {
        self.pixels
    }

    /// Takes in a frame's GPU time.
    pub fn update(&mut self, frame_ms: f32) {
        if frame_ms > Self::BUDGET_MS * 1.1 {
            self.pixels += Self::STEP;
        } else if frame_ms < Self::BUDGET_MS * 0.8 {
            self.pixels -= Self::STEP;
        }
        self.pixels = self.pixels.clamp(Self::MIN, Self::MAX);
    }
}

/// `lod_gpu.wgsl`'s `View`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ViewUniform {
    planes: [[f32; 4]; 6],
    view_proj: [[f32; 4]; 4],
    cam_int: [i32; 4],
    cam_frac: [f32; 4],
    eye: [f32; 4],
    misc: [f32; 4],
    times: [i32; 4],
}

/// What one frame's walk needs of the camera.
pub struct Camera {
    pub planes: [Vec4; 6],
    pub view_proj: Mat4,
    pub camera: DVec3,
    /// The eye, camera-relative, where quads facing away are left out.
    pub eye: Option<Vec3>,
    /// Screen height / 2 / tan(fov / 2).
    pub focal: f32,
    /// Milliseconds on the renderer's clock (`Renderer::lod_millis`).
    pub now: i32,
}

/// A buffer that grows by doubling, losing its contents when it does.
struct Grown {
    buffer: wgpu::Buffer,
    size: u64,
    label: &'static str,
    usage: wgpu::BufferUsages,
}

impl Grown {
    fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        let size = 256;
        Self {
            buffer: Self::create(device, label, size, usage),
            size,
            label,
            usage,
        }
    }

    fn create(
        device: &wgpu::Device,
        label: &'static str,
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

    /// Room for `bytes`; true when the buffer is a new one (empty, and not
    /// the one the bind groups hold).
    fn ensure(&mut self, device: &wgpu::Device, bytes: u64) -> bool {
        if bytes <= self.size {
            return false;
        }
        self.size = bytes.next_power_of_two();
        self.buffer = Self::create(device, self.label, self.size, self.usage);
        true
    }
}

/// One arena page's share of the draw list.
struct Segment {
    page: u32,
    /// First draw of the share, and how many it holds at most.
    first: u32,
    capacity: u32,
}

pub struct LodGpu {
    traverse: wgpu::ComputePipeline,
    advance: wgpu::ComputePipeline,
    traverse_layout: wgpu::BindGroupLayout,
    advance_layout: wgpu::BindGroupLayout,
    visibility_group: Option<wgpu::BindGroup>,
    needs_visibility_group: bool,
    view: wgpu::Buffer,
    nodes: Grown,
    tiles: Grown,
    queue: Grown,
    state: Grown,
    draws: Grown,
    instances: Grown,
    args: wgpu::Buffer,
    /// The traverse and advance bind groups, until a buffer is replaced.
    groups: Option<[wgpu::BindGroup; 2]>,
    /// The `Renderer::lod_revision` the tree was built from.
    revision: Option<u64>,
    levels: u32,
    roots: u32,
    pages: u32,
    draw_bytes: u64,
    segments: Vec<Segment>,
    /// The GPU wrote a count for each page's draws (`MULTI_DRAW_INDIRECT_COUNT`).
    counted: bool,
    water: Vec<TileKey>,
    animated: Vec<u16>,
    threshold: Threshold,
    /// This frame's walk is recorded: `draw` has a list to draw.
    walked: bool,
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl LodGpu {
    /// The device feature that lets the GPU write each page's draw count.
    pub fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
        adapter.features() & wgpu::Features::MULTI_DRAW_INDIRECT_COUNT
    }

    /// Whether the device can run the walk: compute shaders, indirect
    /// execution and the storage buffers it binds.
    pub fn supported(adapter: &wgpu::Adapter, device: &wgpu::Device) -> bool {
        let flags = adapter.get_downlevel_capabilities().flags;
        let limits = device.limits();
        flags.contains(
            wgpu::DownlevelFlags::COMPUTE_SHADERS | wgpu::DownlevelFlags::INDIRECT_EXECUTION,
        ) && limits.max_storage_buffers_per_shader_stage >= 6
            && limits.max_compute_workgroup_size_x >= GROUP
    }

    /// The walk where this device should run it, or `None`: indirect draws
    /// with a first instance, compute shaders, and not the Intel Vulkan
    /// drivers whose first instance is broken (`flat::is_broken_intel_vulkan`).
    /// On by default where the GPU can write the draw counts (Vulkan, DX12);
    /// elsewhere (Metal replays every entry of a multi-draw as a call, so
    /// the whole list would cost more than the walk it replaces) only with
    /// `HORIZON_GPU_LOD=1`; `0` turns it off.
    pub fn create(adapter: &wgpu::Adapter, device: &wgpu::Device) -> Option<Self> {
        let counted = device
            .features()
            .contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT);
        let wanted = match std::env::var("HORIZON_GPU_LOD").as_deref() {
            Ok("0") => false,
            Ok("1") => true,
            _ => counted,
        };
        let indirect = device
            .features()
            .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
            && adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION);
        if !wanted
            || !indirect
            || !Self::supported(adapter, device)
            || super::flat::is_broken_intel_vulkan(&adapter.get_info())
        {
            return None;
        }
        Self::new(device, counted, Visibility::default())
    }

    /// The walk's pipelines, or `None` when the device refuses them (the
    /// caller keeps the CPU walk). `counted`: the device has
    /// `MULTI_DRAW_INDIRECT_COUNT`.
    pub fn new(device: &wgpu::Device, counted: bool, visibility: Visibility) -> Option<Self> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let compute = wgpu::ShaderStages::COMPUTE;
        let traverse_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Far tile traversal"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: compute,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1, true),
                storage(2, true),
                storage(3, false),
                storage(4, false),
                storage(5, false),
                storage(6, false),
            ],
        });
        let advance_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Far tile traversal levels"),
            entries: &[storage(0, false), storage(1, false)],
        });
        let source = format!(
            "{}const FADE_MS: i32 = {};\nconst UV_STEP: f32 = {:?};\n{}\n{}",
            crate::mesher::side_rules(),
            tree::FADE_MS,
            1.0 / crate::mesher::TILED_UV_STEPS,
            visibility.wgsl,
            include_str!("lod_gpu.wgsl"),
        );
        let traverse_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Far tile traversal"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let advance_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Far tile traversal levels"),
            source: wgpu::ShaderSource::Wgsl(include_str!("lod_gpu_advance.wgsl").into()),
        });
        let mut groups = vec![Some(&traverse_layout)];
        groups.extend(visibility.layout.as_ref().map(Some));
        let traverse = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Far tile traversal"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Far tile traversal"),
                    bind_group_layouts: &groups,
                    ..Default::default()
                }),
            ),
            module: &traverse_module,
            entry_point: Some("traverse"),
            compilation_options: Default::default(),
            cache: None,
        });
        let advance = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Far tile traversal levels"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Far tile traversal levels"),
                    bind_group_layouts: &[Some(&advance_layout)],
                    ..Default::default()
                }),
            ),
            module: &advance_module,
            entry_point: Some("advance"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = pollster::block_on(scope.pop()) {
            log::warn!("Horizon renderer: GPU far terrain is off, its shaders failed: {error}");
            return None;
        }
        let storage_usage = wgpu::BufferUsages::STORAGE;
        Some(Self {
            traverse,
            advance,
            traverse_layout,
            advance_layout,
            visibility_group: None,
            needs_visibility_group: visibility.layout.is_some(),
            view: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Far tile view"),
                size: std::mem::size_of::<ViewUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            nodes: Grown::new(device, "Far tile nodes", storage_usage),
            tiles: Grown::new(device, "Far tile records", storage_usage),
            queue: Grown::new(device, "Far tile queue", storage_usage),
            state: Grown::new(
                device,
                "Far tile traversal state",
                storage_usage | wgpu::BufferUsages::INDIRECT,
            ),
            draws: Grown::new(
                device,
                "Far tile draws",
                storage_usage | wgpu::BufferUsages::INDIRECT,
            ),
            instances: Grown::new(
                device,
                "Far tile instances",
                storage_usage | wgpu::BufferUsages::VERTEX,
            ),
            args: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Far tile dispatch"),
                size: 16,
                usage: wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            groups: None,
            revision: None,
            levels: 0,
            roots: 0,
            pages: 0,
            draw_bytes: 0,
            segments: Vec::new(),
            counted,
            water: Vec::new(),
            animated: Vec::new(),
            threshold: Threshold::default(),
            walked: false,
        })
    }

    /// Binds the group 1 the visibility test reads (`Visibility::layout`).
    pub fn set_visibility_group(&mut self, group: Option<wgpu::BindGroup>) {
        self.visibility_group = group;
    }

    /// Whether the tree is already built from `revision` of the tiles.
    pub fn is_current(&self, revision: u64) -> bool {
        self.revision == Some(revision)
    }

    /// The tiles with water, for the CPU's walk (`Tile::visible`).
    pub fn water(&self) -> &[TileKey] {
        &self.water
    }

    /// The animated sprite layers any resident tile uses: drawn whether or
    /// not it is in view, as the CPU cannot tell which the GPU kept.
    pub fn animated_layers(&self) -> &[u16] {
        &self.animated
    }

    /// Takes a frame's GPU time to tune the size threshold.
    pub fn frame_time(&mut self, gpu_ms: f32) {
        self.threshold.update(gpu_ms);
    }

    /// Builds the tree of `entries` (the tiles with solid quads) for tile
    /// set `revision`, and writes it to the GPU in `encoder`'s order.
    pub fn rebuild(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut super::upload::Uploads,
        revision: u64,
        entries: &[Entry],
        water: Vec<TileKey>,
        animated: Vec<u16>,
    ) {
        let tree = tree::build(entries);
        self.revision = Some(revision);
        self.water = water;
        self.animated = animated;
        self.levels = tree.levels;
        self.roots = tree.roots;
        self.pages = tree.tiles_per_page.len() as u32;
        self.segments.clear();
        self.draw_bytes = 0;
        if tree.nodes.is_empty() {
            return;
        }
        let per_tile = if self.counted { MOST_RUNS } else { 1 };
        let mut bases = vec![0u32; tree.tiles_per_page.len()];
        let mut first = 0;
        for (page, &tiles) in tree.tiles_per_page.iter().enumerate() {
            if tiles > 0 {
                bases[page] = first;
                self.segments.push(Segment {
                    page: page as u32,
                    first,
                    capacity: tiles * per_tile,
                });
                first += tiles * per_tile;
            }
        }
        self.draw_bytes = u64::from(first) * DRAW_BYTES;
        let nodes = tree.nodes.len() as u64;
        let tiles = tree.tiles.len() as u64;
        let state_words = PAGES + 2 * u64::from(self.pages);
        let mut grown = false;
        grown |= self.nodes.ensure(device, nodes * 48);
        grown |= self.tiles.ensure(device, tiles * 80);
        grown |= self.queue.ensure(device, nodes * 8);
        grown |= self.state.ensure(device, state_words * 4);
        grown |= self.draws.ensure(device, self.draw_bytes);
        grown |= self.instances.ensure(device, tiles * 16);
        if grown {
            self.groups = None;
        }
        uploads.buffer(
            encoder,
            &self.nodes.buffer,
            0,
            bytemuck::cast_slice(&tree.nodes),
        );
        uploads.buffer(
            encoder,
            &self.tiles.buffer,
            0,
            bytemuck::cast_slice(&tree.tiles),
        );
        uploads.buffer(
            encoder,
            &self.state.buffer,
            (PAGES + u64::from(self.pages)) * 4,
            bytemuck::cast_slice(&bases),
        );
    }

    fn make_groups(&self, device: &wgpu::Device) -> [wgpu::BindGroup; 2] {
        let whole = |buffer: &wgpu::Buffer| buffer.as_entire_binding();
        let traverse = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Far tile traversal"),
            layout: &self.traverse_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: whole(&self.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: whole(&self.nodes.buffer),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: whole(&self.tiles.buffer),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: whole(&self.queue.buffer),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: whole(&self.state.buffer),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: whole(&self.draws.buffer),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: whole(&self.instances.buffer),
                },
            ],
        });
        let advance = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Far tile traversal levels"),
            layout: &self.advance_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: whole(&self.state.buffer),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: whole(&self.args),
                },
            ],
        });
        [traverse, advance]
    }

    /// Records this frame's walk of the tree. Does nothing without tiles, or
    /// while the visibility test's bind group is missing; `draw` then draws
    /// nothing, and the caller keeps the CPU walk.
    pub fn walk(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut super::upload::Uploads,
        frame: &Camera,
    ) -> bool {
        self.walked = false;
        if self.levels == 0 || (self.needs_visibility_group && self.visibility_group.is_none()) {
            return false;
        }
        if self.groups.is_none() {
            self.groups = Some(self.make_groups(device));
        }
        let camera_block = frame.camera.floor();
        let fraction = (frame.camera - camera_block).as_vec3();
        let eye = frame.eye.unwrap_or(Vec3::ZERO);
        let view = ViewUniform {
            planes: frame.planes.map(|p| p.to_array()),
            view_proj: frame.view_proj.to_cols_array_2d(),
            cam_int: [
                camera_block.x as i32,
                camera_block.y as i32,
                camera_block.z as i32,
                0,
            ],
            cam_frac: [fraction.x, fraction.y, fraction.z, 0.0],
            eye: [
                eye.x,
                eye.y,
                eye.z,
                f32::from(u8::from(frame.eye.is_some())),
            ],
            misc: [frame.focal, self.threshold.pixels(), 0.0, 0.0],
            times: [frame.now, i32::from(!self.counted), self.pages as i32, 0],
        };
        uploads.buffer(encoder, &self.view, 0, bytemuck::bytes_of(&view));
        // Nothing walked, no draws yet; the first level is the roots.
        let counters = (PAGES + u64::from(self.pages)) * 4;
        encoder.clear_buffer(&self.state.buffer, 0, Some(counters));
        if !self.counted {
            encoder.clear_buffer(&self.draws.buffer, 0, Some(self.draw_bytes));
        }
        let roots: Vec<u32> = (0..self.roots).collect();
        uploads.buffer(encoder, &self.queue.buffer, 0, bytemuck::cast_slice(&roots));
        uploads.buffer(
            encoder,
            &self.state.buffer,
            0,
            bytemuck::bytes_of(&[self.roots, 0u32]),
        );
        uploads.buffer(
            encoder,
            &self.args,
            0,
            bytemuck::bytes_of(&[self.roots.div_ceil(GROUP), 1u32, 1, 0]),
        );
        let [traverse_group, advance_group] = self.groups.as_ref().expect("bind groups");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Far tile traversal"),
            timestamp_writes: None,
        });
        for level in 0..self.levels {
            pass.set_pipeline(&self.traverse);
            pass.set_bind_group(0, traverse_group, &[]);
            if let Some(group) = &self.visibility_group {
                pass.set_bind_group(1, group, &[]);
            }
            pass.dispatch_workgroups_indirect(&self.args, 0);
            if level + 1 < self.levels {
                pass.set_pipeline(&self.advance);
                pass.set_bind_group(0, advance_group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
        self.walked = true;
        true
    }

    /// Draws the solid quads of the tiles the walk kept, with the terrain
    /// pipeline and bind group already set: `indices` are the quad indices,
    /// the instances this module's, each arena page's vertices one call.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        arena: &super::arena::Arena,
        indices: &wgpu::Buffer,
    ) {
        if !self.walked {
            return;
        }
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_vertex_buffer(1, self.instances.buffer.slice(..));
        for segment in &self.segments {
            if !arena.has_page(segment.page) {
                continue;
            }
            pass.set_vertex_buffer(0, arena.buffer(segment.page).slice(..));
            let offset = u64::from(segment.first) * DRAW_BYTES;
            if self.counted {
                pass.multi_draw_indexed_indirect_count(
                    &self.draws.buffer,
                    offset,
                    &self.state.buffer,
                    (PAGES + u64::from(segment.page)) * 4,
                    segment.capacity,
                );
            } else {
                pass.multi_draw_indexed_indirect(&self.draws.buffer, offset, segment.capacity);
            }
        }
    }

    /// Whether this frame's walk was recorded.
    pub fn walked(&self) -> bool {
        self.walked
    }
}

/// A tile's record, for `tree::build`: its place in the arena and sort
/// order, with the times (`shown`, `dying`) on the renderer's clock.
pub(super) fn record(
    page: u32,
    base: u32,
    level: u8,
    sides: &crate::mesher::Sides,
    shown: Option<i32>,
    dying: Option<i32>,
    alone: bool,
) -> TileRecord {
    TileRecord {
        base,
        page,
        level: u32::from(level),
        shown: shown.unwrap_or(tree::NO_TIME),
        dying: dying.unwrap_or(tree::NO_TIME),
        alone: u32::from(alone),
        ends: sides.ends(),
        planes: sides.planes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_size_threshold_follows_the_frame_time_within_its_range() {
        let mut threshold = Threshold::default();
        assert_eq!(threshold.pixels(), 1.0);
        for _ in 0..1000 {
            threshold.update(30.0);
        }
        assert_eq!(threshold.pixels(), 8.0, "slow frames stop at the top");
        threshold.update(16.7);
        assert_eq!(threshold.pixels(), 8.0, "frames near the budget leave it");
        for _ in 0..1000 {
            threshold.update(5.0);
        }
        assert_eq!(threshold.pixels(), 1.0, "short frames bring it back");
    }

    #[test]
    fn the_view_uniform_is_what_the_shader_declares() {
        // 6 planes, a matrix, and five vectors: lod_gpu.wgsl's `View`.
        assert_eq!(std::mem::size_of::<ViewUniform>(), 96 + 64 + 5 * 16);
        assert_eq!(std::mem::size_of::<tree::Node>(), 48);
        assert_eq!(std::mem::size_of::<TileRecord>(), 80);
    }

    #[test]
    fn a_tile_record_keeps_a_meshs_buckets() {
        let sides = crate::mesher::Sides::unsorted(4);
        let r = record(2, 8, 3, &sides, Some(5), None, false);
        assert_eq!((r.page, r.base, r.level, r.shown), (2, 8, 3, 5));
        assert_eq!((r.dying, r.alone), (tree::NO_TIME, 0));
        assert_eq!(r.ends, sides.ends());
    }
}
