//! Whether this machine's GPU traces hardware rays through wgpu (docs/raytracing.md, the hybrid):
//! one triangle in a bottom-level acceleration structure, an instance of it in a top-level one,
//! and a compute pass's ray queries against it, one an opaque hit, one through the candidate
//! path an alpha test would take (`rayQueryConfirmIntersection`). Prints what came back.
//!
//!     cargo run -p horizon-client --example rt_query_probe

use wgpu::util::DeviceExt;

const SHADER: &str = r"
enable wgpu_ray_query;
@group(0) @binding(0) var scene: acceleration_structure;
@group(0) @binding(1) var<storage, read_write> out: array<f32, 4>;
@compute @workgroup_size(1) fn main() {
    // Straight down onto the triangle at y = 0: an opaque hit, its distance.
    var q: ray_query;
    rayQueryInitialize(&q, scene, RayDesc(0x01u, 0xffu, 0.0, 100.0, vec3(0.2, 5.0, 0.2), vec3(0.0, -1.0, 0.0)));
    rayQueryProceed(&q);
    let hit = rayQueryGetCommittedIntersection(&q);
    out[0] = select(-1.0, hit.t, hit.kind != RAY_QUERY_INTERSECTION_NONE);
    // The same, the triangle taken as not opaque: its candidate confirmed by hand, as an alpha
    // test does once it has read the texel at the candidate's barycentrics.
    var c: ray_query;
    rayQueryInitialize(&c, scene, RayDesc(0x02u, 0xffu, 0.0, 100.0, vec3(0.2, 5.0, 0.2), vec3(0.0, -1.0, 0.0)));
    var candidates = 0.0;
    while rayQueryProceed(&c) {
        let candidate = rayQueryGetCandidateIntersection(&c);
        candidates += 1.0;
        out[2] = candidate.barycentrics.x;
        rayQueryConfirmIntersection(&c);
    }
    let confirmed = rayQueryGetCommittedIntersection(&c);
    out[1] = select(-1.0, confirmed.t, confirmed.kind != RAY_QUERY_INTERSECTION_NONE);
    out[3] = candidates;
}
";

fn main() {
    pollster::block_on(run());
}

async fn run() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        })
        .await
        .expect("an adapter");
    let info = adapter.get_info();
    let supported = adapter
        .features()
        .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
    println!(
        "{} ({:?}, {}): ray queries {supported}",
        info.name, info.backend, info.driver
    );
    if !supported {
        return;
    }
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("ray query probe"),
            required_features: wgpu::Features::EXPERIMENTAL_RAY_QUERY,
            required_limits: wgpu::Limits::default()
                .using_minimum_supported_acceleration_structure_values(),
            // SAFETY: the probe's purpose is to try the experimental ray queries.
            experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            ..Default::default()
        })
        .await
        .expect("a device with ray queries");
    let vertices: [[f32; 3]; 3] = [[-1.0, 0.0, -1.0], [1.0, 0.0, -1.0], [0.0, 0.0, 1.0]];
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("triangle"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::BLAS_INPUT,
    });
    let size = wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: 3,
        index_format: None,
        index_count: None,
        flags: wgpu::AccelerationStructureGeometryFlags::empty(),
    };
    let blas = device.create_blas(
        &wgpu::CreateBlasDescriptor {
            label: Some("triangle"),
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        },
        wgpu::BlasGeometrySizeDescriptors::Triangles {
            descriptors: vec![size.clone()],
        },
    );
    let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
        label: Some("scene"),
        max_instances: 1,
        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
    });
    tlas[0] = Some(wgpu::TlasInstance::new(
        &blas,
        [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        0,
        0xff,
    ));
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("out"),
        size: 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("read"),
        size: 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("probe"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("probe"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("probe"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::AccelerationStructure(&tlas),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: out.as_entire_binding(),
            },
        ],
    });
    let started = std::time::Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.build_acceleration_structures(
        [&wgpu::BlasBuildEntry {
            blas: &blas,
            geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                size: &size,
                vertex_buffer: &vertex_buffer,
                first_vertex: 0,
                vertex_stride: 12,
                index_buffer: None,
                first_index: None,
                transform_buffer: None,
                transform_buffer_offset: None,
            }]),
        }],
        [&tlas],
    );
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&out, 0, &read, 0, 16);
    queue.submit([encoder.finish()]);
    read.slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("mapped"));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("polled");
    let values: Vec<f32> =
        bytemuck::cast_slice(&read.slice(..).get_mapped_range().expect("mapped")).to_vec();
    println!(
        "opaque hit t {} (5 expected); alpha-tested hit t {} after {} candidate(s), barycentric u {} ({:?})",
        values[0],
        values[1],
        values[3],
        values[2],
        started.elapsed()
    );
}
