//! Far terrain drawn with mesh shaders (Vulkan and Metal, through naga's
//! WGSL mesh shader front end; DX12 has no HLSL mesh back end in the naga we
//! vendor, so it keeps the multi-draw path).
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
}

/// The device feature far terrain asks for: when the setting (or
/// `HORIZON_MESH_SHADERS=1`) says so and the adapter can run it.
pub fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
    if graphics::mesh_shaders_requested() && supported(adapter) {
        wgpu::Features::EXPERIMENTAL_MESH_SHADER
    } else {
        wgpu::Features::empty()
    }
}
