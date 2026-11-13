//! The natural spawn chunk counter (`DistanceManager.naturalSpawnChunkCounter`)
//! and the order it hands its chunks out in.
//!
//! `ChunkMap.collectSpawningChunks` walks `getSpawnCandidateChunks`, the key
//! set of a `Long2ByteOpenHashMap`, and `ServerChunkCache.tickChunks` shuffles
//! the list it builds. The shuffle draws the level's random once per chunk in
//! that order, so which chunk spends which draw — and so where the mobs of a
//! pass land — follows the map's iteration order. Reproducing it means
//! reproducing both the counter that fills the map (`FixedPlayerDistanceChunkTracker`,
//! a `ChunkTracker` over a `DynamicGraphMinFixedPoint`) and the table's own
//! order (`fastutil`'s open addressing), because the counter only ever puts
//! and removes, and the table's slots remember the order it did that in.
use horizon_client::fast_hash::FxHashMap;
use horizon_core::random::{LegacyRandom, RandomSource};

/// `Util.shuffle(chunks, level.getRandom())` in `ServerChunkCache.tickChunks`:
/// one `nextInt` per chunk from the end down, so the order the counter hands
/// the chunks out in decides which chunk spends which draw.
pub(super) fn shuffle_spawning_chunks(chunks: &mut [(i32, i32)], random: &mut LegacyRandom) {
    for i in (1..chunks.len()).rev() {
        let j = random.next_int_bound(i as i32 + 1).unwrap_or(0) as usize;
        chunks.swap(i, j);
    }
}

/// `FixedPlayerDistanceChunkTracker(8)`: chunks up to 8 away from a player
/// chunk are in the map, and `ChunkTracker`'s level count is two more.
const MAX_DISTANCE: i32 = 8;
const LEVEL_COUNT: i32 = MAX_DISTANCE + 2;
/// `computedLevels`' default (`(byte)-1`, read as `& 255`): not queued.
const NO_COMPUTED_LEVEL: i32 = 255;

/// `ChunkPos.pack`.
const fn pack(x: i32, z: i32) -> i64 {
    (x as i64 & 0xFFFF_FFFF) | ((z as i64 & 0xFFFF_FFFF) << 32)
}
/// `ChunkPos.INVALID_CHUNK_POS`: `ChunkTracker.isSource`'s node and the parent
/// `ChunkTracker.update` passes, a chunk position past the world border that no
/// real chunk is. (`DynamicGraphMinFixedPoint.SOURCE` is `Long.MAX_VALUE`, but
/// `ChunkTracker` overrides `isSource` with this one.)
const INVALID_CHUNK_POS: i64 = pack(1875066, 1875066);
fn unpack(pos: i64) -> (i32, i32) {
    (pos as i32, (pos >> 32) as i32)
}

/// `DistanceManager.naturalSpawnChunkCounter`: the chunks within 8 of a
/// player's chunk, in the order its map walks them.
#[derive(Default)]
pub(super) struct SpawnChunkTracker {
    /// `FixedPlayerDistanceChunkTracker.chunks`: the levels of the chunks in
    /// range, and the order `getSpawnCandidateChunks` returns them in.
    chunks: ChunkLevels,
    /// `DynamicGraphMinFixedPoint.computedLevels`: the level each queued
    /// chunk was queued with, `NO_COMPUTED_LEVEL` when it is not queued.
    /// Its own order never matters (vanilla only gets, puts and removes).
    computed: FxHashMap<i64, u8>,
    /// `DynamicGraphMinFixedPoint.priorityQueue`.
    queue: LeveledQueue,
    /// `DistanceManager.playersPerChunk`, by chunk: how many players hold it
    /// (`ChunkTracker.getLevelFromSource` asks whether any does).
    players: FxHashMap<i64, u32>,
}
impl SpawnChunkTracker {
    /// `DistanceManager.addPlayer`: a player holds a level 0 chunk, which
    /// lowers the levels around it (`ChunkTracker.update(pos, 0, true)`).
    pub(super) fn add_player(&mut self, x: i32, z: i32) {
        let pos = pack(x, z);
        *self.players.entry(pos).or_insert(0) += 1;
        self.update(pos, 0, true);
    }
    /// `DistanceManager.removePlayer`: no player holds the chunk any more, so
    /// the levels around it rise again (`update(pos, Integer.MAX_VALUE, false)`).
    pub(super) fn remove_player(&mut self, x: i32, z: i32) {
        let pos = pack(x, z);
        if let Some(held) = self.players.get_mut(&pos) {
            *held -= 1;
            if *held == 0 {
                self.players.remove(&pos);
                self.update(pos, i32::MAX, false);
            }
        }
    }
    /// `FixedPlayerDistanceChunkTracker.runAllUpdates`: work off the queue.
    pub(super) fn run_updates(&mut self) {
        self.run_some_updates(i32::MAX);
    }
    /// The map's key set (`getSpawnCandidateChunks`): chunk coordinates in the
    /// order `Long2ByteOpenHashMap.keySet().iterator()` visits them.
    pub(super) fn chunks(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.chunks.keys().map(unpack)
    }
    /// `getNaturalSpawnChunkCount`.
    pub(super) fn len(&self) -> usize {
        self.chunks.size
    }

    /// `ChunkTracker.update`.
    fn update(&mut self, node: i64, new_level_from: i32, only_decreased: bool) {
        self.check_edge(INVALID_CHUNK_POS, node, new_level_from, only_decreased);
    }
    /// `FixedPlayerDistanceChunkTracker.getLevel`: the map's level, or
    /// `defaultReturnValue` (`maxDistance + 2`) for a chunk that is not in it.
    fn level(&self, node: i64) -> i32 {
        self.chunks.get(node).map_or(MAX_DISTANCE + 2, i32::from)
    }
    /// `FixedPlayerDistanceChunkTracker.setLevel`: a chunk further than
    /// `maxDistance` leaves the map, any other is put with its level.
    fn set_level(&mut self, node: i64, level: i32) {
        if level > MAX_DISTANCE {
            self.chunks.remove(node);
        } else {
            self.chunks.put(node, level as u8);
        }
    }
    /// `ChunkTracker.getLevelFromSource`: a player's chunk is level 0.
    fn level_from_source(&self, node: i64) -> i32 {
        if self.players.contains_key(&node) {
            0
        } else {
            i32::MAX
        }
    }
    /// `ChunkTracker.computeLevelFromNeighbor`: one more than the neighbour,
    /// or the source level when the neighbour is `INVALID_CHUNK_POS`.
    fn level_from_neighbor(&self, from: i64, to: i64, from_level: i32) -> i32 {
        if from == INVALID_CHUNK_POS {
            self.level_from_source(to)
        } else {
            from_level + 1
        }
    }
    /// `ChunkTracker.getComputedLevel`: the lowest level the neighbours allow.
    fn computed_level(&self, node: i64, known_parent: i64, known_level_from_parent: i32) -> i32 {
        let (x, z) = unpack(node);
        let mut computed = known_level_from_parent;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let mut neighbor = pack(x + dx, z + dz);
                if neighbor == node {
                    neighbor = INVALID_CHUNK_POS;
                }
                if neighbor != known_parent {
                    let cost = self.level_from_neighbor(neighbor, node, self.level(neighbor));
                    if computed > cost {
                        computed = cost;
                    }
                    if computed == 0 {
                        return computed;
                    }
                }
            }
        }
        computed
    }
    /// `DynamicGraphMinFixedPoint.checkEdge(from, to, newLevelFrom, onlyDecreased)`.
    fn check_edge(&mut self, from: i64, to: i64, new_level_from: i32, only_decreased: bool) {
        let level_to = self.level(to);
        let old_computed = self
            .computed
            .get(&to)
            .map_or(NO_COMPUTED_LEVEL, |&v| i32::from(v));
        self.check_edge_with(
            from,
            to,
            new_level_from,
            level_to,
            old_computed,
            only_decreased,
        );
    }
    /// `DynamicGraphMinFixedPoint.checkNeighbor`.
    fn check_neighbor(&mut self, from: i64, to: i64, level: i32, only_decreased: bool) {
        let stored_old = self
            .computed
            .get(&to)
            .map_or(NO_COMPUTED_LEVEL, |&v| i32::from(v));
        let level_from = self
            .level_from_neighbor(from, to, level)
            .clamp(0, LEVEL_COUNT - 1);
        if only_decreased {
            self.check_edge_with(from, to, level_from, self.level(to), stored_old, true);
            return;
        }
        let was_consistent = stored_old == NO_COMPUTED_LEVEL;
        let old_computed = if was_consistent {
            self.level(to).clamp(0, LEVEL_COUNT - 1)
        } else {
            stored_old
        };
        if level_from == old_computed {
            let level_to = if was_consistent {
                old_computed
            } else {
                self.level(to)
            };
            self.check_edge_with(from, to, LEVEL_COUNT - 1, level_to, stored_old, false);
        }
    }
    /// `DynamicGraphMinFixedPoint.checkEdge`'s six-argument body.
    fn check_edge_with(
        &mut self,
        from: i64,
        to: i64,
        new_level_from: i32,
        level_to: i32,
        old_computed: i32,
        only_decreased: bool,
    ) {
        if to == INVALID_CHUNK_POS {
            return;
        }
        let level_to = level_to.clamp(0, LEVEL_COUNT - 1);
        let new_level_from = new_level_from.clamp(0, LEVEL_COUNT - 1);
        let was_consistent = old_computed == NO_COMPUTED_LEVEL;
        let old_computed = if was_consistent {
            level_to
        } else {
            old_computed
        };
        let new_computed = if only_decreased {
            old_computed.min(new_level_from)
        } else {
            self.computed_level(to, from, new_level_from)
                .clamp(0, LEVEL_COUNT - 1)
        };
        let old_priority = priority(level_to, old_computed);
        if level_to != new_computed {
            let new_priority = priority(level_to, new_computed);
            if old_priority != new_priority && !was_consistent {
                self.queue.dequeue(to, old_priority, new_priority);
            }
            self.queue.enqueue(to, new_priority);
            self.computed.insert(to, new_computed as u8);
        } else if !was_consistent {
            self.queue.dequeue(to, old_priority, LEVEL_COUNT as usize);
            self.computed.remove(&to);
        }
    }
    /// `ChunkTracker.checkNeighborsAfterUpdate`.
    fn check_neighbors_after_update(&mut self, node: i64, level: i32, only_decrease: bool) {
        if only_decrease && level >= LEVEL_COUNT - 2 {
            return;
        }
        let (x, z) = unpack(node);
        for dx in -1..=1 {
            for dz in -1..=1 {
                let neighbor = pack(x + dx, z + dz);
                if neighbor != node {
                    self.check_neighbor(node, neighbor, level, only_decrease);
                }
            }
        }
    }
    /// `DynamicGraphMinFixedPoint.runUpdates`.
    fn run_some_updates(&mut self, mut count: i32) {
        while !self.queue.is_empty() && count > 0 {
            count -= 1;
            let node = self.queue.remove_first();
            let level = self.level(node).clamp(0, LEVEL_COUNT - 1);
            let computed = self
                .computed
                .remove(&node)
                .map_or(NO_COMPUTED_LEVEL, i32::from);
            if computed < level {
                self.set_level(node, computed);
                self.check_neighbors_after_update(node, computed, true);
            } else if computed > level {
                self.set_level(node, LEVEL_COUNT - 1);
                if computed != LEVEL_COUNT - 1 {
                    self.queue
                        .enqueue(node, priority(LEVEL_COUNT - 1, computed));
                    self.computed.insert(node, computed as u8);
                }
                self.check_neighbors_after_update(node, level, false);
            }
        }
    }
}

/// `DynamicGraphMinFixedPoint.calculatePriority`.
fn priority(level: i32, computed_level: i32) -> usize {
    level.min(computed_level).clamp(0, LEVEL_COUNT - 1) as usize
}

/// `LeveledPriorityQueue`: one `LongLinkedOpenHashSet` per priority, taken
/// from the lowest one; a node keeps its place in its bucket, and moving it
/// to another priority puts it at the end of that bucket.
struct LeveledQueue {
    buckets: Vec<Vec<i64>>,
    first_queued: usize,
}
impl Default for LeveledQueue {
    fn default() -> Self {
        Self {
            buckets: vec![Vec::new(); LEVEL_COUNT as usize],
            first_queued: LEVEL_COUNT as usize,
        }
    }
}
impl LeveledQueue {
    fn is_empty(&self) -> bool {
        self.first_queued >= LEVEL_COUNT as usize
    }
    fn enqueue(&mut self, node: i64, key: usize) {
        if !self.buckets[key].contains(&node) {
            self.buckets[key].push(node);
        }
        if self.first_queued > key {
            self.first_queued = key;
        }
    }
    fn dequeue(&mut self, node: i64, key: usize, upper_bound: usize) {
        let bucket = &mut self.buckets[key];
        if let Some(pos) = bucket.iter().position(|&n| n == node) {
            bucket.remove(pos);
        }
        if bucket.is_empty() && self.first_queued == key {
            self.check_first_queued_level(upper_bound);
        }
    }
    fn remove_first(&mut self) -> i64 {
        let node = self.buckets[self.first_queued].remove(0);
        if self.buckets[self.first_queued].is_empty() {
            self.check_first_queued_level(LEVEL_COUNT as usize);
        }
        node
    }
    fn check_first_queued_level(&mut self, upper_bound: usize) {
        let old = self.first_queued;
        self.first_queued = upper_bound;
        for i in old + 1..upper_bound {
            if !self.buckets[i].is_empty() {
                self.first_queued = i;
                break;
            }
        }
    }
}

/// fastutil's `Long2ByteOpenHashMap` (`new Long2ByteOpenHashMap()`: 16 expected
/// entries, load factor 0.75, no shrinking), for the order its key iterator
/// visits the keys in: the key 0 first, then the table from its end down.
struct ChunkLevels {
    key: Vec<i64>,
    value: Vec<u8>,
    /// The value of the key 0, which the table keeps apart (at index `n`).
    null_value: Option<u8>,
    size: usize,
    max_fill: usize,
    /// `minN`: the capacity the table was built with. It shrinks back no
    /// further than this, and only once it holds a quarter of a full table.
    min_n: usize,
}
impl Default for ChunkLevels {
    fn default() -> Self {
        let n = array_size(16);
        Self {
            key: vec![0; n],
            value: vec![0; n],
            null_value: None,
            size: 0,
            max_fill: max_fill(n),
            min_n: n,
        }
    }
}
impl ChunkLevels {
    fn n(&self) -> usize {
        self.key.len()
    }
    fn mask(&self) -> usize {
        self.n() - 1
    }
    /// `HashCommon.mix(long)`.
    fn mix(key: i64) -> usize {
        let mut h = (key as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 32;
        (h ^ (h >> 16)) as u32 as usize
    }
    fn slot(&self, key: i64) -> usize {
        Self::mix(key) & self.mask()
    }
    fn get(&self, key: i64) -> Option<u8> {
        if key == 0 {
            return self.null_value;
        }
        let mut pos = self.slot(key);
        while self.key[pos] != 0 {
            if self.key[pos] == key {
                return Some(self.value[pos]);
            }
            pos = (pos + 1) & self.mask();
        }
        None
    }
    /// `put(k, v)`: a key already in the table keeps its slot.
    fn put(&mut self, key: i64, value: u8) {
        if key == 0 {
            if self.null_value.is_some() {
                self.null_value = Some(value);
                return;
            }
            self.null_value = Some(value);
        } else {
            let mut pos = self.slot(key);
            while self.key[pos] != 0 {
                if self.key[pos] == key {
                    self.value[pos] = value;
                    return;
                }
                pos = (pos + 1) & self.mask();
            }
            self.key[pos] = key;
            self.value[pos] = value;
        }
        let grow = self.size >= self.max_fill;
        self.size += 1;
        if grow {
            self.rehash(array_size(self.size + 1));
        }
    }
    fn remove(&mut self, key: i64) {
        if key == 0 {
            if self.null_value.take().is_some() {
                self.size -= 1;
                self.shrink();
            }
            return;
        }
        let mut pos = self.slot(key);
        while self.key[pos] != 0 {
            if self.key[pos] == key {
                self.size -= 1;
                self.shift_keys(pos);
                self.shrink();
                return;
            }
            pos = (pos + 1) & self.mask();
        }
    }
    /// `remove`'s `if (n > minN && size < maxFill / 4 && n > 16) rehash(n / 2)`.
    fn shrink(&mut self) {
        if self.n() > self.min_n && self.size < self.max_fill / 4 && self.n() > 16 {
            self.rehash(self.n() / 2);
        }
    }
    /// `HashCommon.rehash`: the entries back into a larger table, in the order
    /// the old one holds them from its end down.
    fn rehash(&mut self, new_n: usize) {
        let mask = new_n - 1;
        let mut keys = vec![0i64; new_n];
        let mut values = vec![0u8; new_n];
        let real = self.size - usize::from(self.null_value.is_some());
        let mut i = self.n();
        for _ in 0..real {
            loop {
                i -= 1;
                if self.key[i] != 0 {
                    break;
                }
            }
            let key = self.key[i];
            let mut pos = Self::mix(key) & mask;
            while keys[pos] != 0 {
                pos = (pos + 1) & mask;
            }
            keys[pos] = key;
            values[pos] = self.value[i];
        }
        self.key = keys;
        self.value = values;
        self.max_fill = max_fill(new_n);
    }
    /// `shiftKeys`: pull the entries that probed past `pos` back into the gap.
    fn shift_keys(&mut self, mut pos: usize) {
        let mask = self.mask();
        loop {
            let last = pos;
            pos = (last + 1) & mask;
            let curr = loop {
                let curr = self.key[pos];
                if curr == 0 {
                    self.key[last] = 0;
                    return;
                }
                let slot = Self::mix(curr) & mask;
                let moves = if last <= pos {
                    last >= slot || slot > pos
                } else {
                    last >= slot && slot > pos
                };
                if moves {
                    break curr;
                }
                pos = (pos + 1) & mask;
            };
            self.key[last] = curr;
            self.value[last] = self.value[pos];
        }
    }
    /// `keySet().iterator()`: the key 0, then the table from its end down.
    fn keys(&self) -> impl Iterator<Item = i64> + '_ {
        self.null_value
            .iter()
            .map(|_| 0)
            .chain((0..self.n()).rev().filter_map(|pos| {
                let key = self.key[pos];
                (key != 0).then_some(key)
            }))
    }
}

/// `HashCommon.arraySize(expected, 0.75F)`.
fn array_size(expected: usize) -> usize {
    ((expected as f64 / 0.75).ceil() as usize)
        .next_power_of_two()
        .max(2)
}
/// `HashCommon.maxFill(n, 0.75F)`.
fn max_fill(n: usize) -> usize {
    ((n as f64 * 0.75).ceil() as usize).min(n - 1)
}
