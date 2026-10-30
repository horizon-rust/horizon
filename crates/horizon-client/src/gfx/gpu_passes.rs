//! GPU time of each stage of a frame, for finding where a frame's budget goes (the VR budget in
//! docs/raytracing.md): `HORIZON_GPU_PASSES=1` stamps the GPU's timestamp between the frame's
//! stages and logs each stage's median every `WINDOW` frames, per view (`desktop`, `eye1`,
//! `eye2`, ...).
//!
//! A stamp is a timestamp written between two passes: a stage's time is the gap since the last
//! stamp, so it counts what the GPU drained in it (passes that overlap are charged to the one that
//! ends last), and the stages always add up to the frame. A stage stamped more than once (the
//! denoiser's four passes) is summed. With the variable off, every call is one relaxed load.
//!
//! Frames are timed whenever a readback slot is free, and read back a few frames later, so the
//! CPU never waits for them. The device needs timestamps inside encoders (Vulkan and D3D12 have
//! them; on Metal the variable only logs that it cannot time).

use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

/// Frames in flight at once (a VR frame is a submission per eye, so two eyes a frame).
const SLOTS: usize = 12;
/// Stamps a frame takes at most.
const CAPACITY: u32 = 96;
/// Timed frames each stage's median is taken over, and the number between two reports.
const WINDOW: usize = 60;

const FREE: u8 = 0;
const PENDING: u8 = 1;
const MAPPED: u8 = 2;

static ENABLED: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<Option<Profiler>> = Mutex::new(None);

struct Slot {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    state: Arc<AtomicU8>,
    /// The view this frame drew, and each stamp's stage after the first (the start).
    label: String,
    names: Vec<&'static str>,
}

/// One view's last frames: each stage's times in milliseconds, in the order they first ran.
#[derive(Default)]
struct History {
    stages: Vec<(&'static str, VecDeque<f32>)>,
    totals: VecDeque<f32>,
    since_report: usize,
}

struct Profiler {
    slots: Vec<Slot>,
    next: usize,
    /// The slot the frame being encoded stamps into.
    current: Option<usize>,
    period_ns: f32,
    views: Vec<(String, History)>,
    /// What the frames being timed show (the VR head's pose): a change starts the windows again.
    note: String,
    /// Frames begun, read back and thrown away (stamps out of order), for the log's tally.
    begun: u32,
    read: u32,
    dropped: u32,
}

/// Whether `HORIZON_GPU_PASSES` asks for the stage times.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Makes the profiler when the variable is set; the device must time inside encoders.
pub fn init(device: &wgpu::Device, queue: &wgpu::Queue) {
    if std::env::var_os("HORIZON_GPU_PASSES").is_none() {
        return;
    }
    if !device
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
    {
        log::warn!("Horizon: HORIZON_GPU_PASSES needs timestamps inside encoders; not timing");
        return;
    }
    let slots = (0..SLOTS)
        .map(|_| Slot {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("GPU stage timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: CAPACITY,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU stage timestamps resolve"),
                size: u64::from(CAPACITY) * 8,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            read: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU stage timestamps readback"),
                size: u64::from(CAPACITY) * 8,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            state: Arc::new(AtomicU8::new(FREE)),
            label: String::new(),
            names: Vec::new(),
        })
        .collect();
    if let Ok(mut state) = STATE.lock() {
        *state = Some(Profiler {
            slots,
            next: 0,
            current: None,
            period_ns: queue.get_timestamp_period(),
            views: Vec::new(),
            note: String::new(),
            begun: 0,
            read: 0,
            dropped: 0,
        });
        ENABLED.store(true, Ordering::Relaxed);
        log::info!("Horizon: timing the GPU's stages (HORIZON_GPU_PASSES)");
    }
}

/// Says what the frames show (`head 90 0`): the medians are taken over frames with the same note,
/// so that a run that looks around reports each look on its own.
pub fn note(text: impl FnOnce() -> String) {
    if !enabled() {
        return;
    }
    let Ok(mut state) = STATE.lock() else { return };
    let Some(p) = state.as_mut() else { return };
    let text = text();
    if p.note != text {
        p.note = text;
        p.views.clear();
    }
}

/// Starts timing the frame `encoder` records, for the view `label`, when a slot is free.
pub fn begin(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, label: &str) {
    if !enabled() {
        return;
    }
    let Ok(mut state) = STATE.lock() else { return };
    let Some(p) = state.as_mut() else { return };
    // Frames whose timestamps have arrived are read before a slot is taken.
    let _ = device.poll(wgpu::PollType::Poll);
    p.collect();
    p.begun += 1;
    if p.begun % 600 == 0 {
        log::info!(
            "GPU stages: {} frames begun, {} read back, {} thrown away (stamps out of order)",
            p.begun,
            p.read,
            p.dropped
        );
    }
    let slot = p.next;
    p.current = (p.slots[slot].state.load(Ordering::Acquire) == FREE).then_some(slot);
    let Some(slot) = p.current else { return };
    let s = &mut p.slots[slot];
    s.label.clear();
    s.label.push_str(label);
    s.names.clear();
    encoder.write_timestamp(&s.queries, 0);
}

/// Ends the stage `name` here: its time is what the GPU took since the last stamp.
pub fn stamp(encoder: &mut wgpu::CommandEncoder, name: &'static str) {
    if !enabled() {
        return;
    }
    let Ok(mut state) = STATE.lock() else { return };
    let Some(p) = state.as_mut() else { return };
    let Some(slot) = p.current else { return };
    let s = &mut p.slots[slot];
    let index = s.names.len() as u32 + 1;
    if index >= CAPACITY {
        return;
    }
    s.names.push(name);
    encoder.write_timestamp(&s.queries, index);
}

/// After the frame's last stage, before the encoder finishes: copies the stamps out.
pub fn end(encoder: &mut wgpu::CommandEncoder) {
    if !enabled() {
        return;
    }
    let Ok(mut state) = STATE.lock() else { return };
    let Some(p) = state.as_mut() else { return };
    let Some(slot) = p.current else { return };
    let s = &p.slots[slot];
    let count = s.names.len() as u32 + 1;
    encoder.resolve_query_set(&s.queries, 0..count, &s.resolve, 0);
    encoder.copy_buffer_to_buffer(&s.resolve, 0, &s.read, 0, u64::from(count) * 8);
}

/// After the frame is submitted: reads its stamps back once the GPU has them.
pub fn submitted() {
    if !enabled() {
        return;
    }
    let Ok(mut state) = STATE.lock() else { return };
    let Some(p) = state.as_mut() else { return };
    let Some(slot) = p.current.take() else { return };
    let s = &p.slots[slot];
    let bytes = (s.names.len() as u64 + 1) * 8;
    let flag = s.state.clone();
    flag.store(PENDING, Ordering::Release);
    s.read
        .slice(..bytes)
        .map_async(wgpu::MapMode::Read, move |r| {
            flag.store(if r.is_ok() { MAPPED } else { FREE }, Ordering::Release)
        });
    p.next = (slot + 1) % SLOTS;
}

impl Profiler {
    fn collect(&mut self) {
        for slot in 0..SLOTS {
            if self.slots[slot].state.load(Ordering::Acquire) != MAPPED {
                continue;
            }
            let s = &self.slots[slot];
            let bytes = (s.names.len() as u64 + 1) * 8;
            let stamps: Vec<u64> = match s.read.slice(..bytes).get_mapped_range() {
                Ok(data) => data
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|b| u64::from_le_bytes(*b))
                    .collect(),
                Err(_) => Vec::new(),
            };
            s.read.unmap();
            s.state.store(FREE, Ordering::Release);
            let ms = |ticks: u64| (ticks as f64 * f64::from(self.period_ns) / 1e6) as f32;
            self.read += 1;
            if stamps.len() < 2 || stamps.windows(2).any(|w| w[1] < w[0]) {
                self.dropped += 1;
                continue;
            }
            let s = &self.slots[slot];
            let index = match self.views.iter().position(|v| v.0 == s.label) {
                Some(i) => i,
                None => {
                    self.views.push((s.label.clone(), History::default()));
                    self.views.len() - 1
                }
            };
            let (view, history) = &mut self.views[index];
            // A stage stamped several times in a frame counts once, summed.
            let mut frame: Vec<(&'static str, f32)> = Vec::new();
            for (name, w) in s.names.iter().zip(stamps.windows(2)) {
                let t = ms(w[1] - w[0]);
                match frame.iter_mut().find(|f| f.0 == *name) {
                    Some(f) => f.1 += t,
                    None => frame.push((name, t)),
                }
            }
            for (name, t) in frame {
                let i = match history.stages.iter().position(|h| h.0 == name) {
                    Some(i) => i,
                    None => {
                        history.stages.push((name, VecDeque::new()));
                        history.stages.len() - 1
                    }
                };
                push(&mut history.stages[i].1, t);
            }
            push(
                &mut history.totals,
                ms(stamps[stamps.len() - 1] - stamps[0]),
            );
            history.since_report += 1;
            if history.since_report >= WINDOW {
                history.since_report = 0;
                report(view, &self.note, history);
            }
        }
    }
}

fn push(window: &mut VecDeque<f32>, ms: f32) {
    if window.len() == WINDOW {
        window.pop_front();
    }
    window.push_back(ms);
}

/// The value at `quantile` (0 to 1) of `window`.
fn quantile(window: &VecDeque<f32>, quantile: f32) -> f32 {
    let mut v: Vec<f32> = window.iter().copied().collect();
    v.sort_by(f32::total_cmp);
    v.get(((v.len() as f32 - 1.0) * quantile).round() as usize)
        .copied()
        .unwrap_or(0.0)
}

/// One log line: the view's stages' medians in milliseconds (over the frames each ran in), then
/// the frame's median and 95th percentile.
fn report(label: &str, note: &str, history: &History) {
    let parts: Vec<String> = history
        .stages
        .iter()
        .map(|(name, w)| format!("{name} {:.3}", quantile(w, 0.5)))
        .collect();
    log::info!(
        "GPU stages [{label}] {note} (median ms of the last {} frames): {} | frame median {:.3}, p95 {:.3}",
        history.totals.len(),
        parts.join("; "),
        quantile(&history.totals, 0.5),
        quantile(&history.totals, 0.95),
    );
}
