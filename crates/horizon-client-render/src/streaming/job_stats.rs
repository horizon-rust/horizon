use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
pub const KINDS: [&str; 7] = [
    "load", "generate", "decorate", "finish", "light", "mesh", "other",
];
/// The slot of `KINDS` the mesh job holds: the one the admission check prices.
pub(super) const MESH_KIND: usize = 5;
/// The slot of the jobs that are counted rather than timed: with statistics off
/// only the mesh job reads the clock, and these (saves, light views, freed
/// meshes) are many and short, so the two reads would cost more than they do.
pub(super) const OTHER_KIND: usize = 6;
/// Samples the mesh estimate needs before the admission check trusts it: the
/// first seeds it and the second corrects the seed, so the check prices nothing
/// for the first two jobs rather than for the first tick. Four samples arrived
/// only at the end of that tick, and the first tick is exactly the moment of
/// entering the world the check is for. Until it has warmed up the check prices
/// nothing: the behaviour before #18239.
const MESH_WARMUP: u64 = 2;
/// The estimate's step: an eighth of the way to a sample, so twenty samples
/// follow a step in cost to within a few percent.
const MESH_EWMA_SHIFT: u32 = 3;
static ENABLED: AtomicBool = AtomicBool::new(false);
static NANOS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
static COUNT: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
static MESH_EWMA: AtomicU64 = AtomicU64::new(0);
static MESH_SAMPLES: AtomicU64 = AtomicU64::new(0);
pub fn enable(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}
pub(super) fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}
pub(super) fn add(kind: usize, nanos: u64) {
    NANOS[kind].fetch_add(nanos, Ordering::Relaxed);
    COUNT[kind].fetch_add(1, Ordering::Relaxed);
}
/// (kind, nanoseconds, count) since the last call; resets them.
pub fn take() -> Vec<(&'static str, u64, u64)> {
    (0..KINDS.len())
        .map(|i| {
            (
                KINDS[i],
                NANOS[i].swap(0, Ordering::Relaxed),
                COUNT[i].swap(0, Ordering::Relaxed),
            )
        })
        .collect()
}
/// One estimate step: an eighth of the way from `prev` to `sample`, or the
/// sample itself when there is none to step from yet. In i64 so a cheaper sample
/// pulls the estimate down as fast as a dearer one lifts it; u64's shift of a
/// subtraction could only ever grow it.
fn ewma(prev: u64, sample: u64, samples: u64) -> u64 {
    if samples == 0 {
        return sample;
    }
    let (prev, sample) = (prev as i64, sample as i64);
    (prev + ((sample - prev) >> MESH_EWMA_SHIFT)).max(0) as u64
}
/// Feed one finished mesh job's cost to the estimate. Every worker calls this,
/// so it is two relaxed atomics and a sample lost to a race costs one step of
/// 1/8 — the estimate is a hint for the admission check, not a measurement.
pub(super) fn observe_mesh(nanos: u64) {
    let samples = MESH_SAMPLES.load(Ordering::Relaxed);
    let prev = MESH_EWMA.load(Ordering::Relaxed);
    MESH_EWMA.store(ewma(prev, nanos, samples), Ordering::Relaxed);
    MESH_SAMPLES.fetch_add(1, Ordering::Relaxed);
}
/// The estimate as the admission check reads it: `None` until a few mesh jobs
/// have priced it, and `None` at zero, which prices nothing.
fn ready(samples: u64, cost: u64) -> Option<u64> {
    (samples >= MESH_WARMUP && cost > 0).then_some(cost)
}
/// What one mesh job costs, once the estimate has warmed up; `None` until then.
/// `None` too where the platform has no thread CPU clock (`spans::thread_cpu`):
/// every job then measures zero, the estimate stays cold and the admission check
/// prices nothing — the behaviour before #18239, kept as that platform's
/// fallback.
pub(super) fn mesh_cost_ns() -> Option<u64> {
    ready(
        MESH_SAMPLES.load(Ordering::Relaxed),
        MESH_EWMA.load(Ordering::Relaxed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The estimate follows the cost both ways: the first sample seeds it rather
    /// than creeping up from zero an eighth at a time, and twenty samples follow
    /// a step in cost to within a few percent. It is priced only once a few
    /// samples have arrived, and never at zero.
    #[test]
    fn the_mesh_estimate_follows_the_cost_it_is_fed() {
        assert_eq!(ewma(0, 30_000_000, 0), 30_000_000);
        assert!(ewma(0, 30_000_000, 1) < 30_000_000);
        // Seeded at 5 ms by a first, cheap job, then twenty 30 ms ones.
        let mut cost = ewma(0, 5_000_000, 0);
        for _ in 0..20 {
            cost = ewma(cost, 30_000_000, 1);
        }
        assert!((25_000_000..=30_000_000).contains(&cost), "{cost}");
        // A cheaper job pulls the estimate down as fast as a dearer lifts it.
        assert!(ewma(cost, 10_000_000, 1) < cost);
        assert!(ewma(cost, 60_000_000, 1) > cost);
        // Nothing is priced until a few samples have priced it: a statistic that
        // has seen nothing bounds nothing, and the admission check then prices
        // nothing (`fits`). Zero stays unpriced however many samples arrived —
        // the platform with no thread CPU clock measures nothing — and one sample
        // short of the warm-up is still cold.
        assert_eq!(
            ready(0, 0),
            None,
            "a statistic that has seen nothing bounds nothing"
        );
        assert_eq!(
            ready(MESH_WARMUP, 0),
            None,
            "a zero-cost estimate prices nothing"
        );
        assert_eq!(ready(MESH_WARMUP - 1, 30_000_000), None, "still cold");
        assert_eq!(ready(MESH_WARMUP, 30_000_000), Some(30_000_000));
    }
}
