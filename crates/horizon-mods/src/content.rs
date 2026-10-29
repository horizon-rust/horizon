//! A content mod's `assets/` and `data/` (API 0.2, docs/modpacks.md): the
//! files it adds to the game's resource tree, and the blocks and items it
//! declares in `data/<id>/horizon/block/` and `data/<id>/horizon/item/`.
//!
//! Content is data, not code: nothing here runs in the sandbox. What a mod
//! may add is fenced instead. Its own namespaces (its id and the ones its
//! manifest lists), plus additions to vanilla tags and to the convention
//! tags (`c`, which NeoForge and Fabric mods share). Paths are plain, files
//! and totals are capped, vanilla ids cannot be redefined, and no two mods
//! share a namespace ([`clash`]).
use crate::manifest::{Capability, MANIFEST, Manifest};
use cap_std::fs::Dir;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};

/// Content files per mod, bytes per file and bytes per mod.
pub const MAX_FILES: usize = 65_536;
pub const MAX_FILE: u64 = 16 << 20;
pub const MAX_TOTAL: u64 = 1024 << 20;

/// Checks a content path: `assets/` or `data/`, then `/`-separated parts of
/// `[a-z0-9_.-]`, none of them `.` or `..` (vanilla's resource-location
/// characters, so the path means the same on every OS).
pub fn check_path(name: &str) -> Result<(), String> {
    let bad = || format!("{name:?} is not a plain assets/ or data/ path");
    let mut parts = name.split('/');
    if !matches!(parts.next(), Some("assets" | "data")) {
        return Err(bad());
    }
    let mut count = 0;
    for part in parts {
        count += 1;
        let chars_ok = part
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
        if part.is_empty() || part == "." || part == ".." || !chars_ok {
            return Err(bad());
        }
    }
    // assets/<namespace>/<file> at least.
    if count < 2 {
        return Err(bad());
    }
    Ok(())
}

/// A vanilla identifier, `minecraft:path`.
fn vanilla_id(id: &str) -> bool {
    id.strip_prefix("minecraft:").is_some_and(|path| {
        !path.is_empty()
            && path
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_./-".contains(&b))
    })
}

/// What a declared block changes from its template; unset keeps the
/// template's value.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BlockProperties {
    /// Destroy time in seconds of hand mining (-1: unbreakable).
    pub hardness: Option<f32>,
    pub explosion_resistance: Option<f32>,
    /// Light emitted, 0 to 15.
    pub light: Option<u8>,
    pub requires_tool: Option<bool>,
    pub friction: Option<f32>,
    /// A `MapColor` id, 0 to 61.
    pub map_color: Option<u8>,
    /// A vanilla block whose sounds this block uses.
    pub sounds: Option<String>,
}

/// Explicit registry states. Aliases choose template physics, not custom behavior.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BlockStates {
    #[serde(deserialize_with = "unique_state_map")]
    pub properties: BTreeMap<String, Vec<String>>,
    pub default: String,
    #[serde(deserialize_with = "unique_state_map")]
    pub aliases: BTreeMap<String, String>,
}
fn unique_state_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Map<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Map<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a bounded state map with unique keys")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut input: A,
        ) -> Result<Self::Value, A::Error> {
            let mut output = BTreeMap::new();
            while let Some((key, value)) = input.next_entry::<String, T>()? {
                if output.len() >= MAX_BLOCK_STATES || output.contains_key(&key) {
                    return Err(serde::de::Error::custom(
                        "duplicate or excessive custom state keys",
                    ));
                }
                output.insert(key, value);
            }
            Ok(output)
        }
    }
    deserializer.deserialize_map(Map(std::marker::PhantomData))
}
pub const MAX_BLOCK_PROPERTIES: usize = 8;
pub const MAX_PROPERTY_VALUES: usize = 26;
pub const MAX_BLOCK_STATES: usize = 4374;
impl BlockStates {
    pub fn key(properties: &BTreeMap<String, String>) -> String {
        properties
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(",")
    }
    pub fn variants(&self, template: &str) -> Result<Vec<BTreeMap<String, String>>, String> {
        let token = |s: &str| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
        };
        if self.properties.len() > MAX_BLOCK_PROPERTIES {
            return Err("at most 8 custom block properties".into());
        }
        let mut count = 1usize;
        for (name, values) in &self.properties {
            if !token(name)
                || values.is_empty()
                || values.len() > MAX_PROPERTY_VALUES
                || values.iter().any(|v| !token(v))
                || values.iter().collect::<BTreeSet<_>>().len() != values.len()
            {
                return Err(format!("invalid custom property domain {name}"));
            }
            count = count
                .checked_mul(values.len())
                .ok_or("custom state count overflow")?;
            if count > MAX_BLOCK_STATES {
                return Err("at most 4374 custom states per block".into());
            }
        }
        if self.default.len() > 2048 || self.aliases.len() != count {
            return Err("custom states require one default and an alias for every variant".into());
        }
        let mut variants = vec![BTreeMap::new()];
        for (name, values) in &self.properties {
            variants = variants
                .into_iter()
                .flat_map(|state| {
                    values.iter().map(move |value| {
                        let mut state = state.clone();
                        state.insert(name.clone(), value.clone());
                        state
                    })
                })
                .collect();
        }
        let mut found_default = false;
        for state in &variants {
            let key = Self::key(state);
            found_default |= key == self.default;
            let alias = self
                .aliases
                .get(&key)
                .ok_or_else(|| format!("missing custom state alias {key}"))?;
            if alias.len() > 2048
                || !(alias == template
                    || alias
                        .strip_prefix(template)
                        .is_some_and(|s| s.starts_with('[') && s.ends_with(']')))
            {
                return Err(format!(
                    "{key}: alias must name a complete state of template {template}"
                ));
            }
        }
        if !found_default {
            return Err("custom default is not a declared variant".into());
        }
        Ok(variants)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockFile {
    template: String,
    #[serde(default)]
    states: Option<BlockStates>,
    #[serde(default)]
    properties: BlockProperties,
    #[serde(default = "yes")]
    item: bool,
}
fn yes() -> bool {
    true
}

/// A block the mod adds: a vanilla template's behaviour, states and shapes
/// under a new id, with `properties` applied.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// `<mod id>:<path>`.
    pub id: String,
    /// The vanilla block it copies, `minecraft:<path>`.
    pub template: String,
    pub states: Option<BlockStates>,
    pub properties: BlockProperties,
    /// Also add a block item with the same id.
    pub item: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Epic,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemFile {
    template: String,
    max_stack_size: Option<u8>,
    rarity: Option<Rarity>,
}

/// An item the mod adds: a vanilla template item's components under a new
/// id, with these changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    /// The vanilla item it copies, `minecraft:<path>`.
    pub template: String,
    pub max_stack_size: Option<u8>,
    pub rarity: Option<Rarity>,
}

#[derive(Debug)]
enum Source {
    Dir(PathBuf, Dir),
    Zip(PathBuf),
}

/// A mod's content: its files and declarations.
#[derive(Debug)]
pub struct Content {
    source: Source,
    /// Every content file (`assets/...`, `data/...`), sorted.
    pub files: Vec<String>,
    pub blocks: Vec<Block>,
    pub items: Vec<Item>,
    /// The namespaces it owns: the mod id, then the manifest's `namespaces`.
    pub namespaces: Vec<String>,
}

/// Reads a mod's content files with an owned handle ([`Content::reader`]).
pub enum FileReader {
    Dir(Dir),
    Zip(Box<zip::ZipArchive<std::fs::File>>),
}
impl FileReader {
    /// One content file's bytes.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, String> {
        check_path(name)?;
        self.read_bounded(name, MAX_FILE)
            .map_err(|e| format!("{name}: {e}"))
    }

    /// At most `limit` bytes, with one extra byte to detect growth.
    pub(crate) fn read_bounded(&mut self, name: &str, limit: u64) -> Result<Vec<u8>, String> {
        check_path(name).map_err(|_| "invalid resource path")?;
        let unavailable = |_| "resource unavailable".to_string();
        let reader: Box<dyn Read + '_> = match self {
            Self::Dir(root) => {
                if !root.symlink_metadata(name).map_err(unavailable)?.is_file() {
                    return Err("resource unavailable".into());
                }
                let file = root.open(name).map_err(unavailable)?;
                if !file.metadata().map_err(unavailable)?.is_file() {
                    return Err("resource unavailable".into());
                }
                Box::new(file)
            }
            Self::Zip(zip) => Box::new(zip.by_name(name).map_err(|_| "resource unavailable")?),
        };
        let mut out = Vec::new();
        reader
            .take(limit.min(MAX_FILE) + 1)
            .read_to_end(&mut out)
            .map_err(unavailable)?;
        if out.len() as u64 > limit.min(MAX_FILE) {
            return Err("resource file too large".into());
        }
        Ok(out)
    }
}

/// Why two content mods cannot be on together: a namespace both own (one's
/// id or listed namespace is the other's), which would mix their files.
/// `a` and `b` are (mod id, owned namespaces).
pub fn clash(a: (&str, &[String]), b: (&str, &[String])) -> Option<String> {
    let shared = a.1.iter().find(|ns| b.1.contains(ns))?;
    Some(format!(
        "mods {} and {} both use the namespace {shared}; turn one of them off",
        a.0, b.0
    ))
}

impl Content {
    /// The content of the mod at `path` (a `.hmod` or a mod folder).
    pub fn read(path: &Path, manifest: &Manifest) -> Result<Content, String> {
        let (source, sized) = if path.is_dir() {
            let root = Dir::open_ambient_dir(path, cap_std::ambient_authority())
                .map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            for top in ["assets", "data"] {
                walk(&root, top, &mut out)?;
            }
            (Source::Dir(path.to_owned(), root), out)
        } else {
            let mut zip = open_zip(path)?;
            let mut out = Vec::new();
            for i in 0..zip.len() {
                let entry = zip.by_index_raw(i).map_err(|e| e.to_string())?;
                let name = entry.name();
                if (name.starts_with("assets/") || name.starts_with("data/")) && !entry.is_dir() {
                    out.push((name.to_owned(), entry.size()));
                }
            }
            (Source::Zip(path.to_owned()), out)
        };
        let mut files = Vec::with_capacity(sized.len());
        let mut total = 0u64;
        for (name, size) in sized {
            check_path(&name)?;
            if size > MAX_FILE {
                return Err(format!("{name} is over {} MiB", MAX_FILE >> 20));
            }
            total += size;
            files.push(name);
        }
        if files.len() > MAX_FILES {
            return Err(format!("more than {MAX_FILES} content files"));
        }
        if total > MAX_TOTAL {
            return Err(format!("content is over {} MiB", MAX_TOTAL >> 20));
        }
        files.sort();
        files.dedup();
        let mut content = Content {
            source,
            files,
            blocks: Vec::new(),
            items: Vec::new(),
            namespaces: manifest.owned_namespaces().map(str::to_owned).collect(),
        };
        content.check(manifest)?;
        Ok(content)
    }

    /// Where the files are read from: the mod's folder or its `.hmod`.
    pub fn source(&self) -> &Path {
        match &self.source {
            Source::Dir(path, _) | Source::Zip(path) => path,
        }
    }

    /// No files: a mod with code only.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// One content file's bytes.
    pub fn file(&self, name: &str) -> Result<Vec<u8>, String> {
        self.reader()?.read(name)
    }

    /// A reader of its content files. A packaged mod's archive is opened
    /// once for all of them (opening it reads its whole index).
    pub fn reader(&self) -> Result<FileReader, String> {
        Ok(match &self.source {
            Source::Dir(_, root) => FileReader::Dir(root.try_clone().map_err(|e| e.to_string())?),
            Source::Zip(path) => FileReader::Zip(Box::new(open_zip(path)?)),
        })
    }

    /// The guest's fence is checked before any file is opened.
    #[cfg(feature = "runtime")]
    pub(crate) fn resource_path(&self, name: &str) -> Result<bool, String> {
        if name.len() > 1024 || check_path(name).is_err() {
            return Err("invalid resource path".into());
        }
        let namespace = name.split('/').nth(1).unwrap_or_default();
        if matches!(namespace, "minecraft" | "c" | "horizon")
            || !self.namespaces.iter().any(|ns| ns == namespace)
        {
            return Err("resource outside owner".into());
        }
        Ok(self
            .files
            .binary_search_by(|file| file.as_str().cmp(name))
            .is_ok())
    }

    /// Where content may go, and the declarations.
    fn check(&mut self, manifest: &Manifest) -> Result<(), String> {
        let id = manifest.id.as_str();
        let registries = manifest.capabilities.contains(&Capability::Registries);
        if !self.files.is_empty()
            && !registries
            && !manifest.capabilities.contains(&Capability::Resources)
        {
            return Err(format!(
                "has assets/ or data/ but not capability registries or resources in {MANIFEST}"
            ));
        }
        let mut blocks = Vec::new();
        let mut items = Vec::new();
        for name in &self.files {
            let mut parts = name.splitn(3, '/');
            let (top, namespace, rest) = (
                parts.next().unwrap_or(""),
                parts.next().unwrap_or(""),
                parts.next().unwrap_or(""),
            );
            let own = namespace == id;
            let owned = own || manifest.namespaces.iter().any(|n| n == namespace);
            // Vanilla's own files stay vanilla's; a mod may only add to its
            // tags, and to the convention tags mods share.
            let shared = matches!(namespace, "minecraft" | "c");
            let allowed =
                owned || (registries && top == "data" && shared && rest.starts_with("tags/"));
            if !allowed {
                return Err(format!(
                    "{name}: a mod adds files in its own namespaces ({top}/{id}/ and those listed in namespaces) or to vanilla and convention (c) tags"
                ));
            }
            if !registries && top == "data" && rest.starts_with("horizon/") {
                return Err(format!("{name}: declarations need capability registries"));
            }
            let Some(decl) = (owned && top == "data")
                .then(|| rest.strip_prefix("horizon/"))
                .flatten()
            else {
                continue;
            };
            let parse = |kind: &str| -> Option<String> {
                let path = decl
                    .strip_prefix(kind)?
                    .strip_prefix('/')?
                    .strip_suffix(".json")?;
                Some(format!("{namespace}:{path}"))
            };
            if let Some(block) = parse("block") {
                blocks.push((block, name.clone()));
            } else if let Some(item) = parse("item") {
                items.push((item, name.clone()));
            } else {
                return Err(format!(
                    "{name}: data/{id}/horizon/ holds block/*.json and item/*.json only"
                ));
            }
        }
        let mut ids = BTreeSet::new();
        for (block, name) in blocks {
            let file: BlockFile =
                serde_json::from_slice(&self.file(&name)?).map_err(|e| format!("{name}: {e}"))?;
            check_block(&file).map_err(|e| format!("{name}: {e}"))?;
            if file.item {
                ids.insert(block.clone());
            }
            self.blocks.push(Block {
                id: block,
                template: file.template,
                states: file.states,
                properties: file.properties,
                item: file.item,
            });
        }
        for (item, name) in items {
            let file: ItemFile =
                serde_json::from_slice(&self.file(&name)?).map_err(|e| format!("{name}: {e}"))?;
            if !vanilla_id(&file.template) {
                return Err(format!(
                    "{name}: template {:?} is not a vanilla item (minecraft:...)",
                    file.template
                ));
            }
            if file.max_stack_size.is_some_and(|n| !(1..=99).contains(&n)) {
                return Err(format!("{name}: max_stack_size is 1 to 99"));
            }
            if !ids.insert(item.clone()) {
                return Err(format!("{name}: {item} is also a block's item"));
            }
            self.items.push(Item {
                id: item,
                template: file.template,
                max_stack_size: file.max_stack_size,
                rarity: file.rarity,
            });
        }
        Ok(())
    }
}

fn check_block(file: &BlockFile) -> Result<(), String> {
    if !vanilla_id(&file.template) {
        return Err(format!(
            "template {:?} is not a vanilla block (minecraft:...)",
            file.template
        ));
    }
    if let Some(states) = &file.states {
        states.variants(&file.template)?;
    }
    let p = &file.properties;
    let finite = |v: Option<f32>| v.is_none_or(f32::is_finite);
    if !finite(p.hardness) || !finite(p.explosion_resistance) || !finite(p.friction) {
        return Err("hardness, explosion_resistance and friction are numbers".into());
    }
    if p.hardness.is_some_and(|h| h < 0.0 && h != -1.0) {
        return Err("hardness is 0 or more, or -1 (unbreakable)".into());
    }
    if p.explosion_resistance.is_some_and(|r| r < 0.0) {
        return Err("explosion_resistance is 0 or more".into());
    }
    if p.friction.is_some_and(|f| !(0.0..=2.0).contains(&f)) {
        return Err("friction is 0 to 2".into());
    }
    if p.light.is_some_and(|l| l > 15) {
        return Err("light is 0 to 15".into());
    }
    if p.map_color.is_some_and(|c| c > 61) {
        return Err("map_color is a MapColor id, 0 to 61".into());
    }
    if p.sounds.as_deref().is_some_and(|s| !vanilla_id(s)) {
        return Err("sounds names a vanilla block (minecraft:...)".into());
    }
    Ok(())
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    zip::ZipArchive::new(file).map_err(|e| e.to_string())
}

/// Files under `dir`, relative to `root` with `/` separators. Symbolic links
/// are not followed: a mod folder holds its own files.
fn walk(root: &Dir, dir: &str, out: &mut Vec<(String, u64)>) -> Result<(), String> {
    let meta = match root.symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    if !meta.is_dir() {
        return Ok(());
    }
    for entry in root.read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let file_name = entry.file_name();
        let file_name = file_name.to_str().ok_or("content path is not UTF-8")?;
        let name = format!("{dir}/{file_name}");
        let meta = root.symlink_metadata(&name).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            walk(root, &name, out)?;
        } else if meta.is_file() {
            out.push((name, meta.len()));
        }
        if out.len() > MAX_FILES {
            return Err(format!("more than {MAX_FILES} content files"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Package;

    const MANIFEST_TEXT: &str = r#"
id = "ruby"
name = "Ruby"
version = "0.1.0"
api = "0.2"
side = "both"
capabilities = ["registries"]
"#;
    const BLOCK: &[u8] = br#"{"template": "minecraft:iron_block",
        "properties": {"hardness": 4.0, "light": 3, "sounds": "minecraft:amethyst_block"}}"#;
    const ITEM: &[u8] = br#"{"template": "minecraft:diamond", "rarity": "uncommon"}"#;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-content-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn hmod(name: &str, manifest: &str, files: &[(&str, &[u8])]) -> Result<Content, String> {
        let dir = scratch(name);
        let path = dir.join("m.hmod");
        std::fs::write(&path, Package::write_files(manifest, None, files)?).unwrap();
        let package = Package::read(&path);
        let content = package.and_then(|p| Content::read(&path, &p.manifest));
        std::fs::remove_dir_all(&dir).unwrap();
        content
    }

    #[test]
    fn content_only_mod_round_trips() {
        let files: &[(&str, &[u8])] = &[
            ("data/ruby/horizon/block/ruby_block.json", BLOCK),
            ("data/ruby/horizon/item/ruby.json", ITEM),
            ("data/ruby/recipe/ruby_block.json", b"{}"),
            ("data/minecraft/tags/block/mineable/pickaxe.json", b"{}"),
            ("data/c/tags/item/gems/ruby.json", b"{}"),
            ("assets/ruby/textures/block/ruby_block.png", b"png"),
        ];
        let content = hmod("ok", MANIFEST_TEXT, files).unwrap();
        assert_eq!(content.files.len(), 6);
        assert_eq!(
            content.blocks,
            [Block {
                id: "ruby:ruby_block".into(),
                template: "minecraft:iron_block".into(),
                states: None,
                properties: BlockProperties {
                    hardness: Some(4.0),
                    light: Some(3),
                    sounds: Some("minecraft:amethyst_block".into()),
                    ..BlockProperties::default()
                },
                item: true,
            }]
        );
        assert_eq!(content.items[0].id, "ruby:ruby");
        assert_eq!(content.items[0].rarity, Some(Rarity::Uncommon));
    }

    #[test]
    fn folder_mod_reads_the_same() {
        let dir = scratch("folder");
        std::fs::write(dir.join(MANIFEST), MANIFEST_TEXT).unwrap();
        let block = dir.join("data/ruby/horizon/block");
        std::fs::create_dir_all(&block).unwrap();
        std::fs::write(block.join("ruby_block.json"), BLOCK).unwrap();
        let package = Package::read(&dir).unwrap();
        assert!(package.wasm.is_none());
        let content = Content::read(&dir, &package.manifest).unwrap();
        assert_eq!(content.files, ["data/ruby/horizon/block/ruby_block.json"]);
        assert_eq!(content.file(&content.files[0]).unwrap(), BLOCK);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_content_outside_its_fence() {
        let refused = |name: &str, files: &[(&str, &[u8])]| {
            let e = hmod(name, MANIFEST_TEXT, files).unwrap_err();
            assert!(!e.is_empty());
            e
        };
        let e = refused("other-ns", &[("data/other/horizon/block/a.json", BLOCK)]);
        assert!(e.contains("own namespaces"), "{e}");
        refused(
            "vanilla-asset",
            &[("assets/minecraft/textures/block/stone.png", b"x")],
        );
        refused(
            "vanilla-recipe",
            &[("data/minecraft/recipe/stone.json", b"{}")],
        );
        refused("convention-recipe", &[("data/c/recipe/stone.json", b"{}")]);
        refused("convention-asset", &[("assets/c/lang/en_us.json", b"{}")]);
        let e = refused(
            "modded-template",
            &[(
                "data/ruby/horizon/block/a.json",
                br#"{"template": "ruby:other"}"#,
            )],
        );
        assert!(e.contains("not a vanilla block"), "{e}");
        refused(
            "light",
            &[(
                "data/ruby/horizon/block/a.json",
                br#"{"template": "minecraft:stone", "properties": {"light": 16}}"#,
            )],
        );
        refused(
            "unknown-key",
            &[(
                "data/ruby/horizon/block/a.json",
                br#"{"template": "minecraft:stone", "glow": true}"#,
            )],
        );
        let e = refused(
            "clash",
            &[
                ("data/ruby/horizon/block/a.json", BLOCK),
                ("data/ruby/horizon/item/a.json", ITEM),
            ],
        );
        assert!(e.contains("also a block's item"), "{e}");
        refused("other-kind", &[("data/ruby/horizon/entity/a.json", b"{}")]);
        refused(
            "stack",
            &[(
                "data/ruby/horizon/item/a.json",
                br#"{"template": "minecraft:stick", "max_stack_size": 100}"#,
            )],
        );
    }

    #[test]
    fn listed_namespaces_hold_content() {
        let manifest = format!("{MANIFEST_TEXT}namespaces = [\"kaisyn\"]\n");
        let files: &[(&str, &[u8])] = &[
            ("data/kaisyn/worldgen/template_pool/a.json", b"{}"),
            ("data/kaisyn/structure/a.nbt", b"nbt"),
            ("assets/kaisyn/lang/en_us.json", b"{}"),
        ];
        let content = hmod("listed", &manifest, files).unwrap();
        assert_eq!(content.files.len(), 3);
        assert_eq!(content.namespaces, ["ruby", "kaisyn"]);
        // Declarations stay in the id's namespace.
        let decl: &[(&str, &[u8])] = &[("data/kaisyn/horizon/block/a.json", BLOCK)];
        let e = hmod("listed-decl", &manifest, decl).unwrap_err();
        assert!(e.contains("declared in data/ruby/horizon/"), "{e}");
        // A namespace it does not list is still outside.
        let other: &[(&str, &[u8])] = &[("data/other/structure/a.nbt", b"nbt")];
        assert!(hmod("listed-other", &manifest, other).is_err());
    }

    #[test]
    fn mods_sharing_a_namespace_clash() {
        let t_and_t = ["t_and_t".to_owned(), "kaisyn".to_owned()];
        let kaisyn = ["kaisyn".to_owned()];
        let ruby = ["ruby".to_owned(), "gems".to_owned()];
        assert_eq!(
            clash(("t_and_t", &t_and_t), ("kaisyn", &kaisyn)).as_deref(),
            Some("mods t_and_t and kaisyn both use the namespace kaisyn; turn one of them off")
        );
        assert_eq!(clash(("t_and_t", &t_and_t), ("ruby", &ruby)), None);
    }

    #[test]
    fn content_needs_the_registries_capability() {
        let code_only = MANIFEST_TEXT
            .replace("api = \"0.2\"", "api = \"0.1\"")
            .replace("capabilities = [\"registries\"]", "capabilities = []");
        // No code and no registries: nothing to load.
        let e = hmod("empty", &code_only, &[]).unwrap_err();
        assert!(e.contains("mod.wasm: missing"), "{e}");
        // Registries on API 0.1.
        let old = MANIFEST_TEXT.replace("api = \"0.2\"", "api = \"0.1\"");
        assert!(Manifest::parse(&old).unwrap_err().contains("needs api"));
        // Content without the capability, in a mod with code.
        let dir = scratch("no-cap");
        let path = dir.join("m.hmod");
        let files: &[(&str, &[u8])] = &[("assets/ruby/lang/en_us.json", b"{}")];
        std::fs::write(
            &path,
            Package::write_files(&code_only, Some(b"\0asm"), files).unwrap(),
        )
        .unwrap();
        let package = Package::read(&path).unwrap();
        let e = Content::read(&path, &package.manifest).err().unwrap();
        assert!(e.contains("not capability registries"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paths_are_plain() {
        assert!(check_path("assets/ruby/textures/block/ruby_block.png").is_ok());
        assert!(check_path("data/ruby/loot_table/blocks/a.json").is_ok());
        for bad in [
            "assets/ruby",
            "assets/../x/y",
            "assets/ruby//a",
            "assets/Ruby/a",
            "assets/ruby/a b",
            "/assets/ruby/a",
            "other/ruby/a",
            "assets\\ruby\\a",
        ] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn resources_do_not_grant_registrations_or_shared_tags() {
        let manifest = MANIFEST_TEXT.replace("[\"registries\"]", "[\"resources\"]")
            + "experimental = [\"resources\"]\n";
        let m = Manifest::parse(&manifest).unwrap();
        let dir = scratch("resources-policy");
        for name in [
            "data/ruby/horizon/block/a.json",
            "data/minecraft/tags/block/a.json",
            "data/c/tags/item/a.json",
        ] {
            let path = dir.join("policy.hmod");
            std::fs::write(
                &path,
                Package::write_files(&manifest, Some(b"synthetic"), &[(name, b"{}")]).unwrap(),
            )
            .unwrap();
            assert!(Content::read(&path, &m).is_err(), "{name}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_owned_reader_outlives_content_and_the_directory_name() {
        let dir = scratch("owned-reader");
        std::fs::create_dir_all(dir.join("assets/ruby")).unwrap();
        std::fs::write(dir.join("assets/ruby/a.bin"), b"original scratch bytes").unwrap();
        let mut reader = {
            let content = Content::read(&dir, &Manifest::parse(MANIFEST_TEXT).unwrap()).unwrap();
            content.reader().unwrap()
        };
        let moved = dir.with_extension("moved");
        std::fs::rename(&dir, &moved).unwrap();
        std::fs::create_dir_all(dir.join("assets/ruby")).unwrap();
        std::fs::write(dir.join("assets/ruby/a.bin"), b"replacement scratch bytes").unwrap();
        assert_eq!(
            reader.read("assets/ruby/a.bin").unwrap(),
            b"original scratch bytes"
        );
        drop(reader);
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(moved).unwrap();
    }

    #[test]
    fn resource_inventory_retains_package_count_and_total_limits() {
        let dir = scratch("inventory-limits");
        let m = Manifest::parse(MANIFEST_TEXT).unwrap();
        let folder = dir.join("data/ruby");
        std::fs::create_dir_all(&folder).unwrap();
        for i in 0..=(MAX_TOTAL / MAX_FILE) {
            std::fs::File::create(folder.join(format!("{i}.bin")))
                .unwrap()
                .set_len(MAX_FILE)
                .unwrap();
        }
        assert_eq!(
            Content::read(&dir, &m).unwrap_err(),
            "content is over 1024 MiB"
        );
        let path = dir.join("count.hmod");
        let names: Vec<_> = (0..=MAX_FILES)
            .map(|i| format!("data/ruby/{i}.bin"))
            .collect();
        let files: Vec<_> = names.iter().map(|name| (name.as_str(), &b""[..])).collect();
        std::fs::write(
            &path,
            Package::write_files(MANIFEST_TEXT, None, &files).unwrap(),
        )
        .unwrap();
        assert_eq!(
            Content::read(&path, &m).unwrap_err(),
            format!("more than {MAX_FILES} content files")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
