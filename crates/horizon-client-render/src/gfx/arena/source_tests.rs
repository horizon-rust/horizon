//! Original sidecar lifetime controls, executed explicitly on an offscreen adapter.
use super::*;
use crate::mesher::{TerrainSource, TerrainVertex};

fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    offset: u64,
    size: u64,
) -> Vec<u8> {
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, offset, &out, 0, size);
    queue.submit([encoder.finish()]);
    out.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    out.slice(..).get_mapped_range().unwrap().to_vec()
}

#[test]
#[ignore = "offscreen GPU control; run explicitly with an available adapter"]
fn terrain_source_arena_gpu_grow_move_recycle_and_refusal() {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("GPU control requires an adapter");
    eprintln!("TERRAIN_SOURCE_ARENA_ADAPTER {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let mut arena = Arena::new(&device, 24, GRAIN, true);
    arena.page_limit = GRAIN * 2;
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = super::super::upload::Uploads::new(&device);
    let first = arena.alloc(&device, &mut encoder, 4).unwrap();
    assert!(
        arena.sources(first.0).is_none(),
        "default geometry allocates no source buffer"
    );
    let geometry = [TerrainVertex::new([2.25, 3.75, 4.5], [0.25, 0.75], 7, 0xff123456, 0x8070); 4];
    uploads.buffer(
        &mut encoder,
        arena.buffer(first.0),
        first.1 * 24,
        bytemuck::cast_slice(&geometry),
    );
    let source = TerrainSource::new(71, [2, 3, 4], 13, false);
    arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        first.0,
        first.1,
        4,
        &[source],
        false,
    );
    let second = arena.alloc(&device, &mut encoder, GRAIN).unwrap();
    assert_eq!(second, (first.0, GRAIN), "grew the first page");
    let third = arena.alloc(&device, &mut encoder, GRAIN).unwrap();
    assert_ne!(third.0, first.0, "move crosses pages");
    arena.copy(&mut encoder, first, third, 4);
    arena.release(first.0, first.1, 4);
    let reused = arena.alloc_existing(4).unwrap();
    assert_eq!(reused, first);
    arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        reused.0,
        reused.1,
        4,
        &[],
        false,
    );
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    assert_eq!(
        read(
            &device,
            &queue,
            arena.sources(third.0).unwrap(),
            third.1 * 8,
            8
        ),
        bytemuck::bytes_of(&source)
    );
    assert_eq!(
        read(
            &device,
            &queue,
            arena.sources(first.0).unwrap(),
            first.1 * 8,
            8
        ),
        [0; 8]
    );
    assert_eq!(
        read(&device, &queue, arena.buffer(third.0), third.1 * 24, 96),
        bytemuck::cast_slice::<_, u8>(&geometry)
    );

    // A real requested binding limit refuses metadata/table allocation. The native
    // vertex allocation still fits; this is a controlled refusal, not a real OOM.
    let limits = wgpu::Limits {
        max_storage_buffer_binding_size: 256,
        ..Default::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    }))
    .unwrap();
    let mut arena = Arena::translucent(&device, 24, GRAIN);
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = super::super::upload::Uploads::new(&device);
    let at = arena.alloc(&device, &mut encoder, 4).unwrap();
    uploads.buffer(
        &mut encoder,
        arena.buffer(at.0),
        at.1 * 24,
        bytemuck::cast_slice(&geometry),
    );
    arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        at.0,
        at.1,
        4,
        &[source],
        false,
    );
    assert!(
        arena.sources(at.0).is_none(),
        "sidecar refused without losing vertices"
    );
    assert!(super::super::pack_post::block_ids(&device, &queue, &[0; 128], &[0, 1]).is_none());
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    assert_eq!(
        read(&device, &queue, arena.buffer(at.0), at.1 * 24, 96),
        bytemuck::cast_slice::<_, u8>(&geometry)
    );
}

#[test]
#[ignore = "offscreen GPU control; run explicitly with an available adapter"]
fn terrain_source_arena_packed_and_wide_mesh_in_same_page() {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("GPU control requires an adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let mut arena = Arena::new(&device, 24, GRAIN, true);
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = super::super::upload::Uploads::new(&device);

    let packed_at = arena.alloc(&device, &mut encoder, 2).unwrap();
    let wide_at = arena.alloc(&device, &mut encoder, 8).unwrap();
    assert_eq!(
        packed_at.0, wide_at.0,
        "both meshes reside in the same arena page"
    );

    let packed_sources = [
        TerrainSource::new(10, [1, 2, 3], 5, false),
        TerrainSource::new(20, [4, 5, 6], 8, false),
    ];
    let wide_sources = [
        TerrainSource::new(30, [7, 8, 9], 12, false),
        TerrainSource::new(40, [10, 11, 12], 15, true),
    ];

    arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        packed_at.0,
        packed_at.1,
        2,
        &packed_sources,
        true,
    );

    arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        wide_at.0,
        wide_at.1,
        8,
        &wide_sources,
        false,
    );

    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);

    let sources_buffer = arena.sources(packed_at.0).cloned().unwrap();

    assert_eq!(
        read(&device, &queue, &sources_buffer, packed_at.1 * 8, 16),
        bytemuck::cast_slice::<_, u8>(&packed_sources),
        "packed mesh records match at its slot range"
    );

    let mut expected_wide = Vec::with_capacity(8);
    for &s in &wide_sources {
        expected_wide.extend([s; 4]);
    }
    assert_eq!(
        read(&device, &queue, &sources_buffer, wide_at.1 * 8, 64),
        bytemuck::cast_slice::<_, u8>(&expected_wide),
        "wide mesh records repeated across all 4 vertices of each quad"
    );

    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = super::super::upload::Uploads::new(&device);
    arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        wide_at.0,
        wide_at.1,
        8,
        &[],
        false,
    );
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);

    assert_eq!(
        read(&device, &queue, &sources_buffer, packed_at.1 * 8, 16),
        bytemuck::cast_slice::<_, u8>(&packed_sources),
        "packed mesh sources preserved after wide mesh cleared"
    );
    assert_eq!(
        read(&device, &queue, &sources_buffer, wide_at.1 * 8, 64),
        vec![0u8; 64],
        "wide mesh slots cleared"
    );
}
