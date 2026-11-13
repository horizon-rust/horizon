//! Running out of video memory: a frame that fails for it is skipped and the render distance
//! shrinks (halved per level, down to an eighth), then comes back once the terrain arenas
//! have eased (`Renderer::oom_level` is also what the game's chunk streaming follows).

use std::time::Instant;

use super::Renderer;

/// Maximum reduction level: 0 = full distance, 1 = half, 2 = quarter, 3 = eighth.
const MAX_LEVEL: u32 = 3;
/// Seconds the level stays put after it changed.
const COOLDOWN_SECS: f32 = 5.0;
/// Consecutive skipped frames after which the log says the renderer is stuck.
const MAX_SKIPPED_FRAMES: u32 = 10;
/// Usage ratio above which the render distance is reduced.
const USAGE_HIGH: f32 = 0.90;
/// Usage ratio below which the render distance is restored.
const USAGE_LOW: f32 = 0.70;
/// The farthest the render distance shrinks to, in blocks (4 chunks).
const MIN_DETAIL_DISTANCE: f32 = 4.0 * 16.0;

pub(super) struct Oom {
    level: u32,
    /// Seconds left before `level` may change again: a frame-time cooldown, so it does not
    /// depend on the frame rate.
    cooldown_secs: f32,
    /// Consecutive frames skipped for lack of memory; 0 after a drawn frame.
    skipped_frames: u32,
    /// When the last frame started, for the time between frames.
    last_frame: Option<Instant>,
    /// `Graphics::oom_memory_budget_fraction` and `oom_budget_fallback_bytes`.
    budget: (f32, u64),
}

impl Default for Oom {
    fn default() -> Self {
        Self {
            level: 0,
            cooldown_secs: 0.0,
            skipped_frames: 0,
            last_frame: None,
            budget: (0.65, 2 << 30),
        }
    }
}

/// The detail distance (blocks) at `level`: halved per level, never under 4 chunks.
fn detail_distance(level: u32, distance: f32) -> f32 {
    (distance / 2u32.pow(level) as f32).max(MIN_DETAIL_DISTANCE)
}

/// A step of the hysteresis after a drawn frame: the cooldown runs down by `dt`; once over,
/// usage above `USAGE_HIGH` raises the level and below `USAGE_LOW` lowers it, one level at
/// a time. Between the two nothing changes.
fn hysteresis(level: u32, cooldown: f32, usage: f32, dt: f32) -> (u32, f32) {
    if cooldown > 0.0 {
        return (level, (cooldown - dt).max(0.0));
    }
    if usage > USAGE_HIGH && level < MAX_LEVEL {
        return (level + 1, COOLDOWN_SECS);
    }
    if usage < USAGE_LOW && level > 0 {
        return (level - 1, COOLDOWN_SECS);
    }
    (level, 0.0)
}

/// A step after a frame that ran out of memory: the cooldown runs down by `dt` as in
/// [`hysteresis`], so repeated failures escalate once per cooldown rather than never; the
/// level rises when it is over. Returns the level, the cooldown and whether it rose.
fn escalate(level: u32, cooldown: f32, dt: f32) -> (u32, f32, bool) {
    let cooldown = (cooldown - dt).max(0.0);
    if cooldown > 0.0 {
        return (level, cooldown, false);
    }
    ((level + 1).min(MAX_LEVEL), COOLDOWN_SECS, true)
}

impl Renderer {
    /// How far the render distance is reduced for lack of video memory: 0 = not at all,
    /// each level halves it.
    pub fn oom_level(&self) -> u32 {
        self.oom.level
    }

    /// The video-memory budget the terrain arenas are measured against, from
    /// `Graphics::oom_memory_budget_fraction` and `oom_budget_fallback_bytes`.
    pub fn set_oom_budget(&mut self, fraction: f32, fallback_bytes: u64) {
        self.oom.budget = (fraction, fallback_bytes);
    }

    /// The frame's detail distance (blocks) after the reduction.
    pub(super) fn oom_detail_distance(&self, distance: f32) -> f32 {
        detail_distance(self.oom.level, distance)
    }

    /// Bytes of video memory the terrain arenas use, over the budget: 0 when it is unknown.
    fn memory_usage_ratio(&self) -> f32 {
        let (fraction, fallback) = self.oom.budget;
        // wgpu's public API does not expose the GPU's total memory, so the budget is a
        // share of the fallback until a platform query is added.
        let budget = if fraction <= 0.0 {
            fallback
        } else {
            (fallback as f64 * f64::from(fraction)) as u64
        };
        if budget == 0 {
            return 0.0;
        }
        self.arena_bytes() as f32 / budget as f32
    }

    /// The end of a window frame, once its error scopes are popped: `failed` is what they
    /// caught. On out of memory the frame is skipped (`true`) and the distance shrinks;
    /// otherwise the distance follows the arenas' usage.
    pub(super) fn oom_frame_skipped(
        &mut self,
        failed: Option<&wgpu::Error>,
        distance: f32,
    ) -> bool {
        let now = Instant::now();
        let dt = self
            .oom
            .last_frame
            .replace(now)
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        let Some(error) = failed else {
            self.oom.skipped_frames = 0;
            let usage = self.memory_usage_ratio();
            (self.oom.level, self.oom.cooldown_secs) =
                hysteresis(self.oom.level, self.oom.cooldown_secs, usage, dt);
            return false;
        };
        let (level, cooldown, rose) = escalate(self.oom.level, self.oom.cooldown_secs, dt);
        (self.oom.level, self.oom.cooldown_secs) = (level, cooldown);
        if rose {
            // Give back what is cheap to load again: the arenas' empty pages.
            self.graveyard.add(self.arena.trim());
            if let Some(arena) = &mut self.translucent_arena {
                self.graveyard.add(arena.trim());
            }
        }
        let chunks = self.oom_detail_distance(distance) as i32 / 16;
        self.oom.skipped_frames = self.oom.skipped_frames.saturating_add(1);
        log::warn!(
            "Horizon renderer: out of video memory ({error}); render distance reduced to {chunks} chunks, skipping this frame (skipped {})",
            self.oom.skipped_frames,
        );
        if self.oom.skipped_frames >= MAX_SKIPPED_FRAMES {
            log::error!(
                "Horizon renderer: {} consecutive frames out of video memory; render distance is at {chunks} chunks",
                self.oom.skipped_frames,
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_halves_the_detail_distance_down_to_four_chunks() {
        let full = 128.0 * 16.0;
        assert_eq!(detail_distance(0, full), full);
        assert_eq!(detail_distance(1, full), 1024.0);
        assert_eq!(detail_distance(3, full), 256.0);
        assert_eq!(detail_distance(3, 2.0 * 16.0), 64.0);
    }

    #[test]
    fn usage_moves_the_level_one_step_per_cooldown() {
        assert_eq!(hysteresis(0, 0.0, 0.95, 0.0), (1, COOLDOWN_SECS));
        assert_eq!(hysteresis(3, 0.0, 0.99, 0.0), (3, 0.0));
        assert_eq!(hysteresis(3, 0.0, 0.50, 0.0), (2, COOLDOWN_SECS));
        assert_eq!(hysteresis(0, 0.0, 0.10, 0.0), (0, 0.0));
        assert_eq!(hysteresis(2, 0.0, 0.80, 0.0), (2, 0.0));
        assert_eq!(hysteresis(0, 2.0, 0.99, 0.5), (0, 1.5));
        assert_eq!(hysteresis(3, 0.3, 0.10, 1.0), (3, 0.0));
    }

    #[test]
    fn repeated_out_of_memory_frames_escalate_once_per_cooldown() {
        // The first failure raises the level at once.
        let (level, cooldown, rose) = escalate(0, 0.0, 0.016);
        assert_eq!((level, rose), (1, true));
        assert_eq!(cooldown, COOLDOWN_SECS);
        // The next ones only run the cooldown down...
        let (level, cooldown, rose) = escalate(level, cooldown, 2.0);
        assert_eq!((level, rose), (1, false));
        assert_eq!(cooldown, COOLDOWN_SECS - 2.0);
        // ...until it is over: then the level rises again, to the top and no further.
        let (level, cooldown, rose) = escalate(level, cooldown, 3.5);
        assert_eq!((level, rose), (2, true));
        assert_eq!(cooldown, COOLDOWN_SECS);
        assert_eq!(
            escalate(MAX_LEVEL, 0.0, 0.0),
            (MAX_LEVEL, COOLDOWN_SECS, true)
        );
    }
}
