//! Auto-exposure for Linear Light with RT lighting (`tonemap`, exposure.wgsl), as RTX Remix's:
//! the scene's mean log luminance, the middle of the view counting most (0.75), adapted over time
//! on the GPU (speed 10) into the multiplier the grading pass applies before tone mapping, limited
//! to -2.5..+1.5 stops (`PostParams` lowers the top by situation).

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// The longest step the adaptation takes, for a frame that came after a stall.
const LONGEST_STEP: f32 = 0.25;

/// The pipeline, made once.
pub struct Exposure {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    log: bool,
}

/// The exposure, adapting from frame to frame. Each view's run (the window's, an eye's, a quad
/// view's) moves it by the time since the last one, so views drawn together share it.
pub struct Meter {
    /// What the grading pass reads (x: the multiplier), the compute pass writes.
    state: wgpu::Buffer,
    last: Cell<Option<Instant>>,
    /// A fixed exposure never runs the pass.
    fixed: bool,
    /// The log's frame count and its readback in flight.
    frames: Cell<u32>,
    pending: RefCell<Option<(wgpu::Buffer, Arc<AtomicBool>)>>,
}

/// `HORIZON_EXPOSURE=0` keeps the exposure at 1 (the scene as drawn); a number is a fixed value
/// in stops.
fn fixed() -> Option<f32> {
    let v = std::env::var("HORIZON_EXPOSURE").ok()?;
    match v.trim() {
        "0" | "off" => Some(1.0),
        v => v.parse::<f32>().ok().map(f32::exp2),
    }
}

impl Exposure {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon auto-exposure"),
            source: wgpu::ShaderSource::Wgsl(include_str!("exposure.wgsl").into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let buffer = |ty| wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon auto-exposure"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(1, buffer(wgpu::BufferBindingType::Uniform)),
                entry(
                    2,
                    buffer(wgpu::BufferBindingType::Storage { read_only: false }),
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon auto-exposure"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        // `HORIZON_EXPOSURE_KEY`, `_SPEED` and `_CENTRE`, for tuning (exposure.wgsl).
        let constant = |name: &'static str| {
            std::env::var(format!("HORIZON_EXPOSURE_{name}"))
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .map(|v| (name, v))
        };
        let constants: Vec<_> = ["KEY", "SPEED", "CENTRE"]
            .into_iter()
            .filter_map(constant)
            .collect();
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Horizon auto-exposure"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_exposure"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            },
            cache: None,
        });
        Self {
            layout,
            pipeline,
            log: std::env::var("HORIZON_EXPOSURE_LOG").as_deref() == Ok("1"),
        }
    }

    /// The exposure, starting at 1.
    pub fn meter(&self, device: &wgpu::Device) -> Meter {
        let make = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: true,
            })
        };
        // x: the multiplier; z: the first frame has not set the average yet (unless fixed).
        let state = make(
            "Horizon exposure",
            16,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::UNIFORM
                | wgpu::BufferUsages::COPY_SRC,
        );
        let value = fixed();
        let words = [value.unwrap_or(1.0), 0.0, 0.0, 0.0];
        state
            .slice(..)
            .get_mapped_range_mut()
            .expect("mapped at creation")
            .copy_from_slice(bytemuck::cast_slice(&words));
        state.unmap();
        Meter {
            state,
            last: Cell::new(None),
            fixed: value.is_some(),
            frames: Cell::new(0),
            pending: RefCell::new(None),
        }
    }

    /// Adapts `meter` to the light of `level` (the scene's light, small: `Chain::meter`).
    pub fn adapt(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        meter: &Meter,
        level: &wgpu::TextureView,
        params: &wgpu::Buffer,
    ) {
        if meter.fixed {
            return;
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon auto-exposure"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(level),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: meter.state.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Auto-exposure"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        if self.log {
            meter.log(device, encoder);
        }
    }
}

impl Meter {
    /// `HORIZON_EXPOSURE_LOG=1`: about twice a second, the multiplier, the adapted mean log2
    /// luminance and the stops it is at, read back a few frames later (for tuning).
    fn log(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        let mut pending = self.pending.borrow_mut();
        if let Some((buffer, ready)) = pending.as_ref() {
            if ready.load(Ordering::Acquire) {
                let words: [f32; 4] = bytemuck::pod_read_unaligned(
                    &buffer
                        .slice(..)
                        .get_mapped_range()
                        .expect("mapped for reading"),
                );
                eprintln!(
                    "Horizon exposure: x{:.2} ({:+.2} stops), scene mean {:+.2} log2",
                    words[0], words[3], words[1]
                );
                buffer.unmap();
                *pending = None;
            }
            return;
        }
        let frames = self.frames.get() + 1;
        self.frames.set(frames);
        if !frames.is_multiple_of(30) {
            return;
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon exposure's readback"),
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&self.state, 0, &buffer, 0, 16);
        let ready = Arc::new(AtomicBool::new(false));
        let flag = ready.clone();
        encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |_| {
            flag.store(true, Ordering::Release);
        });
        *pending = Some((buffer, ready));
    }

    /// Seconds since the last call, at most `LONGEST_STEP`: the adaptation's time step.
    pub fn step(&self) -> f32 {
        let now = Instant::now();
        self.last
            .replace(Some(now))
            .map_or(0.0, |t| (now - t).as_secs_f32())
            .min(LONGEST_STEP)
    }

    /// The buffer the grading pass reads: x is the multiplier.
    pub fn state(&self) -> &wgpu::Buffer {
        &self.state
    }
}
