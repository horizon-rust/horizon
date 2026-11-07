//! Prebuilt native builds (#11859): the game compiled for one exact set of registry native mods by
//! horizon-mods' CI, so players download it instead of compiling.
//!
//! A build is named by its request (`horizon_release_sign::native_build::Request`): the target, the
//! commit of the release or Nightly it starts from, and each mod's id, version and the SHA-256 of
//! its `.hnmod`. [`request_for`] computes it the same way on the player's computer and in CI.
//! [`export`] is CI's side: `horizon-launcher native-build --prebuilt-out DIR FOLDER` builds the
//! `.hnmod` files in FOLDER with the player's pipeline (the same pins, hook points and left-out
//! mods), and writes the program, its `native-mods.json` and the request into DIR, for the workflow
//! to sign. A set with a post-build step is never exported: the step runs on the player's game files.

use crate::native_mods::{self, LeftOut, Profile, Step};
use horizon_release_sign::native_build::{Mod, Request};
use std::fs;
use std::path::{Path, PathBuf};

/// The request's text in an export's folder: what the workflow checks against its own.
pub const REQUEST_FILE: &str = "request.txt";

/// The `.hnmod` files directly in `dir`, by name.
pub fn hnmod_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "hnmod"))
        .collect();
    files.sort();
    Ok(files)
}

/// The request for building `files` (registry `.hnmod`s, each checked as the registry checks it)
/// for `target` on commit `base`: each mod's id and version from its `horizon-native.json`, and the
/// file's SHA-256.
pub fn request_for(target: &str, base: &str, files: &[PathBuf]) -> Result<Request, String> {
    let mods = files
        .iter()
        .map(|path| {
            let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let bundle = horizon_mods::index::native::Bundle::read(path)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Mod {
                id: bundle.manifest.id,
                version: bundle.manifest.version,
                sha256: horizon_release_sign::sha256_hex(&bytes),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Request::new(target, base, mods)
}

/// What [`export`] made.
#[derive(Debug)]
pub struct Exported {
    pub request: Request,
    /// The version a player's own build of this set would install (`native-<commit>-<key>`).
    pub version: String,
    /// The mods the build left out, as a player's build leaves them out.
    pub left_out: Vec<LeftOut>,
}

/// Builds every `.hnmod` in `folder` on installed version `base` as a player's launcher would,
/// and writes into `out` (a new folder): the program, its `native-mods.json` and [`REQUEST_FILE`].
/// Nothing is installed and `base` needs no prepared game files.
pub fn export(
    base: &str,
    folder: &Path,
    out: &Path,
    profile: Profile,
    progress: impl Fn(Step),
) -> Result<Exported, String> {
    let files = hnmod_files(folder)?;
    if files.is_empty() {
        return Err(format!("{} holds no .hnmod file.", folder.display()));
    }
    let empty = fs::read_dir(out).map_or(true, |mut dir| dir.next().is_none());
    if !empty {
        return Err(format!(
            "{} already holds files: export into a new folder.",
            out.display()
        ));
    }
    let request = request_for(
        native_mods::build_target(base)?,
        &native_mods::revision_of(base)?,
        &files,
    )?;
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    // Unpacked as a player's installed mods are (`native_mods::installed_mods`), so the build's
    // record names them alike.
    let incoming = native_mods::root().join("prebuilt.incoming");
    let _ = fs::remove_dir_all(&incoming);
    for path in &files {
        let bundle = horizon_mods::index::native::Bundle::read(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        native_mods::unpack(&bundle, &incoming)?;
    }
    let built = native_mods::build_prebuilt(base, &incoming, out, profile, progress);
    let _ = fs::remove_dir_all(&incoming);
    let (version, left_out) = built?;
    fs::write(out.join(REQUEST_FILE), request.text()).map_err(|e| e.to_string())?;
    Ok(Exported {
        request,
        version,
        left_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("native-prebuilt-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A registry `.hnmod` for `id` at `version`, as `.hnmod` files are zipped.
    fn hnmod(dir: &Path, file: &str, id: &str, version: &str) -> PathBuf {
        hnmod_with(dir, file, id, version, "")
    }

    /// [`hnmod`] with `extra` at the end of its manifest.
    fn hnmod_with(dir: &Path, file: &str, id: &str, version: &str, extra: &str) -> PathBuf {
        let pin = "a".repeat(40);
        let path = dir.join(file);
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let manifest = serde_json::json!({
            "schema": 1, "id": id, "version": version, "game": "26.3", "horizon": pin, "mixins": pin
        });
        let source = format!(
            "[mod]\nid = \"{id}\"\nversion = \"{version}\"\n[compatibility]\nhorizon_revision = \"{pin}\"\n{extra}"
        );
        for (name, text) in [
            ("horizon-native.json".to_string(), manifest.to_string()),
            (format!("mixins/{id}/horizon-mixin.toml"), source),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn sha256(path: &Path) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(fs::read(path).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn a_request_names_each_file_by_its_manifest_and_its_bytes() {
        let dir = scratch("request");
        // The file name is not the id: the manifest says what the mod is.
        let zoom = hnmod(&dir, "b.hnmod", "zoom", "2.0.0");
        let minimap = hnmod(&dir, "a-renamed.hnmod", "minimap", "1.4.0");
        fs::write(dir.join("notes.txt"), "not a mod").unwrap();
        let files = hnmod_files(&dir).unwrap();
        assert_eq!(files, vec![minimap.clone(), zoom.clone()]);
        let base = "6".repeat(40);
        let request = request_for("x86_64-unknown-linux-gnu", &base, &files).unwrap();
        let expected = Request::new(
            "x86_64-unknown-linux-gnu",
            &base,
            vec![
                Mod {
                    id: "zoom".into(),
                    version: "2.0.0".into(),
                    sha256: sha256(&zoom),
                },
                Mod {
                    id: "minimap".into(),
                    version: "1.4.0".into(),
                    sha256: sha256(&minimap),
                },
            ],
        )
        .unwrap();
        assert_eq!(request, expected);
        // Another file for the same mod and version is another build.
        let other = scratch("request-other");
        let edited = hnmod_with(&other, "b.hnmod", "zoom", "2.0.0", "# edited\n");
        fs::copy(&minimap, other.join("a.hnmod")).unwrap();
        let changed = request_for(
            "x86_64-unknown-linux-gnu",
            &base,
            &hnmod_files(&other).unwrap(),
        )
        .unwrap();
        assert_ne!(sha256(&edited), sha256(&zoom));
        assert_ne!(changed.key(), request.key());
        fs::remove_dir_all(dir).unwrap();
        fs::remove_dir_all(other).unwrap();
    }

    #[test]
    fn a_file_the_registry_would_refuse_names_no_build() {
        let dir = scratch("refused");
        fs::write(dir.join("broken.hnmod"), b"not a zip").unwrap();
        let files = hnmod_files(&dir).unwrap();
        assert!(request_for("x86_64-unknown-linux-gnu", &"6".repeat(40), &files).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_export_needs_mods_and_a_new_folder_before_anything_runs() {
        let mods = scratch("export-mods");
        let out = scratch("export-out");
        let nothing = export("v0.0.1", &mods, &out.join("new"), Profile::Release, |_| {});
        assert!(nothing.unwrap_err().contains("no .hnmod"));
        hnmod(&mods, "zoom.hnmod", "zoom", "2.0.0");
        fs::write(out.join("left-over"), "").unwrap();
        let taken = export("v0.0.1", &mods, &out, Profile::Release, |_| {});
        assert!(taken.unwrap_err().contains("already holds files"));
        fs::remove_dir_all(mods).unwrap();
        fs::remove_dir_all(out).unwrap();
    }
}
