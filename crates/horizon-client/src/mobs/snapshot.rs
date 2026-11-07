//! Observable entity reads made by a region transaction. An unchanged query
//! result includes negative space: an earlier entity entering an empty queried
//! cell invalidates the prediction. No distance/velocity halo is assumed.
use super::{
    Seen, Species,
    seen_grid::{Cell, Columns, SeenGrid, SnapshotGrid, corner, query_columns},
};
use crate::fast_hash::{FxHashMap, FxHashSet};
use horizon_core::geometry::Aabb;
use std::{cell::RefCell, ops::Index, sync::OnceLock};

#[derive(Default, Debug, Clone)]
pub struct ReadSet {
    pub(super) all: bool,
    pub(super) slots: FxHashSet<usize>,
    pub(super) species: FxHashSet<Species>,
    pub(super) queries: Vec<(Aabb, Option<Columns>, bool)>,
}
impl ReadSet {
    pub fn query(&mut self, area: Aabb, columns: Option<Columns>) {
        self.queries.push((area, columns, false));
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ReadConflict {
    FullScan,
    EntitySlot,
    GridShape,
    SpatialCell,
    Species,
}

/// What one owner's predictions would change, if they all commit.
#[derive(Default)]
pub struct OwnerWrites {
    pub cells: Vec<Cell>,
    pub species: Vec<Species>,
    pub reach: f64,
    /// Cells here that the tick's grid does not have yet.
    pub new_cells: usize,
}

/// More than one owner writes this.
const MANY: usize = usize::MAX;

/// What every prediction of a round would change, so that a read can be judged
/// against the whole round before anything is applied. It is stricter than
/// [`SnapshotChanges`], which only knows the slots the walk has reached: when no
/// read meets a foreign write here, none can there, at any point of the walk.
pub struct RoundWrites {
    cells: FxHashMap<Cell, usize>,
    species: FxHashMap<Species, usize>,
    writers: usize,
    only_writer: usize,
    reach: f64,
    grid_cells: usize,
}
impl RoundWrites {
    pub fn new(grid: &SeenGrid, cells: usize) -> Self {
        Self {
            cells: FxHashMap::with_capacity_and_hasher(cells, Default::default()),
            species: Default::default(),
            writers: 0,
            only_writer: 0,
            reach: grid.reach(),
            grid_cells: grid.cell_count(),
        }
    }
    pub fn add(&mut self, owner: usize, writes: &OwnerWrites) {
        self.writers += 1;
        self.only_writer = owner;
        self.reach = self.reach.max(writes.reach);
        // An upper bound: two owners may add the same cell. Too many cells can
        // only make a read look changed.
        self.grid_cells += writes.new_cells;
        for &cell in &writes.cells {
            let entry = self.cells.entry(cell).or_insert(owner);
            if *entry != owner {
                *entry = MANY;
            }
        }
        for &species in &writes.species {
            let entry = self.species.entry(species).or_insert(owner);
            if *entry != owner {
                *entry = MANY;
            }
        }
    }
    /// Whether anyone but `owner` writes.
    fn foreign_writer(&self, owner: usize) -> bool {
        self.writers > 1 || (self.writers == 1 && self.only_writer != owner)
    }
}
impl ReadSet {
    /// Whether this owner's read meets nothing another owner's prediction
    /// writes, with every slot of `owners` written.
    ///
    /// A query's shape only grows with the grid's reach and cell count, and the
    /// walk's grid lies between the prediction's and the round's final one, so a
    /// shape that also holds on the final grid holds throughout the walk.
    pub fn untouched_by_others(
        &self,
        owner: usize,
        owners: &[usize],
        writes: &RoundWrites,
    ) -> bool {
        if self.all && writes.foreign_writer(owner) {
            return false;
        }
        if self
            .species
            .iter()
            .any(|species| writes.species.get(species).is_some_and(|&o| o != owner))
        {
            return false;
        }
        if self.slots.iter().any(|&slot| owners[slot] != owner) {
            return false;
        }
        self.queries.iter().all(|&(area, expected, spatial)| {
            if query_columns(area, spatial, writes.reach, writes.grid_cells) != expected {
                return false;
            }
            let Some(columns) = expected else {
                return true;
            };
            if columns.count() <= writes.cells.len() as u64 {
                !columns
                    .cells()
                    .any(|cell| writes.cells.get(&cell).is_some_and(|&o| o != owner))
            } else {
                !writes
                    .cells
                    .iter()
                    .any(|(&cell, &o)| o != owner && columns.contains(cell))
            }
        })
    }
}

/// Mutations already published in the original global entity order. Owner
/// identity is frozen for this tick, even when an entity migrates spatially.
#[derive(Clone)]
pub struct SnapshotChanges {
    owners: Vec<usize>,
    slots: Vec<bool>,
    cells: FxHashMap<Cell, Vec<usize>>,
    owners_ticked: FxHashSet<usize>,
    species: FxHashMap<Species, Vec<usize>>,
}
impl SnapshotChanges {
    pub fn new(owners: Vec<usize>) -> Self {
        Self {
            slots: vec![false; owners.len()],
            owners,
            cells: Default::default(),
            owners_ticked: Default::default(),
            species: Default::default(),
        }
    }
    pub fn touch(&mut self, slot: usize, before: Aabb, after: Aabb) {
        let owner = self.owners[slot];
        self.slots[slot] = true;
        self.owners_ticked.insert(owner);
        for cell in [corner(before), corner(after)] {
            let owners = self.cells.entry(cell).or_default();
            if !owners.contains(&owner) {
                owners.push(owner);
            }
        }
    }
    pub fn species_changed(&mut self, slot: usize, before: Species, after: Species) {
        let owner = self.owners[slot];
        for species in [before, after] {
            let owners = self.species.entry(species).or_default();
            if !owners.contains(&owner) {
                owners.push(owner);
            }
        }
    }
    pub fn valid(&self, reads: &ReadSet, owner: usize, grid: &SeenGrid) -> bool {
        self.conflict(reads, owner, grid).is_none()
    }
    pub fn conflict(&self, reads: &ReadSet, owner: usize, grid: &SeenGrid) -> Option<ReadConflict> {
        if reads.species.iter().any(|species| {
            self.species
                .get(species)
                .is_some_and(|owners| owners.iter().any(|&other| other != owner))
        }) {
            return Some(ReadConflict::Species);
        }
        if reads.all && self.owners_ticked.iter().any(|&other| other != owner) {
            return Some(ReadConflict::FullScan);
        }
        if reads
            .slots
            .iter()
            .any(|&slot| self.slots[slot] && self.owners[slot] != owner)
        {
            return Some(ReadConflict::EntitySlot);
        }
        for &(area, expected_columns, spatial) in &reads.queries {
            // Growing boxes and retained/new grid cells can change both the
            // candidate range and full-scan fallback. Compare the real shape.
            let current = if spatial {
                grid.spatial_columns(area)
            } else {
                grid.columns(area)
            };
            if current != expected_columns {
                return Some(ReadConflict::GridShape);
            }
            if let Some(columns) = expected_columns {
                let foreign = |owners: &Vec<usize>| owners.iter().any(|&other| other != owner);
                if columns.count() <= self.cells.len() as u64 {
                    for cell in columns.cells() {
                        if self.cells.get(&cell).is_some_and(foreign) {
                            return Some(ReadConflict::SpatialCell);
                        }
                    }
                } else if self
                    .cells
                    .iter()
                    .any(|(&cell, owners)| columns.contains(cell) && foreign(owners))
                {
                    return Some(ReadConflict::SpatialCell);
                }
            }
        }
        None
    }
}

/// Ordered membership of the tick's snapshot, built the first time a mob asks:
/// most ticks nobody does. A mob's species doesn't change, so its slot's doesn't
/// during the tick either, and how far the walk has got doesn't show in the
/// result. It lends only slot indexes; changed same-region membership is
/// supplied by the overlay.
#[derive(Default)]
pub struct SpeciesIndex {
    slots: OnceLock<FxHashMap<Species, Vec<usize>>>,
}
impl SpeciesIndex {
    /// The slots of a species, ascending.
    fn of(&self, snapshot: &[Seen], species: Species) -> &[usize] {
        let slots = self.slots.get_or_init(|| {
            let mut slots: FxHashMap<Species, Vec<usize>> = Default::default();
            for (slot, seen) in snapshot.iter().enumerate() {
                slots.entry(seen.species).or_default().push(slot);
            }
            slots
        });
        slots.get(&species).map_or(&[], Vec::as_slice)
    }
}

/// Only one region's changed entries, over the tick's borrowed base snapshot.
#[derive(Default)]
pub struct SnapshotOverlay {
    entries: FxHashMap<usize, Seen>,
}
impl SnapshotOverlay {
    pub fn update(&mut self, slot: usize, seen: Seen) {
        self.entries.insert(slot, seen);
    }
}

/// Prevents unobserved raw-slice scans in mob implementations. Point lookups
/// record their slot; iteration records a conservative complete-snapshot read.
/// Spatial queries use get_untracked only after recording their queried cells.
#[derive(Clone, Copy)]
pub struct SeenView<'a> {
    base: &'a [Seen],
    overlay: Option<&'a SnapshotOverlay>,
    species: Option<&'a SpeciesIndex>,
    reads: Option<&'a RefCell<ReadSet>>,
}
impl<'a> SeenView<'a> {
    pub fn observed(
        base: &'a [Seen],
        overlay: &'a SnapshotOverlay,
        reads: &'a RefCell<ReadSet>,
    ) -> Self {
        Self {
            base,
            overlay: Some(overlay),
            species: None,
            reads: Some(reads),
        }
    }
    pub fn with_species(mut self, species: &'a SpeciesIndex) -> Self {
        self.species = Some(species);
        self
    }
    /// Observe one species, including empty membership and same-tick changes.
    /// Global insertion order is preserved after additions or removals.
    pub fn of_species(self, species: Species) -> impl Iterator<Item = &'a Seen> {
        if let Some(reads) = self.reads {
            reads.borrow_mut().species.insert(species);
        }
        // Both ascending; at most one of them holds anything. A slot that left
        // the species is filtered out below, so only one that entered it makes
        // a list of its own.
        let (mut shared, mut owned): (&'a [usize], Vec<usize>) = match self.species {
            Some(index) => (index.of(self.base, species), Vec::new()),
            None => (
                &[],
                self.base
                    .iter()
                    .enumerate()
                    .filter(|(_, seen)| seen.species == species)
                    .map(|(slot, _)| slot)
                    .collect(),
            ),
        };
        if let Some(overlay) = self.overlay {
            let mut entered = overlay
                .entries
                .iter()
                .filter(|(slot, seen)| {
                    seen.species == species && self.base[**slot].species != species
                })
                .map(|(&slot, _)| slot)
                .peekable();
            if entered.peek().is_some() {
                owned.extend_from_slice(shared);
                shared = &[];
                owned.extend(entered);
                owned.sort_unstable();
            }
        }
        shared.iter().copied().chain(owned).filter_map(move |slot| {
            self.get_untracked(slot)
                .filter(|seen| seen.species == species)
        })
    }
    pub fn len(self) -> usize {
        self.base.len()
    }
    pub fn is_empty(self) -> bool {
        self.base.is_empty()
    }
    pub fn get(self, slot: usize) -> Option<&'a Seen> {
        let value = self.get_untracked(slot);
        if value.is_some()
            && let Some(reads) = self.reads
        {
            reads.borrow_mut().slots.insert(slot);
        }
        value
    }
    pub(super) fn get_untracked(self, slot: usize) -> Option<&'a Seen> {
        self.overlay
            .and_then(|o| o.entries.get(&slot))
            .or_else(|| self.base.get(slot))
    }
    pub(super) fn record_query(self, area: Aabb, grid: &dyn SnapshotGrid, spatial: bool) {
        if let Some(reads) = self.reads {
            let columns = if spatial {
                grid.spatial_columns(area)
            } else {
                grid.columns(area)
            };
            reads.borrow_mut().queries.push((area, columns, spatial));
        }
    }
    pub fn iter(self) -> SeenIter<'a> {
        if let Some(reads) = self.reads {
            reads.borrow_mut().all = true;
        }
        SeenIter {
            view: self,
            slot: 0,
        }
    }
}
impl<'a> From<&'a [Seen]> for SeenView<'a> {
    fn from(base: &'a [Seen]) -> Self {
        Self {
            base,
            overlay: None,
            species: None,
            reads: None,
        }
    }
}
impl<'a> From<&'a Vec<Seen>> for SeenView<'a> {
    fn from(base: &'a Vec<Seen>) -> Self {
        base.as_slice().into()
    }
}
impl<'a, const N: usize> From<&'a [Seen; N]> for SeenView<'a> {
    fn from(base: &'a [Seen; N]) -> Self {
        base.as_slice().into()
    }
}
impl Index<usize> for SeenView<'_> {
    type Output = Seen;
    fn index(&self, slot: usize) -> &Seen {
        self.get(slot).expect("snapshot slot")
    }
}
pub struct SeenIter<'a> {
    view: SeenView<'a>,
    slot: usize,
}
impl<'a> Iterator for SeenIter<'a> {
    type Item = &'a Seen;
    fn next(&mut self) -> Option<Self::Item> {
        let value = self.view.get_untracked(self.slot);
        self.slot += usize::from(value.is_some());
        value
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.view.len() - self.slot;
        (left, Some(left))
    }
}
impl ExactSizeIterator for SeenIter<'_> {}
impl<'a> IntoIterator for SeenView<'a> {
    type Item = &'a Seen;
    type IntoIter = SeenIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<'a> IntoIterator for &SeenView<'a> {
    type Item = &'a Seen;
    type IntoIter = SeenIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::Vec3;
    fn bounds(x: f64, width: f64) -> Aabb {
        Aabb::new(Vec3::new(x, 0.0, 0.0), Vec3::new(x + width, 1.8, width))
    }
    fn grid() -> SeenGrid {
        SeenGrid::new((0..128).map(|i| bounds(f64::from(i) * 8.0, 0.6)))
    }

    #[test]
    fn entering_a_previously_empty_query_cell_invalidates_a_prediction() {
        let mut grid = grid();
        let mut changes = SnapshotChanges::new((0..128).collect());
        let area = bounds(32.0, 1.0);
        let mut reads = ReadSet::default();
        reads.query(area, grid.columns(area));
        assert!(changes.valid(&reads, 0, &grid));
        // An unrelated distant tick has no effect on this query.
        changes.touch(100, bounds(800.0, 0.6), bounds(800.1, 0.6));
        grid.update(100, bounds(800.1, 0.6));
        assert!(changes.valid(&reads, 0, &grid));
        // This is an actual destination, not a maximum-speed halo.
        changes.touch(101, bounds(808.0, 0.6), bounds(32.2, 0.6));
        grid.update(101, bounds(32.2, 0.6));
        assert!(!changes.valid(&reads, 0, &grid));
    }

    #[test]
    fn sparse_spatial_reads_keep_negative_space_without_observing_distant_entities() {
        let mut grid = grid();
        let mut changes = SnapshotChanges::new((0..128).collect());
        let area = bounds(32.0, 64.0);
        assert!(grid.columns(area).is_none());
        let mut reads = ReadSet::default();
        reads.queries.push((area, grid.spatial_columns(area), true));
        changes.touch(100, bounds(800.0, 0.6), bounds(800.1, 0.6));
        grid.update(100, bounds(800.1, 0.6));
        assert!(changes.valid(&reads, 0, &grid));
        changes.touch(101, bounds(808.0, 0.6), bounds(32.2, 0.6));
        grid.update(101, bounds(32.2, 0.6));
        assert!(!changes.valid(&reads, 0, &grid));
    }

    #[test]
    fn species_reads_track_changes_and_new_members_without_observing_other_species() {
        let grid = grid();
        let mut changes = SnapshotChanges::new((0..128).collect());
        let mut reads = ReadSet::default();
        reads.species.insert(Species::Villager);
        changes.species_changed(100, Species::Zombie, Species::Zombie);
        assert!(changes.valid(&reads, 0, &grid));
        changes.species_changed(0, Species::Zombie, Species::Villager);
        assert!(
            changes.valid(&reads, 0, &grid),
            "own membership is supplied by the overlay"
        );
        changes.species_changed(101, Species::Zombie, Species::Villager);
        assert!(
            !changes.valid(&reads, 0, &grid),
            "a foreign new member changes an empty query"
        );
        let mut existing = SnapshotChanges::new((0..128).collect());
        existing.species_changed(100, Species::Villager, Species::Villager);
        assert!(
            !existing.valid(&reads, 0, &grid),
            "existing villagers' POI state also matters"
        );
    }

    #[test]
    fn species_overlay_preserves_order_when_entities_enter_and_leave_the_query() {
        use super::super::{Mob, sensor_scan_tests::kind};
        let mut base: Vec<_> = (1..=3)
            .map(|id| Mob::new(id, kind(), horizon_core::geometry::Vec3::ZERO, 0.0).seen(None))
            .collect();
        base[1].species = Species::Villager;
        let index = SpeciesIndex::default();
        let mut overlay = SnapshotOverlay::default();
        let mut entered = base[0].clone();
        entered.species = Species::Villager;
        overlay.update(0, entered);
        let mut left = base[1].clone();
        left.species = Species::Cow;
        overlay.update(1, left);
        let reads = RefCell::new(ReadSet::default());
        let ids = |overlay: &SnapshotOverlay| {
            SeenView::observed(&base, overlay, &reads)
                .with_species(&index)
                .of_species(Species::Villager)
                .map(|seen| seen.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&overlay), [1]);
        let mut entered = base[2].clone();
        entered.species = Species::Villager;
        overlay.update(2, entered);
        assert_eq!(ids(&overlay), [1, 3]);
        assert!(!reads.borrow().all);
        assert!(reads.borrow().species.contains(&Species::Villager));
    }

    #[test]
    fn species_lists_keep_slot_order_on_every_view() {
        use super::super::{Mob, sensor_scan_tests::kind};
        let mut base: Vec<_> = (1..=5)
            .map(|id| Mob::new(id, kind(), horizon_core::geometry::Vec3::ZERO, 0.0).seen(None))
            .collect();
        for slot in [1, 2, 4] {
            base[slot].species = Species::Villager;
        }
        let ids = |view: SeenView<'_>| {
            view.of_species(Species::Villager)
                .map(|seen| seen.id)
                .collect::<Vec<_>>()
        };
        let index = SpeciesIndex::default();
        let walk = SeenView::from(&base).with_species(&index);
        assert_eq!(ids(walk), [2, 3, 5]);
        assert_eq!(ids(SeenView::from(&base)), [2, 3, 5], "without an index");

        // As the walk replaces entries: the live one answers, in the same place.
        let mut live = base.clone();
        live[2].alive = false;
        let view = SeenView::from(&live).with_species(&index);
        let dead: Vec<_> = view
            .of_species(Species::Villager)
            .map(|seen| (seen.id, seen.alive))
            .collect();
        assert_eq!(dead, [(2, true), (3, false), (5, true)]);

        // An index first asked once the walk is under way gives the same list.
        let late = SpeciesIndex::default();
        assert_eq!(ids(SeenView::from(&live).with_species(&late)), [2, 3, 5]);

        // Someone leaving is no reason to copy, and comes out all the same.
        let reads = RefCell::new(ReadSet::default());
        let mut overlay = SnapshotOverlay::default();
        let observed = |overlay: &SnapshotOverlay| {
            ids(SeenView::observed(&base, overlay, &reads).with_species(&index))
        };
        assert_eq!(observed(&overlay), [2, 3, 5]);
        let mut left = base[2].clone();
        left.species = Species::Cow;
        overlay.update(2, left);
        assert_eq!(observed(&overlay), [2, 5]);
    }

    #[test]
    fn point_and_unbounded_reads_detect_distant_changes() {
        let grid = grid();
        let mut changes = SnapshotChanges::new((0..128).collect());
        let mut point = ReadSet::default();
        point.slots.insert(100);
        let all = ReadSet {
            all: true,
            ..Default::default()
        };
        changes.touch(0, bounds(0.0, 0.6), bounds(0.1, 0.6));
        // Own region prefixes are validated by the ordered transaction commit.
        assert!(changes.valid(&point, 0, &grid));
        assert!(changes.valid(&all, 0, &grid));
        changes.touch(100, bounds(800.0, 0.6), bounds(800.1, 0.6));
        assert!(!changes.valid(&point, 0, &grid));
        assert!(!changes.valid(&all, 0, &grid));
    }

    #[test]
    fn a_growing_foreign_box_changes_the_real_query_shape() {
        let mut grid = grid();
        let changes = SnapshotChanges::new((0..128).collect());
        let area = bounds(32.0, 1.0);
        let mut reads = ReadSet::default();
        reads.query(area, grid.columns(area));
        // Even if the changed body's old corner is distant, its larger width
        // changes the grid's conservative candidate range globally.
        grid.update(100, bounds(800.0, 20.0));
        assert!(!changes.valid(&reads, 0, &grid));
    }
}
