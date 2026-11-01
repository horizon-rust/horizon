//! A Java 25 runtime for Android. Android has none, and preparing Horizon's game files runs
//! Mojang's own code (the vanilla exporters) on one. Termux builds OpenJDK 25 natively for
//! Android (bionic, arm64) and publishes it as `.deb` packages; this downloads the JDK and the few
//! small libraries it needs from Termux's repository, checks each against the SHA-256 the
//! repository's index lists, and unpacks them into the app's folder:
//!
//! - `runtime/java/`: the JDK (`bin/java`, `bin/javac`, `lib/modules`...);
//! - `runtime/lib/`: `libz.so.1`, `libandroid-spawn.so`, `libiconv.so` and `libc++_shared.so`, the
//!   libraries the JDK links that Android itself lacks.
//!
//! [`library_path`] is the `LD_LIBRARY_PATH` to run it with. The JDK is Termux's package of
//! OpenJDK (GPLv2 with Classpath Exception) and is not part of Horizon.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::Duration;

pub const REPO: &str = "https://packages.termux.dev/apt/termux-main";
pub const INDEX: &str = "dists/stable/main/binary-aarch64/Packages";
/// The JDK package, and the libraries it needs to run without a screen.
pub const JDK_PACKAGE: &str = "openjdk-25";
pub const LIBRARY_PACKAGES: [&str; 4] = ["libandroid-spawn", "zlib", "libiconv", "libc++"];
/// What `runtime/ready` says when the JDK and every library are in. (An older install wrote just the
/// JDK package name: its JDK is kept and only the libraries are fetched.)
const READY: &str = "openjdk-25 libs-2";
/// Where Termux's packages keep their files.
const PREFIX: &str = "data/data/com.termux/files/usr/";
const JDK_DIR: &str = "lib/jvm/java-25-openjdk/";

/// One package of the repository index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
}

/// The packages of an index (`Packages`): the newest listing of each name.
pub fn parse_index(text: &str) -> HashMap<String, Package> {
    let mut out: HashMap<String, Package> = HashMap::new();
    for stanza in text.split("\n\n") {
        let field = |key: &str| {
            stanza
                .lines()
                .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix(": ")))
                .map(str::trim)
        };
        let (Some(name), Some(version), Some(filename), Some(sha256)) = (
            field("Package"),
            field("Version"),
            field("Filename"),
            field("SHA256"),
        ) else {
            continue;
        };
        let package = Package {
            name: name.to_owned(),
            version: version.to_owned(),
            filename: filename.to_owned(),
            sha256: sha256.to_owned(),
            size: field("Size").and_then(|s| s.parse().ok()).unwrap_or(0),
        };
        // The index may list a package twice (a revision): keep the later listing.
        out.insert(package.name.clone(), package);
    }
    out
}

/// The members of an `ar` archive (a `.deb`): `(name, bytes)`.
pub fn ar_members(bytes: &[u8]) -> Result<Vec<(String, std::ops::Range<usize>)>, String> {
    const MAGIC: &[u8] = b"!<arch>\n";
    if !bytes.starts_with(MAGIC) {
        return Err("The package is not a .deb file.".into());
    }
    let mut at = MAGIC.len();
    let mut out = Vec::new();
    while at + 60 <= bytes.len() {
        let header = &bytes[at..at + 60];
        if &header[58..60] != b"`\n" {
            return Err("The package's archive is damaged.".into());
        }
        let name = String::from_utf8_lossy(&header[..16])
            .trim()
            .trim_end_matches('/')
            .to_owned();
        let size: usize = String::from_utf8_lossy(&header[48..58])
            .trim()
            .parse()
            .map_err(|_| "The package's archive is damaged.".to_string())?;
        let start = at + 60;
        let end = start
            .checked_add(size)
            .filter(|e| *e <= bytes.len())
            .ok_or("The package is cut short.")?;
        out.push((name, start..end));
        at = end + (end % 2);
    }
    Ok(out)
}

/// Where a path inside a package goes under `dest` once `prefix` is taken off, or None for
/// files outside it. Paths that climb out are refused.
pub fn target(dest: &Path, prefix: &str, entry: &Path) -> Result<Option<PathBuf>, String> {
    let mut clean = PathBuf::new();
    for c in entry.components() {
        match c {
            Component::Normal(p) => clean.push(p),
            Component::CurDir => {}
            _ => {
                return Err(format!(
                    "A package path leaves its folder: {}",
                    entry.display()
                ));
            }
        }
    }
    Ok(clean
        .strip_prefix(prefix)
        .ok()
        .filter(|r| !r.as_os_str().is_empty())
        .map(|r| dest.join(r)))
}

/// Unpacks the files of a `.deb`'s data archive that live under `prefix` (in the package) into
/// `dest`, keeping symlinks and permissions. `keep` can narrow it to some of those files.
pub fn extract_deb(
    deb: &Path,
    prefix: &str,
    dest: &Path,
    keep: &dyn Fn(&Path) -> bool,
) -> Result<usize, String> {
    let bytes = fs::read(deb).map_err(|e| format!("Could not read the package: {e}"))?;
    let members = ar_members(&bytes)?;
    let (name, range) = members
        .iter()
        .find(|(n, _)| n.starts_with("data.tar"))
        .ok_or("The package has no data.")?;
    let data = &bytes[range.clone()];
    // The data archive decompressed to a file next to the destination, then read as tar.
    fs::create_dir_all(dest).map_err(|e| format!("Could not make {}: {e}", dest.display()))?;
    let tar_path = dest.join(".data.tar.part");
    {
        let mut out = fs::File::create(&tar_path).map_err(|e| format!("Could not write: {e}"))?;
        if name.ends_with(".xz") {
            lzma_rs::xz_decompress(&mut BufReader::new(data), &mut out)
                .map_err(|e| format!("The package would not unpack: {e}"))?;
        } else if name.ends_with(".gz") {
            std::io::copy(&mut flate2::read::GzDecoder::new(data), &mut out)
                .map_err(|e| format!("The package would not unpack: {e}"))?;
        } else {
            return Err(format!("Unknown package compression ({name})."));
        }
    }
    let mut count = 0;
    let result = (|| -> Result<(), String> {
        let file = fs::File::open(&tar_path).map_err(|e| e.to_string())?;
        let mut archive = tar::Archive::new(BufReader::new(file));
        archive.set_preserve_permissions(true);
        for entry in archive.entries().map_err(|e| e.to_string())? {
            let mut entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path().map_err(|e| e.to_string())?.into_owned();
            let Some(out) = target(dest, prefix, &path)? else {
                continue;
            };
            if !keep(out.strip_prefix(dest).unwrap_or(&out)) {
                continue;
            }
            if let Some(dir) = out.parent() {
                fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            let kind = entry.header().entry_type();
            if kind.is_symlink() {
                // A symlink must stay inside the destination.
                let link = entry
                    .link_name()
                    .map_err(|e| e.to_string())?
                    .map(|l| l.into_owned());
                if let Some(link) = link {
                    if link.is_absolute()
                        || link.components().any(|c| matches!(c, Component::ParentDir))
                    {
                        // Termux's absolute links point into its own prefix: leave them out.
                        continue;
                    }
                }
                let _ = fs::remove_file(&out);
            }
            entry
                .unpack(&out)
                .map_err(|e| format!("{}: {e}", out.display()))?;
            count += 1;
        }
        Ok(())
    })();
    let _ = fs::remove_file(&tar_path);
    result.map(|()| count)
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(120))
        .user_agent("horizon-mobile")
        .try_proxy_from_env(true)
        .build()
}

/// What the installer reports.
pub enum Step {
    /// A step starting, in words.
    Line(String),
    /// Bytes of a download: what, done, total.
    Bytes(String, u64, u64),
}

/// A download in chunks, reporting as it goes, checked against `expect_sha256`; tried three times,
/// as a phone's connection drops.
fn fetch(
    url: &str,
    label: &str,
    expect_sha256: Option<&str>,
    cancel: &AtomicBool,
    progress: &dyn Fn(Step),
) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for attempt in 1..=3 {
        if cancel.load(Relaxed) {
            return Err("Cancelled.".into());
        }
        if attempt > 1 {
            progress(Step::Line(format!(
                "{label}: trying again ({attempt} of 3)"
            )));
        }
        match fetch_once(url, label, expect_sha256, cancel, progress) {
            Ok(bytes) => return Ok(bytes),
            Err(e) if e == "Cancelled." => return Err(e),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn fetch_once(
    url: &str,
    label: &str,
    expect_sha256: Option<&str>,
    cancel: &AtomicBool,
    progress: &dyn Fn(Step),
) -> Result<Vec<u8>, String> {
    let response = agent().get(url).call().map_err(|e| {
        format!("Could not reach the Java download ({e}). Check your internet connection.")
    })?;
    let total: u64 = response
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut reader = response.into_reader().take(1 << 30);
    let mut bytes = Vec::with_capacity(total as usize);
    let mut chunk = vec![0u8; 1 << 16];
    let mut reported = 0u64;
    loop {
        if cancel.load(Relaxed) {
            return Err("Cancelled.".into());
        }
        let n = reader.read(&mut chunk).map_err(|e| {
            format!(
                "The Java download was interrupted after {} MB ({e}).",
                bytes.len() >> 20
            )
        })?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        let done = bytes.len() as u64;
        if done - reported >= (1 << 20) {
            reported = done;
            progress(Step::Bytes(label.to_owned(), done, total));
        }
    }
    if total > 0 && bytes.len() as u64 != total {
        return Err(format!(
            "The Java download was cut short ({} of {} MB).",
            bytes.len() >> 20,
            total >> 20
        ));
    }
    if let Some(want) = expect_sha256 {
        let got: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if got != want {
            return Err("The Java download failed its check.".into());
        }
    }
    Ok(bytes)
}

/// The folder of the JDK under `runtime` (what `JAVA_HOME` names), when it is installed.
pub fn home(runtime: &Path) -> Option<PathBuf> {
    let home = runtime.join("java");
    let ready = fs::read_to_string(runtime.join("ready")).ok()?;
    (home.join("bin/java").is_file() && ready.trim() == READY).then_some(home)
}

/// `LD_LIBRARY_PATH` for running the JDK installed under `runtime`: its own libraries, then the
/// small ones Android lacks.
pub fn library_path(runtime: &Path) -> String {
    let j = runtime.join("java");
    format!(
        "{}:{}:{}",
        j.join("lib").display(),
        j.join("lib/server").display(),
        runtime.join("lib").display()
    )
}

/// Installs the runtime under `runtime`. `progress` gets a line for each step; `cancel` stops it
/// between packages.
pub fn install(
    runtime: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(Step),
) -> Result<PathBuf, String> {
    let line = |text: &str| progress(Step::Line(text.to_owned()));
    if let Some(home) = home(runtime) {
        return Ok(home);
    }
    line("Finding Java 25 for Android");
    let index = String::from_utf8_lossy(&fetch(
        &format!("{REPO}/{INDEX}"),
        "the package list",
        None,
        cancel,
        progress,
    )?)
    .into_owned();
    let packages = parse_index(&index);
    let get = |name: &str| {
        packages
            .get(name)
            .cloned()
            .ok_or_else(|| format!("The Java repository has no {name}."))
    };
    let scratch = runtime.join("download");
    fs::create_dir_all(&scratch)
        .map_err(|e| format!("Could not make {}: {e}", scratch.display()))?;

    // The small libraries first (a few hundred KB), then the JDK (about 115 MB).
    for name in LIBRARY_PACKAGES {
        let done = runtime.join("lib").join(format!(".{name}.ok"));
        if done.is_file() {
            continue;
        }
        let p = get(name)?;
        line(&format!("Downloading {name} {}", p.version));
        let bytes = fetch(
            &format!("{REPO}/{}", p.filename),
            name,
            Some(&p.sha256),
            cancel,
            progress,
        )?;
        let deb = scratch.join(format!("{name}.deb"));
        fs::write(&deb, bytes).map_err(|e| format!("Could not write {}: {e}", deb.display()))?;
        // Only the shared libraries (and their version links).
        extract_deb(&deb, &format!("{PREFIX}lib/"), &runtime.join("lib"), &|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(".so"))
        })?;
        let _ = fs::remove_file(&deb);
        let _ = fs::write(&done, &p.version);
    }
    // An older install already has the JDK: keep it.
    let jdk_in = runtime.join("java/bin/java").is_file()
        && fs::read_to_string(runtime.join("ready")).is_ok_and(|r| r.trim() == JDK_PACKAGE);
    if jdk_in {
        line("Java 25 is already downloaded");
        fs::write(runtime.join("ready"), READY).map_err(|e| format!("Could not write: {e}"))?;
        let _ = fs::remove_dir_all(&scratch);
        return Ok(runtime.join("java"));
    }
    let jdk = get(JDK_PACKAGE)?;
    line(&format!("Downloading Java 25 ({} MB)", jdk.size >> 20));
    let bytes = fetch(
        &format!("{REPO}/{}", jdk.filename),
        "Java 25",
        Some(&jdk.sha256),
        cancel,
        progress,
    )?;
    let deb = scratch.join("jdk.deb");
    fs::write(&deb, bytes).map_err(|e| {
        format!(
            "Could not write {} (is the device full?): {e}",
            deb.display()
        )
    })?;
    line("Unpacking Java 25 (this takes a minute)");
    let home_dir = runtime.join("java");
    extract_deb(&deb, &format!("{PREFIX}{JDK_DIR}"), &home_dir, &|p| {
        // The runtime and the compiler: not the demos, the manual pages, the headers or the jmods.
        !matches!(
            p.components().next().and_then(|c| c.as_os_str().to_str()),
            Some("demo" | "man" | "include" | "jmods" | "legal")
        )
    })?;
    let _ = fs::remove_dir_all(&scratch);
    if !home_dir.join("bin/java").is_file() {
        return Err("Java 25 unpacked without its launcher.".into());
    }
    line("Java 25 is installed");
    fs::write(runtime.join("ready"), READY).map_err(|e| format!("Could not write: {e}"))?;
    Ok(home_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX_TEXT: &str = "Package: zlib\nVersion: 1.3.2\nFilename: pool/z/zlib_1.3.2_aarch64.deb\nSize: 62840\nSHA256: aa\n\nPackage: openjdk-25\nVersion: 25.0.5-1\nFilename: pool/o/jdk1.deb\nSize: 5\nSHA256: bb\n\nPackage: openjdk-25\nVersion: 25.0.5\nFilename: pool/o/jdk2.deb\nSize: 6\nSHA256: cc\n\nPackage: broken\nVersion: 1\n";

    #[test]
    fn the_index_gives_each_package_once() {
        let p = parse_index(INDEX_TEXT);
        assert_eq!(p.len(), 2);
        assert_eq!(p["zlib"].filename, "pool/z/zlib_1.3.2_aarch64.deb");
        assert_eq!(p["zlib"].size, 62840);
        assert_eq!(p["openjdk-25"].sha256, "cc");
        assert!(!p.contains_key("broken"));
    }

    fn ar(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = b"!<arch>\n".to_vec();
        for (name, data) in members {
            out.extend(
                format!(
                    "{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                    0,
                    0,
                    0,
                    100644,
                    data.len()
                )
                .as_bytes(),
            );
            out.extend(*data);
            if data.len() % 2 == 1 {
                out.push(b'\n');
            }
        }
        out
    }

    #[test]
    fn ar_members_are_found_with_odd_sizes() {
        let bytes = ar(&[
            ("debian-binary", b"2.0\n"),
            ("data.tar.xz", b"abc"),
            ("last", b"z"),
        ]);
        let m = ar_members(&bytes).unwrap();
        assert_eq!(
            m.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["debian-binary", "data.tar.xz", "last"]
        );
        assert_eq!(&bytes[m[1].1.clone()], b"abc");
        assert_eq!(&bytes[m[2].1.clone()], b"z");
        assert!(ar_members(b"nope").is_err());
        assert!(ar_members(&bytes[..bytes.len() - 2]).is_err());
    }

    #[test]
    fn targets_strip_the_prefix_and_refuse_escapes() {
        let dest = Path::new("/app/runtime/java");
        let p = Path::new("./data/data/com.termux/files/usr/lib/jvm/java-25-openjdk/bin/java");
        assert_eq!(
            target(dest, &format!("{PREFIX}{JDK_DIR}"), p).unwrap(),
            Some(PathBuf::from("/app/runtime/java/bin/java"))
        );
        assert_eq!(
            target(
                dest,
                &format!("{PREFIX}{JDK_DIR}"),
                Path::new("./usr/bin/x")
            )
            .unwrap(),
            None
        );
        assert!(target(dest, PREFIX, Path::new("../../etc/passwd")).is_err());
        assert!(target(dest, PREFIX, Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn the_library_path_lists_the_jdk_then_the_small_libraries() {
        let lp = library_path(Path::new("/app/runtime"));
        assert_eq!(
            lp,
            "/app/runtime/java/lib:/app/runtime/java/lib/server:/app/runtime/lib"
        );
    }

    #[test]
    fn an_install_is_ready_only_with_the_current_marker() {
        let dir = std::env::temp_dir().join(format!("hb-java-{}", std::process::id()));
        fs::create_dir_all(dir.join("java/bin")).unwrap();
        fs::write(dir.join("java/bin/java"), "x").unwrap();
        // The first version's marker (the JDK only): not ready, the libraries are missing.
        fs::write(dir.join("ready"), JDK_PACKAGE).unwrap();
        assert!(home(&dir).is_none());
        fs::write(dir.join("ready"), READY).unwrap();
        assert_eq!(home(&dir), Some(dir.join("java")));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn libc_plus_plus_is_among_the_libraries() {
        assert!(LIBRARY_PACKAGES.contains(&"libc++"));
    }

    #[test]
    fn a_missing_install_is_not_ready() {
        assert!(home(Path::new("/definitely/not/here")).is_none());
    }
}
