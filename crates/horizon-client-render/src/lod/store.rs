//! Far terrain kept on disk per world and dimension, so that a world visited
//! before shows its far view within seconds instead of sampling it again.
//!
//! The layout: sections of every level
//! kept per world, the identity of the world and of the block registry in the
//! key rather than the build, and lazy write-back through a bounded queue.
//! Here a section is a far tile (16x16 columns of any level) and the store is
//! append-only region files, a record deflated with the columns' own layout
//! (`cache::raw`).
//!
//! Two kinds of source fill it, by `pyramid`:
//! - A world with a generator (`StoredSampler`): the sampler's tiles are kept
//!   as they were sampled, and the summaries of chunks that were resident
//!   (`Lod::observe`) in a separate layer that overrides them in the finest
//!   tiles, as saved chunks do.
//! - A server without one (`RemoteSource`): every chunk received is the finest
//!   tile, and each coarser level is derived from it as chunks arrive (the
//!   most-occupied-wins merge of `Column::merge`), up to the whole world.
//!   Cells nothing was seen in stay unknown and tiles draw nothing there.
use super::{CELLS, Column, MAX_LEVEL, Surface, TileKey, cache};
use crate::{
    biome::Biomes,
    fast_hash::{FxHashMap, FxHashSet},
    streaming::{LodSampler, LodSource},
};
use horizon_world::registry::{BlockRegistry, BlockStateId};
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const MAGIC: &[u8; 4] = b"HFAR";
/// The record layout. A store of another format is cleared region by region.
const FORMAT: u32 = 1;
/// Bump when what a sampler or the summaries put in a tile changes (a new
/// column rule, a better generator), so that stale tiles are not kept: the
/// build itself does not clear the store, this does.
const REVISION: u32 = 1;
const HEADER: u64 = 16;
/// A record: chunk or tile x and z, the payload's length, flags, the hash.
const RECORD: u64 = 24;
/// A tile is a few tens of kilobytes deflated; anything larger is damage.
const MAX_RECORD: u32 = 1 << 22;
/// Region files kept open at once (a process may hold a few hundred).
const OPEN_REGIONS: usize = 48;
/// Dead bytes in a region file beyond its live ones that make it compact.
const COMPACT_SLACK: u64 = 4 << 20;
/// Chunk summaries and compressed tiles waiting for the writer; past them an
/// ingest is dropped (the chunk is summarized again when it next changes)
/// and a sampler thread writes its tile itself.
const MAX_QUEUED_CHUNKS: usize = 4096;
const MAX_QUEUED_TILES: usize = 256;
/// How long the writer sleeps with nothing to do, and how often it writes the
/// pyramid's changed tiles.
const IDLE: Duration = Duration::from_millis(500);
const FLUSH_EVERY: Duration = Duration::from_secs(2);
/// Pyramid tiles the writer keeps in memory (about 30 KiB each).
const PYRAMID_TILES: usize = 512;
/// Tiles of this level and up are sampled again when the pyramid changes
/// them: finer ones take the chunks' summaries (`Lod::apply_detail`).
const REFRESH_LEVEL: u8 = 5;
/// The columns a tile holds, all known.
const FULL: [u64; 4] = [u64::MAX; 4];
/// Flags of a record: a mask of the known columns precedes the columns;
/// sampled with Reduced far detail (an exact sample is better).
const PARTIAL: u32 = 1;
const APPROXIMATE: u32 = 2;

/// FNV-1a: the same in every process and build, which a store key and the
/// hashes in its records need (`DefaultHasher` promises neither).
struct Fnv(u64);
impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// What a store's files depend on: the layout, the sampling revision, the
/// block states and biomes (columns hold their ids) and the world.
pub fn store_key(registry: &BlockRegistry, biomes: &Biomes, world: u64) -> u64 {
    let mut hash = Fnv::new();
    hash.write(&FORMAT.to_le_bytes());
    hash.write(&REVISION.to_le_bytes());
    hash.write(&(registry.len() as u64).to_le_bytes());
    let mut previous = "";
    for id in 0..registry.len() {
        // States of a block are next to each other: its name is enough.
        if let Some(state) = registry.state(BlockStateId(id as u32))
            && state.name != previous
        {
            hash.write(state.name.as_bytes());
            hash.write(&[0]);
            previous = state.name.as_str();
        }
    }
    for biome in &biomes.list {
        hash.write(biome.name.as_bytes());
        hash.write(&[0]);
    }
    hash.write(&world.to_le_bytes());
    hash.0
}

/// A world on a server: its dimension and the hashed seed it sent.
pub fn world_id(dimension: &str, hashed_seed: i64) -> u64 {
    let mut hash = Fnv::new();
    hash.write(dimension.as_bytes());
    hash.write(&hashed_seed.to_le_bytes());
    hash.0
}

/// A directory name for text that names a server or a level.
pub fn dir_name(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// A tile as the store has it.
pub struct StoredTile {
    pub columns: Vec<Column>,
    /// Which columns were seen (bit `z * 16 + x`). Sampled tiles have all.
    pub known: [u64; 4],
    /// Sampled with Reduced far detail.
    pub approximate: bool,
}

/// Where a record lives: a layer (0 the chunk summaries, else a tile level
/// plus one), and the region of 32x32 entries in it.
type RegionId = (u8, i32, i32);
/// A record's own key: the chunk or tile x and z.
type At = (i32, i32);

fn tile_at(key: TileKey) -> (RegionId, At) {
    ((key.level + 1, key.x >> 5, key.z >> 5), (key.x, key.z))
}
fn chunk_at((x, z): At) -> (RegionId, At) {
    ((0, x >> 5, z >> 5), (x, z))
}

#[derive(Clone, Copy)]
struct Entry {
    offset: u64,
    len: u32,
    flags: u32,
    hash: u64,
}

struct Region {
    file: File,
    path: PathBuf,
    key: u64,
    /// Each entry's latest record.
    index: FxHashMap<At, Entry>,
    /// Bytes of the records the index points to, and of the ones it no
    /// longer does.
    live: u64,
    dead: u64,
    used: u64,
}

fn header(key: u64) -> Vec<u8> {
    let mut header = Vec::with_capacity(HEADER as usize);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&FORMAT.to_le_bytes());
    header.extend_from_slice(&key.to_le_bytes());
    header
}

fn record_header(at: At, entry: &Entry) -> [u8; RECORD as usize] {
    let mut record = [0; RECORD as usize];
    record[0..4].copy_from_slice(&at.0.to_le_bytes());
    record[4..8].copy_from_slice(&at.1.to_le_bytes());
    record[8..12].copy_from_slice(&entry.len.to_le_bytes());
    record[12..16].copy_from_slice(&entry.flags.to_le_bytes());
    record[16..24].copy_from_slice(&entry.hash.to_le_bytes());
    record
}

impl Region {
    fn read(&mut self, at: At) -> Option<(Entry, Vec<u8>)> {
        let entry = *self.index.get(&at)?;
        let mut bytes = vec![0; entry.len as usize];
        self.file.seek(SeekFrom::Start(entry.offset)).ok()?;
        self.file.read_exact(&mut bytes).ok()?;
        Some((entry, bytes))
    }

    fn append(&mut self, at: At, flags: u32, hash: u64, payload: &[u8]) {
        if payload.len() > MAX_RECORD as usize {
            return;
        }
        let entry = Entry {
            offset: 0,
            len: payload.len() as u32,
            flags,
            hash,
        };
        let mut record = Vec::with_capacity(RECORD as usize + payload.len());
        record.extend_from_slice(&record_header(at, &entry));
        record.extend_from_slice(payload);
        let Ok(end) = self.file.seek(SeekFrom::End(0)) else {
            return;
        };
        // One write, so a crash leaves at most a torn last record, which
        // the next open cuts off.
        if self.file.write_all(&record).is_err() {
            let _ = self.file.set_len(end);
            return;
        }
        let size = RECORD + u64::from(entry.len);
        self.live += size;
        if let Some(old) = self.index.insert(
            at,
            Entry {
                offset: end + RECORD,
                ..entry
            },
        ) {
            let old = RECORD + u64::from(old.len);
            self.live -= old;
            self.dead += old;
        }
        if self.dead > self.live + COMPACT_SLACK {
            self.compact();
        }
    }

    /// Rewrites the file with the live records only.
    fn compact(&mut self) -> Option<()> {
        let temporary = self.path.with_extension("tmp");
        let mut writer = BufWriter::new(File::create(&temporary).ok()?);
        writer.write_all(&header(self.key)).ok()?;
        let mut index = FxHashMap::default();
        let mut offset = HEADER;
        for (&at, entry) in &self.index {
            let mut payload = vec![0; entry.len as usize];
            self.file.seek(SeekFrom::Start(entry.offset)).ok()?;
            self.file.read_exact(&mut payload).ok()?;
            writer.write_all(&record_header(at, entry)).ok()?;
            writer.write_all(&payload).ok()?;
            index.insert(
                at,
                Entry {
                    offset: offset + RECORD,
                    ..*entry
                },
            );
            offset += RECORD + u64::from(entry.len);
        }
        writer.flush().ok()?;
        drop(writer);
        // The old handle goes first: a file cannot be replaced while open on
        // every platform.
        self.file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&temporary)
            .ok()?;
        std::fs::rename(&temporary, &self.path).ok()?;
        self.index = index;
        self.live = offset - HEADER;
        self.dead = 0;
        Some(())
    }
}

#[derive(Default)]
struct Files {
    regions: FxHashMap<RegionId, Option<Region>>,
    clock: u64,
}

/// A record on its way to a file, already deflated.
struct Record {
    region: RegionId,
    at: At,
    flags: u32,
    hash: u64,
    payload: Vec<u8>,
}

#[derive(Default)]
struct Work {
    /// The latest summary of each chunk, written once however often it
    /// changed meanwhile.
    chunks: FxHashMap<At, Arc<[Column; 256]>>,
    /// Tiles a server supplied (`FarStore::supply_tile`).
    supplied: Vec<(TileKey, Arc<Vec<Column>>, [u64; 4])>,
    tiles: VecDeque<Record>,
}
impl Work {
    fn is_empty(&self) -> bool {
        self.chunks.is_empty() && self.supplied.is_empty() && self.tiles.is_empty()
    }
}

struct Inner {
    dir: PathBuf,
    key: u64,
    pyramid: bool,
    // ponytail: one lock for every region, with file reads under it (as
    // `cache::DiskCache`); per-region locks if the sampling threads ever wait
    // on it.
    files: Mutex<Files>,
    work: Mutex<Work>,
    wake: Condvar,
    stop: AtomicBool,
    /// Tiles the pyramid wrote that a `Lod` may have sampled before.
    changed: Mutex<FxHashSet<TileKey>>,
}

impl Inner {
    fn path(&self, (layer, rx, rz): RegionId) -> PathBuf {
        let name = format!("r.{rx}.{rz}.hfar");
        match layer {
            0 => self.dir.join("chunks").join(name),
            level => self.dir.join(format!("level{:02}", level - 1)).join(name),
        }
    }

    fn region<'a>(
        &self,
        files: &'a mut Files,
        id: RegionId,
        create: bool,
    ) -> Option<&'a mut Region> {
        files.clock += 1;
        let clock = files.clock;
        if create && matches!(files.regions.get(&id), Some(None)) {
            files.regions.remove(&id);
        }
        if !files.regions.contains_key(&id) {
            if files.regions.len() >= OPEN_REGIONS {
                let oldest = files
                    .regions
                    .iter()
                    .min_by_key(|(_, region)| region.as_ref().map_or(0, |region| region.used))
                    .map(|(id, _)| *id);
                if let Some(oldest) = oldest {
                    files.regions.remove(&oldest);
                }
            }
            let opened = self.open(id, create);
            files.regions.insert(id, opened);
        }
        let region = files.regions.get_mut(&id)?.as_mut()?;
        region.used = clock;
        Some(region)
    }

    fn open(&self, id: RegionId, create: bool) -> Option<Region> {
        let path = self.path(id);
        if !create && !path.exists() {
            return None;
        }
        if create {
            std::fs::create_dir_all(path.parent()?).ok()?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false)
            .open(&path)
            .ok()?;
        let mut head = [0; HEADER as usize];
        let current = file.read_exact(&mut head).is_ok()
            && head[..4] == *MAGIC
            && head[4..8] == FORMAT.to_le_bytes()
            && head[8..] == self.key.to_le_bytes();
        let mut index = FxHashMap::default();
        let mut dead = 0;
        if current {
            let size = file.metadata().ok()?.len();
            let mut at = HEADER;
            {
                let mut reader = BufReader::new(&file);
                reader.seek(SeekFrom::Start(HEADER)).ok()?;
                let mut record = [0; RECORD as usize];
                while at + RECORD <= size {
                    if reader.read_exact(&mut record).is_err() {
                        break;
                    }
                    let field = |i: usize| u32::from_le_bytes(record[i..i + 4].try_into().unwrap());
                    let len = field(8);
                    if len > MAX_RECORD || at + RECORD + u64::from(len) > size {
                        break;
                    }
                    let entry = Entry {
                        offset: at + RECORD,
                        len,
                        flags: field(12),
                        hash: u64::from_le_bytes(record[16..24].try_into().unwrap()),
                    };
                    if let Some(old) = index.insert((field(0) as i32, field(4) as i32), entry) {
                        dead += RECORD + u64::from(old.len);
                    }
                    at += RECORD + u64::from(len);
                    if reader.seek_relative(i64::from(len)).is_err() {
                        break;
                    }
                }
            }
            if at < size {
                file.set_len(at).ok()?;
            }
        } else if create {
            file.set_len(0).ok()?;
            file.seek(SeekFrom::Start(0)).ok()?;
            file.write_all(&header(self.key)).ok()?;
        } else {
            // Another build's or world's file: cleared when written to.
            return None;
        }
        let live: u64 = index.values().map(|e| RECORD + u64::from(e.len)).sum();
        let mut region = Region {
            file,
            path,
            key: self.key,
            index,
            live,
            dead,
            used: 0,
        };
        if region.dead > region.live + COMPACT_SLACK {
            region.compact();
        }
        Some(region)
    }

    fn entry(&self, (id, at): (RegionId, At)) -> Option<Entry> {
        let mut files = self.files.lock().ok()?;
        self.region(&mut files, id, false)?.index.get(&at).copied()
    }

    fn read(&self, (id, at): (RegionId, At)) -> Option<(Entry, Vec<u8>)> {
        let mut files = self.files.lock().ok()?;
        self.region(&mut files, id, false)?.read(at)
    }

    fn write(&self, record: Record) {
        let Ok(mut files) = self.files.lock() else {
            return;
        };
        if let Some(region) = self.region(&mut files, record.region, true) {
            region.append(record.at, record.flags, record.hash, &record.payload);
        }
    }

    /// Writes a tile unless the store has these very columns already.
    fn put(&self, place: (RegionId, At), columns: &[Column], known: [u64; 4], approximate: bool) {
        let Some(record) = self.pack(place, columns, known, approximate) else {
            return;
        };
        self.write(record);
    }

    fn pack(
        &self,
        (region, at): (RegionId, At),
        columns: &[Column],
        known: [u64; 4],
        approximate: bool,
    ) -> Option<Record> {
        let raw = cache::raw(columns);
        let flags =
            (if known == FULL { 0 } else { PARTIAL }) | (if approximate { APPROXIMATE } else { 0 });
        let mut hash = Fnv::new();
        hash.write(&raw);
        hash.write(&flags.to_le_bytes());
        if known != FULL {
            for word in known {
                hash.write(&word.to_le_bytes());
            }
        }
        let hash = hash.0;
        if self
            .entry((region, at))
            .is_some_and(|entry| entry.hash == hash)
        {
            return None;
        }
        let mut payload = Vec::new();
        if flags & PARTIAL != 0 {
            for word in known {
                payload.extend_from_slice(&word.to_le_bytes());
            }
        }
        payload.extend_from_slice(&cache::compress(&raw));
        Some(Record {
            region,
            at,
            flags,
            hash,
            payload,
        })
    }

    fn tile(&self, key: TileKey) -> Option<StoredTile> {
        let (entry, bytes) = self.read(tile_at(key))?;
        let (known, body) = if entry.flags & PARTIAL != 0 {
            let mask = bytes.get(..32)?;
            let word = |i: usize| u64::from_le_bytes(mask[i * 8..i * 8 + 8].try_into().unwrap());
            ([word(0), word(1), word(2), word(3)], &bytes[32..])
        } else {
            (FULL, &bytes[..])
        };
        Some(StoredTile {
            columns: Vec::from(cache::decode(body)?),
            known,
            approximate: entry.flags & APPROXIMATE != 0,
        })
    }
}

/// The far-terrain store of one world and dimension: tiles read by sampler
/// threads, chunk summaries fed from the render thread, and one writer
/// thread (`run`) that does the disk work behind both.
pub struct FarStore {
    inner: Arc<Inner>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
}

/// The stores open now, by directory: the sampler threads of a `Lod`, the
/// `Lod` and a later one for the same world share one writer.
static OPEN: Mutex<Vec<(PathBuf, Weak<FarStore>)>> = Mutex::new(Vec::new());

impl FarStore {
    /// The store in `dir` (made when something is written). `key`: what its
    /// files depend on (`store_key`); files with another are cleared.
    /// `pyramid`: coarser levels are derived from the chunks that come in
    /// (a server), rather than sampled.
    pub fn open(dir: PathBuf, key: u64, pyramid: bool) -> Arc<Self> {
        let mut open = OPEN.lock().unwrap();
        open.retain(|(_, store)| store.strong_count() > 0);
        if let Some(store) = open
            .iter()
            .find(|(at, _)| *at == dir)
            .and_then(|(_, store)| store.upgrade())
            && store.inner.key == key
            && store.inner.pyramid == pyramid
        {
            return store;
        }
        let inner = Arc::new(Inner {
            dir: dir.clone(),
            key,
            pyramid,
            files: Mutex::default(),
            work: Mutex::default(),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            changed: Mutex::default(),
        });
        let worker = inner.clone();
        let thread = thread::Builder::new()
            .name("Far store".into())
            .spawn(move || run(worker))
            .expect("far store writer");
        let store = Arc::new(Self {
            inner,
            thread: Mutex::new(Some(thread)),
        });
        open.retain(|(at, _)| *at != dir);
        open.push((dir, Arc::downgrade(&store)));
        store
    }

    /// Whether this store's source has no generator (`open`).
    pub fn is_pyramid(&self) -> bool {
        self.inner.pyramid
    }

    /// The tile the store has at `key`.
    pub fn tile(&self, key: TileKey) -> Option<StoredTile> {
        self.inner.tile(key)
    }

    /// Keeps a tile a sampler made, once the writer gets to it (or at once
    /// when it is far behind). Not for a pyramid store, which derives its own.
    pub fn put_sampled(&self, key: TileKey, columns: &[Column], approximate: bool) {
        if columns.len() != CELLS * CELLS {
            return;
        }
        let Some(record) = self.inner.pack(tile_at(key), columns, FULL, approximate) else {
            return;
        };
        let mut work = self.inner.work.lock().unwrap();
        if work.tiles.len() < MAX_QUEUED_TILES {
            work.tiles.push_back(record);
            drop(work);
            self.inner.wake.notify_one();
        } else {
            drop(work);
            self.inner.write(record);
        }
    }

    /// The summary of chunk (x, z) kept by an earlier visit.
    pub fn visited(&self, x: i32, z: i32) -> Option<Arc<[Column; 256]>> {
        let (_, bytes) = self.inner.read(chunk_at((x, z)))?;
        cache::decode(&bytes).map(Arc::new)
    }

    /// A resident chunk's summary (`Lod::finish_observed`), from the render
    /// thread: it only queues it. Dropped when the writer is far behind.
    pub fn ingest(&self, chunk: (i32, i32), columns: Arc<[Column; 256]>) {
        let mut work = self.inner.work.lock().unwrap();
        if work.chunks.len() >= MAX_QUEUED_CHUNKS && !work.chunks.contains_key(&chunk) {
            return;
        }
        work.chunks.insert(chunk, columns);
        drop(work);
        self.inner.wake.notify_one();
    }

    /// Where a server that sends far tiles of its own (a Horizon server's
    /// `horizon:lod` channel) puts them: they fill the columns nothing was
    /// seen in, and coarser levels are derived from them as from chunks.
    /// `known`: which of the columns it has (bit `z * 16 + x`).
    pub fn supply_tile(&self, key: TileKey, columns: Arc<Vec<Column>>, known: [u64; 4]) {
        if !self.inner.pyramid || columns.len() != CELLS * CELLS {
            return;
        }
        let mut work = self.inner.work.lock().unwrap();
        if work.supplied.len() < MAX_QUEUED_TILES {
            work.supplied.push((key, columns, known));
            drop(work);
            self.inner.wake.notify_one();
        }
    }

    /// The tiles of the coarse levels the pyramid has written since the last
    /// call: a `Lod` that sampled them earlier samples them again.
    pub fn take_changed(&self) -> Vec<TileKey> {
        let mut changed = self.inner.changed.lock().unwrap();
        if changed.is_empty() {
            return Vec::new();
        }
        changed.drain().collect()
    }
}

impl Drop for FarStore {
    fn drop(&mut self) {
        // Under the work lock: the writer between its stop check and its
        // wait must not miss the wake, or join() hangs.
        let work = self.inner.work.lock();
        self.inner.stop.store(true, Ordering::Relaxed);
        drop(work);
        self.inner.wake.notify_all();
        let thread = self.thread.lock().ok().and_then(|mut thread| thread.take());
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

/// The writer: queued tiles to the files, chunk summaries to their layer (or
/// into the pyramid), the pyramid's changed tiles every `FLUSH_EVERY`, and
/// everything left when the store is dropped.
fn run(inner: Arc<Inner>) {
    horizon_threads::behind_server();
    let mut pyramid = Pyramid::default();
    let mut flushed = Instant::now();
    loop {
        let (chunks, supplied, tiles) = {
            let mut work = inner.work.lock().unwrap();
            while work.is_empty() && !inner.stop.load(Ordering::Relaxed) {
                let (guard, timeout) = inner.wake.wait_timeout(work, IDLE).unwrap();
                work = guard;
                if timeout.timed_out() {
                    break;
                }
            }
            (
                std::mem::take(&mut work.chunks),
                std::mem::take(&mut work.supplied),
                std::mem::take(&mut work.tiles),
            )
        };
        let stopping = inner.stop.load(Ordering::Relaxed);
        for record in tiles {
            inner.write(record);
        }
        for (chunk, columns) in chunks {
            if inner.pyramid {
                pyramid.set_chunk(&inner, chunk, &columns);
            } else {
                inner.put(chunk_at(chunk), &columns[..], FULL, false);
            }
        }
        for (key, columns, known) in supplied {
            pyramid.supply(&inner, key, &columns, known);
        }
        if !pyramid.dirty.is_empty() && (stopping || flushed.elapsed() >= FLUSH_EVERY) {
            pyramid.flush(&inner);
            flushed = Instant::now();
        }
        if stopping && inner.work.lock().unwrap().is_empty() {
            break;
        }
    }
}

/// A tile being derived: its columns and which of them are known.
struct PyTile {
    columns: Vec<Column>,
    known: [u64; 4],
    used: u64,
}
impl PyTile {
    fn empty() -> Self {
        Self {
            columns: vec![Column::default(); CELLS * CELLS],
            known: [0; 4],
            used: 0,
        }
    }
    fn knows(&self, index: usize) -> bool {
        self.known[index / 64] & (1 << (index % 64)) != 0
    }
    fn learn(&mut self, index: usize) {
        self.known[index / 64] |= 1 << (index % 64);
    }
}

/// A rectangle of a tile's columns, `[x0, z0, x1, z1)`.
type Rect = [usize; 4];

fn grow(rect: &mut Option<Rect>, x: usize, z: usize) {
    *rect = Some(match *rect {
        None => [x, z, x + 1, z + 1],
        Some([x0, z0, x1, z1]) => [x0.min(x), z0.min(z), x1.max(x + 1), z1.max(z + 1)],
    });
}

/// Recomputes the parent columns above the child's `rect` that changed, each
/// from the known columns among the four below it. The child tile is the
/// parent's `quadrant` (x, z in 0..2): its 8x8 columns' worth. Returns what
/// changed in the parent.
fn merge_up(
    parent: &mut PyTile,
    child: &PyTile,
    (qx, qz): (usize, usize),
    rect: Rect,
) -> Option<Rect> {
    let mut changed = None;
    for pz in rect[1] / 2..=(rect[3] - 1) / 2 {
        for px in rect[0] / 2..=(rect[2] - 1) / 2 {
            let below: Vec<&Column> = [(0, 0), (1, 0), (0, 1), (1, 1)]
                .into_iter()
                .map(|(dx, dz)| (pz * 2 + dz) * CELLS + px * 2 + dx)
                .filter(|&index| child.knows(index))
                .map(|index| &child.columns[index])
                .collect();
            if below.is_empty() {
                continue;
            }
            let column = Column::merge(below.iter().copied(), below.len());
            let (x, z) = (qx * 8 + px, qz * 8 + pz);
            let index = z * CELLS + x;
            if parent.knows(index) && parent.columns[index] == column {
                continue;
            }
            parent.columns[index] = column;
            parent.learn(index);
            grow(&mut changed, x, z);
        }
    }
    changed
}

/// The tiles of every level a server's chunks add up to, in memory until the
/// writer saves them. Only the writer thread uses it.
#[derive(Default)]
struct Pyramid {
    tiles: FxHashMap<TileKey, PyTile>,
    dirty: FxHashSet<TileKey>,
    clock: u64,
}
impl Pyramid {
    fn take(&mut self, inner: &Inner, key: TileKey) -> PyTile {
        self.tiles.remove(&key).unwrap_or_else(|| {
            inner
                .tile(key)
                .map(|stored| PyTile {
                    columns: stored.columns,
                    known: stored.known,
                    used: 0,
                })
                .unwrap_or_else(PyTile::empty)
        })
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// A chunk's summary: the finest tile, then every level above it as far
    /// as it changes anything.
    fn set_chunk(&mut self, inner: &Inner, (x, z): At, columns: &[Column; 256]) {
        let key = TileKey { x, z, level: 0 };
        let mut tile = self.take(inner, key);
        let same = tile.known == FULL && tile.columns[..] == columns[..];
        if !same {
            tile.columns = columns.to_vec();
            tile.known = FULL;
        }
        tile.used = self.tick();
        self.tiles.insert(key, tile);
        if !same {
            self.dirty.insert(key);
            self.propagate(inner, key, [0, 0, CELLS, CELLS]);
        }
    }

    /// A server's tile: fills the columns not known here.
    fn supply(&mut self, inner: &Inner, key: TileKey, columns: &[Column], known: [u64; 4]) {
        if key.level > MAX_LEVEL {
            return;
        }
        let mut tile = self.take(inner, key);
        let mut changed = None;
        for index in 0..CELLS * CELLS {
            if known[index / 64] & (1 << (index % 64)) != 0
                && !tile.knows(index)
                && let Some(column) = columns.get(index)
            {
                tile.columns[index] = column.clone();
                tile.learn(index);
                grow(&mut changed, index % CELLS, index / CELLS);
            }
        }
        tile.used = self.tick();
        self.tiles.insert(key, tile);
        if let Some(rect) = changed {
            self.dirty.insert(key);
            self.propagate(inner, key, rect);
        }
    }

    fn propagate(&mut self, inner: &Inner, mut key: TileKey, mut rect: Rect) {
        while key.level < MAX_LEVEL {
            let parent_key = key.parent();
            let quadrant = (key.x.rem_euclid(2) as usize, key.z.rem_euclid(2) as usize);
            let mut parent = self.take(inner, parent_key);
            let changed = self
                .tiles
                .get(&key)
                .and_then(|child| merge_up(&mut parent, child, quadrant, rect));
            parent.used = self.tick();
            self.tiles.insert(parent_key, parent);
            // Nothing changed up here: nothing does above.
            let Some(next) = changed else {
                break;
            };
            self.dirty.insert(parent_key);
            key = parent_key;
            rect = next;
        }
    }

    /// Writes the changed tiles and tells `take_changed`, then drops the
    /// least recently used tiles past `PYRAMID_TILES`.
    fn flush(&mut self, inner: &Inner) {
        let mut changed = Vec::new();
        for key in std::mem::take(&mut self.dirty) {
            let Some(tile) = self.tiles.get(&key) else {
                continue;
            };
            inner.put(tile_at(key), &tile.columns, tile.known, false);
            if key.level >= REFRESH_LEVEL {
                changed.push(key);
            }
        }
        if !changed.is_empty() {
            inner.changed.lock().unwrap().extend(changed);
        }
        if self.tiles.len() > PYRAMID_TILES {
            let mut used: Vec<u64> = self.tiles.values().map(|tile| tile.used).collect();
            let middle = used.len() / 2;
            let (_, &mut oldest, _) = used.select_nth_unstable(middle);
            self.tiles.retain(|_, tile| tile.used >= oldest);
        }
    }
}

/// Wraps the sampler of a world that has a generator: a tile the store has is
/// read instead of sampled, one it does not is stored after sampling, and
/// the finest tiles take the summaries of chunks seen before (so edits made
/// in resident chunks, and what generation added there, survive the
/// session as saved chunks do).
pub struct StoredSampler {
    inner: Box<dyn LodSampler>,
    store: Arc<FarStore>,
    approximate: bool,
    /// Chunk summaries read lately, newest first.
    visited: VecDeque<(At, Arc<[Column; 256]>)>,
}

/// As `SavedLodSampler`'s limits: the coarsest cells that read single chunks,
/// at Full and at Reduced far detail.
const VISITED_STRIDE: i32 = 16;
const VISITED_STRIDE_REDUCED: i32 = 4;

impl StoredSampler {
    pub fn new(inner: Box<dyn LodSampler>, store: Arc<FarStore>) -> Self {
        Self {
            inner,
            store,
            approximate: false,
            visited: VecDeque::new(),
        }
    }

    fn chunk(&mut self, at: At) -> Option<Arc<[Column; 256]>> {
        if let Some(index) = self.visited.iter().position(|(key, _)| *key == at) {
            let found = self.visited.remove(index)?;
            let columns = found.1.clone();
            self.visited.push_front(found);
            return Some(columns);
        }
        let columns = self.store.visited(at.0, at.1)?;
        self.visited.push_front((at, columns.clone()));
        self.visited.truncate(128);
        Some(columns)
    }
}

/// The tile a `LodSampler::grid` call asks for: a whole tile of 16x16 cells.
fn grid_tile(x: i32, z: i32, stride: i32, side: usize) -> Option<TileKey> {
    if side != CELLS || stride < 1 || stride.count_ones() != 1 {
        return None;
    }
    let level = stride.trailing_zeros() as u8;
    if level > MAX_LEVEL {
        return None;
    }
    let size = stride * CELLS as i32;
    let key = TileKey {
        x: x.div_euclid(size),
        z: z.div_euclid(size),
        level,
    };
    (key.origin()[0] == x && key.origin()[2] == z).then_some(key)
}

impl LodSampler for StoredSampler {
    fn fork(&self) -> Option<Box<dyn LodSampler>> {
        Some(Box::new(Self::new(self.inner.fork()?, self.store.clone())))
    }
    fn request_detail(&mut self, chunks: &[(i32, i32)], center: [i32; 2]) {
        self.inner.request_detail(chunks, center);
    }
    fn drain_details(&mut self) -> Vec<crate::streaming::LodDetail> {
        self.inner.drain_details()
    }
    fn set_approximate(&mut self, approximate: bool) {
        self.approximate = approximate;
        self.inner.set_approximate(approximate);
    }
    fn sample(&mut self, x: i32, z: i32) -> Option<Surface> {
        self.inner.sample(x, z)
    }
    fn column(&mut self, x: i32, z: i32) -> Column {
        self.inner.column(x, z)
    }
    fn reduced_column(&mut self, x: i32, z: i32, width: i32) -> Column {
        self.inner.reduced_column(x, z, width)
    }
    fn grid(&mut self, x: i32, z: i32, stride: i32, side: usize) -> Vec<Column> {
        let Some(key) = grid_tile(x, z, stride, side) else {
            return self.inner.grid(x, z, stride, side);
        };
        // An approximate tile is only good enough for Reduced far detail.
        let stored = self
            .store
            .tile(key)
            .filter(|tile| tile.columns.len() == CELLS * CELLS)
            .filter(|tile| self.approximate || !tile.approximate);
        let mut columns = match stored {
            Some(tile) => tile.columns,
            None => {
                let columns = self.inner.grid(x, z, stride, side);
                self.store.put_sampled(key, &columns, self.approximate);
                columns
            }
        };
        let limit = if self.approximate {
            VISITED_STRIDE_REDUCED
        } else {
            VISITED_STRIDE
        };
        if stride <= limit {
            for (index, column) in columns.iter_mut().enumerate() {
                let (wx, wz) = (
                    x + (index % CELLS) as i32 * stride,
                    z + (index / CELLS) as i32 * stride,
                );
                if let Some(chunk) = self.chunk((wx >> 4, wz >> 4)) {
                    *column = Column::reduced(
                        &chunk,
                        (wx & 15) as usize,
                        (wz & 15) as usize,
                        stride as usize,
                    );
                }
            }
        }
        columns
    }
}

/// The far terrain of a server: no generator, only what its chunks showed.
pub struct RemoteSource {
    store: Arc<FarStore>,
}
impl RemoteSource {
    pub fn new(store: Arc<FarStore>) -> Self {
        Self { store }
    }
}
impl LodSource for RemoteSource {
    fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
        Box::new(RemoteSampler {
            store: self.store.clone(),
        })
    }
    fn store(&self) -> Option<Arc<FarStore>> {
        Some(self.store.clone())
    }
}

struct RemoteSampler {
    store: Arc<FarStore>,
}
impl LodSampler for RemoteSampler {
    fn fork(&self) -> Option<Box<dyn LodSampler>> {
        Some(Box::new(Self {
            store: self.store.clone(),
        }))
    }
    fn sample(&mut self, _x: i32, _z: i32) -> Option<Surface> {
        None
    }
    fn grid(&mut self, x: i32, z: i32, stride: i32, side: usize) -> Vec<Column> {
        grid_tile(x, z, stride, side)
            .and_then(|key| self.store.tile(key))
            .map(|tile| tile.columns)
            .filter(|columns| columns.len() == side * side)
            .unwrap_or_else(|| vec![Column::default(); side * side])
    }
}

/// The directory of a server's far terrain in the game directory.
pub fn server_dir(game_dir: &Path, address: &str, dimension: &str) -> PathBuf {
    game_dir
        .join(".horizon-cache")
        .join("far-terrain")
        .join(dir_name(address))
        .join(dir_name(dimension))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lod::Span;

    fn columns(seed: u32) -> [Column; 256] {
        std::array::from_fn(|i| {
            let i = i as u32 + seed;
            Column {
                surface: None,
                spans: (0..1 + i % 3)
                    .map(|n| Span {
                        min: (n * 10) as i16 - 64,
                        max: (n * 10 + 6) as i16 - 64,
                        state: BlockStateId(1 + (n + i) % 5),
                        biome: 1,
                        light: 240,
                    })
                    .collect(),
            }
        })
    }

    fn dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-far-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn grid_calls_name_whole_tiles_only() {
        assert_eq!(
            grid_tile(-512, 1024, 4, 16),
            Some(TileKey {
                x: -8,
                z: 16,
                level: 2
            })
        );
        assert_eq!(
            grid_tile(0, 0, 1, 16),
            Some(TileKey {
                x: 0,
                z: 0,
                level: 0
            })
        );
        assert_eq!(grid_tile(16, 0, 4, 16), None, "not on a tile's corner");
        assert_eq!(grid_tile(0, 0, 3, 16), None, "not a power of two");
        assert_eq!(grid_tile(0, 0, 2, 8), None, "not 16 cells wide");
    }

    #[test]
    fn tiles_survive_reopening_and_unchanged_ones_are_not_rewritten() {
        let dir = dir("tiles");
        let key = TileKey {
            x: -3,
            z: 40,
            level: 2,
        };
        let store = FarStore::open(dir.clone(), 7, false);
        assert!(store.tile(key).is_none());
        store.put_sampled(key, &columns(0), true);
        store.put_sampled(key, &columns(0), true);
        drop(store);
        let path = dir.join("level02").join("r.-1.1.hfar");
        let size = std::fs::metadata(&path).unwrap().len();

        let store = FarStore::open(dir.clone(), 7, false);
        let tile = store.tile(key).unwrap();
        assert_eq!(tile.columns, columns(0).to_vec());
        assert_eq!(tile.known, FULL);
        assert!(tile.approximate);
        // The same columns again change nothing; others replace them.
        store.put_sampled(key, &columns(0), true);
        drop(store);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), size);
        let store = FarStore::open(dir.clone(), 7, false);
        store.put_sampled(key, &columns(1), false);
        drop(store);
        let store = FarStore::open(dir.clone(), 7, false);
        let tile = store.tile(key).unwrap();
        assert_eq!(tile.columns, columns(1).to_vec());
        assert!(!tile.approximate);
        drop(store);

        // Another world's or build's files are not read.
        let store = FarStore::open(dir.clone(), 8, false);
        assert!(store.tile(key).is_none());
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn visited_chunks_and_a_torn_last_record() {
        let dir = dir("visited");
        let store = FarStore::open(dir.clone(), 7, false);
        store.ingest((5, -33), Arc::new(columns(0)));
        store.ingest((6, -33), Arc::new(columns(1)));
        drop(store);
        let path = dir.join("chunks").join("r.0.-2.hfar");
        let size = std::fs::metadata(&path).unwrap().len();
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(size + 7).unwrap();
        drop(file);

        let store = FarStore::open(dir.clone(), 7, false);
        assert_eq!(*store.visited(5, -33).unwrap(), columns(0));
        assert_eq!(*store.visited(6, -33).unwrap(), columns(1));
        assert!(store.visited(7, -33).is_none());
        drop(store);
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            size,
            "torn tail cut"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn regions_compact_to_their_live_records() {
        let dir = dir("compact");
        let key = TileKey {
            x: 0,
            z: 0,
            level: 0,
        };
        let store = FarStore::open(dir.clone(), 7, false);
        for seed in 0..10 {
            store.put_sampled(key, &columns(seed), false);
        }
        drop(store);
        let path = dir.join("level00").join("r.0.0.hfar");
        let before = std::fs::metadata(&path).unwrap().len();

        let store = FarStore::open(dir.clone(), 7, false);
        {
            let mut files = store.inner.files.lock().unwrap();
            let region = store
                .inner
                .region(&mut files, tile_at(key).0, false)
                .unwrap();
            assert!(region.dead > 0, "rewritten records are dead ones");
            region.compact().unwrap();
            assert_eq!(region.dead, 0);
        }
        assert_eq!(store.tile(key).unwrap().columns, columns(9).to_vec());
        drop(store);
        assert!(std::fs::metadata(&path).unwrap().len() < before);
        let store = FarStore::open(dir.clone(), 7, false);
        assert_eq!(store.tile(key).unwrap().columns, columns(9).to_vec());
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chunks_add_up_to_coarser_tiles_with_unknown_columns_left_out() {
        let dir = dir("pyramid");
        let store = FarStore::open(dir.clone(), 7, true);
        // A chunk of solid ground and the one east of it, nothing elsewhere.
        let ground = |state: u32| -> [Column; 256] {
            std::array::from_fn(|_| Column {
                surface: None,
                spans: vec![Span {
                    min: 0,
                    max: 60,
                    state: BlockStateId(state),
                    biome: 1,
                    light: 240,
                }]
                .into(),
            })
        };
        store.ingest((4, 6), Arc::new(ground(3)));
        store.ingest((5, 6), Arc::new(ground(4)));
        drop(store);

        let store = FarStore::open(dir.clone(), 7, true);
        let level0 = store
            .tile(TileKey {
                x: 4,
                z: 6,
                level: 0,
            })
            .unwrap();
        assert_eq!(level0.known, FULL);
        // Level 1: the tile (2, 3) holds both chunks, one in each of its
        // halves along x; the rest of it is unknown.
        let level1 = store
            .tile(TileKey {
                x: 2,
                z: 3,
                level: 1,
            })
            .unwrap();
        let spot = |x: usize, z: usize| z * 16 + x;
        for z in 0..8 {
            for x in 0..8 {
                // This chunk is quadrant (0, 0) of its parent, the next
                // one the parent's... (5, 6) is quadrant (1, 0) of (2, 3).
                assert!(level1.known[spot(x, z) / 64] & (1 << (spot(x, z) % 64)) != 0);
                assert_eq!(level1.columns[spot(x, z)].spans[0].state, BlockStateId(3));
                assert_eq!(
                    level1.columns[spot(x + 8, z)].spans[0].state,
                    BlockStateId(4)
                );
            }
        }
        assert!(level1.known[spot(0, 8) / 64] & (1 << (spot(0, 8) % 64)) == 0);
        assert!(level1.columns[spot(0, 8)].spans.is_empty());
        // The top level exists and knows the same ground.
        let top = store
            .tile(TileKey {
                x: 0,
                z: 0,
                level: MAX_LEVEL,
            })
            .unwrap();
        assert!(top.known.iter().any(|&word| word != 0));
        // A chunk seen again with the same columns changes nothing; a
        // changed one reaches the levels above.
        store.ingest((4, 6), Arc::new(ground(3)));
        store.ingest((4, 6), Arc::new(ground(9)));
        drop(store);
        let store = FarStore::open(dir.clone(), 7, true);
        let level1 = store
            .tile(TileKey {
                x: 2,
                z: 3,
                level: 1,
            })
            .unwrap();
        assert_eq!(level1.columns[0].spans[0].state, BlockStateId(9));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_server_tile_fills_unknown_columns_only() {
        let dir = dir("supply");
        let store = FarStore::open(dir.clone(), 7, true);
        let place = |state: u32| Column {
            surface: None,
            spans: vec![Span {
                min: 0,
                max: 30,
                state: BlockStateId(state),
                biome: 1,
                light: 240,
            }]
            .into(),
        };
        let key = TileKey {
            x: 1,
            z: 1,
            level: 3,
        };
        let mut known = [0u64; 4];
        known[0] = 0b11;
        store.supply_tile(key, Arc::new(vec![place(2); 256]), known);
        drop(store);
        let store = FarStore::open(dir.clone(), 7, true);
        let tile = store.tile(key).unwrap();
        assert_eq!(tile.known[0], 0b11);
        assert_eq!(tile.columns[1].spans[0].state, BlockStateId(2));
        assert!(tile.columns[2].spans.is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_make_directories() {
        assert_eq!(dir_name("Play.Example.com:25565"), "play.example.com_25565");
        assert_eq!(dir_name("minecraft:the_nether"), "minecraft_the_nether");
        assert_ne!(
            world_id("minecraft:overworld", 1),
            world_id("minecraft:overworld", 2)
        );
    }
}
