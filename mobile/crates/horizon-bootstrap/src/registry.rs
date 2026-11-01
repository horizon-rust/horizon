//! Horizon's mod registry, for the mobile launcher's Mods page: what it lists for mods, resource
//! packs and shader packs, installing one into an instance (the download is checked against the
//! index's size, SHA-256, signature and the file's own manifest by `horizon_mods`), and the
//! files an instance already has.

use horizon_mods::index::{self, Index, Kind, Project, Version};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const INDEX_URL: &str = "https://horizon-rust.github.io/horizon-mods/index.json";
/// The Minecraft version Horizon runs.
pub const GAME: &str = "26.3";

/// What the Mods page has tabs for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Mods,
    /// Rust mods compiled into the game (listed here; building them needs a computer).
    NativeMods,
    ResourcePacks,
    Shaders,
}

impl Sort {
    pub const ALL: [Sort; 4] = [
        Sort::Mods,
        Sort::NativeMods,
        Sort::ResourcePacks,
        Sort::Shaders,
    ];
    /// Native mods can't be installed by the mobile launcher.
    pub fn installable_here(self) -> bool {
        self != Sort::NativeMods
    }
    pub fn label(self) -> &'static str {
        match self {
            Sort::Mods => "Mods",
            Sort::NativeMods => "Native mods",
            Sort::ResourcePacks => "Resource packs",
            Sort::Shaders => "Shader packs",
        }
    }
    /// The folder of an instance its files go in.
    pub fn folder(self) -> &'static str {
        match self {
            Sort::Mods => "horizon-mods",
            Sort::NativeMods => "horizon-native-mods",
            Sort::ResourcePacks => "resourcepacks",
            Sort::Shaders => "shaderpacks",
        }
    }
    fn kind(self) -> Kind {
        match self {
            Sort::Mods => Kind::Mod,
            Sort::NativeMods => Kind::NativeMod,
            Sort::ResourcePacks => Kind::ResourcePack,
            Sort::Shaders => Kind::Shader,
        }
    }
    fn extension(self) -> &'static str {
        self.kind().extension().unwrap_or("zip")
    }
}

/// A project of the registry, as a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub version: String,
    pub size: u64,
    /// Its file is already in the instance.
    pub installed: bool,
    pub authors: String,
    pub downloads: u64,
}

pub fn fetch_index() -> Result<Index, String> {
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .user_agent("horizon-mobile")
        .try_proxy_from_env(true)
        .build()
        .get(INDEX_URL)
        .call()
        .map_err(|_| "Could not load the mod list. Check your internet connection.".to_string())?;
    let mut text = String::new();
    response
        .into_reader()
        .take(32 << 20)
        .read_to_string(&mut text)
        .map_err(|_| "The mod list was interrupted.".to_string())?;
    Index::parse(&text).map_err(|e| format!("The mod list is not usable: {e}"))
}

/// The projects of `sort` that have a version for this game, newest first by name order of the index.
pub fn entries(index: &Index, sort: Sort, instance: &Path) -> Vec<Entry> {
    let have = installed(instance, sort);
    let kind = sort.kind();
    index
        .projects
        .iter()
        .filter(|p| p.kind == kind)
        .filter_map(|p| {
            let v = index.installable(p, GAME, horizon_mods::api_level::LEVEL)?;
            Some(Entry {
                id: p.id.clone(),
                name: p.name.clone(),
                summary: p.summary.clone(),
                version: v.version.clone(),
                size: v.file.size,
                installed: have.iter().any(|f| f == &v.file.name),
                authors: p.authors.join(", "),
                downloads: p.downloads,
            })
        })
        .collect()
}

/// Moves `.hmod` files an older build put in `mods/` to `horizon-mods/`, where the game reads them.
fn migrate_old(instance: &Path) {
    let old = instance.join("mods");
    let Ok(rd) = fs::read_dir(&old) else { return };
    let new = instance.join("horizon-mods");
    for e in rd.flatten() {
        let n = e.file_name();
        if n.to_string_lossy().to_lowercase().ends_with(".hmod") {
            let _ = fs::create_dir_all(&new);
            let _ = fs::rename(e.path(), new.join(&n));
        }
    }
}

/// Turns a mod on in the game's `horizon-mods.txt` (`mod.<id>:true`), as the Mods screen does.
fn enable(instance: &Path, id: &str) {
    let path = instance.join("horizon-mods.txt");
    let text = fs::read_to_string(&path).unwrap_or_default();
    let key = format!("mod.{id}:");
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !l.starts_with(&key))
        .map(str::to_owned)
        .collect();
    lines.push(format!("{key}true"));
    let _ = fs::write(path, lines.join("\n") + "\n");
}

/// The files of `sort` in `instance`, by name.
pub fn installed(instance: &Path, sort: Sort) -> Vec<String> {
    migrate_old(instance);
    let ext = sort.extension();
    let mut names: Vec<String> = fs::read_dir(instance.join(sort.folder()))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            n.to_lowercase().ends_with(&format!(".{ext}"))
                || (sort == Sort::ResourcePacks && n.ends_with(".zip"))
        })
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Removes an installed file of `sort` from `instance`. The name must be a plain file name.
pub fn remove(instance: &Path, sort: Sort, name: &str) -> Result<(), String> {
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return Err("Not a file of this instance.".into());
    }
    fs::remove_file(instance.join(sort.folder()).join(name))
        .map_err(|e| format!("Could not remove {name}: {e}"))
}

fn download(url: &str, size: u64) -> Result<Vec<u8>, String> {
    if let Some(path) = url.strip_prefix("file://") {
        return fs::read(path).map_err(|e| format!("Could not read {path}: {e}"));
    }
    let response = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(60))
        .user_agent("horizon-mobile")
        .try_proxy_from_env(true)
        .build()
        .get(url)
        .call()
        .map_err(|_| "Could not download it. Check your internet connection.".to_string())?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(index::MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "The download was interrupted.".to_string())?;
    if size != 0 && bytes.len() as u64 > index::MAX_FILE {
        return Err("The file is bigger than the registry allows.".into());
    }
    Ok(bytes)
}

fn version_of<'a>(index: &'a Index, id: &str) -> Result<(&'a Project, &'a Version), String> {
    let project = index
        .project(id)
        .ok_or("That project is not in the list any more.")?;
    let version = index
        .installable(project, GAME, horizon_mods::api_level::LEVEL)
        .ok_or("It has no version for this game that this Horizon can run.")?;
    Ok((project, version))
}

/// Downloads project `id` into `instance`'s folder for `sort`, checks it as the desktop launcher
/// does (nothing is kept when a check fails), and returns the installed file's path.
pub fn install(index: &Index, id: &str, sort: Sort, instance: &Path) -> Result<PathBuf, String> {
    if !sort.installable_here() {
        return Err(
            "Native mods are compiled into the game itself, which needs a computer.".into(),
        );
    }
    let (project, version) = version_of(index, id)?;
    if project.kind != sort.kind() {
        return Err("That is not the kind of file this tab installs.".into());
    }
    let name = &version.file.name;
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return Err("The list gives the file a name that is not allowed.".into());
    }
    let dir = instance.join(sort.folder());
    fs::create_dir_all(&dir).map_err(|e| format!("Could not make {}: {e}", dir.display()))?;
    let bytes = download(&version.file.url, version.file.size)?;
    let temp = dir.join(format!(".{name}.part"));
    fs::write(&temp, &bytes).map_err(|e| format!("Could not write: {e}"))?;
    if let Err(e) = index::check_file(index, project, version, &temp) {
        let _ = fs::remove_file(&temp);
        return Err(format!("{} was refused: {e}", project.name));
    }
    let dest = dir.join(name);
    fs::rename(&temp, &dest).map_err(|e| format!("Could not install {name}: {e}"))?;
    if sort == Sort::Mods {
        enable(instance, id);
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("hb-reg-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn the_tabs_have_their_folders() {
        assert_eq!(Sort::Mods.folder(), "horizon-mods");
        assert_eq!(Sort::ResourcePacks.folder(), "resourcepacks");
        assert_eq!(Sort::Shaders.folder(), "shaderpacks");
        assert_eq!(Sort::Mods.extension(), "hmod");
        assert_eq!(Sort::NativeMods.extension(), "hnmod");
        assert!(!Sort::NativeMods.installable_here() && Sort::Mods.installable_here());
        assert_eq!(Sort::ResourcePacks.extension(), "zip");
    }

    #[test]
    fn native_mods_are_listed_but_never_installed() {
        let inst = temp("c");
        assert!(
            install(&Index::default(), "anything", Sort::NativeMods, &inst)
                .unwrap_err()
                .contains("computer")
        );
        let _ = fs::remove_dir_all(inst);
    }

    #[test]
    fn installed_files_are_listed_by_kind_and_can_be_removed() {
        let inst = temp("a");
        // Old builds put mods in `mods/`; listing moves them to the folder the game reads.
        fs::create_dir_all(inst.join("mods")).unwrap();
        fs::create_dir_all(inst.join("resourcepacks")).unwrap();
        fs::write(inst.join("mods/Zed.hmod"), "x").unwrap();
        fs::write(inst.join("mods/alpha.hmod"), "x").unwrap();
        fs::write(inst.join("mods/readme.txt"), "x").unwrap();
        fs::write(inst.join("resourcepacks/pack.zip"), "x").unwrap();
        assert_eq!(installed(&inst, Sort::Mods), ["alpha.hmod", "Zed.hmod"]);
        assert_eq!(installed(&inst, Sort::ResourcePacks), ["pack.zip"]);
        assert!(installed(&inst, Sort::Shaders).is_empty());
        assert!(remove(&inst, Sort::Mods, "../x").is_err());
        assert!(remove(&inst, Sort::Mods, ".hidden").is_err());
        remove(&inst, Sort::Mods, "Zed.hmod").unwrap();
        assert_eq!(installed(&inst, Sort::Mods), ["alpha.hmod"]);
        let _ = fs::remove_dir_all(inst);
    }

    #[test]
    fn an_empty_index_lists_nothing_and_install_refuses_unknown_projects() {
        let index = Index::default();
        let inst = temp("b");
        assert!(entries(&index, Sort::Mods, &inst).is_empty());
        assert!(install(&index, "nothing", Sort::Mods, &inst).is_err());
        let _ = fs::remove_dir_all(inst);
    }
}
