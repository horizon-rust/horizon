//! OptiFine's random entity textures and ETF's blinking and emissive textures
//! (`horizon_optifine::{random_textures, blink, emissive}`) as the engine's
//! [`crate::entity_textures`] tables.

use crate::entity_textures::{
    self as engine, Blink, Blinks, Emissive, EntityTextures, Entry, RandomTextures,
};
use horizon_optifine::random::{self, Rules};
use std::path::Path;

/// The pack's variants, blinks and emissive overlays under the overlay resource root.
pub fn load(root: &Path) -> EntityTextures {
    let random = horizon_optifine::random_textures::RandomTextures::scan(root);
    let blinks = horizon_optifine::blink::Blinks::scan(root, &random);
    let mut emissive = horizon_optifine::emissive::Emissive::scan(root, &random);
    emissive.pair_beside(root, blinks.files());
    let files = random
        .files()
        .map(|(f, n)| (f.to_owned(), n.to_owned()))
        .collect();
    let generation = random.generation();
    let (entries, rules) = random.into_parts();
    let entries = entries
        .into_iter()
        .map(|(texture, entry)| {
            let variants = entry
                .variant_names()
                .map(|(n, name)| (n, name.to_owned()))
                .collect();
            let rules = entry.rules;
            (texture, Entry { rules, variants })
        })
        .collect();
    let (blinks, blink_files) = blinks.into_parts();
    let blinks = blinks
        .into_iter()
        .map(|(texture, b)| {
            let blink = Blink {
                shut: b.shut,
                half: b.half,
                length: b.length,
                frequency: b.frequency,
            };
            (texture, blink)
        })
        .collect();
    let (suffix, overlays, overlay_files) = emissive.into_parts();
    EntityTextures {
        random: RandomTextures::new(entries, files, Box::new(Variants(rules)), generation),
        blinks: Blinks::new(blinks, blink_files),
        emissive: Emissive::new(suffix, overlays, overlay_files),
    }
}

/// The rule files of a scan.
struct Variants(Vec<Rules>);

impl engine::VariantRules for Variants {
    fn matching(&self, rules: usize, facts: &engine::Facts, seed: i32) -> Option<u32> {
        let blocks: Vec<random::Block> = facts
            .blocks
            .iter()
            .map(|b| random::Block {
                id: b.id,
                states: b.states,
            })
            .collect();
        let facts = random::Facts {
            biome: facts.biome,
            y: facts.y,
            blocks: &blocks,
            name: facts.name,
            profession: facts.profession,
            color: facts.color,
            baby: facts.baby,
            health: facts.health,
            moon_phase: facts.moon_phase,
            day_time: facts.day_time,
            weather: facts.weather.map(|w| match w {
                engine::Weather::Clear => random::Weather::Clear,
                engine::Weather::Rain => random::Weather::Rain,
                engine::Weather::Thunder => random::Weather::Thunder,
            }),
            size: facts.size,
        };
        self.0.get(rules)?.matching(&facts, seed)
    }
}
