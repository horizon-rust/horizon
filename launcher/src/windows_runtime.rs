//! Keep the game's Windows executable path stable across installed versions.
//! A slot belongs to one game home; its lock is held until the child exits.
use sha2::{Digest, Sha256};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{fs, io, path::Path, process::Command};

use crate::run_leases::SOURCE;
const SLOT: &str = "HORIZON_WINDOWS_RUNTIME_SLOT";

pub(crate) struct Guard {
    _slot: fs::File,
    _source: fs::File,
}

fn value<'a>(cmd: &'a Command, key: &str) -> Option<&'a std::ffi::OsStr> {
    cmd.get_envs().find(|(k, _)| *k == key).and_then(|(_, v)| v)
}

/// Called before adding a pre-launch command or wrapper, after applying the instance.
pub(crate) fn redirect(cmd: Command) -> Command {
    redirect_in(cmd, &crate::backend::data_dir().join("run"))
}

fn redirect_in(cmd: Command, root: &Path) -> Command {
    if value(&cmd, SOURCE).is_none() || value(&cmd, SLOT).is_some() {
        return cmd;
    }
    let home = value(&cmd, "HORIZON_HOME")
        .map(Path::new)
        .or(cmd.get_current_dir())
        .expect("the game command has a home");
    let home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    let key = format!("{:x}", Sha256::digest(home.as_os_str().as_encoded_bytes()));
    let slot = root.join(key);
    let mut out = Command::new(slot.join("game").join(crate::backend::CLIENT));
    out.args(cmd.get_args());
    if let Some(dir) = cmd.get_current_dir() {
        out.current_dir(dir);
    }
    for (key, value) in cmd.get_envs() {
        match value {
            Some(value) => out.env(key, value),
            None => out.env_remove(key),
        };
    }
    out.env(SLOT, slot);
    out
}

/// Prepare under an exclusive lock. Never fall back to a version-specific executable:
/// a locked or incomplete slot is an error, rather than another firewall identity.
pub(crate) fn prepare(cmd: &Command) -> io::Result<Option<Guard>> {
    prepare_in(cmd, &crate::backend::data_dir().join("run"))
}

fn prepare_in(cmd: &Command, root: &Path) -> io::Result<Option<Guard>> {
    let (Some(source), Some(slot)) = (value(cmd, SOURCE), value(cmd, SLOT)) else {
        return Ok(None);
    };
    let slot = Path::new(slot);
    let valid_key = slot
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.len() == 64 && n.bytes().all(|b| b.is_ascii_hexdigit()));
    if slot.parent() != Some(root) || !valid_key {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    stage(Path::new(source), slot).map(Some)
}

/// Shared claims on the units the staged game leases when it starts. It runs with no holder
/// and only locks units already registered, so the slot, its parents and the game folder
/// (new with the slot) must be registered by the launcher first, and held while it runs.
pub(crate) fn leases(
    cmd: &Command,
) -> Result<Vec<crate::run_leases::Claim>, crate::native_cache::Error> {
    let Some(slot) = value(cmd, SLOT) else {
        return Ok(Vec::new());
    };
    let game = Path::new(slot).join("game").join(crate::backend::CLIENT);
    crate::run_leases::game_units(cmd, &game)
}

/// Everything one handle says about a file without reading it: the object it is (volume serial
/// number and file index, the inode Windows links and renames by), its length, and its write time.
/// A copy keeps the source's length and write time, so only the index separates the two; the other
/// two cost nothing here and narrow a freed index that a stale slot happens to match again.
struct Facts {
    id: (u64, u64),
    size: u64,
    written: u64,
}

fn facts(path: &Path) -> io::Result<Facts> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT,
    };
    // Read-only, and without following a link: the questioned path is the one that matters.
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // The structure is an OS-owned output buffer; file keeps the handle live for the call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Facts {
        id: (
            info.dwVolumeSerialNumber as u64,
            ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        ),
        size: ((info.nFileSizeHigh as u64) << 32) | info.nFileSizeLow as u64,
        written: ((info.ftLastWriteTime.dwHighDateTime as u64) << 32)
            | info.ftLastWriteTime.dwLowDateTime as u64,
    })
}

/// Whether both paths name one file object rather than a copy of it, so the staged one can stay.
/// Only the index settles that: a copy of the source keeps its length and write time, so those two
/// can never stand in for it. A file that cannot be read is staged again.
fn same_file(from: &Path, to: &Path) -> bool {
    let Ok(staged) = facts(to) else {
        return false;
    };
    match facts(from) {
        Ok(source) => {
            source.id == staged.id && source.size == staged.size && source.written == staged.written
        }
        Err(_) => false,
    }
}

fn plain_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    // A replaced runtime slot or game folder must never redirect cleanup to player data.
    if fs::symlink_metadata(path)?.file_attributes() & 0x400 != 0 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(())
}

/// Publish `from` in the slot at `to`, leaving it alone when the same file object is already
/// there: a repeated launch has nothing to rewrite, so nothing wakes the virus scanner either.
fn place(from: &Path, to: &Path) -> io::Result<()> {
    if same_file(from, to) {
        return Ok(());
    }
    relink(from, to)
}

/// Link `from` to `to`, copying where the filesystem refuses a link (another volume, exFAT). The
/// copy has an object of its own, so such a slot is written afresh on every launch.
fn relink(from: &Path, to: &Path) -> io::Result<()> {
    replace(to, |temp| {
        fs::hard_link(from, temp).or_else(|_| fs::copy(from, temp).map(|_| ()))
    })
}

// Rename a fresh link rather than write through a previous one, which would reach into the
// version's own file: the slot name then always holds one complete file.
fn replace(to: &Path, write: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = to.with_file_name(format!(
        ".{}-{}.staged",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    // A name a crashed launch left behind is dropped, never written through: it could be a link to
    // a file the version still needs, and writing would reach that file.
    match fs::remove_file(&temp) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Err(error) = write(&temp) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    let result = fs::rename(&temp, to);
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Remove an entry of either kind, if it is there.
fn discard(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Whether a top-level name is one the slot does not carry.
fn omitted(name: &std::ffi::OsStr, skip: &[&str]) -> bool {
    skip.iter()
        .any(|skipped| name == std::ffi::OsStr::new(skipped))
}

/// Give `to` exactly what `from` holds, leaving equal files untouched. `skip` names top-level
/// entries the slot does not carry.
fn sync(from: &Path, to: &Path, skip: &[&str]) -> io::Result<()> {
    plain_directory(to)?;
    let mut kept = std::collections::HashSet::new();
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if omitted(&name, skip) {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            if !fs::symlink_metadata(&target).is_ok_and(|meta| meta.is_dir()) {
                discard(&target)?;
            }
            sync(&entry.path(), &target, &[])?;
        } else {
            if fs::symlink_metadata(&target).is_ok_and(|meta| meta.is_dir()) {
                discard(&target)?;
            }
            place(&entry.path(), &target)?;
        }
        kept.insert(name);
    }
    for entry in fs::read_dir(to)? {
        let entry = entry?;
        let name = entry.file_name();
        if !omitted(&name, skip) && !kept.contains(&name) {
            discard(&entry.path())?;
        }
    }
    Ok(())
}

/// Whether the staged program can still be replaced. Windows loads an image with delete sharing,
/// so a rename over that name succeeds even while the game runs it; only opening the image for
/// writing is refused, which is what tells staging to stop instead of stranding the running game.
fn replaceable(program: &Path) -> io::Result<()> {
    match fs::OpenOptions::new().write(true).open(program) {
        // Missing is fine: linking the program below creates it.
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        // Anything else, a sharing violation above all, means the slot is in the way.
        result => result.map(|_| ()),
    }
}

fn stage(source: &Path, slot: &Path) -> io::Result<Guard> {
    plain_directory(slot)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(slot.join("run.lock"))?;
    lock.try_lock().map_err(io::Error::other)?;
    let game = slot.join("game");
    plain_directory(&game)?;
    // Asked before the guard below holds the version's exe, since a linked image answers it too.
    replaceable(&game.join(crate::backend::CLIENT))?;
    // Before that guard opens the version's own exe: the handle shares no write, so it would
    // refuse this replacement of the slot name itself.
    place(
        &source.join(crate::backend::CLIENT),
        &game.join(crate::backend::CLIENT),
    )?;
    // Retain the original exe's protection against deletion/rebuild while playing.
    let source_file = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(source.join(crate::backend::CLIENT))?;
    // Runtime files (DLLs, native-data and native-mods.json) stay beside the exe.
    // --resources still names the installed version's own resources.
    sync(source, &game, &[crate::backend::CLIENT, ".local"])?;
    let recorded = serde_json::to_vec(&fs::canonicalize(source)?)?;
    if fs::read(slot.join("source.json")).ok().as_deref() != Some(recorded.as_slice()) {
        fs::write(slot.join("source.json"), recorded)?;
    }
    Ok(Guard {
        _slot: lock,
        _source: source_file,
    })
}

pub(crate) fn in_use(source: &Path) -> bool {
    in_use_in(source, &crate::backend::data_dir().join("run"))
}

/// Whether a program in the version's own folder is running: `horizon-ctl` under the launcher's
/// MCP server, `horizon-server`, `horizon-prepare`. They start from there, not from a runtime
/// slot, and Windows refuses both to open a running image for writing and to delete it.
fn runs_program(dir: &Path) -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let path = entry.path();
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                && fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .is_err_and(|e| e.raw_os_error() == Some(ERROR_SHARING_VIOLATION))
        })
}

fn in_use_in(source: &Path, root: &Path) -> bool {
    if runs_program(source) {
        return true;
    }
    let source = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let slots = match fs::read_dir(root) {
        Ok(slots) => slots,
        Err(e) => return e.kind() != io::ErrorKind::NotFound,
    };
    slots.filter_map(Result::ok).any(|slot| {
        let recorded = fs::read(slot.path().join("source.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<std::path::PathBuf>(&bytes).ok());
        if recorded.as_ref() != Some(&source) {
            return false;
        }
        let Ok(lock) = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(slot.path().join("run.lock"))
        else {
            return true;
        };
        if lock.try_lock().is_err() {
            return true;
        }
        // If the launcher exited first, Windows still protects the running image.
        fs::OpenOptions::new()
            .write(true)
            .open(slot.path().join("game").join(crate::backend::CLIENT))
            .is_err()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_staged_game_finds_every_unit_it_leases_registered() {
        use horizon_platform::mod_package::{
            holder, locks::LockMode, runtime_leases::open_directory, runtime_leases::Registry,
            selection::LaunchArguments,
        };
        // A version, a runtime slot and lease files: the child gets a launcher home of its own.
        const ACTOR: &str = "HORIZON_STAGED_LEASES_TEST_ACTOR";
        if std::env::var_os(ACTOR).is_none() {
            let launcher_home = tempfile::tempdir().unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--include-ignored",
                    "--exact",
                    "windows_runtime::tests::the_staged_game_finds_every_unit_it_leases_registered",
                    "--nocapture",
                ])
                .env(ACTOR, "1")
                .env("HORIZON_LAUNCHER_HOME", launcher_home.path())
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{stdout}\n{stderr}");
            assert!(
                stdout.contains("test result: ok. 1 passed;"),
                "exact-name child did not run one test:\n{stdout}\n{stderr}"
            );
            return;
        }
        let data = crate::backend::data_dir();
        fs::create_dir_all(&data).unwrap();
        let home = tempfile::tempdir().unwrap();
        let version = format!("lease-slot-{}", std::process::id());
        let source = crate::backend::version_dir(&version);
        let resources = source.join(".local/horizon-client/26.3");
        fs::create_dir_all(&resources).unwrap();
        fs::copy(
            Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe"),
            source.join(crate::backend::CLIENT),
        )
        .unwrap();
        fs::write(source.join(".installed"), b"").unwrap();
        let mut game = Command::new(source.join(crate::backend::CLIENT));
        game.current_dir(home.path())
            .env("HORIZON_HOME", home.path())
            .arg("--resources")
            .arg(&resources);
        crate::run_leases::mark(&mut game, &source);
        // What Play does: the launch wrap, then the preparation of the final command.
        let game = crate::launch::wrap(game, &crate::launch::Launch::default()).unwrap();
        let (game, runtime) = crate::backend::prepare_command(game).unwrap();

        // What the game does at start (`acquire_standalone`): lock each unit, creating none.
        let data = data.canonicalize().unwrap();
        let selection = LaunchArguments::for_command(&game).unwrap();
        let units = holder::lease_paths(
            Path::new(game.get_program()),
            &selection,
            &data,
            game.get_current_dir().unwrap(),
        )
        .unwrap();
        assert!(units.iter().any(|(unit, _)| unit.ends_with("game")));
        assert!(units
            .iter()
            .any(|(unit, _)| unit.ends_with(".local/horizon-client/26.3")));
        let registry = Registry::existing(&data).unwrap();
        let missing: Vec<_> = units
            .iter()
            .filter(|(unit, _)| {
                let directory = open_directory(unit).unwrap();
                registry.lock(&directory, LockMode::Shared).is_err()
            })
            .map(|(unit, _)| unit.display().to_string())
            .collect();
        drop(runtime);
        assert!(
            missing.is_empty(),
            "units the game cannot lease: {missing:?}"
        );
    }

    #[test]
    fn a_repeated_launch_links_the_slot_instead_of_copying_it() {
        let root = std::env::temp_dir().join(format!("horizon-linked-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("version");
        fs::create_dir_all(source.join("native-data/mod")).unwrap();
        fs::create_dir_all(source.join(".local/horizon-client/26.3")).unwrap();
        fs::write(source.join(crate::backend::CLIENT), "v1").unwrap();
        fs::write(source.join("native-data/mod/table"), "table").unwrap();
        // Without hard links the slot holds copies, which is the thing about to be compared.
        if fs::hard_link(source.join(crate::backend::CLIENT), root.join("probe.dll")).is_err() {
            return;
        }
        fs::remove_file(root.join("probe.dll")).unwrap();
        let mut cmd = Command::new(source.join(crate::backend::CLIENT));
        cmd.current_dir(&root).arg("--offscreen");
        crate::run_leases::mark(&mut cmd, &source);
        let cmd = redirect_in(cmd, &root.join("run"));
        let guard = prepare_in(&cmd, &root.join("run")).unwrap().unwrap();
        let game = Path::new(cmd.get_program()).parent().unwrap().to_owned();
        assert!(same_file(
            &source.join(crate::backend::CLIENT),
            &game.join(crate::backend::CLIENT)
        ));
        assert!(same_file(
            &source.join("native-data/mod/table"),
            &game.join("native-data/mod/table")
        ));
        assert!(!game.join(".local").exists());
        // Every file of the version is in place, so a second launch must create no entry here: a link
        // or a copy would show as this folder's own write time moving.
        let packages = game.join("native-data/mod");
        let before = fs::metadata(&packages).unwrap().last_write_time();
        // What Play does again: the slot is already right, so nothing in it is written.
        drop(guard);
        drop(prepare_in(&cmd, &root.join("run")).unwrap().unwrap());
        assert_eq!(fs::metadata(&packages).unwrap().last_write_time(), before);
        assert!(same_file(
            &source.join(crate::backend::CLIENT),
            &game.join(crate::backend::CLIENT)
        ));
        assert!(same_file(
            &source.join("native-data/mod/table"),
            &game.join("native-data/mod/table")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_slot_written_by_an_older_launcher_is_linked_again() {
        let root = std::env::temp_dir().join(format!("horizon-migrated-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("version");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join(crate::backend::CLIENT), "v1").unwrap();
        fs::write(source.join("table"), "table").unwrap();
        let mut cmd = Command::new(source.join(crate::backend::CLIENT));
        cmd.current_dir(&root).arg("--offscreen");
        crate::run_leases::mark(&mut cmd, &source);
        let cmd = redirect_in(cmd, &root.join("run"));
        let guard = prepare_in(&cmd, &root.join("run")).unwrap().unwrap();
        let game = Path::new(cmd.get_program()).parent().unwrap().to_owned();
        drop(guard);
        // What the copying staging left in a slot of an older launcher: an object of its own,
        // with the source's length and write time, so only its identity tells them apart.
        let copy = root.join("copy.dll");
        fs::copy(source.join("table"), &copy).unwrap();
        fs::rename(&copy, game.join("table")).unwrap();
        drop(prepare_in(&cmd, &root.join("run")).unwrap().unwrap());
        assert!(same_file(&source.join("table"), &game.join("table")));
        assert_eq!(fs::read_to_string(game.join("table")).unwrap(), "table");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_name_left_by_a_crashed_launch_is_not_written_through() {
        let root = std::env::temp_dir().join(format!("horizon-stale-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("version");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join(crate::backend::CLIENT), "v1").unwrap();
        fs::write(source.join("table"), "staged").unwrap();
        let mut cmd = Command::new(source.join(crate::backend::CLIENT));
        cmd.current_dir(&root).arg("--offscreen");
        crate::run_leases::mark(&mut cmd, &source);
        let cmd = redirect_in(cmd, &root.join("run"));
        let game = Path::new(cmd.get_program()).parent().unwrap().to_owned();
        // Staging names a link of its own under every counter this process can reach, as a crash
        // mid-launch could have, each one a link onto a file the version still needs.
        let foreign = root.join("foreign.dll");
        fs::write(&foreign, "keep").unwrap();
        fs::create_dir_all(&game).unwrap();
        // Every counter this process can reach, and fewer than the 1024 links one file may carry.
        for counter in 0..1000 {
            let stale = game.join(format!(".{}-{counter}.staged", std::process::id()));
            if fs::hard_link(&foreign, &stale).is_err() {
                let _ = fs::remove_dir_all(&root);
                return;
            }
        }
        drop(prepare_in(&cmd, &root.join("run")).unwrap().unwrap());
        assert_eq!(fs::read_to_string(&foreign).unwrap(), "keep");
        assert_eq!(fs::read_to_string(game.join("table")).unwrap(), "staged");
        assert!(same_file(&source.join("table"), &game.join("table")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_staged_slot_follows_the_version_it_came_from() {
        let root = std::env::temp_dir().join(format!("horizon-changed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("version");
        fs::create_dir_all(source.join("native-data/mod")).unwrap();
        fs::write(source.join(crate::backend::CLIENT), "v1").unwrap();
        fs::write(source.join("native-data/mod/table"), "one").unwrap();
        fs::write(source.join("removed.dll"), "old").unwrap();
        fs::write(source.join("replaced.dll"), "first").unwrap();
        let mut cmd = Command::new(source.join(crate::backend::CLIENT));
        cmd.current_dir(&root).arg("--offscreen");
        crate::run_leases::mark(&mut cmd, &source);
        let cmd = redirect_in(cmd, &root.join("run"));
        let guard = prepare_in(&cmd, &root.join("run")).unwrap().unwrap();
        let game = Path::new(cmd.get_program()).parent().unwrap().to_owned();
        assert_eq!(fs::read_to_string(game.join("removed.dll")).unwrap(), "old");
        drop(guard);
        // A version rewritten where it lies keeps its file object, and the slot shows the change.
        fs::write(source.join("native-data/mod/table"), "two").unwrap();
        // An update replaces a file with a new object under the same name.
        let fresh = root.join("fresh.dll");
        fs::write(&fresh, "second").unwrap();
        fs::rename(&fresh, source.join("replaced.dll")).unwrap();
        fs::write(source.join("added.dll"), "new").unwrap();
        fs::remove_file(source.join("removed.dll")).unwrap();
        drop(prepare_in(&cmd, &root.join("run")).unwrap().unwrap());
        assert_eq!(
            fs::read_to_string(game.join("native-data/mod/table")).unwrap(),
            "two"
        );
        assert_eq!(
            fs::read_to_string(game.join("replaced.dll")).unwrap(),
            "second"
        );
        assert_eq!(fs::read_to_string(game.join("added.dll")).unwrap(), "new");
        assert!(!game.join("removed.dll").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_program_running_from_the_version_folder_keeps_it_in_use() {
        let root = std::env::temp_dir().join(format!("horizon-program-{}", std::process::id()));
        let version = root.join("version");
        fs::create_dir_all(&version).unwrap();
        let ctl = version.join("horizon-ctl.exe");
        fs::copy(
            Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe"),
            &ctl,
        )
        .unwrap();
        // No game ever ran from it: no runtime slot names this version.
        let slots = root.join("run");
        assert!(!in_use_in(&version, &slots));
        let mut cmd = Command::new(&ctl);
        cmd.current_dir(&root).args(["/D", "/C", "set /p answer="]);
        crate::process::hide_console(&mut cmd);
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let running = child.try_wait().unwrap().is_none();
        let protected = in_use_in(&version, &slots);
        let _ = child.kill();
        child.wait().unwrap();
        assert!(running && protected);
        assert!(!in_use_in(&version, &slots));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_orphaned_process_keeps_its_runtime_and_source_version_protected() {
        let root = std::env::temp_dir().join(format!("horizon-orphan-{}", std::process::id()));
        let source = root.join("version");
        fs::create_dir_all(&source).unwrap();
        fs::copy(
            Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe"),
            source.join(crate::backend::CLIENT),
        )
        .unwrap();
        let mut cmd = Command::new(source.join(crate::backend::CLIENT));
        cmd.current_dir(&root).args(["/D", "/C", "set /p answer="]);
        crate::run_leases::mark(&mut cmd, &source);
        let mut cmd = redirect_in(cmd, &root.join("run"));
        let guard = prepare_in(&cmd, &root.join("run")).unwrap().unwrap();
        crate::process::hide_console(&mut cmd);
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        drop(guard); // Simulate a launcher exiting before its game.
        let running = child.try_wait().unwrap().is_none();
        let protected = in_use_in(&source, &root.join("run"));
        let replacement = prepare_in(&cmd, &root.join("run"));
        let _ = child.kill();
        child.wait().unwrap();
        assert!(running && protected);
        assert!(replacement.is_err());
        assert!(!in_use_in(&source, &root.join("run")));
        drop(prepare_in(&cmd, &root.join("run")).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn versions_reuse_a_slot_keep_native_files_and_cannot_replace_a_running_game() {
        let root = std::env::temp_dir().join(format!("horizon-runtime-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let command = |version: &str| {
            let source = root.join(version);
            fs::create_dir_all(source.join("native-data/mod")).unwrap();
            fs::write(source.join(crate::backend::CLIENT), version).unwrap();
            fs::write(source.join("native-data/mod/table"), version).unwrap();
            let mut cmd = Command::new(source.join(crate::backend::CLIENT));
            cmd.current_dir(&home)
                .env("HORIZON_HOME", &home)
                .arg("--offscreen");
            crate::run_leases::mark(&mut cmd, &source);
            redirect_in(cmd, &root.join("run"))
        };
        let first = command("v1");
        let lock = prepare_in(&first, &root.join("run")).unwrap().unwrap();
        assert!(in_use_in(&root.join("v1"), &root.join("run")));
        assert!(fs::remove_file(root.join("v1").join(crate::backend::CLIENT)).is_err());
        let game = Path::new(first.get_program()).parent().unwrap();
        fs::write(game.join("removed.dll"), "old").unwrap();
        let second = command("v2");
        assert_eq!(first.get_program(), second.get_program());
        assert!(prepare_in(&second, &root.join("run")).is_err());
        assert_eq!(fs::read_to_string(first.get_program()).unwrap(), "v1");
        drop(lock);
        assert!(!in_use_in(&root.join("v1"), &root.join("run")));
        let lock = prepare_in(&second, &root.join("run")).unwrap().unwrap();
        assert!(!in_use_in(&root.join("v1"), &root.join("run")));
        assert!(in_use_in(&root.join("v2"), &root.join("run")));
        assert_eq!(fs::read_to_string(second.get_program()).unwrap(), "v2");
        assert_eq!(
            fs::read_to_string(game.join("native-data/mod/table")).unwrap(),
            "v2"
        );
        assert!(!game.join("removed.dll").exists());
        assert_eq!(second.get_current_dir(), Some(home.as_path()));
        assert_eq!(second.get_args().collect::<Vec<_>>(), ["--offscreen"]);
        drop(lock);
        let unrelated = root.join("unrelated");
        fs::create_dir_all(&unrelated).unwrap();
        fs::write(unrelated.join("keep.txt"), "player data").unwrap();
        let mut injected = command("v3");
        injected.env(SLOT, &unrelated);
        assert!(prepare_in(&injected, &root.join("run")).is_err());
        assert_eq!(
            fs::read_to_string(unrelated.join("keep.txt")).unwrap(),
            "player data"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
