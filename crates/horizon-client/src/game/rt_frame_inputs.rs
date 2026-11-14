//! What a frame hands RT lighting from the game (`render_frame`): the hit block's sway, the
//! player's body for the rays, the weather's frames and the camera's medium for the froxels
//! (game.rs may not grow).
use super::rain::RainFrame;
use super::{Dim, Game};
use horizon_client::snow::Snow;

impl Game<'_> {
    /// RT lighting's wind on the hit block at `pos`, for its outline to sway with it
    /// (`Frame::outline_wind`): its way of swaying and its sky light.
    pub(super) fn outline_wind(&self, pos: [i32; 3]) -> Option<(u8, f32, f64)> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let f = self.res.facts().get(state);
        let below = self.world.block(pos[0], pos[1] - 1, pos[2]);
        let upper = self.res.facts().get(below).name == f.name;
        let kind = horizon_client::gfx::wind::outline_class(&f.name, |c| f.has_class(c), upper);
        if kind != 0 {
            let sky = self.world.light(pos[0], pos[1], pos[2]) >> 4;
            return Some((kind, f32::from(sky) / 15.0, f64::from(pos[1])));
        }
        None
    }
    /// RT lighting: the player's body in first person, for the hardware rays alone.
    pub(super) fn rt_body(
        &self,
        title: bool,
        camera: glam::DVec3,
    ) -> Vec<horizon_client::mesher::Vertex> {
        let mut rt_body = Vec::new();
        if !title && self.renderer.as_ref().is_some_and(|r| r.hw_rays_on()) {
            self.rt_body_geometry(camera, self.clock.camera_fraction(), &mut rt_body);
        }
        rt_body
    }
    /// RT lighting's natural fogs, and the frame's physical rain and snow (none on the title
    /// screen).
    pub(super) fn weather_frames(
        &mut self,
        title: bool,
        camera: glam::DVec3,
        alpha: f64,
        sun_angle: f32,
    ) -> (Option<RainFrame>, Snow) {
        if !title {
            self.natural_fog(camera, sun_angle);
        }
        let rain = if title {
            None
        } else {
            let sun = horizon_client::celestial::direction(
                sun_angle,
                self.settings.horizon.sun_path_tilt as f32,
            );
            let rain = self.rain_frame(camera, alpha as f32, sun.y);
            Some((rain, self.snow_frame(camera, alpha as f32, sun.y)))
        };
        match rain {
            Some((r, s)) => (Some(r), s),
            None => (None, Default::default()),
        }
    }
    /// The camera's medium for RT lighting's froxels (`Renderer::set_volumetrics`).
    pub(super) fn camera_medium(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        pack_rain: f32,
        pack_thunder: f32,
    ) -> horizon_client::gfx::CameraMedium {
        // RT lighting's froxels, on as Remix's: the camera's medium from its block's sky
        // light (a cave below 3), the water and the weather from the shader pack's
        // (`pack_rain`, `pack_thunder`).
        let at = camera.floor().as_ivec3();
        // The weather's levels even without a shader pack (`frame.pack` has them only with
        // one): the rain's medium, its cloud cover and Remix's rain post-processing.
        let (rain, thunder) = if self.dim == Dim::Overworld {
            self.world.weather.levels(alpha as f32)
        } else {
            (0.0, 0.0)
        };
        horizon_client::gfx::CameraMedium {
            in_water: false,
            sky_light: f32::from(self.world.light(at.x, at.y, at.z) >> 4),
            rain: pack_rain.max(rain),
            thunder: pack_thunder.max(thunder),
            snow: self.snow.share(),
            intensity: self.rain_intensity(),
        }
    }
}
