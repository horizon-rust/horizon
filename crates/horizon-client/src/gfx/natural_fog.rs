//! Natural fogs in RT lighting's froxels (froxels.rs): the valley fog of a still morning and the
//! steam fog over cold water, both made of the relief and of the air, not of the camera's place.
//!
//! - *Valley fog* (radiation fog): through the night the ground radiates its heat away, the air on
//!   it cools to its dew point and, heavier, drains down the slopes into the hollows, where the
//!   fog pools. It is dense down in the valleys, basins and along the rivers, has a sharp top (a
//!   sea of clouds below a hill's top), and none on the ridges; it forms late at night, is at its
//!   thickest before and as the sun rises, and once the sun warms the ground it lifts from below
//!   and tears apart through the morning. Humid air (a wet biome, rain the day before) makes it
//!   likelier and thicker; dry air (a desert) rarely makes any.
//! - *Steam fog* ("sea smoke"): air much colder than the water under it takes the water's vapour
//!   and condenses it again just above, a few blocks of wisps rising off calm lakes and rivers and
//!   drifting with the wind: at night, in the morning and the evening, all day in cold biomes; the
//!   sun burns it off.
//!
//! What this module owns, on the CPU: the time-of-day and air curves (`NaturalFog::uniform`), the
//! weather they read (`FogWeather`, which a real weather can drive), and the relief map around the
//! camera (`FogMaps`): two layers, a near one 2 blocks a texel 512 blocks across, a far one 64
//! blocks a texel 16384 across (the far terrain's 1024 chunks), each texel the column's ground, its
//! water surface, the pool's top (the valley floor around it plus a share of the relief) and its
//! climate. The game samples the columns a few thousand a frame (the world where it is loaded, the
//! far terrain elsewhere); a layer finished goes to the GPU whole. The shader (froxels.wgsl,
//! `fog_*`) makes the density from it, with a slowly drifting 3D noise.

use glam::DVec3;

/// Texels on a side of a layer.
pub const MAP_SIZE: usize = 256;
/// The layers' texel sizes in blocks: the near one (512 blocks across: the froxels' 256 block
/// reach), the far one (16384: the far terrain's).
pub const TEXEL: [i32; 2] = [2, 64];
/// The four layers, `MAP_SIZE` squared texels each: near shape, near climate, far shape, far
/// climate (`FogMaps::upload`'s `layer * 2 + 1` is a climate).
pub const MAP_LAYERS: usize = 4;
/// How far (blocks) the valley floor and the ridges around a column are looked for.
const RELIEF_RADIUS: f32 = 96.0;
/// The pool's top over the valley floor: this share of the relief, within these blocks, plus
/// the ground fog's least depth.
const POOL_SHARE: f32 = 0.35;
const POOL_MIN: f32 = 2.0;
const POOL_MAX: f32 = 32.0;
const GROUND_FOG: f32 = 2.0;

/// The Natural Fog setting: off, light, or normal (the default).
pub fn setting_scale(setting: u32) -> f32 {
    match setting {
        0 => 0.0,
        1 => 0.4,
        _ => 1.0,
    }
}

/// The air the natural fogs form in. `FogWeather::natural` makes one from the day and the time;
/// a real weather can set its own (`Renderer::set_natural_fog`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogWeather {
    /// The air's relative humidity near the ground, 0 to 1: the valley fog forms from about 0.6
    /// (wet biomes earlier, dry ones later) and is at its thickest from 0.95.
    pub humidity: f32,
    /// The air's temperature near the ground, in degrees Celsius, in a temperate biome (a biome's
    /// own climate shifts it: `biome_temperature_offset`).
    pub air_temperature: f32,
    /// The water's, in degrees Celsius (it lags the air's by days: nearly constant).
    pub water_temperature: f32,
}

/// A number 0..1 from an integer, the same on every machine (SplitMix64's finaliser).
fn hash01(n: i64) -> f32 {
    let mut z = (n as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
}

/// The hour of the day (0..24, 6 the sunrise) at a world's day time in ticks.
pub fn hour(day_time: i64) -> f32 {
    ((day_time.rem_euclid(24000) as f32 / 1000.0) + 6.0) % 24.0
}

impl FogWeather {
    /// The air on day `day` at `hour`, `wetness` 0..1 the rain of the last day and a half: the
    /// day's own humidity (some mornings dry, some foggy), plus the rain's, highest at dawn when
    /// the air is coldest; the air's temperature from 2 degrees before dawn to 17 at 15 h; the
    /// water at 12.
    pub fn natural(day: i64, hour: f32, wetness: f32) -> Self {
        let tau = std::f32::consts::TAU;
        let base = 0.5 + 0.38 * hash01(day) + 0.3 * wetness.clamp(0.0, 1.0);
        let humidity = base + 0.14 * (tau * (hour - 5.5) / 24.0).cos();
        let air_temperature = 9.5 + 7.5 * (tau * (hour - 15.0) / 24.0).cos();
        Self {
            humidity: humidity.clamp(0.0, 1.0),
            air_temperature,
            water_temperature: 12.0,
        }
    }
}

/// How far along the valley fog is at `hour`: (formed, dissipated), each 0..1. It forms from 1 h
/// to 7 h (the pool filling up), lifts and tears from 6:45 to 10:30 as the sun warms the ground,
/// and is gone by 11 h (`amount` 0 from then until it forms again).
pub fn valley_phase(hour: f32) -> (f32, f32, f32) {
    // Hours since 21 h: the night's cooling, unbroken across midnight.
    let s = (hour - 21.0).rem_euclid(24.0);
    let smooth = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let formed = smooth(4.0, 10.0, s);
    let dissipated = smooth(9.75, 13.5, s);
    let amount = formed * (1.0 - smooth(12.5, 14.0, s));
    (amount, formed, dissipated)
}

/// The natural fogs' state this frame (`Renderer::set_natural_fog`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NaturalFog {
    /// The Natural Fog setting (0 off, 1 light, 2 normal).
    pub setting: u32,
    /// The world's day time in ticks (0 the sunrise).
    pub day_time: i64,
    pub weather: FogWeather,
    /// The sun's height above the horizon, sine (-1..1): its warmth burns the steam fog off.
    pub sun_height: f32,
    /// The wind, blocks a second along x and z.
    pub wind: [f32; 2],
    /// The fogs can be: the camera is not in water nor deep underground (0..1).
    pub open: f32,
}

impl Default for NaturalFog {
    fn default() -> Self {
        Self {
            setting: 2,
            day_time: 6000,
            weather: FogWeather::natural(0, 12.0, 0.0),
            sun_height: 1.0,
            wind: [0.8, 0.3],
            open: 1.0,
        }
    }
}

/// What the shader reads of the fogs (froxels.wgsl `Fx.fog*`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FogUniform {
    /// x: the valley fog's density scale (0: none), y: the steam fog's, z: the humidity, w: how
    /// far the valley fog has lifted (0..1).
    pub fog: [f32; 4],
    /// x: how far it has formed (its pool's depth, 0..1), y: the air's temperature in a temperate
    /// biome, z: the sun's warmth (0..1), w: a temperate lake's water's (degrees Celsius).
    pub fog2: [f32; 4],
}

impl NaturalFog {
    /// Whether anything can show (the setting on and the camera in the open).
    pub fn active(&self) -> bool {
        setting_scale(self.setting) > 0.0 && self.open > 0.0
    }

    /// The shader's numbers for this frame.
    pub fn uniform(&self) -> FogUniform {
        let scale = setting_scale(self.setting) * self.open.clamp(0.0, 1.0);
        let (amount, formed, dissipated) = valley_phase(hour(self.day_time));
        let w = self.weather;
        // The steam fog where the air is colder than the water (per biome, in the shader), thinned
        // by the sun's warmth.
        let sun = self.sun_height.clamp(0.0, 1.0).sqrt();
        FogUniform {
            fog: [scale * amount, scale, w.humidity, dissipated],
            fog2: [formed, w.air_temperature, sun, w.water_temperature],
        }
    }
}

/// A biome's climate for the fogs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiomeClimate {
    /// How humid: its downfall, wetter for rivers and swamps.
    pub humid: f32,
    /// How calm its water: a lake, a river 1; the open sea 0.5.
    pub calm: f32,
    /// How much warmer than a temperate biome's its air is (degrees Celsius).
    pub air: f32,
    /// How much warmer than `FogWeather::water_temperature` (a temperate lake's) its water is.
    pub water: f32,
}

/// A biome's climate for the fogs, from its name and vanilla's downfall and temperature (0.8 the
/// plains', 0 the snowy ones). The steam fog needs air far colder than the water under it, so:
/// a lake or a river is warmer than the morning air (its water keeps the days' warmth, half its
/// biome's offset); the seas each have their own water and the air over them, and their wind
/// mixes the cold air away (the sea's water counts 6 degrees cooler): a warm or temperate sea
/// hardly steams, a polar one under arctic air does (sea smoke).
pub fn biome_climate(name: &str, downfall: f32, temperature: f32) -> BiomeClimate {
    let name = name.rsplit(':').next().unwrap_or(name);
    let wet = if name.contains("swamp") || name.contains("river") {
        0.35
    } else {
        0.0
    };
    let humid = (downfall * 1.1 + wet).clamp(0.0, 1.4);
    const WIND: f32 = -6.0;
    let (calm, air, water) = if name.contains("ocean") {
        let (air, water) = if name.contains("warm_ocean") && !name.contains("lukewarm") {
            (18.0, 14.0)
        } else if name.contains("lukewarm") {
            (10.0, 6.0)
        } else if name.contains("cold") {
            (-6.0, -6.0)
        } else if name.contains("frozen") {
            (-20.0, -11.0)
        } else {
            (0.0, 0.0)
        };
        (0.5, air, water + WIND)
    } else {
        let air = biome_temperature_offset(temperature);
        if name.contains("beach") {
            (0.5, air, 0.5 * air + WIND)
        } else {
            (1.0, air, 0.5 * air)
        }
    };
    BiomeClimate {
        humid,
        calm,
        air,
        water,
    }
}

/// How much warmer water lying on a block that heats it (magma) is than its biome's.
pub const HEATED_WATER: f32 = 30.0;

/// The air's temperature in a biome of vanilla's `temperature`, relative to a temperate one.
pub fn biome_temperature_offset(temperature: f32) -> f32 {
    ((temperature - 0.8) * 15.0).clamp(-20.0, 12.0)
}

/// One column of the relief map: its ground (the top of its solid blocks, leaves left out), its
/// surface (the water's top where it has water, else the ground), and its biome's climate
/// (`biome_climate`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogColumn {
    pub ground: f32,
    pub surface: f32,
    pub water: bool,
    pub humid: f32,
    pub calm: f32,
    pub cold: f32,
    /// Its water's warmth over a temperate lake's (`BiomeClimate::water`, `HEATED_WATER` more over
    /// magma).
    pub warmth: f32,
}

/// A layer finished: where it lies, and its texels (shape then climate, `MAP_SIZE` squared each).
pub struct FogLayer {
    pub layer: usize,
    /// The world block at its texel (0, 0)'s corner.
    pub origin: [i32; 2],
    pub texels: Vec<[f32; 4]>,
}

/// A layer being sampled.
struct Building {
    layer: usize,
    origin: [i32; 2],
    next: usize,
    columns: Vec<Option<FogColumn>>,
}

/// The relief map's two layers around the camera: where each lies, when it was made, and the one
/// being sampled.
pub struct FogMaps {
    origins: [Option<[i32; 2]>; 2],
    made: [Option<std::time::Instant>; 2],
    building: Option<Building>,
    /// A layer sampled whole, being finished on its own thread (its relief filters take some
    /// milliseconds).
    finishing: Option<std::sync::mpsc::Receiver<FogLayer>>,
}

impl Default for FogMaps {
    fn default() -> Self {
        Self::new()
    }
}

/// Where a layer centred on `camera` starts: snapped to 8 texels, so it moves by steps.
fn origin_for(layer: usize, camera: DVec3) -> [i32; 2] {
    let texel = TEXEL[layer];
    let step = texel * 8;
    let half = texel * MAP_SIZE as i32 / 2;
    [camera.x, camera.z].map(|c| (c as i32).div_euclid(step) * step - half)
}

impl FogMaps {
    pub fn new() -> Self {
        Self {
            origins: [None; 2],
            made: [None; 2],
            building: None,
            finishing: None,
        }
    }

    /// Where the layers lie (`None`: not made yet).
    pub fn origins(&self) -> [Option<[i32; 2]>; 2] {
        self.origins
    }

    /// Forget the layers (another world).
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// The layer to make next, if any: one never made, else one the camera has gone a quarter of
    /// its width from, else one older than its refresh (4 s near, 12 s far: the far terrain
    /// fills in, blocks change).
    fn due(&self, camera: DVec3) -> Option<usize> {
        let refresh = [4.0, 12.0];
        (0..2)
            .find(|&l| self.origins[l].is_none())
            .or_else(|| {
                (0..2).find(|&l| {
                    let o = self.origins[l].expect("made");
                    let half = TEXEL[l] * MAP_SIZE as i32 / 2;
                    let c = [o[0] + half, o[1] + half];
                    let far = (camera.x - f64::from(c[0]))
                        .abs()
                        .max((camera.z - f64::from(c[1])).abs());
                    far > f64::from(half / 2)
                })
            })
            .or_else(|| {
                (0..2)
                    .find(|&l| self.made[l].is_none_or(|t| t.elapsed().as_secs_f32() > refresh[l]))
            })
    }

    /// Sample up to `budget` columns (`sample(x, z)`: the column at block x, z, `None` where
    /// nothing is known), and when a layer is complete return it for the GPU.
    pub fn step(
        &mut self,
        camera: DVec3,
        budget: usize,
        mut sample: impl FnMut(i32, i32) -> Option<FogColumn>,
    ) -> Option<FogLayer> {
        if let Some(rx) = &self.finishing {
            return match rx.try_recv() {
                Ok(layer) => {
                    self.finishing = None;
                    self.origins[layer.layer] = Some(layer.origin);
                    self.made[layer.layer] = Some(std::time::Instant::now());
                    Some(layer)
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.finishing = None;
                    None
                }
            };
        }
        if self.building.is_none() {
            let layer = self.due(camera)?;
            self.building = Some(Building {
                layer,
                origin: origin_for(layer, camera),
                next: 0,
                columns: Vec::with_capacity(MAP_SIZE * MAP_SIZE),
            });
        }
        let b = self.building.as_mut().expect("just made");
        let texel = TEXEL[b.layer];
        let end = (b.next + budget).min(MAP_SIZE * MAP_SIZE);
        for i in b.next..end {
            let (tx, tz) = ((i % MAP_SIZE) as i32, (i / MAP_SIZE) as i32);
            b.columns.push(sample(
                b.origin[0] + tx * texel + texel / 2,
                b.origin[1] + tz * texel + texel / 2,
            ));
        }
        b.next = end;
        if end < MAP_SIZE * MAP_SIZE {
            return None;
        }
        let b = self.building.take().expect("building");
        let (tx, rx) = std::sync::mpsc::channel();
        self.finishing = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(FogLayer {
                layer: b.layer,
                origin: b.origin,
                texels: finish(&b.columns, TEXEL[b.layer] as f32),
            });
        });
        None
    }

    /// `step` until the layer being made is out (for tests: the finishing thread is waited on).
    #[cfg(test)]
    fn step_whole(
        &mut self,
        camera: DVec3,
        sample: impl FnMut(i32, i32) -> Option<FogColumn> + Copy,
    ) -> Option<FogLayer> {
        for _ in 0..1000 {
            if let Some(layer) = self.step(camera, MAP_SIZE * MAP_SIZE, sample) {
                return Some(layer);
            }
            if self.building.is_none() && self.finishing.is_none() {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        None
    }
}

/// A square neighbourhood's extreme along one axis, `radius` texels each side (`pick` min or max).
fn filter_axis(v: &[f32], radius: usize, along_x: bool, pick: fn(f32, f32) -> f32) -> Vec<f32> {
    let n = MAP_SIZE;
    let mut out = vec![0.0; n * n];
    for a in 0..n {
        for b in 0..n {
            let mut m = v[if along_x { a * n + b } else { b * n + a }];
            for k in b.saturating_sub(radius)..(b + radius + 1).min(n) {
                m = pick(m, v[if along_x { a * n + k } else { k * n + a }]);
            }
            out[if along_x { a * n + b } else { b * n + a }] = m;
        }
    }
    out
}

/// A square neighbourhood's extreme, `radius` texels each side.
fn filter(v: &[f32], radius: usize, pick: fn(f32, f32) -> f32) -> Vec<f32> {
    filter_axis(&filter_axis(v, radius, true, pick), radius, false, pick)
}

/// A box blur over `radius` texels each side, both axes (prefix sums).
fn blur(v: &[f32], radius: usize) -> Vec<f32> {
    let n = MAP_SIZE;
    let pass = |v: &[f32], along_x: bool| {
        let at = |a: usize, k: usize| if along_x { a * n + k } else { k * n + a };
        let mut out = vec![0.0; n * n];
        for a in 0..n {
            let mut prefix = vec![0.0f64; n + 1];
            for k in 0..n {
                prefix[k + 1] = prefix[k] + f64::from(v[at(a, k)]);
            }
            for b in 0..n {
                let (lo, hi) = (b.saturating_sub(radius), (b + radius + 1).min(n));
                out[at(a, b)] = ((prefix[hi] - prefix[lo]) / (hi - lo) as f64) as f32;
            }
        }
        out
    };
    pass(&pass(v, true), false)
}

/// The texels of a layer of `columns` (`texel` blocks each): the shape (ground, surface, the
/// pool's top, the valley floor) and the climate (humid, water's share times its calm, cold, 0).
/// Columns not known take their nearest known neighbours' (a few passes), and none known at all a
/// flat sea-level plain with no fog.
pub fn finish(columns: &[Option<FogColumn>], texel: f32) -> Vec<[f32; 4]> {
    let n = MAP_SIZE;
    let mut known: Vec<Option<FogColumn>> = columns.to_vec();
    for _ in 0..8 {
        if known.iter().all(Option::is_some) {
            break;
        }
        let last = known.clone();
        for i in 0..n * n {
            if last[i].is_some() {
                continue;
            }
            let (x, z) = (i % n, i / n);
            let neighbours = [
                (x > 0).then(|| i - 1),
                (x + 1 < n).then(|| i + 1),
                (z > 0).then(|| i - n),
                (z + 1 < n).then(|| i + n),
            ];
            known[i] = neighbours.into_iter().flatten().find_map(|j| last[j]);
        }
    }
    let plain = FogColumn {
        ground: 63.0,
        surface: 63.0,
        water: false,
        humid: 0.0,
        calm: 0.0,
        cold: 0.0,
        warmth: 0.0,
    };
    let known: Vec<FogColumn> = known.into_iter().map(|c| c.unwrap_or(plain)).collect();
    let surface: Vec<f32> = known.iter().map(|c| c.surface).collect();
    let radius = ((RELIEF_RADIUS / texel).round() as usize).max(2);
    let floor = blur(&filter(&surface, radius, f32::min), radius);
    let ridge = blur(&filter(&surface, radius, f32::max), radius);
    let top: Vec<f32> = (0..n * n)
        .map(|i| {
            floor[i] + (POOL_SHARE * (ridge[i] - floor[i])).clamp(POOL_MIN, POOL_MAX) + GROUND_FOG
        })
        .collect();
    let top = blur(&top, radius);
    let mut texels = Vec::with_capacity(2 * n * n);
    texels.extend((0..n * n).map(|i| [known[i].ground, surface[i], top[i], floor[i]]));
    texels.extend(known.iter().map(|c| {
        [
            c.humid,
            if c.water { c.calm } else { 0.0 },
            c.cold,
            if c.water { c.warmth } else { 0.0 },
        ]
    }));
    texels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_valley_fog_forms_peaks_and_lifts() {
        let at = |h| valley_phase(h);
        // None in the afternoon or the evening.
        assert_eq!(at(15.0).0, 0.0);
        assert_eq!(at(21.0).0, 0.0);
        // Forming at 5:30, formed by 6:30, lifting at 9 and gone by 11.
        let (a530, f530, d530) = at(5.5);
        let (a630, f630, d630) = at(6.5);
        let (a9, _, d9) = at(9.0);
        assert!(a530 > 0.5 && f530 < f630, "{a530} {f530} {f630}");
        assert!(a630 > 0.95 && d530 == 0.0 && d630 == 0.0);
        assert!(a9 > 0.5 && d9 > 0.4 && d9 < 0.9, "{a9} {d9}");
        assert_eq!(at(11.0).0, 0.0);
    }

    #[test]
    fn the_hour_follows_the_day_time() {
        assert_eq!(hour(0), 6.0);
        assert_eq!(hour(23500), 5.5);
        assert_eq!(hour(500), 6.5);
        assert_eq!(hour(3000), 9.0);
        assert_eq!(hour(18000), 0.0);
    }

    #[test]
    fn the_air_is_humid_and_cold_at_dawn() {
        let dawn = FogWeather::natural(3, 5.5, 0.0);
        let noon = FogWeather::natural(3, 14.0, 0.0);
        assert!(dawn.humidity > noon.humidity);
        assert!(dawn.air_temperature < dawn.water_temperature);
        assert!(noon.air_temperature > noon.water_temperature);
        // Rain the day before makes the morning wetter.
        assert!(FogWeather::natural(3, 5.5, 1.0).humidity >= dawn.humidity);
    }

    #[test]
    fn the_setting_scales_the_fogs() {
        let fog = |setting| {
            NaturalFog {
                setting,
                day_time: 500,
                ..NaturalFog::default()
            }
            .uniform()
        };
        assert_eq!(fog(0).fog[0], 0.0);
        assert_eq!(fog(0).fog[1], 0.0);
        assert!(fog(1).fog[0] > 0.0 && fog(1).fog[0] < fog(2).fog[0]);
        assert!(
            !NaturalFog {
                setting: 0,
                ..NaturalFog::default()
            }
            .active()
        );
    }

    #[test]
    fn deserts_are_dry_and_snow_is_cold() {
        let desert = biome_climate("minecraft:desert", 0.0, 2.0);
        let swamp = biome_climate("minecraft:swamp", 0.9, 0.8);
        let polar = biome_climate("minecraft:frozen_ocean", 0.5, 0.0);
        assert!(desert.humid < 0.1 && swamp.humid > 1.0);
        assert!(desert.air > 0.0 && polar.air < -10.0);
        assert_eq!(polar.calm, 0.5);
    }

    #[test]
    fn lakes_and_polar_seas_steam_at_dawn_warm_and_temperate_seas_hardly() {
        // The water less the air at dawn (the shader's), the steam fog showing from about 2 and
        // thick from 9.
        let dawn = FogWeather::natural(0, 6.0, 0.0);
        let excess = |name: &str, temperature: f32| {
            let c = biome_climate(name, 0.5, temperature);
            (dawn.water_temperature + c.water) - (dawn.air_temperature + c.air)
        };
        assert!(excess("minecraft:plains", 0.8) > 7.0);
        assert!(excess("minecraft:river", 0.5) > 7.0);
        assert!(excess("minecraft:frozen_ocean", 0.0) > 7.0);
        for sea in [
            "minecraft:ocean",
            "minecraft:deep_ocean",
            "minecraft:warm_ocean",
            "minecraft:lukewarm_ocean",
            "minecraft:cold_ocean",
        ] {
            assert!(excess(sea, 0.5) < 2.5, "{sea}");
        }
    }

    #[test]
    fn the_pool_fills_the_valley_and_not_the_ridge() {
        // A valley along x: the ground rises 1 block a texel away from z = 128 (a V 128 deep).
        let columns: Vec<Option<FogColumn>> = (0..MAP_SIZE * MAP_SIZE)
            .map(|i| {
                let z = (i / MAP_SIZE) as f32;
                let g = 60.0 + (z - 128.0).abs();
                Some(FogColumn {
                    ground: g,
                    surface: g,
                    water: false,
                    humid: 1.0,
                    calm: 1.0,
                    cold: 0.0,
                    warmth: 0.0,
                })
            })
            .collect();
        let texels = finish(&columns, 2.0);
        let at = |z: usize| texels[z * MAP_SIZE + 100];
        let bottom = at(128);
        // The pool's top is above the valley's bottom, and below the ground up the slopes.
        assert!(bottom[2] > bottom[0] + 4.0, "{bottom:?}");
        let slope = at(128 + 60);
        assert!(slope[2] < slope[0], "{slope:?}");
        // The climate layer follows the shape layer.
        assert_eq!(texels[MAP_SIZE * MAP_SIZE][0], 1.0);
    }

    #[test]
    fn layers_are_made_whole_and_in_turn() {
        let mut maps = FogMaps::new();
        let camera = DVec3::new(1000.0, 70.0, -300.0);
        let flat = |_, _| {
            Some(FogColumn {
                ground: 64.0,
                surface: 64.0,
                water: false,
                humid: 0.5,
                calm: 1.0,
                cold: 0.0,
                warmth: 0.0,
            })
        };
        assert!(maps.step(camera, 1000, flat).is_none());
        let near = maps.step_whole(camera, flat).expect("the near layer");
        assert_eq!(near.layer, 0);
        assert_eq!(near.texels.len(), 2 * MAP_SIZE * MAP_SIZE);
        // The camera lies in its middle.
        let o = near.origin;
        assert!((o[0]..o[0] + 512).contains(&1000) && (o[1]..o[1] + 512).contains(&-300));
        let far = maps.step_whole(camera, flat).expect("the far layer");
        assert_eq!(far.layer, 1);
        // Both fresh: nothing due until the camera moves away.
        assert!(maps.step_whole(camera, flat).is_none());
        let moved = camera + DVec3::new(200.0, 0.0, 0.0);
        assert_eq!(maps.step_whole(moved, flat).map(|l| l.layer), Some(0));
    }
}
