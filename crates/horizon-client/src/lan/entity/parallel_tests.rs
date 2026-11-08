//! The clients' tracking passes on several threads send every client the same packets, in the
//! same order, with the same tracking events, as on one.
use super::*;

/// A level of `n` zombies spread over 160 blocks, moved a little each tick.
fn level(n: usize, tick: usize) -> Vec<EntityState> {
    (0..n)
        .map(|i| {
            let (a, b) = ((i * 37 % 160) as f64 - 80.0, (i * 91 % 160) as f64 - 80.0);
            let step = ((i + tick) % 7) as f64 * 0.4;
            EntityState {
                id: 1000 + i as i32,
                uuid: 1000 + i as u128,
                kind: "minecraft:zombie".into(),
                position: [a + step, 64.0, b - step * (i % 3) as f64],
                velocity: [0.1, 0.0, -0.05],
                needs_sync: i % 11 == tick % 11,
                ..EntityState::default()
            }
        })
        .collect()
}

struct Clients {
    chunks: crate::fast_hash::FxHashSet<(i32, i32)>,
    trackers: Vec<Tracker>,
    out: Vec<Vec<Vec<u8>>>,
}

impl Clients {
    fn new(n: usize) -> Self {
        Self {
            chunks: (-12..12).flat_map(|x| (-12..12).map(move |z| (x, z))).collect(),
            trackers: (0..n).map(|_| Tracker::default()).collect(),
            out: vec![Vec::new(); n],
        }
    }
    /// One tick of `level` for every client; what each was sent and its tracking events.
    fn tick(
        &mut self,
        level: &mut Entities,
        entities: &[EntityState],
        tick: usize,
        lanes: usize,
    ) -> Vec<(Vec<Vec<u8>>, Vec<(bool, i32)>)> {
        let refs: Vec<&EntityState> = entities.iter().collect();
        let chunks = &self.chunks;
        let mut viewers: Vec<Viewer> = self
            .trackers
            .iter_mut()
            .zip(&mut self.out)
            .enumerate()
            .map(|(k, (tracker, out))| {
                out.clear();
                let k = k as f64;
                Viewer {
                    entity_id: 1 + k as i32,
                    spectator: false,
                    permission: 0,
                    moved: (tick + k as usize).is_multiple_of(3),
                    position: [k * 13.0 - 70.0 + tick as f64, 64.0, 60.0 - k * 11.0],
                    view_distance: 3 + (k as i32 % 4),
                    chunks,
                    tracker,
                    out,
                    gone: None,
                }
            })
            .collect();
        level.tick_in(&refs, &Riding::default(), &mut viewers, |_| Some(100), lanes);
        drop(viewers);
        self.trackers
            .iter_mut()
            .zip(&self.out)
            .map(|(t, out)| {
                let events = t.take_events().iter().map(|e| (e.start, e.entity)).collect();
                (out.clone(), events)
            })
            .collect()
    }
}

#[test]
fn clients_on_threads_get_what_one_thread_sends() {
    let (mut one, mut many) = (Entities::default(), Entities::default());
    let (mut a, mut b) = (Clients::new(12), Clients::new(12));
    let mut sent = 0;
    for tick in 0..8 {
        let entities = level(200, tick);
        let serial = a.tick(&mut one, &entities, tick, 1);
        let parallel = b.tick(&mut many, &entities, tick, 3);
        assert_eq!(serial, parallel, "tick {tick}");
        sent += serial.iter().map(|(out, _)| out.len()).sum::<usize>();
    }
    // The clients did see and track the zombies.
    assert!(sent > 100, "{sent} packets");
}
