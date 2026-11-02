//! Game builds the player imports: a `horizon-client` they already have (one built on their
//! fork's Actions, to try a merged change before the next Nightly), installed as its own
//! version beside the releases. The program alone can't start: it needs a version folder's
//! game files. So an import is a copy of an installed release or Nightly's folder (its
//! research-lock.json, preparer sources and prepared resources) with the picked program in
//! place of that version's own, as a native build is (`native_mods::install`).
//!
//! Nothing signed it: the Versions page says so. Its tag (`imported-<date>-<sha>`) is no
//! release's, so no update replaces it, prunes it or moves the selection off it.
use crate::backend::{self, CLIENT};
use crate::i18n::{tr, trf};
use crate::native_mods::{self, Target};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;

const PREFIX: &str = "imported-";
/// What an import was made from, in its folder.
const RECORD: &str = "imported.json";
/// The most a program may take: a game build is ~100 MB.
const MAX_PROGRAM: u64 = 1 << 30;

/// Whether an installed version is an imported build.
pub fn is_imported(version: &str) -> bool {
    version.starts_with(PREFIX)
}

/// An imported build as a player reads it: `Imported · 2026-11-01`, or None for any other version.
pub fn label(version: &str) -> Option<String> {
    let date = version.strip_prefix(PREFIX)?.get(..10)?;
    Some(trf("Imported · {date}", &[("date", date)]))
}

/// What an imported build was made from: its `imported.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct About {
    /// The release or Nightly whose game files it plays with.
    pub base: String,
    /// The file the player picked.
    pub file: String,
}

/// What installed imported build `version` was made from; None for any other version.
pub fn about(version: &str) -> Option<About> {
    if !is_imported(version) {
        return None;
    }
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(backend::version_dir(version).join(RECORD)).ok()?)
            .ok()?;
    let field = |name: &str| json[name].as_str().map(str::to_owned);
    Some(About {
        base: field("base")?,
        file: field("file")?,
    })
}

/// A version Horizon published and signed: a release or a Nightly.
fn official(version: &str) -> bool {
    backend::is_game_tag(version) || crate::nightly::date_of(version).is_some()
}

/// The installed version an import takes its game files from: `selected` when Horizon published
/// it, else the newest installed release or Nightly (`installed` is newest first).
pub fn base_for(installed: &[String], selected: Option<&str>) -> Option<String> {
    selected
        .filter(|s| official(s) && installed.iter().any(|t| t == s))
        .map(str::to_owned)
        .or_else(|| installed.iter().find(|t| official(t)).cloned())
}

/// Installs the game program at `path` (or the one inside a zip there) as a new version on the
/// game files of `base` (default: [`base_for`]), prepared from another version where it can be,
/// and selects it. Importing the same program again selects the version it made before.
pub fn import(path: &Path, base: Option<&str>) -> Result<String, String> {
    let file = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let program = read_program(path, &file)?;
    if !runs_here(&program) || horizon_mods::api_level::read_mark(&program).is_none() {
        return Err(trf(
            "{file} is not a Horizon game for this computer.",
            &[("file", &file)],
        ));
    }
    let installed = backend::installed();
    let selected = backend::Settings::load().selected;
    let base = match base {
        Some(b) if official(b) && installed.iter().any(|t| t == b) => b.to_owned(),
        Some(b) => return Err(format!("{b} is not an installed release or Nightly.")),
        None => base_for(&installed, selected.as_deref()).ok_or_else(|| {
            tr("Install a release or Nightly first: an imported build plays with its game files.")
                .to_string()
        })?,
    };
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let tag = install_in(&backend::version_dir(""), &base, &program, &file, &date)?;
    backend::reuse_prepared_for(&tag);
    // Read again: the window may have saved other settings during the copy.
    let mut settings = backend::Settings::load();
    settings.selected = Some(tag.clone());
    settings.save();
    Ok(tag)
}

/// The program at `path`: the file itself when it is named as the game is (`horizon-client`,
/// "horizon-client (1).exe" as a browser saves a second copy), or from a zip (a GitHub Actions
/// artifact), its one entry named as the game is on this platform (`CLIENT`), in any folder.
/// The launcher and horizon-ctl carry the mod API mark too: the name tells them apart.
fn read_program(path: &Path, file: &str) -> Result<Vec<u8>, String> {
    let not_a_game = || {
        trf(
            "{file} is not a Horizon game for this computer.",
            &[("file", file)],
        )
    };
    let open = fs::File::open(path).map_err(|e| format!("{file}: {e}"))?;
    let Ok(mut zip) = zip::ZipArchive::new(open) else {
        let size = fs::metadata(path)
            .map_err(|e| format!("{file}: {e}"))?
            .len();
        if !file.to_ascii_lowercase().starts_with("horizon-client") || size > MAX_PROGRAM {
            return Err(not_a_game());
        }
        return fs::read(path).map_err(|e| format!("{file}: {e}"));
    };
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("{file}: {e}"))?;
        let named = entry
            .enclosed_name()
            .is_some_and(|p| p.file_name() == Some(std::ffi::OsStr::new(CLIENT)));
        if named && entry.is_file() {
            let mut program = Vec::new();
            (&mut entry)
                .take(MAX_PROGRAM + 1)
                .read_to_end(&mut program)
                .map_err(|e| format!("{file}: {e}"))?;
            if program.len() as u64 > MAX_PROGRAM {
                return Err(not_a_game());
            }
            return Ok(program);
        }
    }
    Err(trf(
        "{file} holds no {program}.",
        &[("file", file), ("program", CLIENT)],
    ))
}

/// Whether `program` starts as a program of this platform does: PE on Windows, Mach-O on macOS,
/// ELF elsewhere. A Linux build picked on Windows would only fail to start.
fn runs_here(program: &[u8]) -> bool {
    let magic: &[&[u8]] = if cfg!(windows) {
        &[b"MZ"]
    } else if cfg!(target_os = "macos") {
        // 64-bit Mach-O, and a universal binary.
        &[&[0xcf, 0xfa, 0xed, 0xfe], &[0xca, 0xfe, 0xba, 0xbe]]
    } else {
        &[b"\x7fELF"]
    };
    magic.iter().any(|m| program.starts_with(m))
}

/// Installs `program` in `versions` as a copy of installed version `base` with it in place of
/// `base`'s game, and returns its tag. Copied beside it and renamed when complete, as an
/// install is: an interrupted import is never installed.
fn install_in(
    versions: &Path,
    base: &str,
    program: &[u8],
    file: &str,
    date: &str,
) -> Result<String, String> {
    let sha256: String = Sha256::digest(program)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let short = &sha256[..7];
    // The same program imported on another day is the version made then.
    let made = fs::read_dir(versions)
        .into_iter()
        .flatten()
        .flatten()
        .find_map(|e| {
            let name = e.file_name().into_string().ok()?;
            (is_imported(&name)
                && name.ends_with(&format!("-{short}"))
                && e.path().join(".installed").exists())
            .then_some(name)
        });
    if let Some(tag) = made {
        return Ok(tag);
    }
    let tag = format!("{PREFIX}{date}-{short}");
    let dir = versions.join(&tag);
    let partial = versions.join(format!(".{tag}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    let record = serde_json::json!({ "base": base, "file": file, "sha256": sha256 });
    let copied = fill(&versions.join(base), &partial, program, &record).and_then(|()| {
        let _ = fs::remove_dir_all(&dir);
        fs::rename(&partial, &dir)?;
        fs::write(dir.join(".installed"), "")
    });
    if let Err(e) = copied {
        let _ = fs::remove_dir_all(&partial);
        let _ = fs::remove_dir_all(&dir);
        return Err(trf(
            "Could not import the build: {error}",
            &[("error", &e.to_string())],
        ));
    }
    Ok(tag)
}

/// Fills the new version's folder `to`: version folder `base`'s files but its game and its
/// record (`native_mods::carried_over`, cloned where the disk can), `program` as the game, and
/// what it was made from.
fn fill(base: &Path, to: &Path, program: &[u8], record: &serde_json::Value) -> std::io::Result<()> {
    for entry in fs::read_dir(base)?.flatten() {
        let name = entry.file_name();
        if !native_mods::carried_over(&name, Target::Client) || name == RECORD {
            continue;
        }
        let target = to.join(&name);
        if entry.path().is_dir() {
            backend::copy_dir(&entry.path(), &target)?;
        } else {
            horizon_prepare::clone_or_copy(&entry.path(), &target)?;
            fs::set_permissions(&target, entry.metadata()?.permissions())?;
        }
    }
    fs::write(to.join(CLIENT), program)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(to.join(CLIENT), fs::Permissions::from_mode(0o755))?;
    }
    fs::write(to.join(RECORD), record.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("horizon-import-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// A program as this platform's are, carrying the mod API mark the game's do.
    fn program(body: &[u8]) -> Vec<u8> {
        let magic: &[u8] = if cfg!(windows) {
            b"MZ"
        } else if cfg!(target_os = "macos") {
            &[0xcf, 0xfa, 0xed, 0xfe]
        } else {
            b"\x7fELF"
        };
        [magic, body, &horizon_mods::api_level::MARK[..]].concat()
    }

    #[test]
    fn an_import_is_its_own_version_on_its_bases_game_files() {
        let versions = dir("install");
        let base = versions.join("nightly-2026-11-01-c9d474d");
        fs::create_dir_all(base.join(".local/horizon-client/26.3")).unwrap();
        fs::write(
            base.join(".local/horizon-client/26.3/horizon-resources.json"),
            b"{}",
        )
        .unwrap();
        fs::write(base.join("research-lock.json"), b"lock").unwrap();
        fs::write(base.join(CLIENT), b"signed").unwrap();
        fs::write(base.join(".installed"), b"").unwrap();
        fs::create_dir_all(base.join("Horizon.app")).unwrap();
        let built = program(b"fork build");
        let tag = install_in(
            &versions,
            "nightly-2026-11-01-c9d474d",
            &built,
            CLIENT,
            "2026-11-01",
        )
        .unwrap();
        assert!(
            tag.starts_with("imported-2026-11-01-") && tag.len() == 27,
            "{tag}"
        );
        assert!(is_imported(&tag) && !official(&tag));
        assert_eq!(label(&tag).as_deref(), Some("Imported · 2026-11-01"));
        let dir = versions.join(&tag);
        assert_eq!(fs::read(dir.join(CLIENT)).unwrap(), built);
        assert_eq!(fs::read(base.join(CLIENT)).unwrap(), b"signed");
        assert!(dir.join(".installed").is_file());
        assert!(dir
            .join(".local/horizon-client/26.3/horizon-resources.json")
            .is_file());
        assert_eq!(fs::read(dir.join("research-lock.json")).unwrap(), b"lock");
        // The macOS app is made again from the imported program (game_app.rs).
        assert!(!dir.join("Horizon.app").exists());
        let record: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join(RECORD)).unwrap()).unwrap();
        assert_eq!(record["base"], "nightly-2026-11-01-c9d474d");
        // The same program again is the same version, left as it is.
        let again = install_in(
            &versions,
            "nightly-2026-11-01-c9d474d",
            &built,
            "x",
            "2026-11-02",
        );
        assert_eq!(again.unwrap(), tag);
        assert_eq!(fs::read_dir(&versions).unwrap().count(), 2);
        let _ = fs::remove_dir_all(versions);
    }

    #[test]
    fn only_a_horizon_game_for_this_computer_is_imported() {
        assert!(runs_here(&program(b"")));
        let other = if cfg!(windows) { b"\x7fELF" } else { b"MZ\0\0" };
        assert!(!runs_here(other));
        let d = dir("zip");
        let zip_path = d.join("horizon-client-windows-x86_64.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("README.md", options).unwrap();
        std::io::Write::write_all(&mut zip, b"notes").unwrap();
        zip.start_file(format!("horizon/{CLIENT}"), options)
            .unwrap();
        std::io::Write::write_all(&mut zip, &program(b"zipped")).unwrap();
        zip.finish().unwrap();
        assert_eq!(
            read_program(&zip_path, "a.zip").unwrap(),
            program(b"zipped")
        );
        let name = "horizon-client (1)";
        let plain = d.join(name);
        fs::write(&plain, program(b"plain")).unwrap();
        assert_eq!(read_program(&plain, name).unwrap(), program(b"plain"));
        // The launcher carries the mark too.
        let launcher = d.join("horizon-launcher");
        fs::write(&launcher, program(b"launcher")).unwrap();
        assert!(read_program(&launcher, "horizon-launcher").is_err());
        let empty = d.join("empty.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&empty).unwrap());
        zip.start_file("README.md", options).unwrap();
        zip.finish().unwrap();
        assert!(read_program(&empty, "empty.zip").is_err());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn the_base_is_the_selected_release_or_the_newest_one() {
        let installed = [
            "nightly-2026-11-01-c9d474d".to_string(),
            "v0.2.95".to_string(),
            "native-c9d474d-0123456789ab".to_string(),
            "imported-2026-10-31-abcdef0".to_string(),
        ];
        assert_eq!(
            base_for(&installed, Some("v0.2.95")).as_deref(),
            Some("v0.2.95")
        );
        for selected in [None, Some("imported-2026-10-31-abcdef0"), Some("v0.1.0")] {
            assert_eq!(
                base_for(&installed, selected).as_deref(),
                Some("nightly-2026-11-01-c9d474d")
            );
        }
        assert_eq!(base_for(&installed[2..], None), None);
    }
}
