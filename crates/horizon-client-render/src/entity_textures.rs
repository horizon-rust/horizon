//! Entity textures a pack format vanilla doesn't read changes (OptiFine's random entities, ETF's
//! blinking and emissive textures): the variants of each texture and how an entity picks one,
//! the blink frames a texture shows now and then, and the overlays that glow over a texture.
//! The platform's pack formats fill them once per pack load ([`crate::pack_hooks`]); drawing
//! only looks them up in these tables.
//!
//! Textures have the names the entity atlas gives them (`entity/zombie/zombie`,
//! `<ns>:<path>`); files outside `textures/` are listed with the name they take
//! ([`RandomTextures::files`], [`Blinks::files`], [`Emissive::files`]) for the atlas to pack.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// A pack's entity texture changes: variants, blinks and emissive overlays.
#[derive(Debug, Default)]
pub struct EntityTextures {
    pub random: RandomTextures,
    pub blinks: Blinks,
    pub emissive: Emissive,
}

impl EntityTextures {
    /// The changes of the overlay at `root`, from the installed pack formats.
    pub fn load(root: &Path) -> Self {
        crate::pack_hooks::formats()
            .entity_textures(root)
            .unwrap_or_default()
    }
}

/// What a variant rule can ask about an entity. `None` is a fact the entity doesn't have (a
/// creeper's profession): a rule asking for it doesn't match.
#[derive(Clone, Debug, Default)]
pub struct Facts<'a> {
    /// Where the entity was first seen: its biome (`minecraft:plains`) and block height.
    pub biome: Option<&'a str>,
    pub y: Option<i32>,
    /// The block it stood on and the block it stood in when first seen; empty when unknown.
    pub blocks: &'a [Block<'a>],
    /// Its custom name, with `§` formatting codes where the name has them.
    pub name: Option<&'a str>,
    /// Villager profession (`minecraft:farmer`) and level.
    pub profession: Option<(&'a str, i32)>,
    /// Collar, wool, carpet or shell colour, as a dye name (`light_blue`).
    pub color: Option<&'a str>,
    pub baby: Option<bool>,
    /// Health and max health.
    pub health: Option<(f32, f32)>,
    /// When first seen: moon phase (0-7), time of day (0-23999) and weather.
    pub moon_phase: Option<i32>,
    pub day_time: Option<i32>,
    pub weather: Option<Weather>,
    /// Slime, magma cube or phantom size: 0 small, 1 medium, 3 big.
    pub size: Option<i32>,
}

/// A block and the values of its state properties.
#[derive(Clone, Copy, Debug)]
pub struct Block<'a> {
    pub id: &'a str,
    pub states: &'a [(&'a str, &'a str)],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weather {
    Clear,
    Rain,
    Thunder,
}

/// How a pack format's rule files choose a variant.
pub trait VariantRules: Send + Sync {
    /// The variant rule file `rules` ([`Entry::rules`]) gives an entity with these facts and
    /// seed, `None` when no rule matches.
    fn matching(&self, rules: usize, facts: &Facts, seed: i32) -> Option<u32>;
}

/// Every texture a pack varies, by its atlas name.
#[derive(Default)]
pub struct RandomTextures {
    entries: HashMap<String, Entry>,
    /// Every variant's file (from the resource root) and atlas name.
    files: Vec<(String, String)>,
    rules: Option<Box<dyn VariantRules>>,
    generation: u64,
}

impl std::fmt::Debug for RandomTextures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RandomTextures")
            .field("entries", &self.entries)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

/// One texture's variants.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    /// The rule file that chooses this texture's variant ([`RandomTextures::matching`]).
    pub rules: Option<usize>,
    /// The atlas name of each variant, by its number.
    pub variants: BTreeMap<u32, String>,
}

impl RandomTextures {
    /// `entries` by atlas name, their variants' `files` (file, atlas name), the rule files
    /// `rules` that choose, and `generation`: different for every load (never 0), so choices
    /// made from another load's rules are known to be stale.
    pub fn new(
        entries: HashMap<String, Entry>,
        files: Vec<(String, String)>,
        rules: Box<dyn VariantRules>,
        generation: u64,
    ) -> Self {
        Self {
            entries,
            files,
            rules: Some(rules),
            generation,
        }
    }

    /// This load's number; 0 for the empty set.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The variants of the texture with this atlas name.
    pub fn get(&self, texture: &str) -> Option<&Entry> {
        self.entries.get(texture)
    }

    /// [`VariantRules::matching`].
    pub fn matching(&self, rules: usize, facts: &Facts, seed: i32) -> Option<u32> {
        self.rules.as_ref()?.matching(rules, facts, seed)
    }

    /// Every variant's file (its path from the resource root) and atlas name, for the entity
    /// atlas to pack.
    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files.iter().map(|(f, n)| (f.as_str(), n.as_str()))
    }
}

impl Entry {
    /// The atlas name of variant `index`, or `None` for the texture itself.
    pub fn texture(&self, index: u32) -> Option<&str> {
        self.variants.get(&index).map(String::as_str)
    }

    /// The even choice of an entity with this seed when no rules apply: the texture and its
    /// variants 2, 3... up to the first missing one.
    pub fn even(&self, seed: i32) -> u32 {
        let mut count = 1;
        while self.variants.contains_key(&(count + 1)) {
            count += 1;
        }
        seed.unsigned_abs() % count + 1
    }
}

/// An entity's seed from its UUID (most then least significant 64 bits): Java's
/// `UUID.hashCode`, so it is fixed for the entity's whole life, on every client.
pub fn seed(most: i64, least: i64) -> i32 {
    let x = most ^ least;
    ((x >> 32) ^ x) as i32
}

/// The pack's blinking textures, by the atlas name of the texture they replace.
#[derive(Debug, Default)]
pub struct Blinks {
    blinks: HashMap<String, Blink>,
    /// The blink frames outside `textures/`: file and atlas name.
    files: Vec<(String, String)>,
}

/// A texture's blink.
#[derive(Clone, Debug, PartialEq)]
pub struct Blink {
    /// The atlas names of the shut and half-shut frames.
    pub shut: String,
    pub half: Option<String>,
    /// Ticks a blink lasts (at least 1) and ticks from one blink to the next (more).
    pub length: u32,
    pub frequency: u32,
}

impl Blinks {
    /// `blinks` by the atlas name of the texture they replace, and the frames outside
    /// `textures/` (file, atlas name).
    pub fn new(blinks: HashMap<String, Blink>, files: Vec<(String, String)>) -> Self {
        Self { blinks, files }
    }

    pub fn is_empty(&self) -> bool {
        self.blinks.is_empty()
    }

    pub fn get(&self, texture: &str) -> Option<&Blink> {
        self.blinks.get(texture)
    }

    /// The frames outside `textures/`: file and atlas name, for the entity atlas to pack (it
    /// walks `textures/` itself).
    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files.iter().map(|(f, n)| (f.as_str(), n.as_str()))
    }
}

impl Blink {
    /// The frame an entity shows `tick` ticks into its life: `None` with its eyes open. `phase`
    /// spreads entities' blinks apart. With a half-shut frame, the first and last ticks of a
    /// blink of 3 or more show it.
    pub fn frame(&self, tick: u32, phase: u32) -> Option<&str> {
        let t = (u64::from(tick) + u64::from(phase)) % u64::from(self.frequency);
        let t = t as u32;
        if t >= self.length {
            return None;
        }
        match &self.half {
            Some(half) if self.length >= 3 && (t == 0 || t == self.length - 1) => Some(half),
            _ => Some(&self.shut),
        }
    }
}

/// The pack's emissive overlays, by the atlas name of the texture they glow over.
#[derive(Debug, Default)]
pub struct Emissive {
    /// The name ending of the pack's emissive textures (`_e`).
    pub suffix: String,
    overlays: HashMap<String, String>,
    /// Overlays outside `textures/`: file and atlas name.
    files: Vec<(String, String)>,
}

impl Emissive {
    /// `overlays` (texture → overlay, atlas names) with name ending `suffix`, and those outside
    /// `textures/` (file, atlas name).
    pub fn new(
        suffix: String,
        overlays: HashMap<String, String>,
        files: Vec<(String, String)>,
    ) -> Self {
        Self {
            suffix,
            overlays,
            files,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.overlays.is_empty()
    }

    /// The atlas name of the overlay that glows over `texture`.
    pub fn overlay(&self, texture: &str) -> Option<&str> {
        self.overlays.get(texture).map(String::as_str)
    }

    /// The overlays outside `textures/`: file (from the resource root) and atlas name, for the
    /// entity atlas to pack (it walks `textures/` itself).
    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files.iter().map(|(f, n)| (f.as_str(), n.as_str()))
    }
}
