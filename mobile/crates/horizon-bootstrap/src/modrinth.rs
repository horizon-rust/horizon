//! Modrinth's public API (api.modrinth.com/v2, no key) for resource packs: search, and installing a
//! pack's file into an instance with its SHA-512 checked. The same calls as the desktop launcher's
//! browser (`launcher/src/browser/modrinth.rs`), for the mobile Mods page.

use serde_json::Value;
use sha2::{Digest, Sha512};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const API: &str = "https://api.modrinth.com/v2";
/// The Minecraft version Horizon runs.
pub const GAME: &str = crate::registry::GAME;
/// The biggest pack taken.
const MAX_FILE: u64 = 256 << 20;
pub const PAGE: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub author: String,
    pub downloads: u64,
}

/// The percent-encoding of a query value or a path segment.
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The search's facets (an AND of ORs): resource packs, and unless `everything` those marked for this game.
pub fn facets(everything: bool) -> String {
    let mut all = vec![vec!["project_type:resourcepack".to_owned()]];
    if !everything {
        all.push(vec![format!("versions:{GAME}")]);
    }
    serde_json::to_string(&all).unwrap_or_default()
}

pub fn search_url(text: &str, offset: usize, everything: bool) -> String {
    let mut url = format!(
        "{API}/search?facets={}&index={}&offset={offset}&limit={PAGE}",
        encode(&facets(everything)),
        if text.trim().is_empty() {
            "downloads"
        } else {
            "relevance"
        },
    );
    if !text.trim().is_empty() {
        url.push_str("&query=");
        url.push_str(&encode(text.trim()));
    }
    url
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

/// The hits of a search answer.
pub fn parse_hits(answer: &Value) -> Vec<Hit> {
    answer["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|h| h["project_type"] == "resourcepack")
        .filter_map(|h| {
            Some(Hit {
                id: h["project_id"].as_str()?.to_owned(),
                title: text(&h["title"]),
                summary: text(&h["description"]),
                author: text(&h["author"]),
                downloads: h["downloads"].as_u64().unwrap_or(0),
            })
        })
        .collect()
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .user_agent("horizon-mobile")
        .try_proxy_from_env(true)
        .build()
}

fn get_json(url: &str) -> Result<Value, String> {
    let response = agent().get(url).call().map_err(|e| match e {
        ureq::Error::Status(429, _) => {
            "Modrinth asks to slow down: try again in a minute.".to_string()
        }
        ureq::Error::Status(code, _) => format!("Modrinth answered {code}."),
        ureq::Error::Transport(t) => format!("No connection to Modrinth: {t}"),
    })?;
    response
        .into_json()
        .map_err(|e| format!("Modrinth's answer wasn't readable: {e}"))
}

/// A page of resource packs: `text` empty lists the most downloaded. Returns the hits and whether
/// more may follow.
pub fn search(text: &str, offset: usize, everything: bool) -> Result<(Vec<Hit>, bool), String> {
    let answer = get_json(&search_url(text, offset, everything))?;
    let total = answer["total_hits"].as_u64().unwrap_or(0) as usize;
    let hits = parse_hits(&answer);
    let more = offset + hits.len() < total && !hits.is_empty();
    Ok((hits, more))
}

/// One file to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha512: String,
}

/// The file to install from `GET /project/{id}/version`: among the listed versions the newest that
/// names this game, else the newest; its primary file (else the first).
pub fn pick_file(versions: &Value) -> Option<File> {
    let list: Vec<&Value> = versions
        .as_array()?
        .iter()
        .filter(|v| v["status"].as_str().is_none_or(|s| s == "listed"))
        .collect();
    let mut sorted = list.clone();
    sorted.sort_by(|a, b| text(&b["date_published"]).cmp(&text(&a["date_published"])));
    let for_game = |v: &Value| {
        v["game_versions"]
            .as_array()
            .is_some_and(|g| g.iter().any(|x| x == GAME))
    };
    let version = sorted
        .iter()
        .copied()
        .find(|v| for_game(v))
        .or_else(|| sorted.first().copied())?;
    let files = version["files"].as_array()?;
    let file = files
        .iter()
        .find(|f| f["primary"].as_bool() == Some(true))
        .or_else(|| files.first())?;
    Some(File {
        name: file["filename"].as_str()?.to_owned(),
        url: file["url"].as_str()?.to_owned(),
        size: file["size"].as_u64().unwrap_or(0),
        sha512: file["hashes"]["sha512"].as_str()?.to_ascii_lowercase(),
    })
}

fn sha512_hex(bytes: &[u8]) -> String {
    Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A plain file name for the pack: no folders, ending in `.zip`.
fn pack_name(name: &str) -> Option<String> {
    let name = name.rsplit(['/', '\\']).next()?.trim();
    if name.is_empty() || name.starts_with('.') {
        return None;
    }
    Some(if name.to_lowercase().ends_with(".zip") {
        name.to_owned()
    } else {
        format!("{name}.zip")
    })
}

/// Downloads project `id`'s file into `instance/resourcepacks/`, checks its SHA-512 and size, and
/// returns where it went. Nothing is kept when a check fails.
pub fn install(id: &str, instance: &Path) -> Result<PathBuf, String> {
    let versions = get_json(&format!("{API}/project/{}/version", encode(id)))?;
    let file = pick_file(&versions).ok_or("That pack has no file to download.")?;
    if file.size > MAX_FILE {
        return Err("That pack is bigger than the mobile launcher takes.".into());
    }
    let name = pack_name(&file.name).ok_or("The pack's file name is not allowed.")?;
    let response = agent()
        .get(&file.url)
        .call()
        .map_err(|_| "Could not download it. Check your internet connection.".to_string())?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "The download was interrupted.".to_string())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err("The download is bigger than expected.".into());
    }
    if file.size != 0 && bytes.len() as u64 != file.size {
        return Err("The download is not the size Modrinth lists: nothing was installed.".into());
    }
    if sha512_hex(&bytes) != file.sha512 {
        return Err("The download failed its SHA-512 check: nothing was installed.".into());
    }
    let dir = instance.join("resourcepacks");
    fs::create_dir_all(&dir).map_err(|e| format!("Could not make {}: {e}", dir.display()))?;
    let temp = dir.join(format!(".{name}.part"));
    fs::write(&temp, &bytes).map_err(|e| format!("Could not write: {e}"))?;
    let dest = dir.join(&name);
    fs::rename(&temp, &dest).map_err(|e| format!("Could not install {name}: {e}"))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_search_asks_for_resource_packs_for_this_game() {
        let url = search_url("faithful 32x", 20, false);
        assert!(url.starts_with("https://api.modrinth.com/v2/search?facets="));
        assert!(url.contains(&encode(r#"["project_type:resourcepack"]"#)));
        assert!(url.contains(&encode(&format!("versions:{GAME}"))));
        assert!(url.contains("offset=20") && url.contains("limit=20"));
        assert!(url.contains("index=relevance") && url.contains("&query=faithful%2032x"));
        // No text: the most downloaded; "everything" drops the game filter.
        let all = search_url("", 0, true);
        assert!(all.contains("index=downloads") && !all.contains("query="));
        assert!(!all.contains(&encode("versions:")));
    }

    #[test]
    fn hits_are_parsed_and_other_kinds_dropped() {
        let answer = json!({"total_hits": 3, "hits": [
            {"project_id":"AAA","project_type":"resourcepack","title":"Faithful","description":"Vanilla, but sharper","author":"Faithful Team","downloads":12345},
            {"project_id":"BBB","project_type":"mod","title":"Not a pack"},
            {"project_type":"resourcepack","title":"No id"}
        ]});
        let hits = parse_hits(&answer);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0],
            Hit {
                id: "AAA".into(),
                title: "Faithful".into(),
                summary: "Vanilla, but sharper".into(),
                author: "Faithful Team".into(),
                downloads: 12345
            }
        );
    }

    #[test]
    fn the_file_comes_from_the_newest_version_for_this_game() {
        let versions = json!([
            {"status":"listed","date_published":"2026-01-01T00:00:00Z","game_versions":["26.3"],
             "files":[{"filename":"old.zip","url":"https://cdn/old.zip","size":3,"primary":true,"hashes":{"sha512":"AA"}}]},
            {"status":"listed","date_published":"2026-06-01T00:00:00Z","game_versions":["26.3"],
             "files":[{"filename":"extra.txt","url":"https://cdn/x","size":1,"primary":false,"hashes":{"sha512":"BB"}},
                      {"filename":"new.zip","url":"https://cdn/new.zip","size":5,"primary":true,"hashes":{"sha512":"CC"}}]},
            {"status":"listed","date_published":"2026-09-01T00:00:00Z","game_versions":["1.21"],
             "files":[{"filename":"other.zip","url":"https://cdn/o","size":9,"primary":true,"hashes":{"sha512":"DD"}}]},
            {"status":"draft","date_published":"2026-10-01T00:00:00Z","game_versions":["26.3"],
             "files":[{"filename":"draft.zip","url":"https://cdn/d","size":9,"primary":true,"hashes":{"sha512":"EE"}}]}
        ]);
        let f = pick_file(&versions).unwrap();
        assert_eq!(
            (f.name.as_str(), f.size, f.sha512.as_str()),
            ("new.zip", 5, "cc")
        );
        // With none for this game, the newest listed one.
        let only_other = json!([{"status":"listed","date_published":"2026-09-01","game_versions":["1.21"],
            "files":[{"filename":"o.zip","url":"u","size":1,"hashes":{"sha512":"ff"}}]}]);
        assert_eq!(pick_file(&only_other).unwrap().name, "o.zip");
        assert!(pick_file(&json!([])).is_none());
    }

    #[test]
    fn pack_names_are_plain_zip_files() {
        assert_eq!(pack_name("Faithful.zip").as_deref(), Some("Faithful.zip"));
        assert_eq!(pack_name("pack").as_deref(), Some("pack.zip"));
        assert_eq!(pack_name("../../evil.zip").as_deref(), Some("evil.zip"));
        assert_eq!(pack_name(".hidden.zip"), None);
        assert_eq!(pack_name("  "), None);
    }

    #[test]
    fn sha512_matches_the_known_digest() {
        assert!(sha512_hex(b"abc").starts_with("ddaf35a193617aba"));
    }
}
