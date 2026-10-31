//! Sound physics (a Horizon extra, off by default): the acoustics of Sound
//! Physics Remastered (SonicEther's Sound Physics, continued by Max Henkel and
//! others, GPL-3.0), ported to Rust for Horizon's own mixer. See
//! `docs/sound-physics.md`.
//!
//! When a positional sound starts, rays are traced through the blocks around
//! it: the blocks between it and the listener muffle it by their material
//! (occlusion), rays bounced around the source feed four reverb buses whose
//! decay times go from a small room to a large cave, the paths that reach the
//! listener move where the sound seems to come from (through a door rather
//! than a wall), and water filters it. This module is the calculation only,
//! over any [`Voxels`]; `audio` runs it on its own thread over a snapshot of
//! the chunk sections around the listener and applies the result
//! ([`Environment`]) in the mixer (`reverb`).
use glam::DVec3;

/// `SoundSource` indexes the calculation looks at.
const RECORDS: usize = 2;
const BLOCKS: usize = 4;
/// How far a ray is traced from the sound and from each bounce.
pub const RAY_LENGTH: f64 = 256.0;
/// The golden ratio spacing of the rays around a sound.
const PHI: f32 = 1.618_034;

/// The settings, with Sound Physics Remastered's names and defaults (its
/// `soundphysics.properties`), stored in `horizon-extras.txt` as
/// `soundPhysics.<name>`. Only `enabled` differs: off, as every extra.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    /// Horizon's switches for each effect (the mod has only `enabled`).
    pub reverb: bool,
    pub occlusion: bool,
    pub underwater: bool,
    /// Sounds come from where their reflections reach the listener
    /// (`sound_direction_evaluation`).
    pub reflections: bool,
    pub attenuation_factor: f32,
    pub reverb_attenuation_distance: f32,
    pub reverb_gain: f32,
    pub reverb_brightness: f32,
    pub reverb_distance: f32,
    pub block_absorption: f32,
    pub occlusion_variation: f32,
    pub default_block_reflectivity: f32,
    pub default_block_occlusion_factor: f32,
    pub air_absorption: f32,
    pub underwater_filter: f32,
    pub evaluate_ambient_sounds: bool,
    pub environment_evaluation_ray_count: u32,
    pub environment_evaluation_ray_bounces: u32,
    pub non_full_block_occlusion_factor: f32,
    pub max_occlusion_rays: u32,
    pub max_occlusion: f32,
    pub strict_occlusion: bool,
    pub redirect_non_occluded_sounds: bool,
    pub update_moving_sounds: bool,
    pub sound_update_interval: u32,
    pub max_sound_processing_distance: f64,
    /// Horizon's: playing sounds are traced again when the listener moves
    /// to another block (the mod traces a sound once, when it starts).
    pub update_on_listener_move: bool,
    /// Horizon's spatial extras (`spatial`), independent of `enabled`:
    /// binaural panning and the Doppler effect.
    pub hrtf: bool,
    pub doppler: bool,
    pub doppler_factor: f32,
    /// Flying arrows whoosh (`spatial::whoosh`): vanilla's are silent.
    pub arrow_whoosh: bool,
    /// Drawing a bow creaks (`spatial::creak_samples`): vanilla's is silent.
    pub bow_creak: bool,
    /// How far sounds carry against vanilla's (1: 16 blocks a unit of
    /// volume): the mixer's linear range, the air paths' reach, and in
    /// single player the integrated server's sending range (`shared_range`).
    pub sound_range: f32,
    /// Item pickups play lower and softer (`soft_pickup_pitch`): vanilla's
    /// pop sits an octave up half the time, piercing when mining in a row.
    pub soft_pickup: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            reverb: true,
            occlusion: true,
            underwater: true,
            reflections: true,
            attenuation_factor: 1.0,
            reverb_attenuation_distance: 0.0,
            reverb_gain: 1.0,
            reverb_brightness: 1.0,
            reverb_distance: 1.5,
            block_absorption: 1.0,
            occlusion_variation: 0.35,
            default_block_reflectivity: 0.5,
            default_block_occlusion_factor: 1.0,
            air_absorption: 1.0,
            underwater_filter: 0.9,
            evaluate_ambient_sounds: false,
            environment_evaluation_ray_count: 32,
            environment_evaluation_ray_bounces: 4,
            non_full_block_occlusion_factor: 0.25,
            max_occlusion_rays: 16,
            max_occlusion: 64.0,
            strict_occlusion: false,
            redirect_non_occluded_sounds: true,
            update_moving_sounds: false,
            sound_update_interval: 5,
            max_sound_processing_distance: 512.0,
            update_on_listener_move: true,
            hrtf: false,
            doppler: false,
            doppler_factor: 1.0,
            arrow_whoosh: false,
            bow_creak: false,
            sound_range: 1.0,
            soft_pickup: false,
        }
    }
}
/// The prefix of the settings' keys in `horizon-extras.txt`.
pub const KEY_PREFIX: &str = "soundPhysics.";
/// The values the options page cycles through.
pub const REVERB_GAINS: [f32; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
pub const BLOCK_ABSORPTIONS: [f32; 5] = [0.5, 1.0, 1.5, 2.0, 4.0];
pub const RAY_COUNTS: [u32; 4] = [8, 16, 32, 64];
pub const RAY_BOUNCES: [u32; 4] = [2, 4, 8, 16];
pub const SOUND_RANGES: [f32; 3] = [1.0, 1.5, 2.0];
/// The item pickup's pitch with `soft_pickup`: vanilla draws
/// `((r1 - r2) * 0.7 + 1) * 2`, centred an octave up and clamped by the engine
/// to 0.5..2 (half the pops at 2); this centres it a fifth lower (1.3) with
/// half the spread, 0.8 to 1.8.
pub fn soft_pickup_pitch(vanilla: f32) -> f32 {
    (1.3 + (vanilla - 2.0) * 0.35).clamp(0.8, 1.8)
}
/// How much softer the item pickup plays with `soft_pickup`.
pub const SOFT_PICKUP_VOLUME: f32 = 0.8;
/// The player's `sound_range` for the integrated server in this process,
/// which sends its single player's sounds that much further (`Hub::sound`):
/// the server has no settings of its own. A dedicated server's stays 1.
static SHARED_RANGE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);
pub fn set_shared_range(range: f32) {
    SHARED_RANGE.store(
        range.clamp(1.0, 3.0).to_bits(),
        std::sync::atomic::Ordering::Relaxed,
    );
}
pub fn shared_range() -> f32 {
    f32::from_bits(SHARED_RANGE.load(std::sync::atomic::Ordering::Relaxed))
}
/// The next value of `values` after `current` (before it with `back`), wrapping.
pub fn cycle<T: Copy + PartialOrd>(values: &[T], current: T, back: bool) -> T {
    let i = values.iter().position(|v| *v >= current).unwrap_or(0);
    let at = values.get(i).copied();
    let n = values.len();
    let next = if back {
        (i + n - 1) % n
    } else if at.is_some_and(|v| v > current) {
        // Between two steps: the next one up is the one found.
        i
    } else {
        (i + 1) % n
    };
    values[next]
}
impl Settings {
    /// Read a `soundPhysics.<name>` line of the extras file; false for any
    /// other key. Values out of the mod's ranges are clamped into them.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        let Some(name) = key.strip_prefix(KEY_PREFIX) else {
            return false;
        };
        let flag = |v: &mut bool| match value {
            "true" => *v = true,
            "false" => *v = false,
            _ => {}
        };
        let real = |v: &mut f32, lo: f32, hi: f32| {
            if let Ok(x) = value.parse::<f32>()
                && x.is_finite()
            {
                *v = x.clamp(lo, hi);
            }
        };
        let int = |v: &mut u32, lo: u32, hi: u32| {
            if let Ok(x) = value.parse::<u32>() {
                *v = x.clamp(lo, hi);
            }
        };
        match name {
            "enabled" => flag(&mut self.enabled),
            "reverb" => flag(&mut self.reverb),
            "occlusion" => flag(&mut self.occlusion),
            "underwater" => flag(&mut self.underwater),
            "reflections" | "sound_direction_evaluation" => flag(&mut self.reflections),
            "attenuation_factor" => real(&mut self.attenuation_factor, 0.1, 1.0),
            "reverb_attenuation_distance" => {
                real(&mut self.reverb_attenuation_distance, 0.0, 512.0)
            }
            "reverb_gain" => real(&mut self.reverb_gain, 0.1, 2.0),
            "reverb_brightness" => real(&mut self.reverb_brightness, 0.1, 2.0),
            "reverb_distance" => real(&mut self.reverb_distance, 0.1, 16.0),
            "block_absorption" => real(&mut self.block_absorption, 0.1, 4.0),
            "occlusion_variation" => real(&mut self.occlusion_variation, 0.0, 16.0),
            "default_block_reflectivity" => real(&mut self.default_block_reflectivity, 0.1, 4.0),
            "default_block_occlusion_factor" => {
                real(&mut self.default_block_occlusion_factor, 0.0, 10.0)
            }
            "air_absorption" => real(&mut self.air_absorption, 0.0, 5.0),
            "underwater_filter" => real(&mut self.underwater_filter, 0.0, 1.0),
            "evaluate_ambient_sounds" => flag(&mut self.evaluate_ambient_sounds),
            "environment_evaluation_ray_count" => {
                int(&mut self.environment_evaluation_ray_count, 8, 64)
            }
            "environment_evaluation_ray_bounces" => {
                int(&mut self.environment_evaluation_ray_bounces, 2, 64)
            }
            "non_full_block_occlusion_factor" => {
                real(&mut self.non_full_block_occlusion_factor, 0.0, 1.0)
            }
            "max_occlusion_rays" => int(&mut self.max_occlusion_rays, 1, 128),
            "max_occlusion" => real(&mut self.max_occlusion, 0.0, 1024.0),
            "strict_occlusion" => flag(&mut self.strict_occlusion),
            "redirect_non_occluded_sounds" => flag(&mut self.redirect_non_occluded_sounds),
            "update_moving_sounds" => flag(&mut self.update_moving_sounds),
            "update_on_listener_move" => flag(&mut self.update_on_listener_move),
            "hrtf" => flag(&mut self.hrtf),
            "doppler" => flag(&mut self.doppler),
            "doppler_factor" => real(&mut self.doppler_factor, 0.0, 4.0),
            "arrow_whoosh" => flag(&mut self.arrow_whoosh),
            "bow_creak" => flag(&mut self.bow_creak),
            "sound_range" => real(&mut self.sound_range, 1.0, 3.0),
            "soft_pickup" => flag(&mut self.soft_pickup),
            "sound_update_interval" => int(&mut self.sound_update_interval, 1, u32::MAX),
            "max_sound_processing_distance" => {
                if let Ok(x) = value.parse::<f64>()
                    && x.is_finite()
                {
                    self.max_sound_processing_distance = x.max(0.0);
                }
            }
            _ => {}
        }
        true
    }
    /// Every setting as `soundPhysics.<name>:<value>` lines.
    pub fn serialize(&self) -> String {
        let b = |v: bool| if v { "true" } else { "false" };
        let lines: [(&str, String); 35] = [
            ("enabled", b(self.enabled).into()),
            ("reverb", b(self.reverb).into()),
            ("occlusion", b(self.occlusion).into()),
            ("underwater", b(self.underwater).into()),
            ("reflections", b(self.reflections).into()),
            ("attenuation_factor", self.attenuation_factor.to_string()),
            (
                "reverb_attenuation_distance",
                self.reverb_attenuation_distance.to_string(),
            ),
            ("reverb_gain", self.reverb_gain.to_string()),
            ("reverb_brightness", self.reverb_brightness.to_string()),
            ("reverb_distance", self.reverb_distance.to_string()),
            ("block_absorption", self.block_absorption.to_string()),
            ("occlusion_variation", self.occlusion_variation.to_string()),
            (
                "default_block_reflectivity",
                self.default_block_reflectivity.to_string(),
            ),
            (
                "default_block_occlusion_factor",
                self.default_block_occlusion_factor.to_string(),
            ),
            ("air_absorption", self.air_absorption.to_string()),
            ("underwater_filter", self.underwater_filter.to_string()),
            (
                "evaluate_ambient_sounds",
                b(self.evaluate_ambient_sounds).into(),
            ),
            (
                "environment_evaluation_ray_count",
                self.environment_evaluation_ray_count.to_string(),
            ),
            (
                "environment_evaluation_ray_bounces",
                self.environment_evaluation_ray_bounces.to_string(),
            ),
            (
                "non_full_block_occlusion_factor",
                self.non_full_block_occlusion_factor.to_string(),
            ),
            ("max_occlusion_rays", self.max_occlusion_rays.to_string()),
            ("max_occlusion", self.max_occlusion.to_string()),
            ("strict_occlusion", b(self.strict_occlusion).into()),
            (
                "redirect_non_occluded_sounds",
                b(self.redirect_non_occluded_sounds).into(),
            ),
            ("update_moving_sounds", b(self.update_moving_sounds).into()),
            (
                "update_on_listener_move",
                b(self.update_on_listener_move).into(),
            ),
            ("hrtf", b(self.hrtf).into()),
            ("doppler", b(self.doppler).into()),
            ("doppler_factor", self.doppler_factor.to_string()),
            ("arrow_whoosh", b(self.arrow_whoosh).into()),
            ("bow_creak", b(self.bow_creak).into()),
            ("sound_range", self.sound_range.to_string()),
            ("soft_pickup", b(self.soft_pickup).into()),
            (
                "sound_update_interval",
                self.sound_update_interval.to_string(),
            ),
            (
                "max_sound_processing_distance",
                self.max_sound_processing_distance.to_string(),
            ),
        ];
        lines
            .iter()
            .map(|(name, value)| format!("{KEY_PREFIX}{name}:{value}\n"))
            .collect()
    }
}

/// What the rays need to know about a block state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Rays stop on it: it has a collision shape or a fluid.
    pub hit: bool,
    /// `isFaceSturdy(.., SupportType.FULL)` per `Direction` ordinal (down, up,
    /// north, south, west, east), as bits.
    pub sturdy: u8,
    pub water: bool,
    /// Sound energy a bounce off it keeps (Block Reflectivities).
    pub reflectivity: f32,
    /// How much a sound through it is muffled (Block Occlusion Factors).
    pub occlusion: f32,
    /// The axes (x, y, z as bits 0, 1, 2) its shape closes to sound going
    /// through the air from cell to cell: a full block all three, a closed
    /// door one, an open door the one it swung along, a fence none.
    pub closes: u8,
}
impl Material {
    pub const AIR: Material = Material {
        closes: 0,
        hit: false,
        sturdy: 0,
        water: false,
        reflectivity: 0.0,
        occlusion: 0.0,
    };
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidKind {
    None,
    Water,
    Lava,
}
/// A block state as the material table reads it.
pub struct StateInfo<'a> {
    /// `minecraft:stone`.
    pub block: &'a str,
    /// The `SoundType`'s break and step sounds (`minecraft:block.stone.break`).
    pub break_sound: &'a str,
    pub step_sound: &'a str,
    pub fluid: FluidKind,
    /// A non-empty collision shape.
    pub collides: bool,
    pub sturdy: u8,
    /// `closes_axes` of its shape.
    pub closes: u8,
}
/// The axes a shape (boxes in block space) closes: one of its boxes spans
/// at least 90 % of the cell across both other axes.
pub fn closes_axes(boxes: &[[f64; 6]]) -> u8 {
    let mut closes = 0;
    for b in boxes {
        let span = |a: usize| b[a + 3] - b[a];
        for axis in 0..3 {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            if span(u) >= 0.9 && span(v) >= 0.9 {
                closes |= 1 << axis;
            }
        }
    }
    closes
}
/// The mod's reflectivity defaults by `SoundType`, keyed by the sound type's
/// name in its sound events (`block.<name>.break`).
const REFLECTIVITY: [(&str, f32); 30] = [
    ("stone", 1.5),
    ("netherite_block", 1.5),
    ("tuff", 1.5),
    ("amethyst_block", 1.5),
    ("basalt", 1.5),
    ("calcite", 1.5),
    ("bone_block", 1.5),
    ("copper", 1.25),
    ("deepslate", 1.5),
    ("deepslate_bricks", 1.5),
    ("deepslate_tiles", 1.5),
    ("polished_deepslate", 1.5),
    ("nether_bricks", 1.5),
    ("netherrack", 1.1),
    ("nether_gold_ore", 1.1),
    ("nether_ore", 1.1),
    ("stem", 0.4),
    ("wool", 0.1),
    ("honey_block", 0.1),
    ("moss", 0.1),
    ("soul_sand", 0.2),
    ("soul_soil", 0.2),
    ("coral_block", 0.2),
    ("metal", 1.25),
    ("wood", 0.4),
    ("gravel", 0.3),
    ("grass", 0.3),
    ("glass", 0.75),
    ("sand", 0.2),
    ("snow", 0.15),
];
/// The mod's occlusion defaults by `SoundType`, keyed as `REFLECTIVITY`.
const OCCLUSION: [(&str, f32); 28] = [
    ("wool", 1.5),
    ("moss", 0.75),
    ("honey_block", 0.5),
    ("glass", 0.1),
    ("snow", 0.1),
    ("powder_snow", 0.1),
    ("bamboo", 0.1),
    ("bamboo_sapling", 0.1),
    ("wet_grass", 0.1),
    ("moss_carpet", 0.1),
    ("weeping_vines", 0.0),
    ("twisting_vines", 0.0),
    ("vine", 0.0),
    ("sweet_berry_bush", 0.0),
    ("spore_blossom", 0.0),
    ("small_dripleaf", 0.0),
    ("roots", 0.0),
    ("pointed_dripstone", 0.0),
    ("scaffolding", 0.0),
    ("glow_lichen", 0.0),
    ("crop", 0.0),
    ("fungus", 0.0),
    ("lily_pad", 0.0),
    ("large_amethyst_bud", 0.0),
    ("medium_amethyst_bud", 0.0),
    ("small_amethyst_bud", 0.0),
    ("ladder", 0.0),
    ("chain", 0.0),
];
/// `block.<name>.<what>` → `<name>`.
fn sound_type(event: &str) -> Option<&str> {
    let path = event.split_once(':').map_or(event, |(_, p)| p);
    let rest = path.strip_prefix("block.")?;
    Some(rest.rsplit_once('.').map_or(rest, |(name, _)| name))
}
fn lookup(table: &[(&str, f32)], info: &StateInfo<'_>) -> Option<f32> {
    // Sound types sharing a step sound (crops step like grass) differ in
    // their break sound, so it is looked up first.
    [info.break_sound, info.step_sound]
        .into_iter()
        .filter_map(sound_type)
        .find_map(|name| table.iter().find(|(n, _)| *n == name).map(|(_, v)| *v))
}
/// A state's material under `settings` (the defaults scale with them).
pub fn material(info: &StateInfo<'_>, settings: &Settings) -> Material {
    let block = info.block.trim_start_matches("minecraft:");
    let occlusion = match block {
        "water" => 0.25,
        "lava" => 0.75,
        "jukebox" => 0.0,
        _ => lookup(&OCCLUSION, info).unwrap_or(settings.default_block_occlusion_factor),
    };
    Material {
        hit: info.collides || info.fluid != FluidKind::None,
        sturdy: info.sturdy,
        water: info.fluid == FluidKind::Water,
        reflectivity: lookup(&REFLECTIVITY, info).unwrap_or(settings.default_block_reflectivity),
        occlusion,
        closes: if info.collides { info.closes } else { 0 },
    }
}

/// The blocks the rays travel through.
pub trait Voxels {
    fn material(&self, x: i32, y: i32, z: i32) -> Material;
}
/// Where a ray stopped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub cell: [i32; 3],
    /// The point on the face it entered by.
    pub at: DVec3,
    /// That face's outward normal.
    pub normal: [i32; 3],
}
fn cell_of(p: DVec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}
/// The first block from `from` to `to` that stops rays, other than `ignore`
/// and the cell `from` is in (a ray leaving a block does not hit it, as
/// `AABB.clip` from inside). Blocks count as whole cubes.
pub fn cast<V: Voxels + ?Sized>(
    voxels: &V,
    from: DVec3,
    to: DVec3,
    ignore: Option<[i32; 3]>,
) -> Option<Hit> {
    let d = to - from;
    if d.length_squared() < 1e-18 {
        return None;
    }
    let mut cell = cell_of(from);
    let (f, dir) = (from.to_array(), d.to_array());
    let mut step = [0i32; 3];
    let mut t_max = [f64::INFINITY; 3];
    let mut t_delta = [f64::INFINITY; 3];
    for a in 0..3 {
        if dir[a] > 0.0 {
            step[a] = 1;
            t_max[a] = (f64::from(cell[a]) + 1.0 - f[a]) / dir[a];
            t_delta[a] = 1.0 / dir[a];
        } else if dir[a] < 0.0 {
            step[a] = -1;
            t_max[a] = (f64::from(cell[a]) - f[a]) / dir[a];
            t_delta[a] = -1.0 / dir[a];
        }
    }
    loop {
        let a = if t_max[0] < t_max[1] {
            if t_max[0] < t_max[2] { 0 } else { 2 }
        } else if t_max[1] < t_max[2] {
            1
        } else {
            2
        };
        let t = t_max[a];
        if t > 1.0 {
            return None;
        }
        cell[a] += step[a];
        t_max[a] += t_delta[a];
        if Some(cell) == ignore || !voxels.material(cell[0], cell[1], cell[2]).hit {
            continue;
        }
        let mut normal = [0; 3];
        normal[a] = -step[a];
        return Some(Hit {
            cell,
            at: from + d * t,
            normal,
        });
    }
}
/// `Direction.getApproximateNearest` as an ordinal (down, up, north, south, west, east).
fn nearest_direction(v: DVec3) -> usize {
    let a = v.abs();
    if a.x >= a.y && a.x >= a.z {
        if v.x < 0.0 { 4 } else { 5 }
    } else if a.y >= a.z {
        if v.y < 0.0 { 0 } else { 1 }
    } else if v.z < 0.0 {
        2
    } else {
        3
    }
}

/// A sound to evaluate.
#[derive(Clone, Debug)]
pub struct Request {
    pub at: [f64; 3],
    pub listener: [f64; 3],
    /// The camera is in water (`isUnderWater`).
    pub listener_underwater: bool,
    /// The `SoundSource` index.
    pub source: usize,
    /// The sound event (`minecraft:block.stone.break`).
    pub event: String,
}
/// How the mixer plays a sound: the EFX parameters the mod sets on its
/// OpenAL source (the direct path's and each reverb send's low-pass gain and
/// high-frequency gain) and where it is heard from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Environment {
    pub direct_gain: f32,
    pub direct_cutoff: f32,
    pub send_gain: [f32; 4],
    pub send_cutoff: [f32; 4],
    /// The position the sound is heard from, when its reflections moved it.
    pub redirect: Option<[f64; 3]>,
}
impl Environment {
    /// `setDefaultEnvironment`: dry, unfiltered.
    pub const DRY: Environment = Environment {
        direct_gain: 1.0,
        direct_cutoff: 1.0,
        send_gain: [0.0; 4],
        send_cutoff: [1.0; 4],
        redirect: None,
    };
}
/// Whether a sound gets sound physics at all: ambient loops only with
/// `evaluate_ambient_sounds`, music discs only when moving sounds are
/// updated, and never the sounds the mod's rate config turns off (rain,
/// thunder, goat horns).
pub fn processed(event: &str, source: usize, settings: &Settings) -> bool {
    let path = event.split_once(':').map_or(event, |(_, p)| p);
    if !settings.evaluate_ambient_sounds && path.starts_with("ambient.") {
        return false;
    }
    if !settings.update_moving_sounds && source == RECORDS {
        return false;
    }
    !matches!(
        path,
        "weather.rain" | "weather.rain.above" | "entity.lightning_bolt.thunder"
    ) && !path.starts_with("item.goat_horn.sound.")
}
/// The blocks one path from the sound to the listener crosses, added up.
fn occlusion_along<V: Voxels + ?Sized>(
    voxels: &V,
    sound: DVec3,
    listener: DVec3,
    settings: &Settings,
) -> f64 {
    let mut total = 0.0f64;
    let mut origin = sound;
    let mut last = cell_of(sound);
    for _ in 0..settings.max_occlusion_rays {
        let Some(hit) = cast(voxels, origin, listener, Some(last)) else {
            break;
        };
        last = hit.cell;
        origin = hit.at;
        let m = voxels.material(hit.cell[0], hit.cell[1], hit.cell[2]);
        let centre = DVec3::new(
            f64::from(hit.cell[0]) + 0.5,
            f64::from(hit.cell[1]) + 0.5,
            f64::from(hit.cell[2]) + 0.5,
        );
        let side = nearest_direction(origin - centre);
        let mut occlusion = f64::from(m.occlusion);
        if m.sturdy & (1 << side) == 0 {
            occlusion *= f64::from(settings.non_full_block_occlusion_factor);
        }
        total += occlusion;
        if total > f64::from(settings.max_occlusion) {
            break;
        }
    }
    total
}
/// `calculateOcclusion`: the least occluded of nine parallel paths (one with
/// `strict_occlusion`), so a gap smaller than the variation still lets a
/// sound through; block sounds use at least 0.49 so the block itself does
/// not muffle them.
fn occlusion<V: Voxels + ?Sized>(
    voxels: &V,
    sound: DVec3,
    listener: DVec3,
    block_sound: bool,
    settings: &Settings,
) -> f64 {
    let max = f64::from(settings.max_occlusion);
    let mut least = occlusion_along(voxels, sound, listener, settings);
    let mut variation = f64::from(settings.occlusion_variation);
    if block_sound {
        variation = variation.max(0.49);
    }
    if !settings.strict_occlusion && variation > 0.0 {
        for x in [-1.0, 1.0] {
            for y in [-1.0, 1.0] {
                for z in [-1.0, 1.0] {
                    let offset = DVec3::new(x, y, z) * variation;
                    least = least.min(occlusion_along(
                        voxels,
                        sound + offset,
                        listener + offset,
                        settings,
                    ));
                }
            }
        }
    }
    least.min(max)
}
/// The ray directions around a sound, spread on a golden spiral.
pub fn ray_directions(count: u32) -> Vec<DVec3> {
    let g = PHI * std::f32::consts::TAU;
    (0..count)
        .map(|i| {
            let fi = i as f32 / count as f32;
            let longitude = g * i as f32;
            let latitude = (fi * 2.0 - 1.0).asin();
            DVec3::new(
                f64::from(latitude.cos() * longitude.cos()),
                f64::from(latitude.cos() * longitude.sin()),
                f64::from(latitude.sin()),
            )
        })
        .collect()
}
fn centre(cell: [i32; 3]) -> DVec3 {
    DVec3::new(
        f64::from(cell[0]) + 0.5,
        f64::from(cell[1]) + 0.5,
        f64::from(cell[2]) + 0.5,
    )
}
fn normal(hit: &Hit) -> DVec3 {
    DVec3::new(
        f64::from(hit.normal[0]),
        f64::from(hit.normal[1]),
        f64::from(hit.normal[2]),
    )
}
/// How far from the listener's cell the air paths are followed, each way,
/// at `sound_range` 1: a sound's usual 16-block range.
const FIELD_HALF: [i32; 3] = [16, 12, 16];
const UNREACHED: u16 = u16::MAX;
/// Steps through the air from the listener's cell to every cell around it
/// that sound can reach without going through a block (a breadth-first
/// flood): what tells a door from a wall.
struct Field {
    origin: [i32; 3],
    size: [i32; 3],
    steps: Vec<u16>,
}
impl Field {
    fn index(&self, cell: [i32; 3]) -> Option<usize> {
        let mut i = 0usize;
        for a in (0..3).rev() {
            let d = cell[a] - self.origin[a];
            if !(0..self.size[a]).contains(&d) {
                return None;
            }
            i = i * self.size[a] as usize + d as usize;
        }
        Some(i)
    }
    fn steps(&self, cell: [i32; 3]) -> u16 {
        self.index(cell).map_or(UNREACHED, |i| self.steps[i])
    }
    fn build<V: Voxels + ?Sized>(voxels: &V, listener: [i32; 3], range: f32) -> Self {
        let half: [i32; 3] = FIELD_HALF.map(|h| (h as f32 * range.clamp(1.0, 3.0)).round() as i32);
        let origin = std::array::from_fn(|a| listener[a] - half[a]);
        let size = half.map(|h| h * 2 + 1);
        let len = size.iter().map(|&s| s as usize).product();
        let mut field = Field {
            origin,
            size,
            steps: vec![UNREACHED; len],
        };
        let mut queue = std::collections::VecDeque::new();
        if let Some(i) = field.index(listener) {
            field.steps[i] = 0;
            queue.push_back((listener, 0u16));
        }
        while let Some((cell, steps)) = queue.pop_front() {
            for (axis, dir) in [(0, 1), (0, -1), (1, 1), (1, -1), (2, 1), (2, -1)] {
                let mut next = cell;
                next[axis] += dir;
                let Some(i) = field.index(next) else {
                    continue;
                };
                if field.steps[i] != UNREACHED
                    || voxels.material(next[0], next[1], next[2]).closes & (1 << axis) != 0
                {
                    continue;
                }
                field.steps[i] = steps + 1;
                queue.push_back((next, steps + 1));
            }
        }
        field
    }
}
/// What stays true between sounds while the listener stays in one cell and
/// no section changes: which cells the listener sees, and the air paths.
/// The sound physics thread keeps one; `invalidate` it when blocks change.
#[derive(Default)]
pub struct Cache {
    listener: Option<[i32; 3]>,
    visible: crate::fast_hash::FxHashMap<[i32; 3], bool>,
    field: Option<Field>,
    /// Visibility answers found in the cache, and traced.
    pub hits: u64,
    pub misses: u64,
}
impl Cache {
    pub fn invalidate(&mut self) {
        self.listener = None;
        self.visible.clear();
        self.field = None;
    }
    fn prepare(&mut self, listener: DVec3) {
        let cell = cell_of(listener);
        if self.listener != Some(cell) || self.visible.len() > 1 << 16 {
            self.invalidate();
            self.listener = Some(cell);
        }
    }
    /// The listener sees the centre of `cell`.
    fn sees<V: Voxels + ?Sized>(&mut self, voxels: &V, cell: [i32; 3], listener: DVec3) -> bool {
        if let Some(&seen) = self.visible.get(&cell) {
            self.hits += 1;
            return seen;
        }
        self.misses += 1;
        let seen = cast(voxels, centre(cell), listener, None).is_none();
        self.visible.insert(cell, seen);
        seen
    }
    /// The air cell in front of a hit, when the listener sees it: the
    /// vector from the listener to it (the mod's shared airspace).
    fn hit_airspace<V: Voxels + ?Sized>(
        &mut self,
        voxels: &V,
        hit: &Hit,
        listener: DVec3,
    ) -> Option<DVec3> {
        let air = std::array::from_fn(|a| hit.cell[a] + hit.normal[a]);
        self.sees(voxels, air, listener)
            .then(|| centre(air) - listener)
    }
    /// Where a sound the listener cannot see reaches them through the air:
    /// down the air path from the sound towards the listener, the first cell
    /// the listener sees (a doorway, the mouth of a tunnel). None when no air
    /// path within the field joins them.
    fn opening<V: Voxels + ?Sized>(
        &mut self,
        voxels: &V,
        sound: DVec3,
        listener: DVec3,
        range: f32,
    ) -> Option<DVec3> {
        let listener_cell = cell_of(listener);
        if self.field.is_none() {
            self.field = Some(Field::build(voxels, listener_cell, range));
        }
        let field = self.field.as_ref()?;
        let start = cell_of(sound);
        // A sound inside a block starts from its nearest reached neighbour.
        let mut cell = std::iter::once(start)
            .chain([0, 1, 2].into_iter().flat_map(|a| {
                [1, -1].map(|d| {
                    let mut c = start;
                    c[a] += d;
                    c
                })
            }))
            .min_by_key(|&c| field.steps(c))?;
        if field.steps(cell) == UNREACHED {
            return None;
        }
        let mut path = Vec::new();
        while field.steps(cell) > 0 {
            path.push(cell);
            let here = field.steps(cell);
            cell = (0..6)
                .map(|k| {
                    let mut c = cell;
                    c[k / 2] += if k % 2 == 0 { 1 } else { -1 };
                    c
                })
                .find(|&c| field.steps(c) == here - 1)?;
        }
        path.push(cell);
        let path = path.into_iter().find(|&c| self.sees(voxels, c, listener))?;
        Some(centre(path))
    }
}
/// The share of a bounce's energy a surface of the mod's reflectivity sends
/// on (stone 0.86, wood 0.23, wool 0.06): its absorption is the rest.
pub fn reflection(reflectivity: f32) -> f32 {
    (reflectivity * 0.57).clamp(0.03, 0.9)
}
/// Within this distance of the listener (blocks) a sound's reverb gives way
/// to its direct sound, down to `NEAR_FLOOR` at the listener: the player's
/// own sounds (an item picked up plays at the eyes) stay dry enough to
/// keep their place, and a short bright sound is not drawn out.
pub const NEAR_FIELD: f32 = 2.5;
pub const NEAR_FLOOR: f32 = 0.3;
/// The four buses' decay times (`reverb::presets`).
const BUS_DECAYS: [f32; 4] = [0.15, 0.55, 1.68, 4.142];
/// What the rays found about the space around a sound.
#[derive(Default, Debug)]
struct Room {
    /// Legs traced (a ray's first leg and each bounce), and how many hit.
    legs: u32,
    hits: u32,
    /// The hitting legs' total length.
    length: f64,
    /// The hits' total `reflection`.
    reflected: f64,
}
impl Room {
    /// Eyring's reverberation time: 0.161 V / (-S ln(1 - a)) with the mean
    /// free path 4 V / S the rays measured and the mean absorption a, an
    /// escaped leg absorbing everything.
    fn decay(&self) -> f32 {
        if self.hits == 0 {
            return 0.0;
        }
        let kept = (self.reflected / f64::from(self.legs)).clamp(1e-6, 0.999);
        let free_path = self.length / f64::from(self.hits);
        (0.161 / 4.0 * free_path / -kept.ln()).min(8.0) as f32
    }
    /// How much of the sound comes back: the energy kept per bounce.
    fn level(&self) -> f32 {
        if self.legs == 0 {
            return 0.0;
        }
        (1.2 * self.reflected / f64::from(self.legs)) as f32
    }
}
/// A reverberation time and level as the four buses' send gains: between
/// the two buses whose decays frame it (equal power on a log scale), the
/// shortest bus alone below it, the longest above.
fn bus_sends(decay: f32, level: f32) -> [f32; 4] {
    let mut sends = [0.0; 4];
    let level = level.clamp(0.0, 1.0);
    if decay <= 0.0 || level <= 0.0 {
        return sends;
    }
    if decay <= BUS_DECAYS[0] {
        sends[0] = level * (decay / BUS_DECAYS[0]).sqrt();
        return sends;
    }
    if decay >= BUS_DECAYS[3] {
        sends[3] = level;
        return sends;
    }
    let i = (0..3).find(|&i| decay < BUS_DECAYS[i + 1]).unwrap_or(2);
    let t = (decay / BUS_DECAYS[i]).ln() / (BUS_DECAYS[i + 1] / BUS_DECAYS[i]).ln();
    sends[i] = level * (1.0 - t).sqrt();
    sends[i + 1] = level * t.sqrt();
    sends
}
/// The environment of a sound at `request.at` heard at `request.listener`,
/// whose channel attenuates linearly to `max_distance` (vanilla's
/// `attenuationDistance`), after Sound Physics' `evaluateEnvironment`.
///
/// Occlusion, shared airspace and the filters are the mod's. Where it
/// differs (`docs/sound-physics.md`): the reverb comes from the room the
/// rays measure (Eyring's decay time from their mean free path and the
/// surfaces' absorption) rather than from path lengths scaled by
/// reflectivity, which made every closed stone room ring like a cathedral;
/// a hidden sound is heard from the first point of its air path the
/// listener sees (a doorway), with the mod's average of the reflections
/// only where no air path joins them; visibility is cached per cell; the
/// underwater filter applies last, so shared airspace does not undo it.
pub fn evaluate<V: Voxels + ?Sized>(
    voxels: &V,
    request: &Request,
    max_distance: f32,
    settings: &Settings,
    rays: &[DVec3],
    cache: &mut Cache,
) -> Environment {
    let sound = DVec3::from_array(request.at);
    let listener = DVec3::from_array(request.listener);
    if sound == DVec3::ZERO
        || sound.distance(listener) > settings.max_sound_processing_distance
        || !processed(&request.event, request.source, settings)
    {
        return Environment::DRY;
    }
    cache.prepare(listener);
    let absorption = f64::from(settings.block_absorption) * 3.0;
    let sound_cell = cell_of(sound);
    let underwater = settings.underwater
        && (request.listener_underwater
            || voxels
                .material(sound_cell[0], sound_cell[1], sound_cell[2])
                .water);
    let block_sound = request.source == BLOCKS || request.event.contains("block");
    let occluded = if settings.occlusion {
        occlusion(voxels, sound, listener, block_sound, settings)
    } else {
        0.0
    };
    let through = (-occluded * absorption).exp() as f32;
    let visible = cast(voxels, sound, listener, None).is_none();
    let direction =
        settings.reflections && (occluded > 0.0 || !settings.redirect_non_occluded_sounds);

    let bounces = settings.environment_evaluation_ray_bounces.max(1) as usize;
    let max_length =
        settings.max_sound_processing_distance / f64::from(settings.attenuation_factor);
    let mut room = Room::default();
    let mut shared = 0u32;
    // (vector from the listener, path length) of the hits the listener sees.
    let mut seen: Vec<(DVec3, f64)> = Vec::new();
    if settings.reverb || direction {
        for &first in rays {
            let (mut from, mut dir, mut ignore) = (sound, first, sound_cell);
            let mut travelled = 0.0;
            for _ in 0..=bounces {
                let hit = cast(voxels, from, from + dir * RAY_LENGTH, Some(ignore));
                let counts = travelled >= f64::from(settings.reverb_attenuation_distance);
                if counts {
                    room.legs += 1;
                }
                let Some(hit) = hit else {
                    break;
                };
                let leg = from.distance(hit.at);
                travelled += leg;
                if counts {
                    let m = voxels.material(hit.cell[0], hit.cell[1], hit.cell[2]);
                    room.hits += 1;
                    room.length += leg;
                    room.reflected += f64::from(reflection(m.reflectivity));
                }
                if let Some(v) = cache.hit_airspace(voxels, &hit, listener) {
                    let length = travelled + v.length();
                    if length > 0.0 && length <= max_length {
                        shared += 1;
                        seen.push((v, length));
                    }
                }
                let n = normal(&hit);
                dir -= n * (2.0 * dir.dot(n));
                (from, ignore) = (hit.at, hit.cell);
            }
        }
    }

    let redirect = if !direction {
        None
    } else if visible {
        // Seen directly: only `redirect_non_occluded_sounds` off moves it, to
        // the mod's average with the direct path weighted as the others.
        reflected_position(&seen, Some(sound - listener), sound, listener)
    } else {
        cache
            .opening(voxels, sound, listener, settings.sound_range)
            .map(|opening| {
                listener + (opening - listener).normalize_or_zero() * sound.distance(listener)
            })
            .or_else(|| reflected_position(&seen, None, sound, listener))
    };
    let heard_at = redirect.unwrap_or(sound);

    let rcp_total = 1.0 / (rays.len().max(1) * bounces) as f32;
    let shared = shared as f32 * 64.0 * rcp_total;
    let weights = [20.0f32, 15.0, 10.0, 10.0].map(|w| (shared / w).clamp(0.0, 1.0));
    let mut send_cutoff = weights.map(|w| through * (1.0 - w) + w);
    // Shared airspace lets some filtered dry signal through, which keeps the
    // direction of a sound behind a corner.
    let average = weights.iter().sum::<f32>() * 0.25;
    let mut direct_cutoff = (average.sqrt() * 0.2).max(through);
    if underwater {
        direct_cutoff *= 1.0 - settings.underwater_filter;
        for cutoff in &mut send_cutoff {
            *cutoff *= 0.4;
        }
    }
    let direct_gain = direct_cutoff.powf(0.1);

    // Surfaces absorb more of the highs than of the rest: each bounce keeps
    // a share of them equal to its share of the energy (a hard stone 0.86,
    // wood 0.23, wool 0.06), and the later buses hold later bounces.
    if room.hits > 0 {
        let surface = (room.reflected / f64::from(room.hits)).clamp(0.0, 1.0) as f32;
        for (k, cutoff) in send_cutoff.iter_mut().enumerate() {
            *cutoff *= surface.powi(k as i32 + 1);
        }
    }
    let mut send_gain = [0.0f32; 4];
    if settings.reverb {
        send_gain = bus_sends(room.decay(), room.level());
        for (gain, cutoff) in send_gain.iter_mut().zip(send_cutoff) {
            *gain *= cutoff.powf(0.1);
        }
        // Reverb fades with the sound's distance over `reverb_distance` times
        // its range (the mod's stopgap for OpenAL's room rolloff).
        let distance = listener.distance(heard_at) as f32;
        let range = max_distance * settings.sound_range / settings.attenuation_factor;
        let fade = 1.0 - (distance / (range * settings.reverb_distance)).min(1.0);
        let near = (sound.distance(listener) as f32 / NEAR_FIELD).clamp(NEAR_FLOOR, 1.0);
        for gain in &mut send_gain {
            *gain *= fade.max(0.0) * near;
        }
    }
    Environment {
        direct_gain,
        direct_cutoff,
        send_gain,
        send_cutoff,
        redirect: redirect.map(|p| p.to_array()),
    }
}
/// The mod's `ReflectedAudio.evaluateSoundPosition`: the directions of the
/// paths that reach the listener, weighted by the inverse square of their
/// length (the direct path too), at the sound's distance.
fn reflected_position(
    seen: &[(DVec3, f64)],
    direct: Option<DVec3>,
    sound: DVec3,
    listener: DVec3,
) -> Option<DVec3> {
    if seen.is_empty() {
        return None;
    }
    let weigh = |v: DVec3, length: f64| v.normalize_or_zero() / (length * length).max(1e-6);
    let mut sum = direct.map_or(DVec3::ZERO, |v| weigh(v, v.length()));
    for &(v, length) in seen {
        sum += weigh(v, length);
    }
    let n = sum.normalize_or_zero();
    (n.length() >= 0.5).then(|| listener + n * sound.distance(listener))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::HashMap;

    pub const STONE: Material = Material {
        hit: true,
        sturdy: 0b11_1111,
        water: false,
        reflectivity: 1.5,
        occlusion: 1.0,
        closes: 0b111,
    };
    const WOOL: Material = Material {
        reflectivity: 0.1,
        occlusion: 1.5,
        ..STONE
    };
    const GLASS: Material = Material {
        reflectivity: 0.75,
        occlusion: 0.1,
        ..STONE
    };
    const WATER: Material = Material {
        hit: true,
        sturdy: 0,
        water: true,
        reflectivity: 0.5,
        occlusion: 0.25,
        closes: 0,
    };
    /// Blocks in a map; everything else is air.
    #[derive(Default)]
    pub struct Blocks(pub HashMap<[i32; 3], Material>);
    impl Voxels for Blocks {
        fn material(&self, x: i32, y: i32, z: i32) -> Material {
            self.0.get(&[x, y, z]).copied().unwrap_or(Material::AIR)
        }
    }
    impl Blocks {
        pub fn fill(&mut self, lo: [i32; 3], hi: [i32; 3], m: Material) {
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        self.0.insert([x, y, z], m);
                    }
                }
            }
        }
        /// A hollow box of `m`: walls from `lo` to `hi`, air inside.
        pub fn room(lo: [i32; 3], hi: [i32; 3], m: Material) -> Self {
            let mut b = Blocks::default();
            b.fill(lo, hi, m);
            b.fill(
                [lo[0] + 1, lo[1] + 1, lo[2] + 1],
                [hi[0] - 1, hi[1] - 1, hi[2] - 1],
                Material::AIR,
            );
            b.0.retain(|_, m| m.hit);
            b
        }
    }
    fn request(at: [f64; 3], listener: [f64; 3]) -> Request {
        Request {
            at,
            listener,
            listener_underwater: false,
            source: 5,
            event: "minecraft:entity.zombie.ambient".into(),
        }
    }
    fn on() -> Settings {
        Settings {
            enabled: true,
            ..Settings::default()
        }
    }
    fn run(world: &Blocks, r: &Request, s: &Settings) -> Environment {
        evaluate(
            world,
            r,
            16.0,
            s,
            &ray_directions(s.environment_evaluation_ray_count),
            &mut Cache::default(),
        )
    }
    fn total(e: &Environment) -> f32 {
        e.send_gain.iter().sum()
    }

    #[test]
    fn settings_round_trip_and_clamp() {
        let d = Settings::default();
        assert!(!d.enabled, "an extra is off by default");
        let mut s = Settings::default();
        for line in d.serialize().lines() {
            let (k, v) = line.split_once(':').unwrap();
            assert!(s.parse_line(k, v));
        }
        assert_eq!(s, d);
        assert!(s.parse_line("soundPhysics.reverb_gain", "9"));
        assert_eq!(s.reverb_gain, 2.0);
        assert!(s.parse_line("soundPhysics.environment_evaluation_ray_count", "3"));
        assert_eq!(s.environment_evaluation_ray_count, 8);
        assert!(s.parse_line("soundPhysics.block_absorption", "nan"));
        assert_eq!(s.block_absorption, 1.0);
        assert!(!s.parse_line("fullbright", "true"));
        assert_eq!(cycle(&RAY_COUNTS, 32, false), 64);
        assert_eq!(cycle(&RAY_COUNTS, 64, false), 8);
        assert_eq!(cycle(&RAY_COUNTS, 8, true), 64);
        assert_eq!(cycle(&REVERB_GAINS, 1.1, false), 1.25);
    }

    #[test]
    fn soft_pickups_sit_lower_with_less_spread() {
        assert_eq!(soft_pickup_pitch(2.0), 1.3);
        // Vanilla's whole draw, before the engine's clamp.
        assert_eq!(soft_pickup_pitch(3.4), 1.79);
        assert!((soft_pickup_pitch(0.6) - 0.81).abs() < 1e-6);
        assert!(soft_pickup_pitch(0.5) >= 0.8);
    }

    #[test]
    fn materials_follow_the_sound_type() {
        let s = Settings::default();
        let state = |block, brk, step, fluid, collides| StateInfo {
            block,
            break_sound: brk,
            step_sound: step,
            fluid,
            collides,
            sturdy: 0b11_1111,
            closes: 0b111,
        };
        let stone = material(
            &state(
                "minecraft:stone",
                "minecraft:block.stone.break",
                "minecraft:block.stone.step",
                FluidKind::None,
                true,
            ),
            &s,
        );
        assert_eq!(
            (stone.reflectivity, stone.occlusion, stone.hit),
            (1.5, 1.0, true)
        );
        let wool = material(
            &state(
                "minecraft:white_wool",
                "minecraft:block.wool.break",
                "minecraft:block.wool.step",
                FluidKind::None,
                true,
            ),
            &s,
        );
        assert_eq!((wool.reflectivity, wool.occlusion), (0.1, 1.5));
        // Wheat steps like grass but breaks as a crop.
        let wheat = material(
            &state(
                "minecraft:wheat",
                "minecraft:block.crop.break",
                "minecraft:block.grass.step",
                FluidKind::None,
                false,
            ),
            &s,
        );
        assert_eq!((wheat.occlusion, wheat.hit), (0.0, false));
        let water = material(
            &state("minecraft:water", "", "", FluidKind::Water, false),
            &s,
        );
        assert!(water.hit && water.water);
        assert_eq!(water.occlusion, 0.25);
        let unknown = material(
            &state(
                "minecraft:anvil",
                "minecraft:block.anvil.break",
                "minecraft:block.anvil.step",
                FluidKind::None,
                true,
            ),
            &Settings {
                default_block_reflectivity: 0.7,
                ..s
            },
        );
        assert_eq!((unknown.reflectivity, unknown.occlusion), (0.7, 1.0));
    }

    #[test]
    fn rays_stop_on_the_first_solid_face() {
        let mut b = Blocks::default();
        b.fill([5, 0, 0], [5, 0, 0], STONE);
        let hit = cast(
            &b,
            DVec3::new(0.5, 0.5, 0.5),
            DVec3::new(10.5, 0.5, 0.5),
            None,
        )
        .unwrap();
        assert_eq!(hit.cell, [5, 0, 0]);
        assert_eq!(hit.normal, [-1, 0, 0]);
        assert!((hit.at.x - 5.0).abs() < 1e-9);
        assert!(
            cast(
                &b,
                DVec3::new(0.5, 0.5, 0.5),
                DVec3::new(10.5, 0.5, 0.5),
                Some([5, 0, 0])
            )
            .is_none()
        );
        // Backwards: the +x face.
        let hit = cast(
            &b,
            DVec3::new(9.5, 0.5, 0.5),
            DVec3::new(0.5, 0.5, 0.5),
            None,
        )
        .unwrap();
        assert_eq!(hit.normal, [1, 0, 0]);
        assert!((hit.at.x - 6.0).abs() < 1e-9);
        // Starting inside a block never hits it.
        assert!(
            cast(
                &b,
                DVec3::new(5.5, 0.5, 0.5),
                DVec3::new(5.5, 3.5, 0.5),
                None
            )
            .is_none()
        );
        // Diagonal through a corner gap.
        let hit = cast(
            &b,
            DVec3::new(0.2, 0.3, 0.5),
            DVec3::new(12.0, 0.9, 0.5),
            None,
        );
        assert_eq!(hit.map(|h| h.cell), Some([5, 0, 0]));
    }

    #[test]
    fn open_air_is_dry() {
        let e = run(
            &Blocks::default(),
            &request([0.5, 64.5, 0.5], [8.5, 64.5, 0.5]),
            &on(),
        );
        assert_eq!(e.direct_cutoff, 1.0);
        assert_eq!(e.direct_gain, 1.0);
        assert_eq!(total(&e), 0.0);
        assert_eq!(e.redirect, None);
    }

    #[test]
    fn walls_muffle_by_material_and_thickness() {
        let s = on();
        let listener = [10.5, 64.5, 0.5];
        let sound = [0.5, 64.5, 0.5];
        let wall = |m, thick: i32| {
            let mut b = Blocks::default();
            b.fill([4, 60, -4], [3 + thick, 69, 4], m);
            run(&b, &request(sound, listener), &s)
        };
        let glass = wall(GLASS, 1);
        let stone = wall(STONE, 1);
        let thick = wall(STONE, 3);
        let wool = wall(WOOL, 1);
        assert!(glass.direct_cutoff > stone.direct_cutoff);
        assert!(stone.direct_cutoff > thick.direct_cutoff);
        assert!(stone.direct_cutoff > wool.direct_cutoff);
        // exp(-1 * 3): one stone block.
        assert!(
            (stone.direct_cutoff - (-3.0f32).exp()).abs() < 1e-3,
            "{stone:?}"
        );
        // Switching occlusion off lets it through.
        let mut b = Blocks::default();
        b.fill([4, 60, -4], [4, 69, 4], STONE);
        let off = run(
            &b,
            &request(sound, listener),
            &Settings {
                occlusion: false,
                ..on()
            },
        );
        assert_eq!(off.direct_cutoff, 1.0);
    }

    #[test]
    fn small_gaps_let_sound_around_but_strict_occlusion_does_not() {
        // A post between them, the straight path near its edge: the offset
        // paths on the far side pass beside it.
        let mut b = Blocks::default();
        b.fill([5, 64, 0], [5, 64, 0], STONE);
        let r = request([0.5, 64.5, 0.8], [10.5, 64.5, 0.8]);
        assert_eq!(run(&b, &r, &on()).direct_cutoff, 1.0);
        let strict = run(
            &b,
            &r,
            &Settings {
                strict_occlusion: true,
                ..on()
            },
        );
        assert!(strict.direct_cutoff < 0.1);
    }

    /// The decay time the sends point to: their gains' weighted bus decay.
    fn decay_of(e: &Environment) -> f32 {
        let total = total(e);
        let log: f32 = e
            .send_gain
            .iter()
            .zip(BUS_DECAYS)
            .map(|(g, d)| g * g * d.ln())
            .sum::<f32>()
            / e.send_gain.iter().map(|g| g * g).sum::<f32>();
        assert!(total > 0.0, "{e:?}");
        log.exp()
    }

    #[test]
    fn rooms_and_caves_ring_for_their_size_and_walls() {
        let s = on();
        let small = Blocks::room([-3, 60, -3], [3, 66, 3], STONE);
        let large = Blocks::room([-20, 50, -20], [20, 80, 20], STONE);
        let r = request([-1.5, 63.5, 0.5], [1.5, 63.5, 0.5]);
        let small = run(&small, &r, &s);
        let r = request([-1.5, 60.5, 0.5], [1.5, 60.5, 0.5]);
        let large = run(&large, &r, &s);
        // A 5-block stone room rings under two seconds, never on the
        // longest bus; a 40-block cave on it.
        let (small_decay, large_decay) = (decay_of(&small), decay_of(&large));
        assert!((0.4..2.0).contains(&small_decay), "{small:?} {small_decay}");
        assert_eq!(small.send_gain[3], 0.0, "{small:?}");
        assert!(large_decay > 3.0, "{large:?} {large_decay}");
        assert!(large.send_gain[3] > 0.5, "{large:?}");
        // A bare floor outdoors barely reverberates, and briefly.
        let mut ground = Blocks::default();
        ground.fill([-30, 59, -30], [30, 59, 30], STONE);
        let outdoors = run(&ground, &request([-1.5, 60.5, 0.5], [1.5, 60.5, 0.5]), &s);
        assert!(total(&outdoors) < 0.3 * total(&small), "{outdoors:?}");
        assert_eq!(
            outdoors.send_gain[2] + outdoors.send_gain[3],
            0.0,
            "{outdoors:?}"
        );
        // Wool walls deaden the same room: quieter and shorter.
        let wool = run(
            &Blocks::room([-3, 60, -3], [3, 66, 3], WOOL),
            &request([-1.5, 63.5, 0.5], [1.5, 63.5, 0.5]),
            &s,
        );
        assert!(total(&wool) < 0.3 * total(&small), "{wool:?}");
        assert!(decay_of(&wool) < small_decay);
        // Reverb off.
        let off = run(
            &Blocks::room([-3, 60, -3], [3, 66, 3], STONE),
            &request([-1.5, 63.5, 0.5], [1.5, 63.5, 0.5]),
            &Settings {
                reverb: false,
                ..on()
            },
        );
        assert_eq!(total(&off), 0.0);
    }

    #[test]
    fn near_sounds_stay_dry_and_surfaces_darken_the_reverb() {
        let s = on();
        let cave = Blocks::room([-20, 50, -20], [20, 80, 20], STONE);
        let at = |d: f64| run(&cave, &request([0.5 + d, 60.5, 0.5], [0.5, 60.5, 0.5]), &s);
        // At the ears (an item picked up), a third of the reverb of a sound
        // three blocks away; the direct sound is untouched.
        let (ear, away) = (at(0.0), at(3.0));
        assert!(
            (total(&ear) / total(&away) - NEAR_FLOOR).abs() < 0.05,
            "{ear:?} {away:?}"
        );
        assert_eq!(ear.direct_cutoff, away.direct_cutoff);
        // Later buses are darker; wool far darker than stone.
        assert!(
            away.send_cutoff[3] < away.send_cutoff[0] && away.send_cutoff[0] < 0.9,
            "{away:?}"
        );
        let wool = Blocks::room([-20, 50, -20], [20, 80, 20], WOOL);
        let dark = run(&wool, &request([3.5, 60.5, 0.5], [0.5, 60.5, 0.5]), &s);
        assert!(dark.send_cutoff[0] < 0.1, "{dark:?}");
    }

    #[test]
    fn bus_sends_frame_the_decay_time() {
        assert_eq!(bus_sends(0.0, 1.0), [0.0; 4]);
        assert_eq!(bus_sends(9.0, 0.5), [0.0, 0.0, 0.0, 0.5]);
        let exact = bus_sends(0.55, 1.0);
        assert!((exact[1] - 1.0).abs() < 1e-6 && exact[0] == 0.0 && exact[2].abs() < 1e-6);
        let between = bus_sends(1.0, 1.0);
        assert!(between[1] > 0.0 && between[2] > 0.0 && between[0] == 0.0);
        // Equal power.
        assert!((between[1].powi(2) + between[2].powi(2) - 1.0).abs() < 1e-5);
    }

    /// A stone room east of the listener holding the sound, with a door
    /// (`door` cells removed, or a door block) in its north wall (-z), on
    /// the ground.
    fn room_with_door(door: Option<Material>) -> Blocks {
        let mut b = Blocks::room([4, 60, -4], [12, 68, 4], STONE);
        b.fill([-12, 60, -20], [20, 60, 8], STONE);
        for y in [61, 62] {
            match door {
                None => {
                    b.0.remove(&[8, y, -4]);
                }
                Some(m) => {
                    b.0.insert([8, y, -4], m);
                }
            }
        }
        b
    }

    #[test]
    fn sound_comes_through_the_door() {
        let r = request([8.5, 61.5, 0.5], [0.5, 61.5, -8.5]);
        let listener = DVec3::from_array(r.listener);
        let door = (DVec3::new(8.5, 61.5, -4.0) - listener).normalize();
        let source = (DVec3::from_array(r.at) - listener).normalize();
        // A one-by-two doorway, whatever the rays did: the air path finds it.
        let open = room_with_door(None);
        for rays in [8, 32, 64] {
            let e = run(
                &open,
                &r,
                &Settings {
                    environment_evaluation_ray_count: rays,
                    ..on()
                },
            );
            let to = e.redirect.expect("redirected through the door");
            assert!(e.direct_cutoff < 0.5, "the wall is still in the way: {e:?}");
            let heard = (DVec3::from_array(to) - listener).normalize();
            assert!(heard.dot(door) > 0.99, "{rays} rays: {heard:?}");
            assert!(heard.dot(door) > heard.dot(source));
            // Same distance as the real sound.
            let d0 = DVec3::from_array(r.at).distance(listener);
            assert!((DVec3::from_array(to).distance(listener) - d0).abs() < 1e-6);
        }
        // An open door (a slab along the wall, x) lets it through; a closed
        // one (across the doorway, z) does not, nor a wall.
        let slab = |closes| Material {
            closes,
            sturdy: 0,
            ..STONE
        };
        let swung = run(&room_with_door(Some(slab(0b001))), &r, &on());
        assert!(swung.redirect.is_some());
        assert_eq!(
            run(&room_with_door(Some(slab(0b100))), &r, &on()).redirect,
            None
        );
        assert_eq!(run(&room_with_door(Some(STONE)), &r, &on()).redirect, None);
        // Reflections off: it stays put.
        let off = Settings {
            reflections: false,
            ..on()
        };
        assert_eq!(run(&open, &r, &off).redirect, None);
    }

    #[test]
    fn closed_axes_follow_the_shape() {
        assert_eq!(closes_axes(&[[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]), 0b111);
        // A door against the north side: closes z.
        assert_eq!(closes_axes(&[[0.0, 0.0, 0.0, 1.0, 1.0, 0.1875]]), 0b100);
        // A bottom slab: closes y only.
        assert_eq!(closes_axes(&[[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]), 0b010);
        // A fence post.
        assert_eq!(closes_axes(&[[0.375, 0.0, 0.375, 0.625, 1.0, 0.625]]), 0);
        // No collision: closes nothing.
        let info = StateInfo {
            block: "minecraft:oak_door",
            break_sound: "",
            step_sound: "",
            fluid: FluidKind::None,
            collides: false,
            sturdy: 0,
            closes: 0b100,
        };
        assert_eq!(material(&info, &Settings::default()).closes, 0);
    }

    #[test]
    fn the_air_paths_reach_as_far_as_the_sound_range() {
        // A doorway 20 blocks out along x: beyond the vanilla field (16),
        // inside the doubled one.
        let mut b = Blocks::room([18, 60, -4], [26, 68, 4], STONE);
        b.fill([-4, 60, -12], [30, 60, 8], STONE);
        b.0.remove(&[22, 61, -4]);
        b.0.remove(&[22, 62, -4]);
        let r = request([22.5, 61.5, 0.5], [0.5, 61.5, -8.5]);
        let far = Settings {
            sound_range: 2.0,
            ..on()
        };
        let e = run(&b, &r, &far);
        let to = DVec3::from_array(e.redirect.expect("redirected through the door"));
        let listener = DVec3::from_array(r.listener);
        let door = (DVec3::new(22.5, 61.5, -4.0) - listener).normalize();
        assert!((to - listener).normalize().dot(door) > 0.99);
        assert_eq!(Field::build(&b, [0, 61, -9], 2.0).size, [65, 49, 65]);
        set_shared_range(1.5);
        assert_eq!(shared_range(), 1.5);
        set_shared_range(9.0);
        assert_eq!(shared_range(), 3.0);
        set_shared_range(1.0);
    }

    #[test]
    fn the_cache_answers_repeated_sounds_the_same() {
        let b = room_with_door(None);
        let r = request([8.5, 61.5, 0.5], [0.5, 61.5, -8.5]);
        let s = on();
        let rays = ray_directions(32);
        let mut cache = Cache::default();
        let first = evaluate(&b, &r, 16.0, &s, &rays, &mut cache);
        let traced = cache.misses;
        let again = evaluate(&b, &r, 16.0, &s, &rays, &mut cache);
        assert_eq!(first, again);
        assert_eq!(cache.misses, traced, "nothing traced twice");
        assert!(cache.hits > 0);
        assert_eq!(first, run(&b, &r, &s), "same answer as a cold cache");
        // Another listener cell starts over.
        let moved = Request {
            listener: [0.5, 61.5, -9.5],
            ..r
        };
        evaluate(&b, &moved, 16.0, &s, &rays, &mut cache);
        assert!(cache.misses > traced);
    }

    #[test]
    fn water_filters_and_skipped_sounds_stay_dry() {
        let mut b = Blocks::default();
        b.fill([-8, 50, -8], [8, 70, 8], WATER);
        let mut r = request([0.5, 60.5, 0.5], [2.5, 60.5, 0.5]);
        r.listener_underwater = true;
        let wet = run(&b, &r, &on());
        let dry = run(
            &b,
            &r,
            &Settings {
                underwater: false,
                ..on()
            },
        );
        // The filter applies last: shared airspace does not lift it back.
        assert!(wet.direct_cutoff <= 1.0 - 0.9 + 1e-6, "{wet:?}");
        assert!(
            wet.direct_cutoff < dry.direct_cutoff * 0.5,
            "{wet:?} {dry:?}"
        );
        for event in [
            "minecraft:weather.rain",
            "minecraft:ambient.cave",
            "minecraft:item.goat_horn.sound.3",
        ] {
            r.event = event.into();
            assert_eq!(run(&b, &r, &on()), Environment::DRY, "{event}");
        }
        r.event = "minecraft:music_disc.cat".into();
        r.source = RECORDS;
        assert_eq!(run(&b, &r, &on()), Environment::DRY);
    }
}
