//! Java mods (Fabric, NeoForge, Forge) from the browser (docs/mod-browser.md). Horizon doesn't
//! run their code; it ports what they carry (blocks, items, recipes, loot, tags, worldgen) with
//! `horizon-port-pack`. Before a mod is offered for install its jar is downloaded (checked as
//! every download is), carried over and its classes read (`horizon_port_pack::check`): the
//! verdict (Works, Partly, Doesn't run) and the lists of what ports and what doesn't come from
//! that, and are kept per file so a jar is checked once.
//!
//! Installing copies the carried-over folder mod into `horizon-mods/<mod id>/` beside
//! `options.txt` and turns it on in `horizon-mods.txt`, where the game's Options > Mods reads it.
use super::{installed::Entry, pick, FileRef, Kind, Project, Source, Support, Version};
use horizon_port_pack::check::{self, Check, Verdict};
use std::fs;
use std::path::{Path, PathBuf};

/// Where the checks are kept, under the game folder. Its name changes when the check finds
/// more, so older answers are checked again.
const CHECKS: &str = "mod-browser/checks-1";
/// The game's mod folder and on-list, beside `options.txt` (`horizon_client::mod_list`).
pub const MODS_DIR: &str = "horizon-mods";
const MODS_LIST: &str = "horizon-mods.txt";
/// How deep required mods are followed.
const MAX_DEPTH: usize = 3;

/// The loaders whose jars port-pack reads.
pub const LOADERS: [&str; 4] = ["neoforge", "fabric", "forge", "quilt"];

/// How well a checked mod runs.
pub fn support_of(check: &Check) -> Support {
    match check.verdict {
        Verdict::Works => Support::Works,
        Verdict::Partly => Support::Partial(check.summary.clone()),
        Verdict::No => Support::No(check.summary.clone()),
    }
}

fn checks(home: &Path) -> PathBuf {
    home.join(CHECKS)
}

/// The folder a file's check is kept in, by its checksum.
fn checked_dir(home: &Path, file: &FileRef) -> Option<PathBuf> {
    let (_, hash) = file.checksum()?;
    let hash: String = hash
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(40)
        .collect();
    (hash.len() >= 32).then(|| checks(home).join(hash))
}

fn read_check(dir: &Path) -> Option<Check> {
    let check: Check =
        serde_json::from_str(&fs::read_to_string(dir.join("check.json")).ok()?).ok()?;
    (!refused_id(&check)).then_some(check)
}

/// A `No` from before ids with `-` (Fabric's) carried over (#18616): checked again.
fn refused_id(check: &Check) -> bool {
    check.verdict == Verdict::No
        && check.summary.starts_with("Can't be carried over: mod id ")
        && check
            .summary
            .ends_with(" is not [a-z0-9_]{1,64}: rename it before carrying over")
}

/// Where an install of mod `id` is copied before it is moved into place.
fn staging(home: &Path, id: &str) -> PathBuf {
    home.join(MODS_DIR).join(format!(".{id}.part"))
}

/// A package the game finds (`mod_package::find`, in both mod folders: they share
/// `horizon-mods.txt`) whose manifest has mod id `id`, other than the install's own `target`
/// and its staging folder (an interrupted install leaves one, which the retry replaces).
fn id_in_use(home: &Path, id: &str, target: &Path) -> Option<PathBuf> {
    use horizon_platform::mod_package::{find, mod_list, Package};
    let part = staging(home, id);
    [mod_list::DIR, mod_list::NATIVE_DIR]
        .iter()
        .flat_map(|dir| find(&home.join(dir)))
        .filter(|path| path != target && *path != part)
        .find(|path| Package::read(path).is_ok_and(|package| package.manifest.id == id))
}

/// The check of `file` if it was made before.
pub fn cached(home: &Path, file: &FileRef) -> Option<Check> {
    read_check(&checked_dir(home, file)?)
}

/// Checks `file`: the cached answer, else its jar downloaded, carried over and read. The jar
/// itself isn't kept; the carry is, when something of it ports, for the install to copy.
pub fn check_file(
    home: &Path,
    source: &dyn Source,
    file: &FileRef,
    progress: &dyn Fn(f32),
) -> Result<Check, String> {
    let dir = checked_dir(home, file).ok_or("Its source gives no checksum.")?;
    if let Some(c) = read_check(&dir) {
        return Ok(c);
    }
    let root = checks(home);
    fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    // Old and new browse workers can select the same jar. Lock its cache entry, then
    // reread it so the second worker can reuse a verdict published while it waited.
    let lock_path = dir.with_extension("lock");
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    lock.lock()
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    if let Some(c) = read_check(&dir) {
        return Ok(c);
    }
    // Files with the same name and different checksums can still be checked in parallel.
    let download = tempfile::Builder::new()
        .prefix(".download-")
        .tempdir_in(&root)
        .map_err(|e| format!("{}: {e}", root.display()))?;
    let jar = source.fetch(file, download.path(), progress)?;
    let part = dir.with_extension("part");
    let _ = fs::remove_dir_all(&part);
    let result = check::check(&jar, &part);
    let c = result.inspect_err(|_| {
        let _ = fs::remove_dir_all(&part);
    })?;
    if c.verdict == Verdict::No {
        for d in ["carry", "outside"] {
            let _ = fs::remove_dir_all(part.join(d));
        }
    }
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&part, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(c)
}

/// The version of `project` to install and its check.
pub fn check_project(
    home: &Path,
    source: &dyn Source,
    project: &Project,
    everything: bool,
    progress: &dyn Fn(f32),
) -> Result<(Version, Check), String> {
    check_project_outcome(home, source, project, everything, progress).map_err(
        |error| match error {
            ProjectCheckError::NoEligibleFile(message) | ProjectCheckError::Failed(message) => {
                message
            }
        },
    )
}

#[derive(Debug)]
pub enum ProjectCheckError {
    /// The source answered successfully but has no downloadable version Horizon can use.
    NoEligibleFile(String),
    Failed(String),
}

/// As [`check_project`], with a separate outcome for a successful lookup without an eligible file.
pub fn check_project_outcome(
    home: &Path,
    source: &dyn Source,
    project: &Project,
    everything: bool,
    progress: &dyn Fn(f32),
) -> Result<(Version, Check), ProjectCheckError> {
    let versions = source
        .versions(project)
        .map_err(ProjectCheckError::Failed)?;
    let version = pick(Kind::Mod, &versions, everything)
        .cloned()
        .ok_or_else(|| {
            let reason = match versions.iter().find_map(|v| v.blocked.as_deref()) {
                Some(why) => format!("{} can't be downloaded from here: {why}", project.title),
                None => format!(
                    "{} has no Fabric, NeoForge or Forge version Horizon can port.",
                    project.title
                ),
            };
            ProjectCheckError::NoEligibleFile(reason)
        })?;
    let file = version.file().expect("pick keeps versions with a file");
    let check = check_file(home, source, file, progress).map_err(ProjectCheckError::Failed)?;
    Ok((version, check))
}

/// Whether mod `id` is on in `horizon-mods.txt`.
pub fn is_enabled(home: &Path, id: &str) -> bool {
    let key = format!("mod.{id}");
    fs::read_to_string(home.join(MODS_LIST))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once(':'))
        .any(|(k, v)| k.trim() == key && v.trim() == "true")
}

/// Turns mod `id` on (or off) in `horizon-mods.txt`, keeping the other lines.
pub fn set_enabled(home: &Path, id: &str, on: bool) -> Result<(), String> {
    set_all_enabled(home, &[id], on)
}

/// Turns every mod in `ids` on (or off) in one locked rewrite of `horizon-mods.txt`: either all
/// of them change or, when the write fails, none do.
pub fn set_all_enabled(home: &Path, ids: &[&str], on: bool) -> Result<(), String> {
    let _lock = super::lock_home(home).map_err(|e| e.to_string())?;
    let path = home.join(MODS_LIST);
    let text = fs::read_to_string(&path).unwrap_or_default();
    let keys: Vec<String> = ids.iter().map(|id| format!("mod.{id}")).collect();
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| {
            l.split_once(':')
                .is_none_or(|(k, _)| !keys.iter().any(|key| k.trim() == key))
        })
        .map(str::to_owned)
        .collect();
    if on {
        lines.extend(keys.iter().map(|key| format!("{key}:true")));
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        }
    }
    Ok(())
}

/// Installs the Java mod `project`: checked, carried over into `horizon-mods/<id>/` and turned
/// on, with the mods it requires that port too. A required mod with nothing to port (a library)
/// is listed in what was skipped. Returns the entries installed, the asked-for one last.
pub fn install(
    source: &dyn Source,
    project: &Project,
    everything: bool,
    home: &Path,
    progress: &dyn Fn(&str, f32),
) -> Result<Vec<Entry>, String> {
    let _lock = super::lock_home(home).map_err(|e| e.to_string())?;
    let job = Job {
        source,
        everything,
        home,
        progress,
    };
    let mut done = Vec::new();
    job.add(project, false, 0, &mut done)?;
    Ok(done)
}

/// One install: where from, where to, and how it reports.
struct Job<'a> {
    source: &'a dyn Source,
    everything: bool,
    home: &'a Path,
    progress: &'a dyn Fn(&str, f32),
}

impl Job<'_> {
    fn add(
        &self,
        project: &Project,
        dependency: bool,
        depth: usize,
        done: &mut Vec<Entry>,
    ) -> Result<(), String> {
        let Job {
            source,
            everything,
            home,
            progress,
        } = *self;
        if done.iter().any(|e| e.project == project.id) {
            return Ok(());
        }
        let (version, check) = check_project(home, source, project, everything, &|f| {
            progress(&format!("Checking {}", project.title), f)
        })?;
        if check.verdict == Verdict::No {
            return Err(format!(
                "{} doesn't run in Horizon: {}",
                project.title, check.summary
            ));
        }
        // Refused before any required mod goes in, and again before publishing: one of those
        // may have taken the id or a namespace.
        self.refusal(project, &check)?;
        let mut skipped = check.skipped.clone();
        for id in &version.required {
            let Ok(needed) = source.project(id) else {
                continue;
            };
            if needed.kind != Kind::Mod || depth >= MAX_DEPTH {
                continue;
            }
            match self.add(&needed, true, depth + 1, done) {
                Ok(()) => {}
                Err(_) if dependency => {}
                Err(why) => skipped.push(format!("needs {}: {why}", needed.title)),
            }
        }
        let file = version.file().expect("pick keeps versions with a file");
        let carry = checked_dir(home, file)
            .expect("checked above")
            .join("carry");
        let relative = format!("{MODS_DIR}/{}", check.mod_id);
        let target = home.join(&relative);
        let old = self.refusal(project, &check)?;
        progress(&format!("Installing {}", project.title), 1.);
        let part = staging(home, &check.mod_id);
        let _ = fs::remove_dir_all(&part);
        copy_dir(&carry, &part).inspect_err(|_| {
            let _ = fs::remove_dir_all(&part);
        })?;
        if let Some(old) = &old {
            if let Some(dir) = old.path(home) {
                let _ = fs::remove_dir_all(dir);
            }
            if old.file != relative {
                let _ = set_enabled(home, old.file.trim_start_matches("horizon-mods/"), false);
            }
        }
        fs::rename(&part, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        set_enabled(home, &check.mod_id, true)?;
        let (algo, hash) = file.checksum().expect("checked files have one");
        let entry = Entry {
            source: project.source,
            project: project.id.clone(),
            slug: project.slug.clone(),
            title: project.title.clone(),
            kind: Kind::Mod,
            version: version.id.clone(),
            number: version.number.clone(),
            file: relative,
            checksum: format!("{algo}:{hash}"),
            page: project.page.clone(),
            dependency: dependency && old.as_ref().is_none_or(|o| o.dependency),
            verdict: Some(check.verdict.label().to_owned()),
            skipped,
            namespaces: check.namespaces.clone(),
        };
        let mut installed = super::installed::Installed::load(home);
        installed
            .entries
            .retain(|e| e.source != project.source || e.project != project.id);
        installed.entries.push(entry.clone());
        installed.save(home)?;
        done.push(entry);
        Ok(())
    }

    /// Why `project`, checked as `check`, can't go in, else its own earlier install if any.
    fn refusal(&self, project: &Project, check: &Check) -> Result<Option<Entry>, String> {
        let home = self.home;
        let relative = format!("{MODS_DIR}/{}", check.mod_id);
        let installed = super::installed::Installed::load(home);
        let ours = |e: &Entry| e.source == project.source && e.project == project.id;
        if let Some(other) = installed.entries.iter().find(|e| {
            e.kind == Kind::Mod
                && !ours(e)
                && (e.file == relative || e.namespaces.iter().any(|n| check.namespaces.contains(n)))
        }) {
            return Err(format!(
                "{} shares its id or namespace with {}, installed already: remove one first.",
                project.title, other.title
            ));
        }
        let target = home.join(&relative);
        let old = installed.entries.iter().find(|e| ours(e)).cloned();
        if target.exists() && old.as_ref().is_none_or(|o| o.file != relative) {
            return Err(format!(
                "{relative} is already there and wasn't installed by the browser: move it away first."
            ));
        }
        // A Fabric id's `-` is `_` here: `a-b` would take the id of an installed `a_b.hmod`, which
        // the game refuses twice and whose `mod.a_b` setting removing this mod would clear.
        if let Some(other) = id_in_use(home, &check.mod_id, &target) {
            return Err(format!(
                "Another mod already uses the id {} ({}): remove it first to install {}.",
                check.mod_id,
                other.display(),
                project.title
            ));
        }
        Ok(old)
    }
}

/// Removes an installed mod's folder and turns it off.
pub fn remove(home: &Path, entry: &Entry) -> Result<(), String> {
    let _lock = super::lock_home(home).map_err(|e| e.to_string())?;
    let Some(id) = entry.file.strip_prefix("horizon-mods/") else {
        return Err(format!("{} isn't in {MODS_DIR}/.", entry.file));
    };
    match entry.path(home).map_or(Ok(()), fs::remove_dir_all) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", entry.file)),
    }
    set_enabled(home, id, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{SourceId, GAME};
    use sha2::{Digest, Sha512};
    use std::io::{Cursor, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    #[test]
    fn the_on_list_keeps_other_lines() {
        let home = std::env::temp_dir().join(format!("horizon-java-mods-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join(MODS_LIST),
            "mod.coords_hud:true\nmod.ruby:false\n",
        )
        .unwrap();
        set_enabled(&home, "ruby", true).unwrap();
        set_enabled(&home, "gem", true).unwrap();
        assert_eq!(
            fs::read_to_string(home.join(MODS_LIST)).unwrap(),
            "mod.coords_hud:true\nmod.ruby:true\nmod.gem:true\n"
        );
        set_enabled(&home, "ruby", false).unwrap();
        set_enabled(&home, "gem", false).unwrap();
        assert_eq!(
            fs::read_to_string(home.join(MODS_LIST)).unwrap(),
            "mod.coords_hud:true\n"
        );
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn checks_are_kept_by_checksum() {
        let home = Path::new("/h");
        let file = |sha512: Option<&str>, sha1: Option<&str>| FileRef {
            url: String::new(),
            name: "a.jar".into(),
            size: 1,
            sha512: sha512.map(str::to_owned),
            sha256: None,
            sha1: sha1.map(str::to_owned),
            primary: true,
        };
        let long = "ab".repeat(64);
        assert_eq!(
            checked_dir(home, &file(Some(&long), None)).unwrap(),
            home.join(CHECKS).join("ab".repeat(20))
        );
        assert!(checked_dir(home, &file(None, Some("../../etc"))).is_none());
        assert!(checked_dir(home, &file(None, None)).is_none());
    }

    fn project() -> Project {
        Project {
            source: SourceId::Modrinth,
            id: "fixture".into(),
            slug: "fixture".into(),
            title: "Fixture".into(),
            summary: String::new(),
            author: String::new(),
            downloads: 0,
            page: String::new(),
            icon: None,
            license: "MIT".into(),
            kind: Kind::Mod,
            game_versions: vec![GAME.into()],
            loaders: vec!["fabric".into()],
            blocked: None,
        }
    }

    fn version(files: Vec<FileRef>) -> Version {
        Version {
            id: "fixture-version".into(),
            number: "1.0".into(),
            published: String::new(),
            game_versions: vec![GAME.into()],
            loaders: vec!["fabric".into()],
            files,
            required: Vec::new(),
            blocked: None,
        }
    }

    fn jar_file(bytes: &[u8]) -> FileRef {
        FileRef {
            url: "https://cdn.modrinth.com/data/fixture/versions/1/fixture.jar".into(),
            name: "fixture.jar".into(),
            size: bytes.len() as u64,
            sha512: Some(format!("{:x}", Sha512::digest(bytes))),
            sha256: None,
            sha1: None,
            primary: true,
        }
    }

    struct FixtureSource {
        listed: Result<Vec<Version>, String>,
        download: Result<Vec<u8>, String>,
        simultaneous: Option<Barrier>,
        fetches: AtomicUsize,
    }

    impl Source for FixtureSource {
        fn id(&self) -> SourceId {
            SourceId::Modrinth
        }

        fn search(&self, _: &super::super::Query) -> Result<Vec<Project>, String> {
            Ok(vec![project()])
        }

        fn project(&self, _: &str) -> Result<Project, String> {
            Ok(project())
        }

        fn versions(&self, _: &Project) -> Result<Vec<Version>, String> {
            if let Some(barrier) = &self.simultaneous {
                barrier.wait();
            }
            self.listed.clone()
        }

        fn fetch(
            &self,
            file: &FileRef,
            folder: &Path,
            progress: &dyn Fn(f32),
        ) -> Result<PathBuf, String> {
            self.fetches.fetch_add(1, Ordering::Relaxed);
            if self.simultaneous.is_some() {
                // Leave time for the other worker to ask for the same uncached jar.
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let bytes = self.download.as_deref().map_err(Clone::clone)?;
            super::super::download::save_from_anywhere(file, bytes, folder, progress)
        }
    }

    #[test]
    fn unavailable_files_are_distinct_from_lookup_and_download_failures() {
        let home = tempfile::tempdir().unwrap();
        let project = project();
        let mut source = FixtureSource {
            listed: Ok(vec![version(Vec::new())]),
            download: Err("download timed out".into()),
            simultaneous: None,
            fetches: AtomicUsize::new(0),
        };
        let check = |source: &FixtureSource| {
            check_project_outcome(home.path(), source, &project, false, &|_| {})
        };
        assert!(matches!(
            check(&source),
            Err(ProjectCheckError::NoEligibleFile(_))
        ));
        source.listed = Err("lookup timed out".into());
        assert!(
            matches!(check(&source), Err(ProjectCheckError::Failed(e)) if e == "lookup timed out")
        );
        assert_eq!(source.fetches.load(Ordering::Relaxed), 0);

        source.listed = Ok(vec![version(vec![jar_file(b"fixture")])]);
        assert!(
            matches!(check(&source), Err(ProjectCheckError::Failed(e)) if e == "download timed out")
        );
        assert_eq!(source.fetches.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn concurrent_browse_modes_reuse_the_same_jar_check() {
        let home = tempfile::tempdir().unwrap();
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (path, bytes) in [
            (
                "fabric.mod.json",
                br#"{"schemaVersion":1,"id":"fixture","version":"1.0","name":"Fixture"}"#
                    .as_slice(),
            ),
            ("assets/fixture/models/item/sample.json", b"{}"),
        ] {
            zip.start_file(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let file = jar_file(&bytes);
        let source = FixtureSource {
            listed: Ok(vec![version(vec![file.clone()])]),
            download: Ok(bytes),
            simultaneous: Some(Barrier::new(2)),
            fetches: AtomicUsize::new(0),
        };
        let check =
            |everything| check_project(home.path(), &source, &project(), everything, &|_| {});
        std::thread::scope(|threads| {
            let compatible = threads.spawn(|| check(false));
            let all = threads.spawn(|| check(true));
            assert_eq!(
                compatible.join().unwrap().unwrap().1.verdict,
                Verdict::Works
            );
            assert_eq!(all.join().unwrap().unwrap().1.verdict, Verdict::Works);
        });
        assert_eq!(source.fetches.load(Ordering::Relaxed), 1);
        assert_eq!(cached(home.path(), &file).unwrap().verdict, Verdict::Works);
        assert!(!checked_dir(home.path(), &file)
            .unwrap()
            .with_extension("part")
            .exists());
    }
}
