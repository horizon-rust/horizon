//! Microbenchmark of host preparation, wgpu staging, and upload synchronization.
//! Not the game's Renderer or a complete frame-time comparison.
use std::hint::black_box;
use std::time::Instant;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    uv: [f32; 2],
    layer: u32,
    color: u32,
    light: u32,
    normal: [i8; 4],
    glint_uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ParticleVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    layer: u32,
    color: u32,
    light: u32,
}

impl From<&Vertex> for ParticleVertex {
    fn from(value: &Vertex) -> Self {
        Self {
            pos: value.pos,
            uv: value.uv,
            layer: value.layer,
            color: value.color,
            light: value.light,
        }
    }
}

fn prepare(vertices: &[Vertex], output: &mut Vec<ParticleVertex>) -> bool {
    output.clear();
    if !vertices
        .iter()
        .all(|v| v.normal == [0; 4] && v.glint_uv == [0.0; 2])
    {
        return false;
    }
    output.extend(vertices.iter().map(ParticleVertex::from));
    true
}

fn as_bytes<T: bytemuck::Pod>(slice: &[T]) -> &[u8] {
    bytemuck::cast_slice(slice)
}

fn median_latency(mut f: impl FnMut(), n: usize) -> f64 {
    for _ in 0..20 {
        f();
    }
    let mut samples = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        for _ in 0..n {
            f();
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / n as f64);
    }
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn submit_and_wait(device: &wgpu::Device, queue: &wgpu::Queue) {
    queue.submit([]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
}

async fn run() {
    assert_eq!(std::mem::size_of::<Vertex>(), 44);
    assert_eq!(std::mem::size_of::<ParticleVertex>(), 32);
    let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    instance_descriptor.backends = match std::env::var("BENCH_BACKEND").as_deref() {
        Ok("vulkan") => wgpu::Backends::VULKAN,
        Ok("dx12") => wgpu::Backends::DX12,
        Ok(_) | Err(_) => wgpu::Backends::VULKAN | wgpu::Backends::DX12,
    };
    let instance = wgpu::Instance::new(instance_descriptor);
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await
        .expect("No hardware Vulkan/DX12 adapter; GPU timings not available");
    let info = adapter.get_info();
    println!(
        "GPU: {} ({:?}; {:?})",
        info.name, info.device_type, info.backend
    );
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .expect("GPU device required");
    println!(
        "Scope: wgpu 30 queue.write_buffer + queue.submit + device.poll; not full frame rendering."
    );
    println!(
        "quads,verts,bytes_44,bytes_32,upload_full_us,upload_32_us,full_with_cpu_us,compact_with_cpu_us"
    );
    for quads in [64_usize, 128, 256, 512, 1024, 4096] {
        let n = quads * 4;
        let vertices: Vec<Vertex> = (0..n)
            .map(|i| Vertex {
                pos: [i as f32 * 0.0001, 2.0, 3.0],
                uv: [0.2, 0.8],
                layer: 3 << 16 | (i as u32 & 31),
                color: 0xaabbccdd ^ i as u32,
                light: 0x0000f0f0,
                normal: [0; 4],
                glint_uv: [0.0; 2],
            })
            .collect();
        let mut compact = Vec::with_capacity(n);
        assert!(prepare(&vertices, &mut compact));
        assert_eq!(as_bytes(&vertices)[0..32], as_bytes(&compact)[0..32]);
        let full = as_bytes(&vertices);
        let shrunk = as_bytes(&compact).to_vec();
        let full_gpu = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("full"),
            size: full.len() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });
        let compact_gpu = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("compact"),
            size: shrunk.len() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });
        let iters = if quads >= 1024 { 90 } else { 180 };
        let upload_full_us = median_latency(
            || {
                queue.write_buffer(&full_gpu, 0, black_box(full));
                submit_and_wait(&device, &queue);
            },
            iters,
        );
        let upload_32_us = median_latency(
            || {
                queue.write_buffer(&compact_gpu, 0, black_box(&shrunk));
                submit_and_wait(&device, &queue);
            },
            iters,
        );
        let full_with_cpu_us = median_latency(
            || {
                queue.write_buffer(&full_gpu, 0, black_box(as_bytes(&vertices)));
                submit_and_wait(&device, &queue);
            },
            iters,
        );
        let compact_with_cpu_us = median_latency(
            || {
                assert!(prepare(black_box(&vertices), black_box(&mut compact)));
                queue.write_buffer(&compact_gpu, 0, black_box(as_bytes(&compact)));
                submit_and_wait(&device, &queue);
            },
            iters,
        );
        println!(
            "{quads},{n},{},{},{upload_full_us:.3},{upload_32_us:.3},{full_with_cpu_us:.3},{compact_with_cpu_us:.3}",
            full.len(),
            shrunk.len()
        );
    }
}

fn main() {
    pollster::block_on(run());
}
