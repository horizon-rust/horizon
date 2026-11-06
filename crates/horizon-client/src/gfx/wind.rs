//! The wind over the landscape (RT lighting): a map of how the terrain shapes the wind, worked
//! out ahead from the far field's heights, and the leaves, plants and vines swaying in it, moved
//! in the terrain's vertex shader (terrain.wgsl `wind_offset`).
//!
//! **The map** (`terrain_map`) holds, for every 8x8 blocks of the 2048 around the camera (the far
//! field's heights, `far_heights` level 2: the tops of the terrain and its trees), what does not
//! depend on where the wind blows from: how exposed the place is (its height against the mean
//! around it within 32 and 128 blocks: a ridge or a summit catches the wind, a hollow is
//! sheltered), the terrain's slope (the wind speeds up blowing uphill, the lee side is calm), and
//! how much a valley confines it (the curvature across it, its walls rising on both sides) with
//! the valley's axis, along which the wind is turned and funnelled through the passes. The
//! shader takes the prevailing wind (`rain::Rain::wind`: a breeze turning slowly, stronger in a
//! storm) and bends it there, faster with the altitude.
//!
//! **The swaying** follows what the popular shader packs do (BSL, Complementary, Sildur's: a
//! block id from `block.properties` picks the leaves and the plants, the vertex shader moves
//! them): leaves flutter a few centimetres, every vertex with them; a plant is anchored at its
//! foot and bends at its top (the top of its sprite, as packs tell by `mc_midTexCoord`); a tall
//! plant's lower half moves at its top by half as much as the upper half's top; vines flutter.
//! Gusts are a noise blown across the world with the wind, so waves run through the grass, and
//! the sky light at a vertex (packs' `lmcoord.y`) leaves caves and roofed places still. Which
//! sprite layer is which (`wind_layers`) comes from the blocks' classes and their models' faces,
//! the way `transmission::thin_layers` finds the thin ones.
//!
//! **With the rays**: the sections' structures stay as built (rebuilding every swaying section's
//! every frame would cost far more than the effect), so the shadows the foliage casts stay put;
//! the visible surfaces move, and a ray leaving one of them could meet its own unmoved copy a few
//! centimetres away. The rays' alpha test (shadow_rays_hw.wgsl `hw_opaque`, horizon-rtx
//! `sun.rahit`) ignores a cutout quad met within `SELF_REACH` of the ray's start while the wind
//! is on. `HORIZON_RT_SKIP` bit 4096 turns all of it off (`horizon-ctl rt-skip` too).

use crate::assets::RenderAssets;
use crate::blocks::BlockFacts;
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;
use std::time::{Duration, Instant};

/// `HORIZON_RT_SKIP`'s bit for the wind.
pub const SKIP_BIT: u32 = 4096;
/// The kernel's and the any-hit's reach for a swaying quad's own copy (blocks): above the leaves'
/// flutter, under the gap between two blocks' contact shadows.
pub const SELF_REACH: f32 = 0.1;
/// Texels a side of the map (the far field's level 2).
pub const SIDE: usize = 256;
/// Blocks a texel side.
pub const CELL: i32 = 8;
/// The far field's level the map is made from.
pub const LEVEL: usize = 2;
/// Words of the header before the map (terrain.wgsl `WIND_MAP`).
const HEADER: usize = 16;
/// Words a texel (terrain.wgsl `wind_texel`).
const TEXEL_WORDS: usize = 2;
/// Where the layers' classes start (terrain.wgsl `WIND_LAYERS`), four a word.
const LAYERS_AT: usize = HEADER + SIDE * SIDE * TEXEL_WORDS;
/// Sprite layers the buffer holds (`mesher::OPAQUE_LAYER`).
const MAX_LAYERS: usize = 1 << 15;
/// The map is made again at most this often, when the heights changed.
const REMAP: Duration = Duration::from_secs(2);

/// A sprite layer's way of moving (terrain.wgsl `wind_offset`).
pub mod class {
    pub const NONE: u8 = 0;
    pub const LEAVES: u8 = 1;
    /// Anchored at its foot: grass, ferns, flowers, saplings, crops.
    pub const PLANT: u8 = 2;
    /// A tall plant's lower half and upper half.
    pub const TALL_LOWER: u8 = 3;
    pub const TALL_UPPER: u8 = 4;
    pub const VINE: u8 = 5;
}

/// The class of block state `s`'s faces, by its Java classes and its name (no namespace).
fn block_class(name: &str, has_class: impl Fn(&str) -> bool) -> u8 {
    if has_class("LeavesBlock") {
        return class::LEAVES;
    }
    if has_class("VineBlock") {
        return class::VINE;
    }
    // Flat on the ground or on water, under water, or a stem stacked block on block (each
    // block's foot would stand still between two moving tops): no wind.
    let still = [
        "WaterlilyBlock",
        "FlowerBedBlock",
        "LeafLitterBlock",
        "SeagrassBlock",
        "TallSeagrassBlock",
        "KelpBlock",
        "KelpPlantBlock",
        "MushroomBlock",
        "SugarCaneBlock",
        "BambooStalkBlock",
        "BambooSaplingBlock",
        "CactusBlock",
    ];
    if still.iter().any(|c| has_class(c)) || name.contains("coral") {
        return class::NONE;
    }
    if has_class("DoublePlantBlock") {
        // The half comes from the face's sprite (`face_class`).
        return class::TALL_LOWER;
    }
    if has_class("VegetationBlock") || has_class("DryVegetationBlock") {
        return class::PLANT;
    }
    class::NONE
}

/// A tall plant's face's class by its sprite (`minecraft:block/tall_grass_top`): the upper
/// half's sprites end in `_top` (and the sunflower's head), the rest are the lower half's.
fn tall_half(sprite: &str) -> u8 {
    let name = sprite.rsplit('/').next().unwrap_or(sprite);
    if name.ends_with("_top") || (name.starts_with("sunflower_") && !name.ends_with("_bottom")) {
        class::TALL_UPPER
    } else {
        class::TALL_LOWER
    }
}

/// Each sprite layer's class (`layers` of them), from the blocks whose models draw it. A layer
/// two classes draw keeps the first found.
pub fn wind_layers(facts: &BlockFacts, render: &RenderAssets, layers: usize) -> Vec<u8> {
    let mut out = vec![class::NONE; layers];
    let names: std::collections::HashMap<u32, &str> = render
        .sprite_layers
        .iter()
        .map(|(name, &layer)| (layer, name.as_str()))
        .collect();
    let mut models = Vec::new();
    for (id, s) in facts.states.iter().enumerate() {
        let name = s.name.strip_prefix("minecraft:").unwrap_or(&s.name);
        let kind = block_class(name, |c| s.has_class(c));
        if kind == class::NONE {
            continue;
        }
        for (x, z) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let at = BlockPos::new(x, 0, z);
            if render
                .model_catalog
                .models_at(BlockStateId(id as u32), at, &mut models)
                .is_err()
            {
                break;
            }
            for face in models.iter().flat_map(|m| &m.faces) {
                let kind = if kind == class::TALL_LOWER {
                    names.get(&face.layer).map_or(kind, |n| tall_half(n))
                } else {
                    kind
                };
                if let Some(c) = out.get_mut(face.layer as usize)
                    && *c == class::NONE
                {
                    *c = kind;
                }
            }
        }
    }
    out
}

/// A snorm byte of `v` (-1 to 1), as WGSL's `unpack4x8snorm` reads it.
fn snorm(v: f32) -> u32 {
    u32::from((v.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8)
}

/// The map from the far field's level-2 heights (`side` x `side`, slots toroidal: a world texel's
/// slot is its coordinates modulo `side`), the window's first texel at `origin` (world level-2
/// texels): two words a texel by slot, (exposure, slope x, slope z, confinement) and (the
/// valley's axis x, z), as snorm bytes.
pub fn terrain_map(heights: &[i16], side: usize, origin: [i32; 2]) -> Vec<u32> {
    let n = side;
    let mask = n as i32 - 1;
    let slot = |i: usize, j: usize| {
        (((origin[1] + j as i32) & mask) as usize) * n + ((origin[0] + i as i32) & mask) as usize
    };
    // The window in its own order; an unknown height (the window's edges, a column not loaded)
    // stands at the sea.
    let low = crate::chunk::MIN_Y as i16;
    let h: Vec<f32> = (0..n * n)
        .map(|k| {
            let v = heights[slot(k % n, k / n)];
            if v <= low { 63.0 } else { f32::from(v) }
        })
        .collect();
    // A summed-area table for the means over squares.
    let mut sat = vec![0f64; (n + 1) * (n + 1)];
    for j in 0..n {
        let mut row = 0f64;
        for i in 0..n {
            row += f64::from(h[j * n + i]);
            sat[(j + 1) * (n + 1) + i + 1] = sat[j * (n + 1) + i + 1] + row;
        }
    }
    let mean = |i: usize, j: usize, r: usize| -> f32 {
        let (x0, z0) = (i.saturating_sub(r), j.saturating_sub(r));
        let (x1, z1) = ((i + r + 1).min(n), (j + r + 1).min(n));
        let s = sat[z1 * (n + 1) + x1] - sat[z0 * (n + 1) + x1] - sat[z1 * (n + 1) + x0]
            + sat[z0 * (n + 1) + x0];
        (s / ((x1 - x0) * (z1 - z0)) as f64) as f32
    };
    // The terrain smoothed over 40 blocks, for its slopes and curvature.
    let smooth: Vec<f32> = (0..n * n).map(|k| mean(k % n, k / n, 2)).collect();
    let m = |i: i32, j: i32| smooth[(j.clamp(0, mask) as usize) * n + i.clamp(0, mask) as usize];
    let cell = CELL as f32;
    let mut out = vec![0u32; n * n * TEXEL_WORDS];
    for j in 0..n {
        for i in 0..n {
            let here = h[j * n + i];
            let near = here - mean(i, j, 4);
            let wide = here - mean(i, j, 16);
            let exposure = (0.5 * near + 0.5 * wide) / 16.0;
            let (x, z) = (i as i32, j as i32);
            let c = m(x, z);
            let slope = [
                (m(x + 1, z) - m(x - 1, z)) / (2.0 * cell),
                (m(x, z + 1) - m(x, z - 1)) / (2.0 * cell),
            ];
            // The curvature over 16 blocks each way: the larger eigenvalue is across a valley
            // (its walls rise on both sides), its eigenvector's normal the valley's axis.
            let d = 2.0 * cell;
            let a = (m(x + 2, z) - 2.0 * c + m(x - 2, z)) / (d * d);
            let cz = (m(x, z + 2) - 2.0 * c + m(x, z - 2)) / (d * d);
            let b = (m(x + 2, z + 2) - m(x + 2, z - 2) - m(x - 2, z + 2) + m(x - 2, z - 2))
                / (4.0 * d * d);
            let half = 0.5 * (a - cz);
            let across = 0.5 * (a + cz) + (half * half + b * b).sqrt();
            let v = if b.abs() > 1e-6 || (across - a).abs() > 1e-6 {
                [b, across - a]
            } else {
                [1.0, 0.0]
            };
            let len = (v[0] * v[0] + v[1] * v[1]).sqrt().max(1e-6);
            let axis = [-v[1] / len, v[0] / len];
            let confined =
                (across / 0.015).clamp(0.0, 1.0) * (0.5 - 3.0 * exposure).clamp(0.0, 1.0);
            let at = slot(i, j) * TEXEL_WORDS;
            out[at] = snorm(exposure)
                | snorm(slope[0]) << 8
                | snorm(slope[1]) << 16
                | snorm(confined) << 24;
            out[at + 1] = snorm(axis[0]) | snorm(axis[1]) << 8;
        }
    }
    out
}

/// What the header tells the shader this frame (terrain.wgsl `wind_header`).
pub struct Frame {
    /// The camera's place in the world.
    pub camera: glam::DVec3,
    /// Seconds of the animation's clock.
    pub seconds: f32,
    /// The prevailing wind, m/s along x and z.
    pub wind: [f32; 2],
    /// Whether the foliage sways (RT lighting in a level with weather, the bit clear).
    pub on: bool,
}

/// The wind's buffer (terrain.wgsl group 0, binding 25): the header, the map, the layers'
/// classes.
pub struct Wind {
    pub buffer: wgpu::Buffer,
    /// The far field's heights' generation the map was made from.
    made_from: Option<u64>,
    made_at: Option<Instant>,
    /// The map's first texel (world level-2 texels), once made.
    origin: Option<[i32; 2]>,
    /// `HORIZON_RT_SKIP` has bit 4096.
    pub skipped: bool,
}

impl Wind {
    pub fn new(device: &wgpu::Device) -> Self {
        let words = LAYERS_AT + MAX_LAYERS / 4;
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Wind"),
                size: (words * 4) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            made_from: None,
            made_at: None,
            origin: None,
            skipped: skipped(),
        }
    }

    /// The sprite layers' classes (`wind_layers`).
    pub fn set_layers(&self, queue: &wgpu::Queue, classes: &[u8]) {
        let words: Vec<u32> = classes
            .chunks(4)
            .take(MAX_LAYERS / 4)
            .map(|c| {
                c.iter()
                    .enumerate()
                    .fold(0, |w, (k, &b)| w | u32::from(b) << (8 * k))
            })
            .collect();
        if !words.is_empty() {
            queue.write_buffer(
                &self.buffer,
                (LAYERS_AT * 4) as u64,
                bytemuck::cast_slice(&words),
            );
        }
    }

    /// Makes the map again from the far field's heights when they changed, at most every
    /// `REMAP`: `heights` its level-2 texels, their window's first texel and their generation.
    pub fn update(&mut self, queue: &wgpu::Queue, heights: Option<([i32; 2], &[i16], u64)>) {
        let Some((origin, heights, generation)) = heights else {
            return;
        };
        let now = Instant::now();
        if self.made_from == Some(generation)
            || self.made_at.is_some_and(|t| now.duration_since(t) < REMAP)
                && self.origin == Some(origin)
        {
            return;
        }
        let map = crate::spans::time("wind_map", || terrain_map(heights, SIDE, origin));
        queue.write_buffer(
            &self.buffer,
            (HEADER * 4) as u64,
            bytemuck::cast_slice(&map),
        );
        self.made_from = Some(generation);
        self.made_at = Some(now);
        self.origin = Some(origin);
    }

    /// The header for `frame` (terrain.wgsl `wind_header`).
    pub fn header(&self, frame: &Frame) -> [u32; HEADER] {
        let f = f32::to_bits;
        let mut h = [0u32; HEADER];
        let c = frame.camera;
        if let Some(o) = self.origin {
            let first = [o[0] * CELL, o[1] * CELL];
            h[0] = f((f64::from(first[0]) - c.x) as f32);
            h[1] = f((f64::from(first[1]) - c.z) as f32);
            h[2] = o[0] as u32;
            h[3] = o[1] as u32;
            h[11] = 1;
        }
        h[4] = f(c.x.rem_euclid(4096.0) as f32);
        h[5] = f(c.y as f32);
        h[6] = f(c.z.rem_euclid(4096.0) as f32);
        h[7] = f(frame.seconds);
        h[8] = f(frame.wind[0]);
        h[9] = f(frame.wind[1]);
        h[10] = f(if frame.on && !self.skipped { 1.0 } else { 0.0 });
        h
    }
}

/// Whether `HORIZON_RT_SKIP` turns the wind off.
pub fn skipped() -> bool {
    std::env::var("HORIZON_RT_SKIP")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .is_some_and(|s| s & SKIP_BIT != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texel(map: &[u32], i: usize, j: usize) -> [f32; 6] {
        let w = |v: u32, k: u32| f32::from((v >> (8 * k)) as u8 as i8) / 127.0;
        let a = map[(j * SIDE + i) * 2];
        let b = map[(j * SIDE + i) * 2 + 1];
        [w(a, 0), w(a, 1), w(a, 2), w(a, 3), w(b, 0), w(b, 1)]
    }

    /// Heights by window texel, at window origin 0 (slots = window texels).
    fn map_of(f: impl Fn(f32, f32) -> f32) -> Vec<u32> {
        let heights: Vec<i16> = (0..SIDE * SIDE)
            .map(|k| f((k % SIDE) as f32, (k / SIDE) as f32) as i16)
            .collect();
        terrain_map(&heights, SIDE, [0, 0])
    }

    #[test]
    fn a_summit_is_exposed_and_a_hollow_sheltered() {
        let map = map_of(|x, z| {
            let d = ((x - 128.0).powi(2) + (z - 128.0).powi(2)).sqrt();
            100.0 + 60.0 * (-(d / 12.0).powi(2)).exp()
                - 40.0 * (-((x - 60.0).powi(2) + (z - 60.0).powi(2)) / 100.0).exp()
        });
        assert!(
            texel(&map, 128, 128)[0] > 0.5,
            "{:?}",
            texel(&map, 128, 128)
        );
        assert!(texel(&map, 60, 60)[0] < -0.5, "{:?}", texel(&map, 60, 60));
        assert!(texel(&map, 200, 200)[0].abs() < 0.05);
    }

    #[test]
    fn a_slope_points_uphill() {
        // Rising 1 block for 8 along +x.
        let map = map_of(|x, _| 80.0 + x);
        let t = texel(&map, 100, 100);
        assert!((t[1] - 0.125).abs() < 0.02, "{t:?}");
        assert!(t[2].abs() < 0.02, "{t:?}");
    }

    #[test]
    fn a_valley_confines_the_wind_along_its_axis() {
        // A valley along z: walls rising on both sides of x = 128.
        let map = map_of(|x, _| 80.0 + (x - 128.0).powi(2).min(900.0));
        let t = texel(&map, 128, 128);
        assert!(t[3] > 0.5, "confined {t:?}");
        assert!(t[4].abs() < 0.1 && t[5].abs() > 0.9, "axis along z {t:?}");
        // Flat ground confines nothing.
        let flat = map_of(|_, _| 70.0);
        assert_eq!(texel(&flat, 10, 10)[3], 0.0);
    }

    #[test]
    fn the_slots_are_toroidal() {
        // The same terrain seen from a window starting 3 texels further: each world texel
        // keeps its slot.
        let world = |x: i32, z: i32| 80.0 + 30.0 * ((x as f32) / 9.0).sin() + (z as f32) * 0.5;
        let at = |o: [i32; 2]| {
            let heights: Vec<i16> = (0..SIDE * SIDE)
                .map(|k| {
                    let (sx, sz) = ((k % SIDE) as i32, (k / SIDE) as i32);
                    // The world texel in this slot within the window.
                    let wx = o[0] + (sx - o[0]).rem_euclid(SIDE as i32);
                    let wz = o[1] + (sz - o[1]).rem_euclid(SIDE as i32);
                    world(wx, wz) as i16
                })
                .collect();
            terrain_map(&heights, SIDE, o)
        };
        let a = at([0, 0]);
        let b = at([3, 0]);
        // A texel well inside both windows.
        assert_eq!(a[(50 * SIDE + 120) * 2], b[(50 * SIDE + 120) * 2]);
    }

    #[test]
    fn the_shaders_read_the_buffer_as_it_is_written() {
        let terrain = include_str!("../terrain.wgsl");
        assert!(terrain.contains(&format!("const WIND_LAYERS: u32 = {LAYERS_AT}u;")));
        assert!(terrain.contains(&format!("const WIND_MAP: u32 = {HEADER}u;")));
        assert!(terrain.contains(&format!("const WIND_SIDE: i32 = {SIDE};")));
        assert!(terrain.contains(&format!("const WIND_CELL: f32 = {CELL}.0;")));
        let rays = include_str!("shadow_rays_hw.wgsl");
        assert!(rays.contains(&format!("const WIND_SELF_REACH: f32 = {SELF_REACH};")));
        assert!(
            include_str!("../../../horizon-rtx/shaders/sun.rahit")
                .contains(&format!("gl_HitTEXT < {SELF_REACH})"))
        );
    }

    #[test]
    fn the_classes_follow_the_blocks() {
        assert_eq!(
            block_class("oak_leaves", |c| c == "LeavesBlock"),
            class::LEAVES
        );
        assert_eq!(
            block_class("short_grass", |c| c == "VegetationBlock"),
            class::PLANT
        );
        assert_eq!(
            block_class("tall_grass", |c| c == "VegetationBlock"
                || c == "DoublePlantBlock"),
            class::TALL_LOWER
        );
        assert_eq!(
            block_class("lily_pad", |c| c == "VegetationBlock"
                || c == "WaterlilyBlock"),
            class::NONE
        );
        assert_eq!(block_class("vine", |c| c == "VineBlock"), class::VINE);
        assert_eq!(block_class("stone", |_| false), class::NONE);
        assert_eq!(
            tall_half("minecraft:block/tall_grass_top"),
            class::TALL_UPPER
        );
        assert_eq!(
            tall_half("minecraft:block/tall_grass_bottom"),
            class::TALL_LOWER
        );
        assert_eq!(
            tall_half("minecraft:block/sunflower_front"),
            class::TALL_UPPER
        );
        assert_eq!(
            tall_half("minecraft:block/sunflower_bottom"),
            class::TALL_LOWER
        );
    }
}
