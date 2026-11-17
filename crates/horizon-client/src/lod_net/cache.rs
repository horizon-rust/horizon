//! The server's far tiles, kept for every player: a tile is the same for
//! everyone in its level, so players looking at the same land cost one
//! sample. A tile is kept as its reply without the request's id
//! ([`super::encode_reply`] writes the id first and nothing after depends on
//! it), so a kept tile answers with the bytes a fresh sample would.
//!
//! Tiles are kept by level, position, stride and detail, under the level's
//! far terrain source: a level opened again (another source: its generator
//! and saves read anew) drops its tiles. The oldest used go first past
//! [`MAX_BYTES`].
//!
//! Tiles fine enough to show saved chunks ([`SAVED_STRIDES`]) read the
//! level's save: the chunks queued to it since ([`TileCache::follow`], the
//! save's `far_changes`) drop the tiles over them, so a build shows in the
//! next tile asked for once its chunk is saved (when it unloads, or about 10
//! seconds after it changed while loaded: vanilla's eager chunk saving).
use super::Request;
use crate::{
    save::{SaveDir, far_changes::Changed},
    streaming::LodSource,
};
use horizon_protocol::buf::Writer;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Weak},
};

/// What the kept tiles may weigh, replies' bytes: about 4,000 tiles of a
/// mean 8 KiB (`docs/hosting.md`).
pub const MAX_BYTES: usize = 32 << 20;
/// The strides whose tiles read saved chunks (`save::far_lod`: up to 16
/// blocks a column): coarser ones are the generator's alone.
pub const SAVED_STRIDES: [i32; 5] = [1, 2, 4, 8, 16];

/// `reply`, the answer to request `from`, as the answer to request `to`.
pub fn with_id(reply: &[u8], from: u32, to: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(from as i32);
    let body = reply.strip_prefix(&w.data[..]).unwrap_or(reply);
    let mut w = Writer::new();
    w.varint(to as i32);
    w.data.extend_from_slice(body);
    w.data
}

/// Whether `save` queued a chunk under the tile `request` asks for since
/// `cursor`, for a tile that reads saved chunks.
pub fn changed_under(save: &SaveDir, cursor: u64, request: &Request) -> bool {
    if !SAVED_STRIDES.contains(&request.stride) {
        return false;
    }
    let size = request.stride * super::SIDE as i32;
    let min = (request.x >> 4, request.z >> 4);
    let max = ((request.x + size - 1) >> 4, (request.z + size - 1) >> 4);
    save.far_changes().changed_in(cursor, min, max)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Key {
    level: u32,
    x: i32,
    z: i32,
    stride: i32,
    approximate: bool,
}

struct Kept {
    body: Arc<[u8]>,
    used: u64,
}

/// A level's tiles: its name and the source they were sampled from. A
/// `Weak` keeps the source's address from being taken by another while
/// tiles of it are kept, without keeping its generator alive.
struct Level {
    name: String,
    source: Weak<dyn LodSource>,
    /// Its save, and how far its tiles follow the save's changes.
    save: Option<(Arc<SaveDir>, u64)>,
}

#[derive(Default)]
pub struct TileCache {
    levels: Vec<Level>,
    tiles: HashMap<Key, Kept>,
    /// Keys by last use, oldest first.
    order: BTreeMap<u64, Key>,
    clock: u64,
    bytes: usize,
    max_bytes: usize,
    pub hits: u64,
    pub misses: u64,
}

impl TileCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            ..Self::default()
        }
    }

    /// The level's index for tiles of `source`; a level seen before under
    /// another source loses its tiles.
    fn level(&mut self, name: &str, source: &Arc<dyn LodSource>) -> u32 {
        let same = |level: &Level| std::ptr::addr_eq(level.source.as_ptr(), Arc::as_ptr(source));
        if let Some(at) = self.levels.iter().position(|l| l.name == name) {
            if same(&self.levels[at]) {
                return at as u32;
            }
            self.forget(at as u32);
            self.levels[at].source = Arc::downgrade(source);
            self.levels[at].save = None;
            return at as u32;
        }
        self.levels.push(Level {
            name: name.to_owned(),
            source: Arc::downgrade(source),
            save: None,
        });
        (self.levels.len() - 1) as u32
    }

    /// Drop the tiles over the chunks `save` had queued since the last
    /// call for `level`: before a tile is looked up, so none sampled before a
    /// change answers after it.
    pub fn follow(&mut self, level: &str, source: &Arc<dyn LodSource>, save: &Arc<SaveDir>) {
        let at = self.level(level, source);
        let entry = &mut self.levels[at as usize];
        let (_, cursor) = entry.save.get_or_insert_with(|| {
            // Its tiles so far were sampled from what it has now.
            let end = save.far_changes().end();
            (save.clone(), end)
        });
        match save.far_changes().since(cursor) {
            Changed::Chunks(chunks) => {
                for chunk in chunks {
                    self.changed(at, chunk);
                }
            }
            Changed::All => self.forget(at),
        }
    }

    /// The tiles of `level` over chunk `(cx, cz)` that read saved chunks.
    fn changed(&mut self, level: u32, (cx, cz): (i32, i32)) {
        for stride in SAVED_STRIDES {
            let size = stride * super::SIDE as i32;
            let (x, z) = (
                (cx * 16).div_euclid(size) * size,
                (cz * 16).div_euclid(size) * size,
            );
            for approximate in [false, true] {
                self.remove(&Key {
                    level,
                    x,
                    z,
                    stride,
                    approximate,
                });
            }
        }
    }

    fn forget(&mut self, level: u32) {
        let gone: Vec<Key> = self
            .tiles
            .keys()
            .filter(|k| k.level == level)
            .copied()
            .collect();
        for key in gone {
            self.remove(&key);
        }
    }

    fn remove(&mut self, key: &Key) {
        if let Some(kept) = self.tiles.remove(key) {
            self.order.remove(&kept.used);
            self.bytes -= kept.body.len();
        }
    }

    fn key(&mut self, level: &str, source: &Arc<dyn LodSource>, r: &Request) -> Key {
        Key {
            level: self.level(level, source),
            x: r.x,
            z: r.z,
            stride: r.stride,
            approximate: r.approximate,
        }
    }

    /// The reply to `request` from a kept tile.
    pub fn get(
        &mut self,
        level: &str,
        source: &Arc<dyn LodSource>,
        request: &Request,
    ) -> Option<Vec<u8>> {
        let key = self.key(level, source, request);
        let Some(kept) = self.tiles.get_mut(&key) else {
            self.misses += 1;
            return None;
        };
        self.hits += 1;
        self.clock += 1;
        self.order.remove(&kept.used);
        kept.used = self.clock;
        self.order.insert(self.clock, key);
        let mut w = Writer::new();
        w.varint(request.id as i32);
        w.data.extend_from_slice(&kept.body);
        Some(w.data)
    }

    /// Keep `reply`, a fresh sample's answer to `request`.
    pub fn put(
        &mut self,
        level: &str,
        source: &Arc<dyn LodSource>,
        request: &Request,
        reply: &[u8],
    ) {
        let mut id = Writer::new();
        id.varint(request.id as i32);
        let Some(body) = reply.strip_prefix(&id.data[..]) else {
            return;
        };
        if body.len() > self.max_bytes / 4 {
            return;
        }
        let key = self.key(level, source, request);
        self.remove(&key);
        self.clock += 1;
        self.bytes += body.len();
        self.tiles.insert(
            key,
            Kept {
                body: body.into(),
                used: self.clock,
            },
        );
        self.order.insert(self.clock, key);
        while self.bytes > self.max_bytes
            && let Some((_, oldest)) = self.order.pop_first()
        {
            if let Some(kept) = self.tiles.remove(&oldest) {
                self.bytes -= kept.body.len();
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.tiles.len()
    }
    #[cfg(test)]
    fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::streaming::LodSampler;

    struct Flat;
    impl LodSampler for Flat {
        fn sample(&mut self, _: i32, _: i32) -> Option<crate::lod::Surface> {
            None
        }
    }
    impl LodSource for Flat {
        fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
            Box::new(Flat)
        }
    }

    fn request(id: u32, x: i32) -> Request {
        Request {
            id,
            x,
            z: 0,
            stride: 1,
            approximate: false,
        }
    }

    #[test]
    fn the_oldest_used_tiles_go_first_and_a_new_source_drops_its_level() {
        let source: Arc<dyn LodSource> = Arc::new(Flat);
        let mut cache = TileCache::new(4 * 100);
        let reply = |id: u32| {
            let mut w = Writer::new();
            w.varint(id as i32);
            w.data.extend_from_slice(&[7; 100]);
            w.data
        };
        for (i, x) in [0, 16, 32, 48].into_iter().enumerate() {
            cache.put("o", &source, &request(i as u32, x), &reply(i as u32));
        }
        assert_eq!(cache.bytes(), 400);
        // Used again: the second oldest goes instead.
        assert_eq!(cache.get("o", &source, &request(300, 0)), Some(reply(300)));
        cache.put("o", &source, &request(9, 64), &reply(9));
        assert_eq!(cache.len(), 4);
        assert!(cache.get("o", &source, &request(1, 16)).is_none());
        assert!(cache.get("o", &source, &request(1, 0)).is_some());
        // Another level keeps its own; the level opened again loses its tiles.
        cache.put("n", &source, &request(1, 0), &reply(1));
        let reopened: Arc<dyn LodSource> = Arc::new(Flat);
        assert!(cache.get("o", &reopened, &request(1, 0)).is_none());
        assert!(cache.get("n", &source, &request(1, 0)).is_some());
    }

    #[test]
    fn a_chunk_queued_to_the_save_drops_the_tiles_that_read_it() {
        let dir = std::env::temp_dir().join(format!("horizon-far-tiles-{}", std::process::id()));
        let save = Arc::new(SaveDir::new(
            dir.clone(),
            crate::chunk::LevelShape::OVERWORLD,
        ));
        let source: Arc<dyn LodSource> = Arc::new(Flat);
        let mut cache = TileCache::new(1 << 20);
        let tile = |x: i32, z: i32, stride: i32| Request {
            id: 1,
            x,
            z,
            stride,
            approximate: false,
        };
        let tiles = [
            tile(-16, -16, 1),
            tile(-256, -256, 16),
            tile(-512, -512, 32),
            tile(0, 0, 1),
        ];
        cache.follow("o", &source, &save);
        for r in &tiles {
            cache.put("o", &source, r, &[1, 0, 0]);
        }
        // Chunk (-1, -1) is saved: it is in the stride 1 and 16 tiles, not
        // in the one east of it, and coarser tiles never read saved chunks.
        save.clear_chunks(&[(-1, -1)]).unwrap();
        cache.follow("o", &source, &save);
        let kept: Vec<bool> = (tiles.iter())
            .map(|r| cache.get("o", &source, r).is_some())
            .collect();
        assert_eq!(kept, [false, false, true, true]);
        drop(save);
        let _ = std::fs::remove_dir_all(dir);
    }
}
