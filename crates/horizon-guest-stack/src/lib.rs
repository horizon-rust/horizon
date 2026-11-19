//! Stack policy shared by guest callers and executable link settings.

#[path = "guest_stack_constants.rs"]
mod constants;
pub use constants::{GUEST_STACK_BYTES, HOST_HEADROOM};

/// Spawn guarantees the reservation even on platforms without a stack-size query.
pub fn builder() -> std::thread::Builder {
    std::thread::Builder::new().stack_size(GUEST_STACK_BYTES + HOST_HEADROOM)
}

/// Starts a thread that may enter a guest: guest-sized and named `name`, and in debug builds it
/// asserts at its start that the reservation took, so a test that starts it fails loudly if not.
pub fn spawn<T: Send + 'static>(
    name: String,
    body: impl FnOnce() -> T + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<T>> {
    builder().name(name).spawn(move || {
        debug_assert_eq!(
            check_current(),
            Ok(()),
            "a guest-capable thread must start on the guest stack"
        );
        body()
    })
}

/// Runs `test` on a guest-sized worker named like the calling thread, so libtest reports it under
/// its own name, and resumes its panic with the original payload. For tests that enter a guest:
/// libtest's own threads are too small.
pub fn run_on_guest_stack(test: impl FnOnce() + Send + 'static) {
    let mut worker = builder();
    if let Some(name) = std::thread::current().name() {
        worker = worker.name(name.to_owned());
    }
    if let Err(payload) = worker.spawn(test).unwrap().join() {
        std::panic::resume_unwind(payload);
    }
}

/// Must run before compiler-child handling or any game initialization.
pub fn prepare_main() {
    if let Err(error) = prepare() {
        eprintln!("GuestStackUnavailable: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn prepare() -> std::io::Result<()> {
    // Executable link settings reserve the same stack on Windows and macOS.
    inspect_current().map_err(std::io::Error::other)
}

#[cfg(target_os = "linux")]
fn prepare() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let required = (GUEST_STACK_BYTES + HOST_HEADROOM) as libc::rlim_t;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: limit is writable and getrlimit initializes it on success.
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if limit.rlim_cur >= required && inspect_current().is_ok() {
        return Ok(());
    }
    let describe = |bytes| {
        if bytes == libc::RLIM_INFINITY {
            "unlimited".to_owned()
        } else {
            format!("{} KiB", bytes / 1024)
        }
    };
    let limits = format!(
        "soft={}, hard={}",
        describe(limit.rlim_cur),
        describe(limit.rlim_max)
    );
    if std::env::var_os("HORIZON_STACK_REEXEC").is_some() {
        return Err(std::io::Error::other(format!(
            "stack re-exec marker already set but the stack is insufficient ({limits})"
        )));
    }
    if limit.rlim_max < required {
        return Err(std::io::Error::other(format!(
            "stack hard limit is too low ({limits}); raise ulimit -Hs to at least {} KiB",
            required.saturating_add(HOST_HEADROOM as libc::rlim_t) / 1024
        )));
    }
    // The argument and environment pages sit at the top of the main stack and count
    // against the limit. Reserve additional host headroom for those pages when the
    // hard limit permits it; guest entry still verifies the room left.
    limit.rlim_cur = required
        .saturating_add(HOST_HEADROOM as libc::rlim_t)
        .min(limit.rlim_max);
    // SAFETY: limit is initialized; this changes only this process's resource limit.
    if unsafe { libc::setrlimit(libc::RLIMIT_STACK, &limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Linux chooses its mmap base at exec using RLIMIT_STACK. Re-exec keeps the
    // enlarged stack's address range clear before libraries and workers start.
    Err(std::process::Command::new(std::env::current_exe()?)
        .arg0(std::env::args_os().next().unwrap_or_default())
        .args(std::env::args_os().skip(1))
        .env("HORIZON_STACK_REEXEC", "1")
        .exec())
}

/// Refuse guest entry when the calling thread's stack reservation is too small.
/// Cache the platform query once per thread; the hot path reads only thread-local state.
pub fn check_current() -> Result<(), &'static str> {
    thread_local! {
        static CHECK: std::cell::OnceCell<Result<(), &'static str>> = const { std::cell::OnceCell::new() };
    }
    CHECK.with(|check| *check.get_or_init(inspect_current))
}

fn inspect_current() -> Result<(), &'static str> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: both calls only read this process's ids.
        if unsafe { libc::syscall(libc::SYS_gettid) == libc::c_long::from(libc::getpid()) } {
            return match main_thread_room() {
                Some(room) if room >= GUEST_STACK_BYTES + HOST_HEADROOM => Ok(()),
                Some(_) => Err("GuestStackUnavailable: calling thread needs 136 MiB of stack"),
                None => Err("GuestStackUnavailable: cannot inspect the calling thread stack"),
            };
        }
        let mut attr = std::mem::MaybeUninit::<libc::pthread_attr_t>::uninit();
        // SAFETY: pthread_self is live and attr is writable. Destroy only on success.
        unsafe {
            if libc::pthread_getattr_np(libc::pthread_self(), attr.as_mut_ptr()) != 0 {
                return Err("GuestStackUnavailable: cannot inspect the calling thread stack");
            }
            let mut attr = attr.assume_init();
            let mut size = 0;
            let result = libc::pthread_attr_getstacksize(&attr, &mut size);
            libc::pthread_attr_destroy(&mut attr);
            if result != 0 || size < GUEST_STACK_BYTES + HOST_HEADROOM {
                return Err("GuestStackUnavailable: calling thread needs 136 MiB of stack");
            }
        }
    }
    #[cfg(target_vendor = "apple")]
    {
        // SAFETY: pthread_self names the live calling thread.
        let size = unsafe { libc::pthread_get_stacksize_np(libc::pthread_self()) };
        if size < GUEST_STACK_BYTES + HOST_HEADROOM {
            return Err("GuestStackUnavailable: calling thread needs 136 MiB of stack");
        }
    }
    #[cfg(windows)]
    {
        let mut low = 0;
        let mut high = 0;
        // SAFETY: both pointers refer to writable stack bounds.
        unsafe { get_current_thread_stack_limits(&mut low, &mut high) };
        if high - low < GUEST_STACK_BYTES + HOST_HEADROOM {
            return Err("GuestStackUnavailable: calling thread needs 136 MiB of stack");
        }
    }
    #[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
    return Err("GuestStackUnavailable: stack inspection is unsupported on this platform");
    #[cfg(any(target_os = "linux", target_vendor = "apple", windows))]
    Ok(())
}

/// The main thread's stack grows on demand up to `RLIMIT_STACK`, so its size is the limit, not the
/// pages mapped so far (what musl's `pthread_getattr_np` reports for it). Reads the stack's top and
/// the mapping below it from `/proc/self/maps`, and where the arguments and environment end (the
/// initial stack pointer, `startstack`) from `/proc/self/stat`. The answer doesn't depend on how
/// deep the caller is, so `check_current` may cache it.
#[cfg(target_os = "linux")]
fn main_thread_room() -> Option<usize> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: limit is writable and getrlimit initializes it on success.
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut limit) } != 0 {
        return None;
    }
    // RLIM_INFINITY is the largest value, so it stays unbounded here.
    let soft_limit = usize::try_from(limit.rlim_cur).unwrap_or(usize::MAX);
    // SAFETY: sysconf only reads a system setting.
    let page = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
    // The kernel keeps `stack_guard_gap` (256 pages by default) free above the mapping below.
    let guard_gap = 256 * page;
    let stat = std::fs::read("/proc/self/stat").ok()?;
    // Field 28; the fields after the command name, which ends at the last ')', start at field 3.
    let after_name = &stat[stat.iter().rposition(|&byte| byte == b')')? + 1..];
    let start_stack: usize = std::str::from_utf8(after_name)
        .ok()?
        .split_ascii_whitespace()
        .nth(25)?
        .parse()
        .ok()?;
    // Mapped file names need not be UTF-8: parse the bytes.
    let maps = std::fs::read("/proc/self/maps").ok()?;
    let mut below: usize = 0;
    for line in maps
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let range = line.split(|&byte| byte == b' ').next()?;
        let (start, end) = std::str::from_utf8(range).ok()?.split_once('-')?;
        let start = usize::from_str_radix(start, 16).ok()?;
        let end = usize::from_str_radix(end, 16).ok()?;
        if line.trim_ascii_end().ends_with(b"[stack]") {
            return (start..end).contains(&start_stack).then(|| {
                main_stack_room(
                    soft_limit,
                    below.saturating_add(guard_gap),
                    end,
                    start_stack,
                )
            });
        }
        below = end;
    }
    None
}

/// Room left below `used_from` on a stack whose top is `top`: it may grow to `soft_limit` bytes in
/// all (the pages above `used_from` included), and never below `floor`.
#[cfg(target_os = "linux")]
fn main_stack_room(soft_limit: usize, floor: usize, top: usize, used_from: usize) -> usize {
    top.saturating_sub(floor)
        .min(soft_limit)
        .saturating_sub(top.saturating_sub(used_from))
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetCurrentThreadStackLimits"]
    fn get_current_thread_stack_limits(low: *mut usize, high: *mut usize);
}

#[cfg(test)]
#[test]
fn audited_guest_workers_use_the_stack_policy() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in [
        "crates/horizon-threads/src/jobs.rs",
        "crates/horizon-threads/src/lib.rs",
        "crates/horizon-client/src/lan/io_pool.rs",
        "crates/horizon-client/src/vanillagen.rs",
        "crates/horizon-client/src/vanillagen/lod.rs",
        "crates/horizon-client/src/game/runtime/resource_load.rs",
        "crates/horizon-client/src/game/interface/startup_menu.rs",
        "crates/horizon-client/src/game/world/world_loading.rs",
        "crates/horizon-client/src/game/world/world_entry.rs",
        "crates/horizon-client/src/game/world/packs/mod.rs",
    ] {
        let source = std::fs::read_to_string(root.join(file)).unwrap();
        let source = source.split("\n#[cfg(test)]").next().unwrap();
        assert!(
            ![
                "thread::Builder::new()",
                "thread::spawn(",
                "scope.spawn(",
                "s.spawn("
            ]
            .iter()
            .any(|form| source.contains(form)),
            "{file}: route guest workers through horizon_guest_stack::builder"
        );
    }
    let server =
        std::fs::read_to_string(root.join("crates/horizon-client/src/game/world/integrated.rs"))
            .unwrap();
    // Production code only: a test may start its own server thread with the same policy.
    let server = server.split("\n#[cfg(test)]").next().unwrap();
    assert_eq!(
        server.matches(".stack_size(server_thread_stack())").count(),
        2
    );
    assert_eq!(server.matches("thread::Builder::new()").count(), 2);
    for form in ["thread::spawn(", "scope.spawn(", "s.spawn("] {
        assert!(
            !server.contains(form),
            "server: route guest workers through the stack policy"
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn small_worker_refuses_guest_entry() {
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                assert!(
                    super::check_current()
                        .unwrap_err()
                        .contains("GuestStackUnavailable")
                )
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn main_stack_is_judged_by_its_limit() {
        const MIB: usize = 1024 * 1024;
        let needed = super::GUEST_STACK_BYTES + super::HOST_HEADROOM;
        let top = 0x7fff_0000_0000;
        let far = top - 1024 * MIB;
        // A re-executed Nightly on musl: soft limit 144 MiB, a few hundred KiB used so far.
        assert!(super::main_stack_room(144 * MIB, far, top, top - 300 * 1024) >= needed);
        // A hard limit of 140 MiB caps the soft one there, which still leaves 136 MiB.
        assert!(super::main_stack_room(140 * MIB, far, top, top - 300 * 1024) >= needed);
        // Unlimited, with the next mapping far below.
        assert!(super::main_stack_room(usize::MAX, far, top, top - 300 * 1024) >= needed);
        // Exactly 136 MiB, but 2 MiB of environment above the caller already count against it.
        assert!(super::main_stack_room(136 * MIB, far, top, top - 2 * MIB) < needed);
        // Growth stops 100 MiB below the top, before the limit does.
        assert!(super::main_stack_room(144 * MIB, top - 100 * MIB, top, top - 300 * 1024) < needed);
    }

    #[test]
    fn guest_worker_reserves_the_required_stack() {
        super::builder()
            .spawn(|| super::check_current().unwrap())
            .unwrap()
            .join()
            .unwrap();
    }
}
