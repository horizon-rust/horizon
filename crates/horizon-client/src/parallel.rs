//! The helper threads of the server tick's parallel phases (`HORIZON_PARALLEL_TICK`): one pool,
//! made once, instead of threads started for every phase of every tick. A phase hands it work that
//! gives the same result in whatever order the threads take it (each part's results stay in order).
//! The mob loop's groups don't run here: each must have a thread of its own while it waits for its
//! turn of the level random (`game/mob_islands.rs`).
use rayon::prelude::*;
use std::sync::OnceLock;

/// The pool: as many threads as `horizon-threads` gives the machine, at most 8.
pub fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(lanes())
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
pub fn par_map<T: Sync, R: Send>(
    items: &[T],
    min: usize,
    f: impl Fn(&[T], &mut Vec<R>) + Sync,
) -> Vec<R> {
    let mut out = Vec::with_capacity(items.len());
    let lanes = lanes();
    if lanes <= 1 || items.len() < min {
        f(items, &mut out);
        return out;
    }
    let size = items.len().div_ceil(lanes);
    let parts: Vec<Vec<R>> = pool().install(|| {
        items
            .par_chunks(size)
            .map(|part| {
                let mut done = Vec::with_capacity(part.len());
                f(part, &mut done);
                done
            })
            .collect()
    });
    for p in parts {
        out.extend(p);
    }
    out
}

/// `f` over the matching parts of `a` and `b` (split alike, `size` at a time) on the pool.
pub fn par_zip_mut<A: Send, B: Send>(
    a: &mut [A],
    b: &mut [B],
    size: usize,
    f: impl Fn(&mut [A], &mut [B]) + Sync,
) {
    pool().install(|| {
        a.par_chunks_mut(size)
            .zip(b.par_chunks_mut(size))
            .for_each(|(a, b)| f(a, b));
    });
}
