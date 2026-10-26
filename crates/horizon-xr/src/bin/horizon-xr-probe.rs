//! `horizon-xr-probe [SECONDS]`: opens the active OpenXR runtime and clears the left eye
//! blue and the right eye orange, pulsing, for SECONDS (default 15), then prints what it
//! measured. Proves the OpenXR -> Vulkan -> wgpu chain before the world goes through it.

use std::time::{Duration, Instant};

fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(15);
    let flags = wgpu::InstanceFlags::from_build_config().with_env();
    let opened = horizon_xr::Xr::new("Horizon XR probe", flags, |_| {
        (wgpu::Features::empty(), wgpu::Limits::default())
    })
    .and_then(|mut xr| {
        xr.create_swapchains(wgpu::TextureFormat::Bgra8Unorm)
            .or_else(|_| xr.create_swapchains(wgpu::TextureFormat::Rgba8Unorm))?;
        Ok(xr)
    });
    let mut xr = match opened {
        Ok(xr) => xr,
        Err(e) => {
            eprintln!("horizon-xr-probe: {e}");
            std::process::exit(1);
        }
    };
    let info = xr.adapter.get_info();
    println!("runtime: {}", xr.runtime);
    println!("headset: {}", xr.system_name);
    println!("GPU: {} ({:?})", info.name, info.backend);
    println!("per eye: {}x{}", xr.size[0], xr.size[1]);

    let start = Instant::now();
    let mut frames = 0u32;
    let mut skipped = 0u32;
    let mut first_frame = None;
    let mut gaps = Vec::new();
    let mut last = Instant::now();
    while start.elapsed() < Duration::from_secs(seconds) && !xr.exit {
        if !xr.poll() {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let frame = match xr.begin_frame() {
            Ok(Some(f)) => f,
            Ok(None) => {
                skipped += 1;
                continue;
            }
            Err(e) => {
                eprintln!("horizon-xr-probe: {e}");
                break;
            }
        };
        first_frame.get_or_insert_with(|| start.elapsed());
        let t = start.elapsed().as_secs_f64();
        let pulse = 0.5 + 0.5 * (t * 2.0).sin();
        let colors = [
            wgpu::Color {
                r: 0.05,
                g: 0.2 * pulse,
                b: 0.9,
                a: 1.0,
            },
            wgpu::Color {
                r: 0.95,
                g: 0.45 * pulse,
                b: 0.05,
                a: 1.0,
            },
        ];
        let mut encoder = xr.device.create_command_encoder(&Default::default());
        for eye in 0..2 {
            let texture = xr.eye_texture(&frame, eye);
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(texture.format().remove_srgb_suffix()),
                ..Default::default()
            });
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("XR probe eye"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(colors[eye]),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        xr.queue.submit([encoder.finish()]);
        if let Err(e) = xr.end_frame(frame) {
            eprintln!("horizon-xr-probe: {e}");
            break;
        }
        frames += 1;
        gaps.push(last.elapsed().as_secs_f64() * 1000.0);
        last = Instant::now();
    }
    gaps.sort_by(f64::total_cmp);
    let median = gaps.get(gaps.len() / 2).copied().unwrap_or(0.0);
    println!(
        "frames: {frames} drawn, {skipped} not wanted; first after {:?}; median {median:.2} ms ({:.0} Hz)",
        first_frame.unwrap_or_default(),
        if median > 0.0 { 1000.0 / median } else { 0.0 }
    );
}
