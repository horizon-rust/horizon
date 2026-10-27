//! Sound physics off the game thread: a thread of its own traces the rays
//! (`sound_physics::evaluate`) and hands the result to the mixer, so starting
//! a sound costs the game thread only a message.
//!
//! The thread reads a snapshot of the chunk sections around the listener.
//! Sections are shared (`Arc<Section>`), so the game thread passes the ones
//! that changed since the last tick without copying a block; a write to one
//! the thread still holds copies that section once (`Arc::make_mut`). The
//! mod clones the level on the game thread every 20 ticks instead.
use crate::chunk::{MIN_Y, SECTIONS, Section, index};
use crate::fast_hash::FxHashMap;
use crate::sound_engine::ChannelId;
use crate::sound_physics::{
    self, Environment, FluidKind, Material, Request, Settings, StateInfo, Voxels,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc::{Receiver, Sender, channel},
};
use std::time::{Duration, Instant};

/// Chunks of sections kept around the listener's chunk, each way (the mod's
/// `level_clone_range`): rays past them escape.
pub const RANGE: i32 = 4;
/// Sounds waiting for the thread past which new ones play dry: a burst of
/// hundreds (explosions, farms) must not delay them.
pub const MAX_PENDING: usize = 64;
/// How long a starting sound waits for its environment before playing dry.
pub const HOLD: Duration = Duration::from_millis(40);

type Key = (i32, i32, i32);
/// A block state as the thread builds its material from (`StateInfo`, owned:
/// the names are the block facts' own `Arc`s, so handing them over copies no
/// string).
#[derive(Clone, Debug)]
pub struct BlockState {
    pub block: Arc<str>,
    pub break_sound: Arc<str>,
    pub step_sound: Arc<str>,
    pub fluid: FluidKind,
    pub collides: bool,
    pub sturdy: u8,
    /// `sound_physics::closes_axes` of its shape.
    pub closes: u8,
}
/// Every state's material under `settings`.
pub fn materials(states: &[BlockState], settings: &Settings) -> Arc<[Material]> {
    states
        .iter()
        .map(|s| {
            sound_physics::material(
                &StateInfo {
                    block: &s.block,
                    break_sound: &s.break_sound,
                    step_sound: &s.step_sound,
                    fluid: s.fluid,
                    collides: s.collides,
                    sturdy: s.sturdy,
                    closes: s.closes,
                },
                settings,
            )
        })
        .collect()
}
/// What the thread is told.
enum Job {
    Sections(Vec<(Key, Option<Arc<Section>>)>),
    States(Arc<[BlockState]>),
    Settings(Box<Settings>),
    Evaluate {
        channel: ChannelId,
        request: Request,
        max_distance: f32,
    },
    Clear,
}
/// Where the thread puts what it worked out (the mixer, or a test).
pub trait Sink: Send + 'static {
    fn environment(&self, channel: ChannelId, environment: Environment);
}
/// Time spent, for `census` and the benchmark.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub evaluated: u64,
    pub evaluate_total: Duration,
    pub evaluate_max: Duration,
    /// Sounds that played dry because the thread was behind.
    pub dropped: u64,
    pub syncs: u64,
    pub sync_total: Duration,
    pub sync_max: Duration,
    pub sections: usize,
}
impl Stats {
    pub fn json(&self) -> serde_json::Value {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let mean = |total: Duration, n: u64| if n == 0 { 0.0 } else { ms(total) / n as f64 };
        serde_json::json!({
            "evaluated": self.evaluated,
            "evaluateMeanMs": mean(self.evaluate_total, self.evaluated),
            "evaluateMaxMs": ms(self.evaluate_max),
            "dropped": self.dropped,
            "syncs": self.syncs,
            "syncMeanMs": mean(self.sync_total, self.syncs),
            "syncMaxMs": ms(self.sync_max),
            "sections": self.sections,
        })
    }
}

/// The sections the thread reads, and the block states' materials.
#[derive(Default)]
pub struct Snapshot {
    sections: FxHashMap<Key, Arc<Section>>,
    materials: Arc<[Material]>,
}
impl Voxels for Snapshot {
    #[inline]
    fn material(&self, x: i32, y: i32, z: i32) -> Material {
        let sy = (y - MIN_Y) >> 4;
        if !(0..SECTIONS as i32).contains(&sy) {
            return Material::AIR;
        }
        self.sections
            .get(&(x >> 4, sy, z >> 4))
            .and_then(|s| self.materials.get(s.get(index(x, y, z)) as usize))
            .copied()
            .unwrap_or(Material::AIR)
    }
}
impl Snapshot {
    pub fn new(sections: FxHashMap<Key, Arc<Section>>, materials: Arc<[Material]>) -> Self {
        Self {
            sections,
            materials,
        }
    }
}

/// The game thread's side.
#[derive(Default)]
pub struct Acoustics {
    settings: Settings,
    jobs: Option<Sender<Job>>,
    pending: Arc<AtomicUsize>,
    stats: Arc<Mutex<Stats>>,
    /// The sections last given to the thread.
    sent: FxHashMap<Key, Arc<Section>>,
    /// How many block states the thread was given.
    states: Option<usize>,
}
impl Acoustics {
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    pub fn enabled(&self) -> bool {
        self.settings.enabled
    }
    pub fn stats(&self) -> Stats {
        self.stats.lock().map(|s| *s).unwrap_or_default()
    }
    /// Start the thread (once), delivering to `sink`.
    pub fn start(&mut self, sink: impl Sink) {
        if self.jobs.is_some() {
            return;
        }
        let (send, receive) = channel();
        let (pending, stats) = (self.pending.clone(), self.stats.clone());
        let settings = self.settings.clone();
        let spawned = std::thread::Builder::new()
            .name("sound physics".into())
            .spawn(move || run(receive, sink, settings, pending, stats));
        if spawned.is_ok() {
            self.jobs = Some(send);
        }
    }
    fn send(&mut self, job: Job) {
        if let Some(jobs) = &self.jobs
            && jobs.send(job).is_err()
        {
            self.jobs = None;
        }
    }
    /// New settings: kept here and on the thread. Turning it off drops the snapshot.
    pub fn set_settings(&mut self, settings: &Settings) {
        if *settings == self.settings {
            return;
        }
        self.settings = settings.clone();
        if !settings.enabled {
            self.sent.clear();
            self.send(Job::Clear);
        }
        self.send(Job::Settings(Box::new(settings.clone())));
    }
    /// Whether the thread needs the `count` block states (it builds their
    /// materials itself, again when the default factors change).
    pub fn needs_states(&self, count: usize) -> bool {
        self.settings.enabled && self.states != Some(count)
    }
    pub fn set_states(&mut self, states: Vec<BlockState>) {
        self.states = Some(states.len());
        self.send(Job::States(states.into()));
    }
    /// Give the thread the sections within `RANGE` chunks of the listener
    /// that changed, `chunk` reading a loaded chunk's sections.
    pub fn sync<'c>(
        &mut self,
        listener: [f64; 3],
        mut chunk: impl FnMut(i32, i32) -> Option<&'c [Arc<Section>]>,
    ) {
        if !self.settings.enabled || self.jobs.is_none() {
            return;
        }
        let started = Instant::now();
        let (lx, lz) = (
            (listener[0].floor() as i32) >> 4,
            (listener[2].floor() as i32) >> 4,
        );
        let mut changes = Vec::new();
        for cx in lx - RANGE..=lx + RANGE {
            for cz in lz - RANGE..=lz + RANGE {
                let sections = chunk(cx, cz).unwrap_or(&[]);
                for sy in 0..SECTIONS as i32 {
                    let key = (cx, sy, cz);
                    match (sections.get(sy as usize), self.sent.get(&key)) {
                        (Some(now), Some(sent)) if Arc::ptr_eq(now, sent) => {}
                        (Some(now), _) => {
                            self.sent.insert(key, now.clone());
                            changes.push((key, Some(now.clone())));
                        }
                        (None, Some(_)) => {
                            self.sent.remove(&key);
                            changes.push((key, None));
                        }
                        (None, None) => {}
                    }
                }
            }
        }
        let far = |&(x, _, z): &Key| (x - lx).abs() > RANGE || (z - lz).abs() > RANGE;
        let gone: Vec<Key> = self.sent.keys().copied().filter(far).collect();
        for key in gone {
            self.sent.remove(&key);
            changes.push((key, None));
        }
        if !changes.is_empty() {
            self.send(Job::Sections(changes));
        }
        if let Ok(mut s) = self.stats.lock() {
            let took = started.elapsed();
            s.syncs += 1;
            s.sync_total += took;
            s.sync_max = s.sync_max.max(took);
            s.sections = self.sent.len();
        }
    }
    /// Queue a sound; false when it should play dry now (off, nothing to
    /// trace, or the thread is behind).
    pub fn evaluate(&mut self, channel: ChannelId, request: Request, max_distance: f32) -> bool {
        if !self.settings.enabled
            || self.jobs.is_none()
            || self.states.is_none()
            || !sound_physics::processed(&request.event, request.source, &self.settings)
        {
            return false;
        }
        if self.pending.load(Ordering::Relaxed) >= MAX_PENDING {
            if let Ok(mut s) = self.stats.lock() {
                s.dropped += 1;
            }
            return false;
        }
        self.pending.fetch_add(1, Ordering::Relaxed);
        self.send(Job::Evaluate {
            channel,
            request,
            max_distance,
        });
        true
    }
}

fn run(
    jobs: Receiver<Job>,
    sink: impl Sink,
    mut settings: Settings,
    pending: Arc<AtomicUsize>,
    stats: Arc<Mutex<Stats>>,
) {
    let mut snapshot = Snapshot::default();
    let mut states: Arc<[BlockState]> = Arc::new([]);
    let mut cache = sound_physics::Cache::default();
    let mut rays = sound_physics::ray_directions(settings.environment_evaluation_ray_count);
    for job in jobs {
        match job {
            Job::Sections(changes) => {
                // Light updates copy sections too: only changed blocks
                // empty the cache.
                let mut changed = false;
                for (key, section) in changes {
                    let old = match section {
                        Some(s) => {
                            let old = snapshot.sections.insert(key, s.clone());
                            changed |= old.is_none_or(|o| o.blocks != s.blocks);
                            continue;
                        }
                        None => snapshot.sections.remove(&key),
                    };
                    changed |= old.is_some();
                }
                if changed {
                    cache.invalidate();
                }
            }
            Job::States(s) => {
                cache.invalidate();
                states = s;
                snapshot.materials = materials(&states, &settings);
            }
            Job::Settings(s) => {
                if s.environment_evaluation_ray_count != settings.environment_evaluation_ray_count {
                    rays = sound_physics::ray_directions(s.environment_evaluation_ray_count);
                }
                let defaults = |s: &Settings| {
                    (
                        s.default_block_reflectivity,
                        s.default_block_occlusion_factor,
                    )
                };
                let rebuild = defaults(&s) != defaults(&settings);
                settings = *s;
                if rebuild {
                    cache.invalidate();
                    snapshot.materials = materials(&states, &settings);
                }
            }
            Job::Clear => {
                cache.invalidate();
                snapshot.sections.clear();
            }
            Job::Evaluate {
                channel,
                request,
                max_distance,
            } => {
                let started = Instant::now();
                let environment = sound_physics::evaluate(
                    &snapshot,
                    &request,
                    max_distance,
                    &settings,
                    &rays,
                    &mut cache,
                );
                let took = started.elapsed();
                sink.environment(channel, environment);
                pending.fetch_sub(1, Ordering::Relaxed);
                if let Ok(mut s) = stats.lock() {
                    s.evaluated += 1;
                    s.evaluate_total += took;
                    s.evaluate_max = s.evaluate_max.max(took);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound_physics::tests::STONE;

    struct Collect(Mutex<std::sync::mpsc::Sender<(ChannelId, Environment)>>);
    impl Sink for Collect {
        fn environment(&self, channel: ChannelId, e: Environment) {
            let _ = self.0.lock().unwrap().send((channel, e));
        }
    }
    /// A section of stone below y 4 of its own 16 (air above).
    fn floor_section() -> Arc<Section> {
        let mut s = Section::default();
        for i in 0..4 * 256 {
            s.set(i, 1);
        }
        Arc::new(s)
    }

    #[test]
    fn snapshot_reads_materials_by_state() {
        let mut sections = FxHashMap::default();
        sections.insert((0, 8, 0), floor_section());
        let snapshot = Snapshot::new(sections, vec![Material::AIR, STONE].into());
        // Section 8 starts at y 64.
        assert!(snapshot.material(3, 64, 5).hit);
        assert!(snapshot.material(3, 67, 5).hit);
        assert!(!snapshot.material(3, 68, 5).hit);
        assert!(
            !snapshot.material(17, 64, 5).hit,
            "outside the snapshot is air"
        );
        assert!(!snapshot.material(3, -100, 5).hit);
    }

    #[test]
    fn sync_sends_only_changed_sections_and_the_thread_answers() {
        let (send, receive) = channel();
        let mut a = Acoustics::default();
        a.set_settings(&Settings {
            enabled: true,
            ..Settings::default()
        });
        a.start(Collect(Mutex::new(send)));
        let state = |block: &str, sound: &str, collides| BlockState {
            block: block.into(),
            break_sound: format!("minecraft:block.{sound}.break").into(),
            step_sound: format!("minecraft:block.{sound}.step").into(),
            fluid: FluidKind::None,
            collides,
            sturdy: 0b11_1111,
            closes: if collides { 0b111 } else { 0 },
        };
        let states = vec![
            state("minecraft:air", "stone", false),
            state("minecraft:stone", "stone", true),
        ];
        assert_eq!(materials(&states, &Settings::default())[1], STONE);
        a.set_states(states);
        let mut column: Vec<Arc<Section>> = (0..SECTIONS)
            .map(|_| Arc::new(Section::default()))
            .collect();
        column[8] = floor_section();
        a.sync([8.0, 70.0, 8.0], |x, z| {
            (x == 0 && z == 0).then_some(&column[..])
        });
        assert_eq!(a.stats().sections, SECTIONS);
        let before = a.sent.clone();
        a.sync([8.0, 70.0, 8.0], |x, z| {
            (x == 0 && z == 0).then_some(&column[..])
        });
        assert!(before.iter().all(|(k, v)| Arc::ptr_eq(v, &a.sent[k])));
        let request = Request {
            at: [8.5, 68.5, 8.5],
            listener: [8.5, 68.5, 12.5],
            listener_underwater: false,
            source: 5,
            event: "minecraft:entity.zombie.ambient".into(),
        };
        assert!(a.evaluate(7, request.clone(), 16.0));
        let (channel, e) = receive.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(channel, 7);
        // A floor reflects: some reverb, nothing in the way.
        assert!(e.send_gain.iter().sum::<f32>() > 0.0, "{e:?}");
        assert_eq!(e.direct_cutoff, 1.0);
        // Moving away drops the far sections.
        a.sync([8.0 + 16.0 * 20.0, 70.0, 8.0], |_, _| None);
        assert_eq!(a.stats().sections, 0);
        // Rain is never traced.
        assert!(!a.evaluate(
            8,
            Request {
                event: "minecraft:weather.rain".into(),
                ..request
            },
            16.0
        ));
        assert_eq!(a.stats().evaluated, 1);
    }
}
