//! A build script's cache of per-file results, kept across runs in its `OUT_DIR`.
//!
//! The points scripts rerun on every edit under `game/` and used to parse the whole game again.
//! Each expensive per-file step ([`cached`]) now stores what it extracted from a file, under a
//! key made of the file's content (and whatever else the result depends on), and a rerun parses
//! only the files whose key is new.
//!
//! An entry cannot go stale:
//! - The key holds every input of the step: the file's bytes, never its timestamp, plus the
//!   other values the caller names (the module path, the wire types, the marked list).
//! - Entries live in a folder named after the generator executable's SHA-256, so a rebuilt
//!   generator (new code, new `syn`) starts empty. The hash streams the executable once per
//!   process; it is content-based, so it needs no size or mtime guess.
//! - An entry carries the digest of its payload. A truncated, edited or undecodable entry is a
//!   miss and is computed again, never an error.
//! - Entries are written to a temporary name and renamed, so a reader sees a whole entry or none.
//!
//! Entries a run did not use are deleted when it succeeds, so the folder holds one generation of
//! entries, not one per edit.
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Instant, SystemTime},
};

/// Bump to drop every entry when an entry's layout changes. (A new generator executable does
/// that already; this keeps a layout change visible in the diff.)
const FORMAT: &str = "1";

/// The generator executable's SHA-256, computed once per process.
fn identity() -> Result<&'static str, String> {
    static IDENTITY: OnceLock<Result<String, String>> = OnceLock::new();
    IDENTITY
        .get_or_init(|| {
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            let mut file = fs::File::open(&executable)
                .map_err(|e| format!("{}: {e}", executable.display()))?;
            let mut hasher = Sha256::new();
            let mut buffer = vec![0; 1 << 20];
            loop {
                let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        })
        .as_deref()
        .map_err(String::clone)
}

struct State {
    directory: PathBuf,
    /// Entries this run read or wrote: the ones [`sweep`] keeps.
    touched: HashSet<String>,
    started: SystemTime,
    begun: Instant,
    /// Per kind: entries found, entries computed.
    counts: BTreeMap<&'static str, [u32; 2]>,
    /// Named points in the run, with seconds since it began.
    stamps: Vec<(String, f64)>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Runs `run` with the cache in `out` switched on for this thread; outside it [`cached`] only
/// computes. `out` is the build script's `OUT_DIR`.
pub(crate) fn with_cache<T>(
    out: &Path,
    run: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let name = format!("incremental-{}", identity()?);
    let directory = out.join(&name);
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    struct Reset(Option<State>);
    impl Drop for Reset {
        fn drop(&mut self) {
            STATE.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(STATE.with(|slot| {
        slot.replace(Some(State {
            directory,
            touched: HashSet::new(),
            started: SystemTime::now(),
            begun: Instant::now(),
            counts: BTreeMap::new(),
            stamps: Vec::new(),
        }))
    }));
    let result = run();
    if result.is_ok() {
        sweep(out, &name);
    }
    result
}

/// Whether [`with_cache`] is on: steps that have a cached form take it only then.
pub(crate) fn active() -> bool {
    STATE.with(|slot| slot.borrow().is_some())
}

/// Deletes what this run did not use: other executables' folders (and the old `interfaces-`
/// ones) in `out`, and the unused entries of this one. Best effort; a file newer than the run
/// is left, in case something else is writing.
fn sweep(out: &Path, current: &str) {
    let Some((directory, touched, started)) = STATE.with(|slot| {
        slot.borrow().as_ref().map(|state| {
            (
                state.directory.clone(),
                state.touched.clone(),
                state.started,
            )
        })
    }) else {
        return;
    };
    if let Ok(entries) = fs::read_dir(out) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != current
                && (name.starts_with("incremental-") || name.starts_with("interfaces-"))
            {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    if let Ok(entries) = fs::read_dir(&directory) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let older = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .is_ok_and(|modified| modified < started);
            if older && !touched.contains(&name) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// The SHA-256 of `parts`, each length-prefixed so that no two part lists give one input, with
/// the cache format. A key for [`cached`].
pub(crate) fn digest(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FORMAT.as_bytes());
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    format!("{:x}", hasher.finalize())
}

/// The value of a step: read from the entry `kind`/`key` when it is whole and decodes, else from
/// `compute`, which is then stored. `key` must name every input of `compute` ([`digest`]).
/// Failing to write an entry never fails the step: the cache is disposable.
pub(crate) fn cached<T>(
    kind: &'static str,
    key: &str,
    compute: impl FnOnce() -> Result<T, String>,
    encode: impl FnOnce(&T) -> String,
    decode: impl FnOnce(&str) -> Option<T>,
) -> Result<T, String> {
    let path = STATE.with(|slot| {
        slot.borrow_mut().as_mut().map(|state| {
            let name = format!("{kind}-{key}");
            let path = state.directory.join(&name);
            state.touched.insert(name);
            path
        })
    });
    let Some(path) = path else {
        return compute();
    };
    if let Some(value) = read(&path).and_then(|payload| decode(&payload)) {
        count(kind, 0);
        return Ok(value);
    }
    count(kind, 1);
    let value = compute()?;
    write(&path, &encode(&value));
    Ok(value)
}

fn count(kind: &'static str, which: usize) {
    STATE.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.counts.entry(kind).or_default()[which] += 1;
        }
    });
}

/// The payload of the entry at `path`, when the entry is whole.
fn read(path: &Path) -> Option<String> {
    let text = String::from_utf8(fs::read(path).ok()?).ok()?;
    let (digest, payload) = text.split_once('\n')?;
    (digest == format!("{:x}", Sha256::digest(payload.as_bytes()))).then(|| payload.to_owned())
}

fn write(path: &Path, payload: &str) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let bytes = format!("{:x}\n{payload}", Sha256::digest(payload.as_bytes()));
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| file.write_all(bytes.as_bytes()))
        .and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
}

/// Checks that `code` is Rust that `syn` parses. A text that parsed once parses again, so with the
/// cache on a rerun whose generated text is unchanged skips the parse.
pub(crate) fn parses(code: &str) -> Result<(), String> {
    cached(
        "valid",
        &digest(&[code.as_bytes()]),
        || {
            syn::parse_file(code)
                .map(drop)
                .map_err(|e| format!("generated Rust: {e}"))
        },
        |()| "ok".to_owned(),
        |text| (text == "ok").then_some(()),
    )
}

/// Marks a point in the run for [`report`].
pub(crate) fn stamp(name: &str) {
    STATE.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            let seconds = state.begun.elapsed().as_secs_f64();
            state.stamps.push((name.to_owned(), seconds));
        }
    });
}

/// Lines for the timing log: the [`stamp`]s, and how many entries of each kind were found and
/// computed. Empty when the cache is off.
pub(crate) fn report() -> String {
    STATE.with(|slot| {
        let slot = slot.borrow();
        let Some(state) = slot.as_ref() else {
            return String::new();
        };
        let mut out = String::new();
        let mut before = 0.0;
        for (name, at) in &state.stamps {
            out.push_str(&format!("  {name} +{:.2}s\n", at - before));
            before = *at;
        }
        for (kind, [found, computed]) in &state.counts {
            out.push_str(&format!(
                "  cache {kind}: {found} found, {computed} computed\n"
            ));
        }
        out
    })
}

/// The SHA-256 of every source-like file (`rs`, `toml`, `json`, `wit`) under `roots`, by
/// relative path, skipping `target` folders. A folder that is missing counts as empty and
/// differs from an empty one. For results that depend on whole crates, never on one file.
pub(crate) fn tree_digest(roots: &[PathBuf]) -> Result<String, String> {
    fn walk(base: &Path, dir: &Path, hasher: &mut Sha256) -> Result<(), String> {
        let mut entries = fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        entries.sort();
        for path in entries {
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "target") {
                    continue;
                }
                walk(base, &path, hasher)?;
            } else if path.extension().is_some_and(|ext| {
                ["rs", "toml", "json", "wit"]
                    .iter()
                    .any(|want| ext == *want)
            }) {
                let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let relative = path.strip_prefix(base).unwrap_or(&path);
                let relative = relative.as_os_str().as_encoded_bytes();
                hasher.update((relative.len() as u64).to_le_bytes());
                hasher.update(relative);
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update(&bytes);
            }
        }
        Ok(())
    }
    let mut hasher = Sha256::new();
    for root in roots {
        let name = root.as_os_str().as_encoded_bytes();
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name);
        if root.is_dir() {
            hasher.update([1]);
            walk(root, root, &mut hasher)?;
        } else {
            hasher.update([0]);
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("incremental-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn step(key: &str, computed: &Cell<u32>, value: &str) -> String {
        cached(
            "unit",
            key,
            || {
                computed.set(computed.get() + 1);
                Ok(value.to_owned())
            },
            Clone::clone,
            |text| Some(text.to_owned()),
        )
        .unwrap()
    }

    fn entries(out: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for folder in fs::read_dir(out).unwrap().flatten() {
            for entry in fs::read_dir(folder.path()).unwrap().flatten() {
                found.push(entry.path());
            }
        }
        found.sort();
        found
    }

    #[test]
    fn a_step_is_computed_once_per_key_and_only_inside_a_scope() {
        let out = scratch("once");
        let computed = Cell::new(0);
        // Outside a scope nothing is stored.
        assert_eq!(step("a", &computed, "one"), "one");
        assert_eq!(step("a", &computed, "one"), "one");
        assert_eq!(computed.get(), 2);
        assert!(!active());
        computed.set(0);
        for _ in 0..2 {
            with_cache(&out, || {
                assert!(active());
                assert_eq!(step("a", &computed, "one"), "one");
                assert_eq!(step("b", &computed, "two"), "two");
                Ok(())
            })
            .unwrap();
        }
        assert_eq!(computed.get(), 2, "the second run found both entries");
        assert!(!active());
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn corrupt_entries_are_computed_again() {
        let out = scratch("corrupt");
        let computed = Cell::new(0);
        let run = || {
            with_cache(&out, || Ok(step("a", &computed, "value"))).unwrap();
        };
        run();
        let entry = entries(&out).pop().unwrap();
        for corrupt in ["", "truncated", "wrong-digest\nvalue", "\n"] {
            fs::write(&entry, corrupt).unwrap();
            computed.set(0);
            run();
            assert_eq!(computed.get(), 1, "{corrupt:?}");
            computed.set(0);
            run();
            assert_eq!(computed.get(), 0, "{corrupt:?} was rewritten");
        }
        // An entry that is whole but does not decode is a miss too.
        let decoded = with_cache(&out, || {
            cached(
                "unit",
                "a",
                || Ok(1_u32),
                |n| n.to_string(),
                |text| text.parse().ok(),
            )
        });
        assert_eq!(decoded.unwrap(), 1);
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn a_failed_step_stores_nothing_and_the_scope_resets() {
        let out = scratch("failed");
        let failed: Result<(), String> = with_cache(&out, || {
            cached::<String>(
                "unit",
                "a",
                || Err("expected".into()),
                Clone::clone,
                |text| Some(text.to_owned()),
            )
            .map(|_| ())
        });
        assert_eq!(failed.unwrap_err(), "expected");
        assert!(!active());
        assert!(entries(&out).is_empty());
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn a_successful_run_keeps_only_the_entries_it_used() {
        let out = scratch("sweep");
        let computed = Cell::new(0);
        with_cache(&out, || {
            step("a", &computed, "one");
            Ok(step("b", &computed, "two"))
        })
        .unwrap();
        assert_eq!(entries(&out).len(), 2);
        // Entries are older than the next run's start.
        std::thread::sleep(std::time::Duration::from_millis(20));
        with_cache(&out, || Ok(step("a", &computed, "one"))).unwrap();
        let left = entries(&out);
        assert_eq!(left.len(), 1, "{left:?}");
        assert_eq!(left[0].file_name().unwrap(), "unit-a");
        // A folder of another generator, and the old cache's, go too.
        fs::create_dir_all(out.join("incremental-other")).unwrap();
        fs::create_dir_all(out.join("interfaces-old")).unwrap();
        fs::create_dir_all(out.join("unrelated")).unwrap();
        with_cache(&out, || Ok(step("a", &computed, "one"))).unwrap();
        assert!(!out.join("incremental-other").exists());
        assert!(!out.join("interfaces-old").exists());
        assert!(out.join("unrelated").exists());
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn keys_separate_their_parts() {
        assert_ne!(digest(&[b"ab", b"c"]), digest(&[b"a", b"bc"]));
        assert_eq!(digest(&[b"ab", b"c"]), digest(&[b"ab", b"c"]));
        assert_ne!(digest(&[]), digest(&[b""]));
    }

    #[test]
    fn a_tree_digest_follows_content_names_and_missing_folders() {
        let out = scratch("tree");
        let a = out.join("a");
        fs::create_dir_all(a.join("src")).unwrap();
        fs::create_dir_all(a.join("target")).unwrap();
        fs::write(a.join("src/lib.rs"), "fn a() {}").unwrap();
        let roots = [a.clone(), out.join("missing")];
        let first = tree_digest(&roots).unwrap();
        assert_eq!(first, tree_digest(&roots).unwrap());
        fs::write(a.join("target/ignored.rs"), "x").unwrap();
        fs::write(a.join("src/notes.txt"), "x").unwrap();
        assert_eq!(first, tree_digest(&roots).unwrap());
        fs::write(a.join("src/lib.rs"), "fn b() {}").unwrap();
        let changed = tree_digest(&roots).unwrap();
        assert_ne!(first, changed);
        fs::rename(a.join("src/lib.rs"), a.join("src/other.rs")).unwrap();
        assert_ne!(changed, tree_digest(&roots).unwrap());
        fs::create_dir_all(out.join("missing")).unwrap();
        assert_ne!(first, tree_digest(&roots).unwrap());
        fs::remove_dir_all(out).unwrap();
    }
}
