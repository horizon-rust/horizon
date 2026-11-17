//! The chunks a level queued to its region files, in order, for what keeps
//! far terrain made from saved chunks: the far sampler's decoded chunks
//! (`far_lod`) and the server's far tiles (`lod_net::cache`). Each reader
//! keeps a cursor into the log and drops what the chunks since it made.
use std::{
    collections::VecDeque,
    sync::{Mutex, PoisonError},
};

/// Chunks the log keeps: a reader further behind drops everything.
const KEPT: usize = 1 << 16;

#[derive(Default)]
pub struct FarChanges(Mutex<Log>);

#[derive(Default)]
struct Log {
    /// The position of `chunks[0]` in the log.
    first: u64,
    chunks: VecDeque<(i32, i32)>,
}

/// What changed since a cursor.
#[derive(Debug, PartialEq, Eq)]
pub enum Changed {
    Chunks(Vec<(i32, i32)>),
    /// More than the log keeps: anything may have.
    All,
}

impl Log {
    fn end(&self) -> u64 {
        self.first + self.chunks.len() as u64
    }
}

impl FarChanges {
    fn log(&self) -> std::sync::MutexGuard<'_, Log> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
    pub(super) fn push(&self, chunks: impl IntoIterator<Item = (i32, i32)>) {
        let mut log = self.log();
        log.chunks.extend(chunks);
        while log.chunks.len() > KEPT {
            log.chunks.pop_front();
            log.first += 1;
        }
    }
    /// A cursor past every change so far.
    pub fn end(&self) -> u64 {
        self.log().end()
    }
    /// The chunks changed since `cursor`, which moves past them.
    pub fn since(&self, cursor: &mut u64) -> Changed {
        let log = self.log();
        let from = *cursor;
        *cursor = log.end();
        if from < log.first {
            return Changed::All;
        }
        Changed::Chunks(
            log.chunks
                .range((from - log.first) as usize..)
                .copied()
                .collect(),
        )
    }
    /// Whether `chunk` changed since `cursor`.
    pub fn changed_since(&self, cursor: u64, chunk: (i32, i32)) -> bool {
        self.changed_in(cursor, chunk, chunk)
    }
    /// Whether a chunk from `min` to `max` (both included) changed since
    /// `cursor`.
    pub fn changed_in(&self, cursor: u64, min: (i32, i32), max: (i32, i32)) -> bool {
        let log = self.log();
        cursor < log.first
            || (log.chunks.range((cursor - log.first) as usize..))
                .any(|c| (min.0..=max.0).contains(&c.0) && (min.1..=max.1).contains(&c.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_reader_hears_what_changed_since_it_last_asked() {
        let log = FarChanges::default();
        log.push([(0, 0), (1, 2)]);
        let (mut a, mut b) = (0, log.end());
        log.push([(-3, 4)]);
        assert_eq!(
            log.since(&mut a),
            Changed::Chunks(vec![(0, 0), (1, 2), (-3, 4)])
        );
        assert_eq!(log.since(&mut b), Changed::Chunks(vec![(-3, 4)]));
        assert_eq!(log.since(&mut b), Changed::Chunks(Vec::new()));
        assert!(log.changed_since(1, (-3, 4)) && !log.changed_since(1, (0, 0)));
        // Past what the log keeps, a reader that fell behind hears everything changed.
        log.push((0..=KEPT as i32).map(|i| (i, 0)));
        assert_eq!(log.since(&mut a), Changed::All);
        assert!(log.changed_since(0, (99, 99)));
        assert_eq!(log.since(&mut a), Changed::Chunks(Vec::new()));
    }
}
