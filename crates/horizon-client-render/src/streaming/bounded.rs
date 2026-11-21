//! Bounded submission: the light and mesh stages hand the workers only the
//! nearest of what is ready.

/// The nearest `room` of the `candidates`, ordered: the light and mesh
/// stages submit bounded work, chosen from everything that is ready each
/// `update`, so what runs is always the nearest of what is left — chosen
/// again, from the current centre, as capacity frees, the way vanilla's
/// chunk task queues follow the current ticket levels. The selection is
/// O(n); the sort orders only what is submitted.
pub(super) fn nearest_first<T: Ord>(candidates: &mut Vec<T>, room: usize) {
    if candidates.len() > room {
        candidates.select_nth_unstable(room);
        candidates.truncate(room);
    }
    candidates.sort_unstable();
}

/// The mesh time one worker's columns may take: half a 20 Hz tick (Minecraft's
/// 50 ms), so the pool holds a few heavy columns in flight rather than every idle
/// worker's at once, and a heavy column's tail does not run under the next tick's
/// light and generate jobs. With seven workers that is 175 ms of meshing in
/// flight: five 30 ms sections at a time instead of seven, and 175 one-millisecond
/// sections against the room check's 224, so light work is barely touched. A
/// quarter of a tick left two heavy sections on a quarter of the pool, which the
/// first-chunk and fresh-world timings (#18196, #18192) would show; a whole tick
/// would leave this all but the room check it was. The number is the one tuning
/// winch here, and a measurement decides it.
pub(super) const MESH_SLICE_NANOS: u64 = 25_000_000;

/// Whether one more column of `cost` fits what is left of the mesh slice (issue
/// #18239). `left` is the slice for all workers less what the columns in flight
/// already cost — `None` until the mesh cost estimate has warmed up, when nothing
/// is priced and this says yes, exactly the room check before it. `start_anyway`
/// is a pool with nothing in flight: one column dearer than the whole slice would
/// otherwise wait for room it can never have, and the queue would never drain.
pub(super) fn fits(left: Option<u64>, cost: u64, start_anyway: bool) -> bool {
    left.is_none_or(|left| start_anyway || cost <= left)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The light and mesh stages submit bounded work, the nearest first:
    /// what exceeds the room stays dirty, to be chosen again — from the
    /// current centre — as capacity frees. Submitted whole, the queue once
    /// held every ready column's jobs and grew with the render distance
    /// (#3629); ties break by position, so the choice is deterministic.
    #[test]
    fn bounded_submission_takes_the_nearest_first() {
        let candidates: Vec<(i64, i32, i32, u32)> = (0..40i32)
            .map(|i| {
                let (x, z) = (i / 8 - 2, i % 8 - 3);
                (
                    i64::from((x - 3).pow(2) + (z - 2).pow(2)),
                    x,
                    z,
                    1 << (i % 24),
                )
            })
            .collect();
        let mut expected = candidates.clone();
        expected.sort_unstable();
        let mut none = candidates.clone();
        nearest_first(&mut none, 0);
        assert!(none.is_empty());
        let mut nearest = candidates;
        nearest_first(&mut nearest, 5);
        assert_eq!(nearest, expected[..5]);
        let mut past = expected.clone();
        nearest_first(&mut past, expected.len() + 10);
        assert_eq!(past, expected);
    }

    /// The slice prices what is in flight: a column fits what is left of it, a
    /// cold estimate prices nothing, and the first column into an empty pool goes
    /// in whatever it costs.
    #[test]
    fn a_column_fits_what_is_left_of_the_slice() {
        assert!(
            fits(None, u64::MAX / 2, false),
            "cold statistics price nothing"
        );
        assert!(
            fits(Some(0), u64::MAX / 2, true),
            "an empty pool starts one anyway"
        );
        assert!(fits(Some(40_000_000), 30_000_000, false));
        assert!(fits(Some(30_000_000), 30_000_000, false), "fits exactly");
        assert!(!fits(Some(25_000_000), 30_000_000, false));
    }

    const WORKERS: usize = 7;
    const COLUMNS: usize = 40;
    const MS: u64 = 1_000_000;

    /// One run of the model: what the check admitted, when the nearest column
    /// started, and the most workers that were ever inside a job at once.
    #[derive(Debug)]
    struct Admission {
        admitted: usize,
        first_start: Option<u64>,
        busiest: usize,
    }

    /// The mesh stage as a model, not a run of it (#18239): seven workers, a
    /// 20 Hz tick, and forty columns of one section each. Admission runs `calls`
    /// times a tick — `complete_commit` sits in the frame loop, so 20, 60 and
    /// 240 fps are 1, 3 and 12 calls — and what it has to hold is the work in
    /// flight, not a permission per call: the same heavy columns at every rate.
    /// By count (`left` of `None`, as a cold estimate prices), the same queue
    /// puts every worker inside a heavy column at once.
    fn mesh_model(job: u64, calls: u64, by_cost: bool) -> Admission {
        const TICK: u64 = 50 * 1_000_000;
        const TICKS: u64 = 24;
        let slice = WORKERS as u64 * MESH_SLICE_NANOS;
        let mut free_at = [0u64; WORKERS];
        let mut jobs: Vec<(u64, u64)> = Vec::new();
        let mut queued = COLUMNS;
        for tick in 0..TICKS {
            for call in 0..calls {
                let now = tick * TICK + call * TICK / calls;
                while queued > 0 {
                    // What the check reads: submitted and not yet finished, the
                    // jobs queued with the jobs running.
                    let in_flight = jobs.iter().filter(|&&(_, end)| end > now).count();
                    let left = if by_cost {
                        Some(slice.saturating_sub((in_flight as u64).saturating_mul(job)))
                    } else {
                        None
                    };
                    if !fits(left, job, in_flight == 0) {
                        break;
                    }
                    let worker = (0..WORKERS).min_by_key(|&w| free_at[w]).unwrap();
                    let start = free_at[worker].max(now);
                    free_at[worker] = start + job;
                    jobs.push((start, start + job));
                    queued -= 1;
                }
            }
        }
        // The busiest instant: half-open intervals, so an end (-1) comes before a
        // start (1) at the same time.
        let mut events: Vec<(u64, i8)> = jobs
            .iter()
            .flat_map(|&(start, end)| [(start, 1), (end, -1)])
            .collect();
        events.sort_unstable();
        let (mut busy, mut busiest) = (0i8, 0i8);
        for (_, delta) in events {
            busy += delta;
            busiest = busiest.max(busy);
        }
        Admission {
            admitted: jobs.len(),
            first_start: jobs.iter().map(|&(start, _)| start).min(),
            busiest: busiest as usize,
        }
    }

    /// The slice bounds the heavy meshing in flight and does so the same at 20,
    /// 60 and 240 calls a tick: it is not a permission per call. A heavy section
    /// costs 30 ms, so the pool holds fewer of them than its seven workers, while
    /// a light one (a millisecond) is worth less than the slice and runs as it did
    /// before the check. Nothing is lost on the way: every column is admitted, and
    /// the nearest one starts in the first call of the first tick.
    #[test]
    fn the_slice_bounds_the_mesh_work_in_flight_at_any_frame_rate() {
        let job = 30 * MS;
        let by_count = mesh_model(job, 1, false);
        assert_eq!(
            by_count.busiest, WORKERS,
            "by count, every worker takes a heavy column at once"
        );
        let cap = (WORKERS as u64 * MESH_SLICE_NANOS / job) as usize + 1;
        let mut heavy = Vec::new();
        for calls in [1, 3, 12] {
            let model = mesh_model(job, calls, true);
            assert_eq!(model.admitted, COLUMNS, "{calls} calls: a column was lost");
            assert_eq!(
                model.first_start,
                Some(0),
                "{calls} calls: the nearest column starts in the first call"
            );
            assert!(
                model.busiest <= cap,
                "{calls} calls: {} sections at once, over the slice's {cap}",
                model.busiest
            );
            heavy.push(model.busiest);
        }
        assert!(
            heavy.windows(2).all(|pair| pair[0] == pair[1]),
            "the check runs per call, so the frame rate must not lift it: {heavy:?}"
        );
        assert!(
            heavy[0] < by_count.busiest,
            "the slice must bind a heavy section: {} vs {}",
            heavy[0],
            by_count.busiest
        );
        assert_eq!(
            mesh_model(MS, 1, true).busiest,
            WORKERS,
            "a light section is not what the slice holds back"
        );
    }
}
