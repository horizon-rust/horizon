//! Iteration order of the Java hash collections vanilla keeps keys in:
//! fastutil's open hash maps (open addressing with linear probing,
//! backward-shift deletion and rehashing at the load factor), such as the
//! scoreboard's `Object2ObjectOpenHashMap<String, _>` and the natural spawn
//! counter's `Long2ByteOpenHashMap`, and `java.util.HashSet<String>` (chained
//! buckets split on resize). Commands list, `scoreboard.dat` saves, and
//! natural spawning walks chunks, in these orders.
use std::borrow::Borrow;

/// A key as a fastutil open hash map places it: `HashCommon.mix` of its hash,
/// and whether it is a primitive map's null key (0), kept out of the table.
pub trait OpenKey: PartialEq {
    fn mix(&self) -> usize;
    fn is_null(&self) -> bool {
        false
    }
}
impl OpenKey for str {
    fn mix(&self) -> usize {
        mix(java_hash(self))
    }
}
impl OpenKey for String {
    fn mix(&self) -> usize {
        mix(java_hash(self))
    }
}
impl OpenKey for i64 {
    fn mix(&self) -> usize {
        let h = (*self as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let h = h ^ (h >> 32);
        (h ^ (h >> 16)) as u32 as usize
    }
    fn is_null(&self) -> bool {
        *self == 0
    }
}

/// The keys of a fastutil open hash map in their table slots.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenOrder<K = String> {
    keys: Vec<Option<K>>,
    /// The null key (`containsNullKey`): first in `iter`, last in `stream`.
    null: Option<K>,
    size: usize,
    min_n: usize,
    load: f32,
}
impl<K: OpenKey + Clone> Default for OpenOrder<K> {
    /// `new Object2ObjectOpenHashMap<>()`: 16 expected entries, load 0.75.
    fn default() -> Self {
        Self::new(16, 0.75)
    }
}
/// `String.hashCode`.
pub fn java_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}
/// `HashCommon.mix`.
fn mix(x: i32) -> usize {
    let h = (x as u32).wrapping_mul(0x9E37_79B9);
    (h ^ (h >> 16)) as usize
}
/// `HashCommon.arraySize`.
fn array_size(expected: usize, f: f32) -> usize {
    let wanted = (expected as f64 / f64::from(f)).ceil() as usize;
    wanted.max(2).next_power_of_two()
}
/// `HashCommon.maxFill`.
fn max_fill(n: usize, f: f32) -> usize {
    ((n as f64 * f64::from(f)).ceil() as usize).min(n - 1)
}
impl<K: OpenKey + Clone> OpenOrder<K> {
    pub fn new(expected: usize, load: f32) -> Self {
        let n = array_size(expected, load);
        Self {
            keys: vec![None; n],
            null: None,
            size: 0,
            min_n: n,
            load,
        }
    }
    pub fn len(&self) -> usize {
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    fn slot<Q: ?Sized + OpenKey>(&self, key: &Q) -> usize {
        key.mix() & (self.keys.len() - 1)
    }
    pub fn contains<Q: ?Sized + OpenKey>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        if key.is_null() {
            return self.null.is_some();
        }
        let mask = self.keys.len() - 1;
        let mut pos = self.slot(key);
        while let Some(k) = &self.keys[pos] {
            if k.borrow() == key {
                return true;
            }
            pos = (pos + 1) & mask;
        }
        false
    }
    /// `put`: false when the key was already there.
    pub fn insert<Q: ?Sized + OpenKey + ToOwned<Owned = K>>(&mut self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        let mask = self.keys.len() - 1;
        let mut pos = self.slot(key);
        let slot = if key.is_null() {
            &mut self.null
        } else {
            while let Some(k) = &self.keys[pos] {
                if k.borrow() == key {
                    return false;
                }
                pos = (pos + 1) & mask;
            }
            &mut self.keys[pos]
        };
        if slot.replace(key.to_owned()).is_some() {
            return false;
        }
        // `if (size++ >= maxFill) rehash(arraySize(size + 1, f))`.
        let old = self.size;
        self.size += 1;
        if old >= max_fill(self.keys.len(), self.load) {
            self.rehash(array_size(self.size + 1, self.load));
        }
        true
    }
    /// `remove`: false when the key was not there.
    pub fn remove<Q: ?Sized + OpenKey>(&mut self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        let mask = self.keys.len() - 1;
        let mut pos = self.slot(key);
        if key.is_null() {
            if self.null.take().is_none() {
                return false;
            }
        } else {
            loop {
                match &self.keys[pos] {
                    None => return false,
                    Some(k) if k.borrow() == key => break,
                    Some(_) => pos = (pos + 1) & mask,
                }
            }
            self.shift_keys(pos);
        }
        self.size -= 1;
        let n = self.keys.len();
        if n > self.min_n && self.size < max_fill(n, self.load) / 4 && n > 16 {
            self.rehash(n / 2);
        }
        true
    }
    fn shift_keys(&mut self, mut pos: usize) {
        let mask = self.keys.len() - 1;
        loop {
            let last = pos;
            pos = (last + 1) & mask;
            loop {
                let Some(curr) = self.keys[pos].clone() else {
                    self.keys[last] = None;
                    return;
                };
                let slot = self.slot(&curr);
                let stop = if last <= pos {
                    last >= slot || slot > pos
                } else {
                    last >= slot && slot > pos
                };
                if stop {
                    self.keys[last] = Some(curr);
                    break;
                }
                pos = (pos + 1) & mask;
            }
        }
    }
    fn rehash(&mut self, new_n: usize) {
        let old = std::mem::replace(&mut self.keys, vec![None; new_n]);
        let mask = new_n - 1;
        // `for (int j = realSize(); j-- != 0;) { while (key[--i] == null); ... }`
        for key in old.into_iter().rev().flatten() {
            let mut pos = key.mix() & mask;
            while self.keys[pos].is_some() {
                pos = (pos + 1) & mask;
            }
            self.keys[pos] = Some(key);
        }
    }
    /// `iterator()` and `forEach`: from the last slot down.
    pub fn iter(&self) -> impl Iterator<Item = &K> {
        self.null.iter().chain(self.keys.iter().rev().flatten())
    }
    /// `spliterator()` (what `stream()` uses): from the first slot up.
    pub fn stream(&self) -> impl Iterator<Item = &K> {
        self.keys.iter().flatten().chain(&self.null)
    }
}

/// The keys of a `java.util.HashSet<String>` in their buckets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HashSetOrder {
    /// Empty until the first insertion (`HashMap.resize` on first `put`).
    table: Vec<Vec<String>>,
    size: usize,
}
/// `HashMap.hash`.
fn spread(s: &str) -> usize {
    let h = java_hash(s) as u32;
    (h ^ (h >> 16)) as usize
}
impl HashSetOrder {
    pub fn len(&self) -> usize {
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    pub fn contains(&self, key: &str) -> bool {
        if self.table.is_empty() {
            return false;
        }
        let i = spread(key) & (self.table.len() - 1);
        self.table[i].iter().any(|k| k == key)
    }
    pub fn insert(&mut self, key: &str) -> bool {
        if self.table.is_empty() {
            self.table = vec![Vec::new(); 16];
        }
        let i = spread(key) & (self.table.len() - 1);
        if self.table[i].iter().any(|k| k == key) {
            return false;
        }
        self.table[i].push(key.to_owned());
        self.size += 1;
        if self.size > self.table.len() * 3 / 4 {
            self.resize();
        }
        true
    }
    /// Doubles the table; each bucket splits into its low and high halves in order.
    fn resize(&mut self) {
        let old = std::mem::take(&mut self.table);
        let n = old.len() * 2;
        self.table = vec![Vec::new(); n];
        for bucket in old {
            for key in bucket {
                let i = spread(&key) & (n - 1);
                self.table[i].push(key);
            }
        }
    }
    pub fn remove(&mut self, key: &str) -> bool {
        if self.table.is_empty() {
            return false;
        }
        let i = spread(key) & (self.table.len() - 1);
        let Some(at) = self.table[i].iter().position(|k| k == key) else {
            return false;
        };
        self.table[i].remove(at);
        self.size -= 1;
        true
    }
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.table.iter().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_order() {
        assert_eq!(java_hash("harness"), {
            "harness"
                .chars()
                .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
        });
        let mut o = OpenOrder::new(16, 0.5);
        for k in ["harness", "harness_h"] {
            o.insert(k);
        }
        assert_eq!(o.iter().count(), 2);
        let mut asc: Vec<&String> = o.stream().collect();
        asc.reverse();
        assert_eq!(asc, o.iter().collect::<Vec<_>>());
        o.remove("harness");
        assert_eq!(o.iter().collect::<Vec<_>>(), vec!["harness_h"]);
        for i in 0..40 {
            o.insert(&format!("k{i}"));
        }
        assert_eq!(o.iter().count(), 41);
        for i in 0..40 {
            o.remove(&format!("k{i}"));
        }
        assert_eq!(o.iter().collect::<Vec<_>>(), vec!["harness_h"]);
    }

    #[test]
    fn hash_set_buckets() {
        let mut s = HashSetOrder::default();
        for i in 0..30 {
            assert!(s.insert(&format!("p{i}")));
        }
        assert!(!s.insert("p3"));
        assert_eq!(s.len(), 30);
        assert!(s.remove("p3") && !s.contains("p3"));
        // "Aa" and "BB" share a hash: insertion order within the bucket.
        let mut t = HashSetOrder::default();
        t.insert("BB");
        t.insert("Aa");
        assert_eq!(t.iter().collect::<Vec<_>>(), vec!["BB", "Aa"]);
    }
}
