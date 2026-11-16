//! Reserve an SMT core for the server and move cooperating workers off it.
//! Other threads and processes may still use that core. Reservations are planned
//! anew for each world. The game's own pool threads move themselves; on Linux a keeper
//! moves the threads that never chose their CPUs: the ones the server thread starts, which
//! inherit its mask, and the ones free to run anywhere (see `os::Keeper`). A thread that
//! picked its own mask is never touched.
//! `HORIZON_SERVER_CORE=0` disables this; dedicated servers require `=1`.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Fewer cores with SMT than this, and no core is reserved: on a 4-core/8-thread laptop the
/// workers would lose a quarter of their CPUs. Counting only cores with SMT also keeps a hybrid
/// CPU with two performance cores (Intel's E-cores have no sibling) from giving one of its two
/// fast cores to the server and leaving the render thread the other.
pub const MIN_SMT_CORES: usize = 6;

/// One physical core: its logical CPUs and how fast it is (higher is faster: the maximum
/// frequency on Linux, the efficiency class on Windows; equal where the system doesn't say).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Core {
    pub cpus: Vec<usize>,
    pub rank: u64,
}

/// Where the threads go: the server thread on `reserved` (one core's CPUs, the server free to use
/// either sibling so another program on one doesn't stall it), cooperating workers on `others`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub reserved: Vec<usize>,
    pub others: Vec<usize>,
}

/// A CPU list as the kernel writes it (`0,16`, `0-1`, `0-3,8-11`); what doesn't parse is skipped.
pub fn parse_cpu_list(text: &str) -> Vec<usize> {
    let mut cpus = BTreeSet::new();
    for part in text
        .trim()
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let range = match part.split_once('-') {
            Some((a, b)) => a.trim().parse().ok().zip(b.trim().parse().ok()),
            None => part.parse::<usize>().ok().map(|n| (n, n)),
        };
        if let Some((a, b)) = range {
            cpus.extend(a..=b.min(a.saturating_add(4096)));
        }
    }
    cpus.into_iter().collect()
}

/// Which core the server thread takes on a machine of `cores` when the game may use `allowed`:
/// the fastest core with SMT whose CPUs are all allowed (the lowest-numbered of equals), `None`
/// where reserving doesn't pay (see [`MIN_SMT_CORES`]) or nothing would be left for the others.
pub fn plan(cores: &[Core], allowed: &[usize]) -> Option<Plan> {
    let allowed: BTreeSet<usize> = allowed.iter().copied().collect();
    let candidates: Vec<&Core> = cores
        .iter()
        .filter(|c| c.cpus.len() >= 2 && c.cpus.iter().all(|cpu| allowed.contains(cpu)))
        .collect();
    if candidates.len() < MIN_SMT_CORES {
        return None;
    }
    let core = candidates
        .iter()
        .max_by_key(|c| (c.rank, std::cmp::Reverse(c.cpus.iter().min().copied())))?;
    let reserved: Vec<usize> = core.cpus.clone();
    let others: Vec<usize> = allowed
        .iter()
        .copied()
        .filter(|cpu| !reserved.contains(cpu))
        .collect();
    (!others.is_empty()).then_some(Plan { reserved, others })
}

/// `HORIZON_SERVER_CORE=0` (or `off`, `false`) turns the reservation off.
fn switched_off(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some("0" | "off" | "false"))
}

/// Hosted servers opt in explicitly because multiple processes can choose the same core.
pub fn dedicated_enabled() -> bool {
    dedicated_enabled_value(std::env::var("HORIZON_SERVER_CORE").ok().as_deref())
}

fn dedicated_enabled_value(value: Option<&str>) -> bool {
    value == Some("1")
}

// The lock serializes publication with self-applied changes. The generation makes the
// common worker path one relaxed load; workers reconcile when they next run a job.
static ACTIVE: Mutex<Option<Plan>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);
// How many CPUs the active plan was taken from (0: none).
static WHOLE: AtomicUsize = AtomicUsize::new(0);

/// Serializes the tests that pin threads or start a keeper (which moves every thread of the
/// test process that never chose its CPUs) with the tests that read a thread's CPUs.
#[cfg(test)]
pub(crate) static RESERVATION_TEST: Mutex<()> = Mutex::new(());

/// The CPUs the process may use while a reservation narrows its threads' own masks: the
/// server thread's is its core alone and the threads moved off it have the rest, so
/// `std::thread::available_parallelism` asked on any of them undercounts. `None` without one.
pub(crate) fn whole_set() -> Option<NonZeroUsize> {
    NonZeroUsize::new(WHOLE.load(Ordering::Relaxed))
}

fn whole_count(plan: &Plan) -> usize {
    plan.reserved
        .iter()
        .chain(&plan.others)
        .collect::<BTreeSet<_>>()
        .len()
}

struct WorkerState {
    generation: u64,
    previous: Option<os::State>,
}

impl Drop for WorkerState {
    fn drop(&mut self) {
        if let Some(state) = self.previous.take() {
            os::restore(&state);
        }
    }
}

thread_local! {
    static WORKER: std::cell::RefCell<WorkerState> = const { std::cell::RefCell::new(WorkerState { generation: u64::MAX, previous: None }) };
    static SERVER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A thread-bound reservation, released by the server thread itself.
pub struct ServerCore {
    plan: Plan,
    state: os::State,
    keeper: os::Keeper,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl ServerCore {
    /// The CPUs the server thread runs on.
    pub fn cpus(&self) -> &[usize] {
        &self.plan.reserved
    }
}

impl Drop for ServerCore {
    fn drop(&mut self) {
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        self.keeper.stop();
        os::restore(&self.state);
        SERVER.with(|server| server.set(false));
        WHOLE.store(0, Ordering::Relaxed);
        *active = None;
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Recompute the plan for each world, then pin only this thread.
pub fn take() -> Option<ServerCore> {
    if switched_off(std::env::var("HORIZON_SERVER_CORE").ok().as_deref()) {
        return None;
    }
    let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    if active.is_some() {
        return None;
    }
    let (cores, allowed) = os::topology()?;
    let plan = plan(&cores, &allowed)?;
    // A lazily initialized pool must not size itself from the server's temporary pin.
    let _ = crate::worker_count();
    let state = os::reserve(&plan)?;
    let keeper = os::Keeper::start(&plan);
    SERVER.with(|server| server.set(true));
    WHOLE.store(whole_count(&plan), Ordering::Relaxed);
    *active = Some(plan.clone());
    GENERATION.fetch_add(1, Ordering::Relaxed);
    Some(ServerCore {
        plan,
        state,
        keeper,
        _thread: std::marker::PhantomData,
    })
}

/// Reconcile this owned helper's selection with the current reservation.
pub fn leave_reserved() {
    let generation = GENERATION.load(Ordering::Relaxed);
    if SERVER.with(|server| server.get()) {
        return;
    }
    WORKER.with(|worker| {
        let mut worker = worker.borrow_mut();
        if worker.generation == generation {
            return;
        }
        let active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = worker.previous.take() {
            os::restore(&previous);
        }
        worker.previous = active.as_ref().and_then(os::leave);
        worker.generation = GENERATION.load(Ordering::Relaxed);
    });
}

/// Remove reserved CPUs only when at least one previously allowed CPU remains.
#[cfg(any(target_os = "linux", windows, test))]
fn narrowed(previous: &[usize], reserved: &[usize]) -> Option<Vec<usize>> {
    let remaining: Vec<_> = previous
        .iter()
        .copied()
        .filter(|cpu| !reserved.contains(cpu))
        .collect();
    (!remaining.is_empty() && remaining.len() != previous.len()).then_some(remaining)
}

#[cfg(target_os = "linux")]
mod os {
    use super::{Core, Plan};
    use std::collections::BTreeSet;
    use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
    use std::thread::JoinHandle;
    use std::time::Duration;
    pub struct State {
        original: Vec<usize>,
    }

    fn read(path: String) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    pub fn affinity(tid: i32) -> Option<Vec<usize>> {
        // SAFETY: a zeroed cpu_set_t is the empty set; the size passed is its own.
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            if libc::sched_getaffinity(tid, std::mem::size_of::<libc::cpu_set_t>(), &mut set) != 0 {
                return None;
            }
            Some(
                (0..libc::CPU_SETSIZE as usize)
                    .filter(|&cpu| libc::CPU_ISSET(cpu, &set))
                    .collect(),
            )
        }
    }

    pub fn set_affinity(tid: i32, cpus: &[usize]) -> bool {
        // SAFETY: as in `affinity`; CPU_SET ignores nothing in range, and every id is checked.
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            for &cpu in cpus.iter().filter(|&&c| c < libc::CPU_SETSIZE as usize) {
                libc::CPU_SET(cpu, &mut set);
            }
            libc::sched_setaffinity(tid, std::mem::size_of::<libc::cpu_set_t>(), &set) == 0
        }
    }

    /// The cores of the CPUs this thread may use, from sysfs, and those CPUs.
    pub fn topology() -> Option<(Vec<Core>, Vec<usize>)> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let allowed = super::parse_cpu_list(
            status
                .lines()
                .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))?,
        );
        let mut cores: Vec<Core> = Vec::new();
        for &cpu in &allowed {
            let base = format!("/sys/devices/system/cpu/cpu{cpu}");
            let cpus = read(format!("{base}/topology/thread_siblings_list"))
                .map(|s| super::parse_cpu_list(&s))
                .filter(|cpus| cpus.contains(&cpu))
                .unwrap_or_else(|| vec![cpu]);
            if cores.iter().any(|c| c.cpus == cpus) {
                continue;
            }
            // AMD's preferred cores and Intel's Turbo Boost Max cores report a higher maximum.
            let rank = read(format!("{base}/cpufreq/cpuinfo_max_freq"))
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            cores.push(Core { cpus, rank });
        }
        Some((cores, allowed))
    }

    pub fn reserve(plan: &Plan) -> Option<State> {
        let original = affinity(0)?;
        if !plan.reserved.iter().all(|cpu| original.contains(cpu))
            || !set_affinity(0, &plan.reserved)
        {
            return None;
        }
        Some(State { original })
    }

    pub fn leave(plan: &Plan) -> Option<State> {
        let mut original = affinity(0)?;
        // A thread the pinned server thread started (a tick's parallel helpers, a pool it was the
        // first to use) inherited only the reserved CPUs, as Linux threads start with their
        // creator's mask. What it would have had without the reservation is the allowed set the
        // plan was made from: move it to the others and give that set back on release.
        if original.iter().all(|cpu| plan.reserved.contains(cpu)) {
            original = plan
                .others
                .iter()
                .chain(&plan.reserved)
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
        }
        let remaining = super::narrowed(&original, &plan.reserved)?;
        set_affinity(0, &remaining).then_some(State { original })
    }

    pub fn restore(state: &State) {
        set_affinity(0, &state.original);
    }

    fn threads() -> Vec<i32> {
        std::fs::read_dir("/proc/self/task")
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
            .collect()
    }

    /// Two kinds of thread would undo the reservation, measured on a 16-core/32-thread laptop:
    /// - threads the pinned server thread starts (chunk encoders, save compressors, a network
    ///   thread...) begin on its CPUs only, as Linux threads inherit their creator's mask: with
    ///   31 encoders and 16 compressors left there, ticks took three times as long;
    /// - threads free to run anywhere (the render, main and network threads) drift onto the
    ///   reserved core once the workers have left it, as it is then the machine's idlest:
    ///   one run in three took three times as long, the other two gained a third of the win.
    ///
    /// Once a second the keeper moves every thread whose mask is exactly the reserved CPUs or
    /// exactly the whole allowed set (masks nobody chose but by inheriting or never choosing)
    /// to the others; a thread that chose any other mask is left alone. When the world closes it
    /// gives every thread still on exactly the others the whole allowed set back: the ones it
    /// moved, and the ones a moved thread started meanwhile, which inherited the others.
    pub struct Keeper {
        stop: Option<Sender<()>>,
        thread: Option<JoinHandle<()>>,
        others: Vec<usize>,
        allowed: Vec<usize>,
    }

    impl Keeper {
        pub fn start(plan: &Plan) -> Keeper {
            // SAFETY: gettid takes nothing and can't fail.
            let server = unsafe { libc::gettid() };
            let (stop, stopped) = channel::<()>();
            let allowed: Vec<usize> = plan
                .others
                .iter()
                .chain(&plan.reserved)
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let (reserved, others, whole) =
                (plan.reserved.clone(), plan.others.clone(), allowed.clone());
            let thread = std::thread::Builder::new()
                .name("Server core keeper".into())
                .spawn(move || {
                    set_affinity(0, &others);
                    loop {
                        for tid in threads().into_iter().filter(|&t| t != server) {
                            let mask = affinity(tid);
                            if mask.as_deref() == Some(&reserved[..])
                                || mask.as_deref() == Some(&whole[..])
                            {
                                set_affinity(tid, &others);
                            }
                        }
                        if !matches!(
                            stopped.recv_timeout(Duration::from_secs(1)),
                            Err(RecvTimeoutError::Timeout)
                        ) {
                            return;
                        }
                    }
                })
                .ok();
            Keeper {
                stop: Some(stop),
                thread,
                others: plan.others.clone(),
                allowed,
            }
        }

        pub fn stop(&mut self) {
            self.stop.take();
            if self.thread.take().is_none_or(|t| t.join().is_err()) {
                return;
            }
            for tid in threads() {
                if affinity(tid).as_deref() == Some(&self.others[..]) {
                    set_affinity(tid, &self.allowed);
                }
            }
        }
    }
}

#[cfg(windows)]
mod os {
    use super::{Core, Plan};
    use std::sync::Mutex;
    use windows_sys::Win32::System::SystemInformation::{
        CpuSetInformation, GetSystemCpuSetInformation, SYSTEM_CPU_SET_INFORMATION,
    };
    use windows_sys::Win32::System::Threading::{
        GetActiveProcessorGroupCount, GetCurrentProcess, GetCurrentThread, GetProcessAffinityMask,
        GetProcessDefaultCpuSets, GetThreadGroupAffinity, GetThreadSelectedCpuSets,
        SetThreadSelectedCpuSets,
    };

    /// Logical processor index → CPU set id, for the machine's one processor group.
    static IDS: Mutex<Vec<(usize, u32)>> = Mutex::new(Vec::new());
    pub struct State {
        previous: Vec<u32>,
    }

    fn ids(cpus: &[usize]) -> Vec<u32> {
        let ids = IDS.lock().unwrap_or_else(|e| e.into_inner());
        cpus.iter()
            .filter_map(|cpu| ids.iter().find(|(c, _)| c == cpu).map(|&(_, id)| id))
            .collect()
    }

    /// The cores from the system's CPU sets (siblings share a core index) and the logical
    /// processors the process's affinity mask allows. `None` past 64 CPUs (more than one
    /// processor group), which this keeps out of.
    pub fn topology() -> Option<(Vec<Core>, Vec<usize>)> {
        // SAFETY: plain calls; the out pointers are live locals, the buffer is as long as said,
        // and each record is read within the length the system returned, by its own size.
        unsafe {
            if GetActiveProcessorGroupCount() != 1 {
                return None;
            }
            let mut count = 0;
            if GetProcessDefaultCpuSets(GetCurrentProcess(), std::ptr::null_mut(), 0, &mut count)
                == 0
                || count != 0
            {
                return None;
            }
            let (mut process_mask, mut system_mask) = (0usize, 0usize);
            if GetProcessAffinityMask(GetCurrentProcess(), &mut process_mask, &mut system_mask) == 0
            {
                return None;
            }
            let mut length = 0u32;
            GetSystemCpuSetInformation(
                std::ptr::null_mut(),
                0,
                &mut length,
                GetCurrentProcess(),
                0,
            );
            if length == 0 {
                return None;
            }
            let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
            if GetSystemCpuSetInformation(
                buffer.as_mut_ptr().cast(),
                length,
                &mut length,
                GetCurrentProcess(),
                0,
            ) == 0
            {
                return None;
            }
            if length as usize > buffer.len() * std::mem::size_of::<u64>() {
                return None;
            }
            let bytes = buffer.as_ptr().cast::<u8>();
            let (mut offset, mut cores, mut allowed) = (0usize, Vec::<(u8, Core)>::new(), vec![]);
            let mut ids = Vec::new();
            while offset + std::mem::size_of::<u32>() * 2 <= length as usize {
                let size = bytes.add(offset).cast::<u32>().read_unaligned() as usize;
                if size < std::mem::size_of::<u32>() * 2 || size > length as usize - offset {
                    break;
                }
                let kind = bytes.add(offset + 4).cast::<i32>().read_unaligned();
                if kind == CpuSetInformation
                    && size >= std::mem::size_of::<SYSTEM_CPU_SET_INFORMATION>()
                {
                    let info = bytes
                        .add(offset)
                        .cast::<SYSTEM_CPU_SET_INFORMATION>()
                        .read_unaligned();
                    let set = info.Anonymous.CpuSet;
                    let cpu = set.LogicalProcessorIndex as usize;
                    if set.Group != 0 || cpu >= usize::BITS as usize {
                        return None;
                    }
                    ids.push((cpu, set.Id));
                    if process_mask & (1usize << cpu) != 0 {
                        allowed.push(cpu);
                    }
                    match cores.iter_mut().find(|(index, _)| *index == set.CoreIndex) {
                        Some((_, core)) => core.cpus.push(cpu),
                        None => cores.push((
                            set.CoreIndex,
                            Core {
                                cpus: vec![cpu],
                                rank: set.EfficiencyClass as u64,
                            },
                        )),
                    }
                }
                offset += size;
            }
            *IDS.lock().unwrap_or_else(|e| e.into_inner()) = ids;
            allowed.sort_unstable();
            Some((cores.into_iter().map(|(_, c)| c).collect(), allowed))
        }
    }

    fn selection() -> Option<Vec<u32>> {
        unsafe {
            let mut count = 0;
            // A non-empty selection is foreign: leave it untouched.
            if GetThreadSelectedCpuSets(GetCurrentThread(), std::ptr::null_mut(), 0, &mut count)
                == 0
                || count != 0
            {
                return None;
            }
            Some(Vec::new())
        }
    }

    fn select(cpus: &[usize]) -> Option<State> {
        let previous = selection()?;
        let selected = ids(cpus);
        if selected.len() != cpus.len() || selected.is_empty() {
            return None;
        }
        unsafe {
            (SetThreadSelectedCpuSets(GetCurrentThread(), selected.as_ptr(), selected.len() as u32)
                != 0)
                .then_some(State { previous })
        }
    }
    fn thread_mask() -> Option<Vec<usize>> {
        // SAFETY: the output is initialized and belongs to this thread.
        unsafe {
            let mut group = std::mem::zeroed();
            if GetThreadGroupAffinity(GetCurrentThread(), &mut group) == 0 || group.Group != 0 {
                return None;
            }
            Some(
                (0..usize::BITS as usize)
                    .filter(|cpu| group.Mask & (1usize << cpu) != 0)
                    .collect(),
            )
        }
    }
    pub fn reserve(plan: &Plan) -> Option<State> {
        let mask = thread_mask()?;
        if !plan.reserved.iter().all(|cpu| mask.contains(cpu)) {
            return None;
        }
        select(&plan.reserved)
    }
    pub fn leave(plan: &Plan) -> Option<State> {
        let mask = thread_mask()?;
        select(&super::narrowed(&mask, &plan.reserved)?)
    }
    /// A Windows thread doesn't inherit its creator's CPU set, so nothing needs moving back.
    pub struct Keeper;
    impl Keeper {
        pub fn start(_plan: &Plan) -> Keeper {
            Keeper
        }
        pub fn stop(&mut self) {}
    }

    pub fn restore(state: &State) {
        // SAFETY: this is the calling thread, and the selection was saved by us.
        unsafe {
            let ptr = if state.previous.is_empty() {
                std::ptr::null()
            } else {
                state.previous.as_ptr()
            };
            SetThreadSelectedCpuSets(GetCurrentThread(), ptr, state.previous.len() as u32);
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod os {
    use super::{Core, Plan};

    pub struct State;

    /// No hard affinity here (macOS's affinity tags are hints the scheduler may ignore).
    pub fn topology() -> Option<(Vec<Core>, Vec<usize>)> {
        None
    }
    pub fn reserve(_plan: &Plan) -> Option<State> {
        None
    }
    pub fn leave(_plan: &Plan) -> Option<State> {
        None
    }
    pub fn restore(_state: &State) {}

    pub struct Keeper;
    impl Keeper {
        pub fn start(_plan: &Plan) -> Keeper {
            Keeper
        }
        pub fn stop(&mut self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `physical` cores of `smt` CPUs each, numbered as Linux does (core n's CPUs n, n +
    /// physical...), all equally fast.
    fn machine(physical: usize, smt: usize) -> Vec<Core> {
        (0..physical)
            .map(|n| Core {
                cpus: (0..smt).map(|t| n + t * physical).collect(),
                rank: 0,
            })
            .collect()
    }

    fn all(cores: &[Core]) -> Vec<usize> {
        let mut cpus: Vec<usize> = cores.iter().flat_map(|c| c.cpus.clone()).collect();
        cpus.sort_unstable();
        cpus
    }

    #[test]
    fn cpu_lists_read_as_the_kernel_writes_them() {
        assert_eq!(parse_cpu_list("0,16\n"), vec![0, 16]);
        assert_eq!(parse_cpu_list("0-1"), vec![0, 1]);
        assert_eq!(parse_cpu_list("0-3,8-11"), vec![0, 1, 2, 3, 8, 9, 10, 11]);
        assert_eq!(parse_cpu_list("5"), vec![5]);
        assert_eq!(parse_cpu_list(""), Vec::<usize>::new());
        assert_eq!(parse_cpu_list("x,2"), vec![2]);
        assert_eq!(parse_cpu_list(&usize::MAX.to_string()), vec![usize::MAX]);
    }

    #[test]
    fn dedicated_servers_require_explicit_opt_in() {
        assert!(!dedicated_enabled_value(None));
        assert!(!dedicated_enabled_value(Some("0")));
        assert!(!dedicated_enabled_value(Some("true")));
        assert!(dedicated_enabled_value(Some("1")));
    }

    #[test]
    fn sixteen_cores_two_ccds_reserve_a_core_and_its_sibling() {
        let cores = machine(16, 2);
        let plan = plan(&cores, &all(&cores)).unwrap();
        assert_eq!(plan.reserved, vec![0, 16]);
        assert_eq!(plan.others.len(), 30);
        assert!(!plan.others.contains(&0) && !plan.others.contains(&16));
    }

    #[test]
    fn eight_cores_reserve_one() {
        let cores = machine(8, 2);
        let plan = plan(&cores, &all(&cores)).unwrap();
        assert_eq!(plan.reserved, vec![0, 8]);
        assert_eq!(plan.others, (1..8).chain(9..16).collect::<Vec<_>>());
    }

    #[test]
    fn four_cores_and_no_smt_reserve_nothing() {
        let cores = machine(4, 2);
        assert_eq!(plan(&cores, &all(&cores)), None);
        let cores = machine(16, 1);
        assert_eq!(plan(&cores, &all(&cores)), None);
    }

    #[test]
    fn an_affinity_mask_is_worked_inside() {
        let cores = machine(16, 2);
        // CPU 0 taken away by the player: core 0 is out, core 1 (1 and 17) is the first whole one.
        let allowed: Vec<usize> = (1..32).collect();
        let plan = plan(&cores, &allowed).unwrap();
        assert_eq!(plan.reserved, vec![1, 17]);
        assert_eq!(plan.others.len(), 29);
        assert!(plan.others.iter().all(|cpu| allowed.contains(cpu)));
        // Limited to the first CCD's 8 cores and their siblings: still enough.
        let ccd: Vec<usize> = (0..8).chain(16..24).collect();
        assert_eq!(super::plan(&cores, &ccd).unwrap().others.len(), 14);
        // Limited to 4 cores: not enough left to give one away.
        let four: Vec<usize> = (0..4).chain(16..20).collect();
        assert_eq!(super::plan(&cores, &four), None);
        // One CPU per core (siblings excluded): no SMT inside the mask.
        assert_eq!(super::plan(&cores, &(0..16).collect::<Vec<_>>()), None);
    }

    #[test]
    fn a_hybrid_cpu_reserves_its_fastest_core_with_smt() {
        // 6 performance cores with SMT (CPUs 0-11, sibling pairs), 8 efficiency cores without
        // (12-19); performance core 2 boosts highest.
        let mut cores: Vec<Core> = (0..6)
            .map(|n| Core {
                cpus: vec![2 * n, 2 * n + 1],
                rank: if n == 2 { 5_000_000 } else { 4_700_000 },
            })
            .collect();
        cores.extend((12..20).map(|cpu| Core {
            cpus: vec![cpu],
            rank: 3_500_000,
        }));
        let plan = plan(&cores, &all(&cores)).unwrap();
        assert_eq!(plan.reserved, vec![4, 5]);
        assert_eq!(plan.others.len(), 18);
        // Two performance cores and eight efficiency cores: no reservation.
        let small: Vec<Core> = cores[..2].iter().chain(&cores[6..]).cloned().collect();
        assert_eq!(super::plan(&small, &all(&small)), None);
    }

    #[test]
    fn narrowing_never_widens_and_skips_an_empty_result() {
        assert_eq!(narrowed(&[0, 2, 4], &[0, 1]), Some(vec![2, 4]));
        assert_eq!(narrowed(&[0], &[0, 1]), None);
        assert_eq!(narrowed(&[2], &[0, 1]), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn worker_reconciles_each_generation() {
        use std::sync::mpsc::channel;
        let _test = RESERVATION_TEST.lock().unwrap();
        // The count is kept from its first ask: make that before this test narrows any thread.
        let _ = crate::worker_count();
        let original = os::affinity(0).unwrap();
        if original.len() < 3 {
            return;
        }
        let (go, receive) = channel::<()>();
        let (send, result) = channel();
        let thread = std::thread::spawn(move || {
            for _ in 0..4 {
                receive.recv().unwrap();
                leave_reserved();
                send.send(os::affinity(0).unwrap()).unwrap();
            }
        });
        let publish = |reserved: Option<usize>| {
            let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            *active = reserved.map(|cpu| Plan {
                reserved: vec![cpu],
                others: original.iter().copied().filter(|c| *c != cpu).collect(),
            });
            GENERATION.fetch_add(1, Ordering::Relaxed);
        };
        publish(Some(original[0]));
        go.send(()).unwrap();
        assert_eq!(result.recv().unwrap(), original[1..]);
        go.send(()).unwrap();
        assert_eq!(result.recv().unwrap(), original[1..]);
        publish(Some(original[1]));
        go.send(()).unwrap();
        assert_eq!(
            result.recv().unwrap(),
            original
                .iter()
                .copied()
                .filter(|c| *c != original[1])
                .collect::<Vec<_>>()
        );
        publish(None);
        go.send(()).unwrap();
        assert_eq!(result.recv().unwrap(), original);
        thread.join().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_helper_started_by_the_pinned_server_leaves_its_core() {
        let _test = RESERVATION_TEST.lock().unwrap();
        // The count is kept from its first ask: make that before this test narrows any thread.
        let _ = crate::worker_count();
        let original = os::affinity(0).unwrap();
        if original.len() < 3 {
            return;
        }
        let plan = Plan {
            reserved: vec![original[0]],
            others: original[1..].to_vec(),
        };
        let server_plan = plan.clone();
        let (helper_while, helper_after) = std::thread::spawn(move || {
            let state = os::reserve(&server_plan).unwrap();
            *ACTIVE.lock().unwrap() = Some(server_plan.clone());
            GENERATION.fetch_add(1, Ordering::Relaxed);
            // Started on the pinned thread, so it begins with the reserved CPUs only.
            let (release, released) = std::sync::mpsc::channel::<()>();
            let helper = std::thread::spawn(move || {
                leave_reserved();
                let during = os::affinity(0).unwrap();
                released.recv().unwrap();
                leave_reserved();
                (during, os::affinity(0).unwrap())
            });
            std::thread::sleep(std::time::Duration::from_millis(50));
            *ACTIVE.lock().unwrap() = None;
            GENERATION.fetch_add(1, Ordering::Relaxed);
            release.send(()).unwrap();
            let result = helper.join().unwrap();
            os::restore(&state);
            result
        })
        .join()
        .unwrap();
        assert_eq!(helper_while, plan.others);
        assert_eq!(helper_after, original);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_keeper_moves_threads_that_never_chose_their_cpus() {
        use std::sync::mpsc::{Receiver, Sender, channel};
        // A thread that never moves itself and reports its mask when asked; `own` is a mask it
        // picks for itself first.
        fn probe(own: Option<Vec<usize>>) -> (Sender<()>, Receiver<Vec<usize>>) {
            let (ask, asked) = channel::<()>();
            let (answer, answers) = channel();
            std::thread::spawn(move || {
                if let Some(own) = own {
                    os::set_affinity(0, &own);
                }
                while asked.recv().is_ok() {
                    answer.send(os::affinity(0).unwrap()).unwrap();
                }
            });
            (ask, answers)
        }
        fn mask(probe: &(Sender<()>, Receiver<Vec<usize>>)) -> Vec<usize> {
            probe.0.send(()).unwrap();
            probe.1.recv().unwrap()
        }
        let _test = RESERVATION_TEST.lock().unwrap();
        // The count is kept from its first ask: make that before this test narrows any thread.
        let _ = crate::worker_count();
        let original = os::affinity(0).unwrap();
        if original.len() < 3 {
            return;
        }
        let plan = Plan {
            reserved: vec![original[0]],
            others: original[1..].to_vec(),
        };
        let chosen_mask = original[..2].to_vec();
        let server_plan = plan.clone();
        let seen = std::thread::spawn(move || {
            let free = probe(None);
            let chosen = probe(Some(chosen_mask));
            mask(&chosen);
            let state = os::reserve(&server_plan).unwrap();
            let mut keeper = os::Keeper::start(&server_plan);
            let inherited = probe(None);
            // Started by a thread the keeper had moved: it inherits the others.
            let born_moved = probe(Some(server_plan.others.clone()));
            let at_start = [mask(&inherited), mask(&free), mask(&chosen)];
            std::thread::sleep(std::time::Duration::from_millis(2500));
            let during = [mask(&inherited), mask(&free), mask(&chosen)];
            keeper.stop();
            let after = [
                mask(&inherited),
                mask(&free),
                mask(&chosen),
                mask(&born_moved),
            ];
            os::restore(&state);
            (at_start, during, after)
        })
        .join()
        .unwrap();
        let chosen = original[..2].to_vec();
        assert_eq!(
            seen.0,
            [plan.reserved.clone(), original.clone(), chosen.clone()]
        );
        assert_eq!(
            seen.1,
            [plan.others.clone(), plan.others.clone(), chosen.clone()]
        );
        assert_eq!(
            seen.2,
            [original.clone(), original.clone(), chosen, original.clone()]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reservation_restores_the_calling_thread() {
        let _test = RESERVATION_TEST.lock().unwrap();
        // The count is kept from its first ask: make that before this test narrows any thread.
        let _ = crate::worker_count();
        std::thread::spawn(|| {
            let original = os::affinity(0).unwrap();
            if original.len() < 2 {
                return;
            }
            let plan = Plan {
                reserved: vec![original[0]],
                others: original[1..].to_vec(),
            };
            let state = os::reserve(&plan).unwrap();
            assert_eq!(os::affinity(0).unwrap(), plan.reserved);
            *ACTIVE.lock().unwrap() = Some(plan.clone());
            SERVER.with(|server| server.set(true));
            let keeper = os::Keeper::start(&plan);
            let guard = ServerCore {
                plan,
                state,
                keeper,
                _thread: std::marker::PhantomData,
            };
            drop(guard);
            assert_eq!(os::affinity(0).unwrap(), original);
            let missing = Plan {
                reserved: vec![libc::CPU_SETSIZE as usize],
                others: original.clone(),
            };
            assert!(os::reserve(&missing).is_none());
        })
        .join()
        .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pools_sized_while_reserved_count_the_whole_set() {
        let _test = RESERVATION_TEST.lock().unwrap();
        // The count is kept from its first ask: make that before this test narrows any thread.
        let _ = crate::worker_count();
        std::thread::spawn(|| {
            let original = os::affinity(0).unwrap();
            if original.len() < 3 {
                return;
            }
            let plan = Plan {
                reserved: vec![original[0]],
                others: original[1..].to_vec(),
            };
            let state = os::reserve(&plan).unwrap();
            WHOLE.store(whole_count(&plan), Ordering::Relaxed);
            let on_server = crate::available_parallelism().unwrap().get();
            let moved = std::thread::spawn({
                let others = plan.others.clone();
                move || {
                    os::set_affinity(0, &others);
                    crate::available_parallelism().unwrap().get()
                }
            })
            .join()
            .unwrap();
            WHOLE.store(0, Ordering::Relaxed);
            let after = crate::available_parallelism().unwrap().get();
            os::restore(&state);
            assert_eq!(
                std::thread::available_parallelism().unwrap().get(),
                original.len()
            );
            assert_eq!(
                (on_server, moved, after),
                (original.len(), original.len(), 1)
            );
        })
        .join()
        .unwrap();
    }
}
