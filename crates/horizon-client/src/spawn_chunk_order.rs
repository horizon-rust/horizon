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
//! order (`fastutil_order::OpenOrder`), because the counter only ever puts
//! and removes, and the table's slots remember the order it did that in.
//! tools/test_spawn_chunk_order_parity.py holds both to the real classes.
use crate::fast_hash::FxHashMap;
use crate::fastutil_order::OpenOrder;
use horizon_core::random::{LegacyRandom, RandomSource};

/// `Util.shuffle(chunks, level.getRandom())` in `ServerChunkCache.tickChunks`:
/// one `nextInt` per chunk from the end down, so the order the counter hands
/// the chunks out in decides which chunk spends which draw.
pub fn shuffle_spawning_chunks(chunks: &mut [(i32, i32)], random: &mut LegacyRandom) {
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
pub struct SpawnChunkTracker {
    /// `FixedPlayerDistanceChunkTracker.chunks` (`new Long2ByteOpenHashMap()`):
    /// the order `getSpawnCandidateChunks` returns the chunks in range in,
    /// and their levels.
    order: OpenOrder<i64>,
    levels: FxHashMap<i64, u8>,
    /// `DynamicGraphMinFixedPoint.computedLevels`: the level each queued
    /// chunk was queued with, `NO_COMPUTED_LEVEL` when it is not queued.
    /// Its own order never matters (vanilla only gets, puts and removes).
    computed: FxHashMap<i64, u8>,
    /// `DynamicGraphMinFixedPoint.priorityQueue`.
    queue: LeveledQueue,
    /// `DistanceManager.playersPerChunk`, by chunk: how many players hold it
    /// (`ChunkTracker.getLevelFromSource` asks whether any does).
    players: FxHashMap<i64, u32>,
    /// Each player in the counter and its section (`ChunkMap`'s last
    /// `SectionPos`), in joining order.
    sources: Vec<(u128, [i32; 3])>,
}
impl SpawnChunkTracker {
    /// A player is in section `[x, y, z]`: one that just joined is added
    /// (`ChunkMap.updatePlayerStatus`), one that changed section leaves its
    /// old one and is added to the new one (`ChunkMap.move`). A move between
    /// the sections of one chunk does that too, which sends a chunk still
    /// queued to the back of its queue.
    pub fn place(&mut self, id: u128, section: [i32; 3]) {
        match self.sources.iter_mut().find(|(i, _)| *i == id) {
            Some((_, old)) if *old == section => return,
            Some((_, old)) => {
                let left = std::mem::replace(old, section);
                self.remove_player(left);
            }
            None => self.sources.push((id, section)),
        }
        self.add_player(section);
    }
    /// The players `keep` turns down leave (`ChunkMap.updatePlayerStatus`'s
    /// `removePlayer`), in joining order.
    pub fn retain(&mut self, mut keep: impl FnMut(u128) -> bool) {
        let mut i = 0;
        while i < self.sources.len() {
            let (id, section) = self.sources[i];
            if keep(id) {
                i += 1;
            } else {
                self.sources.remove(i);
                self.remove_player(section);
            }
        }
    }
    /// `DistanceManager.addPlayer`: a player holds a level 0 chunk, which
    /// lowers the levels around it (`ChunkTracker.update(pos, 0, true)`).
    fn add_player(&mut self, [x, _, z]: [i32; 3]) {
        let pos = pack(x, z);
        *self.players.entry(pos).or_insert(0) += 1;
        self.update(pos, 0, true);
    }
    /// `DistanceManager.removePlayer`: once no player holds the chunk, the
    /// levels around it rise again (`update(pos, Integer.MAX_VALUE, false)`).
    fn remove_player(&mut self, [x, _, z]: [i32; 3]) {
        let pos = pack(x, z);
        if let Some(held) = self.players.get_mut(&pos) {
            *held -= 1;
            if *held == 0 {
                self.players.remove(&pos);
                self.update(pos, i32::MAX, false);
            }
        }
    }
    /// The map's key set (`getSpawnCandidateChunks`): chunk coordinates in the
    /// order `Long2ByteOpenHashMap.keySet().iterator()` visits them.
    pub fn chunks(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.order.iter().map(|&pos| unpack(pos))
    }
    /// `getNaturalSpawnChunkCount`.
    pub fn len(&self) -> usize {
        self.order.len()
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// `ChunkTracker.update`: `DynamicGraphMinFixedPoint.checkEdge` from the
    /// source.
    fn update(&mut self, node: i64, new_level_from: i32, only_decreased: bool) {
        let old_computed = self.stored_computed(node);
        let level_to = self.level(node);
        let from = INVALID_CHUNK_POS;
        self.check_edge(
            from,
            node,
            new_level_from,
            level_to,
            old_computed,
            only_decreased,
        );
    }
    /// `computedLevels.get(node) & 255`.
    fn stored_computed(&self, node: i64) -> i32 {
        self.computed
            .get(&node)
            .map_or(NO_COMPUTED_LEVEL, |&v| i32::from(v))
    }
    /// `FixedPlayerDistanceChunkTracker.getLevel`: the map's level, or
    /// `defaultReturnValue` (`maxDistance + 2`) for a chunk that is not in it.
    fn level(&self, node: i64) -> i32 {
        self.levels
            .get(&node)
            .map_or(MAX_DISTANCE + 2, |&l| i32::from(l))
    }
    /// `FixedPlayerDistanceChunkTracker.setLevel`: a chunk further than
    /// `maxDistance` leaves the map, any other is put with its level.
    fn set_level(&mut self, node: i64, level: i32) {
        if level > MAX_DISTANCE {
            self.order.remove(&node);
            self.levels.remove(&node);
        } else {
            self.order.insert(&node);
            self.levels.insert(node, level as u8);
        }
    }
    /// `ChunkTracker.computeLevelFromNeighbor`: one more than the neighbour,
    /// or from `INVALID_CHUNK_POS`, `getLevelFromSource` (0 on a player's chunk).
    fn level_from_neighbor(&self, from: i64, to: i64, from_level: i32) -> i32 {
        match from {
            INVALID_CHUNK_POS if self.players.contains_key(&to) => 0,
            INVALID_CHUNK_POS => i32::MAX,
            _ => from_level + 1,
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
    /// `DynamicGraphMinFixedPoint.checkNeighbor`.
    fn check_neighbor(&mut self, from: i64, to: i64, level: i32, only_decreased: bool) {
        let stored_old = self.stored_computed(to);
        let level_from = self
            .level_from_neighbor(from, to, level)
            .clamp(0, LEVEL_COUNT - 1);
        if only_decreased {
            self.check_edge(from, to, level_from, self.level(to), stored_old, true);
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
            self.check_edge(from, to, LEVEL_COUNT - 1, level_to, stored_old, false);
        }
    }
    /// `DynamicGraphMinFixedPoint.checkEdge`'s six-argument form.
    fn check_edge(
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
    /// `FixedPlayerDistanceChunkTracker.runAllUpdates`: work off the queue
    /// (`DynamicGraphMinFixedPoint.runUpdates`).
    pub fn run_updates(&mut self) {
        while !self.queue.is_empty() {
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
