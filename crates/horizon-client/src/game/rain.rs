//! Physical rain with RT lighting (`horizon_client::rain`): the rain map around the camera, how wet
//! the world is, and the rain handed to the renderer. With it vanilla's rain columns and splash
//! particles give way to the renderer's drops (snow stays vanilla's).
use super::{Dim, Game};
use horizon_client::blocks::Fluid;
use horizon_client::rain::{Rain, RainMap, Surface, Wetness, classify};
use std::time::Instant;

/// The rain's state between frames.
#[derive(Default)]
pub(super) struct RainState {
    map: RainMap,
    wetness: Wetness,
    last: Option<Instant>,
    /// The dimension the map was filled in.
    dim: Option<Dim>,
}

/// What a frame hands the renderer: the rain, the wetness, and the map's cells when they changed.
pub(super) struct RainFrame {
    pub rain: Rain,
    pub wetness: Wetness,
    pub cells: Option<Vec<u32>>,
}

impl Game<'_> {
    /// Whether the renderer's physical rain replaces vanilla's: RT lighting on, in the Overworld,
    /// with the Weather option, unless `HORIZON_PHYSICAL_RAIN=0`.
    pub(super) fn physical_rain(&self) -> bool {
        let s = &self.settings.horizon;
        s.rt_lighting
            && s.linear_light
            && s.weather
            && self.dim == Dim::Overworld
            && std::env::var("HORIZON_PHYSICAL_RAIN").map_or(true, |v| v != "0")
    }

    /// This frame's rain: the wetness moved on by the time since the last frame (times
    /// `HORIZON_WET_SPEED`, for time lapses), the map brought up to date around the camera while
    /// anything is wet or it rains.
    pub(super) fn rain_frame(
        &mut self,
        camera: glam::DVec3,
        alpha: f32,
        sun_height: f32,
    ) -> RainFrame {
        let now = Instant::now();
        let elapsed = self
            .rain
            .last
            .replace(now)
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(1.0));
        if !self.physical_rain() {
            self.rain.wetness = Wetness::default();
            return RainFrame {
                rain: Rain::default(),
                wetness: Wetness::default(),
                cells: None,
            };
        }
        let (level, thunder) = self.world.weather.levels(alpha);
        let rain = Rain::from_levels(level, thunder, self.water_seconds());
        let speed = std::env::var("HORIZON_WET_SPEED")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(1.0)
            .max(0.0);
        self.rain
            .wetness
            .step(elapsed * speed, rain.rate, sun_height);
        if self.rain.dim != Some(self.dim) {
            self.rain.map.clear();
            self.rain.dim = Some(self.dim);
        }
        // Kept in the sun too: the shore's wet sand reads it (terrain.wgsl `rt_shore`), rain or not.
        let (x, z) = (camera.x.floor() as i32, camera.z.floor() as i32);
        let mut map = std::mem::take(&mut self.rain.map);
        map.update(
            x,
            z,
            |x, z| self.rain_height(x, z),
            |x, z, h| self.rain_column(x, z, h),
        );
        self.rain.map = map;
        let cells = self
            .rain
            .map
            .take_changed()
            .then(|| self.rain.map.cells().to_vec());
        // HORIZON_RAIN_DEBUG: the rain, the wetness and the camera's column every 2 s.
        if std::env::var_os("HORIZON_RAIN_DEBUG").is_some() && self.world.game_time % 40 == 0 {
            let w = self.rain.wetness;
            let (x, z) = (camera.x.floor() as i32, camera.z.floor() as i32);
            let i = (z.rem_euclid(horizon_client::rain::MAP) * horizon_client::rain::MAP
                + x.rem_euclid(horizon_client::rain::MAP)) as usize;
            log::info!(
                "Horizon rain: {:.1} mm/h, wind {:?}, wet {:.2}/{:.2}, puddles {:.2}/{:.2}, \
                 the camera's column {:#010x}",
                rain.rate,
                rain.wind,
                w.wet,
                w.wet_sun,
                w.puddles,
                w.puddles_sun,
                self.rain.map.cells()[i]
            );
        }
        RainFrame {
            rain,
            wetness: self.rain.wetness,
            cells,
        }
    }

    /// The seconds the renderer's water and drops animate by: the same clock is not reachable
    /// here, so the gusts run on the game's ticks.
    fn water_seconds(&self) -> f32 {
        (self.world.game_time.rem_euclid(72_000) as f32) / 20.0
    }

    /// A column of the rain map: its top block's surface, the block light above it, whether it
    /// rains (not snows) there.
    fn rain_column(&self, x: i32, z: i32, height: i32) -> (Surface, u8, bool) {
        if !self.world.chunks.contains_key(&(x >> 4, z >> 4))
            || height <= horizon_client::chunk::MIN_Y
        {
            return (Surface::Unknown, 0, false);
        }
        let state = self.world.block(x, height - 1, z);
        let f = self.res.facts().get(state);
        // Its top's height when its shape covers the whole column.
        let top = match f.outline.as_ref() {
            [b] if b[0] <= 0.0 && b[2] <= 0.0 && b[3] >= 1.0 && b[5] >= 1.0 && b[1] <= 0.0 => {
                Some(b[4])
            }
            _ => None,
        };
        let water = f.fluid == Fluid::Water && !f.full_collision;
        let surface = classify(&f.name, &f.sounds.step, top, water, f.fluid == Fluid::Lava);
        let light = self.world.light(x, height, z) & 15;
        let rains = self.precipitation(x, height, z) == Some(false);
        (surface, light, rains)
    }
}
