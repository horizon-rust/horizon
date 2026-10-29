//! The Horizon side of a mod port's harness (tools/mod_port_harness.py,
//! docs/modpacks.md): builds the content root for carried-over mods and
//! writes what Horizon makes of every block state, drop, recipe and tag the
//! NeoForge oracle exported.
//!
//! cargo run -p horizon-client --example mod_port_probe -- PREPARED WORK OUT.json [MOD...]
//!
//! WORK is a `horizon-port-pack` work folder: `oracle/facts.json` and
//! `mods/<id>/carry/`. The root is built in `WORK/horizon-root`.
use horizon_client::{
    Result,
    assets::Assets,
    blocks::BlockFacts,
    crafting::{Recipe, Recipes},
    items::{ItemStack, Items},
    mod_content::{self, ContentMod},
    stations::StationRecipes,
};
use horizon_core::{geometry::Axis, random::LegacyRandom};
use horizon_world::chunk::BlockState;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

fn boxes(list: impl IntoIterator<Item = [f64; 6]>) -> Value {
    let mut v: Vec<[f64; 6]> = list.into_iter().collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    json!(v)
}

fn sound(s: &horizon_client::blocks::Sounds) -> Value {
    json!({"volume": s.volume, "pitch": s.pitch, "break": &*s.break_sound, "step": &*s.step,
        "place": &*s.place, "hit": &*s.hit, "fall": &*s.fall})
}

fn tool(name: &str) -> Option<ItemStack> {
    match name {
        "hand" => None,
        "silk_touch_pickaxe" => {
            let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
            pick.enchant("minecraft:silk_touch", 1);
            Some(pick)
        }
        other => Some(ItemStack::new(&format!("minecraft:{other}"), 1)),
    }
}

fn selected_mods(work: &Path, wanted: &[String]) -> Result<Vec<(String, horizon_mods::Package)>> {
    let mut paths = BTreeMap::new();
    for entry in fs::read_dir(work.join("mods"))? {
        let entry = entry?;
        let dir = entry.path().join("carry");
        if dir.join("horizon-mod.toml").is_file() {
            paths.insert(entry.file_name().to_string_lossy().into_owned(), dir);
        }
    }
    if work.join("inventory.json").exists() {
        let records: Vec<Value> = serde_json::from_slice(&fs::read(work.join("inventory.json"))?)?;
        let mut owned = BTreeSet::new();
        for record in &records {
            let key = record["key"].as_str().ok_or("inventory: missing key")?;
            let kind = record["path"]
                .as_str()
                .ok_or("inventory: missing path")?
                .split('/')
                .next()
                .unwrap_or_default();
            let folder = if matches!(kind, "datapacks" | "resourcepacks" | "shaderpacks") {
                format!("{kind}@{key}")
            } else {
                key.to_owned()
            };
            let unique = records
                .iter()
                .filter(|r| r["key"].as_str() == Some(key))
                .count()
                == 1;
            let folder = if paths.contains_key(&folder) {
                folder
            } else if unique && paths.contains_key(key) {
                key.to_owned()
            } else {
                continue;
            };
            let source = paths[&folder].parent().unwrap().join("source.json");
            if source.exists() {
                let source: String = serde_json::from_slice(&fs::read(source)?)?;
                if source != record["path"].as_str().ok_or("inventory: missing path")? {
                    continue;
                }
            } else if !unique && folder == key {
                continue;
            }
            owned.insert(folder);
        }
        paths.retain(|name, _| owned.contains(name));
    }
    let mods: Vec<_> = paths
        .into_iter()
        .map(|(name, path)| horizon_mods::Package::read(&path).map(|package| (name, package)))
        .collect::<std::result::Result<_, _>>()?;
    if wanted.is_empty() {
        return Ok(mods);
    }
    let mut selected = BTreeSet::new();
    for selector in wanted {
        let exact = mods.iter().position(|(folder, _)| folder == selector);
        let matches: Vec<_> = match exact {
            Some(i) => vec![i],
            None => mods
                .iter()
                .enumerate()
                .filter(|(_, (_, p))| p.manifest.id == *selector)
                .map(|(i, _)| i)
                .collect(),
        };
        if matches.len() != 1 {
            return Err(format!(
                "{selector}: expected one carried folder or manifest ID, found {}",
                matches.len()
            )
            .into());
        }
        selected.insert(matches[0]);
    }
    Ok(mods
        .into_iter()
        .enumerate()
        .filter(|(i, _)| selected.contains(i))
        .map(|(_, m)| m)
        .collect())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [base, work, out, wanted @ ..] = args.as_slice() else {
        return Err("usage: mod_port_probe PREPARED WORK OUT.json [MOD...]".into());
    };
    let (base, work) = (Path::new(base), Path::new(work));
    let oracle: Value = serde_json::from_slice(&fs::read(work.join("oracle/facts.json"))?)?;
    let mut mods = Vec::new();
    let mut owners = Vec::new();
    for (folder, package) in selected_mods(work, wanted)? {
        let dir = work.join("mods").join(&folder).join("carry");
        owners.push(json!({"folder": folder, "id": &package.manifest.id}));
        let content = horizon_mods::Content::read(&dir, &package.manifest)?;
        mods.push(ContentMod {
            id: package.manifest.id.clone(),
            content,
        });
    }
    mods.sort_by(|a, b| a.id.cmp(&b.id));
    let root = work.join("horizon-root");
    let began = std::time::Instant::now();
    let step = |what: &str| {
        let what = what.to_owned();
        move |e: Box<dyn std::error::Error>| format!("{what}: {e}")
    };
    mod_content::build(base, &mods, &root).map_err(step("content root"))?;
    let built = began.elapsed().as_secs_f64();
    let assets = Assets::load(&root).map_err(step("assets"))?;
    let registry = assets.registry;
    let facts = BlockFacts::load(&root, registry).map_err(step("block facts"))?;
    let items = Items::load(&root, registry, &facts).map_err(step("items"))?;
    let recipes = Recipes::load(&root, &items).map_err(step("recipes"))?;
    let stations = StationRecipes::load(&root, &items);
    let collisions =
        horizon_client::collisions::load(&root, registry).map_err(step("collisions"))?;
    let loaded = began.elapsed().as_secs_f64() - built;
    let ours = |id: &str| {
        id.split_once(':').is_some_and(|(ns, _)| {
            mods.iter()
                .any(|m| m.content.namespaces.iter().any(|n| n == ns))
        })
    };
    let tools: Vec<String> = serde_json::from_value(oracle["tools"].clone())?;

    let mut blocks = Map::new();
    for (name, block) in oracle["blocks"].as_object().ok_or("oracle: no blocks")? {
        if !ours(name) {
            continue;
        }
        let Some(default) = registry.default_state(name) else {
            blocks.insert(name.clone(), Value::Null);
            continue;
        };
        let mut states = Map::new();
        for (key, o) in block["states"].as_object().into_iter().flatten() {
            let properties: BTreeMap<String, String> = key
                .split(',')
                .filter(|p| !p.is_empty())
                .filter_map(|p| p.split_once('='))
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect();
            let descriptor = BlockState {
                name: name.clone(),
                properties,
            };
            let Some(id) = registry.id(&descriptor) else {
                states.insert(key.clone(), Value::Null);
                continue;
            };
            let f = facts.get(id);
            let collision = collisions
                .cached_shapes()
                .cached_shape(id)
                .ok()
                .flatten()
                .map(|s| {
                    boxes(s.boxes().iter().map(|b| {
                        [
                            b.min(Axis::X),
                            b.min(Axis::Y),
                            b.min(Axis::Z),
                            b.max(Axis::X),
                            b.max(Axis::Y),
                            b.max(Axis::Z),
                        ]
                    }))
                });
            let mut row = json!({
                "hardness": f.hardness,
                "explosion_resistance": f.explosion_resistance,
                "emission": f.emission,
                "requires_tool": f.requires_tool,
                "friction": f.friction,
                "map_color_id": f.map_color,
                "can_occlude": f.can_occlude,
                "sound": sound(&f.sounds),
                "outline": boxes(f.outline.iter().copied()),
                "collision": collision,
                "classes": f.classes.iter().map(|c| &**c).collect::<Vec<_>>(),
            });
            if o.get("drops").is_some() {
                let mut drops = Map::new();
                for t in &tools {
                    let mut random = LegacyRandom::new(0);
                    let held = tool(t);
                    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
                    for s in items.block_drops(registry, id, held.as_ref(), &mut random) {
                        *counts.entry(s.item.to_string()).or_default() += s.count;
                    }
                    drops.insert(t.clone(), json!(counts));
                }
                row["drops"] = Value::Object(drops);
            }
            states.insert(key.clone(), row);
        }
        let item = facts.get(default).item.to_string();
        blocks.insert(
            name.clone(),
            json!({"states": states, "item": item, "item_known": items.items.contains_key(&*item)}),
        );
    }

    // Vanilla's sounds, by block: the harness maps the oracle's version's
    // sound events to this one's through them.
    let mut vanilla_sounds = Map::new();
    for name in oracle["blocks"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k)
    {
        if let Some(id) = name
            .starts_with("minecraft:")
            .then(|| registry.default_state(name))
            .flatten()
        {
            vanilla_sounds.insert(name.clone(), sound(&facts.get(id).sounds));
        }
    }

    let mut item_rows = Map::new();
    for name in oracle["items"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k)
    {
        item_rows.insert(
            name.clone(),
            match items.items.get(name.as_str()) {
                Some(i) => json!({"max_stack_size": i.max_stack, "block": i.block.map(|b| b.0)}),
                None => Value::Null,
            },
        );
    }

    let sorted = |list: &[String]| {
        let mut v = list.to_vec();
        v.sort();
        v.dedup();
        v
    };
    let mut recipe_rows = Map::new();
    for (name, recipe) in &recipes.crafting {
        if !ours(name) {
            continue;
        }
        let row = match recipe {
            Recipe::Shaped { cells, result, .. } => json!({
                "result": result.item.as_deref(), "count": result.count,
                "ingredients": cells.iter().flatten().map(|i| sorted(&i.1)).collect::<Vec<_>>()}),
            Recipe::Shapeless {
                ingredients,
                result,
            } => json!({
                "result": result.item.as_deref(), "count": result.count,
                "ingredients": ingredients.iter().map(|i| sorted(&i.1)).collect::<Vec<_>>()}),
            Recipe::Special(_) => json!({"special": true}),
        };
        recipe_rows.insert(name.clone(), row);
    }
    for c in recipes.cooking.iter().filter(|c| ours(&c.id)) {
        recipe_rows.insert(
            c.id.clone(),
            json!({"result": &*c.result.item, "count": c.result.count,
                "ingredients": [sorted(&c.ingredient.1)]}),
        );
    }
    for s in stations.stonecutting.iter().filter(|s| ours(&s.id)) {
        recipe_rows.insert(
            s.id.clone(),
            json!({"result": &*s.result.item, "count": s.result.count,
                "ingredients": [sorted(&s.input.1)]}),
        );
    }

    let mut block_tags = Map::new();
    for (tag, members) in &items.block_tags {
        let mut ids: Vec<&String> = members.iter().filter(|m| ours(m)).collect();
        if !ids.is_empty() {
            ids.sort();
            block_tags.insert(tag.clone(), json!(ids));
        }
    }

    let report = json!({
        "mods": owners,
        "states": registry.len(),
        "build_seconds": built,
        "load_seconds": loaded,
        "blocks": blocks,
        "items": item_rows,
        "recipes": recipe_rows,
        "block_tags": block_tags,
        "vanilla_sounds": vanilla_sounds,
    });
    fs::write(out, serde_json::to_vec(&report)?)?;
    println!(
        "{out}: {} blocks, {} items, {} recipes; root built in {built:.2} s, loaded in {loaded:.2} s",
        blocks.len(),
        item_rows.len(),
        recipe_rows.len()
    );
    Ok(())
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn folder_and_public_ids_resolve_current_owners() {
        struct TestDir(std::path::PathBuf);
        impl Drop for TestDir {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let root = TestDir(
            std::env::temp_dir().join(format!("horizon-mod-port-selection-{}", std::process::id())),
        );
        fs::create_dir(&root.0).unwrap();
        let id = "a_review_pack";
        let data_source = "datapacks/a_review_pack.zip";
        let mod_source = "mods/opaque.jar";
        for folder in [id, "datapacks@a_review_pack"] {
            let output = root.0.join("mods").join(folder);
            fs::create_dir_all(output.join("carry")).unwrap();
            fs::write(output.join("carry/horizon-mod.toml"), format!("id = \"{id}\"\nname = \"Original fixture\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n")).unwrap();
            fs::write(
                output.join("source.json"),
                serde_json::to_vec(data_source).unwrap(),
            )
            .unwrap();
        }
        let record = |path: &str| json!({"key": id, "path": path, "name": "Original fixture", "version": "1", "class": "content", "tier": "data", "confidence": "kind", "source": "file", "license": "MIT", "client": "", "server": "", "depends": [], "layer": 0});
        let inventory = root.0.join("inventory.json");
        fs::write(
            &inventory,
            serde_json::to_vec(&[record(data_source)]).unwrap(),
        )
        .unwrap();
        for wanted in [
            vec![],
            vec![id.to_owned()],
            vec!["datapacks@a_review_pack".to_owned()],
        ] {
            let selected = selected_mods(&root.0, &wanted).unwrap();
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].0, "datapacks@a_review_pack");
            assert_eq!(selected[0].1.manifest.id, id);
        }
        assert!(selected_mods(&root.0, &["missing".into()]).is_err());
        fs::write(
            &inventory,
            serde_json::to_vec(&[record(data_source), record(mod_source)]).unwrap(),
        )
        .unwrap();
        assert_eq!(selected_mods(&root.0, &[]).unwrap().len(), 1);
        let source = root.0.join("mods/a_review_pack/source.json");
        fs::write(&source, serde_json::to_vec(mod_source).unwrap()).unwrap();
        assert_eq!(selected_mods(&root.0, &[]).unwrap().len(), 2);
        let selected = selected_mods(&root.0, &[id.into()]).unwrap();
        assert_eq!(selected[0].0, id);
        fs::write(source, "{}").unwrap();
        assert!(selected_mods(&root.0, &[id.into()]).is_err());
    }
}
