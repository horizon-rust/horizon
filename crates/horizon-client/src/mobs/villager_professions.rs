//! Data-driven villager professions, beside vanilla's fixed fifteen.
//!
//! A pack or content mod writes one `data/<namespace>/villager_profession/
//! <name>.json` per profession:
//!
//! ```json
//! { "poi": "morevillagers:oceanography_table",
//!   "work_sound": "entity.villager.work_cartographer",
//!   "look": "cartographer",
//!   "gift": "morevillagers:gameplay/hero_of_the_village/oceanographer_gift" }
//! ```
//!
//! `poi` is the job-site block (required); `work_sound` is the sound event
//! without `minecraft:`; `look` is the vanilla profession whose texture and
//! hat the villager wears; `gift` defaults to
//! `<namespace>:gameplay/hero_of_the_village/<name>_gift`. Entries are sorted
//! by id, so both sides of a connection assign the same index.

use crate::mobs::render;
use horizon_world::loot::PackOverlay;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::{OnceLock, RwLock},
};

/// Vanilla's profession table has this many entries; custom ones start here.
pub const FIRST: u8 = 15;
/// Network ids of custom professions. Vanilla's built-in ids stay below this;
/// a vanilla client never receives one because it cannot load the content.
pub const NETWORK_BASE: i32 = 1_000_000;

#[derive(Clone, Debug)]
pub struct Profession {
    /// The namespaced id (`morevillagers:oceanographer`).
    pub id: &'static str,
    /// The path part (`oceanographer`), also the texture and trade-set name.
    pub name: &'static str,
    /// The job-site block, moved into the POI map.
    pub poi: &'static str,
    /// The vanilla profession whose texture and hat this one wears.
    pub look: &'static str,
    /// A sound event without `minecraft:`.
    pub work_sound: Option<&'static str>,
    /// The hero-of-the-village gift loot table.
    pub gift: &'static str,
}

#[derive(Default)]
struct Registry {
    /// In id order; index = `FIRST + position`.
    professions: Vec<Profession>,
    by_poi: HashMap<&'static str, u8>,
    by_id: HashMap<&'static str, u8>,
    by_name: HashMap<&'static str, u8>,
}

/// The current table. A reload leaks the old one (a few short strings) so
/// callers can hold `&'static` names while rendering.
static REGISTRY: OnceLock<RwLock<&'static Registry>> = OnceLock::new();

fn read() -> &'static Registry {
    let slot = REGISTRY.get_or_init(|| RwLock::new(Box::leak(Box::new(Registry::default()))));
    *slot.read().unwrap_or_else(|e| e.into_inner())
}

fn install(registry: Registry) {
    let registry: &'static Registry = Box::leak(Box::new(registry));
    let slot = REGISTRY.get_or_init(|| RwLock::new(Box::leak(Box::new(Registry::default()))));
    *slot.write().unwrap_or_else(|e| e.into_inner()) = registry;
}

/// Load the resource root's professions and the world's data packs over them.
/// Called where a world's tables load and on `/reload`.
pub fn load(root: &Path, overlay: &PackOverlay) {
    let mut files: BTreeMap<String, Value> = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(root.join("data")) {
        for entry in entries.flatten() {
            let namespace = entry.file_name().to_string_lossy().into_owned();
            let dir = entry.path().join("villager_profession");
            let Ok(folder) = horizon_world::json_folder::read(&dir) else {
                continue;
            };
            for file in &folder.files {
                let Ok(value) = serde_json::from_slice(file.bytes().unwrap_or_default()) else {
                    continue;
                };
                files.insert(format!("{namespace}:{}", file.name), value);
            }
        }
    }
    for (id, sources) in overlay.kind("villager_profession") {
        if let Ok(value) = serde_json::from_slice(PackOverlay::top(sources)) {
            files.insert(id.to_owned(), value);
        }
    }
    install(build(files));
}

/// The table from its JSON files, without installing it.
fn build(files: BTreeMap<String, Value>) -> Registry {
    let mut registry = Registry::default();
    for (id, value) in files {
        let Some((namespace, name)) = id.split_once(':') else {
            continue;
        };
        let (namespace, name) = (namespace.to_owned(), name.to_owned());
        let Some(poi) = value["poi"].as_str() else {
            log::warn!("Horizon: villager profession {id} has no poi");
            continue;
        };
        let id: &'static str = Box::leak(id.into_boxed_str());
        let name: &'static str = Box::leak(name.into_boxed_str());
        let poi: &'static str = Box::leak(poi.to_owned().into_boxed_str());
        let look = value["look"]
            .as_str()
            .unwrap_or("none")
            .trim_start_matches("minecraft:");
        let look: &'static str = Box::leak(look.to_owned().into_boxed_str());
        let work_sound = value["work_sound"]
            .as_str()
            .map(|s| s.trim_start_matches("minecraft:"))
            .map(|s| &*Box::leak(s.to_owned().into_boxed_str()));
        let gift = value["gift"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{namespace}:gameplay/hero_of_the_village/{name}_gift"));
        let gift: &'static str = Box::leak(gift.into_boxed_str());
        if registry.professions.len() + usize::from(FIRST) > usize::from(u8::MAX) {
            log::warn!("Horizon: villager profession {id} has no index left");
            break;
        }
        let index = FIRST + registry.professions.len() as u8;
        registry.by_poi.insert(poi, index);
        registry.by_id.insert(id, index);
        registry.by_name.insert(name, index);
        registry.professions.push(Profession {
            id,
            name,
            poi,
            look,
            work_sound,
            gift,
        });
    }
    registry
}

/// The custom profession at an index, if any.
fn custom(index: u8) -> Option<&'static Profession> {
    read().professions.get(index.checked_sub(FIRST)? as usize)
}
/// The path name at any profession index (`oceanographer`, `farmer`, ...).
pub fn name(index: u8) -> &'static str {
    custom(index)
        .map(|p| p.name)
        .unwrap_or_else(|| render::PROFESSIONS[index as usize % render::PROFESSIONS.len()])
}

/// The vanilla profession whose texture and hat an index wears.
pub fn look(index: u8) -> &'static str {
    custom(index)
        .map(|p| p.look)
        .unwrap_or_else(|| render::PROFESSIONS[index as usize % render::PROFESSIONS.len()])
}

/// The namespaced id at any profession index.
pub fn id(index: u8) -> String {
    match custom(index) {
        Some(p) => p.id.to_owned(),
        None => format!("minecraft:{}", name(index)),
    }
}

/// The translation key a villager's command reply uses.
pub fn translation_key(index: u8) -> String {
    let id = id(index);
    match id.split_once(':') {
        Some(("minecraft", path)) => format!("entity.minecraft.villager.{path}"),
        Some((namespace, path)) => format!("entity.minecraft.villager.{namespace}.{path}"),
        None => format!("entity.minecraft.villager.{id}"),
    }
}

/// A profession's index from an id or a bare name.
pub fn resolve(profession: &str) -> Option<u8> {
    let (namespace, path) = profession
        .split_once(':')
        .unwrap_or(("minecraft", profession));
    if namespace == "minecraft" {
        if let Some(i) = render::PROFESSIONS.iter().position(|p| *p == path) {
            return Some(i as u8);
        }
        // A bare name may be a data pack's profession.
        return read().by_name.get(path).copied();
    }
    read().by_id.get(profession).copied()
}

/// The job-site block's profession index.
pub fn job_site(block: &str) -> Option<u8> {
    read().by_poi.get(block).copied()
}

/// `PoiTypes.register`: the POI type of a block state and its max tickets.
pub fn poi_type(state: &horizon_world::chunk::BlockState) -> Option<(&'static str, i32)> {
    if let Some(poi) = custom(job_site(&state.name)?) {
        return Some((poi.id, 1));
    }
    let name = state.name.strip_prefix("minecraft:")?;
    Some(match name {
        "blast_furnace" => ("minecraft:armorer", 1),
        "smoker" => ("minecraft:butcher", 1),
        "cartography_table" => ("minecraft:cartographer", 1),
        "brewing_stand" => ("minecraft:cleric", 1),
        "composter" => ("minecraft:farmer", 1),
        "barrel" => ("minecraft:fisherman", 1),
        "fletching_table" => ("minecraft:fletcher", 1),
        "cauldron" | "lava_cauldron" | "water_cauldron" | "powder_snow_cauldron" => {
            ("minecraft:leatherworker", 1)
        }
        "lectern" => ("minecraft:librarian", 1),
        "stonecutter" => ("minecraft:mason", 1),
        "loom" => ("minecraft:shepherd", 1),
        "smithing_table" => ("minecraft:toolsmith", 1),
        "grindstone" => ("minecraft:weaponsmith", 1),
        "bell" => ("minecraft:meeting", 32),
        "beehive" => ("minecraft:beehive", 0),
        "bee_nest" => ("minecraft:bee_nest", 0),
        "nether_portal" => ("minecraft:nether_portal", 0),
        "lodestone" => ("minecraft:lodestone", 0),
        "test_instance_block" => ("minecraft:test_instance", 0),
        // PoiTypes.BEDS is the sixteen colours: no straw bed.
        bed if bed.ends_with("_bed") && bed != "straw_bed" => {
            if state.properties.get("part").map(String::as_str) != Some("head") {
                return None;
            }
            ("minecraft:home", 1)
        }
        rod if rod.ends_with("lightning_rod") => ("minecraft:lightning_rod", 0),
        _ => return None,
    })
}

/// `VillagerProfession.workSound` of a custom profession.
pub fn work_sound(index: u8) -> Option<&'static str> {
    custom(index).and_then(|p| p.work_sound)
}

/// `GiveGiftToHero.getLootTableToThrow` of a custom profession.
pub fn gift_table(index: u8) -> Option<&'static str> {
    custom(index).map(|p| p.gift)
}

fn network_id_of(index: u8, count: usize) -> Option<i32> {
    let offset = index.checked_sub(FIRST)?;
    (usize::from(offset) < count).then(|| NETWORK_BASE + i32::from(offset))
}

fn from_network_id_of(id: i32, count: usize) -> Option<u8> {
    let offset = usize::try_from(id.checked_sub(NETWORK_BASE)?).ok()?;
    (offset < count)
        .then(|| FIRST.checked_add(u8::try_from(offset).ok()?))
        .flatten()
}

/// The id a client is sent for a custom profession (`None` for vanilla).
pub fn network_id(index: u8) -> Option<i32> {
    network_id_of(index, read().professions.len())
}

/// The custom profession index for a network id from `network_id`.
pub fn from_network_id(id: i32) -> Option<u8> {
    from_network_id_of(id, read().professions.len())
}

/// A custom profession's network id, or the built-in `fallback` for a
/// vanilla profession.
pub fn network_id_or(index: u8, fallback: impl FnOnce(&str) -> Option<i32>) -> Option<i32> {
    network_id(index).or_else(|| fallback(name(index)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_adds_professions_after_vanilla() {
        let files = BTreeMap::from([(
            "example:test".to_owned(),
            serde_json::json!({"poi": "example:bench", "look": "farmer",
                "work_sound": "entity.villager.work_farmer"}),
        )]);
        let registry = build(files);
        let p = &registry.professions[0];
        assert_eq!(p.id, "example:test");
        assert_eq!(p.name, "test");
        assert_eq!(p.look, "farmer");
        assert_eq!(p.work_sound, Some("entity.villager.work_farmer"));
        assert_eq!(p.gift, "example:gameplay/hero_of_the_village/test_gift");
        assert_eq!(registry.by_poi.get("example:bench"), Some(&FIRST));
        assert_eq!(registry.by_id.get("example:test"), Some(&FIRST));
        assert_eq!(registry.by_name.get("test"), Some(&FIRST));
    }

    #[test]
    fn custom_network_ids_round_trip() {
        for i in 0..10 {
            let index = FIRST + i;
            let id = network_id_of(index, 10).unwrap();
            assert_eq!(from_network_id_of(id, 10), Some(index));
        }
        assert_eq!(network_id_of(14, 10), None);
        assert_eq!(from_network_id_of(NETWORK_BASE + 10, 10), None);
    }
}
