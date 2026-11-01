//! Far terrain drawn with mesh shaders (Vulkan and Metal, through naga's
//! WGSL mesh shader front end; DX12 has no HLSL mesh back end in the naga we
//! vendor, so it keeps the multi-draw path).
//!
//! The GPU walk (`lod_gpu`) keeps the tiles in view and, instead of indexed
//! draws, lists each tile's meshlets (`lod_meshlet`) in groups of 32. One
//! task workgroup per group (`lod_mesh.wgsl`'s `ts_main`) tests its meshlets
//! against the frustum and the eye, a mesh workgroup per survivor shades its
//! 16 quads' vertices read from the arena page, and terrain.wgsl's `fs_solid`
//! shades the pixels. One indirect mesh draw per arena page, its task count
//! written by the walk. The draw path stays the fallback: no feature, a
//! pipeline the device refuses, or the setting Off.
use crate::graphics;

/// Threads in a task workgroup: one meshlet each.
pub const TASK_THREADS: u32 = 32;
/// Threads in a mesh workgroup: one vertex each of a meshlet's 16 quads.
pub const MESH_THREADS: u32 = 64;
/// What a task workgroup hands its mesh workgroups: the tile, how many
/// meshlets survived and their indices (`lod_mesh.wgsl`'s `Payload`).
pub const PAYLOAD_BYTES: u32 = 8 + 4 * TASK_THREADS;

/// Whether this adapter's mesh shaders can run the far terrain's: the
/// feature on a backend naga writes mesh shaders for, a hardware adapter
/// (Mesa's software rasterizer fails on naga's mesh shader code), and
/// limits for a meshlet of 16 quads and the task shader's payload.
pub fn supported(adapter: &wgpu::Adapter) -> bool {
    let info = adapter.get_info();
    if !matches!(info.backend, wgpu::Backend::Vulkan | wgpu::Backend::Metal)
        || info.device_type == wgpu::DeviceType::Cpu
        || !adapter
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_MESH_SHADER)
    {
        return false;
    }
    let limits = adapter.limits();
    limits.max_task_invocations_per_workgroup >= TASK_THREADS
        && limits.max_task_invocations_per_dimension >= TASK_THREADS
        && limits.max_mesh_invocations_per_workgroup >= MESH_THREADS
        && limits.max_mesh_invocations_per_dimension >= MESH_THREADS
        && limits.max_mesh_output_vertices >= 4 * super::lod_meshlet::QUADS
        && limits.max_mesh_output_primitives >= 2 * super::lod_meshlet::QUADS
        && limits.max_task_payload_size >= PAYLOAD_BYTES
        && limits.max_task_workgroups_per_dimension >= 65_535
        && limits.max_mesh_workgroups_per_dimension >= TASK_THREADS
        && limits.max_storage_buffers_per_shader_stage >= 8
}

/// The device feature far terrain asks for: when the setting (or
/// `HORIZON_MESH_SHADERS=1`) says so and the adapter can run it.
pub fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
    if graphics::mesh_shaders_requested(adapter.get_info().backend) && supported(adapter) {
        wgpu::Features::EXPERIMENTAL_MESH_SHADER
    } else {
        wgpu::Features::empty()
    }
}

/// What the pipeline shares with the terrain's own.
pub struct Setup<'a> {
    /// The terrain bind group layout, group 0 (visible to the mesh stage).
    pub terrain_layout: &'a wgpu::BindGroupLayout,
    /// terrain.wgsl's module and its pipeline constants, for `fs_solid`.
    pub fragment: &'a wgpu::ShaderModule,
    pub constants: &'a [(&'a str, f64)],
    pub format: wgpu::TextureFormat,
}

/// The mesh pipeline and the layout of its group 1.
pub struct Pipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub layout: wgpu::BindGroupLayout,
}

fn entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl Pipeline {
    /// The pipeline, or `None` (with a warning) when the device refuses it.
    pub fn new(device: &wgpu::Device, setup: &Setup<'_>) -> Option<Self> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let task = wgpu::ShaderStages::TASK;
        let mesh = wgpu::ShaderStages::MESH;
        let uniform = wgpu::BufferBindingType::Uniform;
        let storage = wgpu::BufferBindingType::Storage { read_only: true };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Far tile meshlets"),
            entries: &[
                entry(0, task, uniform),
                entry(1, task | mesh, storage),
                entry(2, task, storage),
                entry(3, task | mesh, storage),
                entry(4, mesh, storage),
                entry(5, task, uniform),
            ],
        });
        let source = format!(
            "enable wgpu_mesh_shader;\n{}{}\n{}",
            crate::mesher::side_rules(),
            include_str!("../terrain.wgsl"),
            include_str!("lod_mesh.wgsl"),
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Far tile meshlets"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_mesh_pipeline(&wgpu::MeshPipelineDescriptor {
            label: Some("Far tile meshlets"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Far tile meshlets"),
                    bind_group_layouts: &[Some(setup.terrain_layout), Some(&layout)],
                    ..Default::default()
                }),
            ),
            task: Some(wgpu::TaskState {
                module: &module,
                entry_point: Some("ts_main"),
                compilation_options: Default::default(),
            }),
            mesh: wgpu::MeshState {
                module: &module,
                entry_point: Some("ms_main"),
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: setup.fragment,
                entry_point: Some("fs_solid"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: setup.constants,
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format: setup.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        if let Some(error) = pollster::block_on(scope.pop()) {
            log::warn!("Horizon renderer: mesh shaders are off for far terrain: {error}");
            return None;
        }
        Some(Self { pipeline, layout })
    }
}
