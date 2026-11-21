//! Default fastutil object map with live indexed entries and native traversal.
use crate::object_open_set::{CursorError, ObjectKey};
use std::{cell::Cell, rc::Rc};
#[derive(Clone)]
struct Table<K, V> {
    keys: Vec<Option<K>>,
    values: Vec<Option<V>>,
}
impl<K, V> Table<K, V> {
    fn new(n: usize) -> Self {
        Self {
            keys: std::iter::repeat_with(|| None).take(n + 1).collect(),
            values: std::iter::repeat_with(|| None).take(n + 1).collect(),
        }
    }
}
/// An entry retains its array index, not a copy of its key/value. Clones share
/// invalidation when its entry iterator removes that entry.
#[derive(Clone)]
pub struct Entry(Rc<Cell<i32>>);
pub struct Cursor<K> {
    position: i32,
    last: i32,
    remaining: i32,
    null: bool,
    wrapped: Option<Vec<Option<K>>>,
    entry: Option<Entry>,
}
impl<K> Cursor<K> {
    pub fn has_next(&self) -> bool {
        self.remaining != 0
    }
}
#[derive(Debug)]
pub enum TraversalError<E> {
    Cursor(CursorError),
    Callback(E),
}
/// The map owns its table by value, so a lookup is a plain array read and the map is `Send` whenever its keys
/// and values are: a region's thread takes it with the entity that owns it. No lock, no atomic, no sharing.
///
/// A method takes `&self` or `&mut self` and never calls back into the map while it reads the table, so there
/// is no nested borrow to get wrong (the table was a mutex for a while, where one deadlocked).
pub struct ObjectOpenMap<K, V> {
    table: Table<K, V>,
    size: i32,
    null: bool,
    /// One slot per [`Self::for_each`] running: the table as it stood when a callback first rehashed the map
    /// (the traversal keeps reading its keys from it, as fastutil's iterator holds the old array), `None` until then.
    traversals: Vec<Option<Table<K, V>>>,
}
/// A copy with its own table: the copy's changes never reach the original.
impl<K: Clone, V: Clone> Clone for ObjectOpenMap<K, V> {
    fn clone(&self) -> Self {
        Self {
            table: self.table.clone(),
            size: self.size,
            null: self.null,
            traversals: Vec::new(),
        }
    }
}
impl<K, V> Default for ObjectOpenMap<K, V> {
    fn default() -> Self {
        Self {
            table: Table::new(32),
            size: 0,
            null: false,
            traversals: Vec::new(),
        }
    }
}
fn mixed(hash: i32) -> usize {
    let h = (hash as u32).wrapping_mul(0x9e3779b9);
    (h ^ (h >> 16)) as usize
}
impl<K: ObjectKey + Clone, V: Clone> ObjectOpenMap<K, V> {
    pub fn len(&self) -> i32 {
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    fn n(&self) -> usize {
        self.table.keys.len() - 1
    }
    fn max_fill(&self) -> i32 {
        (self.n() * 3 / 4) as i32
    }
    fn find(&self, key: &K) -> usize {
        let table = &self.table;
        let mask = table.keys.len() - 2;
        let mut p = mixed(key.object_hash()) & mask;
        while table.keys[p].as_ref().is_some_and(|k| k != key) {
            p = (p + 1) & mask;
        }
        p
    }
    pub fn contains_key(&self, key: Option<&K>) -> bool {
        key.map_or(self.null, |k| {
            let p = self.find(k);
            self.table.keys[p].is_some()
        })
    }
    pub fn get(&self, key: Option<&K>) -> Option<V> {
        let p = key.map_or(self.n(), |k| self.find(k));
        let table = &self.table;
        if key.map_or(self.null, |_| table.keys[p].is_some()) {
            table.values[p].clone()
        } else {
            None
        }
    }
    pub fn entries(&self) -> Vec<(Option<K>, Option<V>)> {
        let table = &self.table;
        let n = table.keys.len() - 1;
        let mut entries = Vec::new();
        if self.null {
            entries.push((None, table.values[n].clone()));
        }
        for p in (0..n).rev() {
            if table.keys[p].is_some() {
                entries.push((table.keys[p].clone(), table.values[p].clone()));
            }
        }
        entries
    }
    pub fn put(&mut self, key: Option<K>, value: Option<V>) -> Option<V> {
        let p = key.as_ref().map_or(self.n(), |k| self.find(k));
        let existing = if key.is_some() {
            self.table.keys[p].is_some()
        } else {
            self.null
        };
        let previous = std::mem::replace(&mut self.table.values[p], value);
        if existing {
            return previous;
        }
        if key.is_none() {
            self.null = true;
        }
        self.table.keys[p] = key;
        let old = self.size;
        self.size = self.size.wrapping_add(1);
        if old >= self.max_fill() {
            self.rehash(
                (((self.size + 1) as f64 / 0.75).ceil() as usize)
                    .max(2)
                    .next_power_of_two(),
            );
        }
        None
    }
    fn rehash(&mut self, n: usize) {
        let old = &self.table;
        let old_n = old.keys.len() - 1;
        let mut table = Table::new(n);
        for p in (0..old_n).rev() {
            if let Some(key) = &old.keys[p] {
                let mut dest = mixed(key.object_hash()) & (n - 1);
                while table.keys[dest].is_some() {
                    dest = (dest + 1) & (n - 1);
                }
                table.keys[dest] = Some(key.clone());
                table.values[dest] = old.values[p].clone();
            }
        }
        table.values[n] = old.values[old_n].clone();
        // A traversal that has not seen a rehash yet keeps the table it started on.
        let old = std::mem::replace(&mut self.table, table);
        for slot in &mut self.traversals {
            if slot.is_none() {
                *slot = Some(old.clone());
            }
        }
    }
    fn shift(
        &mut self,
        mut hole: usize,
        mut cursor: Option<&mut Cursor<K>>,
    ) -> Result<(), CursorError> {
        let table = &mut self.table;
        let mask = table.keys.len() - 2;
        loop {
            let mut p = (hole + 1) & mask;
            loop {
                let Some(key) = &table.keys[p] else {
                    *table
                        .keys
                        .get_mut(hole)
                        .ok_or(CursorError::IndexOutOfBounds)? = None;
                    table.values[hole] = None;
                    return Ok(());
                };
                let home = mixed(key.object_hash()) & mask;
                if if hole <= p {
                    hole >= home || home > p
                } else {
                    hole >= home && home > p
                } {
                    break;
                }
                p = (p + 1) & mask;
            }
            if p < hole
                && let Some(c) = &mut cursor
            {
                c.wrapped
                    .get_or_insert_with(Vec::new)
                    .push(table.keys[p].clone());
            }
            let key = table.keys[p].clone();
            let value = table.values[p].clone();
            *table
                .keys
                .get_mut(hole)
                .ok_or(CursorError::IndexOutOfBounds)? = key;
            table.values[hole] = value;
            hole = p;
        }
    }
    pub fn remove(&mut self, key: Option<&K>) -> Option<V> {
        if !self.contains_key(key) {
            return None;
        }
        let p = key.map_or(self.n(), |k| self.find(k));
        let old = self.table.values[p].clone();
        self.size = self.size.wrapping_sub(1);
        if key.is_none() {
            self.null = false;
            let table = &mut self.table;
            table.keys[p] = None;
            table.values[p] = None;
        } else {
            self.shift(p, None).unwrap();
        }
        if self.n() > 32 && self.size < self.max_fill() / 4 {
            self.rehash(self.n() / 2);
        }
        old
    }
    pub fn clear(&mut self) {
        if self.size == 0 {
            return;
        }
        self.size = 0;
        self.null = false;
        let table = &mut self.table;
        table.keys.fill(None);
        table.values.fill(None);
    }
    pub fn cursor(&self) -> Cursor<K> {
        Cursor {
            position: self.n() as i32,
            last: -1,
            remaining: self.size,
            null: self.null,
            wrapped: None,
            entry: None,
        }
    }
    pub fn next_entry(&self, c: &mut Cursor<K>) -> Result<Entry, CursorError> {
        if !c.has_next() {
            return Err(CursorError::NoSuchElement);
        }
        c.remaining = c.remaining.wrapping_sub(1);
        let index = if c.null {
            c.null = false;
            c.last = self.n() as i32;
            c.last
        } else {
            loop {
                c.position = c.position.wrapping_sub(1);
                if c.position < 0 {
                    c.last = i32::MIN;
                    let key = c
                        .wrapped
                        .as_ref()
                        .ok_or(CursorError::MissingWrapped)?
                        .get(c.position.wrapping_neg().wrapping_sub(1) as usize)
                        .ok_or(CursorError::IndexOutOfBounds)?
                        .as_ref()
                        .ok_or(CursorError::MissingWrapped)?;
                    let p = self.find(key);
                    // Deleting a wrapped member externally can make native iteration
                    // nonterminating. Such invalidated wrapped iterators are excluded.
                    if self.table.keys[p].is_none() {
                        return Err(CursorError::MissingWrapped);
                    }
                    break p as i32;
                }
                if self
                    .table
                    .keys
                    .get(c.position as usize)
                    .ok_or(CursorError::IndexOutOfBounds)?
                    .is_some()
                {
                    c.last = c.position;
                    break c.position;
                }
            }
        };
        let entry = Entry(Rc::new(Cell::new(index)));
        c.entry = Some(entry.clone());
        Ok(entry)
    }
    pub fn entry_key(&self, e: &Entry) -> Result<Option<K>, CursorError> {
        self.table
            .keys
            .get(e.0.get() as usize)
            .cloned()
            .ok_or(CursorError::IndexOutOfBounds)
    }
    pub fn entry_value(&self, e: &Entry) -> Result<Option<V>, CursorError> {
        self.table
            .values
            .get(e.0.get() as usize)
            .cloned()
            .ok_or(CursorError::IndexOutOfBounds)
    }
    pub fn set_entry_value(
        &mut self,
        e: &Entry,
        value: Option<V>,
    ) -> Result<Option<V>, CursorError> {
        Ok(std::mem::replace(
            self.table
                .values
                .get_mut(e.0.get() as usize)
                .ok_or(CursorError::IndexOutOfBounds)?,
            value,
        ))
    }
    pub fn remove_current(&mut self, c: &mut Cursor<K>) -> Result<(), CursorError> {
        if c.last == -1 {
            return Err(CursorError::NoCurrentEntry);
        }
        if c.last == self.n() as i32 {
            self.null = false;
            let t = &mut self.table;
            let n = t.keys.len() - 1;
            t.keys[n] = None;
            t.values[n] = None;
        } else if c.position >= 0 {
            self.shift(c.last as usize, Some(c))?;
        } else {
            let key = c
                .wrapped
                .as_mut()
                .ok_or(CursorError::MissingWrapped)?
                .get_mut(c.position.wrapping_neg().wrapping_sub(1) as usize)
                .ok_or(CursorError::IndexOutOfBounds)?
                .take();
            self.remove(key.as_ref());
            c.last = -1;
            if let Some(e) = &c.entry {
                e.0.set(-1);
            }
            return Ok(());
        }
        self.size = self.size.wrapping_sub(1);
        c.last = -1;
        if let Some(e) = &c.entry {
            e.0.set(-1);
        }
        Ok(())
    }
    pub fn for_each<E>(
        &mut self,
        mut action: impl FnMut(&mut Self, Option<K>, Option<V>) -> Result<(), E>,
    ) -> Result<(), TraversalError<E>> {
        if self.null {
            let n = self.n();
            let value = self.table.values[n].clone();
            action(self, None, value).map_err(TraversalError::Callback)?;
        }
        // Keys are captured after the null callback. Values remain live even if
        // a callback rehashes the map and replaces both native arrays.
        let n = self.n();
        let slot = self.traversals.len();
        self.traversals.push(None);
        let result = self.traverse(slot, n, &mut action);
        self.traversals.truncate(slot);
        result
    }
    fn traverse<E>(
        &mut self,
        slot: usize,
        n: usize,
        action: &mut impl FnMut(&mut Self, Option<K>, Option<V>) -> Result<(), E>,
    ) -> Result<(), TraversalError<E>> {
        for p in (0..n).rev() {
            let key = self.traversals[slot].as_ref().unwrap_or(&self.table).keys[p].clone();
            if key.is_some() {
                let value = self
                    .table
                    .values
                    .get(p)
                    .cloned()
                    .ok_or(TraversalError::Cursor(CursorError::IndexOutOfBounds))?;
                action(self, key, value).map_err(TraversalError::Callback)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq, Eq)]
    struct K(i32);
    impl ObjectKey for K {
        fn object_hash(&self) -> i32 {
            self.0
        }
    }

    #[test]
    fn a_copy_has_its_own_table() {
        let mut map = ObjectOpenMap::default();
        map.put(Some(K(1)), Some("one"));
        let mut copy = map.clone();
        copy.put(Some(K(1)), Some("uno"));
        copy.put(Some(K(2)), Some("dos"));
        assert_eq!(map.get(Some(&K(1))), Some("one"));
        assert!(!map.contains_key(Some(&K(2))));
        assert_eq!((map.len(), copy.len()), (1, 2));
    }

    #[test]
    fn a_traversal_keeps_its_keys_through_a_rehash_in_a_callback() {
        let mut map = ObjectOpenMap::default();
        for i in 0..5 {
            map.put(Some(K(i)), Some(i));
        }
        let mut seen = Vec::new();
        map.for_each(|m, k, _| -> Result<(), ()> {
            seen.push(k.unwrap().0);
            if seen.len() == 1 {
                for i in 100..200 {
                    m.put(Some(K(i)), Some(i));
                }
            }
            Ok(())
        })
        .unwrap();
        // Keys added before the rehash sit in the old table, so the traversal may meet them: it still ends, and
        // meets each of the first five once.
        for i in 0..5 {
            assert_eq!(seen.iter().filter(|&&k| k == i).count(), 1);
        }
        assert_eq!(map.len(), 105);
    }
}
