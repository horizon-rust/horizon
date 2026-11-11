//! The live settings panel's performance view (game/live_panel.rs): while it is on, the render
//! thread's steps (its spans, wall time) and the GPU's stages per view (`gfx::gpu_passes`, read
//! back a few frames late, never waited for) are summed over about a second, and the page shows
//! the means a frame: frames a second, frame time, what the CPU and the GPU spent it on, per eye
//! in VR. Off, nothing is timed. Its own cost (summing and building the view) is measured and
//! shown with it.

use horizon_client::spans::Span;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// How long a window of frames lasts before its means are shown.
const WINDOW: Duration = Duration::from_secs(1);
/// The steps the page lists, the slowest first.
const SHOWN: usize = 14;

/// The performance view's window and what it last showed.
pub(super) struct Perf {
    started: Instant,
    last_frame: Option<Instant>,
    frames: u32,
    frame_ms: f64,
    frame_max: f32,
    gpu_wait_ms: f64,
    /// Each step's summed milliseconds, by name and depth (0 outermost).
    steps: Vec<(&'static str, u8, f64)>,
    /// Time this view spent on itself in the window.
    cost: Duration,
    /// Whether spans and GPU stages were recorded before the view turned them on.
    spans_before: bool,
    gpu_before: bool,
    shown: Value,
}

impl Perf {
    /// Turns the timings on (what was already on stays on when the view goes).
    pub(super) fn start() -> Self {
        let spans_before = horizon_client::spans::recording_here();
        let gpu_before = horizon_client::gfx::gpu_passes::enabled();
        horizon_client::spans::record_here(true);
        let gpu = gpu_before || horizon_client::gfx::gpu_passes::set_enabled(true);
        Self {
            started: Instant::now(),
            last_frame: None,
            frames: 0,
            frame_ms: 0.0,
            frame_max: 0.0,
            gpu_wait_ms: 0.0,
            steps: Vec::new(),
            cost: Duration::ZERO,
            spans_before,
            gpu_before,
            shown: json!({ "starting": true, "gpu_timing": gpu }),
        }
    }

    /// Turns off what `start` turned on.
    pub(super) fn stop(self) {
        if !self.spans_before {
            horizon_client::spans::record_here(false);
        }
        if !self.gpu_before {
            horizon_client::gfx::gpu_passes::set_enabled(false);
        }
    }

    /// One frame drawn: its spans (`spans::take_raw`'s) and how long the CPU waited on the GPU.
    pub(super) fn frame(&mut self, spans: &[Span], gpu_wait: Duration) {
        let now = Instant::now();
        if let Some(last) = self.last_frame.replace(now) {
            let ms = (now - last).as_secs_f32() * 1000.0;
            self.frames += 1;
            self.frame_ms += f64::from(ms);
            self.frame_max = self.frame_max.max(ms);
            self.gpu_wait_ms += gpu_wait.as_secs_f64() * 1000.0;
        }
        for span in spans.iter().filter(|s| s.depth <= 1) {
            match self
                .steps
                .iter_mut()
                .find(|(name, depth, _)| *name == span.name && *depth == span.depth)
            {
                Some(step) => step.2 += f64::from(span.ms),
                None => self.steps.push((span.name, span.depth, f64::from(span.ms))),
            }
        }
        if now - self.started >= WINDOW {
            self.show(now);
        }
        self.cost += now.elapsed();
    }

    /// The window's means a frame, for the page; the next window begins.
    fn show(&mut self, now: Instant) {
        let frames = f64::from(self.frames.max(1));
        let seconds = (now - self.started).as_secs_f64();
        let mut steps = std::mem::take(&mut self.steps);
        steps.sort_by(|a, b| b.2.total_cmp(&a.2));
        let list = |depth: u8| -> Vec<Value> {
            steps
                .iter()
                .filter(|s| s.1 == depth)
                .take(SHOWN)
                .map(|(name, _, ms)| json!({ "name": name, "ms": ms / frames }))
                .collect()
        };
        let gpu = horizon_client::gfx::gpu_passes::take_panel_means();
        self.shown = json!({
            "fps": f64::from(self.frames) / seconds.max(1e-3),
            "frame_ms": self.frame_ms / frames,
            "frame_max_ms": self.frame_max,
            "gpu_wait_ms": self.gpu_wait_ms / frames,
            "cpu": list(0),
            "cpu_inner": list(1),
            "gpu": gpu,
            "gpu_timing": horizon_client::gfx::gpu_passes::enabled(),
            // What the view cost the render thread, in milliseconds a second.
            "cost_ms_per_s": (self.cost + now.elapsed()).as_secs_f64() * 1000.0 / seconds.max(1e-3),
        });
        self.started = now;
        self.frames = 0;
        self.frame_ms = 0.0;
        self.frame_max = 0.0;
        self.gpu_wait_ms = 0.0;
        self.cost = Duration::ZERO;
    }

    /// What the page shows.
    pub(super) fn shown(&self) -> &Value {
        &self.shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(name: &'static str, ms: f32, depth: u8) -> Span {
        Span {
            name,
            ms,
            cpu_ms: ms,
            depth,
        }
    }

    #[test]
    fn a_window_shows_each_step_as_its_mean_a_frame_slowest_first() {
        let mut p = Perf::start();
        p.last_frame = Some(Instant::now());
        // Two frames: render 4 then 6 ms, tick 1 ms in one, a step inside render 2 ms.
        p.frame(
            &[span("tick", 1.0, 0), span("render", 4.0, 0)],
            Duration::ZERO,
        );
        p.started = Instant::now() - WINDOW;
        p.frame(
            &[span("prepare", 2.0, 1), span("render", 6.0, 0)],
            Duration::from_millis(3),
        );
        let shown = p.shown().clone();
        p.stop();
        let names: Vec<&str> = shown["cpu"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["render", "tick"]);
        assert_eq!(shown["cpu"][0]["ms"].as_f64(), Some(5.0));
        assert_eq!(shown["cpu"][1]["ms"].as_f64(), Some(0.5));
        assert_eq!(shown["cpu_inner"][0]["name"], "prepare");
        assert_eq!(shown["gpu_wait_ms"].as_f64(), Some(1.5));
    }
}
