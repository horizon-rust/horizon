//! Physical snow with RT lighting (docs/raytracing.md, "Snow"): what the CPU knows of it. The GPU
//! side is terrain.wgsl's `vs_snow`/`fs_snow` (the flakes) and `rt_snow` (the cover), and
//! `gfx/snow.rs`. It shares the rain map (`crate::rain::RainMap`), whose columns say where it
//! snows rather than rains (vanilla's precipitation by biome and height), but it is not the rain
//! drawn white:
//!
//! - **The flakes** are aggregates of ice crystals, as many for the rate as Gunn and Marshall's
//!   sizes say ([`slope`]), drifting down at about a metre a second whatever their size
//!   ([`terminal_velocity`], Locatelli and Hobbs 1974): slow enough that the wind carries them
//!   further sideways than down. They follow the wind the terrain makes (gfx/wind.rs, faster on
//!   the ridges, calm in the lee, turned along the valleys) and its gusts, and each tumbles and
//!   flutters on its own spiral. They scatter light as small white bodies (a diffuse sphere's
//!   phase, the crystals' forward lobe toward the sun, now and then a crystal's glint), not as
//!   lenses; one that lands rests a moment on the ground before it joins the cover.
//! - **The veil** far away is the froxels' snow medium: snow hides the distance more than rain of
//!   the same water, a blizzard 1.5 km away ([`extinction`], from the visibility it leaves).
//! - **The cover** builds up while it snows, about ten times the water's depth in fresh snow, the
//!   game's time running 72 times the real one ([`Cover`]); the sun melts it once it stops. Where
//!   it snows, the tops of the columns whiten in patches first (grass and leaves need more to
//!   hide), whole once it is deep enough, the snow rimming the top edge of their sides and
//!   plastered on the walls that face the wind; its crystals glint in the sun. A grass blade, a
//!   pond or a lava pool takes none.

/// The rate vanilla's rain level stands for where it snows (moderate snow), and its thunder's (a
/// blizzard), in millimetres of water an hour.
pub const SNOW_MM_H: f32 = 1.5;
pub const BLIZZARD_MM_H: f32 = 5.0;
/// Fresh snow's depth for its water (a density of about 0.1).
pub const SNOW_RATIO: f32 = 10.0;
/// The game's time against the real one: a game day is 20 minutes.
pub const GAME_TIME: f32 = 72.0;
/// The deepest the cover goes, in centimetres (a vanilla snow layer is 12.5).
pub const MAX_COVER_CM: f32 = 12.0;
/// `HORIZON_RT_SKIP`'s bit for the physical snow: vanilla's snow columns instead.
pub const SKIP_BIT: u32 = 1_048_576;

/// The snow's rate from vanilla's rain level and its thunder level (already times the rain's).
pub fn rate_mm_h(rain: f32, thunder: f32) -> f32 {
    let rain = rain.clamp(0.0, 1.0);
    let storm = if rain > 0.0 {
        (thunder / rain).clamp(0.0, 1.0)
    } else {
        0.0
    };
    rain * (SNOW_MM_H + (BLIZZARD_MM_H - SNOW_MM_H) * storm)
}

/// Gunn and Marshall's (1958) slope for a rate, per millimetre of melted diameter: N(D) = N0
/// exp(-slope D), N0 = 3800 rate^-0.87 a cubic metre a millimetre. Heavier snow, larger flakes.
pub fn slope(rate_mm_h: f32) -> f32 {
    2.55 * rate_mm_h.max(0.01).powf(-0.48)
}

/// A flake's diameter (mm) for its melted diameter (mm): a loose aggregate of dendrites, about a
/// sixtieth as dense as water, is four times as wide as its water.
pub fn flake_diameter(melted_mm: f32) -> f32 {
    4.0 * melted_mm
}

/// An aggregate's terminal velocity in m/s for its diameter in millimetres (Locatelli and Hobbs
/// 1974, unrimed aggregates of dendrites): 0.8 D^0.16, under a metre and a quarter a second at any
/// size. Its drag holds it to the air's speed: the wind carries it.
pub fn terminal_velocity(diameter_mm: f32) -> f32 {
    0.8 * diameter_mm.max(0.05).powf(0.16)
}

/// The light the falling snow takes out of a beam a metre: 3.912 over the visibility it leaves,
/// about 2.5 km in light snow (1 mm/h), 2.2 km in vanilla's snow and 1.5 km in a blizzard. Real
/// snow's visibility goes as the rate to the -0.8 (Rasmussen et al. 1999: some 800 m in a
/// blizzard), but at that a storm drowned the land a hundred blocks away in white: here it goes
/// as the rate to the -0.3.
pub fn extinction(rate_mm_h: f32) -> f32 {
    if rate_mm_h <= 0.0 {
        return 0.0;
    }
    3.912 / (2500.0 * rate_mm_h.powf(-0.3))
}

/// The snow this frame, for the renderer: its rate (0: none, or vanilla's snow) and how deep its
/// cover lies, in centimetres. The wind is the rain's (`crate::rain::Rain::wind`, and the
/// terrain's, gfx/wind.rs).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snow {
    pub rate: f32,
    pub cover: f32,
}
impl Snow {
    /// The snow's rate for vanilla's levels, this snowfall's `intensity` times
    /// (`crate::rain::Intensity`: flurries to a heavy fall); `HORIZON_SNOW_MM_H` forces it while it
    /// snows.
    pub fn rate_for(rain: f32, thunder: f32, intensity: f32) -> f32 {
        let rate = rate_mm_h(rain, thunder) * intensity.max(0.0);
        if rate > 0.0
            && let Some(forced) = std::env::var("HORIZON_SNOW_MM_H")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
        {
            return forced.max(0.0) * rain.clamp(0.0, 1.0);
        }
        rate
    }
}

/// How deep the fresh snow lies, in centimetres. It builds up while it snows, ten times the
/// water's depth, the game's time running [`GAME_TIME`] times the real one (a centimetre in about
/// half a minute of moderate snow); once it stops, the sun melts it (by its height: a centimetre
/// in four minutes at noon), not the shade (the air where it snows is below freezing).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cover {
    pub depth: f32,
}
/// Centimetres a real second the noon sun melts.
const MELT: f32 = 1.0 / 240.0;
impl Cover {
    /// `seconds` later under snow of `rate` mm/h, the sun `sun_height` high (its direction's y).
    pub fn step(&mut self, seconds: f32, rate: f32, sun_height: f32) {
        let dt = seconds.max(0.0);
        if rate > 0.0 {
            self.depth += dt * rate * SNOW_RATIO * 0.1 * GAME_TIME / 3600.0;
        } else {
            self.depth -= dt * MELT * (sun_height * 2.0).clamp(0.0, 1.0);
        }
        self.depth = self.depth.clamp(0.0, MAX_COVER_CM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flakes_drift_down_at_about_a_metre_a_second() {
        assert!((terminal_velocity(1.0) - 0.8).abs() < 1e-4);
        assert!(terminal_velocity(0.5) > 0.7 && terminal_velocity(0.5) < 0.8);
        assert!(terminal_velocity(10.0) > 1.1 && terminal_velocity(10.0) < 1.25);
    }

    #[test]
    fn heavier_snow_has_larger_flakes() {
        assert!(1.0 / slope(0.5) < 1.0 / slope(1.5));
        assert!(1.0 / slope(1.5) < 1.0 / slope(5.0));
        // About 0.4 mm of water on average in moderate snow: flakes about 1.6 mm wide.
        assert!(flake_diameter(1.0 / slope(1.0)) > 1.4 && flake_diameter(1.0 / slope(1.0)) < 1.8);
    }

    #[test]
    fn snow_hides_the_distance_more_than_rain() {
        // About 2.5 km's visibility at 1 mm/h, still 1.5 km in a blizzard: a storm hides the
        // distance, not the next hill.
        assert!((extinction(1.0) - 0.001565).abs() < 1e-6);
        let blizzard = 3.912 / extinction(BLIZZARD_MM_H);
        assert!(blizzard > 1400.0 && blizzard < 1700.0, "{blizzard}");
        assert!(extinction(BLIZZARD_MM_H) > extinction(SNOW_MM_H));
        // Rain of the same water.
        assert!(extinction(SNOW_MM_H) > crate::rain::extinction(SNOW_MM_H));
        assert_eq!(extinction(0.0), 0.0);
    }

    #[test]
    fn vanilla_levels_give_snow_and_blizzards() {
        assert_eq!(rate_mm_h(0.0, 0.0), 0.0);
        assert_eq!(rate_mm_h(1.0, 0.0), SNOW_MM_H);
        assert_eq!(rate_mm_h(1.0, 1.0), BLIZZARD_MM_H);
    }

    #[test]
    fn the_cover_builds_while_it_snows_and_the_sun_melts_it() {
        let mut c = Cover::default();
        for _ in 0..60 {
            c.step(1.0, SNOW_MM_H, 1.0);
        }
        // A minute of moderate snow: about 1.8 cm.
        assert!(c.depth > 1.5 && c.depth < 2.1, "{}", c.depth);
        for _ in 0..3600 {
            c.step(1.0, BLIZZARD_MM_H, 1.0);
        }
        assert_eq!(c.depth, MAX_COVER_CM);
        let mut night = c;
        for _ in 0..600 {
            night.step(1.0, 0.0, -0.5);
        }
        assert_eq!(night.depth, MAX_COVER_CM, "no sun, no melting");
        for _ in 0..600 {
            c.step(1.0, 0.0, 1.0);
        }
        assert!(c.depth < MAX_COVER_CM - 2.0);
    }
}
