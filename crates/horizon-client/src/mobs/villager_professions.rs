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
//! `<namespace>:gameplay/hero_of_the_village/<name>_gift`. Local indices stay
//! with an id across reloads; the network id is the id's position in the
//! current id-sorted definitions, so a host and guest with the same files
//! agree even when they loaded them in different orders.

use crate::mobs::render;
use horizon_world::loot::PackOverlay;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::{
        Mutex, OnceLock, RwLock,
        atomic::{AtomicU32, Ordering},
    },
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
    /// By index minus `FIRST`. A slot for a profession a later reload removes
    /// stays so live villagers keep their index and look; only the lookup maps
    /// are rebuilt.
    professions: Vec<Option<Profession>>,
    by_poi: HashMap<&'static str, u8>,
    by_id: HashMap<&'static str, u8>,
    by_name: HashMap<&'static str, u8>,
    /// The current definition order's wire index, by id. It is rebuilt from
    /// the current files on every load, so a host and guest with the same
    /// definitions agree even when their local index histories differ.
    wire: HashMap<&'static str, u16>,
    wire_by_position: Vec<&'static str>,
}

/// Bumped whenever the table is installed, so caches that derive from it
/// (the villager POI table) rebuild on `/reload`.
static VERSION: AtomicU32 = AtomicU32::new(0);

/// Indexes handed out across reloads, so adding a profession cannot renumber
/// the ones live villagers already carry.
struct Assignments {
    by_id: HashMap<String, u8>,
    next: u16,
}
impl Default for Assignments {
    fn default() -> Self {
        Self {
            by_id: HashMap::new(),
            next: u16::from(FIRST),
        }
    }
}
impl Assignments {
    fn assign(&mut self, id: &str) -> Option<u8> {
        if let Some(index) = self.by_id.get(id) {
            return Some(*index);
        }
        let index = u8::try_from(self.next).ok()?;
        self.next = self.next.checked_add(1)?;
        self.by_id.insert(id.to_owned(), index);
        Some(index)
    }
}

static ASSIGNMENTS: OnceLock<Mutex<Assignments>> = OnceLock::new();

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
    VERSION.fetch_add(1, Ordering::Relaxed);
}

/// The current table's generation. A consumer caching something derived from
/// it compares this and rebuilds when it changes.
pub fn version() -> u32 {
    VERSION.load(Ordering::Relaxed)
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
    let previous = read();
    let mut assignments = ASSIGNMENTS
        .get_or_init(|| Mutex::new(Assignments::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let registry = build(files, Some(previous), &mut assignments);
    drop(assignments);
    install(registry);
}

/// The table from its JSON files, without installing it. `previous` keeps
/// retired slots; `assignments` keeps every id on the index it first had.
fn build(
    files: BTreeMap<String, Value>,
    previous: Option<&Registry>,
    assignments: &mut Assignments,
) -> Registry {
    let mut registry = Registry::default();
    if let Some(previous) = previous {
        registry.professions = previous.professions.clone();
    }
    for (id, value) in files {
        let Some((namespace, name)) = id.split_once(':') else {
            continue;
        };
        let (namespace, name) = (namespace.to_owned(), name.to_owned());
        let Some(poi) = value["poi"].as_str() else {
            log::warn!("Horizon: villager profession {id} has no poi");
            continue;
        };
        let Some(index) = assignments.assign(&id) else {
            log::warn!("Horizon: villager profession {id} has no index left");
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
        let slot = usize::from(index - FIRST);
        if registry.professions.len() <= slot {
            registry.professions.resize_with(slot + 1, || None);
        }
        let wire = registry.wire_by_position.len() as u16;
        registry.wire.insert(id, wire);
        registry.wire_by_position.push(id);
        registry.by_poi.insert(poi, index);
        registry.by_id.insert(id, index);
        registry.by_name.insert(name, index);
        registry.professions[slot] = Some(Profession {
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
    read()
        .professions
        .get(index.checked_sub(FIRST)? as usize)?
        .as_ref()
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
    translation_key_of(&id(index))
}

fn translation_key_of(id: &str) -> String {
    match id.split_once(':') {
        // VillagerProfession builds `entity.<namespace>.villager.<path>`.
        Some((namespace, path)) => format!("entity.{namespace}.villager.{path}"),
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
    if let Some(poi) = job_site(&state.name).and_then(custom) {
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

fn network_id_of(registry: &Registry, index: u8) -> Option<i32> {
    let profession = registry
        .professions
        .get(index.checked_sub(FIRST)? as usize)?
        .as_ref()?;
    let wire = *registry.wire.get(profession.id)?;
    Some(NETWORK_BASE + i32::from(wire))
}

fn from_network_id_of(registry: &Registry, id: i32) -> Option<u8> {
    let wire = usize::try_from(id.checked_sub(NETWORK_BASE)?).ok()?;
    let profession = *registry.wire_by_position.get(wire)?;
    registry.by_id.get(profession).copied()
}

/// The id a client is sent for a custom profession (`None` for vanilla).
pub fn network_id(index: u8) -> Option<i32> {
    network_id_of(read(), index)
}

/// The custom profession index for a network id from `network_id`.
pub fn from_network_id(id: i32) -> Option<u8> {
    from_network_id_of(read(), id)
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
        let registry = build(files, None, &mut Assignments::default());
        let p = registry.professions[0].as_ref().unwrap();
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
    fn vanilla_pois_still_classify_without_a_custom_match() {
        let state = |name: &str| horizon_world::chunk::BlockState {
            name: name.into(),
            properties: Default::default(),
        };
        assert_eq!(
            poi_type(&state("minecraft:bell")),
            Some(("minecraft:meeting", 32))
        );
        assert_eq!(
            poi_type(&state("minecraft:blast_furnace")),
            Some(("minecraft:armorer", 1))
        );
        assert_eq!(poi_type(&state("minecraft:stone")), None);
    }

    #[test]
    fn reloads_keep_an_id_on_its_index() {
        let profession = |poi: &str| serde_json::json!({"poi": poi, "look": "farmer"});
        let mut assignments = Assignments::default();
        let first = build(
            BTreeMap::from([("example:zookeeper".to_owned(), profession("example:a"))]),
            None,
            &mut assignments,
        );
        // A new id sorts before zookeeper, but must not take its index.
        let second = build(
            BTreeMap::from([
                ("example:beekeeper".to_owned(), profession("example:b")),
                ("example:zookeeper".to_owned(), profession("example:a")),
            ]),
            Some(&first),
            &mut assignments,
        );
        assert_eq!(second.by_id.get("example:zookeeper"), Some(&FIRST));
        assert_eq!(second.by_id.get("example:beekeeper"), Some(&(FIRST + 1)));
        assert_eq!(
            second.professions[0].as_ref().map(|p| p.name),
            Some("zookeeper")
        );
        // Removing a profession keeps its slot for live villagers, but no
        // longer offers it as a new job.
        let third = build(
            BTreeMap::from([("example:beekeeper".to_owned(), profession("example:b"))]),
            Some(&second),
            &mut assignments,
        );
        assert_eq!(
            third.professions[0].as_ref().map(|p| p.id),
            Some("example:zookeeper")
        );
        assert!(!third.by_id.contains_key("example:zookeeper"));
    }

    #[test]
    fn custom_network_ids_round_trip() {
        let files = (0..10)
            .map(|i| {
                (
                    format!("example:p{i}"),
                    serde_json::json!({"poi": format!("example:b{i}"), "look": "farmer"}),
                )
            })
            .collect();
        let registry = build(files, None, &mut Assignments::default());
        for i in 0..10 {
            let index = FIRST + i;
            let id = network_id_of(&registry, index).unwrap();
            assert_eq!(from_network_id_of(&registry, id), Some(index));
        }
        assert_eq!(network_id_of(&registry, 14), None);
        assert_eq!(from_network_id_of(&registry, NETWORK_BASE + 10), None);
    }

    #[test]
    fn wire_ids_do_not_depend_on_local_index_history() {
        let profession = |poi: &str| serde_json::json!({"poi": poi, "look": "farmer"});
        let mut host = Assignments::default();
        let host_first = build(
            BTreeMap::from([("example:zookeeper".to_owned(), profession("example:a"))]),
            None,
            &mut host,
        );
        let host = build(
            BTreeMap::from([
                ("example:beekeeper".to_owned(), profession("example:b")),
                ("example:zookeeper".to_owned(), profession("example:a")),
            ]),
            Some(&host_first),
            &mut host,
        );
        // A fresh guest sorts beekeeper first, so zookeeper is local 16 here
        // and 15 on the host; the wire id must still be the same.
        let mut guest = Assignments::default();
        let guest = build(
            BTreeMap::from([
                ("example:beekeeper".to_owned(), profession("example:b")),
                ("example:zookeeper".to_owned(), profession("example:a")),
            ]),
            None,
            &mut guest,
        );
        let host_zookeeper = network_id_of(&host, FIRST).unwrap();
        let guest_zookeeper = network_id_of(&guest, FIRST + 1).unwrap();
        assert_eq!(host_zookeeper, guest_zookeeper);
        assert_eq!(from_network_id_of(&guest, host_zookeeper), Some(FIRST + 1));
    }

    #[test]
    fn translation_keys_use_the_profession_namespace() {
        assert_eq!(
            translation_key_of("mymod:beekeeper"),
            "entity.mymod.villager.beekeeper"
        );
        assert_eq!(
            translation_key_of("minecraft:farmer"),
            "entity.minecraft.villager.farmer"
        );
    }
}
