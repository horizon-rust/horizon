//! Integer part registry, following the public fastutil javadoc-level description
//! of `Int2ObjectOpenHashMap` (https://fastutil.di.unimi.it/docs/).
//! Keys use multiplicative mixing, linear probing, backward-shift deletion, and
//! descending-slot iteration (with the zero key first). No Mojang implementation
//! is used here.
#[derive(Debug)]
pub(super) struct DragonPartsTable {
    slots: Vec<Option<u32>>,
    zero: bool,
    size: usize,
}
impl Default for DragonPartsTable {
    fn default() -> Self {
        Self {
            slots: vec![None; 32],
            zero: false,
            size: 0,
        }
    }
}
impl DragonPartsTable {
    fn home(key: u32, mask: usize) -> usize {
        let h = key.wrapping_mul(0x9e3779b9);
        ((h ^ (h >> 16)) as usize) & mask
    }
    pub(super) fn insert(&mut self, key: u32) {
        if key == 0 {
            if self.zero {
                return;
            }
            self.zero = true;
        } else {
            let mask = self.slots.len() - 1;
            let mut p = Self::home(key, mask);
            while let Some(old) = self.slots[p] {
                if old == key {
                    return;
                }
                p = (p + 1) & mask;
            }
            self.slots[p] = Some(key);
        }
        self.size += 1;
        if self.size >= self.slots.len() * 3 / 4 {
            let capacity = self.slots.len() * 2;
            let old = std::mem::replace(&mut self.slots, vec![None; capacity]);
            for key in old.into_iter().rev().flatten() {
                let mask = self.slots.len() - 1;
                let mut p = Self::home(key, mask);
                while self.slots[p].is_some() {
                    p = (p + 1) & mask;
                }
                self.slots[p] = Some(key);
            }
        }
    }
    pub(super) fn remove(&mut self, key: u32) {
        if key == 0 {
            if self.zero {
                self.zero = false;
                self.size -= 1;
            }
            return;
        }
        let mask = self.slots.len() - 1;
        let mut hole = Self::home(key, mask);
        loop {
            match self.slots[hole] {
                None => return,
                Some(k) if k == key => break,
                _ => hole = (hole + 1) & mask,
            }
        }
        self.size -= 1;
        let mut scan = (hole + 1) & mask;
        while let Some(k) = self.slots[scan] {
            let home = Self::home(k, mask);
            if (scan.wrapping_sub(home) & mask) >= (scan.wrapping_sub(hole) & mask) {
                self.slots[hole] = Some(k);
                hole = scan;
            }
            scan = (scan + 1) & mask;
        }
        self.slots[hole] = None;
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.zero
            .then_some(0)
            .into_iter()
            .chain(self.slots.iter().rev().filter_map(|k| *k))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    // Literal orders were independently calculated from the specified mix/probe
    // algorithm, not obtained from DragonPartsTable. The removal calculation
    // reinserts the affected cluster, independently of our backward-shift code.
    #[test]
    fn one_dragon_descends_slots() {
        let mut t = DragonPartsTable::default();
        for k in 21..=28 {
            t.insert(k);
        }
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            [25, 27, 28, 24, 26, 23, 22, 21]
        );
    }
    #[test]
    fn two_dragons_collide() {
        let mut t = DragonPartsTable::default();
        for k in (21..=28).chain(31..=38) {
            t.insert(k);
        }
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            [
                33, 35, 25, 27, 37, 31, 32, 28, 34, 24, 38, 36, 26, 23, 22, 21
            ]
        );
    }
    #[test]
    fn removal_repairs_probe_chains() {
        let mut t = DragonPartsTable::default();
        for k in (21..=28).chain(31..=38) {
            t.insert(k);
        }
        for k in 21..=28 {
            t.remove(k);
        }
        for k in 41..=48 {
            t.insert(k);
        }
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            [
                33, 35, 37, 47, 48, 31, 32, 34, 46, 38, 36, 45, 44, 42, 41, 43
            ]
        );
    }
    #[test]
    fn growth_rehashes_in_descending_order() {
        let mut t = DragonPartsTable::default();
        for k in (21..=28).chain(31..=38).chain(41..=48) {
            t.insert(k);
        }
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            [
                33, 25, 37, 28, 24, 26, 23, 22, 21, 35, 27, 47, 48, 31, 32, 34, 36, 38, 46, 41, 45,
                44, 42, 43
            ]
        );
    }
    #[test]
    fn zero_is_first_and_removed() {
        let mut t = DragonPartsTable::default();
        t.insert(21);
        t.insert(0);
        t.insert(0);
        assert_eq!(t.iter().collect::<Vec<_>>(), [0, 21]);
        t.remove(0);
        assert_eq!(t.iter().collect::<Vec<_>>(), [21]);
    }
}

/// Every ender dragon of a level, in join order, with the level's part table.
/// The table travels with the dimension-owned set, including actor loans.
///
/// Until the fight binding lands the methods without an id (`as_ref`, `as_mut`,
/// `extract`, `put_back`, `replace`, `install`, `discard`) address "the dragon",
/// the first one, exactly as the former singleton slot did; `install` and
/// `replace` keep the set at one dragon. Anything keyed by an entity id uses
/// [`Self::get`] / [`Self::get_mut`].
#[derive(Default)]
pub(in crate::game) struct DragonSet {
    list: horizon_client::join_seq::JoinList<crate::game::dragon::Dragon>,
    table: std::cell::RefCell<DragonPartsTable>,
    /// `(join, id)` of each dragon whose eight parts are in the table; a loaned
    /// dragon stays registered.
    registered: std::cell::RefCell<Vec<(horizon_client::join_seq::JoinSeq, u32)>>,
}
impl DragonSet {
    fn unregister(&self, join: horizon_client::join_seq::JoinSeq) {
        let mut registered = self.registered.borrow_mut();
        if let Some(at) = registered.iter().position(|(j, _)| *j == join) {
            let (_, id) = registered.remove(at);
            let mut table = self.table.borrow_mut();
            for part in id + 1..=id + 8 {
                table.remove(part);
            }
        }
    }
    fn register(&self, join: horizon_client::join_seq::JoinSeq, id: u32) {
        if self.registered.borrow().contains(&(join, id)) {
            return;
        }
        self.unregister(join);
        if id != 0 {
            let mut table = self.table.borrow_mut();
            for part in id + 1..=id + 8 {
                table.insert(part);
            }
            self.registered.borrow_mut().push((join, id));
        }
    }
    pub(super) fn parts_table(&self) -> std::cell::Ref<'_, DragonPartsTable> {
        self.table.borrow()
    }
    #[cfg(test)]
    fn part_ids(&self) -> Vec<u32> {
        self.table.borrow().iter().collect()
    }
    /// Bind the set to its dimension; the eight parts are its fixed children.
    pub(in crate::game) fn bind_residency_specialized(
        &mut self,
        journal: &horizon_client::join_seq::residency::Journal,
        owner: horizon_client::join_seq::residency::Owner,
        identity: fn(&crate::game::dragon::Dragon) -> (u32, horizon_client::join_seq::JoinSeq),
        children: (
            horizon_client::join_seq::residency::Kind,
            u8,
            fn(&crate::game::dragon::Dragon, u8) -> Option<[i32; 4]>,
        ),
    ) {
        self.list
            .bind_residency_specialized(journal, owner, identity, children);
    }
    pub(in crate::game) fn iter(&self) -> std::slice::Iter<'_, crate::game::dragon::Dragon> {
        self.list.iter()
    }
    /// The dragon with entity id `id` (never an unassigned one).
    pub(in crate::game) fn get(&self, id: u32) -> Option<&crate::game::dragon::Dragon> {
        self.list.iter().find(|d| d.id != 0 && d.id == id)
    }
    pub(in crate::game) fn get_mut(&mut self, id: u32) -> Option<&mut crate::game::dragon::Dragon> {
        self.list.iter_mut().find(|d| d.id != 0 && d.id == id)
    }
    pub(in crate::game) fn is_some(&self) -> bool {
        !self.list.is_empty()
    }
    pub(in crate::game) fn is_none(&self) -> bool {
        self.list.is_empty()
    }
    /// Add a dragon after the ones already there.
    pub(in crate::game) fn push(&mut self, dragon: crate::game::dragon::Dragon) {
        self.list.push(dragon);
        let last = &self.list[self.list.len() - 1];
        self.register(last.join, last.id);
    }
    pub(in crate::game) fn as_ref(&self) -> Option<&crate::game::dragon::Dragon> {
        self.list.first()
    }
    pub(in crate::game) fn as_mut(&mut self) -> Option<&mut crate::game::dragon::Dragon> {
        self.list.get_mut(0)
    }
    pub(in crate::game) fn locate(&self, join: horizon_client::join_seq::JoinSeq) -> Option<usize> {
        self.list.locate(join)
    }
    /// Replace the set by one dragon, retiring the old ones.
    pub(in crate::game) fn replace(&mut self, dragon: crate::game::dragon::Dragon) {
        self.discard();
        self.push(dragon);
    }
    // Preserve the ownership-returning refusal contract without allocating.
    #[allow(clippy::result_large_err)]
    pub(in crate::game) fn install(
        &mut self,
        dragon: crate::game::dragon::Dragon,
    ) -> Result<(), crate::game::dragon::Dragon> {
        if self.is_some() {
            return Err(dragon);
        }
        self.push(dragon);
        Ok(())
    }
    /// Retire every dragon and forget their parts.
    pub(in crate::game) fn discard(&mut self) {
        let joins: Vec<_> = self.list.iter().map(|d| d.join).collect();
        for join in joins {
            self.unregister(join);
        }
        self.list.clear();
    }
    /// Report the allocator's delayed zero-to-live id assignment.
    pub(in crate::game) fn id_assigned(&self) {
        for (index, dragon) in self.list.iter().enumerate() {
            let old = self
                .registered
                .borrow()
                .iter()
                .find(|(j, _)| *j == dragon.join)
                .map(|e| e.1);
            if old != Some(dragon.id) && !(old.is_none() && dragon.id == 0) {
                self.register(dragon.join, dragon.id);
                self.list.id_assigned(index);
            }
        }
    }
    /// Lift the first dragon without retiring it (its parts stay registered).
    pub(in crate::game) fn extract(&mut self) -> Option<crate::game::dragon::Dragon> {
        let join = self.list.first()?.join;
        self.list.extract(join)
    }
    /// Return an extracted dragon to the front (the tick frame uses it from the next PR on).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::game) fn put_back(&mut self, dragon: crate::game::dragon::Dragon) {
        self.list.put_back(0, dragon);
    }
    pub(in crate::game) fn retire_extracted(
        &self,
        id: u32,
        join: horizon_client::join_seq::JoinSeq,
    ) {
        if self.registered.borrow().contains(&(join, id)) {
            self.unregister(join);
        }
        self.list.retire_extracted(id, join);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::game::Vec3;
    use crate::game::dragon::Dragon;
    #[test]
    fn dragon_set_tracks_registration_not_temporary_loans() {
        let mut slot = DragonSet::default();
        slot.replace(Dragon::new(Vec3::ZERO, 0.0, 0));
        assert!(slot.part_ids().is_empty());
        slot.as_mut().unwrap().id = 20;
        slot.id_assigned();
        assert_eq!(slot.part_ids(), [25, 27, 28, 24, 26, 23, 22, 21]);
        let dragon = slot.extract().unwrap();
        assert_eq!(slot.part_ids().len(), 8);
        slot.put_back(dragon);
        assert_eq!(slot.part_ids(), [25, 27, 28, 24, 26, 23, 22, 21]);
        slot.discard();
        assert!(slot.part_ids().is_empty());
    }
    #[test]
    fn two_dragons_register_their_own_parts_and_retire_independently() {
        let mut set = DragonSet::default();
        for id in [20, 30] {
            let mut d = Dragon::new(Vec3::ZERO, 0.0, 0);
            d.id = id;
            set.push(d);
        }
        assert_eq!(set.iter().count(), 2);
        assert_eq!(set.get(30).map(|d| d.id), Some(30));
        assert!(set.get(0).is_none() && set.get(31).is_none());
        assert_eq!(
            set.part_ids(),
            [
                33, 35, 25, 27, 37, 31, 32, 28, 34, 24, 38, 36, 26, 23, 22, 21
            ]
        );
        let first = set.extract().unwrap();
        assert_eq!(set.as_ref().map(|d| d.id), Some(30));
        set.retire_extracted(first.id, first.join);
        assert_eq!(set.part_ids(), [33, 35, 37, 31, 32, 34, 38, 36]);
    }
}
