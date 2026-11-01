//! The player's own download of the pinned Minecraft version from Mojang, in the official
//! launcher's folder layout (`versions/<v>/<v>.json`, `versions/<v>/<v>.jar`,
//! `libraries/...`, `assets/indexes/<id>.json`, `assets/objects/<xx>/<hash>`), plus the
//! server jar at `server/server.jar`. Each file is checked against the sha1 Mojang lists
//! and written whole, so a download stopped half way keeps what it finished and the next
//! run fetches only the rest.

use serde_json::Value;
use sha1::{Digest, Sha1};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::Duration;

pub const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const OBJECTS: &str = "https://resources.download.minecraft.net";
/// The version Horizon is built for.
pub const VERSION: &str = "26.3";

/// How far the download is, for the screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub stage: String,
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// One file to fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub url: String,
    pub sha1: String,
    pub size: u64,
    pub dest: PathBuf,
    /// Checked by size alone when it is already there (the thousands of asset objects).
    pub size_only: bool,
}

const CANCELLED: &str = "Download cancelled. Download again to go on where it stopped.";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(60))
        .user_agent("horizon-mobile")
        .try_proxy_from_env(true)
        .build()
}

fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let response = agent().get(url).call().map_err(|_| {
        "Could not reach Mojang's servers. Check your internet connection.".to_string()
    })?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(1 << 31)
        .read_to_end(&mut bytes)
        .map_err(|_| "The download was interrupted.".to_string())?;
    Ok(bytes)
}

pub fn sha1_hex(bytes: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn sha1_file(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether `item` is already on disk, whole and right.
fn present(item: &Item) -> bool {
    match fs::metadata(&item.dest) {
        Ok(m) if m.is_file() && (item.size == 0 || m.len() == item.size) => {
            item.size_only || sha1_file(&item.dest).is_some_and(|s| s == item.sha1)
        }
        _ => false,
    }
}

fn write_whole(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    let fail = |_| format!("Could not write {}", dest.display());
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).map_err(fail)?;
    }
    let tmp = dest.with_extension("part");
    fs::write(&tmp, bytes).map_err(fail)?;
    fs::rename(&tmp, dest).map_err(fail)
}

fn fetch(item: &Item) -> Result<u64, String> {
    let mut last = String::new();
    for _ in 0..3 {
        match get_bytes(&item.url) {
            Ok(bytes) if sha1_hex(&bytes) == item.sha1 => {
                write_whole(&item.dest, &bytes)?;
                return Ok(bytes.len() as u64);
            }
            Ok(_) => last = format!("A file from Mojang failed its check ({}).", item.url),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Whether a library's rules allow it here. Android counts as Linux on arm64; a rule that
/// names a feature (demo, custom resolution) never applies.
pub fn library_allowed(library: &Value) -> bool {
    let Some(rules) = library.get("rules").and_then(Value::as_array) else {
        return true;
    };
    let mut allowed = false;
    for rule in rules {
        if rule.get("features").is_some() {
            continue;
        }
        let os = rule.get("os");
        let matches = os.is_none_or(|os| {
            let name_ok = os
                .get("name")
                .and_then(Value::as_str)
                .is_none_or(|n| n == "linux");
            let arch_ok = os
                .get("arch")
                .and_then(Value::as_str)
                .is_none_or(|a| matches!(a, "aarch64" | "arm64"));
            name_ok && arch_ok
        });
        if matches {
            allowed = rule.get("action").and_then(Value::as_str) == Some("allow");
        }
    }
    allowed
}

fn item_from(v: &Value, dest: PathBuf, size_only: bool) -> Result<Item, String> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    Ok(Item {
        url: text("url").ok_or("Mojang's version file has no download address.")?,
        sha1: text("sha1").ok_or("Mojang's version file has no checksum.")?,
        size: v.get("size").and_then(Value::as_u64).unwrap_or(0),
        dest,
        size_only,
    })
}

/// A path from Mojang's JSON that stays inside `base`.
fn inside(base: &Path, relative: &str) -> Result<PathBuf, String> {
    let rel = Path::new(relative);
    if rel.is_absolute()
        || rel.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return Err("Mojang's version file names a path outside its folder.".into());
    }
    Ok(base.join(rel))
}

/// The files of the version JSON without the assets: the client jar, the server jar and the
/// libraries allowed here.
pub fn game_items(version: &Value, minecraft: &Path, id: &str) -> Result<Vec<Item>, String> {
    let downloads = version
        .get("downloads")
        .ok_or("The version file lists no downloads.")?;
    let mut items = vec![
        item_from(
            downloads
                .get("client")
                .ok_or("The version file has no client jar.")?,
            minecraft
                .join("versions")
                .join(id)
                .join(format!("{id}.jar")),
            false,
        )?,
        item_from(
            downloads
                .get("server")
                .ok_or("The version file has no server jar.")?,
            minecraft.join("server/server.jar"),
            false,
        )?,
    ];
    for library in version
        .get("libraries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !library_allowed(library) {
            continue;
        }
        let Some(artifact) = library.pointer("/downloads/artifact") else {
            continue;
        };
        let path = artifact
            .get("path")
            .and_then(Value::as_str)
            .ok_or("A library has no path.")?;
        items.push(item_from(
            artifact,
            inside(&minecraft.join("libraries"), path)?,
            false,
        )?);
    }
    Ok(items)
}

/// The asset objects of an asset index.
pub fn asset_items(index: &Value, minecraft: &Path) -> Result<Vec<Item>, String> {
    let objects = index
        .get("objects")
        .and_then(Value::as_object)
        .ok_or("The asset index has no objects.")?;
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for object in objects.values() {
        let hash = object
            .get("hash")
            .and_then(Value::as_str)
            .ok_or("An asset has no hash.")?;
        if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) || !seen.insert(hash) {
            continue;
        }
        let prefix = &hash[..2];
        items.push(Item {
            url: format!("{OBJECTS}/{prefix}/{hash}"),
            sha1: hash.to_owned(),
            size: object.get("size").and_then(Value::as_u64).unwrap_or(0),
            dest: minecraft.join("assets/objects").join(prefix).join(hash),
            size_only: true,
        });
    }
    Ok(items)
}

fn json(bytes: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(bytes).map_err(|_| "Mojang sent a file that isn't JSON.".to_string())
}

/// Downloads version `id` into `minecraft`. `with_assets` adds the asset index and every
/// asset object (the sounds and languages; several hundred megabytes). `progress` is called
/// as files finish; `cancel` stops it between files.
pub fn download(
    minecraft: &Path,
    id: &str,
    with_assets: bool,
    cancel: &AtomicBool,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<(), String> {
    let stage = |s: &str| {
        progress(Progress {
            stage: s.to_owned(),
            ..Progress::default()
        });
    };
    stage("Finding the version");
    let manifest = json(&get_bytes(MANIFEST)?)?;
    let entry = manifest
        .get("versions")
        .and_then(Value::as_array)
        .and_then(|v| {
            v.iter()
                .find(|e| e.get("id").and_then(Value::as_str) == Some(id))
        })
        .ok_or_else(|| format!("Mojang lists no version {id}."))?;
    let url = entry
        .get("url")
        .and_then(Value::as_str)
        .ok_or("The version has no address.")?;
    let sha1 = entry
        .get("sha1")
        .and_then(Value::as_str)
        .ok_or("The version has no checksum.")?;
    let version_bytes = get_bytes(url)?;
    if sha1_hex(&version_bytes) != sha1 {
        return Err("The version file failed its check.".into());
    }
    write_whole(
        &minecraft
            .join("versions")
            .join(id)
            .join(format!("{id}.json")),
        &version_bytes,
    )?;
    let version = json(&version_bytes)?;

    let mut items = game_items(&version, minecraft, id)?;
    if with_assets {
        stage("Reading the asset list");
        let index = version
            .get("assetIndex")
            .ok_or("The version file has no asset index.")?;
        let index_id = index
            .get("id")
            .and_then(Value::as_str)
            .ok_or("The asset index has no id.")?;
        let index_item = item_from(
            index,
            minecraft
                .join("assets/indexes")
                .join(format!("{index_id}.json")),
            false,
        )?;
        if !present(&index_item) {
            fetch(&index_item)?;
        }
        let index_json = json(
            &fs::read(&index_item.dest)
                .map_err(|_| "Could not read the asset index.".to_string())?,
        )?;
        items.extend(asset_items(&index_json, minecraft)?);
    }
    fetch_all(items, cancel, progress)
}

/// Fetches every item not already present, on a few threads.
pub fn fetch_all(
    items: Vec<Item>,
    cancel: &AtomicBool,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<(), String> {
    let total = items.len();
    let bytes_total: u64 = items.iter().map(|i| i.size).sum();
    let done = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    let next = AtomicUsize::new(0);
    let failure: Mutex<Option<String>> = Mutex::new(None);
    let report = |label: &str| {
        progress(Progress {
            stage: label.to_owned(),
            files_done: done.load(Relaxed),
            files_total: total,
            bytes_done: bytes.load(Relaxed),
            bytes_total,
        });
    };
    report("Downloading");
    std::thread::scope(|scope| {
        for _ in 0..6 {
            scope.spawn(|| {
                loop {
                    if cancel.load(Relaxed) || failure.lock().is_ok_and(|f| f.is_some()) {
                        return;
                    }
                    let i = next.fetch_add(1, Relaxed);
                    let Some(item) = items.get(i) else {
                        return;
                    };
                    let got = if present(item) {
                        Ok(item.size)
                    } else {
                        fetch(item)
                    };
                    match got {
                        Ok(n) => {
                            bytes.fetch_add(n, Relaxed);
                            done.fetch_add(1, Relaxed);
                            report("Downloading");
                        }
                        Err(e) => {
                            if let Ok(mut f) = failure.lock() {
                                f.get_or_insert(e);
                            }
                            return;
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = failure.into_inner().ok().flatten() {
        return Err(e);
    }
    if cancel.load(Relaxed) {
        return Err(CANCELLED.into());
    }
    report("Done");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rules_pick_linux_arm64_android() {
        assert!(library_allowed(&json!({})));
        assert!(library_allowed(&json!({"rules":[{"action":"allow"}]})));
        assert!(library_allowed(
            &json!({"rules":[{"action":"allow","os":{"name":"linux"}}]})
        ));
        assert!(!library_allowed(
            &json!({"rules":[{"action":"allow","os":{"name":"windows"}}]})
        ));
        assert!(!library_allowed(
            &json!({"rules":[{"action":"allow","os":{"name":"osx"}}]})
        ));
        assert!(!library_allowed(
            &json!({"rules":[{"action":"allow","os":{"name":"linux","arch":"x86_64"}}]})
        ));
        assert!(library_allowed(
            &json!({"rules":[{"action":"allow","os":{"name":"linux","arch":"aarch64"}}]})
        ));
        assert!(!library_allowed(
            &json!({"rules":[{"action":"allow"},{"action":"disallow","os":{"name":"linux"}}]})
        ));
        assert!(!library_allowed(
            &json!({"rules":[{"action":"allow","features":{"is_demo_user":true}}]})
        ));
    }

    #[test]
    fn paths_cannot_leave_the_folder() {
        let base = Path::new("/data/libraries");
        assert!(inside(base, "a/b/c.jar").is_ok());
        assert!(inside(base, "../evil").is_err());
        assert!(inside(base, "/etc/passwd").is_err());
    }

    #[test]
    fn game_items_list_jars_and_allowed_libraries() {
        let v = json!({
            "downloads": {
                "client": {"url":"https://x/c.jar","sha1":"aa","size":3},
                "server": {"url":"https://x/s.jar","sha1":"bb","size":4}
            },
            "libraries": [
                {"name":"a","downloads":{"artifact":{"path":"a/a.jar","url":"https://x/a","sha1":"cc","size":1}}},
                {"name":"w","rules":[{"action":"allow","os":{"name":"windows"}}],
                 "downloads":{"artifact":{"path":"w/w.jar","url":"https://x/w","sha1":"dd","size":1}}}
            ]
        });
        let items = game_items(&v, Path::new("/mc"), "26.3").unwrap();
        let dests: Vec<_> = items
            .iter()
            .map(|i| i.dest.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            dests,
            [
                "/mc/versions/26.3/26.3.jar",
                "/mc/server/server.jar",
                "/mc/libraries/a/a.jar"
            ]
        );
    }

    #[test]
    fn asset_items_dedupe_and_reject_bad_hashes() {
        let h = "0123456789abcdef0123456789abcdef01234567";
        let index = json!({"objects":{
            "a": {"hash": h, "size": 5},
            "b": {"hash": h, "size": 5},
            "c": {"hash": "zz", "size": 1}
        }});
        let items = asset_items(&index, Path::new("/mc")).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, format!("{OBJECTS}/01/{h}"));
        assert!(items[0].dest.ends_with(format!("assets/objects/01/{h}")));
    }

    #[test]
    fn present_checks_size_and_sha1() {
        let dir = std::env::temp_dir().join(format!("hb-dl-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f");
        fs::write(&file, b"hello").unwrap();
        let good = sha1_hex(b"hello");
        let mut item = Item {
            url: String::new(),
            sha1: good,
            size: 5,
            dest: file.clone(),
            size_only: false,
        };
        assert!(present(&item));
        item.sha1 = "0".repeat(40);
        assert!(!present(&item));
        item.size_only = true;
        assert!(present(&item));
        item.size = 6;
        assert!(!present(&item));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cancelled_before_start_downloads_nothing() {
        let dir = std::env::temp_dir().join(format!("hb-cancel-{}", std::process::id()));
        let item = Item {
            url: "https://invalid.invalid/x".into(),
            sha1: "0".repeat(40),
            size: 1,
            dest: dir.join("x"),
            size_only: false,
        };
        let cancel = AtomicBool::new(true);
        let r = fetch_all(vec![item], &cancel, &|_| {});
        assert!(r.is_err());
        assert!(!dir.join("x").exists());
    }
}
