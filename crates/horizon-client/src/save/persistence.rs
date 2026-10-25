//! One world's admission gate and owned persistence overlay.
use super::{Record, SaveData};
use crate::Result;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct WorldPath {
    path: PathBuf,
    owner: Option<Arc<Persistence>>,
}
impl Default for WorldPath {
    fn default() -> Self {
        Self::unmanaged(PathBuf::new())
    }
}
impl std::fmt::Debug for WorldPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.path.fmt(f)
    }
}
impl std::ops::Deref for WorldPath {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}
impl AsRef<Path> for WorldPath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}
impl WorldPath {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            owner: Some(Arc::new(Persistence::default())),
        }
    }
    pub fn unmanaged(path: PathBuf) -> Self {
        Self { path, owner: None }
    }
    pub fn persistence(&self) -> Option<&Arc<Persistence>> {
        self.owner.as_ref()
    }
    pub fn join(&self, path: impl AsRef<Path>) -> Self {
        self.with_path(self.path.join(path))
    }
    pub fn with_path(&self, path: PathBuf) -> Self {
        Self {
            path,
            owner: self.owner.clone(),
        }
    }
    pub fn exists(&self) -> bool {
        self.owner.as_ref().is_some_and(|o| o.has_file(&self.path)) || self.path.exists()
    }
    pub fn is_file(&self) -> bool {
        self.owner.as_ref().is_some_and(|o| o.has_file(&self.path)) || self.path.is_file()
    }
    pub(super) fn deferred(&self) -> bool {
        self.owner
            .as_ref()
            .is_some_and(|owner| owner.has_file(&self.path))
    }
    pub fn children(&self) -> std::io::Result<Vec<Self>> {
        let mut paths = std::collections::BTreeSet::new();
        match fs::read_dir(&self.path) {
            Ok(entries) => {
                for entry in entries {
                    paths.insert(entry?.path());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if let Some(owner) = &self.owner {
            for path in owner.state.lock().unwrap().files.keys() {
                if let Ok(relative) = path.strip_prefix(&self.path)
                    && let Some(child) = relative.components().next()
                {
                    paths.insert(self.path.join(child));
                }
            }
        }
        Ok(paths.into_iter().map(|path| self.with_path(path)).collect())
    }
    pub(super) fn record<T>(&self, result: &Result<T>) {
        if let (Some(owner), Err(error)) = (&self.owner, result) {
            owner.record_error(format!("{}: {error}", self.path.display()));
        }
    }
}

/// Plain paths are for offline tools. Runtime paths retain their world owner.
pub trait SavePath: AsRef<Path> {
    fn owned(&self) -> WorldPath;
}
impl SavePath for Path {
    fn owned(&self) -> WorldPath {
        WorldPath::unmanaged(self.to_path_buf())
    }
}
impl SavePath for PathBuf {
    fn owned(&self) -> WorldPath {
        WorldPath::unmanaged(self.clone())
    }
}
impl SavePath for WorldPath {
    fn owned(&self) -> WorldPath {
        self.clone()
    }
}
impl<T: SavePath + ?Sized> SavePath for &T {
    fn owned(&self) -> WorldPath {
        (*self).owned()
    }
}

struct FileWrite {
    bytes: Vec<u8>,
    rotate: bool,
}
struct State {
    autosave: bool,
    held: Option<String>,
    ended: HashSet<String>,
    files: BTreeMap<PathBuf, FileWrite>,
    dimensions: BTreeMap<PathBuf, Arc<SaveData>>,
    error: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            autosave: true,
            held: None,
            ended: HashSet::new(),
            files: BTreeMap::new(),
            dimensions: BTreeMap::new(),
            error: None,
        }
    }
}
#[derive(Default)]
pub struct Persistence {
    state: Mutex<State>,
}
impl Persistence {
    pub(super) fn register(&self, data: Arc<SaveData>) -> Arc<SaveData> {
        self.state
            .lock()
            .unwrap()
            .dimensions
            .entry(data.root.clone())
            .or_insert(data)
            .clone()
    }
    fn has_file(&self, path: &Path) -> bool {
        self.state.lock().unwrap().files.contains_key(path)
    }
    pub fn record_error(&self, error: String) {
        self.state.lock().unwrap().error.get_or_insert(error);
    }
    pub fn check(&self) -> Result<()> {
        let state = self.state.lock().map_err(|_| "persistence poisoned")?;
        if let Some(error) = &state.error {
            return Err(error.clone().into());
        }
        for data in state.dimensions.values() {
            data.write_result()?;
        }
        Ok(())
    }
    pub fn held(&self) -> Result<Option<String>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| "persistence poisoned")?
            .held
            .clone())
    }
    pub fn autosave_enabled(&self) -> bool {
        self.state.lock().unwrap().autosave
    }
    pub fn set_autosave(&self, enabled: bool) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| "persistence poisoned")?;
        state.autosave = enabled;
        if enabled && state.held.is_none() {
            for data in state.dimensions.values() {
                data.submit_pending()?;
            }
        }
        Ok(())
    }
    pub(super) fn queue(
        &self,
        data: &SaveData,
        entities: bool,
        records: Vec<((i32, i32), Record)>,
    ) -> Result<()> {
        let state = self.state.lock().map_err(|_| "persistence poisoned")?;
        data.queue(records, entities, state.autosave && state.held.is_none())
    }
    /// Check before preparing players and generation. True means already held.
    pub fn can_begin(&self, id: &str) -> Result<bool> {
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            return Err("invalid snapshot ID".into());
        }
        let state = self.state.lock().map_err(|_| "persistence poisoned")?;
        if let Some(held) = &state.held {
            return if held == id {
                Ok(true)
            } else {
                Err(format!("snapshot {held} is held").into())
            };
        }
        if state.ended.contains(id) {
            return Err("snapshot was already ended or cancelled".into());
        }
        Ok(false)
    }
    /// Seal admission before waiting on any writer.
    pub fn begin(&self, id: &str) -> Result<()> {
        self.can_begin(id)?;
        let dimensions = {
            let mut state = self.state.lock().map_err(|_| "persistence poisoned")?;
            if state.ended.contains(id) {
                return Err("snapshot was already ended or cancelled".into());
            }
            match state.held.as_deref() {
                Some(held) if held != id => return Err(format!("snapshot {held} is held").into()),
                Some(_) => {}
                None => {
                    for data in state.dimensions.values() {
                        data.submit_pending()?;
                    }
                    state.held = Some(id.to_owned());
                }
            }
            state.dimensions.values().cloned().collect::<Vec<_>>()
        };
        let result = (|| {
            for data in dimensions {
                data.drain()?;
            }
            self.check()?;
            if self.held()?.as_deref() != Some(id) {
                return Err("snapshot was cancelled while draining".into());
            }
            Ok(())
        })();
        if let Err(error) = result {
            return match self.end(id) {
                Ok(()) => Err(error),
                Err(release) => {
                    Err(format!("{error}; releasing snapshot failed: {release}").into())
                }
            };
        }
        Ok(())
    }
    /// End also tombstones unseen IDs, so a delayed begin cannot acquire them.
    pub fn end(&self, id: &str) -> Result<()> {
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            return Err("invalid snapshot ID".into());
        }
        let mut state = self.state.lock().map_err(|_| "persistence poisoned")?;
        if let Some(held) = &state.held
            && held != id
        {
            return Err(format!("snapshot {held} is held").into());
        }
        state.held = None;
        state.ended.insert(id.to_owned());
        let mut result = Self::flush_files(&mut state);
        if state.autosave {
            for data in state.dimensions.values() {
                let next = data.submit_pending();
                if result.is_ok() {
                    result = next;
                }
            }
        }
        result?;
        if let Some(error) = &state.error {
            return Err(error.clone().into());
        }
        for data in state.dimensions.values() {
            data.write_result()?;
        }
        Ok(())
    }
    pub fn writes_finished(&self) -> Result<bool> {
        self.check()?;
        let state = self.state.lock().map_err(|_| "persistence poisoned")?;
        if state.held.is_some() {
            return Err("snapshot holds world persistence".into());
        }
        if !state.files.is_empty() {
            return Ok(false);
        }
        for data in state.dimensions.values() {
            if !data.writes_finished()? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn flush(&self) -> Result<()> {
        let dimensions = {
            let mut state = self.state.lock().map_err(|_| "persistence poisoned")?;
            if let Some(id) = &state.held {
                return Err(format!("snapshot {id} holds world persistence").into());
            }
            Self::flush_files(&mut state)?;
            for data in state.dimensions.values() {
                data.submit_pending()?;
            }
            state.dimensions.values().cloned().collect::<Vec<_>>()
        };
        for data in dimensions {
            data.drain()?;
        }
        self.check()
    }
    fn flush_files(state: &mut State) -> Result<()> {
        let paths: Vec<_> = state.files.keys().cloned().collect();
        for path in paths {
            let write = &state.files[&path];
            if let Err(error) = disk_write(&path, &write.bytes, write.rotate) {
                let message = format!("{}: {error}", path.display());
                state.error.get_or_insert(message.clone());
                return Err(message.into());
            }
            state.files.remove(&path);
        }
        Ok(())
    }
}

pub fn read_bytes(path: &(impl SavePath + ?Sized)) -> std::io::Result<Vec<u8>> {
    let path = path.owned();
    if let Some(owner) = &path.owner {
        let state = owner
            .state
            .lock()
            .map_err(|_| std::io::Error::other("persistence poisoned"))?;
        if let Some(write) = state.files.get(&path.path) {
            return Ok(write.bytes.clone());
        }
        return fs::read(&path.path);
    }
    fs::read(&path.path)
}
pub fn read_string(path: &(impl SavePath + ?Sized)) -> std::io::Result<String> {
    String::from_utf8(read_bytes(path)?)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}
pub fn write_json(path: &(impl SavePath + ?Sized), value: &serde_json::Value) -> Result<()> {
    let path = path.owned();
    let result = (|| write_bytes(&path, serde_json::to_vec_pretty(value)?))();
    path.record(&result);
    result
}
pub fn write_bytes(path: &(impl SavePath + ?Sized), bytes: impl AsRef<[u8]>) -> Result<()> {
    write_file(&path.owned(), bytes.as_ref().to_vec(), false)
}
pub(super) fn write_file(path: &WorldPath, bytes: Vec<u8>, rotate: bool) -> Result<()> {
    if let Some(owner) = &path.owner {
        let mut state = owner.state.lock().map_err(|_| "persistence poisoned")?;
        state
            .files
            .insert(path.path.clone(), FileWrite { bytes, rotate });
        if state.held.is_some() {
            return Ok(());
        }
        let write = &state.files[&path.path];
        if let Err(error) = disk_write(&path.path, &write.bytes, rotate) {
            let message = format!("{}: {error}", path.display());
            state.error.get_or_insert(message.clone());
            return Err(message.into());
        }
        state.files.remove(&path.path);
        return Ok(());
    }
    disk_write(&path.path, &bytes, rotate)
}
fn disk_write(path: &Path, bytes: &[u8], rotate: bool) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if !rotate {
        fs::write(path, bytes)?;
        return Ok(());
    }
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("data.dat");
    let temporary = path.with_file_name(format!("{file}_new"));
    fs::write(&temporary, bytes)?;
    if path.exists() {
        let old = path.with_file_name(format!("{file}_old"));
        fs::rename(path, old)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::{SaveDir, cached_region, read_dat, write_dat};
    use horizon_world::nbt::{NamedTag, Tag};
    use std::{
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    struct Fixture(WorldPath);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "horizon-persistence-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(WorldPath::new(path))
        }
        fn dimension(&self, name: &str) -> SaveDir {
            SaveDir::owned(self.0.join(name))
        }
        fn owner(&self) -> Arc<Persistence> {
            self.0.persistence().unwrap().clone()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tag(n: i32) -> NamedTag {
        NamedTag {
            name: "".into(),
            tag: Tag::compound([("value", Tag::Int(n))]),
        }
    }
    fn queue(save: &SaveDir, n: i32) {
        save.write_records("region", vec![((0, 0), Some(tag(n)))], false)
            .unwrap();
    }
    fn value(save: &SaveDir) -> Option<i32> {
        save.read_record("region", 0, 0)
            .and_then(|t| crate::save::number(t.tag.get("value")))
            .map(|n| n as i32)
    }
    fn disk(save: &SaveDir) -> Option<i32> {
        value(&SaveDir::new(save.root.clone()))
    }

    #[test]
    fn failed_backup_rotation_preserves_current_data() {
        let f = Fixture::new();
        let path = f.0.join("state.dat");
        write_dat(&path, tag(11).tag).unwrap();
        let before = fs::read(&path).unwrap();
        fs::create_dir(path.with_file_name("state.dat_old")).unwrap();

        let error = write_dat(&path, tag(29).tag).unwrap_err();
        assert!(error.to_string().contains("state.dat"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(path.with_file_name("state.dat_new").is_file());
        assert_eq!(read_dat(&path).unwrap().get("value"), Some(&Tag::Int(29)));
        assert!(f.owner().check().is_err());
    }

    #[test]
    fn snapshot_drains_admitted_records_and_defers_newer_versions() {
        let f = Fixture::new();
        let save = f.dimension("overworld");
        let region = cached_region(
            &save.regions,
            (0, 0),
            &save.region_path("region", 0, 0),
            true,
        )
        .unwrap()
        .unwrap();
        let blocked = region.lock().unwrap();
        queue(&save, 1);
        let owner = f.owner();
        let (send, receive) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            send.send(owner.begin("cut").map_err(|e| e.to_string()))
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while f.owner().held().unwrap().is_none() {
            assert!(Instant::now() < deadline, "begin never sealed admission");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            receive.try_recv().is_err(),
            "begin acknowledged a blocked writer"
        );
        let duplicate_owner = f.owner();
        let (duplicate_send, duplicate_receive) = mpsc::channel();
        let duplicate = std::thread::spawn(move || {
            duplicate_send
                .send(duplicate_owner.begin("cut").map_err(|e| e.to_string()))
                .unwrap();
        });
        assert!(
            duplicate_receive
                .recv_timeout(Duration::from_millis(20))
                .is_err(),
            "duplicate begin skipped the drain"
        );
        queue(&save, 2);
        queue(&save, 3);
        assert_eq!(value(&save), Some(3));
        drop(blocked);
        receive
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        thread.join().unwrap();
        duplicate_receive
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        duplicate.join().unwrap();
        assert_eq!(
            disk(&save),
            Some(1),
            "a deferred version replaced the archived cut"
        );
        assert!(save.flush().is_err(), "deferred is not durable");
        f.owner().end("cut").unwrap();
        f.owner().flush().unwrap();
        assert_eq!(disk(&save), Some(3));
        assert!(save.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn held_files_level_metadata_and_new_dimensions_read_the_overlay() {
        let f = Fixture::new();
        let mut level = crate::save::Level {
            name: "before".into(),
            ..Default::default()
        };
        level.write(&f.0).unwrap();
        let before = fs::read(f.0.join("level.dat")).unwrap();
        f.owner().begin("files").unwrap();
        level.name = "after".into();
        level.write(&f.0).unwrap();
        let path = f.0.join("new-folder/settings.json");
        write_json(&path, &serde_json::json!({"state": 1})).unwrap();
        write_json(&path, &serde_json::json!({"state": 2})).unwrap();
        assert!(!f.0.as_ref().join("new-folder").exists());
        assert_eq!(fs::read(f.0.join("level.dat")).unwrap(), before);
        assert_eq!(
            read_dat(&f.0.join("level.dat"))
                .unwrap()
                .get("Data")
                .unwrap()
                .get("LevelName"),
            Some(&Tag::String("after".into()))
        );
        assert_eq!(read_string(&path).unwrap(), "{\n  \"state\": 2\n}");
        let dim = f.dimension("new-dimension");
        queue(&dim, 17);
        drop(dim);
        let reopened = f.dimension("new-dimension");
        assert_eq!(value(&reopened), Some(17));
        assert_eq!(disk(&reopened), None);
        f.owner().end("files").unwrap();
        f.owner().flush().unwrap();
        assert_eq!(disk(&reopened), Some(17));
        assert_eq!(fs::read(&path).unwrap(), read_bytes(&path).unwrap());
        assert_ne!(fs::read(f.0.join("level.dat")).unwrap(), before);
    }

    #[test]
    fn saved_data_helpers_retain_the_owner_and_read_new_namespaces() {
        let f = Fixture::new();
        f.owner().begin("saved-data").unwrap();
        let mut storage = crate::command_storage::CommandStorage::default();
        let contents = Tag::compound([("answer", Tag::Int(42))]);
        storage.set("new_namespace:entry", contents.clone());
        storage.save(&f.0);
        assert_eq!(
            crate::command_storage::CommandStorage::load(&f.0).get("new_namespace:entry"),
            contents
        );
        let mut watches = crate::stopwatches::Stopwatches::default();
        watches.create("test");
        watches.save(&f.0);
        assert_eq!(
            crate::stopwatches::Stopwatches::load(&f.0).ids(),
            vec!["test"]
        );
        let rules = crate::gamerules::GameRules::default();
        rules.save(&f.0).unwrap();
        assert_eq!(
            crate::gamerules::GameRules::load(&f.0).by_name(),
            rules.by_name()
        );
        assert!(!f.0.as_ref().join("data").exists());
        f.owner().end("saved-data").unwrap();
        assert!(
            f.0.as_ref()
                .join("data/new_namespace/command_storage.dat")
                .exists()
        );
    }

    #[test]
    fn autosave_and_snapshot_ownership_are_independent_and_cancel_is_final() {
        let f = Fixture::new();
        let save = f.dimension("overworld");
        f.owner().set_autosave(false).unwrap();
        queue(&save, 1);
        assert_eq!(disk(&save), None);
        assert!(
            !save.writes_finished().unwrap(),
            "deferred chunks were reported durable"
        );
        write_dat(&f.0.join("player.dat"), tag(2).tag).unwrap();
        assert!(
            f.0.as_ref().join("player.dat").exists(),
            "vanilla save-off allows player files"
        );
        f.owner().begin("one").unwrap();
        assert_eq!(disk(&save), Some(1), "snapshot must force deferred chunks");
        f.owner().begin("one").unwrap();
        assert!(f.owner().begin("two").is_err());
        assert!(f.owner().end("two").is_err());
        queue(&save, 2);
        f.owner().set_autosave(true).unwrap();
        f.owner().set_autosave(false).unwrap();
        f.owner().end("one").unwrap();
        f.owner().end("one").unwrap();
        assert!(!f.owner().autosave_enabled());
        assert_eq!(disk(&save), Some(1));
        assert_eq!(value(&save), Some(2));
        f.owner().flush().unwrap();
        assert_eq!(disk(&save), Some(2));
        assert!(
            !f.owner().autosave_enabled(),
            "forced save changed operator preference"
        );
        f.owner().end("cancelled").unwrap();
        assert!(f.owner().begin("cancelled").is_err());
        assert!(f.owner().begin("one").is_err());
    }

    #[test]
    fn deferred_entity_deletions_survive_dimension_disposal() {
        let f = Fixture::new();
        let save = f.dimension("end");
        save.save_entities(vec![((0, 0), vec![tag(1).tag])])
            .unwrap();
        f.owner().begin("delete").unwrap();
        save.save_entities_later(vec![((0, 0), Vec::new())])
            .unwrap();
        assert!(save.load_entities(0, 0).is_empty());
        assert_eq!(SaveDir::new(save.root.clone()).load_entities(0, 0).len(), 1);
        let root = save.root.clone();
        drop(save);
        f.owner().end("delete").unwrap();
        f.owner().flush().unwrap();
        assert!(SaveDir::new(root).load_entities(0, 0).is_empty());
    }

    #[test]
    fn worlds_do_not_share_holds_or_policies() {
        let held = Fixture::new();
        let other = Fixture::new();
        held.owner().begin("private").unwrap();
        held.owner().set_autosave(false).unwrap();
        let dim = other.dimension("overworld");
        queue(&dim, 5);
        other.owner().flush().unwrap();
        write_bytes(&other.0.join("records.json"), b"{}").unwrap();
        assert_eq!(disk(&dim), Some(5));
        assert!(other.0.as_ref().join("records.json").exists());
        held.owner().end("private").unwrap();
    }

    #[test]
    fn failed_release_retries_owned_payloads_and_never_acknowledges_recovery() {
        let f = Fixture::new();
        let save = f.dimension("overworld");
        fs::write(f.0.join("blocked"), b"file").unwrap();
        f.owner().begin("release-error").unwrap();
        let path = f.0.join("blocked/state.dat");
        write_dat(&path, tag(19).tag).unwrap();
        queue(&save, 23);
        let error = f.owner().end("release-error").unwrap_err().to_string();
        assert!(error.contains("blocked/state.dat"), "{error}");
        assert_eq!(read_dat(&path), Some(tag(19).tag));
        assert!(f.owner().held().unwrap().is_none());
        save.inner.drain().unwrap();
        assert_eq!(
            disk(&save),
            Some(23),
            "file failure stranded released regions"
        );
        assert!(
            f.owner().end("release-error").is_err(),
            "retry hid the release failure"
        );
        assert_eq!(read_dat(&path), Some(tag(19).tag));
        fs::remove_file(f.0.join("blocked")).unwrap();
        assert!(
            f.owner().end("release-error").is_err(),
            "sticky failure requires restart"
        );
        assert!(
            path.as_ref().is_file(),
            "retry did not persist its owned file"
        );
        assert_eq!(read_dat(&path), Some(tag(19).tag));
        assert!(f.owner().begin("later").is_err());
    }

    #[test]
    fn ignored_file_errors_and_region_failures_prevent_snapshot_success() {
        let files = Fixture::new();
        fs::write(files.0.join("blocked"), b"file").unwrap();
        let path = files.0.join("blocked/player.dat");
        assert!(write_dat(&path, tag(8).tag).is_err());
        assert!(read_dat(&path).is_some(), "failed payload lost ownership");
        assert!(files.owner().begin("bad-file").is_err());
        assert!(files.owner().held().unwrap().is_none());
        let regions = Fixture::new();
        let save = regions.dimension("overworld");
        fs::create_dir_all(&save.root).unwrap();
        fs::write(save.root.join("entities"), b"file").unwrap();
        save.save_entities_later(vec![((0, 0), vec![tag(9).tag])])
            .unwrap();
        assert!(regions.owner().flush().is_err());
        assert_eq!(save.load_entities(0, 0).len(), 1);
        assert!(regions.owner().begin("bad-region").is_err());
        assert!(regions.owner().held().unwrap().is_none());
    }
}
