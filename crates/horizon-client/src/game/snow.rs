//! Physical snow with RT lighting (`horizon_client::snow`): how deep the cover lies, how much of
//! what falls at the camera is snow (for the froxels' medium), and the snow handed to the renderer.
//! With it vanilla's snow columns give way to the renderer's flakes. It rides on the physical rain
//! (game/rain.rs: the same option, the same rain map, whose columns say where it snows); bit
//! 1048576 of `HORIZON_RT_SKIP` (or `HORIZON_PHYSICAL_SNOW=0`) keeps vanilla's snow.
use super::Game;
use horizon_client::snow::{Cover, SKIP_BIT, Snow};
use std::time::Instant;

/// The snow's state between frames.
#[derive(Default)]
pub(super) struct SnowState {
    cover: Cover,
    /// How much of what falls at the camera is snow, eased (0 to 1).
    share: f32,
    last: Option<Instant>,
    /// Whether a frame has run since the world opened (one opened in the snow starts covered).
    started: bool,
    /// Whether the physical snow was on at the last frame, to log when it changes.
    logged: Option<bool>,
}

/// Whether `HORIZON_RT_SKIP` has the snow's bit, or `HORIZON_PHYSICAL_SNOW=0`.
fn snow_skipped() -> bool {
    static SKIPPED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SKIPPED.get_or_init(|| {
        std::env::var("HORIZON_RT_SKIP")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .is_some_and(|s| s & SKIP_BIT != 0)
            || std::env::var("HORIZON_PHYSICAL_SNOW").is_ok_and(|v| v == "0")
    })
}

/// Seconds for the camera's share of snow to follow a biome's edge.
const SHARE_EASE: f32 = 3.0;
/// The cover a world opened while it snows starts with (cm).
const OPENED_COVER: f32 = 4.0;

impl Game<'_> {
    /// Whether the renderer's physical snow replaces vanilla's: with the physical rain, unless
    /// skipped.
    pub(super) fn physical_snow(&self) -> bool {
        self.physical_rain() && !snow_skipped()
    }

    /// This frame's snow: the cover moved on by the time since the last frame (times
    /// `HORIZON_SNOW_SPEED`, for time lapses; `HORIZON_SNOW_COVER_CM` forces it), and the camera's
    /// share of snow eased toward its column's. Call it after `rain_frame` (the rain map).
    pub(super) fn snow_frame(&mut self, camera: glam::DVec3, alpha: f32, sun_height: f32) -> Snow {
        let now = Instant::now();
        let elapsed = self
            .snow
            .last
            .replace(now)
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(1.0));
        let on = self.physical_snow();
        if self.snow.logged != Some(on) {
            self.snow.logged = Some(on);
            let s = &self.settings.horizon;
            log::info!(
                "Horizon snow: physical snow {} (RT lighting {}, Linear Light {}, Weather {}, \
                 Overworld {}, skipped {}); vanilla's snow columns {}",
                if on { "on" } else { "off" },
                s.rt_lighting,
                s.linear_light,
                s.weather,
                self.dim == super::Dim::Overworld,
                snow_skipped(),
                if on { "hidden" } else { "drawn" }
            );
        }
        if !on {
            self.snow.cover = Cover::default();
            self.snow.share = 0.0;
            self.snow.started = false;
            return Snow::default();
        }
        let (level, thunder) = self.world.weather.levels(alpha);
        let rate = Snow::rate_for(level, thunder, self.rain_intensity());
        if !self.snow.started {
            self.snow.started = true;
            if rate > 0.0 {
                self.snow.cover.depth = OPENED_COVER;
            }
        }
        let speed = std::env::var("HORIZON_SNOW_SPEED")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(1.0)
            .max(0.0);
        self.snow.cover.step(elapsed * speed, rate, sun_height);
        let cover = std::env::var("HORIZON_SNOW_COVER_CM")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .map_or(self.snow.cover.depth, |c| {
                c.clamp(0.0, horizon_client::snow::MAX_COVER_CM)
            });
        let (x, y, z) = (
            camera.x.floor() as i32,
            camera.y.floor() as i32,
            camera.z.floor() as i32,
        );
        let here = if self.precipitation(x, y, z) == Some(true) {
            1.0
        } else {
            0.0
        };
        let k = (elapsed / SHARE_EASE).min(1.0);
        self.snow.share += (here - self.snow.share) * k;
        // HORIZON_SNOW_DEBUG: the snow every 2 s.
        if std::env::var_os("HORIZON_SNOW_DEBUG").is_some() && self.world.game_time % 40 == 0 {
            log::info!(
                "Horizon snow: {rate:.2} mm/h, cover {cover:.2} cm, share at the camera {:.2}",
                self.snow.share
            );
        }
        Snow { rate, cover }
    }
}

impl SnowState {
    /// How much of what falls at the camera is snow (the froxels' medium), 0 without the physical
    /// snow.
    pub(super) fn share(&self) -> f32 {
        self.share
    }
}
