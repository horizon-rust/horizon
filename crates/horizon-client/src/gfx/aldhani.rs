//! The Eye of Aldhani (docs/raytracing.md), an experimental sky: `HORIZON_METEORS=aldhani` arms it,
//! the control socket's `aldhani start|stop|SECONDS` runs it (RT lighting). For five minutes
//! thousands of meteors burn at once out of one radiant on the southern horizon, the sky around it
//! glows, and every meteor lights the terrain through the rays. Each frame `aldhani.wgsl` makes the
//! field on the GPU from the event's clock alone (no state from frame to frame, the same for the
//! screen and every eye): the meteors burning, their streaks binned by tile of the sky for the sky
//! pass (terrain.wgsl `aldhani_streaks`), and the running sum of their light the rays draw one from
//! (shadow_rays.wgsl `aldhani_light`). The model's shared part (sizes, the radiant, the storm's
//! envelope, the sky's glow) is one text in the three shaders, kept equal by a test.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::upload::Uploads;

/// The slots a frame looks back over (aldhani.wgsl `ALDHANI_METEORS`).
const METEORS: u64 = 32768;
/// The sky's tiles a side and the streaks a tile holds at most (`ALDHANI_TILES`, `ALDHANI_TILE_CAP`).
const TILES: u64 = 64;
const TILE_CAP: u64 = 256;
/// The event's seconds (`ALDHANI_EVENT`), and the seconds its stream's front takes to cross the
/// sky (`ALDHANI_SWEEP`): the sky by the eye fills and empties that much later.
pub const EVENT: f32 = 300.0;
pub const SWEEP: f32 = 35.0;
/// A slot's seconds: at most one meteor each, so 8000 a second at the storm's height, burning for
/// one to two seconds each (the slots looked back over, all but the long ones', cover 3.6 s, past
/// the longest a meteor lasts, 2.8 s).
const SLOT: f32 = 1.0 / 8000.0;
/// The long ones' slots (aldhani.wgsl `ALDHANI_LONG`, the field's last) and their seconds: one in
/// two holds an earthgrazer, 128 a second, their 4096 slots covering 16 s, past the longest, 15 s.
const LONG: u64 = 4096;
const LONG_SLOT: f32 = 1.0 / 256.0;
const _: () = assert!(LONG as f32 * LONG_SLOT >= 15.0, "the long slots cover the longest earthgrazer");
/// The tiles' words: their counts, their lists, then the glow summed and blurred, red, green and
/// blue (`ALDHANI_GLOW_SUM`, `ALDHANI_GLOW_BLUR`).
const GLOW_SUM: u64 = TILES * TILES + TILES * TILES * TILE_CAP;
const TILE_WORDS: u64 = GLOW_SUM + 6 * TILES * TILES;
/// The field's bytes: the totals, the running sum, the streaks' lines, the meteors (aldhani.wgsl
/// `AldhaniField`).
const FIELD_BYTES: u64 = 16 + METEORS * 4 + METEORS * 16 + METEORS * 64;

/// Whether the environment arms the mode (`HORIZON_METEORS=aldhani`).
pub fn armed() -> bool {
    static ARMED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ARMED.get_or_init(|| std::env::var("HORIZON_METEORS").as_deref() == Ok("aldhani"))
}

/// The field's buffers and passes, and the event's clock.
pub struct Aldhani {
    field: wgpu::Buffer,
    tiles: wgpu::Buffer,
    params: wgpu::Buffer,
    /// The making, the summing and the glow's passes and their group (only when armed).
    passes: Option<(
        wgpu::ComputePipeline,
        wgpu::ComputePipeline,
        wgpu::ComputePipeline,
        wgpu::BindGroup,
    )>,
    /// When the event began, on the renderer's clock (seconds).
    start: Option<f32>,
    /// The totals read back for the log: the buffer, set when mapped, and when the last was asked.
    pending: Option<(wgpu::Buffer, Arc<AtomicBool>)>,
    logged: f32,
}

impl Aldhani {
    pub fn new(device: &wgpu::Device) -> Self {
        let armed = armed();
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let field = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Eye of Aldhani's field"),
            size: FIELD_BYTES,
            usage: storage | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let tiles = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Eye of Aldhani's tiles"),
            size: if armed {
                TILE_WORDS * 4
            } else {
                16
            },
            usage: storage,
            mapped_at_creation: false,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Eye of Aldhani"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let passes = armed.then(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Eye of Aldhani"),
                source: wgpu::ShaderSource::Wgsl(include_str!("aldhani.wgsl").into()),
            });
            let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            };
            let writable = wgpu::BufferBindingType::Storage { read_only: false };
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Eye of Aldhani"),
                entries: &[
                    entry(0, wgpu::BufferBindingType::Uniform),
                    entry(1, writable),
                    entry(2, writable),
                ],
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Eye of Aldhani"),
                bind_group_layouts: &[Some(&layout)],
                ..Default::default()
            });
            let pipeline = |entry: &str| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Eye of Aldhani"),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Eye of Aldhani"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: field.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: tiles.as_entire_binding(),
                    },
                ],
            });
            (
                pipeline("cs_aldhani"),
                pipeline("cs_aldhani_sum"),
                pipeline("cs_aldhani_glow"),
                group,
            )
        });
        Self {
            field,
            tiles,
            params,
            passes,
            start: None,
            pending: None,
            logged: f32::MIN,
        }
    }

    /// The field (shadow_rays.wgsl and terrain.wgsl `aldhani_field`).
    pub fn field(&self) -> &wgpu::Buffer {
        &self.field
    }

    /// The streaks by tile of the sky (terrain.wgsl `aldhani_tiles`).
    pub fn tiles(&self) -> &wgpu::Buffer {
        &self.tiles
    }

    /// Starts the event `seconds` in at clock time `now` (the control socket's `aldhani`), or stops
    /// it (`None`). Refused unless the environment armed the mode.
    pub fn set(&mut self, now: f32, seconds: Option<f32>) -> Result<(), &'static str> {
        if self.passes.is_none() {
            return Err("the Eye of Aldhani needs HORIZON_METEORS=aldhani");
        }
        self.start = seconds.map(|s| now - s.clamp(0.0, EVENT + SWEEP));
        Ok(())
    }

    /// The seconds into the event at clock time `now`, while it runs.
    pub fn time(&mut self, now: f32) -> Option<f32> {
        let t = now - self.start?;
        if !(0.0..=EVENT + SWEEP).contains(&t) {
            self.start = None;
            return None;
        }
        Some(t)
    }

    /// Makes this frame's field `t` seconds into the event (the passes before the rays and the
    /// sky); about every five seconds logs how many meteors burn and are drawn.
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        t: f32,
    ) {
        let Some((make, sum, glow, group)) = &self.passes else {
            return;
        };
        uploads.buffer(
            encoder,
            &self.params,
            0,
            bytemuck::cast_slice(&[t, SLOT, LONG_SLOT, 0.0]),
        );
        encoder.clear_buffer(&self.tiles, 0, Some(TILES * TILES * 4));
        encoder.clear_buffer(&self.tiles, GLOW_SUM * 4, Some(3 * TILES * TILES * 4));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Eye of Aldhani"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, group, &[]);
            pass.set_pipeline(make);
            pass.dispatch_workgroups((METEORS as u32).div_ceil(256), 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Eye of Aldhani's sum"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, group, &[]);
            pass.set_pipeline(sum);
            pass.dispatch_workgroups(1, 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Eye of Aldhani's glow"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, group, &[]);
            pass.set_pipeline(glow);
            let groups = (TILES as u32).div_ceil(8);
            pass.dispatch_workgroups(groups, groups, 1);
        }
        self.log(device, encoder, t);
    }

    fn log(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, t: f32) {
        if let Some((buffer, ready)) = &self.pending {
            if ready.load(Ordering::Acquire) {
                let words: [f32; 4] = bytemuck::pod_read_unaligned(
                    &buffer
                        .slice(..)
                        .get_mapped_range()
                        .expect("mapped for reading"),
                );
                eprintln!(
                    "Horizon Eye of Aldhani: {t:.0} s, {} meteors burning, {} streaks drawn, light sum {:.3}",
                    words[1], words[2], words[0]
                );
                buffer.unmap();
                self.pending = None;
            }
            return;
        }
        if t - self.logged < 5.0 && t >= self.logged {
            return;
        }
        self.logged = t;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Eye of Aldhani's readback"),
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&self.field, 0, &buffer, 0, 16);
        let ready = Arc::new(AtomicBool::new(false));
        let flag = ready.clone();
        encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |_| {
            flag.store(true, Ordering::Release);
        });
        self.pending = Some((buffer, ready));
    }
}

#[cfg(test)]
mod tests {
    /// The field's shared part is one text in its three shaders: the sky draws, and the terrain is
    /// lit by, the very field the pass makes.
    #[test]
    fn the_field_is_the_same_in_the_three_shaders() {
        let block = |source: &str| {
            let source = source.replace("\r\n", "\n");
            let from = source.find("// ALDHANI-BEGIN").expect("begin");
            let to = source.find("// ALDHANI-END").expect("end");
            source[from..to].to_string()
        };
        let made = block(include_str!("aldhani.wgsl"));
        assert_eq!(made, block(include_str!("../terrain.wgsl")));
        assert_eq!(made, block(include_str!("shadow_rays.wgsl")));
    }

    /// The sizes the Rust side allocates are the shaders'.
    #[test]
    fn the_sizes_are_the_shaders() {
        let source = include_str!("aldhani.wgsl");
        for (name, value) in [
            ("ALDHANI_METEORS", super::METEORS),
            ("ALDHANI_TILES", super::TILES),
            ("ALDHANI_TILE_CAP", super::TILE_CAP),
            ("ALDHANI_LONG", super::LONG),
        ] {
            assert!(
                source.contains(&format!("const {name}: u32 = {value}u;")),
                "{name}"
            );
        }
        assert!(source.contains(&format!("const ALDHANI_EVENT: f32 = {:.1};", super::EVENT)));
        assert!(source.contains(&format!("const ALDHANI_SWEEP: f32 = {:.1};", super::SWEEP)));
    }
}
