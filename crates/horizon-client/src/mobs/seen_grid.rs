//! Ordered snapshot box queries and sparse per-region speculative overlays.
//! An overlay copies only cells changed by its own entities. It reproduces the
//! serial grid's candidate order, growing reach and retained empty cells.
use crate::fast_hash::FxHashMap;
use horizon_core::geometry::{Aabb, Axis};

const CELL: f64 = 4.0;
pub type Cell = (i32, i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Columns {
    pub lo: Cell,
    pub hi: Cell,
}
impl Columns {
    pub fn contains(self, cell: Cell) -> bool {
        cell.0 >= self.lo.0 && cell.0 <= self.hi.0 && cell.1 >= self.lo.1 && cell.1 <= self.hi.1
    }
    pub fn count(self) -> u64 {
        let span = |lo: i32, hi: i32| (i64::from(hi) - i64::from(lo) + 1) as u64;
        span(self.lo.0, self.hi.0).saturating_mul(span(self.lo.1, self.hi.1))
    }
    pub fn cells(self) -> impl Iterator<Item = Cell> {
        (self.lo.0..=self.hi.0).flat_map(move |x| (self.lo.1..=self.hi.1).map(move |z| (x, z)))
    }
}

/// The same query interface for a committed snapshot and a speculative overlay.
pub trait SnapshotGrid: Sync {
    /// None means the serial grid falls back to the complete snapshot.
    fn columns(&self, area: Aabb) -> Option<Columns>;
    fn candidates(&self, area: Aabb, out: &mut Vec<u32>) -> bool;
    /// For callers that enforce their geometric filter, keep spatial reads
    /// local even when scanning populated cells is cheaper than empty columns.
    fn spatial_columns(&self, area: Aabb) -> Option<Columns>;
    fn spatial_candidates(&self, area: Aabb, out: &mut Vec<u32>) -> bool;
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct SeenGrid {
    cells: FxHashMap<Cell, Vec<u32>>,
    cell_of: Vec<Cell>,
    reach: f64,
}
fn column(x: f64, z: f64) -> Cell {
    ((x / CELL).floor() as i32, (z / CELL).floor() as i32)
}
pub fn corner(b: Aabb) -> Cell {
    column(b.min(Axis::X), b.min(Axis::Z))
}
pub(super) fn width(b: Aabb) -> f64 {
    (b.max(Axis::X) - b.min(Axis::X)).max(b.max(Axis::Z) - b.min(Axis::Z))
}
fn spatial_columns(area: Aabb, reach: f64) -> Option<Columns> {
    let margin = reach + 1.0;
    let lo = column(area.min(Axis::X) - margin, area.min(Axis::Z) - margin);
    let hi = column(area.max(Axis::X), area.max(Axis::Z));
    (lo.0 <= hi.0 && lo.1 <= hi.1).then_some(Columns { lo, hi })
}
fn columns(area: Aabb, reach: f64, count: usize) -> Option<Columns> {
    spatial_columns(area, reach).filter(|columns| columns.count() <= count as u64)
}
/// The columns a query covers on a grid of this reach and this many cells.
pub(super) fn query_columns(
    area: Aabb,
    spatial: bool,
    reach: f64,
    cells: usize,
) -> Option<Columns> {
    if spatial {
        spatial_columns(area, reach)
    } else {
        columns(area, reach, cells)
    }
}

impl SeenGrid {
    pub fn new(boxes: impl Iterator<Item = Aabb>) -> Self {
        let mut grid = Self::default();
        for (i, b) in boxes.enumerate() {
            let at = corner(b);
            grid.reach = grid.reach.max(width(b));
            grid.cells.entry(at).or_default().push(i as u32);
            grid.cell_of.push(at);
        }
        grid
    }
    pub(super) fn reach(&self) -> f64 {
        self.reach
    }
    pub(super) fn cell_count(&self) -> usize {
        self.cells.len()
    }
    pub(super) fn has_cell(&self, cell: Cell) -> bool {
        self.cells.contains_key(&cell)
    }
    pub fn update(&mut self, i: usize, b: Aabb) {
        self.reach = self.reach.max(width(b));
        let at = corner(b);
        let old = std::mem::replace(&mut self.cell_of[i], at);
        if old == at {
            return;
        }
        if let Some(list) = self.cells.get_mut(&old)
            && let Some(k) = list.iter().position(|&j| j == i as u32)
        {
            list.swap_remove(k);
        }
        self.cells.entry(at).or_default().push(i as u32);
    }
    pub fn columns(&self, area: Aabb) -> Option<Columns> {
        columns(area, self.reach, self.cells.len())
    }
    pub fn candidates(&self, area: Aabb, out: &mut Vec<u32>) -> bool {
        out.clear();
        let Some(columns) = self.columns(area) else {
            return false;
        };
        for cell in columns.cells() {
            if let Some(list) = self.cells.get(&cell) {
                out.extend_from_slice(list);
            }
        }
        out.sort_unstable();
        true
    }
}
impl SnapshotGrid for SeenGrid {
    fn columns(&self, area: Aabb) -> Option<Columns> {
        self.columns(area)
    }
    fn candidates(&self, area: Aabb, out: &mut Vec<u32>) -> bool {
        self.candidates(area, out)
    }
    fn spatial_columns(&self, area: Aabb) -> Option<Columns> {
        spatial_columns(area, self.reach)
    }
    fn spatial_candidates(&self, area: Aabb, out: &mut Vec<u32>) -> bool {
        out.clear();
        let Some(columns) = self.spatial_columns(area) else {
            return false;
        };
        if columns.count() <= self.cells.len() as u64 {
            for cell in columns.cells() {
                if let Some(list) = self.cells.get(&cell) {
                    out.extend_from_slice(list);
                }
            }
        } else {
            for (&cell, list) in &self.cells {
                if columns.contains(cell) {
                    out.extend_from_slice(list);
                }
            }
        }
        out.sort_unstable();
        true
    }
}

/// Region-owned changes over an immutable shared grid. Empty cells are kept,
/// exactly as in SeenGrid::update: their presence affects the scan fallback.
pub struct GridOverlay<'a> {
    base: &'a SeenGrid,
    cells: FxHashMap<Cell, Vec<u32>>,
    cell_of: FxHashMap<usize, Cell>,
    added_cells: usize,
    reach: f64,
}
impl<'a> GridOverlay<'a> {
    pub fn new(base: &'a SeenGrid) -> Self {
        Self {
            base,
            cells: Default::default(),
            cell_of: Default::default(),
            added_cells: 0,
            reach: base.reach,
        }
    }
    fn cell_mut(&mut self, at: Cell) -> &mut Vec<u32> {
        self.cells
            .entry(at)
            .or_insert_with(|| match self.base.cells.get(&at) {
                Some(list) => list.clone(),
                None => {
                    self.added_cells += 1;
                    Vec::new()
                }
            })
    }
    pub fn update(&mut self, slot: usize, bounds: Aabb) {
        self.reach = self.reach.max(width(bounds));
        let at = corner(bounds);
        let old = self
            .cell_of
            .insert(slot, at)
            .unwrap_or(self.base.cell_of[slot]);
        if old == at {
            return;
        }
        let list = self.cell_mut(old);
        if let Some(i) = list.iter().position(|&i| i == slot as u32) {
            list.swap_remove(i);
        }
        self.cell_mut(at).push(slot as u32);
    }
}
impl SnapshotGrid for GridOverlay<'_> {
    fn columns(&self, area: Aabb) -> Option<Columns> {
        columns(area, self.reach, self.base.cells.len() + self.added_cells)
    }
    fn candidates(&self, area: Aabb, out: &mut Vec<u32>) -> bool {
        out.clear();
        let Some(columns) = self.columns(area) else {
            return false;
        };
        for cell in columns.cells() {
            if let Some(list) = self.cells.get(&cell).or_else(|| self.base.cells.get(&cell)) {
                out.extend_from_slice(list);
            }
        }
        out.sort_unstable();
        true
    }
    fn spatial_columns(&self, area: Aabb) -> Option<Columns> {
        spatial_columns(area, self.reach)
    }
    fn spatial_candidates(&self, area: Aabb, out: &mut Vec<u32>) -> bool {
        out.clear();
        let Some(columns) = self.spatial_columns(area) else {
            return false;
        };
        if columns.count() <= (self.base.cells.len() + self.added_cells) as u64 {
            for cell in columns.cells() {
                if let Some(list) = self.cells.get(&cell).or_else(|| self.base.cells.get(&cell)) {
                    out.extend_from_slice(list);
                }
            }
        } else {
            for (&cell, list) in &self.base.cells {
                if columns.contains(cell) {
                    out.extend_from_slice(self.cells.get(&cell).unwrap_or(list));
                }
            }
            for (&cell, list) in &self.cells {
                if columns.contains(cell) && !self.base.cells.contains_key(&cell) {
                    out.extend_from_slice(list);
                }
            }
        }
        out.sort_unstable();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::Vec3;

    fn boxed(x: f64, y: f64, z: f64, w: f64) -> Aabb {
        Aabb::new(Vec3::new(x, y, z), Vec3::new(x + w, y + 1.8, z + w))
    }

    #[test]
    fn sparse_queries_filter_all_boxes_without_a_global_scan_or_unbounded_walk() {
        let mut boxes: Vec<_> = (0..100)
            .flat_map(|owner| {
                (0..70).map(move |slot| boxed(owner as f64 * 512.0, 0.0, slot as f64 * 0.001, 0.6))
            })
            .collect();
        let mut grid = SeenGrid::new(boxes.iter().copied());
        let mut overlay = GridOverlay::new(&grid);
        let area = boxed(512.0 - 32.0, -1.0, -32.0, 64.0);
        let mut out = Vec::new();
        assert!(
            !grid.candidates(area, &mut out),
            "legacy heuristic uses a full scan here"
        );
        assert!(grid.spatial_candidates(area, &mut out));
        assert_eq!(out, (70..140).collect::<Vec<u32>>());
        let mut predicted = Vec::new();
        assert!(overlay.spatial_candidates(area, &mut predicted));
        assert_eq!(out, predicted);
        let moved = boxed(513.0, 0.0, 0.0, 2.0);
        overlay.update(140, moved);
        assert!(overlay.spatial_candidates(area, &mut predicted));
        assert_eq!(predicted, (70..=140).collect::<Vec<u32>>());
        drop(overlay);
        grid.update(140, moved);
        boxes[140] = moved;
        assert!(grid.spatial_candidates(area, &mut out));
        let expected: Vec<_> = boxes
            .iter()
            .enumerate()
            .filter(|(_, bounds)| bounds.intersects(area))
            .map(|(slot, _)| slot as u32)
            .collect();
        assert_eq!(out, expected);
        let giant = boxed(-1.0e20, -1.0, -1.0e20, 2.0e20);
        assert!(grid.spatial_candidates(giant, &mut out));
        assert_eq!(out, (0..7000).collect::<Vec<u32>>());
    }

    #[test]
    fn candidates_hold_every_meeting_box_in_order() {
        // A small LCG keeps the test free of dependencies.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |range: f64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * range
        };
        let mut boxes: Vec<Aabb> = (0..300)
            .map(|_| {
                boxed(
                    next(64.0) - 32.0,
                    next(8.0),
                    next(64.0) - 32.0,
                    0.3 + next(1.2),
                )
            })
            .collect();
        let mut grid = SeenGrid::new(boxes.iter().copied());
        let mut out = Vec::new();
        let mut indexed = 0;
        for round in 0..2000 {
            // Move one box, sometimes far and wider, as mobs do between queries.
            let i = round % boxes.len();
            let w = if round % 97 == 0 { 3.9 } else { 0.6 };
            boxes[i] = boxed(next(80.0) - 40.0, next(8.0), next(80.0) - 40.0, w);
            grid.update(i, boxes[i]);
            let area = boxed(next(70.0) - 35.0, next(8.0), next(70.0) - 35.0, next(2.0));
            let expected: Vec<u32> = (0..boxes.len() as u32)
                .filter(|&j| boxes[j as usize].intersects(area))
                .collect();
            assert!(grid.spatial_candidates(area, &mut out));
            let actual: Vec<_> = out
                .iter()
                .copied()
                .filter(|&slot| boxes[slot as usize].intersects(area))
                .collect();
            assert_eq!(actual, expected);
            if grid.candidates(area, &mut out) {
                indexed += 1;
                assert!(out.windows(2).all(|p| p[0] < p[1]), "ascending, no repeats");
                let met: Vec<u32> = out
                    .iter()
                    .copied()
                    .filter(|&j| boxes[j as usize].intersects(area))
                    .collect();
                assert_eq!(met, expected);
            }
        }
        assert!(indexed > 1900, "the grid answered {indexed} of 2000");
    }

    #[test]
    fn sparse_overlay_reproduces_serial_candidates_after_movement_and_resizing() {
        let initial: Vec<_> = (0..240)
            .map(|i| boxed((i % 24) as f64 * 4.0, 0.0, (i / 24) as f64 * 4.0, 0.6))
            .collect();
        let base = SeenGrid::new(initial.iter().copied());
        let mut serial = SeenGrid::new(initial.iter().copied());
        let mut overlay = GridOverlay::new(&base);
        for tick in 0..400 {
            // One owner changes only these slots. Other owners remain borrowed.
            let slot = [0, 19, 43, 118][tick % 4];
            let bounds = boxed(
                (tick % 53) as f64 - 20.0,
                0.0,
                (tick % 37) as f64,
                if tick % 23 == 0 { 9.0 } else { 0.6 },
            );
            serial.update(slot, bounds);
            overlay.update(slot, bounds);
            for range in [1.0, 12.0, 48.0, 900.0] {
                let area = boxed((tick % 41) as f64, 0.0, (tick % 29) as f64, range);
                let (mut expected, mut actual) = (Vec::new(), Vec::new());
                assert_eq!(overlay.columns(area), serial.columns(area));
                assert_eq!(
                    overlay.candidates(area, &mut actual),
                    serial.candidates(area, &mut expected)
                );
                assert_eq!(actual, expected);
                assert_eq!(overlay.spatial_columns(area), serial.spatial_columns(area));
                assert!(overlay.spatial_candidates(area, &mut actual));
                assert!(serial.spatial_candidates(area, &mut expected));
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn unbounded_areas_fall_back_to_a_full_scan() {
        let grid = SeenGrid::new([boxed(0.0, 0.0, 0.0, 0.6)].into_iter());
        let mut out = Vec::new();
        let huge = Aabb::new(
            Vec3::new(f64::NEG_INFINITY, 0.0, 0.0),
            Vec3::new(f64::INFINITY, 1.0, 1.0),
        );
        assert!(!grid.candidates(huge, &mut out));
    }
}
