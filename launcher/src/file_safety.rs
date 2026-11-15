//! Small file helpers for the launcher's backend: owner-only and unique temporary files, a lock-free
//! cache read, and the Mojang server jar checked against its SHA-1.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A new file at `path` (any old one there removed first) only its owner can read, holding `bytes`.
pub(crate) fn write_owner_only(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let _ = fs::remove_file(path);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    io::Write::write_all(&mut options.open(path)?, bytes)
}

/// Where to copy `name` into `dir` before renaming it into place: its own name per copy, so two
/// threads completing the same file never write into the one the other just renamed.
pub(crate) fn unique_partial(dir: &Path, name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COPIES: AtomicUsize = AtomicUsize::new(0);
    let n = COPIES.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".{name}.partial-{}-{n}", std::process::id()))
}

/// `key`'s value in `cache`, else `read`'s, kept. `read` runs without the lock: it may be slow (a
/// whole program read), and another key's reader must not wait for it. Two first lookups at once
/// only read twice.
pub(crate) fn cached<T: Copy>(
    cache: &Mutex<Vec<(String, T)>>,
    key: &str,
    read: impl FnOnce() -> T,
) -> T {
    let lock = || cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&(_, value)) = lock().iter().find(|(k, _)| k == key) {
        return value;
    }
    let value = read();
    let mut entries = lock();
    if !entries.iter().any(|(k, _)| k == key) {
        entries.push((key.to_owned(), value));
    }
    value
}

/// The `version` server jar at `jar`, as the version's `manifest` names it: downloaded with `get`
/// unless the cached copy has the SHA-1 the manifest lists, and refused when the download doesn't.
/// A bad copy (a captive portal's page, a cut-off body) is fetched again, not kept failing every
/// Prepare.
pub(crate) fn server_jar(
    jar: &Path,
    manifest: &Path,
    version: &str,
    log: &dyn Fn(String),
    get: &dyn Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(manifest).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let expected = json["downloads"]["server"]["sha1"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let matches = |bytes: &[u8]| expected.is_empty() || crate::sources::sha1_hex(bytes) == expected;
    if fs::read(jar).is_ok_and(|bytes| matches(&bytes)) {
        return Ok(());
    }
    let url = json["downloads"]["server"]["url"]
        .as_str()
        .ok_or_else(|| format!("{} has no server download", manifest.display()))?;
    log(format!("Downloading the {version} server jar from Mojang"));
    if let Some(dir) = jar.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let body = get(url)?;
    if !matches(&body) {
        return Err(format!(
            "The {version} server jar from {url} doesn't match its SHA-1. Try again."
        ));
    }
    let partial = jar.with_extension("partial");
    fs::write(&partial, body)
        .and_then(|_| fs::rename(&partial, jar))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-file-safety-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_server_jar_is_kept_only_when_it_is_the_one_named() {
        let dir = scratch("jar");
        let (jar, manifest) = (dir.join("jars/1.0.jar"), dir.join("1.0.json"));
        let good = b"the real jar".to_vec();
        let sha1 = crate::sources::sha1_hex(&good);
        fs::write(
            &manifest,
            format!(r#"{{"downloads":{{"server":{{"url":"https://x/s.jar","sha1":"{sha1}"}}}}}}"#),
        )
        .unwrap();
        let fetched = std::cell::Cell::new(0);
        let serve = |body: Vec<u8>| move |_: &str| -> Result<Vec<u8>, String> { Ok(body.clone()) };
        let log = |_: String| {};
        // A captive portal's page: refused, nothing kept.
        assert!(server_jar(&jar, &manifest, "1.0", &log, &serve(b"<html>".to_vec())).is_err());
        assert!(!jar.exists());
        // The real one: kept; then not fetched again.
        server_jar(&jar, &manifest, "1.0", &log, &serve(good.clone())).unwrap();
        let count = |_: &str| -> Result<Vec<u8>, String> {
            fetched.set(fetched.get() + 1);
            Ok(good.clone())
        };
        server_jar(&jar, &manifest, "1.0", &log, &count).unwrap();
        assert_eq!(fetched.get(), 0);
        // A bad copy already there is fetched again.
        fs::write(&jar, b"cut off").unwrap();
        server_jar(&jar, &manifest, "1.0", &log, &count).unwrap();
        assert_eq!((fetched.get(), fs::read(&jar).unwrap()), (1, good));
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_files_and_unique_partials() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("owner");
        let path = dir.join("settings.tmp");
        fs::write(&path, b"old").unwrap();
        write_owner_only(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
        assert_ne!(
            unique_partial(&dir, "a.json"),
            unique_partial(&dir, "a.json")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cached_value_is_read_once() {
        let cache = Mutex::new(Vec::new());
        let reads = std::cell::Cell::new(0);
        let read = || {
            reads.set(reads.get() + 1);
            7
        };
        assert_eq!(cached(&cache, "v1", read), 7);
        assert_eq!(cached(&cache, "v1", read), 7);
        assert_eq!(reads.get(), 1);
    }
}
