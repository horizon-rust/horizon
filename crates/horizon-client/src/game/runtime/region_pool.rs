//! The threads region ticks run on: started once and kept, so a tick with a hundred regions does not start a
//! hundred threads (that took more of the server thread's tick than the regions' own wait).
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send>;

struct Pool {
    jobs: Sender<Job>,
    queue: Arc<Mutex<Receiver<Job>>>,
    /// Workers waiting for a job and not yet promised one.
    idle: Arc<AtomicUsize>,
    started: AtomicUsize,
}

/// Runs `job` on a pool thread; a new thread starts only when every one is busy (a region that runs past the
/// tick's deadline keeps its thread). Returns false if no thread could start.
pub(super) fn run(job: impl FnOnce() + Send + 'static) -> bool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    let pool = POOL.get_or_init(|| {
        let (jobs, queue) = channel();
        Pool {
            jobs,
            queue: Arc::new(Mutex::new(queue)),
            idle: Arc::new(AtomicUsize::new(0)),
            started: AtomicUsize::new(0),
        }
    });
    // Promise the job to an idle worker, or start one for it.
    let mut idle = pool.idle.load(Ordering::Acquire);
    loop {
        if idle == 0 {
            let (queue, idle) = (Arc::clone(&pool.queue), Arc::clone(&pool.idle));
            let n = pool.started.fetch_add(1, Ordering::Relaxed);
            let started = std::thread::Builder::new()
                .name(format!("Region worker {n}"))
                .spawn(move || {
                    loop {
                        let job = queue
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .recv();
                        let Ok(job) = job else { return };
                        job();
                        idle.fetch_add(1, Ordering::Release);
                    }
                });
            if started.is_err() {
                return false;
            }
            break;
        }
        match pool
            .idle
            .compare_exchange(idle, idle - 1, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => break,
            Err(now) => idle = now,
        }
    }
    pool.jobs.send(Box::new(job)).is_ok()
}
