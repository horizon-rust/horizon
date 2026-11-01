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
//! With mesh shaders on (`lod_mesh`) the walk lists each kept tile's meshlets
//! for a task shader instead of writing draws, and the page's mesh draw takes
//! the place of its multi-draw.
//!
//! The walk goes level by level with `dispatchIndirect`, with a frustum
//! test and a projected size test, here as a cull rather than
//! the choice of a level (Horizon's plan picks the tiles on the CPU).
use super::{lod_mesh, lod_meshlet::Meshlet};
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
    mask: [[u32; 4]; super::lod::MASK_VECS],
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
    /// The columns near chunks draw in full (`Renderer::lod_mask`).
    pub mask: [[u32; 4]; super::lod::MASK_VECS],
}

/// A buffer that grows by doubling, losing its contents when it does.
pub(super) struct Grown {
    pub buffer: wgpu::Buffer,
    size: u64,
    label: &'static str,
    usage: wgpu::BufferUsages,
}

impl Grown {
    pub fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
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
    pub fn ensure(&mut self, device: &wgpu::Device, bytes: u64) -> bool {
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

/// The mesh shader path's own resources (`lod_mesh`): the meshlet pipeline,
/// the traversal that lists meshlets for it instead of drawing, and the
/// lists.
struct MeshPath {
    pipeline: wgpu::RenderPipeline,
    /// Group 1 of `pipeline`.
    layout: wgpu::BindGroupLayout,
    traverse: wgpu::ComputePipeline,
    traverse_layout: wgpu::BindGroupLayout,
    traverse_group: Option<wgpu::BindGroup>,
    /// A task workgroup each: a tile and a group of its meshlets.
    items: Grown,
    /// Per page the task workgroups to dispatch, then 1 and 1.
    task_args: Grown,
    meshlets: Grown,
    /// Per arena page, its first item (a uniform the task shader reads).
    page_info: Vec<wgpu::Buffer>,
    /// This frame's bind groups of group 1, by arena page.
    page_groups: Vec<(u32, wgpu::BindGroup)>,
    /// The setting: the mesh shaders draw instead of the indexed draws.
    on: bool,
}

pub struct LodGpu {
    mesh: Option<MeshPath>,
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
    /// `HORIZON_GPU_LOD=1`: used where it would not be by default.
    forced: bool,
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

/// The traversal's WGSL: the constants it reads, the visibility test, the
/// shader; for the mesh shader path (`items`) with its lists.
pub(super) fn traverse_source(visibility: &str, items: bool) -> String {
    format!(
        "{}const FADE_MS: i32 = {};\nconst UV_STEP: f32 = {:?};\n{}\n{}\n{}",
        crate::mesher::side_rules(),
        tree::FADE_MS,
        1.0 / crate::mesher::TILED_UV_STEPS,
        if items {
            include_str!("lod_gpu_items.wgsl")
        } else {
            "const MESH_PATH: bool = false;\nfn push_items(ti: u32, page: u32) {}\n"
        },
        visibility,
        include_str!("lod_gpu.wgsl"),
    )
}

impl MeshPath {
    /// The traversal for meshlets, after `pipeline`, or `None` (with a warning)
    /// when the device refuses it.
    fn new(
        device: &wgpu::Device,
        pipeline: lod_mesh::Pipeline,
        visibility: &Visibility,
        usage: wgpu::BufferUsages,
    ) -> Option<Self> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let compute = wgpu::ShaderStages::COMPUTE;
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: compute,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }];
        entries.extend([
            storage(1, true),
            storage(2, true),
            storage(3, false),
            storage(4, false),
            storage(5, false),
            storage(6, false),
            storage(7, false),
            storage(8, false),
        ]);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Far tile meshlet traversal"),
            entries: &entries,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Far tile meshlet traversal"),
            source: wgpu::ShaderSource::Wgsl(traverse_source(&visibility.wgsl, true).into()),
        });
        let mut groups = vec![Some(&layout)];
        groups.extend(visibility.layout.as_ref().map(Some));
        let traverse = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Far tile meshlet traversal"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Far tile meshlet traversal"),
                    bind_group_layouts: &groups,
                    ..Default::default()
                }),
            ),
            module: &module,
            entry_point: Some("traverse"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = pollster::block_on(scope.pop()) {
            log::warn!("Horizon renderer: mesh shaders are off for far terrain: {error}");
            return None;
        }
        Some(Self {
            pipeline: pipeline.pipeline,
            layout: pipeline.layout,
            traverse,
            traverse_layout: layout,
            traverse_group: None,
            items: Grown::new(device, "Far tile meshlet items", usage),
            task_args: Grown::new(
                device,
                "Far tile task dispatches",
                usage | wgpu::BufferUsages::INDIRECT,
            ),
            meshlets: Grown::new(device, "Far tile meshlets", usage),
            page_info: Vec::new(),
            page_groups: Vec::new(),
            on: false,
        })
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
    /// On by default where the GPU can write the draw counts (Vulkan, DX12) or
    /// the mesh shader path is on, which draws a few indirect calls per page;
    /// elsewhere (Metal replays every entry of a multi-draw as a call, so the
    /// whole list would cost more than the walk it replaces) only with
    /// `HORIZON_GPU_LOD=1`; `0` turns it off (`active`).
    pub fn create(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        mesh: Option<lod_mesh::Setup<'_>>,
    ) -> Option<Self> {
        let counted = device
            .features()
            .contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT);
        let env = std::env::var("HORIZON_GPU_LOD");
        let wanted = match env.as_deref() {
            Ok("0") => false,
            Ok("1") => true,
            _ => counted || mesh.is_some(),
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
        let mut gpu = Self::new(device, counted, Visibility::default(), mesh)?;
        gpu.forced = env.as_deref() == Ok("1");
        Some(gpu)
    }

    /// Whether the walk beats the CPU's here: the GPU writes counts, the mesh
    /// shaders are on, or `HORIZON_GPU_LOD=1` says to.
    pub fn active(&self) -> bool {
        self.counted || self.forced || self.mesh_shaders()
    }

    /// The walk's pipelines, or `None` when the device refuses them (the
    /// caller keeps the CPU walk). `counted`: the device has
    /// `MULTI_DRAW_INDIRECT_COUNT`.
    pub fn new(
        device: &wgpu::Device,
        counted: bool,
        visibility: Visibility,
        mesh: Option<lod_mesh::Setup<'_>>,
    ) -> Option<Self> {
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
        let traverse_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Far tile traversal"),
            source: wgpu::ShaderSource::Wgsl(traverse_source(&visibility.wgsl, false).into()),
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
        let mesh = mesh.and_then(|setup| {
            let pipeline = lod_mesh::Pipeline::new(device, &setup)?;
            MeshPath::new(device, pipeline, &visibility, storage_usage)
        });
        Some(Self {
            mesh,
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
            forced: false,
            water: Vec::new(),
            animated: Vec::new(),
            threshold: Threshold::default(),
            walked: false,
        })
    }

    /// Binds the group 1 the visibility test reads (`Visibility::layout`).
    #[allow(dead_code)] // The hook stage 5's HiZ test binds its resources through.
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
        entries: &[Entry<'_>],
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
        // Each page's share of the list: draws, or with mesh shaders the task workgroups.
        let mesh_on = self.mesh.as_ref().is_some_and(|m| m.on);
        let per_tile = if self.counted { MOST_RUNS } else { 1 };
        let mut bases = vec![0u32; tree.tiles_per_page.len()];
        let mut first = 0;
        for (page, &tiles) in tree.tiles_per_page.iter().enumerate() {
            if tiles > 0 {
                let share = if mesh_on {
                    tree.groups_per_page[page]
                } else {
                    tiles * per_tile
                };
                bases[page] = first;
                self.segments.push(Segment {
                    page: page as u32,
                    first,
                    capacity: share,
                });
                first += share;
            }
        }
        self.draw_bytes = if mesh_on {
            0
        } else {
            u64::from(first) * DRAW_BYTES
        };
        let nodes = tree.nodes.len() as u64;
        let tiles = tree.tiles.len() as u64;
        let state_words = PAGES + 2 * u64::from(self.pages);
        let mut grown = false;
        grown |= self
            .nodes
            .ensure(device, nodes * std::mem::size_of::<tree::Node>() as u64);
        grown |= self
            .tiles
            .ensure(device, tiles * std::mem::size_of::<TileRecord>() as u64);
        grown |= self.queue.ensure(device, nodes * 8);
        grown |= self.state.ensure(device, state_words * 4);
        grown |= self.draws.ensure(device, self.draw_bytes);
        grown |= self.instances.ensure(device, tiles * 16);
        if grown {
            self.groups = None;
            if let Some(mesh) = &mut self.mesh {
                mesh.traverse_group = None;
            }
        }
        if mesh_on && let Some(mesh) = &mut self.mesh {
            let mut grown = false;
            grown |= mesh.items.ensure(device, u64::from(first) * 16);
            grown |= mesh.meshlets.ensure(
                device,
                tree.meshlets.len() as u64 * std::mem::size_of::<Meshlet>() as u64,
            );
            grown |= mesh.task_args.ensure(device, u64::from(self.pages) * 16);
            if grown {
                mesh.traverse_group = None;
            }
            uploads.buffer(
                encoder,
                &mesh.meshlets.buffer,
                0,
                bytemuck::cast_slice(&tree.meshlets),
            );
            // Each page's first item, for the task shader (`lod_mesh.wgsl`'s `page`).
            mesh.page_info.clear();
            for segment in &self.segments {
                let info = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Far tile page items"),
                    size: 16,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                uploads.buffer(
                    encoder,
                    &info,
                    0,
                    bytemuck::bytes_of(&[segment.first, 0u32, 0, 0]),
                );
                mesh.page_info.push(info);
            }
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

    /// The traversal's bindings, groups 0 of both its pipelines.
    fn base_entries(&self) -> Vec<wgpu::BindGroupEntry<'_>> {
        [
            &self.view,
            &self.nodes.buffer,
            &self.tiles.buffer,
            &self.queue.buffer,
            &self.state.buffer,
            &self.draws.buffer,
            &self.instances.buffer,
        ]
        .into_iter()
        .enumerate()
        .map(|(binding, buffer)| wgpu::BindGroupEntry {
            binding: binding as u32,
            resource: buffer.as_entire_binding(),
        })
        .collect()
    }

    fn make_groups(&self, device: &wgpu::Device) -> [wgpu::BindGroup; 2] {
        let traverse = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Far tile traversal"),
            layout: &self.traverse_layout,
            entries: &self.base_entries(),
        });
        let advance = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Far tile traversal levels"),
            layout: &self.advance_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.state.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.args.as_entire_binding(),
                },
            ],
        });
        [traverse, advance]
    }

    /// The meshlet traversal's group 0: the draw path's, with the lists.
    fn make_mesh_group(&self, device: &wgpu::Device, mesh: &MeshPath) -> wgpu::BindGroup {
        let mut entries = self.base_entries();
        entries.push(wgpu::BindGroupEntry {
            binding: 7,
            resource: mesh.task_args.buffer.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 8,
            resource: mesh.items.buffer.as_entire_binding(),
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Far tile meshlet traversal"),
            layout: &mesh.traverse_layout,
            entries: &entries,
        })
    }

    /// Records this frame's walk of the tree. Does nothing without tiles, or
    /// while the visibility test's bind group is missing; `draw` then draws
    /// nothing, and the caller keeps the CPU walk.
    pub fn walk(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut super::upload::Uploads,
        arena: &super::arena::Arena,
        frame: &Camera,
    ) -> bool {
        self.walked = false;
        if self.levels == 0 || (self.needs_visibility_group && self.visibility_group.is_none()) {
            return false;
        }
        if self.groups.is_none() {
            self.groups = Some(self.make_groups(device));
        }
        let mesh_on = self.mesh.as_ref().is_some_and(|m| m.on);
        if mesh_on {
            if self
                .mesh
                .as_ref()
                .is_some_and(|m| m.traverse_group.is_none())
            {
                let group = self.mesh.as_ref().map(|m| self.make_mesh_group(device, m));
                if let Some(mesh) = &mut self.mesh {
                    mesh.traverse_group = group;
                }
            }
            // This frame's bind groups of the pages' vertices (their buffers may have grown).
            let groups: Vec<(u32, wgpu::BindGroup)> = self
                .segments
                .iter()
                .enumerate()
                .filter(|(_, segment)| arena.has_page(segment.page))
                .filter_map(|(i, segment)| {
                    let mesh = self.mesh.as_ref()?;
                    Some((
                        segment.page,
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("Far tile meshlets"),
                            layout: &mesh.layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: self.view.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: self.instances.buffer.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: mesh.items.buffer.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: mesh.meshlets.buffer.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 4,
                                    resource: arena.buffer(segment.page).as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 5,
                                    resource: mesh.page_info.get(i)?.as_entire_binding(),
                                },
                            ],
                        }),
                    ))
                })
                .collect();
            if let Some(mesh) = &mut self.mesh {
                mesh.page_groups = groups;
            }
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
            mask: frame.mask,
        };
        uploads.buffer(encoder, &self.view, 0, bytemuck::bytes_of(&view));
        // Nothing walked, no draws yet; the first level is the roots.
        let counters = (PAGES + u64::from(self.pages)) * 4;
        encoder.clear_buffer(&self.state.buffer, 0, Some(counters));
        if mesh_on {
            // Per page no task workgroups yet, and the one and one the dispatch needs.
            let dispatches: Vec<u32> = (0..self.pages).flat_map(|_| [0, 1, 1, 0]).collect();
            if let Some(mesh) = &self.mesh {
                uploads.buffer(
                    encoder,
                    &mesh.task_args.buffer,
                    0,
                    bytemuck::cast_slice(&dispatches),
                );
            }
        } else if !self.counted {
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
        let [draw_group, advance_group] = self.groups.as_ref().expect("bind groups");
        let (traverse, traverse_group) = match self.mesh.as_ref().filter(|m| m.on) {
            Some(mesh) => match &mesh.traverse_group {
                Some(group) => (&mesh.traverse, group),
                None => return false,
            },
            None => (&self.traverse, draw_group),
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Far tile traversal"),
            timestamp_writes: None,
        });
        for level in 0..self.levels {
            pass.set_pipeline(traverse);
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

    /// Draws the solid quads of the tiles the walk kept into a pass whose
    /// terrain bind group is `terrain`: with `solid` (the terrain pipeline) and
    /// the quad `indices`, a multi-draw per arena page; or, with mesh shaders
    /// on, the meshlet pipeline and a mesh draw per page.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        arena: &super::arena::Arena,
        indices: &wgpu::Buffer,
        solid: &wgpu::RenderPipeline,
        terrain: &wgpu::BindGroup,
    ) {
        if !self.walked {
            return;
        }
        if let Some(mesh) = self.mesh.as_ref().filter(|m| m.on) {
            pass.set_pipeline(&mesh.pipeline);
            pass.set_bind_group(0, terrain, &[]);
            for (page, group) in &mesh.page_groups {
                pass.set_bind_group(1, group, &[]);
                pass.draw_mesh_tasks_indirect(&mesh.task_args.buffer, u64::from(*page) * 16);
            }
            return;
        }
        pass.set_pipeline(solid);
        pass.set_bind_group(0, terrain, &[]);
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

    /// Whether far terrain is drawn with mesh shaders: the device has them
    /// and the setting is on.
    pub fn mesh_shaders(&self) -> bool {
        self.mesh.as_ref().is_some_and(|m| m.on)
    }

    /// Whether the device can draw far terrain with mesh shaders at all.
    pub fn has_mesh_shaders(&self) -> bool {
        self.mesh.is_some()
    }

    /// Switches the mesh shader path (the Mesh Shaders setting, once the device
    /// has them); the lists are built again for the other path.
    pub fn set_mesh_shaders(&mut self, on: bool) {
        if let Some(mesh) = &mut self.mesh
            && mesh.on != on
        {
            mesh.on = on;
            self.revision = None;
        }
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
        meshlet_first: 0,
        meshlet_count: 0,
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
        // 6 planes, a matrix, five vectors and the column mask: lod_gpu.wgsl's `View`.
        assert_eq!(
            std::mem::size_of::<ViewUniform>(),
            96 + 64 + 5 * 16 + 128 * 16
        );
        assert_eq!(std::mem::size_of::<tree::Node>(), 48);
        assert_eq!(std::mem::size_of::<TileRecord>(), 88);
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
