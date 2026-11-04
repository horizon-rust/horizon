//! The meshed sections waiting for their upload to the GPU, nearest to the
//! camera first, and how many a frame takes.
//!
//! `Streaming::update` hands over every mesh that finished since the last
//! frame. Flying at 100 blocks/s at render distance 32 that is about 230
//! section meshes in a burst, and each upload (an arena allocation, the
//! vertex copy into the staging belt, the occlusion set and the RT grid's
//! voxels, lights and albedo) cost the frame 1 to 4 ms on an RTX 4090
//! machine: the CPU frame's 99th percentile went from 14 ms standing still to
//! 38 ms. The queue keeps what a frame leaves for later in a heap keyed by
//! the squared distance to the camera's section, so a frame takes the
//! nearest ones without sorting all of them; the heap is rebuilt (O(n), no
//! sort) only when the camera enters another section.
use crate::mesher::{SectionMesh, TerrainVertex};
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    time::{Duration, Instant},
};

use crate::fast_hash::FxHashMap;

/// A section: its column's x, its index in the column (0 is y -64), its
/// column's z.
pub type SectionKey = (i32, i32, i32);

/// The main thread's time for section uploads in a frame, whatever the
/// player's own limits, when the frame before did not wait on the GPU
/// (`time_budget`): about 120 to 400 sections at the 5 to 17 µs one costs. Past it, the frame stops after its current upload; sections within
/// `NEAR` of the camera still go.
pub const TIME_PER_FRAME: Duration = Duration::from_millis(2);
/// The most a frame spends on uploads when the frame before waited on the
/// GPU or the display (`time_budget`): half of a 60 Hz frame at most.
pub const MOST_PER_FRAME: Duration = Duration::from_millis(8);
/// What `time_budget` leaves of the last frame's wait, against the wait
/// changing from a frame to the next.
pub const WAIT_MARGIN: Duration = Duration::from_millis(1);

/// A frame's time for uploads, from how long the last frame's render waited
/// on the GPU or the display (`Renderer::last_wait`: acquiring, presenting,
/// offscreen pacing). That wait is time the main thread had nothing to do:
/// uploads may take it, less `WAIT_MARGIN`, between `TIME_PER_FRAME` and
/// `MOST_PER_FRAME`. A GPU-bound frame then streams faster at no cost, and
/// a CPU-bound one (no wait) keeps the 2 ms ceiling.
pub fn time_budget(gpu_wait: Duration) -> Duration {
    gpu_wait
        .saturating_sub(WAIT_MARGIN)
        .clamp(TIME_PER_FRAME, MOST_PER_FRAME)
}
/// The squared distance in sections, from the camera's, under which the
/// time ceiling never holds a section back: the 3x3x3 sections around the
/// camera, the ground underfoot and the walls beside it.
pub const NEAR: i64 = 3;

/// The squared distance in sections between `key` and `centre`.
pub fn distance(centre: SectionKey, key: SectionKey) -> i64 {
    let d = |a: i32, b: i32| i64::from(a) - i64::from(b);
    let (dx, dy, dz) = (d(key.0, centre.0), d(key.1, centre.1), d(key.2, centre.2));
    dx * dx + dy * dy + dz * dz
}

/// The bytes of a mesh's vertices, as the upload budget counts them.
pub fn mesh_bytes(mesh: &SectionMesh) -> u64 {
    ((mesh.solid.len() + mesh.translucent.len()) * std::mem::size_of::<TerrainVertex>()) as u64
}

/// A frame's limits on section uploads: the player's Chunk Updates and
/// Upload Budget options (0 is no limit), and the time ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Sections a frame uploads at most; 0 is no limit.
    pub count: usize,
    /// Vertex bytes a frame uploads at most, though always one section; 0
    /// is no limit.
    pub bytes: u64,
    /// The time after which only sections within `NEAR` go.
    pub time: Duration,
}

impl Budget {
    /// The player's options, in sections and KiB, and the time
    /// `time_budget` gives after a frame that waited `gpu_wait`.
    pub fn new(chunk_updates: u32, upload_budget_kib: u32, gpu_wait: Duration) -> Self {
        Self {
            count: chunk_updates as usize,
            bytes: u64::from(upload_budget_kib) * 1024,
            time: time_budget(gpu_wait),
        }
    }
}

/// What a frame has uploaded so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spent {
    pub count: usize,
    pub bytes: u64,
    pub time: Duration,
}

/// Whether a frame that has spent `spent` uploads the next section, of
/// `bytes` vertex bytes at squared distance `far` from the camera's section.
/// The first section of a frame always goes, so the queue always moves; the
/// player's count is a hard limit, as the option says; the byte budget holds
/// back only a section past it; the time ceiling holds back only sections
/// outside `NEAR`, so the far rings wait and the ground underfoot does not.
pub fn may_upload(budget: &Budget, spent: &Spent, bytes: u64, far: i64) -> bool {
    if spent.count == 0 {
        return true;
    }
    if budget.count > 0 && spent.count >= budget.count {
        return false;
    }
    if budget.bytes > 0 && spent.bytes + bytes > budget.bytes {
        return false;
    }
    spent.time < budget.time || far <= NEAR
}

/// The sections waiting for an upload, each with its latest mesh.
#[derive(Default)]
pub struct UploadQueue {
    /// The latest mesh of each waiting section and the number it was queued
    /// with: a heap entry with another number is stale.
    meshes: FxHashMap<SectionKey, (u64, SectionMesh)>,
    /// (distance to `centre`, queue number, section), nearest first, then
    /// oldest first. Holds stale entries until they reach the top or the
    /// heap is rebuilt.
    heap: BinaryHeap<Reverse<(i64, u64, SectionKey)>>,
    centre: SectionKey,
    next: u64,
}

impl UploadQueue {
    pub fn len(&self) -> usize {
        self.meshes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }
    pub fn contains(&self, key: &SectionKey) -> bool {
        self.meshes.contains_key(key)
    }
    /// The waiting sections and their meshes, in no order.
    pub fn iter(&self) -> impl Iterator<Item = (&SectionKey, &SectionMesh)> {
        self.meshes.iter().map(|(k, (_, mesh))| (k, mesh))
    }
    pub fn clear(&mut self) {
        self.meshes.clear();
        self.heap.clear();
    }

    /// The camera's section, which the order follows: a new one re-keys
    /// the heap, once per section the camera enters.
    pub fn set_centre(&mut self, centre: SectionKey) {
        if centre != self.centre {
            self.centre = centre;
            self.rebuild();
        }
    }
    fn rebuild(&mut self) {
        let centre = self.centre;
        self.heap = self
            .meshes
            .iter()
            .map(|(&k, &(n, _))| Reverse((distance(centre, k), n, k)))
            .collect();
    }

    /// A meshed section: it replaces any mesh of the section still waiting.
    pub fn push(&mut self, key: SectionKey, mesh: SectionMesh) {
        let n = self.next;
        self.next += 1;
        self.meshes.insert(key, (n, mesh));
        self.heap
            .push(Reverse((distance(self.centre, key), n, key)));
    }

    /// Drops the sections of the columns `keep` refuses (unloaded ones).
    pub fn retain_columns(&mut self, mut keep: impl FnMut(i32, i32) -> bool) {
        self.meshes.retain(|k, _| keep(k.0, k.2));
        // Stale entries leave the heap as they reach its top; one that
        // holds many more of them than live ones is rebuilt.
        if self.heap.len() > 2 * self.meshes.len() + 64 {
            self.rebuild();
        }
    }

    /// The nearest waiting section and its mesh, when `take` accepts it
    /// (given the mesh and its squared distance); otherwise it stays.
    pub fn pop_if(
        &mut self,
        mut take: impl FnMut(&SectionMesh, i64) -> bool,
    ) -> Option<(SectionKey, SectionMesh)> {
        while let Some(&Reverse((far, n, key))) = self.heap.peek() {
            match self.meshes.get(&key) {
                Some((m, mesh)) if *m == n => {
                    if !take(mesh, far) {
                        return None;
                    }
                    self.heap.pop();
                    return self.meshes.remove(&key).map(|(_, mesh)| (key, mesh));
                }
                // Replaced by a newer mesh, or its column unloaded.
                _ => {
                    self.heap.pop();
                }
            }
        }
        None
    }

    /// Uploads with `upload` the nearest waiting sections a frame's `budget`
    /// allows, timing them; returns what the frame spent.
    pub fn drain(
        &mut self,
        budget: &Budget,
        mut upload: impl FnMut(SectionKey, SectionMesh),
    ) -> Spent {
        let started = Instant::now();
        let mut spent = Spent::default();
        while let Some((key, mesh)) = self.pop_if(|mesh, far| {
            spent.time = started.elapsed();
            may_upload(budget, &spent, mesh_bytes(mesh), far)
        }) {
            spent.count += 1;
            spent.bytes += mesh_bytes(&mesh);
            upload(key, mesh);
        }
        spent.time = started.elapsed();
        spent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(vertices: usize) -> SectionMesh {
        SectionMesh {
            solid: vec![bytemuck::Zeroable::zeroed(); vertices],
            ..SectionMesh::default()
        }
    }
    fn unlimited() -> Budget {
        Budget {
            count: 0,
            bytes: 0,
            time: Duration::MAX,
        }
    }
    fn order(q: &mut UploadQueue, budget: &Budget) -> Vec<SectionKey> {
        let mut out = Vec::new();
        q.drain(budget, |k, _| out.push(k));
        out
    }

    #[test]
    fn nearest_sections_go_first_and_ties_in_arrival_order() {
        let mut q = UploadQueue::default();
        q.set_centre((0, 8, 0));
        for k in [
            (5, 8, 0),
            (0, 8, 1),
            (0, 9, 0),
            (-1, 8, 0),
            (0, 8, 0),
            (3, 3, 3),
        ] {
            q.push(k, mesh(1));
        }
        assert_eq!(
            order(&mut q, &unlimited()),
            [
                (0, 8, 0),
                (0, 8, 1),
                (0, 9, 0),
                (-1, 8, 0),
                (5, 8, 0),
                (3, 3, 3)
            ]
        );
        assert!(q.is_empty());
    }

    #[test]
    fn a_new_centre_reorders_what_waits() {
        let mut q = UploadQueue::default();
        q.set_centre((0, 0, 0));
        for x in 0..10 {
            q.push((x, 0, 0), mesh(1));
        }
        q.set_centre((9, 0, 0));
        let got = order(&mut q, &unlimited());
        assert_eq!(got, (0..10).rev().map(|x| (x, 0, 0)).collect::<Vec<_>>());
    }

    #[test]
    fn a_newer_mesh_replaces_the_waiting_one() {
        let mut q = UploadQueue::default();
        q.push((1, 0, 0), mesh(1));
        q.push((2, 0, 0), mesh(1));
        q.push((1, 0, 0), mesh(7));
        assert_eq!(q.len(), 2);
        let mut got = Vec::new();
        q.drain(&unlimited(), |k, m| got.push((k, m.solid.len())));
        assert_eq!(got, [((1, 0, 0), 7), ((2, 0, 0), 1)]);
    }

    #[test]
    fn unloaded_columns_leave_and_the_heap_stays_small() {
        let mut q = UploadQueue::default();
        for x in 0..1000 {
            q.push((x, 0, 0), mesh(1));
        }
        q.retain_columns(|x, _| x % 100 == 0);
        assert_eq!(q.len(), 10);
        assert!(q.heap.len() <= 2 * q.len() + 64);
        assert!(q.contains(&(300, 0, 0)) && !q.contains(&(301, 0, 0)));
        let got = order(&mut q, &unlimited());
        assert_eq!(got, (0..10).map(|i| (i * 100, 0, 0)).collect::<Vec<_>>());
    }

    #[test]
    fn the_players_count_and_bytes_hold() {
        let mut q = UploadQueue::default();
        for x in 0..20 {
            q.push((x, 0, 0), mesh(10));
        }
        let count = Budget {
            count: 3,
            ..unlimited()
        };
        assert_eq!(order(&mut q, &count).len(), 3);
        let bytes = mesh_bytes(&mesh(10));
        let budget = Budget {
            bytes: bytes * 5 + 1,
            ..unlimited()
        };
        assert_eq!(order(&mut q, &budget).len(), 5);
        // A section bigger than the whole budget still goes alone.
        let tiny = Budget {
            bytes: 1,
            ..unlimited()
        };
        assert_eq!(order(&mut q, &tiny).len(), 1);
        assert_eq!(q.len(), 11);
    }

    #[test]
    fn the_time_ceiling_holds_back_only_the_far_sections() {
        let budget = Budget {
            count: 0,
            bytes: 0,
            time: Duration::from_millis(2),
        };
        let late = Spent {
            count: 50,
            bytes: 1 << 20,
            time: Duration::from_millis(3),
        };
        // The 3x3x3 sections around the camera go, however late.
        assert!(may_upload(&budget, &late, 4096, 0));
        assert!(may_upload(&budget, &late, 4096, NEAR));
        assert!(!may_upload(&budget, &late, 4096, NEAR + 1));
        // Within the time, anything goes; the first section always does.
        let early = Spent {
            time: Duration::from_micros(500),
            ..late
        };
        assert!(may_upload(&budget, &early, 4096, 1000));
        assert!(may_upload(&budget, &Spent::default(), 4096, 1000));
        let first_late = Spent { count: 0, ..late };
        assert!(may_upload(&budget, &first_late, 4096, 1000));
        // The player's count still holds the near sections.
        let counted = Budget {
            count: 50,
            ..budget
        };
        assert!(!may_upload(&counted, &late, 4096, 0));
    }

    #[test]
    fn the_time_budget_takes_the_last_frames_gpu_wait() {
        let ms = Duration::from_millis;
        // No wait, or less than the floor and margin: the 2 ms ceiling.
        assert_eq!(time_budget(Duration::ZERO), TIME_PER_FRAME);
        assert_eq!(time_budget(ms(3)), ms(2));
        // A GPU-bound frame's wait, less the margin.
        assert_eq!(time_budget(ms(5)), ms(4));
        assert_eq!(
            time_budget(Duration::from_micros(7500)),
            Duration::from_micros(6500)
        );
        // Never past the cap, however long the wait (a stalled display).
        assert_eq!(time_budget(ms(9)), MOST_PER_FRAME);
        assert_eq!(time_budget(Duration::from_secs(5)), MOST_PER_FRAME);
        let b = Budget::new(0, 0, ms(6));
        assert_eq!((b.count, b.bytes, b.time), (0, 0, ms(5)));
    }

    #[test]
    fn a_zero_time_budget_still_makes_progress_and_serves_the_near_ring() {
        let mut q = UploadQueue::default();
        q.set_centre((0, 4, 0));
        for x in -3..=3 {
            q.push((x, 4, 0), mesh(1));
        }
        let budget = Budget {
            count: 0,
            bytes: 0,
            time: Duration::ZERO,
        };
        // The first section, then those within NEAR, the older first.
        let got = order(&mut q, &budget);
        assert_eq!(got, [(0, 4, 0), (-1, 4, 0), (1, 4, 0)]);
        // A later frame with only far ones left uploads one of them.
        assert_eq!(order(&mut q, &budget).len(), 1);
    }
}
