//! Immutable, thread-shareable mob tick inputs. Native callbacks stay on the
//! coordinator and are never hidden inside a shared world reference.
use super::*;

#[derive(Clone)]
pub struct TickResources<'a> {
    pub world: &'a World,
    pub registry: &'a horizon_world::registry::BlockRegistry,
    pub facts: &'a BlockFacts,
    pub fluid_tables: &'a crate::motion::FluidResources,
    pub movement: &'a horizon_world::movement_properties::BoundMovementProperties,
    pub collision: &'a BlockCollisionRegistry<'a>,
    pub suffocation: &'a horizon_world::suffocation::SuffocationRegistry<'a>,
    pub data: &'a MobData,
    pub player: Option<PlayerView>,
    pub controlling_player_movement: Option<Vec3>,
    pub nearest_player: Option<Vec3>,
    pub dragon: Option<(u32, TargetInfo)>,
    pub leash_boats: &'a [(u32, leash::HolderState)],
    pub daylight: bool,
    pub monsters_burn: bool,
    pub game_time: i64,
    pub day_time: i64,
    pub raining: bool,
    pub thundering: bool,
    pub nether: bool,
    pub block_tags: &'a HashMap<String, std::collections::HashSet<String>>,
    pub landing: &'a horizon_world::block_landing::BlockLandingRegistry,
    pub biomes: &'a crate::biome::Biomes,
    pub mob_griefing: bool,
    pub universal_anger: bool,
    pub mob_drops: bool,
    pub max_entity_cramming: i32,
    pub difficulty: spawning::LocalDifficulty,
    pub end: bool,
    pub world_seed: i64,
    pub sea_level: i32,
    pub teams: std::sync::Arc<Teams<'a>>,
    pub items: Option<&'a crate::items::Items>,
    pub lid_openers: &'a HashMap<(i32, i32, i32), i32>,
    pub border: Option<&'a horizon_core::geometry::border::BorderGeometry>,
}
impl<'a> TickResources<'a> {
    pub fn new(world: &MobWorld<'a>) -> Self {
        Self {
            world: world.world,
            registry: world.registry,
            facts: world.facts,
            fluid_tables: world.fluid_tables,
            movement: world.movement,
            collision: world.collision,
            suffocation: world.suffocation,
            data: world.data,
            player: world.player.clone(),
            controlling_player_movement: world.controlling_player_movement,
            nearest_player: world.nearest_player,
            dragon: world.dragon,
            leash_boats: world.leash_boats,
            daylight: world.daylight,
            monsters_burn: world.monsters_burn,
            game_time: world.game_time,
            day_time: world.day_time,
            raining: world.raining,
            thundering: world.thundering,
            nether: world.nether,
            block_tags: world.block_tags,
            landing: world.landing,
            biomes: world.biomes,
            mob_griefing: world.mob_griefing,
            universal_anger: world.universal_anger,
            mob_drops: world.mob_drops,
            max_entity_cramming: world.max_entity_cramming,
            difficulty: world.difficulty,
            end: world.end,
            world_seed: world.world_seed,
            sea_level: world.sea_level,
            teams: world.teams.clone(),
            items: world.items,
            lid_openers: world.lid_openers,
            border: world.border,
        }
    }
    pub fn view<'s>(
        &'s self,
        player: Option<PlayerView>,
        others: snapshot::SeenView<'s>,
        grid: &'s dyn seen_grid::SnapshotGrid,
        index: &'s crate::fast_hash::FxHashMap<u32, u32>,
    ) -> MobWorld<'s> {
        MobWorld {
            others,
            others_grid: Some(grid),
            others_index: Some(index),
            mod_effects: None,
            world: self.world,
            registry: self.registry,
            facts: self.facts,
            fluid_tables: self.fluid_tables,
            movement: self.movement,
            collision: self.collision,
            suffocation: self.suffocation,
            data: self.data,
            player,
            controlling_player_movement: self.controlling_player_movement,
            nearest_player: self.nearest_player,
            dragon: self.dragon,
            leash_boats: self.leash_boats,
            daylight: self.daylight,
            monsters_burn: self.monsters_burn,
            game_time: self.game_time,
            day_time: self.day_time,
            raining: self.raining,
            thundering: self.thundering,
            nether: self.nether,
            block_tags: self.block_tags,
            landing: self.landing,
            biomes: self.biomes,
            mob_griefing: self.mob_griefing,
            universal_anger: self.universal_anger,
            mob_drops: self.mob_drops,
            max_entity_cramming: self.max_entity_cramming,
            difficulty: self.difficulty,
            end: self.end,
            world_seed: self.world_seed,
            sea_level: self.sea_level,
            teams: self.teams.clone(),
            items: self.items,
            lid_openers: self.lid_openers,
            border: self.border,
        }
    }
}

// This compile-time contract rejects thread-affine caches added to immutable
// world inputs. Actual state mutation belongs to a region-owned transaction.
fn _shareable(resources: TickResources<'_>) {
    fn require<T: Send + Sync>(_: T) {}
    require(resources);
}

/// A region's immutable sources, lent until the worker barrier. Each worker
/// creates its exclusively owned mutable inputs; the coordinator retains the
/// originals for ordered replay. World/chunks are never cloned.
pub struct RegionInput<'a> {
    pub owner: usize,
    pub mobs: Vec<(usize, &'a Mob)>,
}

#[derive(Clone)]
pub struct MobContext {
    pub bound_player: Option<u32>,
    pub nearest_player: Option<Vec3>,
    pub difficulty: spawning::LocalDifficulty,
    pub section_order: u64,
}

#[derive(Clone)]
pub struct TickPrediction {
    pub slot: usize,
    pub owner: usize,
    pub mob: Mob,
    /// The guest this mob saw (`None`: the host or nobody), so a commit need
    /// not choose again.
    pub conn: Option<u32>,
    /// How the mob looks to the others after its tick, so a commit need not
    /// read it all again.
    pub seen: Seen,
    pub kept: bool,
    pub events: Vec<MobEvent>,
    pub reads: snapshot::ReadSet,
    pub random_before: LegacyRandom,
    pub random_after: LegacyRandom,
    /// The mob this ticked from, to check a commit replaces nothing newer.
    #[cfg(debug_assertions)]
    pub input: String,
}
impl TickPrediction {
    pub fn rng_valid(&self, current: &LegacyRandom) -> bool {
        !self.random_after.was_used() || current.same_stream(&self.random_before)
    }
}

/// What a commit tells the walk about the mob it swapped in.
pub struct Committed {
    pub kept: bool,
    /// The guest it saw.
    pub conn: Option<u32>,
    /// How it looks to the others now (`Seen::alive` already knows `kept`).
    pub seen: Seen,
}

pub struct RegionTick<'a> {
    pub resources: &'a TickResources<'a>,
    pub snapshot: &'a [Seen],
    pub grid: &'a seen_grid::SeenGrid,
    pub species: &'a snapshot::SpeciesIndex,
    pub index: &'a crate::fast_hash::FxHashMap<u32, u32>,
    pub contexts: &'a [MobContext],
    pub players: &'a players::PlayerDirectory<'a>,
    pub path_changes: &'a [[i32; 3]],
    pub random: &'a LegacyRandom,
    pub player_uuid: Option<[i32; 4]>,
    pub tnt_explodes: bool,
}
impl RegionTick<'_> {
    fn predict(&self, input: RegionInput<'_>) -> Vec<TickPrediction> {
        self.predict_inner::<false>(input).0
    }
    // The ordinary instantiation has no clock reads. Profile sums describe
    // worker work, not elapsed tick time, and never enter the span hierarchy.
    fn predict_inner<const PROFILE: bool>(
        &self,
        input: RegionInput<'_>,
    ) -> (Vec<TickPrediction>, [std::time::Duration; 3]) {
        use std::{
            cell::RefCell,
            time::{Duration, Instant},
        };
        let mut times = [Duration::ZERO; 3];
        // This rule was historically thread-local. Set it per level job, not
        // just when a persistent worker starts (different levels may differ).
        sulfur_cube::set_tnt_explodes(self.tnt_explodes);
        let mut overlay = snapshot::SnapshotOverlay::default();
        let mut grid = seen_grid::GridOverlay::new(self.grid);
        let mut pending = PendingPushes::default();
        let mut random = self.random.clone();
        let mut out = Vec::with_capacity(input.mobs.len());
        let owned_slots: crate::fast_hash::FxHashMap<_, _> = input
            .mobs
            .iter()
            .map(|(slot, mob)| (mob.id, *slot))
            .collect();
        let mut pending_brains: crate::fast_hash::FxHashMap<u32, Vec<MobEvent>> =
            Default::default();
        for (slot, original) in input.mobs {
            // Active raiders' hurt ordering still uses a global sequence. No
            // speculative execution may consume it, including rejected work.
            if raider::is_raider(original.species()) || original.extra.raider.is_some() {
                break;
            }
            let prepare = PROFILE.then(Instant::now);
            let mut mob = original.clone();
            if let Some(mut events) = pending_brains.remove(&mob.id) {
                apply_brain_events(std::slice::from_mut(&mut mob), &mut events, 0);
            }
            apply_mob_pushes(&mut mob, &mut pending);
            blocks_changed(&mut mob, self.path_changes);
            #[cfg(debug_assertions)]
            let mob_input = format!("{mob:?}");
            let reads = RefCell::new(snapshot::ReadSet::default());
            let mut events = Vec::new();
            let random_before = random.clone();
            random.clear_usage();
            if let Some(start) = prepare {
                times[0] += start.elapsed();
            }
            let simulate = PROFILE.then(Instant::now);
            let (kept, conn) = {
                let context = &self.contexts[slot];
                let conn = self
                    .players
                    .player_for(input.owner, &mob, context.bound_player);
                let player = conn
                    .and_then(|conn| self.players.guest(conn))
                    .or(self.resources.player.as_ref())
                    .cloned();
                let mut world = self.resources.view(
                    player,
                    snapshot::SeenView::observed(self.snapshot, &overlay, &reads)
                        .with_species(self.species),
                    &grid,
                    self.index,
                );
                world.controlling_player_movement = self.players.controlling_movement(&mob);
                world.nearest_player = context.nearest_player;
                world.difficulty = context.difficulty;
                let kept =
                    mob.tick_with_order(&world, &mut random, &mut events, context.section_order);
                (kept, conn)
            };
            if let Some(start) = simulate {
                times[1] += start.elapsed();
            }
            let output = PROFILE.then(Instant::now);
            // Preserve push order while keeping the original events intact for
            // the coordinator's authoritative global-order application.
            for event in &events {
                if let MobEvent::Push {
                    target: Who::Mob(id),
                    velocity,
                } = event
                {
                    pending.push(*id, Vec3::new(velocity[0], velocity[1], velocity[2]));
                }
            }
            let mut current = mob.seen(self.player_uuid);
            current.alive &= kept;
            grid.update(slot, current.bounds);
            let seen = current.clone();
            overlay.update(slot, current);
            // Replay known single-entity brain writes into the corresponding
            // future owned inputs, preserving their original event order.
            let mut writes = BrainWrites::of(&events);
            writes.targets.sort_unstable();
            writes.targets.dedup();
            for target in writes.targets {
                if owned_slots
                    .get(&target)
                    .is_some_and(|&target_slot| target_slot > slot)
                {
                    pending_brains
                        .entry(target)
                        .or_default()
                        .extend(events.iter().cloned());
                }
            }
            let global_conflict = writes.global;
            out.push(TickPrediction {
                slot,
                owner: input.owner,
                mob,
                conn,
                seen,
                kept,
                events,
                reads: reads.into_inner(),
                random_before,
                random_after: random.clone(),
                #[cfg(debug_assertions)]
                input: mob_input,
            });
            if let Some(start) = output {
                times[2] += start.elapsed();
            }
            if global_conflict {
                break;
            }
        }
        (out, times)
    }
}

/// One persistent bounded pool per server, shared by all of its levels. The
/// install call lends immutable tick data safely; it joins all region jobs
/// before returning predictions or propagating a worker panic.
pub struct RegionExecutor {
    pool: Option<std::sync::Arc<rayon::ThreadPool>>,
}
impl RegionExecutor {
    pub fn new(workers: usize) -> Result<Self, rayon::ThreadPoolBuildError> {
        if workers <= 1 {
            return Ok(Self { pool: None });
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers.clamp(1, 8))
            .thread_name(|index| format!("Horizon region {index}"))
            .build()?;
        Ok(Self {
            pool: Some(std::sync::Arc::new(pool)),
        })
    }
    /// The workers, for the server's other work that does not depend on order.
    pub fn pool(&self) -> Option<std::sync::Arc<rayon::ThreadPool>> {
        self.pool.clone()
    }
    /// Frees a finished round on the pool: its entries hold the mobs that were
    /// replaced, each with its own allocations, and nothing has to wait for
    /// them. One worker takes it all, so the coordinator's scans still find
    /// the others free.
    pub fn retire(&self, predictions: Vec<Vec<TickPrediction>>) {
        match &self.pool {
            Some(pool) => pool.spawn(move || drop(predictions)),
            None => drop(predictions),
        }
    }
    /// `f` of every item and its slot, in order, on the workers.
    pub fn map<T: Sync, R: Send>(
        &self,
        items: &[T],
        f: impl Fn(usize, &T) -> R + Send + Sync,
    ) -> Vec<R> {
        use rayon::prelude::*;
        let each = |(slot, item)| f(slot, item);
        match &self.pool {
            Some(pool) => pool.install(|| {
                items
                    .par_iter()
                    .enumerate()
                    .with_min_len(64)
                    .map(each)
                    .collect()
            }),
            None => items.iter().enumerate().map(each).collect(),
        }
    }
    /// `f` of every region's list on the workers, one task for each: unlike
    /// [`Self::map`], which keeps short runs together, a region is a lot of work.
    fn each_region<T: Sync, R: Send>(
        &self,
        lists: &[T],
        f: impl Fn(&T) -> R + Send + Sync,
    ) -> Vec<R> {
        use rayon::prelude::*;
        match &self.pool {
            Some(pool) => pool.install(|| lists.par_iter().map(&f).collect()),
            None => lists.iter().map(&f).collect(),
        }
    }
    /// Whether anything runs on other threads.
    pub fn parallel(&self) -> bool {
        self.pool.is_some()
    }
    /// The slots of the items `pick` chooses, in order, on the workers.
    pub fn slots<T: Sync>(
        &self,
        items: &[T],
        pick: impl Fn(&T) -> bool + Send + Sync,
    ) -> Vec<usize> {
        use rayon::prelude::*;
        let chosen = |(slot, item)| pick(item).then_some(slot);
        match &self.pool {
            Some(pool) => pool.install(|| {
                items
                    .par_iter()
                    .enumerate()
                    .with_min_len(64)
                    .filter_map(chosen)
                    .collect()
            }),
            None => items.iter().enumerate().filter_map(chosen).collect(),
        }
    }
    /// `each` for every pair of `targets` and `sources`, with the scratch each
    /// worker made for itself. The pairs are independent, so the order is not
    /// observable.
    pub fn zip_each<T: Send, U: Sync, S>(
        &self,
        targets: &mut [T],
        sources: &[U],
        scratch: impl Fn() -> S + Send + Sync,
        each: impl Fn(&mut S, &mut T, &U) + Send + Sync,
    ) {
        use rayon::prelude::*;
        match &self.pool {
            Some(pool) => pool.install(|| {
                targets
                    .par_iter_mut()
                    .zip(sources)
                    .with_min_len(64)
                    .for_each_init(scratch, |state, (target, source)| {
                        each(state, target, source)
                    });
            }),
            None => {
                let mut state = scratch();
                for (target, source) in targets.iter_mut().zip(sources) {
                    each(&mut state, target, source);
                }
            }
        }
    }
    pub fn predict(
        &self,
        inputs: Vec<RegionInput<'_>>,
        tick: &RegionTick<'_>,
    ) -> Vec<Vec<TickPrediction>> {
        use rayon::prelude::*;
        static PROFILE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *PROFILE.get_or_init(|| std::env::var("HORIZON_REGION_PROFILE").is_ok_and(|v| v == "1"))
        {
            let run = |input| tick.predict_inner::<true>(input);
            let jobs: Vec<_> = match &self.pool {
                Some(pool) => pool.install(|| inputs.into_par_iter().map(run).collect()),
                None => inputs.into_iter().map(run).collect(),
            };
            let mut sums = [0.0; 3];
            let mut predictions = Vec::with_capacity(jobs.len());
            for (predicted, times) in jobs {
                for (sum, time) in sums.iter_mut().zip(times) {
                    *sum += time.as_secs_f64() * 1000.0;
                }
                predictions.push(predicted);
            }
            eprintln!(
                "region worker sums (ms): prepare={:.3} simulate={:.3} observations={:.3}",
                sums[0], sums[1], sums[2]
            );
            return predictions;
        }
        match &self.pool {
            Some(pool) => pool.install(|| {
                inputs
                    .into_par_iter()
                    .map(|input| tick.predict(input))
                    .collect()
            }),
            None => inputs
                .into_iter()
                .map(|input| tick.predict(input))
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RegionStats {
    pub regions: usize,
    pub predicted: usize,
    pub committed: usize,
    pub replayed: usize,
    pub rng_conflicts: usize,
    pub read_conflicts: usize,
    pub read_full: usize,
    pub read_slot: usize,
    pub read_shape: usize,
    pub read_cell: usize,
    pub read_species: usize,
    /// 1 when this round's predictions could all be applied on the workers.
    pub clean_rounds: usize,
    /// 1 when they could not, and why: the round took the serial walk.
    pub fallback_rounds: usize,
    pub fallback: Option<NotClean>,
}
impl RegionStats {
    /// The numbers any path through a round has to agree on.
    pub fn outcome(self) -> Self {
        Self {
            clean_rounds: 0,
            fallback_rounds: 0,
            fallback: None,
            ..self
        }
    }
}

/// Why a round can't be applied on the workers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotClean {
    /// A mob has no prediction: unloaded or outside the entity-ticking area, a
    /// raider, a region sitting out, or a prediction cut short.
    Unpredicted,
    /// A prediction drew from the level's random, which is a shared checkpoint.
    SharedRandom,
    /// A prediction writes another mob's brain, or more mobs than it names.
    BrainWrite,
    /// A prediction pushes a mob another region owns.
    ForeignPush,
    /// A prediction read what another region's prediction writes.
    ForeignRead,
}

/// What a clean round's predictions need from the coordinator's tick.
pub struct CleanContext<'a> {
    /// Whether any guest is in play: only then do mobs keep a bound player.
    pub guests: bool,
    /// The guests' entity ids by connection.
    pub guest_ids: &'a [(u32, u32)],
    /// The events a guest's own turn handles (`lan_mobs.rs`).
    pub acts_on_player: fn(&MobEvent) -> bool,
    pub index: &'a crate::fast_hash::FxHashMap<u32, u32>,
}

/// What the walk leaves for the rest of the tick.
pub struct WalkOutput<'a> {
    pub keep: &'a mut Vec<bool>,
    pub events: &'a mut Vec<MobEvent>,
    pub guest_events: &'a mut Vec<(u32, u32, MobEvent)>,
    pub dead: &'a mut Vec<Mob>,
    pub corpses: &'a mut Vec<Mob>,
    pub gone: &'a mut Vec<(u32, Vec3, f64, f64)>,
    pub pushes: &'a mut PendingPushes,
    pub died: &'a mut crate::fast_hash::FxHashSet<u32>,
    pub bindings: &'a mut crate::fast_hash::FxHashMap<u32, u32>,
}

/// A classified round: each list's slots, read once so that nothing has to go
/// back to the predictions' memory to find them.
pub struct CleanPlan {
    slots: Vec<Vec<u32>>,
}

/// One mob of a clean round, while the workers apply it and until the merge.
struct CleanSlot<'a> {
    mob: &'a mut Mob,
    prediction: &'a mut TickPrediction,
    seen: &'a mut Seen,
    /// The pushes the walk would still hold after this mob: at itself or a mob
    /// it has passed. One at a later mob reached it in its own prediction.
    pushes: Vec<(u32, Vec3)>,
    guest_events: Vec<(u32, u32, MobEvent)>,
    corpse: bool,
    gone: bool,
    dead: bool,
    grid_changes: bool,
    binding_changes: bool,
}
impl CleanSlot<'_> {
    /// Whether the merge has nothing to do here but note `kept`.
    fn quiet(&self) -> bool {
        self.prediction.kept
            && self.prediction.events.is_empty()
            && self.pushes.is_empty()
            && self.guest_events.is_empty()
            && !(self.corpse || self.gone || self.dead || self.grid_changes || self.binding_changes)
    }
}

/// Runs in `HORIZON_SIMULATION_MODE=verify`: what the clean path made of a copy
/// of a round the classifier called clean, for the serial walk to answer.
pub struct CleanRehearsal {
    mobs: Vec<Mob>,
    snapshot: Vec<Seen>,
    grid: seen_grid::SeenGrid,
    random: LegacyRandom,
    stats: RegionStats,
    keep: Vec<bool>,
    events: Vec<MobEvent>,
    guest_events: Vec<(u32, u32, MobEvent)>,
    dead: Vec<Mob>,
    corpses: Vec<Mob>,
    gone: Vec<(u32, Vec3, f64, f64)>,
    pushes: PendingPushes,
    died: crate::fast_hash::FxHashSet<u32>,
    bindings: crate::fast_hash::FxHashMap<u32, u32>,
}
impl CleanRehearsal {
    /// Panics at the first thing the clean path got differently from the walk,
    /// given what the walk finished with.
    pub fn verify(
        &self,
        mobs: &[Mob],
        snapshot: &[Seen],
        grid: &seen_grid::SeenGrid,
        random: &LegacyRandom,
        stats: RegionStats,
        walk: &WalkOutput<'_>,
    ) {
        let (clean, serial) = (&self.mobs[..], mobs);
        let same = |what: &str, clean: String, serial: String| {
            assert!(
                clean == serial,
                "a clean round and the serial walk differ in the {what}\nclean:  {clean}\nserial: {serial}"
            );
        };
        assert_eq!(clean.len(), serial.len(), "a clean round's mob count");
        for (clean, serial) in clean.iter().zip(serial) {
            same("mobs", format!("{clean:?}"), format!("{serial:?}"));
        }
        same(
            "snapshot",
            format!("{:?}", self.snapshot),
            format!("{snapshot:?}"),
        );
        assert!(
            self.grid == *grid,
            "a clean round's grid differs from the walk's"
        );
        assert!(
            self.random.same_stream(random),
            "a clean round left the level random differently"
        );
        same(
            "stats",
            format!("{:?}", self.stats.outcome()),
            format!("{:?}", stats.outcome()),
        );
        same(
            "keep",
            format!("{:?}", self.keep),
            format!("{:?}", walk.keep),
        );
        same(
            "events",
            format!("{:?}", self.events),
            format!("{:?}", walk.events),
        );
        same(
            "guest events",
            format!("{:?}", self.guest_events),
            format!("{:?}", walk.guest_events),
        );
        same(
            "dead",
            format!("{:?}", self.dead),
            format!("{:?}", walk.dead),
        );
        same(
            "corpses",
            format!("{:?}", self.corpses),
            format!("{:?}", walk.corpses),
        );
        same(
            "gone",
            format!("{:?}", self.gone),
            format!("{:?}", walk.gone),
        );
        same(
            "pushes",
            format!("{:?}", self.pushes.waiting()),
            format!("{:?}", walk.pushes.waiting()),
        );
        assert!(
            self.died == *walk.died,
            "a clean round's death records differ"
        );
        assert!(
            self.bindings == *walk.bindings,
            "a clean round's bindings differ"
        );
    }
}

/// The authoritative ordered commit. A failed region prefix can never provide
/// another prediction this tick. Conflicts replay against the live snapshot
/// and RNG, before any dependent entity or event is published.
#[derive(Clone)]
pub struct TickRound {
    /// Each job's results, ascending by slot. They stay where the workers put
    /// them: a mob is thousands of bytes, and only a commit needs to touch it.
    predictions: Vec<Vec<TickPrediction>>,
    /// The list in `predictions` of each owner, and how far the walk has read it.
    queues: Vec<Option<usize>>,
    cursors: Vec<usize>,
    owners: Vec<usize>,
    failed: Vec<bool>,
    /// What each owner predicted and how much of it committed this round.
    predicted: Vec<usize>,
    committed: Vec<usize>,
    changes: snapshot::SnapshotChanges,
    pub stats: RegionStats,
    section_base: u64,
    /// Each mob's context, which a clean round reads its old binding from.
    contexts: Vec<MobContext>,
}
impl TickRound {
    pub fn new(
        owners: Vec<usize>,
        predictions: Vec<Vec<TickPrediction>>,
        section_base: u64,
    ) -> Self {
        let active_regions = predictions
            .iter()
            .filter(|predictions| !predictions.is_empty())
            .count();
        let regions = owners.iter().copied().max().map_or(0, |owner| owner + 1);
        let mut queues = vec![None; regions];
        let mut predicted = vec![0; regions];
        for (queue, list) in predictions.iter().enumerate() {
            if let Some(first) = list.first() {
                predicted[first.owner] = list.len();
                debug_assert!(list.iter().all(|p| owners[p.slot] == first.owner));
                debug_assert!(list.windows(2).all(|p| p[0].slot < p[1].slot));
                assert!(queues[first.owner].is_none(), "one list per owner");
                queues[first.owner] = Some(queue);
            }
        }
        Self {
            queues,
            cursors: vec![0; regions],
            changes: snapshot::SnapshotChanges::new(owners.clone()),
            owners,
            failed: vec![false; regions],
            committed: vec![0; regions],
            stats: RegionStats {
                regions: active_regions,
                predicted: predicted.iter().sum(),
                ..Default::default()
            },
            predicted,
            predictions,
            section_base,
            contexts: Vec::new(),
        }
    }
    pub fn with_contexts(mut self, contexts: Vec<MobContext>) -> Self {
        self.contexts = contexts;
        self
    }
    /// Whether every prediction can be applied at once, on the workers, with
    /// the result of the serial walk. Reads tick-start state and predictions
    /// only: nothing changes here, so a round that isn't clean simply walks.
    /// The round's stats say which it was.
    pub fn classify(
        &mut self,
        executor: &RegionExecutor,
        snapshot: &[Seen],
        grid: &seen_grid::SeenGrid,
        index: &crate::fast_hash::FxHashMap<u32, u32>,
    ) -> Option<CleanPlan> {
        let plan = self.clean_plan(executor, snapshot, grid, index);
        match &plan {
            Ok(_) => self.stats.clean_rounds = 1,
            Err(reason) => {
                self.stats.fallback_rounds = 1;
                self.stats.fallback = Some(*reason);
            }
        }
        plan.ok()
    }
    fn clean_plan(
        &self,
        executor: &RegionExecutor,
        snapshot: &[Seen],
        grid: &seen_grid::SeenGrid,
        index: &crate::fast_hash::FxHashMap<u32, u32>,
    ) -> Result<CleanPlan, NotClean> {
        let owners = &self.owners[..];
        if self.stats.predicted != snapshot.len() || self.contexts.len() != snapshot.len() {
            return Err(NotClean::Unpredicted);
        }
        let summaries = executor.each_region(&self.predictions, |list| {
            let mut writes = snapshot::OwnerWrites::default();
            let mut slots = Vec::with_capacity(list.len());
            for prediction in list {
                prediction_is_self_contained(prediction, owners, index)?;
                slots.push(prediction.slot as u32);
                let before = &snapshot[prediction.slot];
                let after = &prediction.seen;
                writes.cells.push(seen_grid::corner(before.bounds));
                writes.cells.push(seen_grid::corner(after.bounds));
                writes.reach = writes.reach.max(seen_grid::width(after.bounds));
                for species in [before.species, after.species] {
                    if !writes.species.contains(&species) {
                        writes.species.push(species);
                    }
                }
            }
            writes.cells.sort_unstable();
            writes.cells.dedup();
            writes.new_cells = writes.cells.iter().filter(|&&c| !grid.has_cell(c)).count();
            Ok((writes, slots))
        });
        let mut round = Vec::with_capacity(summaries.len());
        for summary in summaries {
            round.push(summary?);
        }
        let cells = round.iter().map(|(writes, _)| writes.cells.len()).sum();
        let mut writes = snapshot::RoundWrites::new(grid, cells);
        for (list, (owner_writes, _)) in self.predictions.iter().zip(&round) {
            if let Some(first) = list.first() {
                writes.add(first.owner, owner_writes);
            }
        }
        let untouched = executor.each_region(&self.predictions, |list| {
            list.iter()
                .all(|p| p.reads.untouched_by_others(p.owner, owners, &writes))
        });
        if !untouched.into_iter().all(|untouched| untouched) {
            return Err(NotClean::ForeignRead);
        }
        Ok(CleanPlan {
            slots: round.into_iter().map(|(_, slots)| slots).collect(),
        })
    }
    /// Applies a classified round. The mobs and their snapshot entries go on
    /// the workers, each from its own prediction; what needs an order is then
    /// merged here, in slot order, as the walk would have left it.
    pub fn commit_clean(
        &mut self,
        executor: &RegionExecutor,
        plan: CleanPlan,
        mobs: &mut [Mob],
        snapshot: &mut [Seen],
        grid: &mut seen_grid::SeenGrid,
        context: &CleanContext<'_>,
        walk: &mut WalkOutput<'_>,
    ) {
        let reach = grid.reach();
        let died = &*walk.died;
        let mut by_slot: Vec<Option<&mut TickPrediction>> =
            std::iter::repeat_with(|| None).take(mobs.len()).collect();
        for (list, slots) in self.predictions.iter_mut().zip(plan.slots) {
            for (prediction, slot) in list.iter_mut().zip(slots) {
                by_slot[slot as usize] = Some(prediction);
            }
        }
        let mut slots: Vec<CleanSlot<'_>> = mobs
            .iter_mut()
            .zip(snapshot.iter_mut())
            .zip(by_slot)
            .map(|((mob, seen), prediction)| CleanSlot {
                mob,
                seen,
                prediction: prediction.expect("a clean round predicted every slot"),
                pushes: Vec::new(),
                guest_events: Vec::new(),
                corpse: false,
                gone: false,
                dead: false,
                grid_changes: false,
                binding_changes: false,
            })
            .collect();
        executor.zip_each(
            &mut slots,
            &self.contexts,
            || (),
            |_, slot, mob_context| {
                let p = &mut *slot.prediction;
                // The replaced mob stays in the list for [`RegionExecutor::retire`].
                std::mem::swap(slot.mob, &mut p.mob);
                let m = &*slot.mob;
                slot.grid_changes = seen_grid::corner(slot.seen.bounds)
                    != seen_grid::corner(p.seen.bounds)
                    || seen_grid::width(p.seen.bounds) > reach;
                std::mem::swap(slot.seen, &mut p.seen);
                if !p.events.is_empty() {
                    split_events(
                        &mut p.events,
                        p.slot,
                        m.id,
                        p.conn,
                        context,
                        &mut slot.pushes,
                        &mut slot.guest_events,
                    );
                }
                let kept = p.kept;
                let lying = !m.alive() && !m.removed;
                slot.corpse = !kept && lying && !m.extra.effects.is_empty();
                slot.gone = !kept && lying && m.death_time >= 20;
                slot.dead = lying && !died.contains(&m.id);
                slot.binding_changes = context.guests && p.conn != mob_context.bound_player;
            },
        );
        walk.keep.reserve(slots.len());
        for (i, slot) in slots.iter_mut().enumerate() {
            walk.keep.push(slot.prediction.kept);
            if slot.quiet() {
                continue;
            }
            let (m, kept) = (&*slot.mob, slot.prediction.kept);
            walk.events.append(&mut slot.prediction.events);
            if slot.corpse {
                walk.corpses.push(m.clone());
            }
            for &(id, velocity) in &slot.pushes {
                walk.pushes.push(id, velocity);
            }
            if slot.gone {
                let (width, height) = (f64::from(m.width()), f64::from(m.height()));
                walk.gone.push((m.id, m.position, width, height));
            }
            walk.guest_events.append(&mut slot.guest_events);
            if slot.dead && walk.died.insert(m.id) {
                walk.dead.push(m.clone());
            }
            if !kept {
                walk.died.remove(&m.id);
            }
            if slot.grid_changes {
                grid.update(i, slot.seen.bounds);
            }
            if slot.binding_changes {
                match slot.prediction.conn {
                    Some(conn) => walk.bindings.insert(m.id, conn),
                    None => walk.bindings.remove(&m.id),
                };
            }
        }
        self.stats.committed = self.stats.predicted;
        self.committed.clone_from(&self.predicted);
    }
    /// Runs the clean path over copies, for the serial walk to answer
    /// (`HORIZON_SIMULATION_MODE=verify`).
    #[allow(clippy::too_many_arguments)]
    pub fn rehearse_clean(
        &self,
        executor: &RegionExecutor,
        plan: CleanPlan,
        mobs: &[Mob],
        snapshot: &[Seen],
        grid: &seen_grid::SeenGrid,
        random: &LegacyRandom,
        context: &CleanContext<'_>,
        died: &crate::fast_hash::FxHashSet<u32>,
        bindings: &crate::fast_hash::FxHashMap<u32, u32>,
    ) -> CleanRehearsal {
        let mut round = self.clone();
        let mut rehearsal = CleanRehearsal {
            mobs: mobs.to_vec(),
            snapshot: snapshot.to_vec(),
            grid: grid.clone(),
            random: random.clone(),
            stats: RegionStats::default(),
            keep: Vec::new(),
            events: Vec::new(),
            guest_events: Vec::new(),
            dead: Vec::new(),
            corpses: Vec::new(),
            gone: Vec::new(),
            pushes: PendingPushes::default(),
            died: died.clone(),
            bindings: bindings.clone(),
        };
        round.commit_clean(
            executor,
            plan,
            &mut rehearsal.mobs,
            &mut rehearsal.snapshot,
            &mut rehearsal.grid,
            context,
            &mut WalkOutput {
                keep: &mut rehearsal.keep,
                events: &mut rehearsal.events,
                guest_events: &mut rehearsal.guest_events,
                dead: &mut rehearsal.dead,
                corpses: &mut rehearsal.corpses,
                gone: &mut rehearsal.gone,
                pushes: &mut rehearsal.pushes,
                died: &mut rehearsal.died,
                bindings: &mut rehearsal.bindings,
            },
        );
        rehearsal.stats = round.stats;
        rehearsal
    }
    /// Publishes this slot's prediction if it is still valid, returning what
    /// the tick returned and the guest the mob saw. `None`: nothing changed,
    /// and the caller builds the mob's view and [`Self::replay`]s it.
    pub fn commit(
        &mut self,
        slot: usize,
        mob: &mut Mob,
        grid: &seen_grid::SeenGrid,
        random: &mut LegacyRandom,
        events: &mut Vec<MobEvent>,
    ) -> Option<Committed> {
        let owner = self.owners[slot];
        if !self.failed[owner]
            && let Some(queue) = self.queues[owner]
        {
            let list = &mut self.predictions[queue];
            let mut cursor = self.cursors[owner];
            while list.get(cursor).is_some_and(|p| p.slot < slot) {
                cursor += 1;
            }
            self.cursors[owner] = cursor;
            let prediction = list.get_mut(cursor).filter(|p| p.slot == slot)?;
            let rng_valid = prediction.rng_valid(random);
            let read_conflict = self.changes.conflict(&prediction.reads, owner, grid);
            if rng_valid && read_conflict.is_none() {
                // The swap drops whatever the walk wrote to this mob since the
                // prediction cloned it. Every such write has to reach the
                // prediction or void it (`immediate_write`, pushes).
                #[cfg(debug_assertions)]
                debug_assert!(
                    prediction.input == format!("{mob:?}"),
                    "the walk wrote to mob {} after its prediction was made",
                    mob.id
                );
                // The replaced mob stays in the list for [`RegionExecutor::retire`].
                std::mem::swap(mob, &mut prediction.mob);
                if prediction.random_after.was_used() {
                    *random = prediction.random_after.clone();
                }
                events.append(&mut prediction.events);
                self.stats.committed += 1;
                self.committed[owner] += 1;
                return Some(Committed {
                    kept: prediction.kept,
                    conn: prediction.conn,
                    seen: prediction.seen.clone(),
                });
            }
            self.stats.rng_conflicts += usize::from(!rng_valid);
            if let Some(conflict) = read_conflict {
                self.stats.read_conflicts += 1;
                match conflict {
                    snapshot::ReadConflict::FullScan => self.stats.read_full += 1,
                    snapshot::ReadConflict::EntitySlot => self.stats.read_slot += 1,
                    snapshot::ReadConflict::GridShape => self.stats.read_shape += 1,
                    snapshot::ReadConflict::SpatialCell => self.stats.read_cell += 1,
                    snapshot::ReadConflict::Species => self.stats.read_species += 1,
                }
            }
        }
        None
    }
    /// Runs a mob whose prediction did not commit against the live level. The
    /// rest of its owner's prefix observed the prediction, so it is void too.
    pub fn replay(
        &mut self,
        slot: usize,
        mob: &mut Mob,
        world: &MobWorld<'_>,
        random: &mut LegacyRandom,
        events: &mut Vec<MobEvent>,
    ) -> bool {
        self.failed[self.owners[slot]] = true;
        self.stats.replayed += 1;
        mob.tick_with_order(
            world,
            random,
            events,
            self.section_base.wrapping_add(slot as u64),
        )
    }
    /// Each owner that predicted anything: its slot, what it predicted, and how
    /// much of that committed.
    pub fn outcomes(&self) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
        self.predicted
            .iter()
            .zip(&self.committed)
            .enumerate()
            .filter(|(_, (predicted, _))| **predicted > 0)
            .map(|(owner, (&predicted, &committed))| (owner, predicted, committed))
    }
    /// The round's predictions, for [`RegionExecutor::retire`] once the walk ends.
    pub fn take_predictions(&mut self) -> Vec<Vec<TickPrediction>> {
        std::mem::take(&mut self.predictions)
    }
    pub fn snapshot_changed(&mut self, slot: usize, before: &Seen, after: &Seen) {
        self.changes.touch(slot, before.bounds, after.bounds);
        self.changes
            .species_changed(slot, before.species, after.species);
    }
    pub fn despawn_removed(&mut self, slot: usize, bounds: Aabb, species: Species) {
        // Non-entity-ticking chunks run their cheap despawn check serially;
        // workers therefore cannot rely on that owner's predicted prefix.
        self.failed[self.owners[slot]] = true;
        self.changes.touch(slot, bounds, bounds);
        self.changes.species_changed(slot, species, species);
    }
    pub fn events(
        &mut self,
        slot: usize,
        events: &[MobEvent],
        index: &crate::fast_hash::FxHashMap<u32, u32>,
    ) {
        let owner = self.owners[slot];
        for event in events {
            if let MobEvent::Push {
                target: Who::Mob(id),
                ..
            } = event
                && let Some(&target) = index.get(id)
            {
                let target_owner = self.owners[target as usize];
                if target_owner != owner {
                    self.failed[target_owner] = true;
                }
            }
        }
    }
    pub fn immediate_brain_write(
        &mut self,
        source: usize,
        writes: &BrainWrites,
        index: &crate::fast_hash::FxHashMap<u32, u32>,
    ) {
        if writes.global {
            self.failed.fill(true);
            return;
        }
        let owner = self.owners[source];
        for target in &writes.targets {
            if let Some(&slot) = index.get(target) {
                let target_owner = self.owners[slot as usize];
                if target_owner != owner {
                    self.failed[target_owner] = true;
                }
            }
        }
    }
}

/// Whether a prediction leaves everything but its own mob alone: no shared
/// random, no write to another mob, and pushes only at mobs of its own owner,
/// which saw them when their own predictions ran.
fn prediction_is_self_contained(
    prediction: &TickPrediction,
    owners: &[usize],
    index: &crate::fast_hash::FxHashMap<u32, u32>,
) -> Result<(), NotClean> {
    if prediction.random_after.was_used() {
        return Err(NotClean::SharedRandom);
    }
    for event in &prediction.events {
        if !matches!(event.immediate_write(), ImmediateWrite::Nothing) {
            return Err(NotClean::BrainWrite);
        }
        if let MobEvent::Push {
            target: Who::Mob(id),
            ..
        } = event
            && index
                .get(id)
                .is_some_and(|&target| owners[target as usize] != prediction.owner)
        {
            return Err(NotClean::ForeignPush);
        }
    }
    Ok(())
}

/// A clean prediction's events, as the walk would leave them: the pushes it
/// would still hold and the guest's events go out, and the rest keep their order.
fn split_events(
    events: &mut Vec<MobEvent>,
    slot: usize,
    id: u32,
    conn: Option<u32>,
    context: &CleanContext<'_>,
    pushes: &mut Vec<(u32, Vec3)>,
    guest_events: &mut Vec<(u32, u32, MobEvent)>,
) {
    let guest_entity = conn.and_then(|conn| {
        context
            .guest_ids
            .iter()
            .find(|guest| guest.0 == conn)
            .map(|guest| guest.1)
    });
    events.retain_mut(|event| {
        if let MobEvent::Push {
            target: Who::Mob(target),
            velocity,
        } = event
        {
            // A later mob's own prediction had this one.
            let passed = context
                .index
                .get(target)
                .is_none_or(|&target_slot| target_slot as usize <= slot);
            if passed {
                pushes.push((*target, Vec3::new(velocity[0], velocity[1], velocity[2])));
            }
            return false;
        }
        let Some(conn) = conn else {
            return true;
        };
        // ShulkerBullet.finalTarget is the guest's entity.
        if let (MobEvent::Projectile(p), Some(entity)) = (&mut *event, guest_entity)
            && let crate::projectiles::ProjectileKind::ShulkerBullet { target, .. } = &mut p.kind
            && *target == Some(Who::Player)
        {
            *target = Some(Who::Mob(entity));
        }
        if (context.acts_on_player)(event) {
            guest_events.push((conn, id, event.clone()));
            return false;
        }
        true
    });
}

/// Rounds in a row a region must waste before it sits out.
const WASTED_ROUNDS: u32 = 4;
/// Ticks of the first time it sits out; each wasted try after doubles them.
const FIRST_SKIP: u32 = 20;
const MAX_SKIP: u32 = 640;

#[derive(Default)]
struct RegionBackoff {
    wasted_rounds: u32,
    skip: u32,
    penalty: u32,
}

/// Which regions keep paying for predictions that don't commit. Level RNG draws
/// and shared villager reads can void most of a region's work every tick, and
/// then it costs the prediction and the replay. A region that keeps doing so
/// runs on the coordinator for a while, as one with no prediction always can.
/// This changes cost only: a commit and a replay give the same result. Regions
/// that do commit keep their workers.
#[derive(Default)]
pub struct PredictionBackoff {
    regions: crate::fast_hash::FxHashMap<crate::server_regions::RegionId, RegionBackoff>,
}
impl PredictionBackoff {
    /// Whether each of this tick's regions predicts. Counts down the waits, and
    /// forgets regions that are gone (an id changes with the region's chunks,
    /// which at worst costs a few wasted rounds to learn again).
    pub fn tick(&mut self, regions: &[crate::server_regions::Region]) -> Vec<bool> {
        if self.regions.is_empty() {
            return vec![true; regions.len()];
        }
        let mut live = crate::fast_hash::FxHashMap::default();
        let predicts = regions
            .iter()
            .map(|region| {
                let Some(mut state) = self.regions.remove(&region.id) else {
                    return true;
                };
                let predicts = state.skip == 0;
                state.skip = state.skip.saturating_sub(1);
                live.insert(region.id, state);
                predicts
            })
            .collect();
        self.regions = live;
        predicts
    }
    /// A round's result for one region: half or more thrown away is a waste.
    pub fn observe(
        &mut self,
        region: crate::server_regions::RegionId,
        predicted: usize,
        committed: usize,
    ) {
        if committed * 2 > predicted {
            self.regions.remove(&region);
            return;
        }
        let state = self.regions.entry(region).or_default();
        state.wasted_rounds += 1;
        if state.wasted_rounds >= WASTED_ROUNDS {
            state.penalty = (state.penalty * 2).clamp(FIRST_SKIP, MAX_SKIP);
            state.skip = state.penalty;
            // The next round after the wait is the test: one more waste waits longer.
            state.wasted_rounds = WASTED_ROUNDS - 1;
        }
    }
}

/// A level's mob list and death records while a tick holds them out of the
/// level. A panic during the tick leaves them here instead of dropping them, so
/// the crash save doesn't write the level's entities as if it had no mobs.
pub struct TakenMobs {
    pub mobs: crate::join_seq::JoinList<Mob>,
    pub died: crate::fast_hash::FxHashSet<u32>,
    rescue: MobRescue,
}
impl Drop for TakenMobs {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let mut rescued = self
                .rescue
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *rescued = Some((
                std::mem::take(&mut self.mobs),
                std::mem::take(&mut self.died),
            ));
        }
    }
}
type RescuedMobs = (
    crate::join_seq::JoinList<Mob>,
    crate::fast_hash::FxHashSet<u32>,
);
/// Where [`TakenMobs`] leaves them.
#[derive(Clone, Default)]
pub struct MobRescue(std::sync::Arc<std::sync::Mutex<Option<RescuedMobs>>>);
impl MobRescue {
    pub fn hold(
        &self,
        mobs: crate::join_seq::JoinList<Mob>,
        died: crate::fast_hash::FxHashSet<u32>,
    ) -> TakenMobs {
        TakenMobs {
            mobs,
            died,
            rescue: self.clone(),
        }
    }
    /// What a tick left here when it panicked.
    pub fn recover(&self) -> Option<RescuedMobs> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

/// Runs in `HORIZON_SIMULATION_MODE=verify`, with the mob as it was before its
/// commit replayed on the coordinator: a commit must be what a serial tick gives,
/// or a read or write got past the tracking. A panic names the first difference.
pub fn verify_commit(
    slot: usize,
    committed: (&Mob, bool, &[MobEvent], &LegacyRandom),
    replayed: (&Mob, bool, &[MobEvent], &LegacyRandom),
) {
    let id = committed.0.id;
    assert_eq!(
        committed.1, replayed.1,
        "slot {slot}, mob {id}: a commit and its replay disagree on keeping the mob"
    );
    assert!(
        committed.3.same_stream(replayed.3),
        "slot {slot}, mob {id}: a commit and its replay left the level random differently"
    );
    for (what, committed, replayed) in [
        (
            "mob",
            format!("{:?}", committed.0),
            format!("{:?}", replayed.0),
        ),
        (
            "events",
            format!("{:?}", committed.2),
            format!("{:?}", replayed.2),
        ),
    ] {
        if committed != replayed {
            let at = committed
                .bytes()
                .zip(replayed.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(committed.len().min(replayed.len()));
            let part = |text: &str| {
                let bytes = text.as_bytes();
                let (from, to) = (at.saturating_sub(100), (at + 180).min(bytes.len()));
                String::from_utf8_lossy(&bytes[from..to]).into_owned()
            };
            panic!(
                "slot {slot}, mob {id}: the commit and its replay differ in the {what} at {at}\ncommit: {}\nreplay: {}",
                part(&committed),
                part(&replayed)
            );
        }
    }
}

/// Per-level EntitySection insertion order. Reserving a slot for every entity
/// keeps relative order correct even if an earlier immediate interaction makes
/// a later mob change sections. Unused reservations are harmless gaps.
pub struct SectionOrder {
    next: u64,
}
impl Default for SectionOrder {
    fn default() -> Self {
        Self { next: 1 }
    }
}
impl SectionOrder {
    pub fn reserve(&mut self, mobs: &[Mob]) -> u64 {
        let after_existing = mobs
            .iter()
            .map(|mob| mob.extra.section_seq)
            .max()
            .unwrap_or(0)
            .wrapping_add(1);
        let start = self.next.max(after_existing);
        self.next = start.wrapping_add(mobs.len() as u64);
        start
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_pool_keeps_slots_and_results_in_list_order() {
        let executor = RegionExecutor::new(4).unwrap();
        let items: Vec<u32> = (0..1000).collect();
        let sevens: Vec<usize> = (0..1000).step_by(7).collect();
        assert_eq!(executor.slots(&items, |n| n % 7 == 0), sevens);
        let doubled: Vec<u32> = (0..1000).map(|n| n * 2).collect();
        assert_eq!(executor.map(&items, |slot, n| slot as u32 + n), doubled);
    }

    use super::*;
    use crate::fast_hash::FxHashMap;
    use std::sync::Arc;

    fn advance(
        mobs: &mut Vec<Mob>,
        base: &MobWorld<'_>,
        random: &mut LegacyRandom,
        executor: Option<&RegionExecutor>,
    ) -> (Vec<MobEvent>, RegionStats) {
        let section_base = mobs
            .iter()
            .map(|mob| mob.extra.section_seq)
            .max()
            .unwrap_or(0)
            .wrapping_add(1);
        let mut snapshot: Vec<_> = mobs.iter().map(|mob| mob.seen(None)).collect();
        let mut grid = seen_grid::SeenGrid::new(snapshot.iter().map(|seen| seen.bounds));
        let species_index = snapshot::SpeciesIndex::new(&snapshot);
        let index: FxHashMap<_, _> = mobs
            .iter()
            .enumerate()
            .map(|(slot, mob)| (mob.id, slot as u32))
            .collect();
        let contexts: Vec<_> = mobs
            .iter()
            .enumerate()
            .map(|(slot, _)| MobContext {
                bound_player: None,
                nearest_player: base.nearest_player,
                difficulty: base.difficulty,
                section_order: section_base.wrapping_add(slot as u64),
            })
            .collect();
        let player_directory = players::PlayerDirectory::new(base.player.as_ref(), &[]);
        let mut round = executor.map(|executor| {
            let owners: Vec<_> = mobs
                .iter()
                .map(|mob| {
                    if mob.id <= 4 {
                        (mob.id as usize - 1) % 2
                    } else {
                        2
                    }
                })
                .collect();
            let inputs = (0..3)
                .map(|owner| RegionInput {
                    owner,
                    mobs: mobs
                        .iter()
                        .enumerate()
                        .filter(|&(slot, _)| owners[slot] == owner)
                        .collect(),
                })
                .collect();
            let resources = TickResources::new(base);
            let tick = RegionTick {
                resources: &resources,
                snapshot: &snapshot,
                grid: &grid,
                species: &species_index,
                index: &index,
                contexts: &contexts,
                players: &player_directory,
                path_changes: &[],
                random,
                player_uuid: None,
                tnt_explodes: true,
            };
            TickRound::new(owners, executor.predict(inputs, &tick), section_base)
        });
        let mut events = Vec::new();
        let mut pushes = PendingPushes::default();
        let mut keep = Vec::new();
        for slot in 0..mobs.len() {
            let world = MobWorld {
                others: snapshot::SeenView::from(&snapshot).with_species(&species_index),
                others_grid: executor.map(|_| &grid as &dyn seen_grid::SnapshotGrid),
                others_index: Some(&index),
                ..base.clone()
            };
            let mob = &mut mobs[slot];
            apply_mob_pushes(mob, &mut pushes);
            let from = events.len();
            let kept = match round.as_mut() {
                Some(round) => match round.commit(slot, mob, &grid, random, &mut events) {
                    Some(committed) => committed.kept,
                    None => round.replay(slot, mob, &world, random, &mut events),
                },
                None => mob.tick_with_order(
                    &world,
                    random,
                    &mut events,
                    section_base.wrapping_add(slot as u64),
                ),
            };
            if let Some(round) = round.as_mut() {
                round.events(slot, &events[from..], &index);
            }
            take_mob_pushes(&mut events, from, &mut pushes);
            let mut after = mob.seen(None);
            after.alive &= kept;
            let before = std::mem::replace(&mut snapshot[slot], after);
            grid.update(slot, snapshot[slot].bounds);
            if let Some(round) = round.as_mut() {
                round.snapshot_changed(slot, &before, &snapshot[slot]);
            }
            let changed = apply_brain_events(mobs, &mut events, from);
            if let Some(round) = round.as_mut() {
                round.immediate_brain_write(slot, &changed, &index);
            }
            keep.push(kept);
        }
        for mob in mobs.iter_mut() {
            apply_mob_pushes(mob, &mut pushes);
        }
        let mut keep = keep.into_iter();
        mobs.retain(|_| keep.next().unwrap());
        if let (Some(round), Some(executor)) = (round.as_mut(), executor) {
            executor.retire(round.take_predictions());
        }
        (
            events,
            round.map_or_else(Default::default, |round| round.stats),
        )
    }

    fn assert_same_mobs(
        actual: &[Mob],
        expected: &[Mob],
        species: &str,
        near: bool,
        tick: usize,
        mode: &str,
    ) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            let a = format!("{actual:?}");
            let b = format!("{expected:?}");
            if a != b {
                let offset = a
                    .bytes()
                    .zip(b.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(a.len().min(b.len()));
                let lo = offset.saturating_sub(100);
                let hi_a = (offset + 180).min(a.len());
                let hi_b = (offset + 180).min(b.len());
                panic!(
                    "{mode} {species}, near={near}, tick={tick}, mob={}: first difference at {offset}\nactual: {}\nserial: {}",
                    actual.id,
                    &a[lo..hi_a],
                    &b[lo..hi_b]
                );
            }
        }
    }

    #[test]
    fn full_mob_transactions_match_serial_for_independence_and_conflicts() {
        // Existing resource-backed mob tests use this helper and skip when the
        // owner's prepared vanilla data is unavailable. No substitute oracle
        // or weakened expected values are used when resources are present.
        let ran = super::super::world::with_test_world(&[], |world, types| {
            let serial_regions = RegionExecutor::new(1).unwrap();
            let parallel = RegionExecutor::new(3).unwrap();
            for (species, near, breeding) in [
                ("cow", false, false),
                ("cow", true, false),
                ("cow", false, true),
                ("cow", true, true),
                ("zombie", false, false),
                ("zombie", true, false),
                ("villager", false, false),
                ("bee", false, false),
            ] {
                let kind = Arc::new(types.types[&format!("minecraft:{species}")].clone());
                let dummy = super::super::sensor_scan_tests::kind();
                let mut initial = Vec::new();
                for id in 1..=132 {
                    let mut mob = Mob::new(
                        id,
                        if id <= 4 { kind.clone() } else { dummy.clone() },
                        Vec3::ZERO,
                        0.0,
                    );
                    mob.persistent = true;
                    mob.no_gravity = true;
                    mob.position.y = 80.0;
                    if id <= 4 {
                        let owner = (id - 1) % 2;
                        mob.position.x = if near {
                            f64::from(id) * 0.1
                        } else {
                            f64::from(owner) * 1024.0 + f64::from(id / 2) * 0.1
                        };
                    } else {
                        mob.position.x = 4096.0 + f64::from(id) * 8.0;
                        mob.no_ai = true;
                    }
                    mob.previous = mob.position;
                    if id <= 4 && breeding {
                        mob.extra.in_love = 600;
                    }
                    if id <= 4 && species == "bee" {
                        bee::set_has_nectar(&mut mob, true);
                    }
                    initial.push(mob);
                }
                let mut expected = initial.clone();
                let mut serial = initial.clone();
                let mut concurrent = initial;
                let mut expected_rng = LegacyRandom::new(1234);
                let mut serial_rng = expected_rng.clone();
                let mut concurrent_rng = expected_rng.clone();
                let mut accepted = 0;
                let mut replayed = 0;
                for tick in 0..if breeding { 80 } else { 24 } {
                    let (events, _) = advance(&mut expected, world, &mut expected_rng, None);
                    let (serial_events, _) =
                        advance(&mut serial, world, &mut serial_rng, Some(&serial_regions));
                    let (parallel_events, stats) =
                        advance(&mut concurrent, world, &mut concurrent_rng, Some(&parallel));
                    assert_same_mobs(&serial, &expected, species, near, tick, "serial");
                    assert_same_mobs(&concurrent, &expected, species, near, tick, "parallel");
                    assert_eq!(format!("{serial_events:?}"), format!("{events:?}"));
                    assert_eq!(format!("{parallel_events:?}"), format!("{events:?}"));
                    assert!(serial_rng.same_stream(&expected_rng));
                    assert!(concurrent_rng.same_stream(&expected_rng));
                    accepted += stats.committed;
                    replayed += stats.replayed;
                }
                assert!(accepted > 0, "actual predictions must be committed");
                if species == "bee" {
                    assert!(
                        !expected_rng.same_stream(&LegacyRandom::new(1234)),
                        "nectar drips must exercise the shared level RNG"
                    );
                }
                if matches!(species, "cow" | "zombie") && !near {
                    assert_eq!(
                        replayed, 0,
                        "independent real mob ticks should commit without replay"
                    );
                }
                if near {
                    assert!(replayed > 0, "interacting owners must exercise replay");
                }
            }
        });
        // The parity job has the data and says so, so that this can't quietly
        // stop running there.
        assert!(
            ran || std::env::var_os("HORIZON_REQUIRE_TEST_RESOURCES").is_none(),
            "HORIZON_REQUIRE_TEST_RESOURCES is set and the prepared resources are missing"
        );
        if !ran {
            eprintln!(
                "prepared vanilla resources unavailable; resource-backed transaction test skipped"
            );
        }
    }

    fn test_mob(id: u32) -> Mob {
        Mob::new(id, super::super::sensor_scan_tests::kind(), Vec3::ZERO, 0.0)
    }

    #[test]
    fn a_region_wasting_its_predictions_sits_out_and_gets_retried() {
        use crate::server_regions::{RegionId, Regions};
        let mut regions = Regions::default();
        regions.rebuild(1, [(0, 0), (10, 0)].into_iter());
        let (bad, good) = (RegionId((0, 0)), RegionId((10, 0)));
        let mut backoff = PredictionBackoff::default();
        let slot = |id| regions.regions().iter().position(|r| r.id == id).unwrap();
        let (bad_slot, good_slot) = (slot(bad), slot(good));
        // A few bad rounds in a row are not enough, and a good one starts over.
        for _ in 0..3 {
            backoff.observe(bad, 10, 5);
        }
        backoff.observe(bad, 10, 6);
        for _ in 0..3 {
            backoff.observe(bad, 10, 0);
            assert!(backoff.tick(regions.regions())[bad_slot]);
        }
        // The fourth makes it sit out, without touching the other region.
        backoff.observe(bad, 10, 0);
        backoff.observe(good, 10, 10);
        let ticks_out = |backoff: &mut PredictionBackoff| {
            (0..)
                .take_while(|_| {
                    let predicts = backoff.tick(regions.regions());
                    assert!(predicts[good_slot]);
                    !predicts[bad_slot]
                })
                .count()
        };
        assert_eq!(ticks_out(&mut backoff), FIRST_SKIP as usize);
        // Back and still wasting: longer, up to the limit.
        backoff.observe(bad, 10, 0);
        assert_eq!(ticks_out(&mut backoff), FIRST_SKIP as usize * 2);
        for _ in 0..8 {
            backoff.observe(bad, 10, 0);
            ticks_out(&mut backoff);
        }
        backoff.observe(bad, 10, 0);
        assert_eq!(ticks_out(&mut backoff), MAX_SKIP as usize);
        // Back and committing: forgiven, and it takes a full run of waste again.
        backoff.observe(bad, 10, 10);
        for _ in 0..3 {
            backoff.observe(bad, 10, 0);
            assert!(backoff.tick(regions.regions())[bad_slot]);
        }
    }

    #[test]
    fn backoff_forgets_a_region_that_is_gone() {
        use crate::server_regions::{RegionId, Regions};
        let mut regions = Regions::default();
        regions.rebuild(1, [(0, 0)].into_iter());
        let mut backoff = PredictionBackoff::default();
        for _ in 0..WASTED_ROUNDS {
            backoff.observe(RegionId((0, 0)), 10, 0);
        }
        assert_eq!(backoff.tick(regions.regions()), [false]);
        regions.rebuild(2, [(5, 5)].into_iter());
        assert_eq!(backoff.tick(regions.regions()), [true]);
        regions.rebuild(3, [(0, 0)].into_iter());
        assert_eq!(backoff.tick(regions.regions()), [true]);
    }

    /// A prediction of `mob` that commits unless something stopped it.
    fn round_predicting(mob: &Mob) -> (TickRound, seen_grid::SeenGrid) {
        let random = LegacyRandom::new(1);
        let seen = mob.seen(None);
        let grid = seen_grid::SeenGrid::new(std::iter::once(seen.bounds));
        let prediction = TickPrediction {
            slot: 0,
            owner: 0,
            mob: mob.clone(),
            conn: None,
            seen,
            kept: true,
            events: Vec::new(),
            reads: Default::default(),
            random_before: random.clone(),
            random_after: random,
            #[cfg(debug_assertions)]
            input: format!("{mob:?}"),
        };
        (TickRound::new(vec![0], vec![vec![prediction]], 1), grid)
    }

    #[test]
    fn a_commit_replacing_an_untouched_mob_passes_the_check_and_counts() {
        let mut mob = test_mob(1);
        let (mut round, grid) = round_predicting(&mob);
        let mut random = LegacyRandom::new(1);
        let committed = round.commit(0, &mut mob, &grid, &mut random, &mut Vec::new());
        assert!(committed.is_some());
        assert_eq!(round.outcomes().collect::<Vec<_>>(), [(0, 1, 1)]);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "after its prediction was made")]
    fn a_commit_over_a_write_the_prediction_never_saw_panics() {
        let mut mob = test_mob(1);
        let (mut round, grid) = round_predicting(&mob);
        mob.extra.age = 6000;
        let mut random = LegacyRandom::new(1);
        round.commit(0, &mut mob, &grid, &mut random, &mut Vec::new());
    }

    /// A prediction of a mob standing in `cell` of its owner's region.
    fn predicted(id: u32, slot: usize, owner: usize, x: f64) -> (Mob, Seen, TickPrediction) {
        let mut mob = test_mob(id);
        mob.position = Vec3::new(x, 64.0, 0.0);
        let before = mob.seen(None);
        let mut after = mob.clone();
        after.position.x += 0.01;
        let seen = after.seen(None);
        let prediction = TickPrediction {
            slot,
            owner,
            mob: after,
            conn: None,
            seen,
            kept: true,
            events: Vec::new(),
            reads: Default::default(),
            random_before: LegacyRandom::new(1),
            random_after: LegacyRandom::new(1),
            #[cfg(debug_assertions)]
            input: format!("{mob:?}"),
        };
        (mob, before, prediction)
    }

    struct CleanTest {
        round: TickRound,
        mobs: Vec<Mob>,
        snapshot: Vec<Seen>,
        grid: seen_grid::SeenGrid,
        index: FxHashMap<u32, u32>,
    }
    /// Three mobs: slots 0 and 2 share owner 0, far from owner 1's slot 1.
    fn clean_test(edit: impl FnOnce(&mut [TickPrediction; 3])) -> CleanTest {
        let (m0, s0, p0) = predicted(10, 0, 0, 2.0);
        let (m1, s1, p1) = predicted(11, 1, 1, 900.0);
        let (m2, s2, p2) = predicted(12, 2, 0, 6.0);
        let mut predictions = [p0, p1, p2];
        edit(&mut predictions);
        let [p0, p1, p2] = predictions;
        let snapshot = vec![s0, s1, s2];
        let grid = seen_grid::SeenGrid::new(snapshot.iter().map(|seen| seen.bounds));
        let context = MobContext {
            bound_player: None,
            nearest_player: None,
            difficulty: crate::mobs::spawning::LocalDifficulty::new(2, 0, 0, 1.0),
            section_order: 0,
        };
        CleanTest {
            round: TickRound::new(vec![0, 1, 0], vec![vec![p0, p2], vec![p1]], 1)
                .with_contexts(vec![context; 3]),
            mobs: vec![m0, m1, m2],
            snapshot,
            grid,
            index: [(10, 0), (11, 1), (12, 2)].into_iter().collect(),
        }
    }
    fn classify(mut test: CleanTest) -> RegionStats {
        let executor = RegionExecutor::new(2).unwrap();
        test.round
            .classify(&executor, &test.snapshot, &test.grid, &test.index);
        test.round.stats
    }
    fn push(target: u32) -> MobEvent {
        MobEvent::Push {
            target: Who::Mob(target),
            velocity: [0.5, 0.0, 0.0],
        }
    }

    #[test]
    fn a_round_of_independent_predictions_is_clean() {
        let stats = classify(clean_test(|_| {}));
        assert_eq!((stats.clean_rounds, stats.fallback_rounds), (1, 0));
        assert_eq!(stats.fallback, None);
    }

    #[test]
    fn what_a_prediction_does_beyond_itself_sends_the_round_to_the_walk() {
        let cases: [(fn(&mut [TickPrediction; 3]), NotClean); 8] = [
            (
                |p| {
                    let _ = p[1].random_after.next_float();
                },
                NotClean::SharedRandom,
            ),
            (
                |p| {
                    p[0].events.push(MobEvent::BreedTarget {
                        mob: 12,
                        partner: 10,
                    });
                },
                NotClean::BrainWrite,
            ),
            (
                |p| {
                    p[2].events.push(MobEvent::StartRiding {
                        rider: 12,
                        vehicle: 10,
                    });
                },
                NotClean::BrainWrite,
            ),
            (|p| p[0].events.push(push(11)), NotClean::ForeignPush),
            // A full scan, a foreign slot, the foreign mob's cell, its species.
            (|p| p[0].reads.all = true, NotClean::ForeignRead),
            (
                |p| {
                    p[0].reads.slots.insert(1);
                },
                NotClean::ForeignRead,
            ),
            (
                |p| {
                    let area = p[1].seen.bounds;
                    let columns = Some(seen_grid::Columns {
                        lo: seen_grid::corner(area),
                        hi: seen_grid::corner(area),
                    });
                    p[0].reads.queries.push((area, columns, true));
                },
                NotClean::ForeignRead,
            ),
            (
                |p| {
                    p[0].reads.species.insert(p[1].seen.species);
                },
                NotClean::ForeignRead,
            ),
        ];
        for (i, (edit, reason)) in cases.into_iter().enumerate() {
            let stats = classify(clean_test(edit));
            assert_eq!(
                (stats.clean_rounds, stats.fallback_rounds, stats.fallback),
                (0, 1, Some(reason)),
                "case {i}"
            );
        }
    }

    #[test]
    fn reads_and_pushes_inside_a_region_stay_clean() {
        let stats = classify(clean_test(|p| {
            p[0].reads.slots.insert(2);
            p[2].events.push(push(10));
            p[0].events.push(push(12));
            p[0].events.push(push(999));
        }));
        assert_eq!(stats.clean_rounds, 1, "{stats:?}");
    }

    #[test]
    fn a_read_that_the_rounds_growth_would_reshape_is_not_clean() {
        let stats = classify(clean_test(|p| {
            // The foreign mob grows: the reach of the grid the walk would query.
            let wider = p[1].seen.bounds.inflate(Vec3::new(4.0, 0.0, 4.0));
            p[1].seen.bounds = wider;
            let area = Aabb::new(Vec3::new(2.0, 64.0, 0.0), Vec3::new(3.0, 65.0, 1.0));
            let columns = Some(seen_grid::Columns {
                lo: (0, -1),
                hi: (0, 0),
            });
            p[0].reads.queries.push((area, columns, true));
        }));
        assert_eq!(stats.fallback, Some(NotClean::ForeignRead));
    }

    #[test]
    fn a_mob_without_a_prediction_sends_the_round_to_the_walk() {
        let mut test = clean_test(|_| {});
        let (mob, seen, _) = predicted(13, 3, 1, 910.0);
        test.mobs.push(mob);
        test.snapshot.push(seen);
        test.index.insert(13, 3);
        assert_eq!(classify(test).fallback, Some(NotClean::Unpredicted));
    }

    #[test]
    fn a_clean_commit_merges_in_slot_order_and_keeps_only_what_the_walk_would_hold() {
        let attack = |amount| MobEvent::AttackPlayer(amount, [0.0; 3]);
        let mut test = clean_test(|p| {
            let sound = |id: &'static str| MobEvent::Sound(id, [0.0; 3], 1.0, 1.0);
            p[0].events = vec![sound("a"), push(12), sound("b")];
            p[1].events = vec![sound("c"), attack(1.0), sound("d")];
            p[1].conn = Some(7);
            p[2].events = vec![push(10), sound("e"), push(12)];
            p[2].kept = false;
            p[2].mob.health = 0.0;
            p[2].mob.death_time = 20;
            p[2].mob.extra.effects.push(("minecraft:speed", 1, 0));
            p[2].seen.bounds = p[2].seen.bounds.inflate(Vec3::new(40.0, 0.0, 40.0));
        });
        let (mut keep, mut events, mut guest_events) = (Vec::new(), Vec::new(), Vec::new());
        let (mut dead, mut corpses, mut gone) = (Vec::new(), Vec::new(), Vec::new());
        let mut pushes = PendingPushes::default();
        let mut died = crate::fast_hash::FxHashSet::default();
        let mut bindings = FxHashMap::default();
        bindings.insert(11, 3);
        let executor = RegionExecutor::new(2).unwrap();
        let plan = test
            .round
            .classify(&executor, &test.snapshot, &test.grid, &test.index);
        let wanted_bounds = test.round.predictions[0][1].seen.bounds;
        let mut expected_grid = test.grid.clone();
        expected_grid.update(2, wanted_bounds);
        test.round.contexts[1].bound_player = Some(3);
        test.round.commit_clean(
            &executor,
            plan.expect("clean"),
            &mut test.mobs,
            &mut test.snapshot,
            &mut test.grid,
            &CleanContext {
                guests: true,
                guest_ids: &[(7, 70)],
                acts_on_player: |event| matches!(event, MobEvent::AttackPlayer(..)),
                index: &test.index,
            },
            &mut WalkOutput {
                keep: &mut keep,
                events: &mut events,
                guest_events: &mut guest_events,
                dead: &mut dead,
                corpses: &mut corpses,
                gone: &mut gone,
                pushes: &mut pushes,
                died: &mut died,
                bindings: &mut bindings,
            },
        );
        assert_eq!(keep, [true, true, false]);
        let order: Vec<_> = events
            .iter()
            .map(|event| match event {
                MobEvent::Sound(id, ..) => *id,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(order, ["a", "b", "c", "d", "e"]);
        assert_eq!(guest_events.len(), 1);
        assert!(matches!(
            guest_events[0],
            (7, 11, MobEvent::AttackPlayer(..))
        ));
        // Slot 0's push reached slot 2 in its prediction. Slot 2's at itself and
        // at slot 0, which the walk has passed, wait for the end of the walk.
        let waiting: Vec<_> = pushes.waiting().into_keys().collect();
        assert_eq!(waiting, [10, 12]);
        assert_eq!(gone.len(), 1);
        assert_eq!(corpses.len(), 1);
        assert_eq!(dead.len(), 1);
        assert!(died.is_empty(), "a body that went needs no record");
        assert_eq!(bindings.get(&11), Some(&7));
        assert!(test.grid == expected_grid);
        assert_eq!(test.snapshot[2].bounds, wanted_bounds);
        assert_eq!(test.mobs[0].position.x, 2.01);
        assert_eq!(test.round.stats.committed, 3);
        assert_eq!(
            test.round.outcomes().collect::<Vec<_>>(),
            [(0, 2, 2), (1, 1, 1)]
        );
    }

    #[test]
    fn a_panic_during_the_tick_leaves_the_mobs_to_be_recovered() {
        let rescue = MobRescue::default();
        let mut mobs = crate::join_seq::JoinList::default();
        mobs.push(test_mob(7));
        let died = crate::fast_hash::FxHashSet::from_iter([7]);
        let held = rescue.clone();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _taken = held.hold(mobs, died);
            panic!("a tick");
        }));
        assert!(unwound.is_err());
        let (mobs, died) = rescue.recover().expect("left for the crash save");
        assert_eq!(mobs.iter().map(|m| m.id).collect::<Vec<_>>(), [7]);
        assert!(died.contains(&7));
        assert!(rescue.recover().is_none(), "recovered once");
    }

    #[test]
    fn a_tick_that_ends_normally_leaves_nothing_to_recover() {
        let rescue = MobRescue::default();
        drop(rescue.hold(Default::default(), Default::default()));
        assert!(rescue.recover().is_none());
    }

    #[test]
    fn verify_accepts_equal_results_and_names_a_difference() {
        let (a, mut b) = (test_mob(1), test_mob(1));
        let random = LegacyRandom::new(1);
        verify_commit(0, (&a, true, &[], &random), (&b, true, &[], &random));
        b.extra.age = 6000;
        let message = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            verify_commit(0, (&a, true, &[], &random), (&b, true, &[], &random));
        }))
        .unwrap_err();
        let message = message.downcast_ref::<String>().unwrap();
        assert!(message.contains("differ in the mob"), "{message}");
    }
}
