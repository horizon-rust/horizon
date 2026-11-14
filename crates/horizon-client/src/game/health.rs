//! Server health: `/tps` (the server's TPS and tick times over 5 s, 15 s, 1 min, 5 min and 15 min, and each
//! region's last tick) and `/mspt` (tick times from the last 5 s, 10 s and 1 min).
use super::Game;
use horizon_client::text::Text;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Ticks kept: 15 minutes at 20 a second.
const KEPT: usize = 15 * 60 * 20;

struct Ticks {
    /// When each tick ended and what it took (ms), oldest first.
    ticks: VecDeque<(Instant, f32)>,
}

static TICKS: Mutex<Ticks> = Mutex::new(Ticks {
    ticks: VecDeque::new(),
});

/// One region's last tick, for `/tps` .
#[derive(Clone, Debug, Default)]
pub(super) struct RegionStat {
    pub level: String,
    /// The block at its first section's corner.
    pub at: (i32, i32),
    pub chunks: usize,
    pub players: usize,
    pub ms: f32,
    /// Still ticking past the server's tick (lag isolation).
    pub late: bool,
}

static REGIONS: Mutex<Vec<RegionStat>> = Mutex::new(Vec::new());

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The server's loop, after each tick: what it took.
pub(super) fn record_tick(took: Duration) {
    let now = Instant::now();
    let mut t = lock(&TICKS);
    if t.ticks.len() == KEPT {
        t.ticks.pop_front();
    }
    t.ticks.push_back((now, took.as_secs_f32() * 1000.0));
}

/// A level's regions this tick (`tick_world_regions`), replacing that level's last ones.
pub(super) fn record_regions(level: &str, stats: Vec<RegionStat>) {
    let mut r = lock(&REGIONS);
    r.retain(|s| s.level != level);
    r.extend(stats);
}

/// TPS (at most 20 a second, as Paper counts it) and the tick times (average, max) of the ticks that ended
/// in the last `window`.
fn window(t: &Ticks, now: Instant, window: Duration) -> Option<(f64, f64, f64, f64)> {
    let since = now.checked_sub(window)?;
    let mut n = 0usize;
    let (mut sum, mut min, mut max) = (0.0f64, f64::MAX, 0.0f64);
    let mut first = None;
    for &(at, ms) in t.ticks.iter().rev() {
        if at < since {
            break;
        }
        first = Some(at);
        n += 1;
        let ms = f64::from(ms);
        sum += ms;
        min = min.min(ms);
        max = max.max(ms);
    }
    let first = first?;
    // Over what the ticks really cover (a server up for 3 s has 3 s of a 1 min window).
    let span = now.duration_since(first).as_secs_f64().max(0.05) + 0.05;
    let tps = (n as f64 / span.min(window.as_secs_f64())).min(20.0);
    Some((tps, sum / n as f64, min, max))
}

fn color_tps(tps: f64) -> u32 {
    if tps >= 19.5 {
        horizon_client::text::GREEN
    } else if tps >= 15.0 {
        horizon_client::text::YELLOW
    } else {
        horizon_client::text::RED
    }
}

impl Game<'_> {
    /// `/tps`: the server health report.
    pub(super) fn cmd_tps(
        &mut self,
    ) -> std::result::Result<i32, horizon_client::command::CommandError> {
        let now = Instant::now();
        let (lines, tps_now) = {
            let t = lock(&TICKS);
            let mut lines = Vec::new();
            let windows = [
                ("5s", 5u64),
                ("15s", 15),
                ("1m", 60),
                ("5m", 300),
                ("15m", 900),
            ];
            let stats: Vec<_> = windows
                .iter()
                .map(|&(_, s)| window(&t, now, Duration::from_secs(s)))
                .collect();
            let tps: Vec<String> = stats
                .iter()
                .map(|s| s.map_or("-".into(), |s| format!("{:.2}", s.0)))
                .collect();
            let mspt: Vec<String> = stats
                .iter()
                .map(|s| s.map_or("-".into(), |s| format!("{:.1}", s.1)))
                .collect();
            let names: Vec<&str> = windows.iter().map(|w| w.0).collect();
            lines.push(format!(
                "Server health: TPS ({}): {}",
                names.join(", "),
                tps.join(", ")
            ));
            lines.push(format!("MSPT ({}): {}", names.join(", "), mspt.join(", ")));
            (lines, stats[0].map_or(20.0, |s| s.0))
        };
        let color = color_tps(tps_now);
        for (i, line) in lines.into_iter().enumerate() {
            let text = Text::literal(line);
            self.system_message(if i == 0 { text.with_color(color) } else { text });
        }
        let regions = lock(&REGIONS).clone();
        let count = regions.len();
        let slowest = regions.iter().map(|r| r.ms).fold(0.0f32, f32::max);
        let average = regions.iter().map(|r| r.ms).sum::<f32>() / count.max(1) as f32;
        let late = regions.iter().filter(|r| r.late).count();
        self.system_message(Text::literal(format!(
            "Regions: {count} (last tick slowest {slowest:.1} ms, average {average:.1} ms; {late} still running)"
        )));
        let mut busiest = regions;
        busiest.sort_by(|a, b| b.ms.total_cmp(&a.ms));
        for r in busiest.iter().take(5) {
            let ms_tps = if r.ms <= 50.0 {
                20.0
            } else {
                1000.0 / f64::from(r.ms)
            };
            self.system_message(
                Text::literal(format!(
                    "  {} near {}, {}: {:.1} ms ({:.1} TPS), {} chunks, {} players{}",
                    r.level,
                    r.at.0,
                    r.at.1,
                    r.ms,
                    ms_tps,
                    r.chunks,
                    r.players,
                    if r.late { ", running late" } else { "" }
                ))
                .with_color(color_tps(ms_tps)),
            );
        }
        Ok(tps_now.round() as i32)
    }

    /// `/mspt`: Paper's tick times (average/min/max) from the last 5 s, 10 s and 1 min.
    pub(super) fn cmd_mspt(
        &mut self,
    ) -> std::result::Result<i32, horizon_client::command::CommandError> {
        let now = Instant::now();
        let parts: Vec<String> = {
            let t = lock(&TICKS);
            [5u64, 10, 60]
                .iter()
                .map(|&s| {
                    window(&t, now, Duration::from_secs(s))
                        .map_or("-".into(), |w| format!("{:.1}/{:.1}/{:.1}", w.1, w.2, w.3))
                })
                .collect()
        };
        self.system_message(Text::literal(
            "Server tick times (avg/min/max) from last 5s, 10s, 1m:".to_owned(),
        ));
        self.system_message(Text::literal(format!("◴ {}", parts.join(", "))));
        Ok(1)
    }
}
