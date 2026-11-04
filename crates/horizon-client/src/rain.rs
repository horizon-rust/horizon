//! Physical rain and the wet world it leaves (docs/raytracing.md, "Rain and wet ground"), with RT
//! lighting: what the CPU knows of it. The GPU side is terrain.wgsl's `RAIN` block (the drops, the
//! wet surfaces, the puddles and the ripples) and `gfx/rain.rs`.
//!
//! - **The rain's strength** is a rate in millimetres an hour ([`rate_mm_h`]), vanilla's rain and
//!   thunder levels for now; a weather system to come sets it instead ([`Rain::rate`]).
//! - **The drops** follow Marshall and Palmer's sizes for that rate ([`slope`]), each falling at
//!   its terminal velocity ([`terminal_velocity`], Atlas, Srivastava and Sekhon's fit to Gunn and
//!   Kinzer's measurements): drizzle fine and slow, a downpour large and fast.
//! - **The veil** far away is the froxels' rain medium, its extinction the drops' own
//!   ([`extinction`]): Remix's rain (0.996 over 10 m) is about 1.5 mm/h of it, its thunder about
//!   25 mm/h.
//! - **Where the rain falls** is vanilla's rain heightmap (MOTION_BLOCKING), around the camera in
//!   a [`RainMap`] of [`MAP`] by [`MAP`] columns, each also saying what its top block is made of
//!   ([`Surface`]): how porous it is, whether puddles can sit on it.
//! - **How wet the world is** builds up while it rains and dries after, the puddles last and the
//!   sunlit surfaces first ([`Wetness`]).

/// The rain map's side, in columns (blocks), around the camera.
pub const MAP: i32 = 256;
/// The rate the rain's level 1 stands for (a heavy shower), and the thunder's (a downpour), mm/h.
pub const RAIN_MM_H: f32 = 8.0;
pub const THUNDER_MM_H: f32 = 35.0;
/// Marshall-Palmer's intercept, drops a cubic metre a millimetre of diameter.
pub const MARSHALL_PALMER_N0: f32 = 8000.0;

/// The rain's rate from vanilla's rain level and its thunder level (already times the rain's).
pub fn rate_mm_h(rain: f32, thunder: f32) -> f32 {
    let rain = rain.clamp(0.0, 1.0);
    let storm = if rain > 0.0 {
        (thunder / rain).clamp(0.0, 1.0)
    } else {
        0.0
    };
    rain * (RAIN_MM_H + (THUNDER_MM_H - RAIN_MM_H) * storm)
}

/// Marshall-Palmer's slope for a rate, per millimetre of diameter: N(D) = N0 exp(-slope D).
pub fn slope(rate_mm_h: f32) -> f32 {
    4.1 * rate_mm_h.max(0.01).powf(-0.21)
}

/// A drop's terminal velocity in m/s for its diameter in millimetres (Atlas et al. 1973, fitted to
/// Gunn and Kinzer 1949): about 2 m/s at half a millimetre, 9 m/s from five.
pub fn terminal_velocity(diameter_mm: f32) -> f32 {
    (9.65 - 10.3 * (-0.6 * diameter_mm).exp()).max(0.0)
}

/// The light the drops take out of a beam a metre (geometric optics, extinction efficiency 2):
/// 2 x pi/4 x the integral of D^2 N(D), that is pi N0 / slope^3, in mm^2/m^3.
pub fn extinction(rate_mm_h: f32) -> f32 {
    if rate_mm_h <= 0.0 {
        return 0.0;
    }
    std::f32::consts::PI * MARSHALL_PALMER_N0 / slope(rate_mm_h).powi(3) * 1e-6
}

/// The rain this frame, for the renderer: its rate and the wind blowing it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rain {
    /// Millimetres an hour (0: none).
    pub rate: f32,
    /// The wind, m/s along x and z.
    pub wind: [f32; 2],
}
impl Rain {
    /// The rain for vanilla's levels at `seconds` (the wind's gusts): a breeze that turns slowly,
    /// stronger in a storm. `HORIZON_RAIN_MM_H` forces the rate while it rains.
    pub fn from_levels(rain: f32, thunder: f32, seconds: f32) -> Self {
        let mut rate = rate_mm_h(rain, thunder);
        if rate > 0.0
            && let Some(forced) = std::env::var("HORIZON_RAIN_MM_H")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
        {
            rate = forced.max(0.0) * rain.clamp(0.0, 1.0);
        }
        let storm = (rate / THUNDER_MM_H).clamp(0.0, 1.0);
        let speed = (1.0 + 3.0 * storm) * (1.0 + 0.4 * (seconds * 0.13).sin());
        let angle = 0.6 + 0.5 * (seconds * 0.011).sin();
        Self {
            rate,
            wind: [speed * angle.cos(), speed * angle.sin()],
        }
    }
}

/// How wet the world is, 0 to 1, in the shade and in full sun, and how far the puddles have
/// grown. It builds up under the rain, faster the harder it falls; once it stops, everything dries,
/// the puddles last, the sunlit surfaces (by the sun's height) faster.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wetness {
    pub wet: f32,
    pub wet_sun: f32,
    pub puddles: f32,
    pub puddles_sun: f32,
}
/// Seconds to soak the surfaces at [`RAIN_MM_H`], and to fill the puddles.
const SOAK: f32 = 50.0;
const FILL: f32 = 240.0;
/// Seconds to dry in the shade: the surfaces, the puddles; the noon sun dries three times as fast.
const DRY: f32 = 420.0;
const DRY_PUDDLES: f32 = 900.0;
const SUN_DRYING: f32 = 2.0;
impl Wetness {
    /// `seconds` later under rain of `rate` mm/h, the sun `sun_height` high (its direction's y).
    pub fn step(&mut self, seconds: f32, rate: f32, sun_height: f32) {
        let dt = seconds.max(0.0);
        if rate > 0.0 {
            let soak = dt / SOAK * (rate / RAIN_MM_H).sqrt();
            for w in [&mut self.wet, &mut self.wet_sun] {
                *w = (*w + soak).min(1.0);
            }
            // Puddles gather once the ground is soaked.
            let fill = dt / FILL * (rate / RAIN_MM_H) * ((self.wet - 0.5) * 2.0).clamp(0.0, 1.0);
            for p in [&mut self.puddles, &mut self.puddles_sun] {
                *p = (*p + fill).min(1.0);
            }
        } else {
            let sun = 1.0 + SUN_DRYING * (sun_height * 3.0).clamp(0.0, 1.0);
            self.wet = (self.wet - dt / DRY).max(0.0);
            self.wet_sun = (self.wet_sun - dt / DRY * sun).max(0.0);
            self.puddles = (self.puddles - dt / DRY_PUDDLES).max(0.0);
            self.puddles_sun = (self.puddles_sun - dt / DRY_PUDDLES * sun).max(0.0);
        }
    }
    /// Whether anything is wet (the map and the shading are needed).
    pub fn any(&self) -> bool {
        self.wet > 0.0 || self.wet_sun > 0.0 || self.puddles > 0.0 || self.puddles_sun > 0.0
    }
}

/// What a column's top block is made of, for the rain: terrain.wgsl's `rain_porosity` and
/// `rain_puddle_bias` read these numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Surface {
    /// Not loaded: the sky light stands in, no puddles.
    Unknown = 0,
    /// Stone, bricks, concrete: a little porous, darkens and shines, puddles.
    Stone = 1,
    /// Planks and logs.
    Wood = 2,
    /// Dirt, mud, gravel, clay: porous, puddles readily.
    Soil = 3,
    /// Sand: drinks the water, darkens, puddles only in a downpour's end.
    Sand = 4,
    /// Wool, moss, hay: drink it all, never shine.
    Wool = 5,
    /// Metal, glass: the water runs off; a film, no darkening, no puddles.
    Runoff = 6,
    /// Leaves: waxy, shine, no puddles (the water drips through).
    Leaves = 7,
    /// Not a full block (slabs, stairs, fences): wet, no puddles.
    Partial = 8,
    /// Grass blocks: soil under grass, puddles more rarely.
    Grass = 9,
    /// A fluid's top: the rain's ripples.
    Water = 10,
    /// A soil block a sixteenth short (paths, farmland): puddles on its top.
    Path = 11,
    /// Snow, ice, lava: never wet.
    Dry = 12,
}

/// A block's [`Surface`]: `name` its id, `step` its step sound (the game's own word on what it is
/// made of, which mods set too), `top` its top's height when its shape covers the whole column
/// (`None` otherwise), `water` whether it holds water.
pub fn classify(name: &str, step: &str, top: Option<f64>, water: bool, lava: bool) -> Surface {
    let id = name.strip_prefix("minecraft:").unwrap_or(name);
    let sound = step
        .strip_prefix("minecraft:block.")
        .and_then(|s| s.strip_suffix(".step"))
        .unwrap_or(step);
    if lava || id.contains("snow") || id.contains("ice") || id == "magma_block" {
        return Surface::Dry;
    }
    if water {
        return Surface::Water;
    }
    if id.contains("leaves") {
        return Surface::Leaves;
    }
    let full = top.is_some_and(|t| t >= 0.999);
    let short = top.is_some_and(|t| (0.9..0.999).contains(&t));
    if id == "dirt_path" || id == "farmland" {
        return if short {
            Surface::Path
        } else {
            Surface::Partial
        };
    }
    if !full {
        return Surface::Partial;
    }
    if id.contains("glass")
        || id.ends_with("_block") && is_metal(id)
        || matches!(
            sound,
            "metal" | "copper" | "glass" | "chain" | "netherite_block" | "anvil" | "heavy_core"
        )
    {
        return Surface::Runoff;
    }
    if id.contains("wool")
        || id.contains("moss")
        || id == "hay_block"
        || id.contains("sponge")
        || matches!(sound, "wool" | "moss")
    {
        return Surface::Wool;
    }
    if id.contains("sand") && !id.contains("sandstone") || id.contains("concrete_powder") {
        return Surface::Sand;
    }
    if id == "grass_block" || id == "mycelium" || id == "podzol" {
        return Surface::Grass;
    }
    if matches!(
        id,
        "dirt" | "coarse_dirt" | "rooted_dirt" | "mud" | "clay" | "gravel" | "packed_mud"
    ) || matches!(sound, "gravel" | "mud" | "rooted_dirt")
    {
        return Surface::Soil;
    }
    if id.contains("planks")
        || id.ends_with("_log")
        || id.ends_with("_wood")
        || id.contains("bamboo_block")
        || matches!(
            sound,
            "wood" | "cherry_wood" | "bamboo_wood" | "nether_wood"
        )
    {
        return Surface::Wood;
    }
    Surface::Stone
}
fn is_metal(id: &str) -> bool {
    ["iron", "gold", "copper", "netherite"]
        .iter()
        .any(|m| id.contains(m))
        && !id.contains("ore")
}

/// One column of the rain map: its rain height (MOTION_BLOCKING's, one above the top block), its
/// top block's surface, the block light just above it, whether it rains there (not snow, not a dry
/// biome), and how many of its four neighbours stand higher (a hollow, where puddles gather).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    pub height: i32,
    pub surface: Surface,
    pub light: u8,
    pub rains: bool,
    pub higher: u8,
}
impl Column {
    /// Packed for the GPU (terrain.wgsl `rain_cell`): the height + 2048 in 12 bits, the surface in
    /// 4, the block light in 4, whether it rains in 1, the higher neighbours in 3, and bits 8..12
    /// of x and z (each 4) to tell a stale texel of the wrapped map.
    pub fn pack(self, x: i32, z: i32) -> u32 {
        let height = (self.height + 2048).clamp(0, 4095) as u32;
        height
            | (self.surface as u32) << 12
            | u32::from(self.light.min(15)) << 16
            | u32::from(self.rains) << 20
            | u32::from(self.higher.min(4)) << 21
            | (((x >> 8) & 15) as u32) << 24
            | (((z >> 8) & 15) as u32) << 28
    }
}

/// Columns refreshed a frame by the sweep (in whole rows), so that a block placed or broken shows
/// in the rain within about [`MAP`] / `SWEEP_ROWS` frames.
const SWEEP_ROWS: i32 = 8;
/// Changed runs of words this close together go up as one range (a copy command costs more than
/// a quarter of a kilobyte more of staging), and past `MAX_RANGES` ranges as one covering them
/// all: a crossing eastward changes a 16-word run in every row, 256 copies otherwise.
const MERGE_GAP: usize = 64;
const MAX_RANGES: usize = 16;

/// The rain map: [`MAP`] x [`MAP`] packed [`Column`]s around the camera, wrapped (a column's
/// texel is its x and z modulo [`MAP`]), its centre moved by whole chunks as the camera moves; the
/// newly covered strips are filled at once, and a sweep refreshes a few rows every frame.
///
/// Only the words that changed are uploaded ([`RainMap::take_upload`]). It used to send the whole
/// map (256 KB, copied and staged) every frame, the sweep reporting a change whether or not a
/// column had: standing still, the sweep's rows now go up only when a block in them changed, and a
/// chunk crossing sends its 16-column strip.
pub struct RainMap {
    cells: Vec<u32>,
    /// The column the window is centred on (a multiple of 16), once filled.
    centre: Option<(i32, i32)>,
    sweep: i32,
    /// Each texel row's changed columns since the last upload, `lo..hi` (empty when equal).
    dirty: Vec<(u16, u16)>,
    /// The heights of the rectangle being filled and its one-column border, reused.
    heights: Vec<i32>,
}
impl Default for RainMap {
    fn default() -> Self {
        Self {
            cells: vec![0; (MAP * MAP) as usize],
            centre: None,
            sweep: 0,
            dirty: vec![(0, 0); MAP as usize],
            heights: Vec::new(),
        }
    }
}
impl RainMap {
    /// The packed columns, row by row (z), each row MAP columns (x), wrapped.
    pub fn cells(&self) -> &[u32] {
        &self.cells
    }
    /// The words that changed since the last call, as [`upload_ranges`] reads them, or `None`.
    pub fn take_upload(&mut self) -> Option<Vec<u32>> {
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for (row, span) in self.dirty.iter_mut().enumerate() {
            let (lo, hi) = std::mem::take(span);
            if lo >= hi {
                continue;
            }
            let (from, to) = (
                row * MAP as usize + lo as usize,
                row * MAP as usize + hi as usize,
            );
            match ranges.last_mut() {
                Some(last) if from - last.1 <= MERGE_GAP => last.1 = to,
                _ => ranges.push((from, to)),
            }
        }
        if ranges.len() > MAX_RANGES {
            ranges = vec![(ranges[0].0, ranges[ranges.len() - 1].1)];
        }
        if ranges.is_empty() {
            return None;
        }
        let words: usize = ranges.iter().map(|(a, b)| b - a + 2).sum();
        let mut out = Vec::with_capacity(words);
        for (from, to) in ranges {
            out.extend([from as u32, (to - from) as u32]);
            out.extend_from_slice(&self.cells[from..to]);
        }
        Some(out)
    }
    /// Forgets everything (a new world or dimension).
    pub fn clear(&mut self) {
        self.cells.fill(0);
        self.centre = None;
        self.dirty.fill((0, MAP as u16));
    }
    /// Brings the map up to date around the camera's column (`x`, `z`): `height` gives a column's
    /// rain height, `column` the rest of it from its height.
    ///
    /// A frame fills either the strips a chunk crossing uncovered (4096 columns a crossing, 7936
    /// diagonally) or, when there was none, the sweep's rows (2048): each column one `column` and,
    /// its heights read once for the rectangle and its border, about 1.25 `height` (it was 5, the
    /// column and its four neighbours each time). Span `rain_map`.
    pub fn update(
        &mut self,
        x: i32,
        z: i32,
        height: impl Fn(i32, i32) -> i32,
        column: impl Fn(i32, i32, i32) -> (Surface, u8, bool),
    ) {
        crate::spans::time("rain_map", || self.update_inner(x, z, &height, &column));
    }
    fn update_inner(
        &mut self,
        x: i32,
        z: i32,
        height: &impl Fn(i32, i32) -> i32,
        column: &impl Fn(i32, i32, i32) -> (Surface, u8, bool),
    ) {
        let centre = (x.div_euclid(16) * 16, z.div_euclid(16) * 16);
        let half = MAP / 2;
        let (x0, z0) = (centre.0 - half, centre.1 - half);
        let mut crossed = true;
        match self.centre {
            Some(old) if old == centre => crossed = false,
            Some(old) if (old.0 - centre.0).abs() < MAP && (old.1 - centre.1).abs() < MAP => {
                // Only the strips the window moved onto: the rows past the old window's, whole,
                // then in the rows both share, the columns past its.
                let (ox0, oz0) = (old.0 - half, old.1 - half);
                let (sz0, sz1) = (z0.max(oz0), (z0 + MAP).min(oz0 + MAP));
                if z0 < sz0 {
                    self.fill(x0, z0, MAP, sz0 - z0, height, column);
                }
                if sz1 < z0 + MAP {
                    self.fill(x0, sz1, MAP, z0 + MAP - sz1, height, column);
                }
                if x0 < ox0 {
                    self.fill(x0, sz0, ox0 - x0, sz1 - sz0, height, column);
                }
                if ox0 + MAP < x0 + MAP {
                    self.fill(ox0 + MAP, sz0, x0 - ox0, sz1 - sz0, height, column);
                }
            }
            _ => self.fill(x0, z0, MAP, MAP, height, column),
        }
        self.centre = Some(centre);
        // The sweep waits while a crossing fills, so a frame pays for one or the other. MAP is a
        // multiple of SWEEP_ROWS: its rows never wrap.
        if !crossed {
            self.fill(x0, z0 + self.sweep, MAP, SWEEP_ROWS, height, column);
            self.sweep = (self.sweep + SWEEP_ROWS) % MAP;
        }
    }
    /// Fills the `w` x `h` columns from (`x0`, `z0`), marking the words that changed.
    fn fill(
        &mut self,
        x0: i32,
        z0: i32,
        w: i32,
        h: i32,
        height: &impl Fn(i32, i32) -> i32,
        column: &impl Fn(i32, i32, i32) -> (Surface, u8, bool),
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        // The rectangle's heights and its border's (not the corners, which no column reads).
        let stride = (w + 2) as usize;
        let mut heights = std::mem::take(&mut self.heights);
        heights.clear();
        heights.resize(stride * (h + 2) as usize, 0);
        for dz in -1..=h {
            for dx in -1..=w {
                let corner = (dx == -1 || dx == w) && (dz == -1 || dz == h);
                if !corner {
                    heights[(dz + 1) as usize * stride + (dx + 1) as usize] =
                        height(x0 + dx, z0 + dz);
                }
            }
        }
        let at = |dx: i32, dz: i32| heights[(dz + 1) as usize * stride + (dx + 1) as usize];
        for dz in 0..h {
            let cz = z0 + dz;
            let row = cz.rem_euclid(MAP) as usize;
            for dx in 0..w {
                let cx = x0 + dx;
                let hh = at(dx, dz);
                let (surface, light, rains) = column(cx, cz, hh);
                let higher = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .filter(|&&(ex, ez)| at(dx + ex, dz + ez) > hh)
                    .count() as u8;
                let packed = Column {
                    height: hh,
                    surface,
                    light,
                    rains,
                    higher,
                }
                .pack(cx, cz);
                let col = cx.rem_euclid(MAP) as usize;
                let cell = &mut self.cells[row * MAP as usize + col];
                if *cell != packed {
                    *cell = packed;
                    let span = &mut self.dirty[row];
                    *span = if span.0 >= span.1 {
                        (col as u16, col as u16 + 1)
                    } else {
                        (span.0.min(col as u16), span.1.max(col as u16 + 1))
                    };
                }
            }
        }
        self.heights = heights;
    }
}

/// The ranges of a [`RainMap::take_upload`]: each its first word's index in the map and its words.
pub fn upload_ranges(upload: &[u32]) -> impl Iterator<Item = (usize, &[u32])> {
    let mut rest = upload;
    std::iter::from_fn(move || {
        let [from, len, tail @ ..] = rest else {
            return None;
        };
        let (words, tail) = tail.split_at((*len as usize).min(tail.len()));
        rest = tail;
        Some((*from as usize, words))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_fall_from_two_to_nine_metres_a_second() {
        assert!((terminal_velocity(0.5) - 2.0).abs() < 0.1);
        assert!((terminal_velocity(1.0) - 4.0).abs() < 0.1);
        assert!((terminal_velocity(2.0) - 6.5).abs() < 0.2);
        assert!((terminal_velocity(5.0) - 9.1).abs() < 0.1);
        assert!(terminal_velocity(0.05) >= 0.0);
    }

    #[test]
    fn harder_rain_has_larger_drops() {
        // The mean diameter is 1 / slope.
        assert!(1.0 / slope(1.0) < 1.0 / slope(5.0));
        assert!(1.0 / slope(5.0) < 1.0 / slope(30.0));
        assert!((slope(1.0) - 4.1).abs() < 1e-4);
    }

    #[test]
    fn the_drops_extinction_brackets_remix_rain_and_thunder() {
        // Remix: rain 0.996 over 10 m, thunder 0.97 over 10 m.
        let remix_rain = -(0.996f32.ln()) / 10.0;
        let remix_thunder = -(0.97f32.ln()) / 10.0;
        assert!(extinction(1.0) < remix_rain && remix_rain < extinction(3.0));
        assert!(extinction(15.0) < remix_thunder && remix_thunder < extinction(40.0));
        assert_eq!(extinction(0.0), 0.0);
    }

    #[test]
    fn vanilla_levels_give_moderate_rain_and_downpours() {
        assert_eq!(rate_mm_h(0.0, 0.0), 0.0);
        assert_eq!(rate_mm_h(1.0, 0.0), RAIN_MM_H);
        assert_eq!(rate_mm_h(1.0, 1.0), THUNDER_MM_H);
        assert!((rate_mm_h(0.5, 0.0) - RAIN_MM_H / 2.0).abs() < 1e-6);
    }

    #[test]
    fn the_world_soaks_then_dries_puddles_last_and_sun_first() {
        let mut w = Wetness::default();
        for _ in 0..60 {
            w.step(1.0, RAIN_MM_H, 1.0);
        }
        assert!(w.wet == 1.0 && w.wet_sun == 1.0);
        assert!(w.puddles > 0.0 && w.puddles < 1.0);
        for _ in 0..600 {
            w.step(1.0, RAIN_MM_H, 1.0);
        }
        assert_eq!(w.puddles, 1.0);
        for _ in 0..300 {
            w.step(1.0, 0.0, 1.0);
        }
        assert!(w.wet_sun < w.wet, "the sun dries first");
        assert!(w.puddles > w.wet, "the puddles last");
        for _ in 0..3000 {
            w.step(1.0, 0.0, 1.0);
        }
        assert!(!w.any());
    }

    #[test]
    fn blocks_are_classed_by_what_they_are_made_of() {
        let s = |name, step, top| classify(name, step, top, false, false);
        let stone = "minecraft:block.stone.step";
        assert_eq!(s("minecraft:cobblestone", stone, Some(1.0)), Surface::Stone);
        assert_eq!(
            s(
                "minecraft:oak_planks",
                "minecraft:block.wood.step",
                Some(1.0)
            ),
            Surface::Wood
        );
        assert_eq!(
            s("minecraft:dirt", "minecraft:block.gravel.step", Some(1.0)),
            Surface::Soil
        );
        assert_eq!(
            s(
                "minecraft:dirt_path",
                "minecraft:block.grass.step",
                Some(0.9375)
            ),
            Surface::Path
        );
        assert_eq!(
            s("minecraft:sand", "minecraft:block.sand.step", Some(1.0)),
            Surface::Sand
        );
        assert_eq!(
            s("minecraft:red_sandstone", stone, Some(1.0)),
            Surface::Stone
        );
        assert_eq!(
            s(
                "minecraft:white_wool",
                "minecraft:block.wool.step",
                Some(1.0)
            ),
            Surface::Wool
        );
        assert_eq!(
            s(
                "minecraft:iron_block",
                "minecraft:block.metal.step",
                Some(1.0)
            ),
            Surface::Runoff
        );
        assert_eq!(
            s("minecraft:glass", "minecraft:block.glass.step", Some(1.0)),
            Surface::Runoff
        );
        assert_eq!(
            s("minecraft:oak_leaves", "minecraft:block.grass.step", None),
            Surface::Leaves
        );
        assert_eq!(
            s("minecraft:stone_slab", stone, Some(0.5)),
            Surface::Partial
        );
        assert_eq!(
            s(
                "minecraft:grass_block",
                "minecraft:block.grass.step",
                Some(1.0)
            ),
            Surface::Grass
        );
        assert_eq!(s("minecraft:snow_block", "", Some(1.0)), Surface::Dry);
        assert_eq!(
            classify("minecraft:water", "", None, true, false),
            Surface::Water
        );
        assert_eq!(
            classify("minecraft:lava", "", None, false, true),
            Surface::Dry
        );
    }

    #[test]
    fn a_column_packs_into_one_word() {
        let c = Column {
            height: -64,
            surface: Surface::Grass,
            light: 14,
            rains: true,
            higher: 3,
        };
        let p = c.pack(-1, 300);
        assert_eq!(p & 4095, 2048 - 64);
        assert_eq!(p >> 12 & 15, Surface::Grass as u32);
        assert_eq!(p >> 16 & 15, 14);
        assert_eq!(p >> 20 & 1, 1);
        assert_eq!(p >> 21 & 7, 3);
        assert_eq!(p >> 24 & 15, 15);
        assert_eq!(p >> 28, 1);
    }

    #[test]
    fn the_map_follows_the_camera_and_fills_new_strips() {
        let mut map = RainMap::default();
        let column = |_: i32, _: i32, _: i32| (Surface::Stone, 0, true);
        map.update(0, 0, |x, _| x, column);
        let at = |map: &RainMap, x: i32, z: i32| {
            map.cells()[(z.rem_euclid(MAP) * MAP + x.rem_euclid(MAP)) as usize]
        };
        assert_eq!(at(&map, 100, 5) & 4095, 2148);
        assert_eq!(
            at(&map, 100, 5) >> 21 & 7,
            1,
            "the east neighbour is higher"
        );
        // 160 blocks east: the columns past the old window are filled, wrapped onto old texels.
        map.update(160, 0, |x, _| x, column);
        assert_eq!(at(&map, 250, 5) & 4095, 2048 + 250);
        assert_eq!(at(&map, 250, 5) >> 24 & 15, 0);
        assert!(map.take_upload().is_some());
    }

    /// Applies an upload to a copy of the GPU's buffer.
    fn apply(gpu: &mut [u32], upload: Option<Vec<u32>>) -> usize {
        let Some(upload) = upload else { return 0 };
        let mut words = 0;
        for (from, cells) in upload_ranges(&upload) {
            gpu[from..from + cells.len()].copy_from_slice(cells);
            words += cells.len();
        }
        words
    }

    #[test]
    fn the_uploads_keep_the_gpu_copy_equal_and_a_fresh_fill_agrees() {
        let height = |x: i32, z: i32| ((x * 7 + z * 13).rem_euclid(23)) - 5;
        let column = |x: i32, z: i32, h: i32| {
            (
                if (x + z + h) % 3 == 0 {
                    Surface::Soil
                } else {
                    Surface::Stone
                },
                (x.rem_euclid(16)) as u8,
                z % 2 == 0,
            )
        };
        let mut map = RainMap::default();
        let mut gpu = vec![0; (MAP * MAP) as usize];
        // Flying east, a jump, then diagonally and back north-west: the strips and the sweep
        // leave the map as one fill where it ends would.
        let mut path = Vec::new();
        for i in 0..200 {
            path.push((i * 3, 0));
        }
        for i in 0..200 {
            path.push((5000 + i * 2, -3000 + i * 2));
        }
        for i in 0..100 {
            path.push((5400 - i * 5, -2600 - i * 4));
        }
        for &(x, z) in &path {
            map.update(x, z, height, column);
            apply(&mut gpu, map.take_upload());
            assert!(gpu == map.cells(), "the GPU's copy at ({x}, {z})");
        }
        let (x, z) = path[path.len() - 1];
        let mut fresh = RainMap::default();
        fresh.update(x, z, height, column);
        assert!(map.cells() == fresh.cells());
    }

    #[test]
    fn standing_still_uploads_only_what_changed() {
        let column = |_: i32, _: i32, _: i32| (Surface::Stone, 0, true);
        let mut map = RainMap::default();
        let mut gpu = vec![0; (MAP * MAP) as usize];
        map.update(0, 0, |_, _| 64, column);
        assert_eq!(apply(&mut gpu, map.take_upload()), (MAP * MAP) as usize);
        // A whole sweep round, nothing changed: nothing goes up.
        for _ in 0..MAP / SWEEP_ROWS {
            map.update(0, 0, |_, _| 64, column);
            assert!(map.take_upload().is_none());
        }
        // A block placed at (10, 20): its column and its four neighbours' "higher" change, in
        // three rows, within a sweep round, and only those words go up.
        let placed = |x: i32, z: i32| if (x, z) == (10, 20) { 65 } else { 64 };
        let mut sent = 0;
        for _ in 0..MAP / SWEEP_ROWS {
            map.update(0, 0, placed, column);
            sent += apply(&mut gpu, map.take_upload());
        }
        assert!(gpu == map.cells());
        assert_eq!(sent, 5, "the column and its four neighbours");
        assert_eq!(map.cells()[(20 * MAP + 10) as usize] & 4095, 2048 + 65);
        assert_eq!(map.cells()[(20 * MAP + 11) as usize] >> 21 & 7, 1);
    }

    #[test]
    fn a_crossing_uploads_its_strip_not_the_map() {
        let column = |_: i32, _: i32, _: i32| (Surface::Stone, 0, true);
        let mut map = RainMap::default();
        map.update(0, 0, |x, _| x, column);
        map.take_upload();
        // One chunk north: 16 new rows, in one or two ranges.
        map.update(0, -16, |x, _| x, column);
        let upload = map.take_upload().expect("the new rows");
        let words: usize = upload_ranges(&upload).map(|(_, w)| w.len()).sum();
        assert_eq!(words, (16 * MAP) as usize);
    }
}
