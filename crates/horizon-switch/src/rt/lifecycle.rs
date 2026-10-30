// Adapted from bevy-horizon (https://github.com/JohnUzoka/bevy-horizon, MIT OR Apache-2.0, see
// licenses/bevy-horizon-*): crates/bevy_horizon/src/lifecycle.rs, the parts that are not Bevy's:
// the services a program opens (romfs, sockets), and closing them so the homebrew loader can read
// the next NRO into the heap. Left out: Bevy's task pools, the frame timer, and the NoSuspend
// focus handling (libnx's default, `SuspendHomeSleep`, suspends the program for HOME and sleep).
use core::ffi::{c_char, c_void};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::logging::{ResultCode, stop_log_writer, sync_log, write_log, write_log_line};

unsafe extern "C" {
    fn romfsMountSelf(name: *const c_char) -> u32;
    fn romfsUnmount(name: *const c_char) -> u32;
    fn socketInitialize(config: *const c_void) -> u32;
    fn socketExit();
    /// libnx closes nvdrv when the last reference goes, and then waits, with no timeout, for its
    /// transfer memory to unlock.
    fn nvExit();
    /// libnx's `Service` for the nvdrv session, zeroed while nvdrv is closed.
    fn nvGetServiceSession() -> *const Service;
    /// libnx `applet.c`: when 1, `appletExit` (run after `userAppExit`) has AM end the process.
    static mut __nx_applet_exit_mode: u32;
}

/// libnx `Service` (switch/sf/service.h). `session` is 0 while the service is closed.
#[repr(C)]
struct Service {
    session: u32,
    own_handle: u32,
    object_id: u32,
    pointer_buffer_size: u16,
}

/// `MAKERESULT(Module_Libnx, LibnxError_IoError)`: what `romfsMountSelf` returns when the NRO has
/// no RomFS.
const LIBNX_IO_ERROR: u32 = 345 | (10 << 9);

static ROMFS_MOUNTED: AtomicBool = AtomicBool::new(false);
/// Sockets lock 2.2 MiB of the heap until `socketExit`.
static SOCKETS_UP: AtomicBool = AtomicBool::new(false);

const MAX_RELEASE_HOOKS: usize = 8;
static RELEASE_HOOKS: [AtomicUsize; MAX_RELEASE_HOOKS] =
    [const { AtomicUsize::new(0) }; MAX_RELEASE_HOOKS];
static RELEASE_HOOK_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Registers `hook` to run when the NRO ends itself, before the services close and the heap is
/// checked for the homebrew loader. Register anything the program opened that keeps heap pages
/// locked until it is closed (a service's transfer memory: audren's `audrenExit`, the console's
/// `consoleExit`). Hooks run newest first, once each. Returns false when all 8 slots are taken.
pub fn register_release_hook(hook: fn()) -> bool {
    let slot = RELEASE_HOOK_COUNT.fetch_add(1, Ordering::SeqCst);
    if slot >= MAX_RELEASE_HOOKS {
        RELEASE_HOOK_COUNT.store(MAX_RELEASE_HOOKS, Ordering::SeqCst);
        return false;
    }
    RELEASE_HOOKS[slot].store(hook as usize, Ordering::SeqCst);
    true
}

fn run_release_hooks() {
    let count = RELEASE_HOOK_COUNT
        .load(Ordering::SeqCst)
        .min(MAX_RELEASE_HOOKS);
    for slot in (0..count).rev() {
        let hook = RELEASE_HOOKS[slot].swap(0, Ordering::SeqCst);
        if hook != 0 {
            // SAFETY: only `register_release_hook` stores non-zero values here, and they are `fn()`s.
            let hook: fn() = unsafe { core::mem::transmute::<usize, fn()>(hook) };
            hook();
            write_log_line(format_args!("[Exit] release hook {slot}: done"));
        }
    }
}

/// Mounts the NRO's RomFS (if it has one) and starts the BSD sockets.
pub(crate) fn init_services() {
    // SAFETY: plain libnx calls, once, on the main thread after startup.
    unsafe {
        let rc = romfsMountSelf(c"romfs".as_ptr());
        if rc == 0 {
            ROMFS_MOUNTED.store(true, Ordering::SeqCst);
            write_log_line(format_args!("[Init] romfs mounted"));
        } else if rc == LIBNX_IO_ERROR {
            write_log_line(format_args!(
                "[Init] no RomFS in this NRO (normal unless elf2nro got --romfsdir)"
            ));
        } else {
            write_log_line(format_args!(
                "[Init] romfsMountSelf failed: {}",
                ResultCode(rc)
            ));
        }
        let rc = socketInitialize(core::ptr::null());
        if rc == 0 {
            SOCKETS_UP.store(true, Ordering::SeqCst);
            write_log_line(format_args!("[Init] BSD sockets up"));
        } else {
            write_log_line(format_args!(
                "[Init] socketInitialize failed: {}",
                ResultCode(rc)
            ));
        }
    }
}

/// Undoes `init_services` and checks that the homebrew loader can read the next NRO into the
/// malloc heap. Called from libnx's `userAppExit` hook (`compat.rs`), which runs whenever the NRO
/// ends itself: `main` returning, `std::process::exit`, or an abort.
///
/// sphaira's loader reads the whole heap in one `fsFileRead`, and the kernel rejects that with
/// 2001-0106 if any page of it is still locked: by a service's transfer memory until the service
/// is closed (`socketExit`, `nvExit`), or by a thread's stack until the thread is joined or, if
/// detached, closed. Runs once and allocates nothing: an abort in here would come back through
/// `userAppExit`. If heap pages are still locked at the end, it sets `__nx_applet_exit_mode`, so
/// AM ends the process and the Switch goes back to HOME, as when any game exits.
pub(crate) fn release_horizon_services() {
    use super::memory::{check_heap_for_exit, wait_for_heap_unlock};
    use std::time::Duration;

    static RELEASED: AtomicBool = AtomicBool::new(false);
    if RELEASED.swap(true, Ordering::SeqCst) {
        return;
    }
    write_log(b"[Exit] userAppExit: main has returned; releasing services before the loader takes the heap back\n");
    stop_log_writer();
    let blocked = check_heap_for_exit("before releasing services");
    run_release_hooks();
    close_detached_threads();
    if SOCKETS_UP.swap(false, Ordering::SeqCst) {
        // SAFETY: closes what `init_services` opened.
        unsafe { socketExit() };
        write_log(b"[Exit] socketExit: done\n");
    }
    if ROMFS_MOUNTED.swap(false, Ordering::SeqCst) {
        // SAFETY: as above.
        unsafe { romfsUnmount(c"romfs".as_ptr()) };
        write_log(b"[Exit] romfsUnmount: done\n");
    }
    release_nvdrv();
    if matches!(blocked, Some(n) if n > 0) {
        const TIMEOUT: Duration = Duration::from_secs(1);
        match wait_for_heap_unlock(TIMEOUT) {
            Some(waited) => write_log_line(format_args!(
                "[Exit] the heap was unlocked {:.2} ms later, once the services had let go of their transfer memory",
                waited.as_secs_f64() * 1000.0
            )),
            None => write_log_line(format_args!(
                "[Exit] the heap was still locked {} ms later: a service kept its transfer memory, or something else still holds heap pages",
                TIMEOUT.as_millis()
            )),
        }
    }
    match check_heap_for_exit("after releasing services") {
        Some(0) => {
            write_log(b"[Exit] PASS: nothing in the heap is locked, so the loader can read the next NRO into it\n");
        }
        Some(_) => {
            // SAFETY: only this thread runs now, and libnx reads the flag later on it, in appletExit.
            unsafe { __nx_applet_exit_mode = 1 };
            write_log(b"[Exit] WARN: heap pages are still locked, so the loader would crash reading the next NRO (2001-0106); exiting through AM to HOME instead\n");
        }
        None => {
            write_log(b"[Exit] heap check failed: svcQueryMemory returned an error; returning to the loader\n");
        }
    }
    sync_log();
}

/// Closes nvdrv if something left it open, unless the GPU driver still uses it: on a clean exit
/// the Vulkan driver has closed it already (Mesa's horizon runtime holds a reference per device).
/// When the program ends without dropping the device (a panic), `nvExit` would close nvdrv under
/// the driver and then wait for its transfer memory with no timeout; so while nvmap objects still
/// hold heap pages this leaves nvdrv open and exits through AM instead.
fn release_nvdrv() {
    // SAFETY: libnx's static Service; nothing else closes it on this thread.
    let open = unsafe { (*nvGetServiceSession()).session != 0 };
    if !open {
        write_log(b"[Exit] nvdrv: already closed (the GPU driver released it)\n");
        return;
    }
    match super::memory::nvmap_heap() {
        Some((0, _)) => {
            write_log(b"[Exit] nvdrv: still open, but no heap pages are held for the GPU; calling nvExit\n");
            // SAFETY: nothing holds nvdrv's memory.
            unsafe { nvExit() };
            write_log(b"[Exit] nvExit: done\n");
        }
        mapped => {
            // SAFETY: as in `release_horizon_services`.
            unsafe { __nx_applet_exit_mode = 1 };
            match mapped {
                Some((blocks, bytes)) => write_log_line(format_args!(
                    "[Exit] nvdrv: still open, and {blocks} heap blocks ({:.2} MiB) are still held for the GPU, so the GPU driver never shut down; skipping nvExit and exiting through AM to HOME",
                    bytes as f64 / (1024.0 * 1024.0)
                )),
                None => {
                    write_log(b"[Exit] nvdrv: still open, and svcQueryMemory failed; skipping nvExit and exiting through AM to HOME\n");
                }
            }
        }
    }
}

/// Closes the detached threads that have exited, waiting up to 1 s for the rest. std detaches a
/// thread whose `JoinHandle` is dropped, and until it is closed its stack keeps 2 MiB of the heap
/// locked (`compat.rs`).
fn close_detached_threads() {
    use std::time::{Duration, Instant};

    const TIMEOUT: Duration = Duration::from_secs(1);
    let started = Instant::now();
    let Some(threads) = super::compat::horizon_stubs::close_detached_threads(TIMEOUT) else {
        write_log_line(format_args!(
            "[Exit] detached threads: another thread was closing them for all of {} ms",
            TIMEOUT.as_millis()
        ));
        return;
    };
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    write_log_line(format_args!(
        "[Exit] detached threads: closed {} now and {} while the program ran ({} threads were created in all); {} still running after {ms:.2} ms",
        threads.closed,
        threads.closed_total - threads.closed,
        threads.created_total,
        threads.running,
    ));
    if threads.close_failed_total + threads.untracked_total > 0 {
        write_log_line(format_args!(
            "[Exit] detached threads: WARN: {} exited but threadClose failed; {} were detached with all 256 slots in use; their stacks stay locked",
            threads.close_failed_total, threads.untracked_total,
        ));
    }
}
