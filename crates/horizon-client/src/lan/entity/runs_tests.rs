//! The tick's changes given as ranges of one buffer (`Body::Run`) are the packets, in the order,
//! that a body for each packet gave every client.
use super::*;

fn entity(id: i32, kind: &str, at: [f64; 2]) -> EntityState {
    EntityState {
        id,
        uuid: id as u128,
        kind: kind.into(),
        position: [at[0], 64.0, at[1]],
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        on_ground: true,
        velocity: [0.0; 3],
        acceleration_power: None,
        minecart_steps: None,
        data: 0,
        meta: Vec::new(),
        forced_meta: [0; 4],
        defaults: Vec::new(),
        equipment: Vec::new(),
        attributes: Vec::new(),
        precise: false,
        vertical_collision: false,
        horizontal_collision: false,
        needs_sync: false,
        sync_position: false,
        stepped: false,
        idle: false,
        bare: false,
        leash: None,
        spectator: false,
        watching: false,
        hidden: None,
    }
}

/// Crowds of players standing close to herds that mostly stand still, with entities crossing
/// sections and cells, appearing, leaving, turning spectator or hidden, being synced as they
/// join, and clients that move, change their view distance, lose chunks and forget what they
/// track: for each client and tick the packets, in order, and the tracking events are the same
/// with ranges as with a body for each packet, whether the clients are given their packets on
/// one thread or on several (both strategies are run), and some clients are of a vanilla
/// release (not Horizon's: they get the other buffer).
///
/// Not exercised: clients viewing a level from another (`sources`, which keep a body for each
/// packet), riders, a client's entity changing id, and the connection's queue and framing (the
/// pool's own tests: `io_pool::tests::a_run_reaches_the_client_as_its_packets`).
#[test]
fn ranges_hold_the_packets_bodies_did() {
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let kinds = ["minecraft:cow", "minecraft:sheep", "minecraft:item"];
    let full: crate::fast_hash::FxHashSet<(i32, i32)> =
        (-6..6).flat_map(|x| (-6..6).map(move |z| (x, z))).collect();
    let (mut compared, mut runs, mut long_runs) = (0, 0, 0);
    for level in 0..16 {
        let apart = level % 2 == 0;
        let mut both = [Entities::default(), Entities::default()];
        both[1].runs_off = true;
        for side in &mut both {
            side.send_apart_always = Some(apart);
        }
        let players = 2 + next(12) as i32;
        let mut trackers: [Vec<Tracker>; 2] =
            std::array::from_fn(|_| (0..players).map(|_| Tracker::default()).collect());
        let mut outs: [Vec<Vec<crate::lan::Body>>; 2] =
            std::array::from_fn(|_| vec![Vec::new(); players as usize]);
        let mut held: Vec<crate::fast_hash::FxHashSet<(i32, i32)>> =
            (0..players).map(|_| full.clone()).collect();
        let at = |next: &mut dyn FnMut(u64) -> u64| {
            [
                (next(1000) as f64 / 1000.0 - 0.5) * 40.0,
                (next(1000) as f64 / 1000.0 - 0.5) * 40.0,
            ]
        };
        let mut all: Vec<EntityState> = (0..players + 40 + next(150) as i32)
            .map(|i| {
                let kind = if i < players {
                    "minecraft:player"
                } else {
                    kinds[next(3) as usize]
                };
                entity(1 + i, kind, at(&mut next))
            })
            .collect();
        let mut next_id = all.len() as i32 + 1;
        let mut view = vec![6; players as usize];
        for tick in 0..40 {
            let mut synced = Vec::new();
            if next(5) == 0 {
                all.push(entity(next_id, kinds[next(3) as usize], at(&mut next)));
                if next(2) == 0 {
                    synced.push((next_id - 1, next_id));
                }
                next_id += 1;
            }
            if next(6) == 0 && all.len() > players as usize + 1 {
                let gone = players as usize + next((all.len() - players as usize) as u64) as usize;
                all.remove(gone);
            }
            for e in &mut all {
                let player = e.kind == "minecraft:player";
                if (player && next(2) == 0) || next(40) == 0 {
                    let far = if next(8) == 0 { 40.0 } else { 1.5 };
                    e.position[0] += (next(1000) as f64 / 1000.0 - 0.5) * far;
                    e.position[2] += (next(1000) as f64 / 1000.0 - 0.5) * 1.5;
                }
                if next(60) == 0 {
                    e.spectator = player && !e.spectator;
                }
                if next(80) == 0 {
                    e.hidden = if e.hidden.is_some() {
                        None
                    } else {
                        Some(next(3) as u8)
                    };
                }
                if next(6) == 0 {
                    e.meta = vec![(0, serializer::BYTE, Value::Byte(next(3) as i8))];
                }
                e.on_ground = next(4) != 0;
                e.yaw = next(360) as f32;
                e.head_yaw = next(360) as f32;
            }
            for k in 0..players as usize {
                if next(30) == 0 {
                    view[k] = 1 + next(8) as i32;
                }
                if next(25) == 0 {
                    let cell = (next(12) as i32 - 6, next(12) as i32 - 6);
                    if !held[k].remove(&cell) {
                        held[k].insert(cell);
                    }
                }
            }
            let refs: Vec<&EntityState> = all.iter().collect();
            let moved: Vec<bool> = (0..players).map(|_| tick == 0 || next(5) != 0).collect();
            let forget: Vec<Option<i32>> = (0..players)
                .map(|_| (next(40) == 0).then(|| 1 + next(60) as i32))
                .collect();
            for side in 0..2 {
                both[side].synced.clone_from(&synced);
                for out in &mut outs[side] {
                    out.clear();
                }
                for (tracker, forget) in trackers[side].iter_mut().zip(&forget) {
                    if let Some(id) = *forget {
                        tracker.forget(id);
                    }
                }
                let mut viewers: Vec<Viewer> = trackers[side]
                    .iter_mut()
                    .zip(outs[side].iter_mut())
                    .enumerate()
                    .map(|(k, (tracker, out))| Viewer {
                        entity_id: 1 + k as i32,
                        horizon: k % 3 != 2,
                        sources: None,
                        spectator: all[k].spectator,
                        permission: (k % 3) as u8,
                        moved: moved[k],
                        position: all[k].position,
                        view_distance: view[k],
                        chunks: &held[k],
                        tracker,
                        out,
                        gone: None,
                    })
                    .collect();
                both[side].tick(&refs, &Riding::default(), &mut viewers, |_| Some(5));
            }
            let events = |trackers: &mut Vec<Tracker>| -> Vec<Vec<(bool, i32)>> {
                trackers
                    .iter_mut()
                    .map(|t| {
                        t.take_events()
                            .iter()
                            .map(|e| (e.start, e.entity))
                            .collect()
                    })
                    .collect()
            };
            let [first, second] = &mut trackers;
            assert_eq!(events(first), events(second), "level {level}, tick {tick}");
            let flat = |outs: &Vec<Vec<crate::lan::Body>>| -> Vec<Vec<Vec<u8>>> {
                outs.iter().map(|o| crate::lan::Body::flatten(o)).collect()
            };
            assert_eq!(flat(&outs[0]), flat(&outs[1]), "level {level}, tick {tick}");
            compared += flat(&outs[0]).iter().map(Vec::len).sum::<usize>();
            for out in &outs[0] {
                for body in out {
                    if let crate::lan::Body::Run(_) = body {
                        runs += 1;
                        long_runs += usize::from(body.packets().len() > 1);
                    }
                }
            }
        }
    }
    assert!(
        compared > 5_000,
        "the clients were sent their entities ({compared})"
    );
    assert!(
        runs > 100 && long_runs > 50,
        "clients were given ranges ({runs}, {long_runs} of several packets)"
    );
}
