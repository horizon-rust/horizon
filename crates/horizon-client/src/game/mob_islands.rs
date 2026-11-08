//! The mob phase of a server tick (`tick_creature_entities`) on several threads, with the result of
//! one thread: vanilla ticks the level's entities one after another, and here too each mob ticks after
//! every mob before it that it could have seen or touched.
//!
//! During the mob loop the level's blocks are read-only (a mob's block writes are events, applied
//! after the loop in order), players are read-only views, and a mob reaches the other mobs only
//! through the snapshot (`Seen`, read near itself or by id), its pushes, and the brain events that
//! write another mob by id. So the mobs split into islands: a mob's reach for the tick (how far it
//! can move, how far it reads others, who it is linked to by id) meets only mobs of its own island.
//! Each group of islands ticks its mobs in list order on its own thread, against a snapshot of its
//! own members; the results merge back in list order.
//!
//! The one stream every mob may draw from is the level random: each group draws from a lane of an
//! [`OrderedDraws`], so a mob's first draw waits until every mob before it in the list has ticked,
//! and the draws come out in the serial order (horizon-core `random/ordered.rs`).
//!
//! `HORIZON_PARALLEL_TICK=1` turns it on (off by default; `horizon_client::lan::entity::parallel_tick`). The tick stays serial with native mods
//! loaded (their hooks can touch anything), on a tick a performance report samples, with few
//! ticking mobs, with a mob whose reach is the whole level, or when one island holds most of them.
use super::lan_mobs::GuestViews;
use horizon_client::{
    fast_hash::{FxHashMap, FxHashSet},
    mobs::{
        ForeignSeen, Mob, MobEvent, MobSlot, MobWorld, PendingPushes, PlayerView, Seen, Species,
        Who, leash, seen_grid::SeenGrid, spawning::LocalDifficulty,
    },
};
use horizon_core::{
    geometry::{Aabb, Axis, Vec3},
    random::{LegacyRandom, OrderedDraws},
};
use std::{
    cell::Cell,
    sync::{Arc, OnceLock},
};

/// Fewer ticking mobs than this tick serially: the plan would cost more than it saves.
const MIN_TICKING: usize = 256;
/// The plan's cell size, in blocks.
const CELL: f64 = 16.0;
/// Cell visits one plan may spend before the tick runs serially instead.
const PLAN_BUDGET: usize = 2_000_000;
/// The least a ticking mob reads around itself: the widest fixed query box of the mob code
/// (`others_near` with 32 blocks), over any smaller follow range.
const MIN_SENSE: f64 = 32.0;
/// How far a reader of the village data (`village_sections`, `others_tickets`) can depend on a
/// villager's points of interest: `sections_to_village` looks up to seven sections away, plus the
/// candidate positions it tests (`land_random_pos` within 15 blocks).
const VILLAGE_SENSE: f64 = 7.0 * 16.0 + 32.0;
/// How far a villager can claim a new point of interest in one tick (the acquisition range).
const POI_CLAIM: f64 = 48.0;
/// Margin added to every reach.
const SLACK: f64 = 2.0;

/// Whether the mob phase may run in groups.
pub(super) fn enabled() -> bool {
    horizon_client::lan::entity::parallel_tick() > 0
}
/// Whether each grouped tick is checked against the serial loop.
pub(super) fn verify() -> bool {
    horizon_client::lan::entity::parallel_tick() == 2
}

/// A mob's place in the loop, as the serial loop decides it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    /// Its chunk isn't loaded: it is kept as it is.
    Unloaded,
    /// Loaded but outside the entity-ticking area: only its despawn check runs.
    Resting,
    /// It ticks.
    Ticking,
}

/// The mobs of each group, by their place in the list, ascending.
pub(super) struct Plan {
    pub groups: Vec<Vec<usize>>,
}

struct Sets {
    parent: Vec<u32>,
}
impl Sets {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n as u32).collect(),
        }
    }
    fn find(&mut self, mut i: u32) -> u32 {
        while self.parent[i as usize] != i {
            let up = self.parent[self.parent[i as usize] as usize];
            self.parent[i as usize] = up;
            i = up;
        }
        i
    }
    fn join(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            // The smaller root wins, so islands keep a stable name.
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            self.parent[hi as usize] = lo;
        }
    }
}

fn grow(b: Aabb, by: f64) -> Aabb {
    b.inflate(Vec3::new(by, by, by))
}
fn cells(b: Aabb) -> Option<(i32, i32, i32, i32)> {
    let (x0, x1) = (b.min(Axis::X), b.max(Axis::X));
    let (z0, z1) = (b.min(Axis::Z), b.max(Axis::Z));
    if !(x0.is_finite() && x1.is_finite() && z0.is_finite() && z1.is_finite()) {
        return None;
    }
    let c = |v: f64| (v / CELL).floor() as i32;
    Some((c(x0), c(z0), c(x1), c(z1)))
}
fn point_box(p: [i32; 3]) -> Aabb {
    let v = Vec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
    Aabb::new(v, Vec3::new(v.x + 1.0, v.y + 1.0, v.z + 1.0))
}
fn union_box(a: Aabb, b: Aabb) -> Aabb {
    Aabb::new(
        Vec3::new(
            a.min(Axis::X).min(b.min(Axis::X)),
            a.min(Axis::Y).min(b.min(Axis::Y)),
            a.min(Axis::Z).min(b.min(Axis::Z)),
        ),
        Vec3::new(
            a.max(Axis::X).max(b.max(Axis::X)),
            a.max(Axis::Y).max(b.max(Axis::Y)),
            a.max(Axis::Z).max(b.max(Axis::Z)),
        ),
    )
}

/// Boxes by the cells they cover.
#[derive(Default)]
struct Cells {
    map: FxHashMap<(i32, i32), Vec<u32>>,
}
impl Cells {
    fn insert(&mut self, i: u32, b: Aabb, budget: &mut usize) -> Option<()> {
        let (x0, z0, x1, z1) = cells(b)?;
        let n = (i64::from(x1 - x0) + 1) * (i64::from(z1 - z0) + 1);
        *budget = budget.checked_sub(usize::try_from(n).ok()?)?;
        for x in x0..=x1 {
            for z in z0..=z1 {
                self.map.entry((x, z)).or_default().push(i);
            }
        }
        Some(())
    }
    fn query(&self, b: Aabb, budget: &mut usize, mut f: impl FnMut(u32)) -> Option<()> {
        let (x0, z0, x1, z1) = cells(b)?;
        let n = (i64::from(x1 - x0) + 1) * (i64::from(z1 - z0) + 1);
        *budget = budget.checked_sub(usize::try_from(n).ok()?)?;
        for x in x0..=x1 {
            for z in z0..=z1 {
                if let Some(list) = self.map.get(&(x, z)) {
                    *budget = budget.checked_sub(list.len())?;
                    list.iter().copied().for_each(&mut f);
                }
            }
        }
        Some(())
    }
}

/// Species whose tick can reach the whole level (the wither's attacks, a warden's suspects kept by
/// UUID): their tick runs serially. The dragon isn't a `Mob` here.
fn global(species: Species) -> bool {
    matches!(species, Species::Wither | Species::Warden)
}
/// The farthest a mob of `species` can be moved in one tick besides its motion: a teleport.
fn jump(species: Species) -> f64 {
    match species {
        // Enderman.teleport: up to 32 blocks on each axis.
        Species::Enderman => 33.0,
        // Shulker.teleportSomewhere: within 8.
        Species::Shulker => 9.0,
        _ => 0.0,
    }
}
/// Mobs whose tick reads every villager's points of interest (`village_sections`,
/// `others_tickets`).
fn reads_villages(species: Species) -> bool {
    matches!(
        species,
        Species::Villager
            | Species::IronGolem
            | Species::Fox
            | Species::Zombie
            | Species::ZombieVillager
            | Species::Husk
            | Species::Drowned
            | Species::ZombifiedPiglin
    )
}

/// The islands of this tick's mobs, packed into at most `lanes` groups; None when the tick should
/// run serially.
pub(super) fn plan(mobs: &[Mob], snapshot: &[Seen], roles: &[Role], lanes: usize) -> Option<Plan> {
    let ticking = roles.iter().filter(|r| **r == Role::Ticking).count();
    if lanes < 2 || ticking < MIN_TICKING || mobs.len() != snapshot.len() {
        return None;
    }
    let n = mobs.len();
    let mut budget = PLAN_BUDGET;
    let mut sets = Sets::new(n);
    // Joining two mobs that can't reach each other only costs parallelism, never the result, so
    // the plan works in columns of CELL blocks: every mob of a column is one island, and a column
    // joins every column its mobs' reaches meet. A column holds the box around its mobs' bodies
    // (each grown by the most it can move: every mob is a body others may read, and a resting one
    // can be despawned), around its ticking mobs' reads, around its villagers' points of interest
    // and around the village reads of the mobs that make them.
    #[derive(Clone, Copy)]
    struct Column {
        first: u32,
        body: Option<Aabb>,
        sense: Option<Aabb>,
        villagers: Option<Aabb>,
        village_reads: Option<Aabb>,
    }
    let widen = |a: Option<Aabb>, b: Aabb| Some(a.map_or(b, |a| union_box(a, b)));
    let mut columns: Vec<Column> = Vec::new();
    let mut column_of = FxHashMap::<(i32, i32), u32>::default();
    let mut index = FxHashMap::<u32, u32>::default();
    for (i, (m, s)) in mobs.iter().zip(snapshot).enumerate() {
        index.entry(s.id).or_insert(i as u32);
        let ticks = roles[i] == Role::Ticking;
        let motion = if ticks {
            if global(s.species) {
                return None;
            }
            let v = m.velocity;
            2.0 * (v.x * v.x + v.y * v.y + v.z * v.z).sqrt() + jump(s.species) + SLACK
        } else {
            0.0
        };
        let body = grow(s.bounds, motion);
        let (x, z) = (s.position.x, s.position.z);
        if !(x.is_finite() && z.is_finite()) {
            return None;
        }
        let key = ((x / CELL).floor() as i32, (z / CELL).floor() as i32);
        let next = columns.len() as u32;
        let c = *column_of.entry(key).or_insert(next);
        if c == next {
            columns.push(Column {
                first: i as u32,
                body: None,
                sense: None,
                villagers: None,
                village_reads: None,
            });
        } else {
            sets.join(columns[c as usize].first, i as u32);
        }
        let col = &mut columns[c as usize];
        col.body = widen(col.body, body);
        if ticks {
            col.sense = widen(
                col.sense,
                grow(body, m.attrs.follow_range.max(MIN_SENSE) + SLACK),
            );
            if reads_villages(s.species) {
                col.village_reads = widen(col.village_reads, grow(body, VILLAGE_SENSE));
            }
        }
        if s.species == Species::Villager {
            // Its claims, and the ones it may make this tick.
            let mut foot = grow(body, POI_CLAIM);
            for p in [s.job, s.bed, s.potential_job, s.meeting]
                .into_iter()
                .flatten()
            {
                foot = union_box(foot, point_box(p));
            }
            col.villagers = widen(col.villagers, foot);
        }
    }
    // Columns by the cells their boxes cover.
    let mut bodies = Cells::default();
    let mut villagers = Cells::default();
    for (c, col) in columns.iter().enumerate() {
        if let Some(b) = col.body {
            bodies.insert(c as u32, b, &mut budget)?;
        }
        if let Some(v) = col.villagers {
            villagers.insert(c as u32, v, &mut budget)?;
        }
    }
    for col in &columns {
        if let Some(sense) = col.sense {
            bodies.query(sense, &mut budget, |j| {
                let other = &columns[j as usize];
                if other.body.is_some_and(|b| b.intersects(sense)) {
                    sets.join(col.first, other.first);
                }
            })?;
        }
        if let Some(reads) = col.village_reads {
            villagers.query(reads, &mut budget, |j| {
                let other = &columns[j as usize];
                if other.villagers.is_some_and(|v| v.intersects(reads)) {
                    sets.join(col.first, other.first);
                }
            })?;
        }
    }
    let link = |sets: &mut Sets, i: usize, id: u32| {
        if let Some(&j) = index.get(&id) {
            sets.join(i as u32, j);
        }
    };
    let mut raids = FxHashMap::<i32, u32>::default();
    for (i, (m, s)) in mobs.iter().zip(snapshot).enumerate() {
        // Ids it can reach at any distance.
        if let Some(Who::Mob(id)) = s.target {
            link(&mut sets, i, id);
        }
        if let Some((Who::Mob(id), _)) = s.hurt_by {
            link(&mut sets, i, id);
        }
        if let Some(leash::Holder::Mob(id)) = s.leash {
            link(&mut sets, i, id);
        }
        if let Some(id) = s.vehicle {
            link(&mut sets, i, id);
        }
        if let Some(id) = m.extra.mount {
            link(&mut sets, i, id);
        }
        if let Some(id) = m.extra.rider {
            link(&mut sets, i, id);
        }
        if s.caravan > 0 {
            link(&mut sets, i, s.caravan - 1);
        }
        if let Some(id) = s.school.0 {
            link(&mut sets, i, id);
        }
        if matches!(
            s.species,
            Species::Vex | Species::Evoker | Species::Illusioner
        ) && m.extra.counter2 > 0
        {
            link(&mut sets, i, m.extra.counter2 as u32);
        }
        // A raid's raiders order their hurts across the raid (`raider::next_hurt_seq`).
        if let Some(raid) = horizon_client::mobs::raider::state(m).and_then(|r| r.raid_id) {
            let first = *raids.entry(raid).or_insert(i as u32);
            sets.join(first, i as u32);
        }
    }
    // Islands, in the order of their first mob; their ticking mobs are their weight.
    let mut islands: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut slot = FxHashMap::<u32, usize>::default();
    for i in 0..n {
        let root = sets.find(i as u32);
        let k = *slot.entry(root).or_insert_with(|| {
            islands.push((0, Vec::new()));
            islands.len() - 1
        });
        islands[k].1.push(i);
        if roles[i] == Role::Ticking {
            islands[k].0 += 1;
        }
    }
    let largest = islands.iter().map(|i| i.0).max().unwrap_or(0);
    if largest * 4 > ticking * 3 {
        return None;
    }
    // Heaviest first, each to the lightest group.
    islands.sort_by_key(|a| std::cmp::Reverse(a.0));
    let mut groups: Vec<(usize, Vec<usize>)> = vec![(0, Vec::new()); lanes];
    for (weight, members) in islands {
        let g = groups
            .iter_mut()
            .min_by_key(|g| g.0)
            .expect("at least two lanes");
        g.0 += weight;
        g.1.extend(members);
    }
    let mut groups: Vec<Vec<usize>> = groups
        .into_iter()
        .filter(|g| !g.1.is_empty())
        .map(|g| g.1)
        .collect();
    for g in &mut groups {
        g.sort_unstable();
    }
    (groups.len() > 1).then_some(Plan { groups })
}

/// What every group reads and none writes.
pub(super) struct Shared<'a> {
    pub uuid: Option<[i32; 4]>,
    /// The host's own player's UUID, when one is in the level (`Game::who_by_uuid`).
    pub host_uuid: Option<[i32; 4]>,
    pub level_id: &'static str,
    pub path_changes: &'a [[i32; 3]],
    pub guest_ids: &'a [(u32, u32)],
    pub roles: &'a [Role],
    /// Whether native mods listen (`native_living_change_target`).
    pub mods: bool,
}

/// One group's mobs and everything the loop keeps for them, in their list order. The serial
/// loop is one group holding every mob.
pub(super) struct Group<M: MobSlot> {
    /// Each member's place in the whole list.
    pub global: Vec<usize>,
    pub mobs: Vec<M>,
    pub snapshot: Vec<Seen>,
    grid: SeenGrid,
    index: FxHashMap<u32, u32>,
    villager_slots: Vec<u32>,
    solid_slots: Vec<u32>,
    pub nearest: Vec<Option<Vec3>>,
    pub pushes: PendingPushes,
    pub events: Vec<MobEvent>,
    /// Each member's events: `events[ranges[k].0..ranges[k].1]`.
    pub ranges: Vec<(usize, usize)>,
    pub random: LegacyRandom,
    pub guests: GuestViews,
    spare: Option<PlayerView>,
    pub player: Option<PlayerView>,
    pub bindings: FxHashMap<u32, u32>,
    pub died: FxHashSet<u32>,
    pub ticked: Vec<u32>,
    pub targets: Vec<Option<Who>>,
    pub keep: Vec<bool>,
    pub dead: Vec<(usize, Mob)>,
    pub corpses: Vec<(usize, Mob)>,
    pub gone: Vec<(usize, super::creatures::Gone)>,
    pub guest_events: Vec<(usize, (u32, u32, MobEvent))>,
}

impl<M: MobSlot> Group<M> {
    /// A group of `mobs`, whose snapshot entries are `snapshot` and nearest players `nearest`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        global: Vec<usize>,
        mobs: Vec<M>,
        snapshot: Vec<Seen>,
        nearest: Vec<Option<Vec3>>,
        events: Vec<MobEvent>,
        random: LegacyRandom,
        guests: GuestViews,
        player: Option<PlayerView>,
        bindings: FxHashMap<u32, u32>,
        died: FxHashSet<u32>,
        whole: Option<(f64, usize)>,
    ) -> Self {
        let grid = SeenGrid::new(snapshot.iter().map(|s| s.bounds));
        // A group's grid meets its mobs as the whole list's grid would.
        let grid = match whole {
            Some(bounds) => grid.answering_as(bounds),
            None => grid,
        };
        // Each mob keeps its snapshot slot all tick; the first slot of an
        // id answers, as a scan would.
        let mut index = FxHashMap::<u32, u32>::default();
        let mut villager_slots = Vec::new();
        let mut solid_slots = Vec::new();
        for (i, s) in snapshot.iter().enumerate() {
            index.entry(s.id).or_insert(i as u32);
            if s.species == Species::Villager {
                villager_slots.push(i as u32);
            }
            if s.may_block_mobs() {
                solid_slots.push(i as u32);
            }
        }
        // What stands in a guest's slot while a mob holds that guest's view for
        // its tick (see `lent` in `tick`).
        let spare = guests.first().map(|g| g.1.clone());
        let targets = mobs.iter().map(|m| m.mob().ai.target).collect();
        let n = mobs.len();
        Self {
            global,
            mobs,
            snapshot,
            grid,
            index,
            villager_slots,
            solid_slots,
            nearest,
            pushes: PendingPushes::default(),
            events,
            ranges: Vec::with_capacity(n),
            random,
            guests,
            spare,
            player,
            bindings,
            died,
            ticked: Vec::new(),
            targets,
            keep: Vec::with_capacity(n),
            dead: Vec::new(),
            corpses: Vec::new(),
            gone: Vec::new(),
            guest_events: Vec::new(),
        }
    }

    /// Every member's turn, in order.
    pub(super) fn run(
        &mut self,
        base: &MobWorld,
        shared: &Shared,
        difficulty: &mut impl FnMut(usize, [f64; 3]) -> LocalDifficulty,
        canceled: &impl Fn(u32) -> bool,
    ) {
        self.run_with(base, shared, difficulty, canceled, None);
    }
    fn run_with(
        &mut self,
        base: &MobWorld,
        shared: &Shared,
        difficulty: &mut impl FnMut(usize, [f64; 3]) -> LocalDifficulty,
        canceled: &impl Fn(u32) -> bool,
        foreign: Option<&Foreign>,
    ) {
        for i in 0..self.mobs.len() {
            let at = self.global[i];
            self.random.begin_item(at);
            if let Some(f) = foreign {
                f.reader.set(at);
            }
            let from = self.events.len();
            self.turn(i, base, shared, difficulty, canceled);
            self.ranges.push((from, self.events.len()));
            if let Some(f) = foreign {
                let _ = f.board.published[at].set(self.snapshot[i].clone());
            }
            self.random.end_item();
        }
        self.random.finish_lane();
    }

    /// Member `i`'s turn of the loop: its tick (or despawn check), then what the loop does with
    /// what it did.
    fn turn(
        &mut self,
        i: usize,
        base: &MobWorld,
        shared: &Shared,
        difficulty: &mut impl FnMut(usize, [f64; 3]) -> LocalDifficulty,
        canceled: &impl Fn(u32) -> bool,
    ) {
        let at = self.global[i];
        let m = self.mobs[i].mob_mut();
        let seen_guest = if self.guests.is_empty() {
            None
        } else {
            let bound = self.bindings.get(&m.id).copied();
            let conn = super::lan_mobs::player_for(m, self.player.as_ref(), &self.guests, bound);
            match conn {
                Some(conn) => {
                    self.bindings.insert(m.id, conn);
                }
                None => {
                    self.bindings.remove(&m.id);
                }
            }
            conn
        };
        if shared.roles[at] == Role::Unloaded {
            self.keep.push(!m.hostile() || m.persistent);
            return;
        }
        let nearest_player = self.nearest[i];
        // ServerLevel.tick: every loaded entity checks despawning, but
        // only those in entity-ticking chunks (within the simulation
        // distance) tick. The despawn check reads the nearest player
        // and the difficulty, not the player the mob sees.
        if shared.roles[at] == Role::Resting {
            let w = MobWorld {
                others: &self.snapshot,
                others_grid: Some(&self.grid),
                others_index: Some(&self.index),
                others_villagers: Some(&self.villager_slots),
                others_solid: Some(&self.solid_slots),
                nearest_player,
                player: None,
                teams: std::sync::Arc::clone(&base.teams),
                ..*base
            };
            let kept = horizon_client::mobs::ai::despawn_check(m, &w);
            if !kept {
                self.died.remove(&m.id);
                self.snapshot[i].alive = false;
            }
            self.keep.push(kept);
            return;
        }
        let controlling_player_movement = horizon_client::mobs::equine::controlled(m)
            .then(|| {
                self.player
                    .as_ref()
                    .filter(|p| p.riding_vehicle == Some(m.id))
                    .map(|p| p.velocity)
                    .or_else(|| {
                        self.guests
                            .iter()
                            .find(|(_, p)| p.riding_vehicle == Some(m.id))
                            .map(|(_, p)| p.velocity)
                    })
            })
            .flatten();
        // A mod canceled its tick: it stays as it is.
        if canceled(m.id) {
            self.keep.push(true);
            return;
        }
        // getCurrentDifficultyAt(blockPosition()): the mob's own chunk,
        // not the local player's (a server thread has no player).
        let difficulty = difficulty(at, [m.position.x, m.position.y, m.position.z]);
        horizon_client::mobs::apply_mob_pushes(m, &mut self.pushes);
        horizon_client::mobs::blocks_changed(m, shared.path_changes);
        let from = self.events.len();
        // The guest's view is lent to the mob for its tick, not copied:
        // the tick only reads it (`&MobWorld`), and it goes back to the
        // guest's slot straight after.
        let lent = seen_guest.and_then(|conn| self.guests.iter().position(|g| g.0 == conn));
        let active_player = match (seen_guest, lent) {
            (None, _) => self.player.take(),
            (Some(_), Some(k)) => self
                .spare
                .take()
                .map(|stand_in| std::mem::replace(&mut self.guests[k].1, stand_in)),
            (Some(_), None) => None,
        };
        let mut w = MobWorld {
            others: &self.snapshot,
            others_grid: Some(&self.grid),
            others_index: Some(&self.index),
            others_villagers: Some(&self.villager_slots),
            others_solid: Some(&self.solid_slots),
            difficulty,
            nearest_player,
            player: active_player,
            teams: std::sync::Arc::clone(&base.teams),
            ..*base
        };
        w.controlling_player_movement = controlling_player_movement;
        self.ticked.push(self.mobs[i].mob().id);
        // `horizon-report`'s cost by mob type (tick_profile.rs).
        let sampled = super::tick_profile::start().map(|t| (t, self.mobs[i].mob().kind.clone()));
        let kept = horizon_client::mobs::riders::tick(
            &mut self.mobs,
            i,
            &w,
            &mut self.random,
            &mut self.events,
        );
        if let Some((started, kind)) = sampled {
            super::tick_profile::add(shared.level_id, &kind, started);
        }
        let m = self.mobs[i].mob_mut();
        if seen_guest.is_none() {
            self.player = w.player;
        } else if let Some(k) = lent {
            self.spare = w
                .player
                .map(|v| std::mem::replace(&mut self.guests[k].1, v));
        }
        // Its tick changed its target: the mods may keep the one it had
        // (`native_living_change_target`), and then what the tick did to
        // the new one is not dealt.
        if let Some(&before) = self.targets.get(i)
            && m.ai.target != before
        {
            let named = if shared.mods {
                super::creatures::target_named(
                    m.ai.target,
                    seen_guest,
                    &self.guests,
                    shared.guest_ids,
                    self.player.as_ref(),
                )
            } else {
                None
            };
            if let Some((player, mob)) = named {
                let kind = super::native_hooks::mob_kind(m);
                let mut cancel = false;
                super::Game::native_living_change_target(
                    shared.level_id,
                    m.id,
                    &kind,
                    player,
                    mob,
                    &mut cancel,
                );
                if cancel {
                    if let Some(new) = m.ai.target {
                        let mut k = from;
                        while k < self.events.len() {
                            if super::creatures::hits_target(&self.events[k], new) {
                                self.events.remove(k);
                            } else {
                                k += 1;
                            }
                        }
                    }
                    m.ai.target = before;
                }
            }
            self.targets[i] = m.ai.target;
        }
        // A killed mob's body is removed: its effects that act on
        // death run (`LivingEntity.remove` with KILLED).
        if !kept && !m.alive() && !m.removed && !m.extra.effects.is_empty() {
            self.corpses.push((at, m.clone()));
        }
        horizon_client::mobs::take_mob_pushes(&mut self.events, from, &mut self.pushes);
        // LivingEntity.tickDeath: after twenty ticks the body is removed.
        if !kept && !m.removed && !m.alive() && m.death_time >= 20 {
            let (width, height) = (f64::from(m.width()), f64::from(m.height()));
            // `AbstractCubeMob.remove` splits a grown cube into its smaller halves when it is
            // removed, twenty ticks after the death. The parent leaves the list before the split
            // runs, so its size, position, kind and persistence are captured here, as they are
            // at the removal.
            let split = if matches!(
                m.species(),
                Species::Slime | Species::MagmaCube | Species::SulfurCube
            ) && m.extra.size > 1
            {
                Some(m.clone())
            } else {
                None
            };
            self.gone
                .push((at, (m.id, m.position, width, height, split)));
        }
        if let Some(conn) = seen_guest {
            // ShulkerBullet.finalTarget is the guest's entity: the
            // bullet homes on it while the projectiles tick, where
            // `Who::Player` is the host's own player.
            if let Some(&(_, id)) = shared.guest_ids.iter().find(|g| g.0 == conn) {
                for e in &mut self.events[from..] {
                    if let MobEvent::Projectile(p) = e
                        && let horizon_client::projectiles::ProjectileKind::ShulkerBullet {
                            target,
                            ..
                        } = &mut p.kind
                        && *target == Some(Who::Player)
                    {
                        *target = Some(Who::Mob(id));
                    }
                }
            }
            let mut k = from;
            while k < self.events.len() {
                if super::lan_mobs::acts_on_player(&self.events[k]) {
                    self.guest_events
                        .push((at, (conn, m.id, self.events.remove(k))));
                } else {
                    k += 1;
                }
            }
        }
        // Killed during its own tick (drowning, burning, falling...).
        if !m.alive() && !m.removed && self.died.insert(m.id) {
            self.dead.push((at, m.clone()));
        }
        if !kept {
            self.died.remove(&m.id);
        }
        self.keep.push(kept);
        // ServerLevel ticks entities one after another: the ones
        // after this mob see it as it is now.
        self.snapshot[i] = super::creatures::seen(m, shared.uuid);
        self.snapshot[i].alive &= kept;
        self.grid.update(i, self.snapshot[i].bounds);
        horizon_client::mobs::apply_brain_events(&mut self.mobs, &mut self.events, from);
    }
}

/// Every mob's snapshot entry across the groups, for a lookup by id of a mob in another group
/// (a remembered mob far away): the serial loop shows a mob after its turn to the mobs after it,
/// and as it was before the tick to the mobs before it.
pub(super) struct Board {
    /// The snapshot as the tick started.
    pub frozen: Vec<Seen>,
    /// Each mob's entry once its turn is over.
    published: Vec<OnceLock<Seen>>,
    /// The first slot of each id, as the serial snapshot's index.
    index: FxHashMap<u32, u32>,
    /// Each mob's group.
    lane_of: Vec<u32>,
    draws: Arc<OrderedDraws>,
}
impl Board {
    pub(super) fn new<M: MobSlot>(
        frozen: Vec<Seen>,
        groups: &[Group<M>],
        draws: Arc<OrderedDraws>,
    ) -> Self {
        let mut index = FxHashMap::default();
        for (i, s) in frozen.iter().enumerate() {
            index.entry(s.id).or_insert(i as u32);
        }
        let mut lane_of = vec![0; frozen.len()];
        for (lane, g) in groups.iter().enumerate() {
            for &k in &g.global {
                lane_of[k] = lane as u32;
            }
        }
        Self {
            published: (0..frozen.len()).map(|_| OnceLock::new()).collect(),
            frozen,
            index,
            lane_of,
            draws,
        }
    }
}
/// A group's view of the [`Board`] at the turn of mob `reader` (its place in the list).
struct Foreign<'b> {
    board: &'b Board,
    reader: Cell<usize>,
}
impl ForeignSeen for Foreign<'_> {
    fn seen(&self, id: u32) -> Option<&Seen> {
        let j = *self.board.index.get(&id)? as usize;
        let reader = self.reader.get();
        let s = if j > reader {
            &self.board.frozen[j]
        } else {
            self.board
                .draws
                .wait_past(self.board.lane_of[j] as usize, j);
            self.board.published[j].get()?
        };
        (s.id == id).then_some(s)
    }
}

/// `Game::who_by_uuid` while the mob loop runs: the game's mob list is taken for the loop, so
/// only the host's own player answers.
struct LoopLookup(Option<[i32; 4]>);
impl horizon_client::mobs::WhoLookup for LoopLookup {
    fn who_by_uuid(&self, uuid: [i32; 4]) -> Option<Who> {
        (self.0 == Some(uuid)).then_some(Who::Player)
    }
}

/// `base` without its hooks into the game, which can't leave the server thread, as a recipe a
/// helper thread builds its own world from: native mod effects (none run on helpers: the phases
/// that use them stay serial with native mods), the UUID lookup and the foreign mobs (set by the
/// caller).
pub(super) fn hookless<'w>(base: &MobWorld<'w>) -> impl Fn() -> MobWorld<'w> + Sync + use<'w> {
    // Every field but the two hooks, named, so that a field added to `MobWorld` is decided here.
    let MobWorld {
        world,
        registry,
        facts,
        fluid_tables,
        movement,
        collision,
        suffocation,
        data,
        player,
        controlling_player_movement,
        nearest_player,
        others,
        dragon,
        leash_boats,
        others_grid,
        others_index,
        others_villagers,
        others_solid,
        daylight,
        monsters_burn,
        game_time,
        day_time,
        raining,
        thundering,
        level: level_key,
        block_tags,
        landing,
        biomes,
        mob_griefing,
        universal_anger,
        mob_drops,
        max_entity_cramming,
        difficulty: base_difficulty,
        dimension_type,
        world_seed,
        sea_level,
        teams,
        items,
        lid_openers,
        mod_effects: _,
        who_lookup: _,
        foreign: _,
        border,
    } = base.clone();
    move || MobWorld {
        world,
        registry,
        facts,
        fluid_tables,
        movement,
        collision,
        suffocation,
        data,
        player: player.clone(),
        controlling_player_movement,
        nearest_player,
        others,
        dragon,
        leash_boats,
        others_grid,
        others_index,
        others_villagers,
        others_solid,
        daylight,
        monsters_burn,
        game_time,
        day_time,
        raining,
        thundering,
        level: level_key,
        block_tags,
        landing,
        biomes,
        mob_griefing,
        universal_anger,
        mob_drops,
        max_entity_cramming,
        difficulty: base_difficulty,
        dimension_type,
        world_seed,
        sea_level,
        teams: std::sync::Arc::clone(&teams),
        items,
        lid_openers,
        mod_effects: None,
        who_lookup: None,
        foreign: None,
        border,
    }
}

/// `f` over `items` on up to `lanes` threads (this one included), in order: the results as one
/// thread would give them. One thread for fewer than `min` items.
pub(super) fn par_map<T: Sync, R: Send>(
    items: &[T],
    lanes: usize,
    min: usize,
    f: impl Fn(&[T], &mut Vec<R>) + Sync,
) -> Vec<R> {
    let mut out = Vec::with_capacity(items.len());
    if lanes <= 1 || items.len() < min {
        f(items, &mut out);
        return out;
    }
    let size = items.len().div_ceil(lanes);
    let parts: Vec<Vec<R>> = std::thread::scope(|s| {
        let f = &f;
        let mut chunks = items.chunks(size);
        let first = chunks.next().unwrap_or(&[]);
        let handles: Vec<_> = chunks
            .map(|c| {
                s.spawn(move || {
                    let mut part = Vec::with_capacity(c.len());
                    f(c, &mut part);
                    part
                })
            })
            .collect();
        let mut head = Vec::with_capacity(first.len());
        f(first, &mut head);
        std::iter::once(head)
            .chain(
                handles
                    .into_iter()
                    .map(|h| h.join().expect("a helper thread panicked")),
            )
            .collect()
    });
    for p in parts {
        out.extend(p);
    }
    out
}

/// The threads a parallel phase of the tick may take (`horizon-threads`' count, at most 8), or 1
/// while the parallel tick is off.
pub(super) fn thread_lanes() -> usize {
    if enabled() {
        horizon_threads::worker_count().clamp(1, 8)
    } else {
        1
    }
}

/// Run `groups` at once, one thread each (the first on this one), each with the thread-local
/// settings of this thread. The groups' world is `base` without its hooks into the game, which
/// can't leave this thread: native mod effects (none: groups only run without native mods) and
/// the UUID lookup ([`LoopLookup`], the same answers).
pub(super) fn run_parallel(
    groups: &mut [Group<&mut Mob>],
    board: &Board,
    base: &MobWorld,
    shared: &Shared,
    difficulty: &(impl Fn(usize) -> LocalDifficulty + Sync),
) {
    let level = horizon_client::block_contact::current_level();
    let tnt = horizon_client::mobs::sulfur_cube::tnt_explodes();
    let rebuild = hookless(base);
    let host = shared.host_uuid;
    let run = |g: &mut Group<&mut Mob>| {
        let lookup = LoopLookup(host);
        let foreign = Foreign {
            board,
            reader: Cell::new(0),
        };
        let mut base = rebuild();
        base.who_lookup = Some(&lookup);
        base.foreign = Some(&foreign);
        g.run_with(
            &base,
            shared,
            &mut |at, _| difficulty(at),
            &|_| false,
            Some(&foreign),
        );
    };
    let (first, rest) = groups.split_first_mut().expect("a plan has groups");
    std::thread::scope(|s| {
        for g in rest {
            let run = &run;
            s.spawn(move || {
                horizon_client::mobs::sulfur_cube::set_tnt_explodes(tnt);
                horizon_client::block_contact::in_level(level, || run(g));
            });
        }
        run(first);
    });
}

/// The level random each group draws from: lanes of one ordered stream.
pub(super) fn lanes(
    random: &LegacyRandom,
    n: usize,
) -> (std::sync::Arc<OrderedDraws>, Vec<LegacyRandom>) {
    let draws = OrderedDraws::new(random, n);
    let lanes = (0..n).map(|k| draws.lane(k)).collect();
    (draws, lanes)
}

/// The groups' results put back in list order, as one serial loop would have left them.
pub(super) struct Merged {
    pub snapshot: Vec<Seen>,
    pub keep: Vec<bool>,
    pub events: Vec<MobEvent>,
    pub pushes: PendingPushes,
    pub random: LegacyRandom,
    pub guests: GuestViews,
    pub player: Option<PlayerView>,
    pub bindings: FxHashMap<u32, u32>,
    pub died: FxHashSet<u32>,
    pub ticked: Vec<u32>,
    pub targets: Vec<Option<Who>>,
    pub dead: Vec<Mob>,
    pub corpses: Vec<Mob>,
    pub gone: Vec<super::creatures::Gone>,
    pub guest_events: Vec<(u32, u32, MobEvent)>,
}

fn strip(v: Vec<(usize, Mob)>) -> Vec<Mob> {
    v.into_iter().map(|(_, m)| m).collect()
}

/// The serial loop's one group, as it left everything, and its mobs.
pub(super) fn merge_one(g: Group<Mob>) -> (Merged, Vec<Mob>) {
    (
        Merged {
            snapshot: g.snapshot,
            keep: g.keep,
            events: g.events,
            pushes: g.pushes,
            random: g.random,
            guests: g.guests,
            player: g.player,
            bindings: g.bindings,
            died: g.died,
            ticked: g.ticked,
            targets: g.targets,
            dead: strip(g.dead),
            corpses: strip(g.corpses),
            gone: g.gone.into_iter().map(|(_, x)| x).collect(),
            guest_events: g.guest_events.into_iter().map(|(_, x)| x).collect(),
        },
        g.mobs,
    )
}

/// Merge the `groups` over `n` mobs, which ticked their mobs in place: what the serial loop would
/// have left, in list order. `draws` is their shared level random; `bindings` and `died` are the
/// loop's maps from before it, of which each group held its own mobs' entries.
pub(super) fn merge<M: MobSlot>(
    mut groups: Vec<Group<M>>,
    n: usize,
    draws: &OrderedDraws,
    mut bindings: FxHashMap<u32, u32>,
    mut died: FxHashSet<u32>,
) -> Merged {
    // Where each mob of the list went: (group, place in it).
    let mut place = vec![(0u32, 0u32); n];
    for (gi, g) in groups.iter().enumerate() {
        for (li, &k) in g.global.iter().enumerate() {
            place[k] = (gi as u32, li as u32);
        }
    }
    let mut keep = Vec::with_capacity(n);
    let mut targets = Vec::with_capacity(n);
    let mut events = Vec::new();
    for &(gi, li) in &place {
        let g = &groups[gi as usize];
        keep.push(g.keep[li as usize]);
        targets.push(g.targets[li as usize]);
        let (from, to) = g.ranges[li as usize];
        events.extend_from_slice(&g.events[from..to]);
    }
    // Each member's binding and death mark as its group left them.
    for g in &groups {
        for m in g.mobs.iter().map(MobSlot::mob) {
            match g.bindings.get(&m.id) {
                Some(&conn) => {
                    bindings.insert(m.id, conn);
                }
                None => {
                    bindings.remove(&m.id);
                }
            }
            if g.died.contains(&m.id) {
                died.insert(m.id);
            } else {
                died.remove(&m.id);
            }
        }
    }
    let mut ticked = Vec::new();
    let mut dead = Vec::new();
    let mut corpses = Vec::new();
    let mut gone = Vec::new();
    let mut guest_events = Vec::new();
    let mut pushes = PendingPushes::default();
    let first = groups.first_mut().expect("groups");
    let guests = std::mem::take(&mut first.guests);
    let player = first.player.take();
    for g in groups {
        ticked.extend(g.ticked);
        dead.extend(g.dead);
        corpses.extend(g.corpses);
        gone.extend(g.gone);
        guest_events.extend(g.guest_events);
        pushes.append(g.pushes);
    }
    // Stable: one mob's own entries keep their order.
    dead.sort_by_key(|e| e.0);
    corpses.sort_by_key(|e| e.0);
    gone.sort_by_key(|e| e.0);
    guest_events.sort_by_key(|e| e.0);
    Merged {
        // The snapshot buffer the next tick fills: the groups' entries are this tick's only.
        snapshot: Vec::new(),
        keep,
        events,
        pushes,
        random: draws.finish(),
        guests,
        player,
        bindings,
        died,
        ticked,
        targets,
        dead: strip(dead),
        corpses: strip(corpses),
        gone: gone.into_iter().map(|(_, x)| x).collect(),
        guest_events: guest_events.into_iter().map(|(_, x)| x).collect(),
    }
}

/// The first difference between the serial loop's result and the groups', if any.
pub(super) fn difference(
    serial: &Merged,
    serial_mobs: &[Mob],
    grouped: &Merged,
    grouped_mobs: &[Mob],
) -> Option<String> {
    if serial_mobs.len() != grouped_mobs.len() {
        return Some(format!(
            "{} mobs serially, {} in groups",
            serial_mobs.len(),
            grouped_mobs.len()
        ));
    }
    // `section_seq` comes from one counter the copies and the groups both take from: only the
    // order of the mobs in one section counts (`aquatic.rs`), and those share a group.
    let plain = |m: &Mob| {
        let mut m = m.clone();
        m.extra.section_seq = 0;
        format!("{m:?}")
    };
    for (ma, b) in serial_mobs.iter().zip(grouped_mobs) {
        let (a, b) = (plain(ma), plain(b));
        if a != b {
            let what = format!("mob {} ({}) at {:?}", ma.id, ma.kind.id, ma.position);
            let at = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
            let from = at.saturating_sub(120);
            return Some(format!(
                "{what} differs: serial ...{}... grouped ...{}...",
                &a[from..(at + 120).min(a.len())],
                &b[from..(at + 120).min(b.len())]
            ));
        }
    }
    if serial.keep != grouped.keep {
        return Some("the kept mobs differ".into());
    }
    let (a, b) = (
        format!("{:?}", serial.events),
        format!("{:?}", grouped.events),
    );
    if a != b {
        return Some(format!(
            "the events differ ({} serially, {} in groups)",
            serial.events.len(),
            grouped.events.len()
        ));
    }
    if serial.random.state() != grouped.random.state() {
        return Some("the level random differs".into());
    }
    if serial.died != grouped.died || serial.bindings != grouped.bindings {
        return Some("the death marks or player bindings differ".into());
    }
    let ids = |v: &[Mob]| v.iter().map(|m| m.id).collect::<Vec<_>>();
    if ids(&serial.dead) != ids(&grouped.dead)
        || ids(&serial.corpses) != ids(&grouped.corpses)
        || serial
            .gone
            .iter()
            .map(|g| g.0)
            .ne(grouped.gone.iter().map(|g| g.0))
        || format!("{:?}", serial.guest_events) != format!("{:?}", grouped.guest_events)
    {
        return Some("the deaths, removals or guest events differ".into());
    }
    if format!("{:?}", serial.targets) != format!("{:?}", grouped.targets) {
        return Some("the targets differ".into());
    }
    None
}

/// `HORIZON_PARALLEL_TICK=verify`: log a grouped tick that differs from the serial loop, and how
/// many grouped ticks were checked, every 100.
pub(super) fn report_check(
    difference: Option<String>,
    groups: usize,
    detail: impl FnOnce() -> String,
) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CHECKED: AtomicU64 = AtomicU64::new(0);
    static DIFFERED: AtomicU64 = AtomicU64::new(0);
    if let Some(diff) = difference {
        DIFFERED.fetch_add(1, Ordering::Relaxed);
        log::error!(
            "mob islands: a tick in {groups} groups differs from the serial loop: {diff} [{}]",
            detail()
        );
    }
    let n = CHECKED.fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 || n.is_multiple_of(100) {
        log::info!(
            "mob islands: {n} grouped ticks checked against the serial loop, {} differed (this one: {groups} groups)",
            DIFFERED.load(Ordering::Relaxed)
        );
    }
}

/// For a logged difference: each mob's group, role and place, with the mobs `id` has in its
/// line-of-sight cache.
pub(super) fn describe(
    id: u32,
    mobs: &[Mob],
    group_of: &FxHashMap<u32, (usize, usize)>,
    roles: &[Role],
) -> String {
    let Some(m) = mobs.iter().find(|m| m.id == id) else {
        return String::new();
    };
    let role = |k: usize| match roles.get(k) {
        Some(Role::Ticking) => "ticking",
        Some(Role::Resting) => "resting",
        Some(Role::Unloaded) => "unloaded",
        None => "?",
    };
    let mut out = format!("reader {id} group/place {:?}", group_of.get(&id));
    for (who, _) in &m.ai.seen_cache {
        if let Who::Mob(o) = who {
            let at = mobs.iter().find(|x| x.id == *o).map(|x| {
                let d = x.position.subtract(m.position);
                (
                    x.kind.id.clone(),
                    (d.x * d.x + d.y * d.y + d.z * d.z).sqrt(),
                )
            });
            let g = group_of.get(o);
            out += &format!(
                "; {o}: group/place {g:?} {} {at:?}",
                g.map_or("", |g| role(g.1))
            );
        }
    }
    out
}

/// `HORIZON_PARALLEL_TICK=verify`: log an item whose move worked out ahead differs from its move
/// at its turn, and how many were checked, every 10,000.
pub(super) fn report_item_check(same: bool, id: u32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CHECKED: AtomicU64 = AtomicU64::new(0);
    static DIFFERED: AtomicU64 = AtomicU64::new(0);
    if !same {
        DIFFERED.fetch_add(1, Ordering::Relaxed);
        log::error!("parallel items: item {id}'s move worked out ahead differs from its turn's");
    }
    let n = CHECKED.fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 || n.is_multiple_of(10_000) {
        log::info!(
            "parallel items: {n} item moves checked against their turns, {} differed",
            DIFFERED.load(Ordering::Relaxed)
        );
    }
}

/// `HORIZON_PARALLEL_TICK=verify`: how the items' moves worked out ahead were used (none worked out,
/// still fitting, changed by a merge), logged every 20,000 items.
pub(super) fn count_item_ahead(fits: Option<bool>) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTS: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    let k = match fits {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    COUNTS[k].fetch_add(1, Ordering::Relaxed);
    let n: u64 = COUNTS.iter().map(|c| c.load(Ordering::Relaxed)).sum();
    if n.is_multiple_of(20_000) {
        log::info!(
            "parallel items: {} not worked out, {} used, {} changed by a merge",
            COUNTS[0].load(Ordering::Relaxed),
            COUNTS[1].load(Ordering::Relaxed),
            COUNTS[2].load(Ordering::Relaxed)
        );
    }
}
