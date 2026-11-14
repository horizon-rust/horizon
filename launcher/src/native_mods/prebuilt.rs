//! The Nightly's compiled crates (docs/native-mods.md, "Compilation cache"): the first native build
//! of a Nightly's commit would compile every crate of the game, so the launcher fetches what the
//! Nightly's build already compiled, `native-cache-<triple>-fast.tar.zst`, and unpacks it as the
//! `fast` folder of `native/target/`. Cargo then recompiles only what the mods change.
//!
//! The archive is listed in the release's signed SHA256SUMS, checked the way the game's own
//! binaries are. Nothing here may fail a build: whatever goes wrong, the build runs as it would
//! have without the archive.
use super::{append_log, files, Step};
use crate::rust_toolchain::Toolchain;
use crate::{backend, signed};
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

/// The archive's `native-cache.json` format this launcher reads.
const FORMAT: u64 = 1;

/// The most the archive may unpack to: a bomb must not fill the disk.
const UNPACKED_LIMIT: u64 = 24 << 30;

/// 2001-01-01: older than any crate the archive's build compiled, so Cargo takes a file the stage
/// still has from this commit's source for one it already compiled.
const OLD: Duration = Duration::from_secs(978_307_200);

/// Whether `folder` (a profile's folder in `native/target/`) holds compiled crates. A cleanup
/// ([`crate::native_cache::Cache::trim`]) empties it but leaves the folder and Cargo's lock, so a
/// folder that is there says nothing about whether anything was built.
pub(super) fn has_output(folder: &Path) -> bool {
    fs::read_dir(folder).is_ok_and(|mut entries| {
        entries.any(|entry| entry.map_or(true, |e| e.file_name() != ".cargo-lock"))
    })
}

/// Puts the Nightly's compiled crates into `target_dir/fast` and `stage` in the state Cargo trusts
/// them in, when this build starts from nothing in the `fast` folder and its commit is the
/// Nightly `origin`'s. True when it did. `fresh` is the source with the mods applied, which the
/// stage is about to be synced from: its `Cargo.lock` is what the build locks to. Called before
/// that sync, so only the files the mods change get a new time.
#[allow(clippy::too_many_arguments)]
pub(super) fn install(
    rust: &Toolchain,
    origin: &str,
    revision: &str,
    source: &Path,
    fresh: &Path,
    stage: &Path,
    target_dir: &Path,
    progress: &impl Fn(Step),
) -> bool {
    match try_install(
        rust, origin, revision, source, fresh, stage, target_dir, progress,
    ) {
        Ok(used) => used,
        Err(why) => {
            let _ = append_log(&format!(
                "The Nightly's compiled crates were not used, building them here: {why}"
            ));
            false
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn try_install(
    rust: &Toolchain,
    origin: &str,
    revision: &str,
    source: &Path,
    fresh: &Path,
    stage: &Path,
    target_dir: &Path,
    progress: &impl Fn(Step),
) -> Result<bool, String> {
    let fast = target_dir.join("fast");
    if has_output(&fast) {
        return Ok(false);
    }
    // Only the Nightly's own commit: its archive holds what that commit compiles to.
    if !horizon_release_sign::is_nightly_build(origin) {
        return Ok(false);
    }
    let commit = origin.get(19..).unwrap_or_default();
    if commit.is_empty() || !revision.starts_with(commit) {
        return Ok(false);
    }
    let triple = rust.host.target;
    let asset = format!("native-cache-{triple}-fast.tar.zst");
    // The Nightly is under `nightly` while it is the newest, else under its date's tag. Its
    // signature names its exact version, so a tag holding another Nightly is refused, not used.
    let date = origin.get(8..18).unwrap_or_default();
    let keys = horizon_release_sign::trusted_keys();
    let mut found = Err(String::new());
    for tag in ["nightly".to_owned(), format!("nightly-{date}")] {
        found = signed::nightly_digest(&tag, origin, &asset, &keys, backend::fetch_small)
            .map(|sha256| (tag, sha256));
        if found.is_ok() {
            break;
        }
    }
    let (tag, sha256) = found.map_err(|why| format!("{asset}: {why}"))?;

    let work = super::root().join("prebuilt.partial");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let used = unpacked(
        rust, &tag, &asset, &sha256, revision, triple, fresh, &work, progress,
    )
    .and_then(|folder| place(&folder, source, stage, &fast));
    let _ = fs::remove_dir_all(&work);
    used.map(|()| true)
}

/// Downloads and unpacks the archive into `work`, and checks what it says it is: the folder that
/// holds its `fast/` and `native-cache.json`.
#[allow(clippy::too_many_arguments)]
fn unpacked(
    rust: &Toolchain,
    tag: &str,
    asset: &str,
    sha256: &str,
    revision: &str,
    triple: &str,
    fresh: &Path,
    work: &Path,
    progress: &impl Fn(Step),
) -> Result<PathBuf, String> {
    progress(Step::Prebuilt(0.0));
    let archive = work.join("download");
    backend::download(
        &signed::asset_url(tag, asset),
        0,
        sha256,
        &archive,
        &|done| progress(Step::Prebuilt(done)),
    )?;
    let folder = work.join("unpacked");
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let file = fs::File::open(&archive).map_err(|e| e.to_string())?;
    let decoder = ruzstd::decoding::StreamingDecoder::new(BufReader::new(file))
        .map_err(|e| format!("{asset} is not a zstd file: {e}"))?;
    let mut tar = tar::Archive::new(decoder.take(UNPACKED_LIMIT));
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| format!("Download interrupted: {e}"))?;
        let kind = entry.header().entry_type();
        // Links are left out, as in the source's own archive.
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        // The files keep the times they were compiled at: Cargo goes by them.
        entry
            .unpack_in(&folder)
            .map_err(|e| format!("Could not unpack {asset}: {e}"))?;
    }
    let manifest = fs::read(folder.join("native-cache.json"))
        .map_err(|_| format!("{asset} has no native-cache.json"))?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest)
        .map_err(|e| format!("{asset}'s native-cache.json: {e}"))?;
    let text = |name: &str| {
        manifest[name]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    if manifest["format"].as_u64() != Some(FORMAT) {
        return Err(format!("{asset} is of a format this launcher doesn't read"));
    }
    if text("commit") != revision {
        return Err(format!("{asset} is of commit {}", text("commit")));
    }
    if text("triple") != triple || text("profile") != "fast" {
        return Err(format!(
            "{asset} is for {} ({})",
            text("triple"),
            text("profile")
        ));
    }
    // The crates are only good for the exact compiler that made them.
    let ours = rustc_text(rust)?;
    if text("rustc") != ours.trim() {
        return Err("it was compiled with another Rust than this launcher's".into());
    }
    let lock = fs::read(fresh.join("Cargo.lock"))
        .map_err(|e| format!("{}: {e}", fresh.join("Cargo.lock").display()))?;
    if !text("cargo_lock_sha256").eq_ignore_ascii_case(&horizon_release_sign::sha256_hex(&lock)) {
        return Err("this build's Cargo.lock is not the one it was compiled with".into());
    }
    if !folder.join("fast").join("deps").is_dir() {
        return Err(format!("{asset} holds no fast/deps"));
    }
    Ok(folder)
}

/// `rustc -vV` of the launcher's toolchain: what the archive's manifest holds for its own.
fn rustc_text(rust: &Toolchain) -> Result<String, String> {
    let rustc = rust
        .dir
        .join("bin")
        .join(format!("rustc{}", std::env::consts::EXE_SUFFIX));
    let out = rust
        .command(&rustc)
        .arg("-vV")
        .output()
        .map_err(|e| format!("rustc doesn't start: {e}"))?;
    if !out.status.success() {
        return Err("rustc -vV failed".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Writes `source`'s files into `stage` with an old time each, then moves the unpacked crates into
/// `fast`. The stage is written from this commit's source whatever it held (it may be another
/// commit's, with files of an old time whose content differs, which Cargo would never compile
/// again): only the files the mods change are written after it, with the current time
/// ([`super::sync`]).
fn place(unpacked: &Path, source: &Path, stage: &Path, fast: &Path) -> Result<(), String> {
    let old = UNIX_EPOCH + OLD;
    let _ = fs::remove_dir_all(stage);
    let written = files(source).and_then(|all| {
        for (name, path) in all {
            let target = stage.join(&name);
            fs::create_dir_all(target.parent().unwrap_or(stage))
                .and_then(|()| fs::copy(&path, &target))
                .and_then(|_| fs::File::options().write(true).open(&target))
                .and_then(|file| file.set_modified(old))
                .map_err(|e| format!("{}: {e}", target.display()))?;
        }
        Ok(())
    });
    if let Err(why) = written {
        let _ = fs::remove_dir_all(stage);
        return Err(why);
    }
    fs::create_dir_all(fast).map_err(|e| format!("{}: {e}", fast.display()))?;
    let mut moved = Vec::new();
    let entries = fs::read_dir(unpacked.join("fast")).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let to = fast.join(entry.file_name());
        let _ = fs::remove_dir_all(&to);
        let _ = fs::remove_file(&to);
        if let Err(e) = fs::rename(entry.path(), &to) {
            for done in moved {
                let _ = fs::remove_dir_all(done);
            }
            return Err(format!("{}: {e}", to.display()));
        }
        moved.push(to);
    }
    Ok(())
}
