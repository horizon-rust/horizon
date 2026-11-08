//! Safe async task offloading API for native mods using `horizon_threads::jobs::pool()`.
//! API @API:mod_async_worker@.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use super::Game;
use horizon_client::mod_api::{Host, TaskCallback};
use horizon_threads::jobs::{Class, Rank, Registration, Source};

struct NativeTaskSource {
    queue: Mutex<VecDeque<(i64, Box<dyn FnOnce() + Send + 'static>)>>,
    next_id: AtomicI64,
}

impl Source for NativeTaskSource {
    fn next(&self) -> Option<Rank> {
        let q = self.queue.lock().ok()?;
        let (id, _) = q.front()?;
        Some(Rank::new(Class::Near, *id))
    }

    fn run(&self, _worker: usize) -> bool {
        let job = self.queue.lock().ok().and_then(|mut q| q.pop_front());
        if let Some((_, job)) = job {
            job();
            true
        } else {
            false
        }
    }

    fn label(&self) -> &'static str {
        "native-mod-tasks"
    }
}

struct TaskRunner {
    source: Arc<NativeTaskSource>,
    registration: Registration,
}

static RUNNER: OnceLock<TaskRunner> = OnceLock::new();

fn runner() -> &'static TaskRunner {
    RUNNER.get_or_init(|| {
        let source = Arc::new(NativeTaskSource {
            queue: Mutex::new(VecDeque::new()),
            next_id: AtomicI64::new(0),
        });
        let registration = horizon_threads::jobs::pool().register(source.clone());
        TaskRunner {
            source,
            registration,
        }
    })
}

static ACTIVE_QUEUE: Mutex<Option<Weak<Mutex<VecDeque<TaskCallback>>>>> = Mutex::new(None);

/// Submit a background job to `horizon_threads::jobs::pool()`.
pub fn spawn_background_job<F>(job: F)
where
    F: FnOnce() + Send + 'static,
{
    let runner = runner();
    let id = runner.source.next_id.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut q) = runner.source.queue.lock() {
        q.push_back((id, Box::new(job)));
    }
    runner.registration.notify();
}

/// Defer a callback to run on the main game/server thread during `native_tick`.
pub fn defer_main_thread<F>(callback: F)
where
    F: FnOnce(&mut dyn Host) + Send + 'static,
{
    if let Ok(lock) = ACTIVE_QUEUE.lock()
        && let Some(weak) = lock.as_ref()
        && let Some(arc) = weak.upgrade()
        && let Ok(mut q) = arc.lock()
    {
        q.push_back(Box::new(callback));
    }
}

/// Offload heavy computation to `horizon_threads::jobs::pool()` and safely run a callback
/// on the main game/server thread with the result during `native_tick`.
pub fn spawn_task<T, F, C>(task: F, callback: C)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    C: FnOnce(&mut dyn Host, T) + Send + 'static,
{
    let target = {
        let lock = ACTIVE_QUEUE.lock().ok();
        lock.and_then(|l| l.as_ref().cloned())
    };
    spawn_background_job(move || {
        let result = task();
        if let Some(weak) = target
            && let Some(arc) = weak.upgrade()
            && let Ok(mut q) = arc.lock()
        {
            q.push_back(Box::new(move |host| callback(host, result)));
        }
    });
}

pub(super) struct NativeTasks {
    pub(super) callbacks: Arc<Mutex<VecDeque<TaskCallback>>>,
}

impl NativeTasks {
    pub(super) fn new() -> Self {
        let callbacks = Arc::new(Mutex::new(VecDeque::new()));
        if let Ok(mut lock) = ACTIVE_QUEUE.lock() {
            *lock = Some(Arc::downgrade(&callbacks));
        }
        Self { callbacks }
    }
}

impl Default for NativeTasks {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for NativeTasks {
    fn drop(&mut self) {
        if let Ok(mut lock) = ACTIVE_QUEUE.lock() {
            *lock = None;
        }
    }
}

impl Game<'_> {
    /// Submit a background job to `horizon_threads::jobs::pool()`. API @API:mod_async_worker@.
    pub(super) fn native_spawn_background_job(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        spawn_background_job(job);
    }

    /// Defer a callback to run on the main game/server thread during `native_tick`. API @API:mod_async_worker@.
    pub(super) fn native_defer_main_thread(&self, callback: TaskCallback) {
        if let Ok(mut q) = self.native_tasks.callbacks.lock() {
            q.push_back(callback);
        }
    }

    /// Offload heavy computation to `horizon_threads::jobs::pool()` and run `callback` on the
    /// main thread during `native_tick`. API @API:mod_async_worker@.
    pub(super) fn native_spawn_task(
        &self,
        task: Box<dyn FnOnce() + Send + 'static>,
        callback: TaskCallback,
    ) {
        let weak = Arc::downgrade(&self.native_tasks.callbacks);
        spawn_background_job(move || {
            task();
            if let Some(arc) = weak.upgrade()
                && let Ok(mut q) = arc.lock()
            {
                q.push_back(callback);
            }
        });
    }

    pub(super) fn native_process_tasks(&mut self) {
        let callbacks: Vec<TaskCallback> = {
            let Ok(mut q) = self.native_tasks.callbacks.lock() else {
                return;
            };
            q.drain(..).collect()
        };
        for cb in callbacks {
            cb(self);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicI32};
    use std::time::{Duration, Instant};

    struct DoubleHost;
    impl Host for DoubleHost {}

    #[test]
    fn background_job_runs_on_worker_pool() {
        let ran = Arc::new(AtomicBool::new(false));
        let ran_clone = ran.clone();
        spawn_background_job(move || {
            ran_clone.store(true, Ordering::SeqCst);
        });
        let start = Instant::now();
        while !ran.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(ran.load(Ordering::SeqCst));
    }

    #[test]
    fn spawn_task_runs_computation_and_delivers_callback() {
        let tasks = NativeTasks::new();
        let mut host = DoubleHost;
        let seen = Arc::new(AtomicI32::new(0));
        let seen_clone = seen.clone();

        spawn_task(
            move || {
                // Background heavy work (e.g. physics calculation)
                100 + 23
            },
            move |_host, result| {
                seen_clone.store(result, Ordering::SeqCst);
            },
        );

        let start = Instant::now();
        while tasks.callbacks.lock().unwrap().is_empty() && start.elapsed() < Duration::from_secs(3)
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(tasks.callbacks.lock().unwrap().len(), 1);

        let cbs: Vec<_> = tasks.callbacks.lock().unwrap().drain(..).collect();
        for cb in cbs {
            cb(&mut host);
        }
        assert_eq!(seen.load(Ordering::SeqCst), 123);
    }

    #[test]
    fn defer_main_thread_enqueues_callback() {
        let tasks = NativeTasks::new();
        let mut host = DoubleHost;
        let ran = Arc::new(AtomicBool::new(false));
        let ran_clone = ran.clone();

        defer_main_thread(move |_host| {
            ran_clone.store(true, Ordering::SeqCst);
        });

        assert_eq!(tasks.callbacks.lock().unwrap().len(), 1);
        let cbs: Vec<_> = tasks.callbacks.lock().unwrap().drain(..).collect();
        for cb in cbs {
            cb(&mut host);
        }
        assert!(ran.load(Ordering::SeqCst));
    }

    #[test]
    fn dropped_tasks_cleans_up_active_queue() {
        let tasks = NativeTasks::new();
        drop(tasks);
        // After drop, defer_main_thread silently ignores or discards without panic or leak
        defer_main_thread(|_host| {});
    }
}
