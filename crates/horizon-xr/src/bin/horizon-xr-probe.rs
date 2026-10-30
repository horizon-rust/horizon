//! `horizon-xr-probe [SECONDS]`: opens the active OpenXR runtime and clears the left eye
//! blue and the right eye orange, pulsing, for SECONDS (default 15), then prints what it
//! measured. Proves the OpenXR -> Vulkan -> wgpu chain before the world goes through it.
//! It first prints the runtime's extensions and swapchain formats.
//!
//! `horizon-xr-probe --hdr [--sdr] [SECONDS]`: an HDR test pattern instead (default 300 s).
//! A panel 1.6 m wide, 1.5 m ahead, holds 12 patches at known luminances, each labelled in
//! nits, drawn into `Rgba16Float` swapchains as scRGB (linear, 1.0 = 80 nits). The runtime
//! shows them as they are: past what the headset can show, patches look the same. `--sdr`
//! draws the same values into the usual 8-bit sRGB swapchains (clipped at 1.0), to compare.
//!
//! With eye tracking it also prints the gaze: how often it was valid and tracked, and the
//! distinct directions it took (in the head's frame), to see how coarse it is.
//! `--foveated` (without `--hdr`) asks for the quad views: the insets are cleared green
//! (left) and purple (right), and it prints their sizes and where their centres went.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use horizon_xr::openxr as xr;

/// The patches, in nits, left to right and top to bottom (as in the shader's `NITS`).
const NITS: [u32; 12] = [10, 20, 40, 80, 100, 203, 400, 600, 800, 1000, 2000, 4000];

/// The gaze's yaw (positive right) and pitch (positive up) in the head's frame, degrees.
fn angles(view: [f32; 3]) -> (f32, f32) {
    (
        view[0].atan2(-view[2]).to_degrees(),
        view[1].clamp(-1.0, 1.0).asin().to_degrees(),
    )
}

/// A field of view's centre (yaw right, pitch up, degrees, from the eye's forward).
fn centre(fov: xr::Fovf) -> (f32, f32) {
    (
        ((fov.angle_left.tan() + fov.angle_right.tan()) * 0.5)
            .atan()
            .to_degrees(),
        ((fov.angle_down.tan() + fov.angle_up.tan()) * 0.5)
            .atan()
            .to_degrees(),
    )
}

/// Tenths of a degree, to tell directions apart.
fn key((yaw, pitch): (f32, f32)) -> (i32, i32) {
    ((yaw * 10.0).round() as i32, (pitch * 10.0).round() as i32)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let hdr = args.iter().any(|a| a == "--hdr");
    let sdr = args.iter().any(|a| a == "--sdr");
    // The pattern draws two eyes: no insets with it.
    let foveated = !hdr && args.iter().any(|a| a == "--foveated");
    let seconds: u64 =
        args.iter()
            .find_map(|s| s.parse().ok())
            .unwrap_or(if hdr { 300 } else { 15 });
    match horizon_xr::runtime_extensions() {
        Ok(names) => println!("extensions ({}): {}", names.len(), names.join(" ")),
        Err(e) => eprintln!("horizon-xr-probe: {e}"),
    }
    let flags = wgpu::InstanceFlags::from_build_config().with_env();
    let opened = horizon_xr::Xr::new("Horizon XR probe", flags, foveated, |_| {
        (wgpu::Features::empty(), wgpu::Limits::default())
    })
    .and_then(|mut xr| {
        let formats = xr.swapchain_formats()?;
        let names: Vec<String> = formats
            .iter()
            .map(|&f| format!("{:?}", ash::vk::Format::from_raw(f as i32)))
            .collect();
        println!("swapchain formats: {}", names.join(" "));
        if hdr && !sdr {
            xr.create_swapchains(wgpu::TextureFormat::Rgba16Float)?;
        } else {
            xr.create_swapchains(wgpu::TextureFormat::Bgra8Unorm)
                .or_else(|_| xr.create_swapchains(wgpu::TextureFormat::Rgba8Unorm))?;
        }
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
    match xr.inset_size {
        Some([w, h]) => println!(
            "quad views: insets {w}x{h}, {}",
            if xr.gaze_foveation {
                "placed by the gaze"
            } else {
                "fixed"
            }
        ),
        None if foveated => println!("quad views: not offered (stereo)"),
        None => {}
    }
    println!(
        "eye tracking: {}{}",
        if xr.eye_gaze { "yes" } else { "no" },
        if xr.invert_gaze_pitch {
            " (pitch inverted)"
        } else {
            ""
        }
    );
    let (mut gazes, mut tracked) = (0u32, 0u32);
    // Directions to a tenth of a degree, and the first frame each was seen.
    let mut seen = BTreeSet::new();
    let mut directions = Vec::new();
    let mut seen_insets = BTreeSet::new();
    let mut insets = Vec::new();
    let mut pattern = hdr.then(|| {
        let format = xr.eye_format();
        println!("pattern: {format:?}, scRGB (1.0 = 80 nits); patches in nits:");
        for row in NITS.chunks(4) {
            println!("  {row:?}");
        }
        Pattern::new(&xr.device, format)
    });

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
        // The frame reads the gaze as of the last sync of the actions.
        let _ = xr.controls();
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
        let since_first = *first_frame.get_or_insert_with(|| start.elapsed());
        if let Some(gaze) = frame.gaze {
            gazes += 1;
            tracked += u32::from(gaze.tracked);
            let at = angles(gaze.view);
            if seen.insert(key(at)) {
                directions.push((frames, at));
            }
        }
        if let Some([left, _]) = frame.insets {
            let at = centre(left.fov);
            if seen_insets.insert(key(at)) {
                insets.push((frames, at));
            }
        }
        let t = start.elapsed().as_secs_f64();
        let mut encoder = xr.device.create_command_encoder(&Default::default());
        if let Some(pattern) = &mut pattern {
            // The panel follows the head for its first two seconds, then stays put.
            if t - since_first.as_secs_f64() < 2.0 {
                pattern.place(frame.head);
            }
            for eye in 0..2 {
                let texture = xr.eye_texture(&frame, eye);
                pattern.draw(&xr.queue, &mut encoder, texture, eye, frame.eyes[eye]);
            }
        } else {
            let pulse = 0.5 + 0.5 * (t * 2.0).sin();
            let color = |r, g: f64, b| wgpu::Color {
                r,
                g: g * pulse,
                b,
                a: 1.0,
            };
            // The eyes, then the insets.
            let colors = [
                color(0.05, 0.2, 0.9),
                color(0.95, 0.45, 0.05),
                color(0.1, 0.9, 0.2),
                color(0.6, 0.1, 0.8),
            ];
            let views = if frame.insets.is_some() { 4 } else { 2 };
            for eye in 0..views {
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
    let list = |title: String, items: &[(u32, (f32, f32))]| {
        println!("{title}");
        for (frame, (yaw, pitch)) in items.iter().take(60) {
            println!("  frame {frame:>6}: {yaw:6.1} {pitch:6.1}");
        }
    };
    if xr.eye_gaze {
        list(
            format!(
                "gaze: valid on {gazes} frames, tracked on {tracked}; {} distinct directions (yaw right, pitch up, degrees, in the head):",
                directions.len()
            ),
            &directions,
        );
    }
    if xr.inset_size.is_some() {
        list(
            format!(
                "left inset: {} distinct centres (yaw right, pitch up, degrees, from the eye):",
                insets.len()
            ),
            &insets,
        );
    }
}

/// The HDR test panel: each eye's pixels are traced to a plane in the stage space.
struct Pattern {
    pipeline: wgpu::RenderPipeline,
    uniforms: [wgpu::Buffer; 2],
    groups: [wgpu::BindGroup; 2],
    /// The panel's centre and orientation (stage space).
    panel: xr::Posef,
}

impl Pattern {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("XR HDR pattern"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("XR HDR pattern"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let layout = pipeline.get_bind_group_layout(0);
        let uniforms = [0, 1].map(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("XR HDR pattern eye"),
                size: 6 * 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let groups = [0, 1].map(|i| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("XR HDR pattern eye"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms[i].as_entire_binding(),
                }],
            })
        });
        Self {
            pipeline,
            uniforms,
            groups,
            panel: xr::Posef::IDENTITY,
        }
    }

    /// Puts the panel 1.5 m ahead of `head`, at its height, upright and facing it.
    fn place(&mut self, head: xr::Posef) {
        let q = head.orientation;
        // The head's forward (-z) turned by its orientation, flattened.
        let fx = -2.0 * (q.x * q.z + q.w * q.y);
        let fz = -(1.0 - 2.0 * (q.x * q.x + q.y * q.y));
        let yaw = (-fx).atan2(-fz);
        let p = head.position;
        self.panel = xr::Posef {
            orientation: xr::Quaternionf {
                x: 0.0,
                y: (yaw / 2.0).sin(),
                z: 0.0,
                w: (yaw / 2.0).cos(),
            },
            position: xr::Vector3f {
                x: p.x - 1.5 * yaw.sin(),
                y: p.y,
                z: p.z - 1.5 * yaw.cos(),
            },
        };
    }

    fn draw(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        eye: usize,
        view: horizon_xr::Eye,
    ) {
        let (o, p) = (view.pose.orientation, view.pose.position);
        let (po, pp) = (self.panel.orientation, self.panel.position);
        let f = view.fov;
        let values: [f32; 24] = [
            o.x,
            o.y,
            o.z,
            o.w,
            p.x,
            p.y,
            p.z,
            0.0,
            f.angle_left.tan(),
            f.angle_right.tan(),
            f.angle_up.tan(),
            f.angle_down.tan(),
            po.x,
            po.y,
            po.z,
            po.w,
            pp.x,
            pp.y,
            pp.z,
            0.0,
            1.6,
            0.9,
            texture.width() as f32,
            texture.height() as f32,
        ];
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        queue.write_buffer(&self.uniforms[eye], 0, &bytes);
        let target = texture.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("XR HDR pattern"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.groups[eye], &[]);
        pass.draw(0..3, 0..1);
    }
}

const SHADER: &str = r#"
struct Eye {
    rot: vec4<f32>,       // the eye's orientation (quaternion), stage space
    pos: vec4<f32>,
    tans: vec4<f32>,      // tan of the left, right, up, down angles
    panel_rot: vec4<f32>, // the panel's; its front faces +z
    panel_pos: vec4<f32>,
    size: vec4<f32>,      // the panel's width and height (m), the image's (px)
};
@group(0) @binding(0) var<uniform> eye: Eye;

// Patches in nits, left to right, top to bottom (the probe's NITS).
const NITS = array<f32, 12>(10.0, 20.0, 40.0, 80.0, 100.0, 203.0, 400.0, 600.0, 800.0, 1000.0, 2000.0, 4000.0);
// 3x5 digits, bit row * 3 + column (top row first).
const DIGITS = array<u32, 10>(31599u, 29850u, 29671u, 31207u, 18925u, 31183u, 31695u, 18727u, 31727u, 31215u);
const LABEL = 0.18; // the labels' grey, linear

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn turn(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

// Whether `q` (metres from the label's top left, y down) falls on a digit of `n`.
fn label(n: u32, q: vec2<f32>, width: f32) -> bool {
    let unit = 0.009;
    var count = 1u;
    var tens = 10u;
    while n >= tens {
        count += 1u;
        tens *= 10u;
    }
    let text = f32(count) * 4.0 * unit - unit;
    let c = (q - vec2<f32>((width - text) / 2.0, 0.0)) / unit;
    if c.x < 0.0 || c.y < 0.0 || c.y >= 5.0 {
        return false;
    }
    let slot = u32(c.x) / 4u;
    let col = u32(c.x) % 4u;
    if slot >= count || col == 3u {
        return false;
    }
    var div = 1u;
    for (var k = slot + 1u; k < count; k++) {
        div *= 10u;
    }
    let digit = (n / div) % 10u;
    return ((DIGITS[digit] >> (u32(c.y) * 3u + col)) & 1u) == 1u;
}

@fragment
fn fs(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / eye.size.zw;
    let dir = turn(eye.rot, vec3<f32>(mix(eye.tans.x, eye.tans.y, uv.x), mix(eye.tans.z, eye.tans.w, uv.y), -1.0));
    let inv = vec4<f32>(-eye.panel_rot.xyz, eye.panel_rot.w);
    let o = turn(inv, eye.pos.xyz - eye.panel_pos.xyz);
    let d = turn(inv, dir);
    if o.z <= 0.0 || d.z >= 0.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let hit = o.xy - d.xy * (o.z / d.z);
    // Metres from the panel's top left, y down.
    let m = vec2<f32>(hit.x + eye.size.x / 2.0, eye.size.y / 2.0 - hit.y);
    let cell = vec2<f32>(eye.size.x / 4.0, eye.size.y / 3.0);
    if m.x < 0.0 || m.y < 0.0 || m.x >= eye.size.x || m.y >= eye.size.y {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let index = u32(m.y / cell.y) * 4u + u32(m.x / cell.x);
    let q = m % cell;
    let margin = 0.02;
    if q.x > margin && q.x < cell.x - margin && q.y > margin && q.y < cell.y - 0.075 {
        let v = NITS[index] / 80.0;
        return vec4<f32>(v, v, v, 1.0);
    }
    if label(u32(NITS[index]), q - vec2<f32>(0.0, cell.y - 0.06), cell.x) {
        return vec4<f32>(LABEL, LABEL, LABEL, 1.0);
    }
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}
"#;
