// Adapted from bevy-horizon (https://github.com/JohnUzoka/bevy-horizon, MIT OR Apache-2.0, see
// licenses/bevy-horizon-*): crates/bevy_horizon/src/memory.rs. Changes: Bevy-free, and every
// `cfg(target_os = "horizon")` dropped because this whole module is Horizon-only.
use core::ffi::c_int;

mod ffi {
    use core::ffi::c_int;

    /// newlib `struct mallinfo` (devkitA64 `malloc.h`).
    #[repr(C)]
    pub struct MallInfo {
        /// Bytes malloc has taken from the heap with `sbrk`.
        pub arena: usize,
        pub ordblks: usize,
        pub smblks: usize,
        pub hblks: usize,
        pub hblkhd: usize,
        pub usmblks: usize,
        pub fsmblks: usize,
        /// Bytes in allocated blocks.
        pub uordblks: usize,
        /// Free bytes, the top chunk included.
        pub fordblks: usize,
        /// The top chunk: free space `free` could give back to the system.
        pub keepcost: usize,
    }

    unsafe extern "C" {
        pub fn appletGetAppletType() -> c_int;
        pub fn svcGetInfo(out: *mut u64, id0: u32, handle: u32, id1: u64) -> u32;
        pub fn envHasHeapOverride() -> bool;
        pub fn envGetHeapOverrideAddr() -> *mut core::ffi::c_void;
        pub fn envGetHeapOverrideSize() -> u64;
        pub fn mallinfo() -> MallInfo;
        /// The malloc heap's bounds, set by libnx's `__libnx_initheap`; `sbrk` hands out memory
        /// between them. NVK lowers `fake_heap_end` to take GPU memory from the top of the heap
        /// (see [`super::heap_end`]), so read it with [`super::heap_limit`].
        pub static fake_heap_start: *mut u8;
        pub static fake_heap_end: *mut u8;
    }

    // The top-down region's API (bevy-horizon's `patches/mesa-switch/`), in NVK.
    unsafe extern "C" {
        /// The region's extent and the bytes in its blocks, GPU and CPU, now and at the most,
        /// and how many GPU blocks it couldn't place. Takes the region's lock; doesn't allocate.
        pub fn nouveau_horizon_region_stats(
            size: *mut u64,
            used: *mut u64,
            peak: *mut u64,
            gpu_fallbacks: *mut u64,
        );
    }
}

/// `fake_heap_end` now: where `sbrk`, and with it malloc, has to stop. NVK lowers and raises it
/// while the app runs, so it's read with a volatile load.
pub(crate) fn heap_limit() -> usize {
    unsafe { core::ptr::read_volatile(&raw const ffi::fake_heap_end) as usize }
}

/// libnx `svc.h`.
const CUR_PROCESS_HANDLE: u32 = 0xFFFF_8001;
const INFO_TYPE_TOTAL_MEMORY_SIZE: u32 = 6;
const INFO_TYPE_USED_MEMORY_SIZE: u32 = 7;

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// libnx `AppletType` (switch/services/applet.h).
fn applet_type_name(applet_type: c_int) -> &'static str {
    match applet_type {
        0 => {
            "Application: running as a game (a forwarder icon or title override), so it gets a game's memory"
        }
        2 => {
            "LibraryApplet: applet mode, e.g. started from the Album, so it gets an applet's memory"
        }
        1 => "SystemApplet",
        3 => "OverlayApplet",
        4 => "SystemApplication",
        -1 => "Default: the loader didn't say",
        -2 => "None",
        _ => "unknown",
    }
}

fn process_info(info_type: u32) -> Option<u64> {
    let mut out = 0u64;
    let rc = unsafe { ffi::svcGetInfo(&mut out, info_type, CUR_PROCESS_HANDLE, 0) };
    (rc == 0).then_some(out)
}

/// libnx `MemType_Heap` (switch/kernel/svc.h).
const MEM_TYPE_HEAP: u32 = 5;

/// Where the heap ends, as libnx's `__libnx_initheap` set it. `fake_heap_end` itself no longer
/// says: NVK takes GPU memory from the top of the heap downward by lowering it, so that `sbrk`,
/// and with it malloc, stops below that memory (the top-down region bevy-horizon's
/// `patches/mesa-switch/` adds to mesa-switch's `nouveau_horizon_memory.c`).
///
/// An NRO's heap is the one the homebrew loader passed in, which libnx's `envGetHeapOverride*`
/// still give. Without one, `svcQueryMemory` walks the heap's blocks up from `fake_heap_start`
/// until the memory type isn't heap; device mapping and locking change a block's attributes, not
/// its type. Worked out once and cached, without allocating.
fn heap_end() -> u64 {
    use core::sync::atomic::{AtomicU64, Ordering};
    static HEAP_END: AtomicU64 = AtomicU64::new(0);

    let cached = HEAP_END.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    let end = unsafe {
        if ffi::envHasHeapOverride() {
            (ffi::envGetHeapOverrideAddr() as u64).saturating_add(ffi::envGetHeapOverrideSize())
        } else {
            walk_heap_end()
        }
    };
    let end = end.max(heap_limit() as u64);
    HEAP_END.store(end, Ordering::Relaxed);
    end
}

/// The end of the run of heap-type blocks that starts at `fake_heap_start`, or 0 if a query fails.
fn walk_heap_end() -> u64 {
    use crate::rt::logging::ffi as log_ffi;

    let mut addr = unsafe { ffi::fake_heap_start as u64 };
    loop {
        let mut info = log_ffi::MemoryInfo::default();
        let mut page_info = 0u32;
        let rc = unsafe { log_ffi::svcQueryMemory(&mut info, &mut page_info, addr) };
        let block_end = info.addr.saturating_add(info.size);
        if rc != 0 || block_end <= addr {
            return 0;
        }
        if info.kind != MEM_TYPE_HEAP {
            return addr;
        }
        addr = block_end;
    }
}

/// The whole heap, NVK's GPU region at the top included.
fn heap_size() -> u64 {
    heap_end().saturating_sub(unsafe { ffi::fake_heap_start as u64 })
}

/// How much of the top of the heap the top-down region has: everything above `fake_heap_end`.
fn top_down_region() -> u64 {
    heap_end().saturating_sub(heap_limit() as u64)
}

/// The boot log's `[Memory]` lines.
pub(crate) fn boot_report() -> String {
    let applet_type = unsafe { ffi::appletGetAppletType() };
    let process = match (
        process_info(INFO_TYPE_TOTAL_MEMORY_SIZE),
        process_info(INFO_TYPE_USED_MEMORY_SIZE),
    ) {
        (Some(total), Some(used)) => format!(
            "{:.1} MiB in total, {:.1} MiB used (svcGetInfo; \"used\" already counts the whole malloc heap below)",
            mib(total),
            mib(used)
        ),
        _ => "svcGetInfo failed".to_string(),
    };
    let heap_from = if unsafe { ffi::envHasHeapOverride() } {
        "set up by the homebrew loader"
    } else {
        "set up by libnx"
    };
    format!(
        "[Memory] applet type {applet_type} = {}\n\
         [Memory] process: {process}\n\
         [Memory] malloc heap: {:.1} MiB, {heap_from}; {}\n",
        applet_type_name(applet_type),
        mib(heap_size()),
        malloc_report(),
    )
}

/// How much of the malloc heap is used: `in use` is what Rust and C code have allocated; `taken`
/// also counts freed blocks malloc keeps for reuse.
pub(crate) fn malloc_report() -> String {
    MallocFigures::now().to_string()
}

/// The malloc heap at one moment, from newlib's `mallinfo` and NVK's top-down region (see
/// bevy-horizon's `memory.rs`; its Rust-allocator figures are left out, there is no
/// `top_down_alloc` yet).
pub(crate) struct MallocFigures {
    in_use: usize,
    taken: usize,
    heap: u64,
    top: u64,
    holes: usize,
    region: u64,
    region_used: u64,
    region_peak: u64,
    gpu_fallbacks: u64,
}

impl MallocFigures {
    pub(crate) fn now() -> Self {
        let info = unsafe { ffi::mallinfo() };
        let heap = heap_size();
        let region = top_down_region();
        let (mut extent, mut used, mut peak, mut gpu_fallbacks) = (0u64, 0u64, 0u64, 0u64);
        unsafe {
            ffi::nouveau_horizon_region_stats(&mut extent, &mut used, &mut peak, &mut gpu_fallbacks)
        };
        Self {
            in_use: info.uordblks,
            taken: info.arena,
            heap,
            top: heap
                .saturating_sub(region)
                .saturating_sub(info.arena as u64)
                + info.keepcost as u64,
            holes: info.fordblks.saturating_sub(info.keepcost),
            region,
            region_used: used,
            region_peak: peak,
            gpu_fallbacks,
        }
    }
}

impl core::fmt::Display for MallocFigures {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "malloc has {:.1} MiB in use, {:.1} MiB taken from the {:.1} MiB heap; {:.1} MiB free at the top, {:.1} MiB free in holes below it",
            mib(self.in_use as u64),
            mib(self.taken as u64),
            mib(self.heap),
            mib(self.top),
            mib(self.holes as u64),
        )?;
        if self.region != 0 {
            write!(
                f,
                "; the top-down region above the top: {:.1} MiB, {:.1} MiB of it in GPU blocks, {:.1} MiB at the most",
                mib(self.region),
                mib(self.region_used),
                mib(self.region_peak),
            )?;
        }
        if self.gpu_fallbacks != 0 {
            write!(
                f,
                "; {} GPU blocks the region couldn't place came from malloc",
                self.gpu_fallbacks
            )?;
        }
        Ok(())
    }
}

/// libnx `Perm_Rw` (switch/kernel/svc.h).
const PERM_RW: u32 = 3;
/// libnx `MemAttr_IsBorrowed` (switch/kernel/svc.h), the kernel's `KMemoryAttribute_Locked`.
const MEM_ATTR_LOCKED: u32 = 1 << 0;
/// The attributes the kernel ignores when it checks a non-secure IPC buffer, such as the one the
/// loader reads the next NRO into: `MemAttr_IsIpcMapped`, `MemAttr_IsDeviceMapped` and
/// `MemAttr_IsPermissionLocked` (Atmosphère `KPageTableBase::SetupForIpcClient`). Any other
/// attribute, `MemAttr_IsBorrowed` (locked) in particular, fails the read with 2001-0106.
const MEM_ATTRS_IPC_IGNORES: u32 = (1 << 1) | (1 << 2) | (1 << 4);
/// libnx `MemAttr_IsDeviceMapped` and `MemAttr_IsUncached`: heap pages an nvmap object still holds.
/// nvservices device-maps every buffer the Vulkan driver hands it (`svcMapDeviceAddressSpace*`),
/// and libnx's `nvMapCreate` marks an uncached one Uncached, which `nvMapClose` clears (libnx
/// `nvidia/map.c`). NVK takes all its GPU memory from this heap, most of it from the top
/// (bevy-horizon's `patches/mesa-switch/`), so such pages mean the driver hasn't shut down.
/// Ryujinx sets only Uncached.
const MEM_ATTRS_NVMAP: u32 = (1 << 2) | (1 << 3);
/// How many blocks `check_heap_for_exit` lists.
const LISTED: usize = 8;

/// A heap block the loader can't read into.
#[derive(Clone, Copy, Default)]
struct BadBlock {
    start: u64,
    end: u64,
    kind: u32,
    perm: u32,
    attr: u32,
}

/// What `scan_heap` found: how many blocks the loader can't read into, and the first few, and
/// how much of the heap nvmap objects hold.
#[derive(Default)]
struct HeapScan {
    blocks: usize,
    bytes: u64,
    first: [BadBlock; LISTED],
    nvmap_blocks: usize,
    nvmap_bytes: u64,
}

/// Walks the malloc heap with `svcQueryMemory`, looking for blocks the loader's read would fail
/// on: ones the process can't read and write, or with an attribute the kernel won't accept in an
/// IPC buffer. Fails with the address and result of a query that failed.
///
/// Allocates nothing, so it is safe at exit even if the exit is an abort because malloc failed.
fn scan_heap() -> Result<HeapScan, (u64, u32)> {
    use crate::rt::logging::ffi as log_ffi;

    let (start, end) = (unsafe { ffi::fake_heap_start as u64 }, heap_end());
    let mut scan = HeapScan::default();
    let mut addr = start;
    while addr < end {
        let mut info = log_ffi::MemoryInfo::default();
        let mut page_info = 0u32;
        let rc = unsafe { log_ffi::svcQueryMemory(&mut info, &mut page_info, addr) };
        let block_end = info.addr.saturating_add(info.size).min(end);
        if rc != 0 || block_end <= addr {
            return Err((addr, rc));
        }
        if (info.perm & PERM_RW) != PERM_RW || (info.attr & !MEM_ATTRS_IPC_IGNORES) != 0 {
            if scan.blocks < LISTED {
                scan.first[scan.blocks] = BadBlock {
                    start: addr,
                    end: block_end,
                    kind: info.kind,
                    perm: info.perm,
                    attr: info.attr,
                };
            }
            scan.blocks += 1;
            scan.bytes += block_end - addr;
        }
        if info.attr & MEM_ATTRS_NVMAP != 0 {
            scan.nvmap_blocks += 1;
            scan.nvmap_bytes += block_end - addr;
        }
        addr = block_end;
    }
    Ok(scan)
}

/// Checks at exit that the homebrew loader can read the next NRO into the malloc heap, and
/// writes the result to `log.txt`. Returns how many blocks it can't read into, or `None` if
/// `svcQueryMemory` failed.
///
/// The loader reads the next NRO into this heap, and the kernel only accepts a read buffer whose
/// pages the process can read and write and that aren't locked (Atmosphère
/// `KPageTableBase::SetupForIpcClient`). Transfer memory (`svcCreateTransferMemory`: sockets,
/// nv, audio) and thread stacks (`svcMapMemory`) leave their heap pages locked with no access
/// until they are closed.
pub(crate) fn check_heap_for_exit(when: &str) -> Option<usize> {
    use crate::rt::logging::write_log_line;

    let scan = match scan_heap() {
        Ok(scan) => scan,
        Err((addr, rc)) => {
            write_log_line(format_args!(
                "[Exit] heap check, {when}: svcQueryMemory({addr:#x}) failed with {}",
                crate::rt::logging::ResultCode(rc)
            ));
            return None;
        }
    };
    match scan.blocks {
        0 => write_log_line(format_args!(
            "[Exit] heap check, {when}: the loader can read into all {:.1} MiB of the malloc heap; every page is read/write and unlocked",
            mib(heap_size())
        )),
        1 => write_log_line(format_args!(
            "[Exit] heap check, {when}: 1 block, {:.2} MiB, would fail the loader's read:",
            mib(scan.bytes)
        )),
        blocks => write_log_line(format_args!(
            "[Exit] heap check, {when}: {blocks} blocks, {:.2} MiB in all, would fail the loader's read{}:",
            mib(scan.bytes),
            if blocks > LISTED {
                " (the first 8 are listed)"
            } else {
                ""
            }
        )),
    }
    for block in &scan.first[..scan.blocks.min(LISTED)] {
        let why = if block.attr & MEM_ATTR_LOCKED != 0 && block.perm == 0 {
            ": locked with no access, as an open transfer memory or a thread stack leaves it"
        } else {
            ""
        };
        write_log_line(format_args!(
            "[Exit]   {:#x}-{:#x}, {:.2} MiB: memory type {:#x}, permission {:#x}, attributes {:#x}{why}",
            block.start,
            block.end,
            mib(block.end - block.start),
            block.kind,
            block.perm,
            block.attr,
        ));
    }
    Some(scan.blocks)
}

/// How many heap blocks, and bytes, nvmap objects still hold: GPU memory the driver hasn't freed.
/// `None` if `svcQueryMemory` failed. Allocates nothing.
pub(crate) fn nvmap_heap() -> Option<(usize, u64)> {
    scan_heap()
        .ok()
        .map(|scan| (scan.nvmap_blocks, scan.nvmap_bytes))
}

/// Waits, checking every millisecond for up to `timeout`, until the loader can read into the
/// whole heap again, and returns how long that took. `None` means the heap was still blocked at
/// the timeout or `svcQueryMemory` failed.
///
/// Closing a service doesn't unlock its transfer memory at once: the pages stay locked until the
/// service's own process notices the closed session and lets go of its mapping. libnx's
/// `_nvCleanup` waits like this (`tmemWaitForPermission`); `_bsdCleanup`, behind `socketExit`,
/// doesn't.
pub(crate) fn wait_for_heap_unlock(timeout: std::time::Duration) -> Option<std::time::Duration> {
    let started = std::time::Instant::now();
    loop {
        match scan_heap() {
            Ok(scan) if scan.blocks == 0 => return Some(started.elapsed()),
            Ok(_) if started.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            _ => return None,
        }
    }
}
