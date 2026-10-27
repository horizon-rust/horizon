//! Distant terrain kept on disk, beside the world's save (`data/horizon-lod`): each chunk's
//! summary (its columns as `Column`s, from its save or from speculative generation) and the
//! samples of the tiles too coarse for chunk summaries. Without it every start sampled and
//! generated the whole distant view again (minutes of CPU at 1024 chunks), and terrain first
//! seen as noise samples changed shape once its chunks' summaries arrived.
//!
//! Entries are grouped in files of 32 by 32 keys and compressed one by one; a file is read
//! whole the first time one of its keys is asked for, and written whole (to a temporary file,
//! then renamed) by `flush`, outside the lock. A file from another build or block registry is
//! ignored. `forget` comes from the game thread with each chunk save: it never reads a file,
//! a removal from one not read yet waits for it (`Files::unapplied`, done by `flush` at the
//! latest).
//! `forget` drops a chunk's summary when its save changes: the next read summarizes the save.
//! Tiles fine enough to be made of chunk summaries (strides up to 16) are kept too, and go
//! whenever one of their chunks changes.
use crate::{
    fast_hash::FxHashMap,
    lod::{Column, Span, Surface},
};
use horizon_world::registry::BlockStateId;
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::PathBuf,
    sync::Mutex,
};

const MAGIC: &[u8; 4] = b"HLOD";
/// Changes with the encoding, or with what a summary or a sample means.
const FORMAT: u32 = 1;
/// Keys per file side.
const SPAN: i32 = 32;
/// Files kept in memory (each about 1024 entries of a few kilobytes).
const LOADED: usize = 48;

/// What is stored: a chunk's summary, or a tile's samples (its origin and stride).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Chunk(i32, i32),
    Tile { x: i32, z: i32, stride: i32 },
}

impl Key {
    /// The file holding it, and its name.
    fn file(self) -> (File, String) {
        match self {
            Key::Chunk(x, z) => {
                let (fx, fz) = (x.div_euclid(SPAN), z.div_euclid(SPAN));
                (File::Chunks(fx, fz), format!("c.{fx}.{fz}.hlod"))
            }
            Key::Tile { x, z, stride } => {
                let size = stride * 16 * SPAN;
                let (fx, fz) = (x.div_euclid(size), z.div_euclid(size));
                (
                    File::Tiles(stride, fx, fz),
                    format!("t{stride}.{fx}.{fz}.hlod"),
                )
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum File {
    Chunks(i32, i32),
    Tiles(i32, i32, i32),
}

#[derive(Default)]
struct Loaded {
    /// Compressed entries.
    entries: FxHashMap<Key, Vec<u8>>,
    dirty: bool,
}

#[derive(Default)]
struct Files {
    loaded: FxHashMap<File, Loaded>,
    order: VecDeque<File>,
    /// Counts chunk changes (`forget`, a chunk's `put`); `touched` holds each chunk's last.
    generation: u64,
    touched: FxHashMap<(i32, i32), u64>,
    /// Chunks summarized from the loaded chunk (`put_live`): the save queued with it next
    /// holds the same blocks, so that `forget` leaves the summary.
    live: crate::fast_hash::FxHashSet<(i32, i32)>,
    /// Entries to remove from files not read yet: removed once they are.
    unapplied: FxHashMap<File, Vec<Key>>,
    /// Files' latest contents not on disk yet (`None`: the file goes), for `flush`; a file
    /// read again before that is read from here.
    pending: FxHashMap<File, (PathBuf, Option<Vec<u8>>)>,
}

impl Files {
    /// Removes `key` from its file, at once if it is read, else when it is.
    fn remove(&mut self, key: Key) {
        let (file, _) = key.file();
        match self.loaded.get_mut(&file) {
            Some(loaded) => {
                if loaded.entries.remove(&key).is_some() {
                    loaded.dirty = true;
                }
            }
            None => self.unapplied.entry(file).or_default().push(key),
        }
    }
}

pub struct LodStore {
    dir: PathBuf,
    /// The build and block registry the entries were made with.
    fingerprint: u64,
    files: Mutex<Files>,
    /// Held by `flush` while it writes: one at a time, so files reach the disk in order.
    writing: Mutex<()>,
}

impl LodStore {
    /// `states`: the block registry's state count, part of what makes entries valid.
    pub fn new(dir: PathBuf, states: usize) -> Self {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (FORMAT, env!("CARGO_PKG_VERSION"), states).hash(&mut h);
        Self {
            dir,
            fingerprint: h.finish(),
            files: Mutex::default(),
            writing: Mutex::new(()),
        }
    }

    pub fn get(&self, key: Key) -> Option<Vec<Column>> {
        let mut files = self.files.lock().ok()?;
        let bytes = self.load(&mut files, key).entries.get(&key)?.clone();
        drop(files);
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(&bytes[..])
            .read_to_end(&mut raw)
            .ok()?;
        decode(&raw)
    }

    /// The count of chunk changes so far: what is made from chunks read after it is
    /// `put_since` it.
    pub fn generation(&self) -> u64 {
        self.files.lock().map_or(0, |f| f.generation)
    }

    /// `put`, unless a chunk it is made of changed after `generation` (it changed while it was
    /// read: what was made of it is already stale).
    pub fn put_since(&self, key: Key, columns: &[Column], generation: u64) {
        let chunks = match key {
            Key::Chunk(x, z) => [x, z, 1],
            Key::Tile { x, z, stride } => [x.div_euclid(16), z.div_euclid(16), stride],
        };
        let stale = self.files.lock().is_ok_and(|f| {
            (0..chunks[2]).any(|dz| {
                (0..chunks[2]).any(|dx| {
                    f.touched
                        .get(&(chunks[0] + dx, chunks[1] + dz))
                        .is_some_and(|&g| g > generation)
                })
            })
        });
        if !stale {
            self.put(key, columns);
        }
    }

    /// A chunk's summary made from the loaded chunk, which its next save holds.
    pub fn put_live(&self, x: i32, z: i32, columns: &[Column]) {
        self.put(Key::Chunk(x, z), columns);
        if let Ok(mut files) = self.files.lock() {
            files.live.insert((x, z));
        }
    }

    pub fn put(&self, key: Key, columns: &[Column]) {
        let mut compressed =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        if compressed.write_all(&encode(columns)).is_err() {
            return;
        }
        let Ok(bytes) = compressed.finish() else {
            return;
        };
        let Ok(mut files) = self.files.lock() else {
            return;
        };
        let file = self.load(&mut files, key);
        file.entries.insert(key, bytes);
        file.dirty = true;
        if let Key::Chunk(x, z) = key {
            self.touch(&mut files, x, z);
        }
    }

    /// A chunk changed: counted, and the tiles made of it go.
    fn touch(&self, files: &mut Files, x: i32, z: i32) {
        files.generation += 1;
        let generation = files.generation;
        files.touched.insert((x, z), generation);
        for stride in [1, 2, 4, 8, 16] {
            let size = 16 * stride;
            let tile = Key::Tile {
                x: (x * 16).div_euclid(size) * size,
                z: (z * 16).div_euclid(size) * size,
                stride,
            };
            files.remove(tile);
        }
    }

    /// Drops a chunk's summary (its save changed).
    pub fn forget(&self, x: i32, z: i32) {
        let key = Key::Chunk(x, z);
        let Ok(mut files) = self.files.lock() else {
            return;
        };
        if files.live.remove(&(x, z)) {
            return;
        }
        self.touch(&mut files, x, z);
        files.remove(key);
    }

    /// Writes the files changed since they were read, and those with removals waiting (read
    /// for them first). The writing happens outside the lock.
    pub fn flush(&self) {
        let Ok(_writing) = self.writing.lock() else {
            return;
        };
        let writes = {
            let Ok(mut files) = self.files.lock() else {
                return;
            };
            let waiting: Vec<File> = files.unapplied.keys().copied().collect();
            for file in waiting {
                let key = files.unapplied[&file][0];
                self.load(&mut files, key);
            }
            let mut writes = std::mem::take(&mut files.pending);
            for (&file, loaded) in files.loaded.iter_mut().filter(|(_, f)| f.dirty) {
                writes.insert(file, self.serialize(file, loaded));
                loaded.dirty = false;
            }
            writes
        };
        let writes = writes.into_values();
        for write in writes {
            self.write(write);
        }
    }

    fn path(&self, key: Key) -> PathBuf {
        self.dir.join(key.file().1)
    }

    /// The file holding `key`, read from disk the first time; the least recently read one
    /// goes (written first if it changed) past `LOADED`.
    fn load<'a>(&self, files: &'a mut Files, key: Key) -> &'a mut Loaded {
        let (file, _) = key.file();
        if !files.loaded.contains_key(&file) {
            while files.order.len() >= LOADED {
                let Some(old) = files.order.pop_front() else {
                    break;
                };
                if let Some(loaded) = files.loaded.remove(&old)
                    && loaded.dirty
                {
                    let contents = self.serialize(old, &loaded);
                    files.pending.insert(old, contents);
                }
            }
            let mut entries: FxHashMap<Key, Vec<u8>> = match files.pending.get(&file) {
                Some((_, contents)) => contents.as_deref().and_then(|b| self.read_file(b)),
                None => std::fs::read(self.path(key))
                    .ok()
                    .and_then(|bytes| self.read_file(&bytes)),
            }
            .unwrap_or_default();
            let mut dirty = false;
            for gone in files.unapplied.remove(&file).unwrap_or_default() {
                dirty |= entries.remove(&gone).is_some();
            }
            files.loaded.insert(file, Loaded { entries, dirty });
            files.order.push_back(file);
        }
        files.loaded.get_mut(&file).expect("loaded above")
    }

    fn read_file(&self, bytes: &[u8]) -> Option<FxHashMap<Key, Vec<u8>>> {
        let mut r = Reader(bytes);
        if r.take(4)? != MAGIC || r.u64()? != self.fingerprint {
            return None;
        }
        let mut entries = FxHashMap::default();
        for _ in 0..r.u32()? {
            let key = match r.u8()? {
                0 => Key::Chunk(r.i32()?, r.i32()?),
                _ => Key::Tile {
                    x: r.i32()?,
                    z: r.i32()?,
                    stride: r.i32()?,
                },
            };
            let len = r.u32()? as usize;
            entries.insert(key, r.take(len)?.to_vec());
        }
        Some(entries)
    }

    /// A file's path and contents, `None` with no entries left (the file goes).
    fn serialize(&self, file: File, loaded: &Loaded) -> (PathBuf, Option<Vec<u8>>) {
        let name = match file {
            File::Chunks(x, z) => format!("c.{x}.{z}.hlod"),
            File::Tiles(s, x, z) => format!("t{s}.{x}.{z}.hlod"),
        };
        let path = self.dir.join(name);
        if loaded.entries.is_empty() {
            return (path, None);
        }
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.fingerprint.to_le_bytes());
        out.extend_from_slice(&(loaded.entries.len() as u32).to_le_bytes());
        for (key, bytes) in &loaded.entries {
            match *key {
                Key::Chunk(x, z) => {
                    out.push(0);
                    out.extend_from_slice(&x.to_le_bytes());
                    out.extend_from_slice(&z.to_le_bytes());
                }
                Key::Tile { x, z, stride } => {
                    out.push(1);
                    for v in [x, z, stride] {
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        (path, Some(out))
    }

    fn write(&self, (path, contents): (PathBuf, Option<Vec<u8>>)) {
        let Some(out) = contents else {
            let _ = std::fs::remove_file(&path);
            return;
        };
        let temporary = path.with_extension("tmp");
        let written = std::fs::create_dir_all(&self.dir)
            .and_then(|()| std::fs::write(&temporary, &out))
            .and_then(|()| std::fs::rename(&temporary, &path));
        if let Err(e) = written {
            log::warn!("Horizon: failed to save distant terrain: {e}");
        }
    }
}

impl Drop for LodStore {
    fn drop(&mut self) {
        self.flush();
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn i16(&mut self) -> Option<i16> {
        Some(i16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

fn encode(columns: &[Column]) -> Vec<u8> {
    let mut out = Vec::with_capacity(columns.len() * 32);
    out.extend_from_slice(&(columns.len() as u32).to_le_bytes());
    for column in columns {
        match column.surface {
            None => out.push(0),
            Some(s) => {
                out.push(if s.water.is_some() { 2 } else { 1 });
                out.extend_from_slice(&s.height.to_le_bytes());
                out.extend_from_slice(&s.state.0.to_le_bytes());
                out.extend_from_slice(&s.biome.to_le_bytes());
                if let Some((height, state)) = s.water {
                    out.extend_from_slice(&height.to_le_bytes());
                    out.extend_from_slice(&state.0.to_le_bytes());
                }
            }
        }
        out.extend_from_slice(&(column.spans.len() as u16).to_le_bytes());
        for span in &column.spans {
            out.extend_from_slice(&span.min.to_le_bytes());
            out.extend_from_slice(&span.max.to_le_bytes());
            out.extend_from_slice(&span.state.0.to_le_bytes());
            out.extend_from_slice(&span.biome.to_le_bytes());
            out.extend_from_slice(&span.light.to_le_bytes());
        }
    }
    out
}

fn decode(bytes: &[u8]) -> Option<Vec<Column>> {
    let mut r = Reader(bytes);
    let count = r.u32()? as usize;
    let mut columns = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        let surface = match r.u8()? {
            0 => None,
            kind => {
                let height = r.i32()?;
                let state = BlockStateId(r.u32()?);
                let biome = r.u16()?;
                let water = if kind == 2 {
                    Some((r.i32()?, BlockStateId(r.u32()?)))
                } else {
                    None
                };
                Some(Surface {
                    height,
                    state,
                    water,
                    biome,
                })
            }
        };
        let spans = (0..r.u16()?)
            .map(|_| {
                Some(Span {
                    min: r.i16()?,
                    max: r.i16()?,
                    state: BlockStateId(r.u32()?),
                    biome: r.u16()?,
                    light: r.u16()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        columns.push(Column { surface, spans });
    }
    Some(columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns(seed: u32) -> Vec<Column> {
        (0..256u32)
            .map(|i| Column {
                surface: (i % 7 != 0).then(|| Surface {
                    height: 60 + (i % 13) as i32,
                    state: BlockStateId(seed + i),
                    water: (i % 3 == 0).then_some((64, BlockStateId(9))),
                    biome: (i % 5) as u16,
                }),
                spans: (0..i % 4)
                    .map(|k| Span {
                        min: -64 + k as i16 * 10,
                        max: -60 + k as i16 * 10,
                        state: BlockStateId(k),
                        biome: 2,
                        light: 0x30f0,
                    })
                    .collect(),
            })
            .collect()
    }

    fn dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-lod-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn entries_survive_a_restart_and_a_forgotten_chunk_goes() {
        let dir = dir("restart");
        let tile = Key::Tile {
            x: -8192,
            z: 4096,
            stride: 64,
        };
        {
            let store = LodStore::new(dir.clone(), 100);
            store.put(Key::Chunk(-1, 40), &columns(1));
            store.put(Key::Chunk(-1, 41), &columns(2));
            store.put(tile, &columns(3));
            store.forget(-1, 41);
        }
        let store = LodStore::new(dir.clone(), 100);
        assert_eq!(store.get(Key::Chunk(-1, 40)), Some(columns(1)));
        assert_eq!(store.get(Key::Chunk(-1, 41)), None);
        assert_eq!(store.get(tile), Some(columns(3)));
        assert_eq!(store.get(Key::Chunk(5, 5)), None);
        // A summary read before its save changed is not kept.
        let before = store.generation();
        store.forget(-1, 40);
        store.put_since(Key::Chunk(-1, 40), &columns(4), before);
        assert_eq!(store.get(Key::Chunk(-1, 40)), None);
        store.put_since(Key::Chunk(-1, 40), &columns(4), store.generation());
        assert_eq!(store.get(Key::Chunk(-1, 40)), Some(columns(4)));
        // A tile made of chunks goes when one of them changes, and is not kept when one changed
        // while it was made.
        let fine = Key::Tile {
            x: -32,
            z: 640,
            stride: 2,
        };
        store.put(fine, &columns(6));
        assert_eq!(store.get(fine), Some(columns(6)));
        let before = store.generation();
        store.forget(-1, 41);
        assert_eq!(store.get(fine), None);
        store.put_since(fine, &columns(6), before);
        assert_eq!(store.get(fine), None);
        store.put_since(fine, &columns(6), store.generation());
        assert_eq!(store.get(fine), Some(columns(6)));
        // A loaded chunk's summary outlives the save queued after it, not the one after that.
        store.put_live(-1, 40, &columns(5));
        store.forget(-1, 40);
        assert_eq!(store.get(Key::Chunk(-1, 40)), Some(columns(5)));
        store.forget(-1, 40);
        assert_eq!(store.get(Key::Chunk(-1, 40)), None);
        // Another registry (another game build) does not read them.
        drop(store);
        assert_eq!(LodStore::new(dir.clone(), 101).get(tile), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_past_the_limit_are_written_before_they_go() {
        let dir = dir("evict");
        let store = LodStore::new(dir.clone(), 1);
        for i in 0..(LOADED as i32 + 4) {
            store.put(Key::Chunk(i * SPAN, 0), &columns(i as u32));
        }
        assert!(store.files.lock().unwrap().loaded.len() <= LOADED);
        for i in 0..(LOADED as i32 + 4) {
            assert_eq!(store.get(Key::Chunk(i * SPAN, 0)), Some(columns(i as u32)));
        }
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
