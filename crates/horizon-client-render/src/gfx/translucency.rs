//! Vanilla 26.3 MeshData.SortState / VertexSorting.byDistance.
//! Keep vertex order intact; only the quad indices change as the camera moves.

/// MeshData.decodeQuadCentroids uses the opposite corners, not all four vertices.
pub fn quad_center(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| (a[i] + b[i]) / 2.0)
}

/// Fixed-point encoding for section-local quad centers (`[-8.0, 23.999]`):
/// `SCALE = 2048.0` represents every power-of-two fraction down to 1/2048 block
/// (~0.5 mm, well below `fluid::EPSILON = 0.001`) bit-exact while cutting center
/// storage from 12 B/quad to 6 B/quad (2x L1 cache density during sorting).
const COMPACT_OFFSET: f32 = 8.0;
const COMPACT_SCALE: f32 = 2048.0;
const COMPACT_INV_SCALE: f32 = 1.0 / COMPACT_SCALE;
const COMPACT_MIN: f32 = -8.0;
const COMPACT_MAX: f32 = 23.999;

/// Below this quad count, in-place unstable sort on packed `(dist_key, quad)` `u64`
/// beats radix sort's histogram pass; at or above it, 4-pass 8-bit LSD radix sort
/// with per-byte skip runs in O(n) with zero allocations.
const RADIX_THRESHOLD: usize = 128;

#[inline(always)]
fn dist_key(d2: f32) -> u32 {
    // `d2` is a sum of squares (non-negative or NaN): clearing the sign bit and
    // inverting turns IEEE-754 order into descending distance order, matching
    // `b.total_cmp(&a)`.
    !(d2.to_bits() & 0x7fff_ffff)
}

#[inline(always)]
fn encode_compact(v: f32) -> u16 {
    ((v + COMPACT_OFFSET) * COMPACT_SCALE).round() as u16
}

/// Whether every quad lies on a single grid-aligned horizontal plane inside the
/// section (e.g., flat ocean water at `y = iy + 0.887889` or flat ice/glass top
/// at integer `y`). Disjoint horizontal tiles on one plane never overlap in screen
/// space, and co-located back faces (water underside) are back-face culled by the
/// translucent terrain pipelines (`cull_mode: Back`/`Front`) and share identical
/// distance anyway. Rejects `y.fract() == 0.5` and off-center X/Z so vertical
/// glass/pane/portal/waterfall faces in one Y layer are never misclassified.
fn is_coplanar_horizontal(centers: &[[f32; 3]]) -> bool {
    let Some(&[_, y0, _]) = centers.first() else {
        return true;
    };
    if !(0.0..=16.0).contains(&y0) || y0.fract() == 0.5 {
        return false;
    }
    centers.iter().all(|c| {
        c[1] == y0
            && (0.0..=16.0).contains(&c[0])
            && (0.0..=16.0).contains(&c[2])
            && c[0].fract() == 0.5
            && c[2].fract() == 0.5
    })
}

fn fits_compact(centers: &[[f32; 3]]) -> bool {
    centers.iter().all(|c| {
        (COMPACT_MIN..=COMPACT_MAX).contains(&c[0])
            && (COMPACT_MIN..=COMPACT_MAX).contains(&c[1])
            && (COMPACT_MIN..=COMPACT_MAX).contains(&c[2])
    })
}

#[derive(Default)]
pub struct Sorter {
    keys: Vec<u64>,
    radix_a: Vec<(u32, u32)>,
    radix_b: Vec<(u32, u32)>,
    indices: Vec<u32>,
}

impl Sorter {
    pub fn sort(&mut self, centers: &[[f32; 3]], camera: [f32; 3]) -> &[u32] {
        self.sort_by_dist(centers, |c| {
            let dx = camera[0] - c[0];
            let dy = camera[1] - c[1];
            let dz = camera[2] - c[2];
            dx * dx + dy * dy + dz * dz
        })
    }

    fn sort_compact(&mut self, centers: &[[u16; 3]], camera: [f32; 3]) -> &[u32] {
        let cam = [
            camera[0] + COMPACT_OFFSET,
            camera[1] + COMPACT_OFFSET,
            camera[2] + COMPACT_OFFSET,
        ];
        self.sort_by_dist(centers, |c| {
            let dx = cam[0] - f32::from(c[0]) * COMPACT_INV_SCALE;
            let dy = cam[1] - f32::from(c[1]) * COMPACT_INV_SCALE;
            let dz = cam[2] - f32::from(c[2]) * COMPACT_INV_SCALE;
            dx * dx + dy * dy + dz * dz
        })
    }

    fn identity(&mut self, quads: u32) -> &[u32] {
        let len = quads as usize;
        self.indices.clear();
        self.indices.reserve(len * 6);
        for quad in 0..quads {
            let v = quad * 4;
            self.indices
                .extend_from_slice(&[v, v + 1, v + 2, v + 2, v + 3, v]);
        }
        &self.indices
    }

    fn sort_by_dist<T>(&mut self, items: &[T], mut dist_sq: impl FnMut(&T) -> f32) -> &[u32] {
        let len = items.len();
        self.indices.clear();
        if len == 0 {
            return &self.indices;
        }
        if len == 1 {
            self.indices.extend_from_slice(&[0, 1, 2, 2, 3, 0]);
            return &self.indices;
        }
        self.indices.reserve(len * 6);
        if len < RADIX_THRESHOLD {
            // Pack `(dist_key << 32) | quad_index` so an in-place unstable sort
            // preserves exact mesh order on ties with zero heap allocation.
            self.keys.clear();
            self.keys.reserve(len);
            for (i, item) in items.iter().enumerate() {
                let key = (u64::from(dist_key(dist_sq(item))) << 32) | u64::from(i as u32);
                self.keys.push(key);
            }
            self.keys.sort_unstable();
            for &entry in &self.keys {
                let v = (entry as u32) * 4;
                self.indices
                    .extend_from_slice(&[v, v + 1, v + 2, v + 2, v + 3, v]);
            }
            return &self.indices;
        }
        // 4-pass 8-bit LSD radix sort with per-byte skip: O(n), stable (preserves
        // mesh order on exact ties), and zero heap allocation after warm-up.
        let mut hist = [[0u32; 256]; 4];
        self.radix_a.clear();
        self.radix_a.reserve(len);
        for (i, item) in items.iter().enumerate() {
            let key = dist_key(dist_sq(item));
            hist[0][(key & 0xff) as usize] += 1;
            hist[1][((key >> 8) & 0xff) as usize] += 1;
            hist[2][((key >> 16) & 0xff) as usize] += 1;
            hist[3][(key >> 24) as usize] += 1;
            self.radix_a.push((key, i as u32));
        }
        let first_key = self.radix_a[0].0;
        let mut from_a = true;
        for (pass, counts) in hist.iter_mut().enumerate() {
            let shift = pass * 8;
            let first_bucket = ((first_key >> shift) & 0xff) as usize;
            if counts[first_bucket] as usize == len {
                continue;
            }
            let mut sum = 0u32;
            for c in counts.iter_mut() {
                let n = *c;
                *c = sum;
                sum += n;
            }
            if from_a {
                self.radix_b.resize(len, (0, 0));
                for &(key, quad) in &self.radix_a {
                    let b = ((key >> shift) & 0xff) as usize;
                    let dst = counts[b] as usize;
                    counts[b] = (dst + 1) as u32;
                    self.radix_b[dst] = (key, quad);
                }
            } else {
                for &(key, quad) in &self.radix_b {
                    let b = ((key >> shift) & 0xff) as usize;
                    let dst = counts[b] as usize;
                    counts[b] = (dst + 1) as u32;
                    self.radix_a[dst] = (key, quad);
                }
            }
            from_a = !from_a;
        }
        let sorted = if from_a { &self.radix_a } else { &self.radix_b };
        for &(_, quad) in sorted {
            let v = quad * 4;
            self.indices
                .extend_from_slice(&[v, v + 1, v + 2, v + 2, v + 3, v]);
        }
        &self.indices
    }
}

enum Centers {
    /// Flat single-plane horizontal surface (e.g., ocean water): identity order
    /// is emitted once on first visibility and never re-sorted. Zero per-quad RAM.
    Coplanar(u32),
    /// Section-local fixed-point coordinates in `[-8.0, 23.999]`, 6 B/quad.
    Compact(Box<[[u16; 3]]>),
    /// Out-of-section coordinates (LOD tiles or overhanging custom models).
    Full(Box<[[f32; 3]]>),
}

pub struct SortState {
    centers: Centers,
    last_block: [i32; 3],
    last_pov: [i8; 3],
    sorted: bool,
}

impl SortState {
    pub fn new(centers: Vec<[f32; 3]>) -> Self {
        let centers = if is_coplanar_horizontal(&centers) {
            Centers::Coplanar(centers.len() as u32)
        } else if fits_compact(&centers) {
            let compact: Box<[[u16; 3]]> = centers
                .into_iter()
                .map(|c| {
                    [
                        encode_compact(c[0]),
                        encode_compact(c[1]),
                        encode_compact(c[2]),
                    ]
                })
                .collect();
            Centers::Compact(compact)
        } else {
            Centers::Full(centers.into_boxed_slice())
        };
        Self {
            centers,
            last_block: [0; 3],
            last_pov: [0; 3],
            sorted: false,
        }
    }

    pub fn is_sorted(&self) -> bool {
        self.sorted
    }

    fn sort_now<'a>(
        &self,
        camera: [f64; 3],
        origin: [i32; 3],
        sorter: &'a mut Sorter,
    ) -> &'a [u32] {
        let local = std::array::from_fn(|i| (camera[i] - f64::from(origin[i])) as f32);
        match &self.centers {
            Centers::Coplanar(quads) => sorter.identity(*quads),
            Centers::Compact(centers) => sorter.sort_compact(centers, local),
            Centers::Full(centers) => sorter.sort(centers, local),
        }
    }

    /// Large tiles cannot use the section viewpoint shortcut: the camera may
    /// cross a quad's plane while remaining on the same side of the first 16³.
    pub fn update_lod<'a>(
        &mut self,
        camera: [f64; 3],
        origin: [i32; 3],
        sorter: &'a mut Sorter,
    ) -> Option<&'a [u32]> {
        if let Centers::Coplanar(quads) = self.centers {
            if self.sorted {
                return None;
            }
            self.sorted = true;
            return Some(sorter.identity(quads));
        }
        let block = camera.map(|v| v.floor() as i32);
        if self.sorted && self.last_block == block {
            return None;
        }
        self.last_block = block;
        self.last_pov = [0; 3];
        self.sorted = true;
        Some(self.sort_now(camera, origin, sorter))
    }

    /// TranslucencyPointOfView and ResortTransparencyTask: diagonal views keep
    /// their ordering until the point of view changes; aligned views also
    /// resort on crossing a block boundary. Sub-block movement needs no upload.
    pub fn update<'a>(
        &mut self,
        camera: [f64; 3],
        origin: [i32; 3],
        sorter: &'a mut Sorter,
    ) -> Option<&'a [u32]> {
        if let Centers::Coplanar(quads) = self.centers {
            if self.sorted {
                return None;
            }
            self.sorted = true;
            return Some(sorter.identity(quads));
        }
        let block = camera.map(|v| v.floor() as i32);
        let pov = std::array::from_fn(|i| ((block[i] >> 4) - (origin[i] >> 4)).clamp(-1, 1) as i8);
        if self.sorted && self.last_pov == pov && (self.last_block == block || !pov.contains(&0)) {
            return None;
        }
        self.last_block = block;
        self.last_pov = pov;
        self.sorted = true;
        Some(self.sort_now(camera, origin, sorter))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_distances_keep_mesh_order_and_triangle_winding() {
        let mut sorter = Sorter::default();
        assert_eq!(
            sorter.sort(
                &[[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [4.0, 0.0, 0.0]],
                [0.0; 3]
            ),
            &[8, 9, 10, 10, 11, 8, 0, 1, 2, 2, 3, 0, 4, 5, 6, 6, 7, 4]
        );
        assert!(sorter.sort(&[], [0.0; 3]).is_empty());
    }

    #[test]
    fn movement_reverses_faces_without_moving_vertices() {
        let centers = [[0.5, 0.5, 0.0], [0.5, 0.5, 1.0]];
        let mut sorter = Sorter::default();
        assert_eq!(sorter.sort(&centers, [0.5, 0.5, -2.0])[0], 4);
        assert_eq!(sorter.sort(&centers, [0.5, 0.5, 3.0])[0], 0);
    }

    #[test]
    fn cached_viewpoint_handles_negative_and_large_coordinates() {
        let origin = [-30_000_000, -64, 30_000_000];
        let mut state = SortState::new(vec![[0.5; 3]]);
        let mut sorter = Sorter::default();
        let camera = [-29_999_999.5, -63.5, 30_000_000.5];
        assert!(state.update(camera, origin, &mut sorter).is_some());
        assert!(state.update(camera, origin, &mut sorter).is_none());
        assert!(
            state
                .update([-29_999_999.1, -63.1, 30_000_000.9], origin, &mut sorter)
                .is_none()
        );
        assert!(
            state
                .update([-29_999_998.9, -63.1, 30_000_000.9], origin, &mut sorter)
                .is_some()
        );
    }
    #[test]
    fn large_tiles_resort_inside_the_same_section_viewpoint() {
        let mut state = SortState::new(vec![[32.0, 0.0, 0.0], [96.0, 0.0, 0.0]]);
        let mut sorter = Sorter::default();
        assert_eq!(
            state
                .update_lod([48.0, 20.0, 20.0], [0; 3], &mut sorter)
                .unwrap()[0],
            4
        );
        assert!(
            state
                .update_lod([48.5, 20.0, 20.0], [0; 3], &mut sorter)
                .is_none()
        );
        // Both views have positive x/y/z signs relative to the first section,
        // so the section-sized shortcut would incorrectly keep the old order.
        assert_eq!(
            state
                .update_lod([80.0, 20.0, 20.0], [0; 3], &mut sorter)
                .unwrap()[0],
            0
        );
    }

    #[test]
    fn coplanar_flat_ocean_emits_once_and_never_resorts() {
        let mut centers = Vec::with_capacity(512);
        let y = 14.0 + 0.888_888_9 - 0.001;
        for z in 0..16 {
            for x in 0..16 {
                let c = [x as f32 + 0.5, y, z as f32 + 0.5];
                // Top and back-facing underside share the same center.
                centers.push(c);
                centers.push(c);
            }
        }
        let mut state = SortState::new(centers);
        assert!(matches!(state.centers, Centers::Coplanar(512)));
        assert!(!state.is_sorted());
        let mut sorter = Sorter::default();
        let first = state
            .update([8.5, 70.0, 8.5], [0, 48, 0], &mut sorter)
            .expect("first frame uploads identity indices");
        assert_eq!(first.len(), 512 * 6);
        assert_eq!(&first[..12], &[0, 1, 2, 2, 3, 0, 4, 5, 6, 6, 7, 4]);
        assert!(state.is_sorted());
        // Crossing blocks, sections, and moving above/below never resorts.
        for cam in [
            [9.5, 70.0, 8.5],
            [8.5, 50.0, 8.5],
            [-40.0, 100.0, 120.0],
            [0.1, 62.0, 0.1],
        ] {
            assert!(state.update(cam, [0, 48, 0], &mut sorter).is_none());
        }
    }

    #[test]
    fn vertical_faces_at_same_y_still_resort() {
        // Two parallel vertical glass faces in the same Y layer share `y = 0.5`.
        let mut state = SortState::new(vec![[0.5, 0.5, 0.0], [0.5, 0.5, 1.0]]);
        assert!(matches!(state.centers, Centers::Compact(_)));
        let mut sorter = Sorter::default();
        assert_eq!(
            state.update([0.5, 0.5, -2.0], [0; 3], &mut sorter).unwrap()[0],
            4
        );
        assert_eq!(
            state.update([0.5, 0.5, 3.0], [0; 3], &mut sorter).unwrap()[0],
            0
        );
    }

    #[test]
    fn radix_and_compact_sort_match_stable_reference() {
        let mut centers = Vec::with_capacity(512);
        for i in 0..512u32 {
            // Mix distinct distances, exact ties, and fluid-epsilon offsets across Y layers.
            let x = (i % 16) as f32 + 0.5;
            let y = ((i / 16) % 4) as f32 + if i % 7 == 0 { 0.001 } else { 0.5 };
            let z = ((i / 64) % 8) as f32 + 0.5;
            centers.push([x, y, z]);
        }
        let camera = [7.25f32, 2.75, 3.125];
        let mut reference: Vec<(u32, f32)> = centers
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let dx = camera[0] - c[0];
                let dy = camera[1] - c[1];
                let dz = camera[2] - c[2];
                (i as u32, dx * dx + dy * dy + dz * dz)
            })
            .collect();
        reference.sort_by(|a, b| b.1.total_cmp(&a.1));
        let expected: Vec<u32> = reference
            .iter()
            .flat_map(|&(q, _)| {
                let v = q * 4;
                [v, v + 1, v + 2, v + 2, v + 3, v]
            })
            .collect();
        let mut sorter = Sorter::default();
        assert_eq!(sorter.sort(&centers, camera), expected.as_slice());

        // Compact storage produces the exact same sorted order for section-local quads.
        let mut state = SortState::new(centers);
        assert!(matches!(state.centers, Centers::Compact(_)));
        let got = state
            .update(camera.map(f64::from), [0; 3], &mut sorter)
            .unwrap();
        assert_eq!(got, expected.as_slice());
    }
}
