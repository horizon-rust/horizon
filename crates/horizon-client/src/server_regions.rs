//! Server region ownership at a tick barrier. Spatial connectivity is an
//! ownership rule, never a proof that gameplay in two regions commutes.
//!
//! No world data is copied here. Slots refer to the level's ordered entity
//! list, and chunk membership is frozen until all work at the barrier joins.
use crate::chunk_map::ChunkMembershipDelta;
use crate::fast_hash::{FxHashMap, FxHashSet};

pub type Chunk = (i32, i32);

/// A region's canonical chunk. Handles are valid only until the next rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionId(pub Chunk);

#[derive(Debug)]
pub struct Region {
    pub id: RegionId,
    chunks: FxHashSet<Chunk>,
    /// Slots retain the input entity order, including ties.
    pub entities: Vec<usize>,
}

/// A level's ownership directory. Never shared mutably with a worker.
#[derive(Default)]
pub struct Regions {
    revision: Option<u64>,
    regions: Vec<Region>,
    owners: FxHashMap<Chunk, usize>,
}

/// The eight chunks around one, in no particular order.
fn around((x, z): Chunk) -> impl Iterator<Item = Chunk> {
    (-1..=1)
        .flat_map(move |dx| (-1..=1).map(move |dz| (dx, dz)))
        .filter(|&offset| offset != (0, 0))
        .filter_map(move |(dx, dz)| Some((x.checked_add(dx)?, z.checked_add(dz)?)))
}

impl Regions {
    /// Follows the chunk map, a chunk at a time when `deltas` (the changes since
    /// the revision this last saw, from `ChunkMap::deltas_since`) allow it: a
    /// moving player changes dozens of chunks of a map that can hold tens of
    /// thousands. `chunks` is the whole membership, for when they don't.
    pub fn update(
        &mut self,
        revision: u64,
        deltas: impl FnOnce(u64) -> Option<Vec<ChunkMembershipDelta>>,
        chunks: impl Iterator<Item = Chunk>,
    ) {
        if self.revision == Some(revision) {
            return;
        }
        let applied = self
            .revision
            .and_then(deltas)
            .is_some_and(|deltas| deltas.into_iter().all(|delta| self.apply(delta)));
        if applied {
            self.revision = Some(revision);
        } else {
            self.rebuild(revision, chunks);
        }
    }

    /// One membership change. False when only a whole rebuild can answer it,
    /// which leaves the regions to that rebuild.
    fn apply(&mut self, delta: ChunkMembershipDelta) -> bool {
        match delta {
            ChunkMembershipDelta::Added(chunk) => {
                self.add(chunk);
                true
            }
            ChunkMembershipDelta::Removed(chunk) => self.remove(chunk),
            ChunkMembershipDelta::Reset => false,
        }
    }

    /// A new chunk joins the regions around it, and merges them.
    fn add(&mut self, chunk: Chunk) {
        if self.owners.contains_key(&chunk) {
            return;
        }
        let mut neighbours: Vec<usize> = around(chunk)
            .filter_map(|next| self.owners.get(&next).copied())
            .collect();
        neighbours.sort_unstable();
        neighbours.dedup();
        let Some(&largest) = neighbours
            .iter()
            .max_by_key(|&&slot| self.regions[slot].chunks.len())
        else {
            self.owners.insert(chunk, self.regions.len());
            self.regions.push(Region {
                id: RegionId(chunk),
                chunks: FxHashSet::from_iter([chunk]),
                entities: Vec::new(),
            });
            return;
        };
        let mut into = largest;
        // Highest slot first: taking one moves only the last region.
        for slot in neighbours.into_iter().rev() {
            if slot == largest {
                continue;
            }
            let last = self.regions.len() - 1;
            let merged = self.take_region(slot);
            if into == last {
                into = slot;
            }
            for &moved in &merged.chunks {
                self.owners.insert(moved, into);
            }
            let region = &mut self.regions[into];
            region.chunks.extend(merged.chunks);
            region.id = region.id.min(merged.id);
        }
        self.owners.insert(chunk, into);
        let region = &mut self.regions[into];
        region.chunks.insert(chunk);
        region.id = region.id.min(RegionId(chunk));
    }

    /// A chunk leaves. Any path through it entered and left by the chunks around
    /// it, so while those still connect among themselves its region stays whole.
    /// False when they don't: the region may have split.
    fn remove(&mut self, chunk: Chunk) -> bool {
        let Some(&slot) = self.owners.get(&chunk) else {
            return true;
        };
        let ring: Vec<Chunk> = around(chunk)
            .filter(|next| self.owners.contains_key(next))
            .collect();
        let mut reached = vec![false; ring.len()];
        let mut stack = Vec::new();
        if !ring.is_empty() {
            reached[0] = true;
            stack.push(0);
        }
        while let Some(at) = stack.pop() {
            for (next, seen) in reached.iter_mut().enumerate() {
                let (a, b): (Chunk, Chunk) = (ring[at], ring[next]);
                let near = a.0.abs_diff(b.0) <= 1 && a.1.abs_diff(b.1) <= 1;
                if near && !*seen {
                    *seen = true;
                    stack.push(next);
                }
            }
        }
        if reached.contains(&false) {
            return false;
        }
        self.owners.remove(&chunk);
        let region = &mut self.regions[slot];
        region.chunks.remove(&chunk);
        if region.chunks.is_empty() {
            self.take_region(slot);
        } else if region.id == RegionId(chunk) {
            region.id = RegionId(*region.chunks.iter().min().expect("not empty"));
        }
        true
    }

    /// Removes a region; the last one takes its slot.
    fn take_region(&mut self, slot: usize) -> Region {
        let region = self.regions.swap_remove(slot);
        if let Some(moved) = self.regions.get(slot) {
            for &chunk in &moved.chunks {
                self.owners.insert(chunk, slot);
            }
        }
        region
    }

    /// Rebuild only when chunk membership changes. The caller's revision must
    /// change on load, unload, reset, or replacement of the whole chunk map.
    /// Eight-connected resident chunks share an owner (including diagonals).
    /// A new bridge merges components; unloading a bridge splits them.
    pub fn rebuild(&mut self, revision: u64, chunks: impl Iterator<Item = Chunk>) {
        if self.revision == Some(revision) {
            return;
        }
        let mut remaining: FxHashSet<Chunk> = chunks.collect();
        let mut ordered: Vec<_> = remaining.iter().copied().collect();
        ordered.sort_unstable();
        self.owners.clear();
        self.regions.clear();
        for start in ordered {
            if !remaining.remove(&start) {
                continue;
            }
            let mut chunks = vec![start];
            let mut head = 0;
            while head < chunks.len() {
                let at = chunks[head];
                head += 1;
                for next in around(at) {
                    if remaining.remove(&next) {
                        chunks.push(next);
                    }
                }
            }
            let slot = self.regions.len();
            self.owners.extend(chunks.iter().map(|&c| (c, slot)));
            self.regions.push(Region {
                id: RegionId(start),
                chunks: chunks.into_iter().collect(),
                entities: Vec::new(),
            });
        }
        self.revision = Some(revision);
    }

    /// Publish migrations together at a barrier. Input slots are unchanged:
    /// grouping by region must never reorder the serial entity tick walk.
    /// Removed entities disappear; new entities enter once, at their new slot.
    /// An entity in no resident chunk belongs to no region; the coordinator's
    /// walk still reaches it. Chunks are not loaded or retained by a migration.
    pub fn index(&mut self, entities: impl Iterator<Item = Chunk>) {
        for region in &mut self.regions {
            region.entities.clear();
        }
        for (i, chunk) in entities.enumerate() {
            if let Some(&owner) = self.owners.get(&chunk) {
                self.regions[owner].entities.push(i);
            }
        }
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions
    }
    pub fn owner(&self, chunk: Chunk) -> Option<RegionId> {
        self.owners.get(&chunk).map(|&i| self.regions[i].id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rebuild(r: &mut Regions, rev: u64, chunks: &[Chunk]) {
        r.rebuild(rev, chunks.iter().copied());
    }

    /// The regions as sets of chunks, in an order of their own.
    fn partition(r: &Regions) -> Vec<(RegionId, Vec<Chunk>)> {
        let mut out: Vec<_> = r
            .regions()
            .iter()
            .map(|region| {
                let mut chunks: Vec<_> = region.chunks.iter().copied().collect();
                chunks.sort_unstable();
                (region.id, chunks)
            })
            .collect();
        out.sort_unstable();
        out
    }

    #[test]
    fn loading_and_unloading_a_bridge_merges_and_splits() {
        let mut r = Regions::default();
        rebuild(&mut r, 1, &[(0, 0), (2, 0), (100, 0)]);
        assert_eq!(r.regions().len(), 3);
        rebuild(&mut r, 2, &[(100, 0), (2, 0), (1, 0), (0, 0)]);
        assert_eq!(r.owner((0, 0)), r.owner((2, 0)));
        assert_ne!(r.owner((0, 0)), r.owner((100, 0)));
        rebuild(&mut r, 3, &[(0, 0), (2, 0)]);
        assert_ne!(r.owner((0, 0)), r.owner((2, 0)));
        assert_eq!(r.owner((100, 0)), None);
    }

    #[test]
    fn diagonal_and_negative_chunks_connect_without_integer_wraparound() {
        let mut r = Regions::default();
        rebuild(
            &mut r,
            1,
            &[(-2, -2), (-1, -1), (i32::MIN, 0), (i32::MAX, 0)],
        );
        assert_eq!(r.owner((-2, -2)), r.owner((-1, -1)));
        assert_ne!(r.owner((i32::MIN, 0)), r.owner((i32::MAX, 0)));
        assert_eq!(r.regions().len(), 3);
    }

    #[test]
    fn migration_removal_and_unload_keep_local_and_global_order() {
        let mut r = Regions::default();
        rebuild(&mut r, 1, &[(0, 0), (10, 0)]);
        r.index([(10, 0), (0, 0), (10, 0), (99, 0)].into_iter());
        assert_eq!(r.regions()[0].entities, [1]);
        assert_eq!(r.regions()[1].entities, [0, 2]);
        r.index([(0, 0), (0, 0), (10, 0)].into_iter());
        assert_eq!(r.regions()[0].entities, [0, 1]);
        assert_eq!(r.regions()[1].entities, [2]);
        rebuild(&mut r, 2, &[]);
        r.index([(0, 0)].into_iter());
        assert!(r.regions().is_empty());
    }

    /// Chunk by chunk must land on what a whole rebuild finds, whatever the
    /// order of the changes: regions, their ids and who owns each chunk.
    #[test]
    fn chunk_by_chunk_updates_match_a_whole_rebuild() {
        // A fixed small generator: enough to bridge, split and empty regions in
        // a small area, the same on every run.
        let mut state = 0x2545_f491_u32;
        let mut next = move |bound: u32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state % bound
        };
        let mut chunks: FxHashSet<Chunk> = FxHashSet::default();
        let mut live = Regions::default();
        live.update(1, |_| None, std::iter::empty());
        let mut incremental = 0;
        for step in 0..4000 {
            let chunk = (next(12) as i32 - 6, next(12) as i32 - 6);
            let delta = if chunks.remove(&chunk) {
                ChunkMembershipDelta::Removed(chunk)
            } else {
                chunks.insert(chunk);
                ChunkMembershipDelta::Added(chunk)
            };
            let revision = step + 2;
            let mut whole = false;
            live.update(
                revision,
                |_| Some(vec![delta]),
                chunks.iter().copied().inspect(|_| whole = true),
            );
            incremental += usize::from(!whole);
            let mut expected = Regions::default();
            expected.rebuild(revision, chunks.iter().copied());
            assert_eq!(partition(&live), partition(&expected), "step {step}");
            for &chunk in &chunks {
                assert_eq!(live.owner(chunk), expected.owner(chunk), "step {step}");
            }
        }
        assert!(
            incremental > 2000,
            "most changes should not rebuild: {incremental}"
        );
    }

    #[test]
    fn removing_a_chunk_splits_or_keeps_its_region_by_what_connects_around_it() {
        let mut r = Regions::default();
        // A straight bridge: its middle joins two ends that don't touch.
        rebuild(&mut r, 1, &[(0, 0), (1, 0), (2, 0)]);
        assert!(!r.remove((1, 0)));
        // A block: anything around the middle still connects.
        let mut r = Regions::default();
        let block: Vec<_> = (0..3).flat_map(|x| (0..3).map(move |z| (x, z))).collect();
        rebuild(&mut r, 1, &block);
        assert!(r.remove((1, 1)));
        assert_eq!(r.regions().len(), 1);
        // The region's own id leaves: the next smallest chunk takes over.
        assert!(r.remove((0, 0)));
        assert_eq!(r.owner((2, 2)), Some(RegionId((0, 1))));
    }

    #[test]
    fn a_reset_or_an_unknown_past_rebuilds_the_whole_map() {
        let mut r = Regions::default();
        rebuild(&mut r, 1, &[(0, 0)]);
        r.update(
            2,
            |_| Some(vec![ChunkMembershipDelta::Reset]),
            [(5, 5)].into_iter(),
        );
        assert_eq!(r.owner((0, 0)), None);
        assert_eq!(r.owner((5, 5)), Some(RegionId((5, 5))));
        r.update(3, |_| None, [(7, 7)].into_iter());
        assert_eq!(r.owner((5, 5)), None);
        assert_eq!(r.owner((7, 7)), Some(RegionId((7, 7))));
    }
}
