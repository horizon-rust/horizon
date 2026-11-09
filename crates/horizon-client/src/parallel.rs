//! The helper threads of the server tick's parallel phases (`HORIZON_PARALLEL_TICK`): one pool,
//! made once, instead of threads started for every phase of every tick. A phase hands it work that
//! gives the same result in whatever order the threads take it (each part's results stay in order).
//! The mob loop's groups don't run here: each must have a thread of its own while it waits for its
//! turn of the level random (`game/mob_islands.rs`).
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

/// The pool: one thread fewer than a phase's lanes (the server thread takes parts too).
pub fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(lanes().saturating_sub(1).max(1))
            .thread_name(|i| format!("tick-helper-{i}"))
            .build()
            .expect("the tick's helper threads start")
    })
}

/// How many threads a phase may share its work among.
pub fn lanes() -> usize {
    horizon_threads::worker_count().clamp(1, 8)
}

/// `f` over `items` in parts on the pool (or all on this thread for fewer than `min`): every part's
/// results, in the order of `items`, as one thread would give them.
///
/// The parts are small (four a thread) and go to whichever thread is free, this one too: a helper
/// the machine is slow to run (the chunk workers compete for the same cores) holds the phase up by
/// one part at most instead of an eighth of the work, and the server thread works instead of only
/// waiting.
pub fn par_map<T: Sync, R: Send>(
    items: &[T],
    min: usize,
    f: impl Fn(&[T], &mut Vec<R>) + Sync,
) -> Vec<R> {
    let lanes = lanes();
    if lanes <= 1 || items.len() < min {
        let mut out = Vec::with_capacity(items.len());
        f(items, &mut out);
        return out;
    }
    let size = items.len().div_ceil(lanes * 4);
    let count = items.len().div_ceil(size);
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Vec<R>>> = (0..count).map(|_| Mutex::new(Vec::new())).collect();
    let work = || {
        loop {
            let k = next.fetch_add(1, Ordering::Relaxed);
            if k >= count {
                break;
            }
            let part = &items[k * size..((k + 1) * size).min(items.len())];
            let mut done = Vec::with_capacity(part.len());
            f(part, &mut done);
            *slots[k].lock().unwrap_or_else(PoisonError::into_inner) = done;
        }
    };
    pool().in_place_scope(|s| {
        for _ in 1..lanes {
            s.spawn(|_| work());
        }
        work();
    });
    let mut out = Vec::with_capacity(items.len());
    for slot in slots {
        out.extend(slot.into_inner().unwrap_or_else(PoisonError::into_inner));
    }
    out
}
