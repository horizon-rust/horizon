//! Where the server thread's time goes when regions are resident: a cheap per-step clock for each tick, summed
//! over 200 ticks and logged. Always on (a handful of `Instant`s a tick); the steps also carry spans of their
//! own for `horizon-report`.
use std::time::Instant;

/// The steps of a server tick, in the order they run.
#[derive(Clone, Copy)]
pub(super) enum Step {
    /// The network: the hub's packets read at the start of the world tick.
    Net,
    /// Working out the regions, and splitting the level into them (the tick after a landing).
    Split,
    /// Starting the region threads: clocks and ids copied in, packets handed over.
    Launch,
    /// Waiting for the regions, up to the deadline.
    Wait,
    /// The regions coming home: the level and the players put back.
    Land,
    /// What no region owns, ticked serially: with no regions, the whole level.
    Leftover,
    /// The server hub's own tick: its players' updates and packets out.
    Lan,
    /// The world tick's later phases: sculk, sleep, the End, particles, bossbars.
    Late,
}
const STEPS: usize = 8;
const NAMES: [&str; STEPS] = [
    "net", "split", "launch", "wait", "land", "leftover", "lan", "late",
];
const WINDOW: u32 = 200;

#[derive(Default)]
pub(super) struct TickProfile {
    /// This tick's steps (ms).
    cur: [f32; STEPS],
    ticks: u32,
    sum: [f32; STEPS],
    /// Every tick's total (ms) and whether it landed or split regions.
    totals: Vec<(f32, bool)>,
    /// Whether regions were resident at any point of the window.
    resident: bool,
    started: Option<Instant>,
}

impl TickProfile {
    /// Adds the time since `since` to `step`; returns now, for the next step.
    pub fn add(&mut self, step: Step, since: Instant) -> Instant {
        let now = Instant::now();
        self.cur[step as usize] += now.duration_since(since).as_secs_f32() * 1000.0;
        now
    }

    pub fn begin(&mut self) {
        self.started = Some(Instant::now());
    }

    /// Notes that regions are resident this tick.
    pub fn note_resident(&mut self) {
        self.resident = true;
    }

    /// The tick is over: sums it in, and logs the window every 200 ticks.
    pub fn end(&mut self) {
        let Some(started) = self.started.take() else {
            return;
        };
        let total = started.elapsed().as_secs_f32() * 1000.0;
        let landing = self.cur[Step::Split as usize] > 0.0 && self.cur[Step::Launch as usize] > 0.0
            || self.cur[Step::Land as usize] > 0.0;
        for (s, c) in self.sum.iter_mut().zip(self.cur.iter_mut()) {
            *s += *c;
            *c = 0.0;
        }
        self.totals.push((total, landing));
        self.ticks += 1;
        if self.ticks >= WINDOW {
            self.report();
        }
    }

    fn report(&mut self) {
        let n = self.ticks.max(1) as f32;
        let totals = std::mem::take(&mut self.totals);
        let resident = std::mem::take(&mut self.resident);
        let sum = std::mem::take(&mut self.sum);
        self.ticks = 0;
        if !resident {
            return;
        }
        let mut sorted: Vec<f32> = totals.iter().map(|t| t.0).collect();
        sorted.sort_by(f32::total_cmp);
        let at = |p: f32| sorted[((sorted.len() as f32 * p) as usize).min(sorted.len() - 1)];
        let tick_total: f32 = totals.iter().map(|t| t.0).sum();
        let landing: Vec<f32> = totals.iter().filter(|t| t.1).map(|t| t.0).collect();
        let landing_max = landing.iter().copied().fold(0.0, f32::max);
        let landing_avg = landing.iter().sum::<f32>() / landing.len().max(1) as f32;
        let steps: Vec<String> = NAMES
            .iter()
            .zip(sum)
            .map(|(name, ms)| format!("{name} {:.1}", ms / n))
            .collect();
        let measured: f32 = sum.iter().sum();
        let global =
            (sum[Step::Leftover as usize] + sum[Step::Lan as usize] + sum[Step::Late as usize]) / n;
        log::info!(
            "regions: wait {:.1} ms, global {global:.1} ms, landing {:.1} ms (avg), landing ticks {} (avg {landing_avg:.1} ms, {landing_max:.1} ms max); \
             tick avg {:.1} p50 {:.1} p95 {:.1} p99 {:.1} max {:.1} ms; per tick: {}, other {:.1}",
            sum[Step::Wait as usize] / n,
            (sum[Step::Land as usize] + sum[Step::Split as usize] + sum[Step::Launch as usize]) / n,
            landing.len(),
            tick_total / n,
            at(0.5),
            at(0.95),
            at(0.99),
            sorted[sorted.len() - 1],
            steps.join(", "),
            (tick_total - measured) / n,
        );
    }
}
