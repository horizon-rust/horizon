//! RT lighting's sky and light for a frame: the rain's cloud cover, Remix's post-processing
//! for the situation, and the sun's or the moon's light the rays trace (gfx.rs may not grow).

use super::*;

impl Renderer {
    /// RT lighting's sky for the frame (`draw_frame`): the cloud cover under the rain, the sky
    /// table following the sun, and Remix's post-processing for the situation. Returns the cover.
    pub(super) fn rt_sky_frame(
        &mut self,
        frame: &Frame,
        rt_sky: bool,
        rt_sun: Vec3,
        moon_lit: f32,
        rain_level: f32,
        thunder_level: f32,
    ) -> glam::Vec4 {
        // RT lighting's cloud cover under the rain (`atmosphere::overcast`), whether or not the
        // clouds are drawn: the sky table turns overcast, the sun and the moon dim behind it. What
        // gets through is a share of what a clear day gives a level surface: the sun's (or the
        // moon's) on it and the clear sky's, about a fifth of the sun's again.
        let overcast = if rt_sky && rain_level > 0.0 && !frame.nether_lighting {
            let level = if rt_sun.y > 0.0 {
                self.atmosphere.sunlight(rt_sun) * rt_sun.y
            } else {
                self.atmosphere.sunlight(-rt_sun)
                    * (atmosphere::moonlight() * moon_lit * (-rt_sun.y))
            };
            let level =
                level.dot(Vec3::new(0.2126, 0.7152, 0.0722)) * 1.2 * atmosphere::PAPER_WHITE;
            let storm = (thunder_level / rain_level).clamp(0.0, 1.0);
            atmosphere::overcast(rain_level, storm, level)
        } else {
            glam::Vec4::ZERO
        };
        if rt_sky {
            self.atmosphere
                .update(&self.queue, rt_sun, moon_lit, overcast);
        }
        // Remix's per-situation post-processing (its minecraft-logic confs): storms first, then
        // the sun's height.
        if let Some(t) = self.tonemap.as_mut().filter(|_| self.rt_lighting) {
            // Remix's rain and thunder cap the exposure for a light they do not dim: under the
            // overcast (`atmosphere::overcast`) the cap rises by the stops the clouds take off.
            let storm = (thunder_level / rain_level.max(1e-3)).clamp(0.0, 1.0);
            let clouds = if overcast.w > 0.0 {
                atmosphere::overcast_stops(rain_level, storm)
            } else {
                0.0
            };
            let params = if thunder_level > 0.5 {
                tonemap::PostParams {
                    ev_max: tonemap::PostParams::THUNDER.ev_max + clouds,
                    ..tonemap::PostParams::THUNDER
                }
            } else if rain_level > 0.5 {
                tonemap::PostParams {
                    ev_max: tonemap::PostParams::RAIN.ev_max + clouds,
                    ..tonemap::PostParams::RAIN
                }
            } else if rt_sun.y < -0.05 {
                tonemap::PostParams::night()
            } else if rt_sun.y < 0.2 {
                tonemap::PostParams::DUSK
            } else {
                tonemap::PostParams::REMIX
            };
            // Shut in from the sky (the camera's sky light level), the room's own top
            // (`PostParams::indoors`); `HORIZON_RT_SKIP` 134217728 keeps the open air's.
            // By day (Remix's own situation: the sun up, no rain) the room's top whatever the
            // level: the open air's cap keeps the dusk and the night dark, and by day a scene
            // darker than the key is so because it is shut in. The level missed rooms open to the
            // sky: under a skylight it is 15 down to the floor and a level less a block away, so
            // a hall lit through its roof kept the open air's +1.5 stops and showed grey.
            // `HORIZON_RT_SKIP` 1073741824 keeps the level alone (shadow_rays.rs `SKIP_GI`).
            let by_day = params == tonemap::PostParams::REMIX && !self.shadow_rays.gi_skipped();
            let params = if frame.title || self.shadow_rays.interior_skipped() {
                params
            } else if by_day {
                params.indoors(0)
            } else {
                params.indoors(frame.pack.eye_light[1])
            };
            t.set_params(if self.eye.is_some() {
                params.for_eye()
            } else {
                params
            });
        }
        overcast
    }
    /// The light the rays trace toward this frame (`Rays::sun_light`): the sun's, or the moon's
    /// once it has set, through the air and the rain's clouds, and its disc's radius.
    pub(super) fn rt_sun_light(
        &self,
        rt_sun: Vec3,
        moon_lit: f32,
        overcast: glam::Vec4,
    ) -> [f32; 4] {
        // By night the rays trace toward the moon: its light by its phase, its own disc for
        // the shadows' penumbra (the Moon Size setting).
        let (light, radius) = if rt_sun.y > 0.0 {
            (self.atmosphere.sunlight(rt_sun), self.rt_sun_radius)
        } else {
            // The sun's light reflected by the lunar regolith, which reflects red better than
            // blue (the Moon's B-V of 0.92 against the Sun's 0.65), then filtered by the air on
            // the way down as the sun's own is (`sunlight` toward the moon).
            (
                self.atmosphere.sunlight(-rt_sun)
                    * Vec3::from(MOON_REFLECTANCE)
                    * (atmosphere::moonlight() * moon_lit),
                self.rt_moon_radius,
            )
        };
        // Behind the rain's clouds: about 3 % of the direct light gets through.
        let light = light * atmosphere::PAPER_WHITE * (1.0 - 0.97 * overcast.w);
        // Rising or setting, the light comes almost level and the rays only meet the terrain
        // around the camera: the far hills that hide the sun cast no shadow, and a valley
        // behind a mountain was lit orange at dawn. The direct light comes in over the first
        // degrees above the horizon (`sun_rise`).
        let light = light * sun_rise(rt_sun.y.abs());
        [light.x, light.y, light.z, radius]
    }
}
