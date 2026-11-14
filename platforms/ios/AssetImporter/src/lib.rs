//! Nova's private, offline installer for USER-SUPPLIED, prepared Horizon files.
//! Contains no assets or download code. Import is transactional: extraction to
//! a fresh sibling directory, validation, then replacement of the installed set.
//!
//! Resource loading into Horizon is a separate, not-yet-implemented milestone.

use std::{
    collections::HashSet,
    error::Error,
    fmt,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use zip::ZipArchive;

const MAX_ENTRIES: usize = 150_000;
const MAX_TOTAL: u64 = 6 * 1024 * 1024 * 1024;
const MAX_FILE: u64 = 1024 * 1024 * 1024;
const MAX_MANIFEST: u64 = 2 * 1024 * 1024;

#[derive(Debug)]
pub struct InstallError(pub String);

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}
impl Error for InstallError {}
impl From<io::Error> for InstallError {
    fn from(e: io::Error) -> Self { Self(format!("File system: {e}")) }
}

fn fail(message: impl Into<String>) -> InstallError { InstallError(message.into()) }

/// Absolute paths, backslashes, drive paths, and traversal are never accepted.
/// ZIP directory entries may end with '/', but no empty middle components.
fn components(name: &str) -> Result<Vec<&str>, InstallError> {
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.is_empty() || trimmed.starts_with('/') || trimmed.contains('\\') || trimmed.contains('\0') {
        return Err(fail("Unsafe path inside archive"));
    }
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains(':')) {
        return Err(fail("Unsafe archive path component"));
    }
    Ok(parts)
}

fn is_symlink(mode: Option<u32>) -> bool {
    mode.is_some_and(|mode| mode & 0o170000 == 0o120000)
}

/// Check for the prepared root at either the archive root, or one enclosing
/// directory (such as "26.3/"). Don't accept nested unrelated archives.
fn find_prepared_root<R: Read + io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Vec<String>, InstallError> {
    if archive.len() > MAX_ENTRIES { return Err(fail("Too many archive entries")); }
    let mut roots = HashSet::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| fail(format!("ZIP entry: {e}")))?;
        let parts = components(entry.name())?;
        if is_symlink(entry.unix_mode()) { return Err(fail("Symbolic links are not allowed")); }
        if entry.is_dir() { continue; }
        if parts.last() == Some(&"horizon-resources.json") && parts.len() <= 2 {
            roots.insert(parts[..parts.len() - 1].iter().map(|s| s.to_string()).collect::<Vec<_>>());
        }
    }
    if roots.len() != 1 {
        return Err(fail("Expected one horizon-resources.json at ZIP root or one enclosing folder"));
    }
    Ok(roots.into_iter().next().unwrap())
}

fn prepare<R: Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    destination: &Path,
) -> Result<(), InstallError> {
    let root = find_prepared_root(archive)?;
    let mut seen = HashSet::new();
    let mut total = 0_u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| fail(format!("ZIP entry: {e}")))?;
        let parts = components(entry.name())?;
        if is_symlink(entry.unix_mode()) { return Err(fail("ZIP contains a symlink")); }
        if !parts.starts_with(&root.iter().map(String::as_str).collect::<Vec<_>>()) {
            continue; // metadata outside the prepared resource root is ignored.
        }
        let remainder = &parts[root.len()..];
        if remainder.is_empty() { continue; }
        let key = remainder.join("/").to_lowercase();
        if !seen.insert(key) { return Err(fail("ZIP contains duplicate resource paths")); }
        let mut out = PathBuf::from(destination);
        for part in remainder { out.push(part); }
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if entry.size() > MAX_FILE { return Err(fail("ZIP contains an oversized file")); }
        total = total.checked_add(entry.size()).ok_or_else(|| fail("ZIP size overflow"))?;
        if total > MAX_TOTAL { return Err(fail("ZIP expands beyond the 6 GiB safety limit")); }
        if out.file_name().and_then(|n| n.to_str()) == Some("horizon-resources.json")
            && remainder.len() == 1 && entry.size() > MAX_MANIFEST
        {
            return Err(fail("Resource manifest is too large"));
        }
        if let Some(parent) = out.parent() { fs::create_dir_all(parent)?; }
        let mut output = File::create(&out)?;
        // Even if the ZIP has incorrect size metadata, do not stream past it.
        let max = entry.size() + 1;
        let written = io::copy(&mut entry.take(max), &mut output)?;
        if written != max - 1 { return Err(fail("ZIP entry size mismatch")); }
        output.flush()?;
    }
    validate_resources(destination)
}

/// Structural checks only. Actual game/version compatibility is NOT proven.
pub fn validate_resources(root: &Path) -> Result<(), InstallError> {
    let manifest = root.join("horizon-resources.json");
    let metadata = fs::metadata(&manifest).map_err(|_| fail("Missing horizon-resources.json"))?;
    if metadata.len() == 0 || metadata.len() > MAX_MANIFEST { return Err(fail("Invalid manifest size")); }
    let bytes = fs::read(&manifest)?;
    if !matches!(serde_json::from_slice::<serde_json::Value>(&bytes),
        Ok(serde_json::Value::Object(_)))
    {
        return Err(fail("Resource manifest must be a JSON object"));
    }
    for file in ["facts.json", "registries.json"] {
        if !root.join(file).is_file() { return Err(fail(format!("Missing required {file}"))); }
    }
    let minecraft = root.join("assets").join("minecraft");
    if !minecraft.is_dir() || fs::read_dir(minecraft)?.next().is_none() {
        return Err(fail("Missing assets/minecraft directory or resources"));
    }
    Ok(())
}

/// Install prepared resources inside app-owned storage. An invalid ZIP never
/// replaces a previously working resource directory.
pub fn install_zip(source: &Path, destination: &Path) -> Result<(), InstallError> {
    let file = File::open(source)?;
    let mut archive = ZipArchive::new(file).map_err(|e| fail(format!("Invalid ZIP: {e}")))?;
    let parent = destination.parent().ok_or_else(|| fail("Resource destination needs a parent"))?;
    fs::create_dir_all(parent)?;
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|_| fail("Clock unavailable"))?.as_nanos();
    let temporary = parent.join(format!(".nova-resources-install-{}-{unique}", std::process::id()));
    fs::create_dir(&temporary)?;
    let result = (|| {
        prepare(&mut archive, &temporary)?;
        // A same-filesystem rename makes successfully prepared resources
        // visible together. Restore the prior install if the final rename fails.
        let backup = parent.join(format!(".nova-resources-backup-{}-{unique}", std::process::id()));
        let had_prior = destination.exists();
        if had_prior { fs::rename(destination, &backup)?; }
        match fs::rename(&temporary, destination) {
            Ok(()) => {
                if had_prior { let _ = fs::remove_dir_all(&backup); }
                Ok(())
            }
            Err(e) => {
                if had_prior { let _ = fs::rename(&backup, destination); }
                Err(e.into())
            }
        }
    })();
    if result.is_err() { let _ = fs::remove_dir_all(&temporary); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::{write::FileOptions, CompressionMethod, ZipWriter};

    fn sample(path: &Path, wrapper: Option<&str>, bad: bool) {
        let file = File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        let p = wrapper.map(|w| format!("{w}/")).unwrap_or_default();
        for (name, contents) in [
            ("horizon-resources.json", if bad { "not-json" } else { "{\"version\":\"26.3\"}" }),
            ("facts.json", "{}"),
            ("registries.json", "{}"),
            ("assets/minecraft/lang/en_us.json", "{}"),
        ] {
            zip.start_file(format!("{p}{name}"), FileOptions::default()
                .compression_method(CompressionMethod::Deflated)).unwrap();
            zip.write_all(contents.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn installs_flat_and_wrapped_prepared_resources() {
        let t = tempfile::tempdir().unwrap();
        for wrapper in [None, Some("26.3")] {
            let src = t.path().join("game.zip");
            sample(&src, wrapper, false);
            let dest = t.path().join("Resources");
            install_zip(&src, &dest).unwrap();
            validate_resources(&dest).unwrap();
            assert!(!dest.join("26.3").exists());
        }
    }

    #[test]
    fn invalid_import_preserves_existing_resources() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("game.zip");
        let dest = t.path().join("Resources");
        sample(&src, None, false);
        install_zip(&src, &dest).unwrap();
        sample(&src, None, true);
        assert!(install_zip(&src, &dest).is_err());
        validate_resources(&dest).unwrap();
    }

    #[test]
    fn paths_cannot_escape_or_use_windows_separators() {
        for name in ["../escape", "/root", "good/../escape", "C:/drive", "a\\b", "a//b"] {
            assert!(components(name).is_err(), "{name}");
        }
        assert!(is_symlink(Some(0o120777)));
    }

    #[test]
    fn malicious_entry_fails_without_writing_outside_destination() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("bad.zip");
        sample(&src, None, false);
        let file = File::create(&src).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file("../outside.txt", FileOptions::default()).unwrap();
        zip.write_all(b"bad").unwrap();
        zip.finish().unwrap();
        assert!(install_zip(&src, &t.path().join("Resources")).is_err());
        assert!(!t.path().join("outside.txt").exists());
    }
}
