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
use horizon_release_sign::native_build::{self, Mod, Request};
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

/// The repository whose releases hold the prebuilt builds.
pub const REGISTRY: &str = "horizon-rust/horizon-mods";
/// The largest archive downloaded (horizon-mods' native-build.yml signs none larger): checked
/// before the download and capped during it, so a bad asset costs at most this much disk and no
/// memory before its signature is checked.
const MAX_ARCHIVE: u64 = 512 << 20;
/// The largest file unpacked from a checked archive.
const MAX_UNPACKED: u64 = 1 << 30;

/// Whether a build of an instance's mods first looks for a prebuilt one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Download it when the registry has one, else compile.
    Auto,
    /// Always compile on this computer.
    #[default]
    Never,
    /// Download it, or fail: never compile.
    Only,
}

impl Mode {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "never" => Some(Self::Never),
            "only" => Some(Self::Only),
            _ => None,
        }
    }

    /// The player's setting (`settings native-prebuilt on|off`): off unless turned on, until the
    /// registry's signing pipeline has been checked end to end (#11859); turning it on by default
    /// is its own change.
    pub fn chosen(settings: &crate::backend::Settings) -> Self {
        match settings.native_prebuilt {
            Some(true) => Self::Auto,
            _ => Self::Never,
        }
    }
}

/// The request for the native mods turned on in instance home `home`, built on `commit` for
/// `target`, when every one of them is a registry file `index` lists (by its SHA-256) and none
/// was taken down; None when one isn't (a mod from GitHub or a folder is built here).
pub fn instance_request(
    home: &Path,
    target: &str,
    commit: &str,
    index: &horizon_mods::index::Index,
) -> Result<Option<Request>, String> {
    let mut files = Vec::new();
    for path in hnmod_files(&home.join(native_mods::INSTALLED))? {
        let bundle = horizon_mods::index::native::Bundle::read(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if crate::browser::java::is_enabled(home, &bundle.manifest.id) {
            files.push(path);
        }
    }
    if files.is_empty() {
        return Ok(None);
    }
    let request = request_for(target, commit, &files)?;
    let listed = request.mods.iter().all(|m| {
        index.revoked(&m.id, &m.sha256).is_none()
            && index.project(&m.id).is_some_and(|p| {
                p.versions.iter().any(|v| {
                    v.version == m.version && v.file.sha256.eq_ignore_ascii_case(&m.sha256)
                })
            })
    });
    Ok(listed.then_some(request))
}

/// Where a prebuilt asset of `request` is: its release of [`REGISTRY`].
pub fn asset_url(request: &Request, asset: &str) -> String {
    format!(
        "https://github.com/{REGISTRY}/releases/download/{}/{asset}",
        native_build::release_tag(&request.base)
    )
}

/// A checked prebuilt build, unpacked: its program and record.
#[derive(Debug)]
pub struct Unpacked {
    pub binary: PathBuf,
    pub about: serde_json::Value,
}

/// Checks a downloaded build of `request` and unpacks it into `into`: `keys` signed `archive`
/// (`signatures` is its signature file) as this request's build; it holds only the program, its
/// `native-mods.json` and `request.txt`; that request is this one; and its record builds no mod
/// the request doesn't name. Anything else is refused before the program is written anywhere.
pub fn check(
    request: &Request,
    archive: &Path,
    signatures: &str,
    keys: &[horizon_release_sign::PublicKey],
    into: &Path,
) -> Result<Unpacked, String> {
    use std::io::Read;
    let key = request.key();
    let name = native_build::archive(&key, &request.target);
    let read_error = |e: std::io::Error| format!("{}: {e}", archive.display());
    // Hashed from disk first: nothing unsigned is read into memory.
    let size = fs::metadata(archive).map_err(read_error)?.len();
    if size > MAX_ARCHIVE {
        return Err(format!("{name}: larger than {MAX_ARCHIVE} bytes"));
    }
    let sha = sha256_file(archive).map_err(read_error)?;
    native_build::verify(&key, &request.target, &sha, &name, signatures, keys)?;
    // The bytes unpacked are the bytes the signature covers: hashed again once in memory.
    let bytes = fs::read(archive).map_err(read_error)?;
    if horizon_release_sign::sha256_hex(&bytes) != sha {
        return Err(format!("{name} changed while it was checked."));
    }
    let client = crate::backend::CLIENT;
    let mut files: std::collections::BTreeMap<String, Vec<u8>> = Default::default();
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(bytes.as_slice()));
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let path = path.to_string_lossy().trim_start_matches("./").to_string();
        if entry.header().entry_type().is_dir() && path.is_empty() {
            continue;
        }
        let known = [client, "native-mods.json", REQUEST_FILE].contains(&path.as_str());
        if !known || !entry.header().entry_type().is_file() || files.contains_key(&path) {
            return Err(format!("{name}: unexpected entry {path:?}"));
        }
        let mut data = Vec::new();
        (&mut entry)
            .take(MAX_UNPACKED)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        files.insert(path, data);
    }
    let text = |file: &str| {
        files
            .get(file)
            .and_then(|b| std::str::from_utf8(b).ok())
            .ok_or_else(|| format!("{name}: no {file}"))
    };
    if text(REQUEST_FILE)? != request.text() {
        return Err(format!("{name} was built for another set of mods."));
    }
    let about: serde_json::Value =
        serde_json::from_str(text("native-mods.json")?).map_err(|e| format!("{name}: {e}"))?;
    let asked = |id: &str, version: &str| {
        request
            .mods
            .iter()
            .any(|m| m.id == id && m.version == version)
    };
    let built = about["mods"].as_array().ok_or(format!("{name}: no mods"))?;
    let named = built.iter().all(|m| {
        asked(
            m["id"].as_str().unwrap_or_default(),
            m["version"].as_str().unwrap_or_default(),
        )
    });
    if built.is_empty() || !named || about["horizon"].as_str().is_none_or(|h| h.len() != 40) {
        return Err(format!("{name}: its record doesn't match its request"));
    }
    let program = files.remove(client).ok_or(format!("{name}: no {client}"))?;
    fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let binary = into.join(client);
    fs::write(&binary, program).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    Ok(Unpacked { binary, about })
}

/// The SHA-256 of the file at `path`, read in pieces.
fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    std::io::copy(&mut fs::File::open(path)?, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Streams `url` to the file `to`, at most [`MAX_ARCHIVE`] bytes (one announced as larger is
/// refused before its body); None when there is no such file.
fn fetch(url: &str, to: &Path, progress: &impl Fn(Step)) -> Result<Option<()>, String> {
    use std::io::{Read, Write};
    let response = match crate::backend::agent(url).get(url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(format!("{url}: {e}")),
    };
    let total: Option<u64> = response
        .header("Content-Length")
        .and_then(|l| l.parse().ok());
    if total.is_some_and(|t| t > MAX_ARCHIVE) {
        return Err(format!("{url}: larger than {MAX_ARCHIVE} bytes"));
    }
    let mut reader = response.into_reader().take(MAX_ARCHIVE);
    let mut file = fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let (mut done, mut buffer) = (0u64, vec![0; 1 << 16]);
    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| format!("{url}: {e}"))?;
        if n == 0 {
            return Ok(Some(()));
        }
        file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if let Some(total) = total.filter(|t| *t > 0) {
            progress(Step::Prebuilt(done as f32 / total as f32));
        }
    }
}

/// Installs the registry's prebuilt build of `request` on installed version `base`, as the
/// version a build of these mods would be, after [`check`]ing it. None when the registry has
/// no build of this set; an error when one exists but fails a check.
pub fn install(
    base: &str,
    request: &Request,
    progress: &impl Fn(Step),
) -> Result<Option<String>, String> {
    let key = request.key();
    // Named by the request alone, so a set switched off and on again plays the kept build.
    let version = format!(
        "{}{}-{}",
        native_mods::PREFIX,
        &request.base[..7],
        &key[..12]
    );
    if native_mods::reuse_installed(&version) {
        return Ok(Some(version));
    }
    let signature = native_build::signature(&key, &request.target);
    let signature_url = asset_url(request, &signature);
    let signatures = match crate::backend::agent(&signature_url)
        .get(&signature_url)
        .call()
    {
        Ok(r) => r.into_string().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let work = native_mods::root().join("prebuilt");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let archive = work.join(native_build::archive(&key, &request.target));
    progress(Step::Prebuilt(0.0));
    let url = asset_url(request, &native_build::archive(&key, &request.target));
    let result = fetch(&url, &archive, progress).and_then(|fetched| {
        if fetched.is_none() {
            return Ok(None);
        }
        let mut unpacked = check(
            request,
            &archive,
            &signatures,
            &native_build::trusted_keys(),
            &work.join("unpacked"),
        )?;
        unpacked.about["base"] = serde_json::json!(base);
        unpacked.about["prebuilt"] = serde_json::json!({ "key": key, "url": url });
        progress(Step::Install);
        native_mods::install_prebuilt(base, &version, &unpacked.binary, &unpacked.about)?;
        Ok(Some(version))
    });
    // A failed or refused download leaves nothing behind either.
    let _ = fs::remove_dir_all(&work);
    result
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

    /// A signed prebuilt archive for `request`, as native-build.yml makes and signs one.
    fn signed_archive(
        dir: &Path,
        request: &Request,
        files: &[(&str, &[u8])],
    ) -> (PathBuf, String, horizon_release_sign::PublicKey) {
        let (signer, _) = horizon_release_sign::SigningKey::generate().unwrap();
        let key = request.key();
        let name = native_build::archive(&key, &request.target);
        let path = dir.join(&name);
        let gz = flate2::write::GzEncoder::new(
            fs::File::create(&path).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gz);
        for (file, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, format!("./{file}"), *bytes)
                .unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        let sha = horizon_release_sign::sha256_hex(&fs::read(&path).unwrap());
        let message = native_build::message(&key, &request.target, &sha, &name);
        (path, signer.sign_bytes(&message) + "\n", signer.public())
    }

    fn minimap_request() -> Request {
        Request::new(
            "x86_64-unknown-linux-gnu",
            &"6".repeat(40),
            vec![Mod {
                id: "minimap".into(),
                version: "2.0.0".into(),
                sha256: "a".repeat(64),
            }],
        )
        .unwrap()
    }

    fn record(mods: &[(&str, &str)]) -> Vec<u8> {
        let mods: Vec<_> = mods
            .iter()
            .map(|(id, v)| serde_json::json!({"id": id, "version": v, "sha256": "f".repeat(64)}))
            .collect();
        serde_json::json!({"base": "nightly-x", "horizon": "6".repeat(40), "mods": mods, "left_out": []})
            .to_string()
            .into_bytes()
    }

    #[test]
    fn a_signed_build_of_this_set_unpacks_and_nothing_else_does() {
        let dir = scratch("check");
        let request = minimap_request();
        let client = crate::backend::CLIENT;
        let text = request.text();
        let good = record(&[("minimap", "2.0.0")]);
        let files: [(&str, &[u8]); 3] = [
            (client, b"the game"),
            ("native-mods.json", &good),
            (REQUEST_FILE, text.as_bytes()),
        ];
        let (archive, signature, public) = signed_archive(&dir, &request, &files);
        let into = dir.join("unpacked");
        let unpacked = check(&request, &archive, &signature, &[public], &into).unwrap();
        assert_eq!(fs::read(&unpacked.binary).unwrap(), b"the game");
        assert_eq!(unpacked.about["mods"][0]["id"], "minimap");

        // Signed by a key the launcher doesn't list.
        let (stranger, _) = horizon_release_sign::SigningKey::generate().unwrap();
        assert!(check(&request, &archive, &signature, &[stranger.public()], &into).is_err());
        // The same archive asked for as another set.
        let other = Request::new(
            &request.target,
            &request.base,
            vec![Mod {
                id: "zoom".into(),
                version: "2.0.0".into(),
                sha256: "b".repeat(64),
            }],
        )
        .unwrap();
        assert!(check(&other, &archive, &signature, &[public], &into).is_err());
        // One byte changed after signing.
        let mut bytes = fs::read(&archive).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&archive, bytes).unwrap();
        assert!(check(&request, &archive, &signature, &[public], &into).is_err());
        // Larger than any signed build: refused before it is read, nothing unpacked.
        let huge = dir.join("huge");
        fs::File::create(&archive)
            .unwrap()
            .set_len(MAX_ARCHIVE + 1)
            .unwrap();
        assert!(check(&request, &archive, &signature, &[public], &huge).is_err());
        assert!(!huge.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prebuilt_builds_are_off_until_the_player_turns_them_on() {
        let mut settings = crate::backend::Settings::default();
        assert_eq!(Mode::chosen(&settings), Mode::Never);
        settings.native_prebuilt = Some(false);
        assert_eq!(Mode::chosen(&settings), Mode::Never);
        settings.native_prebuilt = Some(true);
        assert_eq!(Mode::chosen(&settings), Mode::Auto);
    }

    #[test]
    fn a_signed_archive_that_lies_about_its_contents_is_refused() {
        let request = minimap_request();
        let client = crate::backend::CLIENT;
        let text = request.text();
        let good = record(&[("minimap", "2.0.0")]);
        let extra = record(&[("minimap", "2.0.0"), ("xray", "1.0.0")]);
        let other_text = text.replace("2.0.0", "2.0.1");
        let cases: [Vec<(&str, &[u8])>; 4] = [
            // A file besides the three.
            vec![
                (client, b"g"),
                ("native-mods.json", &good),
                (REQUEST_FILE, text.as_bytes()),
                ("run-me.sh", b"#!/bin/sh"),
            ],
            // A record naming a mod the request doesn't.
            vec![
                (client, b"g"),
                ("native-mods.json", &extra),
                (REQUEST_FILE, text.as_bytes()),
            ],
            // Another set's request inside, signed as this one's build.
            vec![
                (client, b"g"),
                ("native-mods.json", &good),
                (REQUEST_FILE, other_text.as_bytes()),
            ],
            // No program.
            vec![("native-mods.json", &good), (REQUEST_FILE, text.as_bytes())],
        ];
        for (i, files) in cases.iter().enumerate() {
            let dir = scratch(&format!("lies-{i}"));
            let (archive, signature, public) = signed_archive(&dir, &request, files);
            let into = dir.join("unpacked");
            assert!(
                check(&request, &archive, &signature, &[public], &into).is_err(),
                "case {i}"
            );
            assert!(!into.join(client).exists(), "case {i}: nothing is written");
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn only_an_instance_of_registry_files_asks_for_a_prebuilt_build() {
        let home = scratch("instance");
        let installed = home.join(native_mods::INSTALLED);
        fs::create_dir_all(&installed).unwrap();
        let zoom = hnmod(&installed, "zoom.hnmod", "zoom", "2.0.0");
        let local = hnmod(&installed, "mine.hnmod", "mine", "0.1.0");
        let index = |listed: &[(&str, &Path)], revoked: &str| {
            let projects: Vec<_> = listed
                .iter()
                .map(|(id, path)| serde_json::json!({
                    "id": id, "kind": "native-mod", "name": id, "summary": "", "license": "MIT",
                    "source": "https://x.test", "versions": [{"version": "2.0.0", "api": "native-1",
                    "game": "26.3", "file": {"name": format!("{id}.hnmod"), "url": "file:///x",
                    "size": fs::metadata(path).unwrap().len(), "sha256": sha256(path)}}]
                }))
                .collect();
            let text = serde_json::json!({"schema": 1, "projects": projects,
                "revoked": if revoked.is_empty() { serde_json::json!([]) }
                           else { serde_json::json!([{"id": revoked, "reason": "r", "date": "2026-11-07"}]) }});
            horizon_mods::index::Index::parse_local(&text.to_string()).unwrap()
        };
        let base = "6".repeat(40);
        let ask = |index: &horizon_mods::index::Index| {
            instance_request(&home, "x86_64-unknown-linux-gnu", &base, index).unwrap()
        };
        // Nothing turned on: nothing to ask for.
        assert_eq!(ask(&index(&[("zoom", &zoom)], "")), None);
        crate::browser::java::set_enabled(&home, "zoom", true).unwrap();
        let request = ask(&index(&[("zoom", &zoom)], "")).unwrap();
        assert_eq!(request.mods.len(), 1);
        assert_eq!(request.mods[0].sha256, sha256(&zoom));
        // Taken down, or not the file the index lists: built here.
        assert_eq!(ask(&index(&[("zoom", &zoom)], "zoom")), None);
        assert_eq!(ask(&index(&[("zoom", &local)], "")), None);
        // A mod of one's own turned on beside it: built here.
        crate::browser::java::set_enabled(&home, "mine", true).unwrap();
        assert_eq!(ask(&index(&[("zoom", &zoom)], "")), None);
        fs::remove_dir_all(home).unwrap();
    }
}
