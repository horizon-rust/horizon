//! What a client remembers of the far players and mobs a Horizon server feeds it
//! (`horizon_protocol::far_entities`), and how they move between snapshots: players
//! chase the target of their last snapshot, mobs walk to it at a walking pace and
//! stand still there. Bounded; nothing here simulates anything else.
use crate::fast_hash::FxHashMap;
use horizon_protocol::far_entities::Snapshot;

/// Ticks without a snapshot after which an entity is forgotten (the server has stopped
/// feeding, or a bad connection).
const STALE_TICKS: u32 = 1200;
/// How much of the way to its target a player covers in a tick.
const PLAYER_CHASE: f64 = 0.22;
/// A mob's walking pace, in blocks a tick, and its turn, in degrees a tick.
const MOB_SPEED: f64 = 0.2;
const MOB_TURN: f32 = 20.0;
/// Past this distance from its target, in blocks, an entity is put there at once.
const SNAP: f64 = 64.0;

/// What a far entity is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Player {
        uuid: u128,
        flags: u8,
    },
    /// A mob's id in the `minecraft:entity_type` registry, its look flags and variant.
    Mob {
        kind: u16,
        flags: u8,
        variant: u8,
    },
}

/// One remembered entity, `previous` and `position` being the last two ticks' (for the
/// partial tick between them).
#[derive(Clone, Debug)]
pub struct Far {
    pub id: i32,
    pub kind: Kind,
    pub previous: [f64; 3],
    pub position: [f64; 3],
    pub target: [f64; 3],
    pub previous_yaw: f32,
    pub yaw: f32,
    pub target_yaw: f32,
    /// Ticks since a snapshot had it.
    age: u32,
}

impl Far {
    fn new(id: i32, kind: Kind, at: [f64; 3], yaw: f32) -> Self {
        Self {
            id,
            kind,
            previous: at,
            position: at,
            target: at,
            previous_yaw: yaw,
            yaw,
            target_yaw: yaw,
            age: 0,
        }
    }

    /// Where it is `alpha` of the way from the last tick to this one.
    pub fn at(&self, alpha: f64) -> [f64; 3] {
        std::array::from_fn(|i| self.previous[i] + (self.position[i] - self.previous[i]) * alpha)
    }

    /// Its yaw as `at` has its place.
    pub fn yaw_at(&self, alpha: f32) -> f32 {
        self.previous_yaw + wrap(self.yaw - self.previous_yaw) * alpha
    }

    fn tick(&mut self) {
        self.age += 1;
        self.previous = self.position;
        self.previous_yaw = self.yaw;
        let delta: [f64; 3] = std::array::from_fn(|i| self.target[i] - self.position[i]);
        let distance = delta.iter().map(|d| d * d).sum::<f64>().sqrt();
        if distance > SNAP {
            self.position = self.target;
            self.previous = self.target;
            self.yaw = self.target_yaw;
            self.previous_yaw = self.yaw;
            return;
        }
        match self.kind {
            Kind::Player { .. } => {
                for i in 0..3 {
                    self.position[i] += delta[i] * PLAYER_CHASE;
                }
                self.yaw += wrap(self.target_yaw - self.yaw) * PLAYER_CHASE as f32;
            }
            Kind::Mob { .. } if distance > MOB_SPEED => {
                for i in 0..3 {
                    self.position[i] += delta[i] / distance * MOB_SPEED;
                }
                // Facing where it walks (Entity.yRot: 0 is south, +z).
                let heading = (-delta[0]).atan2(delta[2]).to_degrees() as f32;
                self.yaw += wrap(heading - self.yaw).clamp(-MOB_TURN, MOB_TURN);
            }
            Kind::Mob { .. } => {
                self.position = self.target;
                self.yaw += wrap(self.target_yaw - self.yaw).clamp(-MOB_TURN, MOB_TURN);
            }
        }
    }
}

/// `Mth.wrapDegrees`.
fn wrap(degrees: f32) -> f32 {
    let mut a = degrees.rem_euclid(360.0);
    if a >= 180.0 {
        a -= 360.0;
    }
    a
}

/// The mobs kept unless the setting says another number.
pub const DEFAULT_MAX_MOBS: usize = 1000;

/// The far entities of the level the client is in.
pub struct Memory {
    level: Option<i32>,
    entities: FxHashMap<i32, Far>,
    /// The most mobs kept (the client's setting); players are few.
    max_mobs: usize,
}

impl Default for Memory {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_MOBS)
    }
}

impl Memory {
    pub fn new(max_mobs: usize) -> Self {
        Self {
            level: None,
            entities: FxHashMap::default(),
            max_mobs,
        }
    }

    pub fn set_max_mobs(&mut self, max: usize) {
        self.max_mobs = max;
        if self.mobs() > max {
            let mut mobs: Vec<i32> = self
                .entities
                .values()
                .filter(|e| matches!(e.kind, Kind::Mob { .. }))
                .map(|e| e.id)
                .collect();
            // The oldest ids first are as good a choice as any: the next snapshot refills the nearest.
            mobs.sort_unstable();
            for id in mobs.into_iter().skip(max) {
                self.entities.remove(&id);
            }
        }
    }

    pub fn clear(&mut self) {
        self.entities.clear();
        self.level = None;
    }

    /// The server tracks the entity now (it came as a real one): not remembered as a far one.
    pub fn forget(&mut self, id: i32) {
        self.entities.remove(&id);
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
    pub fn mobs(&self) -> usize {
        self.entities
            .values()
            .filter(|e| matches!(e.kind, Kind::Mob { .. }))
            .count()
    }
    pub fn iter(&self) -> impl Iterator<Item = &Far> {
        self.entities.values()
    }

    /// A snapshot replaces the entities of its kind; one from another level starts the memory over.
    pub fn apply(&mut self, snapshot: Snapshot) {
        let level = match &snapshot {
            Snapshot::Players { level, .. } | Snapshot::Mobs { level, .. } => *level,
        };
        if self.level != Some(level) {
            self.entities.clear();
            self.level = Some(level);
        }
        match snapshot {
            Snapshot::Players { players, .. } => self.replace(
                true,
                players
                    .into_iter()
                    .map(|p| {
                        let kind = Kind::Player {
                            uuid: p.uuid,
                            flags: p.flags,
                        };
                        (p.id, kind, p.position, p.yaw)
                    })
                    .collect(),
            ),
            Snapshot::Mobs { mobs, .. } => self.replace(
                false,
                mobs.into_iter()
                    .take(self.max_mobs)
                    .map(|m| {
                        let kind = Kind::Mob {
                            kind: m.kind,
                            flags: m.flags,
                            variant: m.variant,
                        };
                        (m.id, kind, m.position, m.yaw)
                    })
                    .collect(),
            ),
        }
    }

    /// The players (or the mobs) are now `incoming`: the ones already here walk or chase to their
    /// new places, the others are new, and the ones left out are gone.
    fn replace(&mut self, players: bool, incoming: Vec<(i32, Kind, [f64; 3], f32)>) {
        let mut previous = FxHashMap::default();
        self.entities.retain(|id, e| {
            if matches!(e.kind, Kind::Player { .. }) != players {
                return true;
            }
            previous.insert(*id, e.clone());
            false
        });
        for (id, kind, at, yaw) in incoming {
            let far = match previous.remove(&id) {
                Some(mut far) if same(&far.kind, &kind) => {
                    far.kind = kind;
                    far.target = at;
                    far.target_yaw = yaw;
                    far.age = 0;
                    far
                }
                _ => Far::new(id, kind, at, yaw),
            };
            self.entities.insert(id, far);
        }
    }

    /// One client tick: everything walks or chases, and what no snapshot has had for a minute goes.
    pub fn tick(&mut self) {
        for e in self.entities.values_mut() {
            e.tick();
        }
        self.entities.retain(|_, e| e.age < STALE_TICKS);
    }
}

/// The same player, or the same type of mob (an id may be reused by another entity).
fn same(a: &Kind, b: &Kind) -> bool {
    match (a, b) {
        (Kind::Player { uuid: x, .. }, Kind::Player { uuid: y, .. }) => x == y,
        (Kind::Mob { kind: x, .. }, Kind::Mob { kind: y, .. }) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::far_entities::{Mob, Player};

    fn mob(id: i32, x: f64) -> Mob {
        Mob {
            id,
            kind: 5,
            position: [x, 64.0, 0.0],
            yaw: 0.0,
            flags: 0,
            variant: 0,
        }
    }

    fn player(id: i32, x: f64) -> Player {
        Player {
            id,
            uuid: id as u128,
            position: [x, 64.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
        }
    }

    #[test]
    fn a_snapshot_replaces_its_kind_only() {
        let mut m = Memory::new(10);
        m.apply(Snapshot::Mobs {
            level: 1,
            mobs: vec![mob(1, 0.0), mob(2, 0.0)],
        });
        m.apply(Snapshot::Players {
            level: 1,
            players: vec![player(10, 0.0)],
        });
        assert_eq!((m.len(), m.mobs()), (3, 2));
        m.apply(Snapshot::Mobs {
            level: 1,
            mobs: vec![mob(2, 5.0)],
        });
        assert_eq!(
            (m.len(), m.mobs()),
            (2, 1),
            "mob 1 left the list, the player stayed"
        );
        m.apply(Snapshot::Players {
            level: 1,
            players: vec![],
        });
        assert_eq!((m.len(), m.mobs()), (1, 1));
    }

    #[test]
    fn another_level_starts_over() {
        let mut m = Memory::new(10);
        m.apply(Snapshot::Mobs {
            level: 1,
            mobs: vec![mob(1, 0.0)],
        });
        m.apply(Snapshot::Players {
            level: 2,
            players: vec![player(10, 0.0)],
        });
        assert_eq!(m.len(), 1);
        assert!(matches!(m.iter().next().unwrap().kind, Kind::Player { .. }));
    }

    #[test]
    fn the_mob_count_is_bounded() {
        let mut m = Memory::new(3);
        m.apply(Snapshot::Mobs {
            level: 1,
            mobs: (0..10).map(|i| mob(i, 0.0)).collect(),
        });
        assert_eq!(m.mobs(), 3);
        m.set_max_mobs(1);
        assert_eq!(m.mobs(), 1);
    }

    #[test]
    fn a_real_entity_is_forgotten() {
        let mut m = Memory::new(10);
        m.apply(Snapshot::Mobs {
            level: 1,
            mobs: vec![mob(1, 0.0)],
        });
        m.forget(1);
        assert!(m.is_empty());
    }

    #[test]
    fn players_chase_and_mobs_walk() {
        let mut m = Memory::new(10);
        m.apply(Snapshot::Players {
            level: 1,
            players: vec![player(10, 0.0)],
        });
        m.apply(Snapshot::Mobs {
            level: 1,
            mobs: vec![mob(1, 0.0)],
        });
        // The next snapshots move their targets, not the entities.
        m.apply(Snapshot::Players {
            level: 1,
            players: vec![player(10, 10.0)],
        });
        m.apply(Snapshot::Mobs {
            level: 1,
            mobs: vec![mob(1, 10.0)],
        });
        let at = |m: &Memory, id| m.iter().find(|e| e.id == id).unwrap().position[0];
        assert_eq!((at(&m, 10), at(&m, 1)), (0.0, 0.0));
        m.tick();
        // 22% of ten blocks; one pace of a mob.
        assert!((at(&m, 10) - 2.2).abs() < 1e-9);
        assert!((at(&m, 1) - MOB_SPEED).abs() < 1e-9);
        for _ in 0..200 {
            m.tick();
            // Snapshots keep the entities from going stale.
            m.entities.values_mut().for_each(|e| e.age = 0);
        }
        assert!(
            (at(&m, 1) - 10.0).abs() < 1e-9,
            "the mob stands where it was told"
        );
        assert!((at(&m, 10) - 10.0).abs() < 1e-6);
    }

    #[test]
    fn a_far_jump_snaps_and_a_silent_server_is_forgotten() {
        let mut m = Memory::new(10);
        m.apply(Snapshot::Mobs {
            level: 1,
            mobs: vec![mob(1, 0.0)],
        });
        m.entities.get_mut(&1).unwrap().target = [1000.0, 64.0, 0.0];
        m.tick();
        assert_eq!(m.iter().next().unwrap().position[0], 1000.0);
        for _ in 0..STALE_TICKS {
            m.tick();
        }
        assert!(m.is_empty());
    }

    #[test]
    fn a_walking_mob_faces_its_heading() {
        let mut far = Far::new(
            1,
            Kind::Mob {
                kind: 1,
                flags: 0,
                variant: 0,
            },
            [0.0; 3],
            0.0,
        );
        // Walking toward -x: Entity.yRot of 90 faces -x.
        far.target = [-10.0, 0.0, 0.0];
        for _ in 0..10 {
            far.tick();
        }
        assert!((far.yaw - 90.0).abs() < 1e-3, "{}", far.yaw);
    }
}
