//! Regions: the parts of a level that tick on threads of their own.
//!
//! A level is cut into sections of `4×4` chunks. A section is *ticking* when a chunk in it ticks (or a player
//! stands in it) and a *view* section when it is only inside a player's view distance. Ticking sections
//! [`MERGE_RADIUS`] sections apart or closer are one region; a view section is one region with the sections it
//! touches. A region owns its sections and a ring of one section around each ticking one, so two regions' chunks
//! never meet and a region's ticks find the chunks next to theirs.
//!
//! This module only decides which region owns what. It is pure data: no game state, no threads.
// Nothing calls it until the region threads come in the next commits.
#![allow(dead_code)]
use super::{Dim, Game, Mob, Resources};
use horizon_client::Result;
use horizon_client::join_seq::JoinList;
use horizon_core::fast_hash::FxHashMap;
use horizon_core::position::ChunkPos;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// A region's game moves to a thread of its own: `Game` has to be `Send` (checked when this compiles).
const _: fn() = || {
    fn send<T: Send>() {}
    send::<Game<'static>>();
};

/// Chunks per side of a region section, as a shift: 2^2 = 4.
pub(super) const SECTION_SHIFT: i32 = 2;
/// Ticking sections this many sections apart or closer are one region.
pub(super) const MERGE_RADIUS: i32 = 2;

/// Which region each loaded chunk belongs to.
pub(super) struct Regions {
    /// The region of each owned section.
    owned: FxHashMap<(i32, i32), usize>,
    /// How many regions the level has.
    pub count: usize,
    /// Each region's first section (its smallest): what names a region from one tick to the next.
    pub firsts: Vec<(i32, i32)>,
    /// `owned` as a flat table over the sections' bounds, when that is small enough: every split of the level asks
    /// for each chunk's and entity's region (tens of thousands a tick), one array read each.
    grid: Option<Grid>,
}

/// The owned sections' regions in one array over their bounding box (`u16::MAX`: no region).
struct Grid {
    x0: i32,
    z0: i32,
    w: i32,
    h: i32,
    cells: Vec<u16>,
}

impl Grid {
    /// Sections in the table at most (4 M cells, 8 MB): players spread wider than that keep the map.
    const MAX_CELLS: i64 = 1 << 22;
    fn of(owned: &FxHashMap<(i32, i32), usize>) -> Option<Self> {
        if owned.is_empty() {
            return None;
        }
        let (mut x0, mut z0, mut x1, mut z1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &(x, z) in owned.keys() {
            x0 = x0.min(x);
            z0 = z0.min(z);
            x1 = x1.max(x);
            z1 = z1.max(z);
        }
        let (w, h) = (
            i64::from(x1) - i64::from(x0) + 1,
            i64::from(z1) - i64::from(z0) + 1,
        );
        if w * h > Self::MAX_CELLS || owned.values().any(|&r| r >= usize::from(u16::MAX)) {
            return None;
        }
        let (w, h) = (w as i32, h as i32);
        let mut cells = vec![u16::MAX; (w * h) as usize];
        for (&(x, z), &r) in owned {
            cells[((z - z0) * w + (x - x0)) as usize] = r as u16;
        }
        Some(Self {
            x0,
            z0,
            w,
            h,
            cells,
        })
    }
    #[inline]
    fn get(&self, x: i32, z: i32) -> Option<usize> {
        let (dx, dz) = (x.wrapping_sub(self.x0), z.wrapping_sub(self.z0));
        if dx < 0 || dz < 0 || dx >= self.w || dz >= self.h {
            return None;
        }
        match self.cells[(dz * self.w + dx) as usize] {
            u16::MAX => None,
            r => Some(usize::from(r)),
        }
    }
}

const TICKING: u8 = 1;
const VIEW: u8 = 2;

impl Regions {
    /// The regions for the chunks that tick (`ticking`) and the players' chunks (`players`), each seeing `view`
    /// chunks around it (`view <= 0`: a player stands in a ticking section and sees nothing of its own).
    ///
    /// Only regions with a player in them are kept when there are players: a region without one gets no thread, its
    /// chunks tick with the rest no region owns.
    pub(super) fn new(
        ticking: impl IntoIterator<Item = (i32, i32)>,
        players: &[(i32, i32)],
        view: i32,
    ) -> Self {
        let mut kind: FxHashMap<(i32, i32), u8> = FxHashMap::default();
        for (x, z) in ticking {
            kind.insert((x >> SECTION_SHIFT, z >> SECTION_SHIFT), TICKING);
        }
        for &(x, z) in players {
            if view <= 0 {
                kind.insert((x >> SECTION_SHIFT, z >> SECTION_SHIFT), TICKING);
                continue;
            }
            for sx in (x - view) >> SECTION_SHIFT..=(x + view) >> SECTION_SHIFT {
                for sz in (z - view) >> SECTION_SHIFT..=(z + view) >> SECTION_SHIFT {
                    kind.entry((sx, sz)).or_insert(VIEW);
                }
            }
        }
        // The same numbering every tick for the same sections: regions in the order of their first section.
        let mut keys: Vec<(i32, i32)> = kind.keys().copied().collect();
        keys.sort_unstable();
        let mut region: FxHashMap<(i32, i32), usize> = FxHashMap::default();
        let mut count = 0;
        let mut firsts = Vec::new();
        for start in keys {
            if region.contains_key(&start) {
                continue;
            }
            firsts.push(start);
            region.insert(start, count);
            let mut stack = vec![start];
            while let Some((x, z)) = stack.pop() {
                let mine = kind[&(x, z)];
                let reach = if mine == TICKING { MERGE_RADIUS } else { 1 };
                for dx in -reach..=reach {
                    for dz in -reach..=reach {
                        let next = (x + dx, z + dz);
                        let Some(&theirs) = kind.get(&next) else {
                            continue;
                        };
                        if region.contains_key(&next) {
                            continue;
                        }
                        let apart = dx.abs().max(dz.abs());
                        let joins = if mine == TICKING && theirs == TICKING {
                            apart <= MERGE_RADIUS
                        } else {
                            apart <= 1
                        };
                        if joins {
                            region.insert(next, count);
                            stack.push(next);
                        }
                    }
                }
            }
            count += 1;
        }
        if !players.is_empty() {
            let mut has_player = vec![false; count];
            for &(x, z) in players {
                if let Some(&r) = region.get(&(x >> SECTION_SHIFT, z >> SECTION_SHIFT)) {
                    has_player[r] = true;
                }
            }
            if has_player.iter().any(|p| !p) {
                let mut renumber = vec![usize::MAX; count];
                let mut kept = 0;
                for (r, _) in has_player.iter().enumerate().filter(|(_, p)| **p) {
                    renumber[r] = kept;
                    kept += 1;
                }
                region.retain(|_, r| renumber[*r] != usize::MAX);
                for r in region.values_mut() {
                    *r = renumber[*r];
                }
                firsts = firsts
                    .into_iter()
                    .enumerate()
                    .filter(|(r, _)| has_player[*r])
                    .map(|(_, f)| f)
                    .collect();
                count = kept;
            }
        }
        let mut owned = FxHashMap::default();
        for (&(x, z), &r) in &region {
            if kind[&(x, z)] == TICKING {
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        owned.insert((x + dx, z + dz), r);
                    }
                }
            }
        }
        for (&key, &r) in &region {
            owned.insert(key, r);
        }
        let grid = Grid::of(&owned);
        Self {
            owned,
            count,
            firsts,
            grid,
        }
    }

    /// The region that owns chunk `(x, z)`.
    #[inline]
    pub(super) fn of_chunk(&self, (x, z): (i32, i32)) -> Option<usize> {
        let (sx, sz) = (x >> SECTION_SHIFT, z >> SECTION_SHIFT);
        match &self.grid {
            Some(g) => g.get(sx, sz),
            None => self.owned.get(&(sx, sz)).copied(),
        }
    }

    /// The region that owns the block column at `x, z`.
    pub(super) fn of_block(&self, x: f64, z: f64) -> Option<usize> {
        self.of_chunk(((x.floor() as i32) >> 4, (z.floor() as i32) >> 4))
    }
}

/// Entity ids a region game takes from the shared counter at a time, and how few it may have left before a tick
/// before it takes more (no tick hands out anywhere near that many).
const ID_BLOCK: u32 = 1 << 20;
const ID_REFILL: u32 = 1 << 15;

/// What a game keeps for ticking by regions: whether it is a region's game, its handle on the resources that
/// outlives the server's borrow, the end of its block of entity ids, and the games made for regions and kept.
#[derive(Default)]
pub(super) struct RegionState {
    /// This game is a region's: it has no level, players or connections of its own, they are lent in for a tick.
    pub worker: bool,
    /// The resources as a region game holds them, made once and kept for the server's life.
    static_res: Option<&'static Arc<Resources>>,
    /// The end of the block of entity ids this game hands out (a region game's, [`Game::refill_entity_ids`]).
    pub entity_id_end: u32,
    /// The server's shared counter the blocks of entity ids come from.
    pub entity_pool: Option<Arc<AtomicU32>>,
    /// Region games made and not in use, by dimension.
    pub idle: std::collections::HashMap<Dim, Vec<Box<Game<'static>>>>,
    /// Region games being made on threads of their own (making one reads files and builds a whole game: a stall
    /// of seconds on the server's thread), taken in once made.
    factory: Vec<std::sync::mpsc::Receiver<std::result::Result<Box<Game<'static>>, String>>>,
}

impl Game<'_> {
    /// `n` more region games made on threads of their own.
    fn request_region_games(&mut self, n: usize) {
        let res = self.static_resources();
        for _ in 0..n {
            let (tx, rx) = std::sync::mpsc::channel();
            let (pack, options, settings) = (
                self.pack.clone(),
                self.options.clone(),
                self.settings.clone(),
            );
            let spawned = std::thread::Builder::new()
                .name("Region game maker".into())
                .spawn(move || {
                    let made = super::integrated::server_game(res, pack, options, settings)
                        .map(Box::new)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(made);
                });
            if spawned.is_ok() {
                self.regions.factory.push(rx);
            }
        }
    }

    /// The region games made since, set up for the level in place and kept.
    fn collect_region_games(&mut self) {
        let mut i = 0;
        while i < self.regions.factory.len() {
            match self.regions.factory[i].try_recv() {
                Ok(made) => {
                    self.regions.factory.remove(i);
                    match made {
                        Ok(mut w) => {
                            w.copy_world_settings_from(self);
                            w.regions.worker = true;
                            w.next_entity = 0;
                            w.regions.entity_id_end = 0;
                            self.regions.idle.entry(self.dim).or_default().push(w);
                        }
                        Err(e) => log::warn!("a region game could not be made: {e}"),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => i += 1,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.regions.factory.remove(i);
                }
            }
        }
    }

    /// The resources as a region game holds them: its own handle, made once and kept for the server's life.
    fn static_resources(&mut self) -> &'static Arc<Resources> {
        match self.regions.static_res {
            Some(r) => r,
            None => {
                let r: &'static Arc<Resources> = Box::leak(Box::new(Arc::clone(&self.res)));
                self.regions.static_res = Some(r);
                r
            }
        }
    }

    /// A game for a region's thread: the same resources, options and world settings as this one, and no level,
    /// players or connections of its own (a region's are lent in for its tick).
    pub(super) fn region_worker(&mut self) -> Result<Game<'static>> {
        let options = self.options.clone();
        let res = self.static_resources();
        let mut w =
            super::integrated::server_game(res, self.pack.clone(), options, self.settings.clone())?;
        w.copy_world_settings_from(self);
        w.regions.worker = true;
        // No ids of its own: the first tick draws a block from the server's counter (`refill_entity_ids`).
        w.next_entity = 0;
        w.regions.entity_id_end = 0;
        Ok(w)
    }

    /// What every region game shares with the server's: the world's settings and tables, as they are now
    /// (copied again when they change: `/gamerule`, `/difficulty`, `/reload`).
    pub(super) fn copy_world_settings_from(&mut self, from: &Game<'_>) {
        self.level = from.level.clone();
        self.level_shapes = from.level_shapes;
        self.world_root = from.world_root.clone();
        self.world_biomes = from.world_biomes.clone();
        self.end.set_locator(from.end.locator());
        self.dim = from.dim;
        self.world_flat = from.world_flat;
        self.world_debug = from.world_debug;
        // The server's tables, shared: building them reads every item and recipe file from disk.
        self.world_tables = from.world_tables.clone();
        self.share_spawn_conditions();
        self.copy_tick_settings_from(from);
    }

    /// What can change from tick to tick that a region reads but never writes: the rules, the difficulty, the
    /// clock, the border, the ticket levels.
    pub(super) fn copy_tick_settings_from(&mut self, from: &Game<'_>) {
        self.ui.rules = from.ui.rules.clone();
        self.ui.difficulty = from.ui.difficulty;
        self.clock = from.clock.clone();
        self.ticks = from.ticks;
        self.border = from.border.clone();
        self.tickets = from.tickets.clone();
        self.level.difficulty_locked = from.level.difficulty_locked;
    }

    /// The next entity ids this region game hands out: a block of its own from `pool` (the server's shared
    /// counter) when the one it has runs low, so no two threads ever give out the same id.
    pub(super) fn refill_entity_ids(&mut self, pool: &AtomicU32) {
        if self.regions.entity_id_end.saturating_sub(self.next_entity) < ID_REFILL {
            let start = pool.fetch_add(ID_BLOCK, Ordering::Relaxed);
            self.next_entity = start;
            self.regions.entity_id_end = start + ID_BLOCK;
        }
    }
}

impl Game<'_> {
    /// Region `part` of the level in place, moved into region game `w`: its chunks with their block entities,
    /// block and fluid ticks and random-tick caches, every entity that stands in it, and what is kept by block or
    /// by entity in it.
    pub(super) fn lend_region(&mut self, w: &mut Game<'_>, mut part: RegionPart) {
        w.world = part.world;
        w.block_ticks = part.ticks;
        w.random_tick_sections = part.sections;
        w.rand_sequences = part.sequences;
        w.mobs = part.mobs.into();
        w.creaking.lend_scanned(part.scanned);
        w.dropped = part.dropped.into();
        w.orbs = part.orbs.into();
        w.creatures.projectiles = part.projectiles.into();
        w.falling = part.falling.into();
        w.paintings = part.paintings;
        w.item_frames = part.item_frames;
        w.armor_stands = part.armor_stands.into();
        w.cushions = part.cushions;
        w.others = part.others;
        w.boats = part.boats.into();
        w.carts = part.carts.into();
        w.bolts = part.bolts;
        w.crystals = part.crystals;
        w.breath_clouds = part.breath_clouds;
        w.tridents = part.tridents.into();
        w.rockets = part.rockets.into();
        w.ominous_spawners = part.ominous_spawners;
        w.tnts = part.tnts.into();
        w.hoppers = part.hoppers;
        w.jukebox_sounds = part.jukebox_sounds;
        w.lid_openers = part.lid_openers;
        w.knot_ids = part.knot_ids;
        w.lectern_pulses = part.lectern_pulses;
        let a = &mut w.be_anims;
        a.chests = std::mem::take(&mut part.anims.chests);
        a.books = std::mem::take(&mut part.anims.books);
        a.shulkers = std::mem::take(&mut part.anims.shulkers);
        a.bells = std::mem::take(&mut part.anims.bells);
        a.conduits = std::mem::take(&mut part.anims.conduits);
        a.skulls = std::mem::take(&mut part.anims.skulls);
        a.pots = std::mem::take(&mut part.anims.pots);
        a.gateway_servers = std::mem::take(&mut part.anims.gateway_servers);
        a.conduit_servers = std::mem::take(&mut part.anims.conduit_servers);
        // The ids of the region's entities: what is remembered of each goes with them.
        let mut ids: Vec<u32> = w.mobs.iter().map(|m| m.id).collect();
        ids.extend(w.dropped.iter().map(|d| d.id));
        ids.extend(w.boats.iter().map(|b| b.id));
        ids.extend(w.carts.iter().map(|c| c.id));
        ids.extend(w.armor_stands.iter().map(|s| s.id));
        ids.extend(w.orbs.iter().map(|o| o.id as u32));
        ids.extend(w.creatures.projectiles.iter().map(|p| p.id as u32));
        ids.extend(w.falling.iter().map(|f| f.id as u32));
        ids.extend(w.tnts.iter().map(|t| t.id as u32));
        ids.extend(w.tridents.iter().map(|t| t.id as u32));
        ids.extend(w.rockets.iter().map(|r| r.id as u32));
        ids.extend(w.crystals.iter().map(|c| c.id as u32));
        ids.extend(w.bolts.iter().map(|b| b.id as u32));
        ids.extend(w.ominous_spawners.iter().map(|o| o.id as u32));
        ids.extend(w.breath_clouds.iter().map(|b| b.id as u32));
        ids.extend(w.paintings.iter().map(|p| p.id as u32));
        ids.extend(w.item_frames.iter().map(|f| f.id as u32));
        ids.extend(w.cushions.iter().map(|c| c.id as u32));
        ids.extend(w.others.iter().map(|o| o.id as u32));
        for id in ids {
            if let Some(u) = self.persist.uuids.remove(&id) {
                w.persist.uuids.insert(id, u);
                if let Some(attached) = self.entity_attachments.remove(&u) {
                    w.entity_attachments.insert(u, attached);
                }
            }
            if let Some(t) = self.persist.raw.remove(&id) {
                w.persist.raw.insert(id, t);
            }
            if let Some(slots) = self.persist.passenger_slots.remove(&id) {
                w.persist.passenger_slots.insert(id, slots);
            }
            if self.died.remove(&id) {
                w.died.insert(id);
            }
            if let Some(v) = self.love_cause.remove(&id) {
                w.love_cause.insert(id, v);
            }
            if let Some(v) = self.last_hurt_by_player.remove(&id) {
                w.last_hurt_by_player.insert(id, v);
            }
        }
        w.world.game_time = self.world.game_time;
        w.world.day_time = self.world.day_time;
        w.world.weather = self.world.weather;
        w.world.health_rules = self.world.health_rules.clone();
        w.world.watched = self.world.watched.clone();
    }

    /// Region game `w`'s part of the level back in place (after the level's own data, in region order), with
    /// what its tick left for the server: level events, spills, sounds and particles for the network.
    pub(super) fn take_region_back(
        &mut self,
        w: &mut Game<'_>,
        chunks: &[ChunkPos],
    ) -> horizon_client::random_sequences::RandomSequences {
        let stand_in = self.world.stand_in();
        let region_world = std::mem::replace(&mut w.world, stand_in);
        let region_ticks = std::mem::take(&mut w.block_ticks);
        let region_sections = std::mem::take(&mut w.random_tick_sections);
        let mut lt = std::time::Instant::now();
        let world = &mut self.world;
        std::thread::scope(|scope| {
            let back = scope.spawn(move || world.take_back(region_world));
            self.block_ticks.take_back(region_ticks, chunks);
            self.random_tick_sections.take_back(region_sections);
            if let Err(e) = back.join() {
                std::panic::resume_unwind(e);
            }
        });
        lap(&mut lt, "land.world+ticks+sections");
        append_all(&mut self.mobs, drain(&mut w.mobs));
        self.creaking.take_scanned_back(&mut w.creaking);
        append_all(&mut self.dropped, drain(&mut w.dropped));
        append_all(&mut self.orbs, drain(&mut w.orbs));
        append_all(
            &mut self.creatures.projectiles,
            drain(&mut w.creatures.projectiles),
        );
        append_all(&mut self.falling, drain(&mut w.falling));
        self.paintings.append(&mut w.paintings);
        self.item_frames.append(&mut w.item_frames);
        append_all(&mut self.armor_stands, drain(&mut w.armor_stands));
        self.cushions.append(&mut w.cushions);
        self.others.append(&mut w.others);
        append_all(&mut self.boats, drain(&mut w.boats));
        append_all(&mut self.carts, drain(&mut w.carts));
        self.bolts.append(&mut w.bolts);
        self.crystals.append(&mut w.crystals);
        self.breath_clouds.append(&mut w.breath_clouds);
        append_all(&mut self.tridents, drain(&mut w.tridents));
        append_all(&mut self.rockets, drain(&mut w.rockets));
        self.ominous_spawners.append(&mut w.ominous_spawners);
        append_all(&mut self.tnts, drain(&mut w.tnts));
        self.hoppers.extend(w.hoppers.drain());
        self.jukebox_sounds.extend(w.jukebox_sounds.drain());
        self.lid_openers.extend(w.lid_openers.drain());
        self.knot_ids.extend(w.knot_ids.drain());
        self.lectern_pulses.append(&mut w.lectern_pulses);
        let (a, b) = (&mut self.be_anims, &mut w.be_anims);
        a.chests.extend(b.chests.drain());
        a.books.extend(b.books.drain());
        a.shulkers.extend(b.shulkers.drain());
        a.bells.extend(b.bells.drain());
        a.conduits.extend(b.conduits.drain());
        a.skulls.extend(b.skulls.drain());
        a.pots.extend(b.pots.drain());
        a.gateway_servers.extend(b.gateway_servers.drain());
        a.conduit_servers.extend(b.conduit_servers.drain());
        lap(&mut lt, "land.entity lists");
        self.persist.uuids.extend(w.persist.uuids.drain());
        self.persist.raw.extend(w.persist.raw.drain());
        self.persist
            .passenger_slots
            .extend(w.persist.passenger_slots.drain());
        self.entity_attachments.extend(w.entity_attachments.drain());
        self.died.extend(w.died.drain());
        self.love_cause.extend(w.love_cause.drain());
        self.last_hurt_by_player
            .extend(w.last_hurt_by_player.drain());
        lap(&mut lt, "land.persist maps");
        // What the region's tick left for the server's game, after the server's own.
        self.level_events.append(&mut w.level_events);
        self.pending.absorb(std::mem::take(&mut w.pending));
        self.spill.absorb(std::mem::take(&mut w.spill));
        self.creatures
            .lan_sounds
            .append(&mut w.creatures.lan_sounds);
        self.creatures
            .lan_particles
            .append(&mut w.creatures.lan_particles);
        self.lan_events.append(&mut w.lan_events);
        self.lan_block_events.append(&mut w.lan_block_events);
        self.lan_entity_events.append(&mut w.lan_entity_events);
        self.lan_critical_hits.append(&mut w.lan_critical_hits);
        self.lan_cushion_breaks.append(&mut w.lan_cushion_breaks);
        self.lan_songs.append(&mut w.lan_songs);
        self.lan_kinetic_hits.append(&mut w.lan_kinetic_hits);
        self.lan_global_events.append(&mut w.lan_global_events);
        lap(&mut lt, "land.events");
        std::mem::take(&mut w.rand_sequences)
    }
}

/// Whether the level's world phases run by regions on threads of their own (`HORIZON_REGION_THREADS=0` turns it
/// off): only with the parallel tick on, on a server with players in the level.
fn region_threads_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("HORIZON_REGION_THREADS").map_or(true, |v| v.trim() != "0"))
        && horizon_client::parallel::parallel_tick() > 0
}

impl Game<'_> {
    /// The world phases of this tick by regions, on threads of their own: the level split into regions' parts,
    /// each ticked in a region game of its own with its players, then everything landed back in place. Returns
    /// whether it did (`false`: fewer than two regions, nothing was changed and the serial phases run). The
    /// network turn and everything no region owns (portals, sculk, the mods' flushes, the players' own ticks)
    /// still run on this thread afterwards.
    pub(super) fn tick_world_regions(&mut self) -> Result<bool> {
        if !region_threads_on() || self.regions.worker {
            return Ok(false);
        }
        let Some(host) = self.lan.as_ref() else {
            return Ok(false);
        };
        let players: Vec<(i32, i32)> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g))
            .map(|g| {
                (
                    (g.position[0].floor() as i32) >> 4,
                    (g.position[2].floor() as i32) >> 4,
                )
            })
            .collect();
        if players.len() < 2 {
            return Ok(false);
        }
        let levels = self.tickets.levels();
        let ticking = self
            .world
            .chunks
            .keys()
            .copied()
            .filter(|&(x, z)| levels.block_ticking(x, z));
        let regions = Regions::new(ticking, &players, 0);
        if regions.count < 2 {
            return Ok(false);
        }
        let dim = self.dim;
        static LAST: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        if LAST.swap(regions.count, Ordering::Relaxed) != regions.count {
            log::info!(
                "ticking {} regions for {} players",
                regions.count,
                players.len()
            );
        }
        // Games for the regions: made on demand, kept for the next ticks.
        self.collect_region_games();
        let have = self.regions.idle.get(&dim).map_or(0, Vec::len);
        if have < regions.count {
            // Not enough games yet: they are made meanwhile, and this tick runs as one.
            let coming = self.regions.factory.len();
            let wanted = (regions.count - have).saturating_sub(coming).min(4);
            if wanted > 0 {
                self.request_region_games(wanted);
            }
            return Ok(false);
        }
        let mut games = self.regions.idle.remove(&dim).unwrap_or_default();
        let extra: Vec<Box<Game<'static>>> = games.split_off(regions.count);
        let pool = Arc::clone(
            self.regions
                .entity_pool
                .get_or_insert_with(|| Arc::new(AtomicU32::new(self.next_entity + 1))),
        );
        if self.regions.entity_id_end == 0 {
            self.regions.entity_id_end = self.next_entity;
        }
        self.refill_entity_ids(&pool);
        // Each region's players, by connection, split off once.
        let region_of_conn: HashMap<u32, usize> = self
            .lan
            .as_ref()
            .map(|h| {
                h.hub
                    .guests
                    .iter()
                    .filter(|g| h.hub.here(g))
                    .filter_map(|g| {
                        let chunk = (
                            (g.position[0].floor() as i32) >> 4,
                            (g.position[2].floor() as i32) >> 4,
                        );
                        Some((g.conn, regions.of_chunk(chunk)?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let t0 = std::time::Instant::now();
        let mut host = self.lan.take().expect("checked above");
        let mut split_guests = host
            .hub
            .split_guests(regions.count, |conn| region_of_conn.get(&conn).copied());
        let split = self.split_level(&regions);
        let mut flights: Vec<(Box<Game<'static>>, Vec<ChunkPos>, Vec<u32>)> =
            Vec::with_capacity(regions.count);
        for ((part, mut w), (chunks, guests)) in split.parts.into_iter().zip(games).zip(
            split
                .chunks
                .into_iter()
                .zip(split_guests.iter_mut().map(std::mem::take)),
        ) {
            let mob_ids: Vec<u32> = part.mobs.iter().map(|m| m.id).collect();
            let mut lt = std::time::Instant::now();
            self.lend_region(&mut w, part);
            lap(&mut lt, "lend.lend_region");
            w.copy_tick_settings_from(self);
            w.refill_entity_ids(&pool);
            lap(&mut lt, "lend.settings");
            let conns: Vec<u32> = guests.iter().map(|g| g.conn).collect();
            let mut shell = w.lan.take().unwrap_or_else(|| host.region_shell());
            host.lend_region(&mut shell, &conns, &mob_ids, dim, guests);
            lap(&mut lt, "lend.host");
            w.lan = Some(shell);
            flights.push((w, chunks, conns));
        }
        let t_lend = t0.elapsed();
        // The regions tick at once: a thread for each group of them, as many as there are cores.
        let lanes = horizon_threads::worker_count()
            .clamp(1, 16)
            .min(flights.len());
        let mut results: Vec<Result<()>> = Vec::new();
        {
            let per = flights.len().div_ceil(lanes);
            let outcomes: Vec<Vec<std::result::Result<(), String>>> = std::thread::scope(|scope| {
                let handles: Vec<_> = flights
                    .chunks_mut(per)
                    .map(|group| {
                        scope.spawn(move || {
                            group
                                .iter_mut()
                                .map(|(w, _, _)| {
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        w.tick_region_phases().map_err(|e| e.to_string())
                                    }))
                                    .unwrap_or_else(|_| Err("a region thread panicked".to_owned()))
                                })
                                .collect()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or_default())
                    .collect()
            });
            for outcome in outcomes.into_iter().flatten() {
                results.push(outcome.map_err(Into::into));
            }
        }
        let t_run = t0.elapsed();
        // Landed in region order.
        let mut back = Vec::with_capacity(flights.len());
        for (mut w, chunks, _) in flights {
            self.take_region_back(&mut w, &chunks);
            if let Some(shell) = w.lan.as_mut() {
                host.take_region_back(shell);
            }
            back.push(w);
        }
        host.hub.restore_guest_order();
        self.lan = Some(host);
        back.extend(extra);
        self.regions.idle.insert(dim, back);
        {
            use std::sync::atomic::AtomicU64;
            static N: AtomicU64 = AtomicU64::new(0);
            static SUM: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
            let t_land = t0.elapsed();
            let us = |d: std::time::Duration| d.as_micros() as u64;
            SUM[0].fetch_add(us(t_lend), Ordering::Relaxed);
            SUM[1].fetch_add(us(t_run) - us(t_lend), Ordering::Relaxed);
            SUM[2].fetch_add(us(t_land) - us(t_run), Ordering::Relaxed);
            let n = N.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 200 == 0 {
                let avg = |i: usize| SUM[i].swap(0, Ordering::Relaxed) as f64 / 200_000.0;
                {
                    let mut laps = LAPS
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let line: Vec<String> = laps
                        .iter()
                        .map(|(n, us)| format!("{n} {:.1}", *us as f64 / 200_000.0))
                        .collect();
                    laps.clear();
                    let mut world = horizon_client::world::LEND_LAPS
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let wline: Vec<String> = world
                        .iter()
                        .map(|(n, us)| format!("{n} {:.1}", *us as f64 / 200_000.0))
                        .collect();
                    world.clear();
                    log::info!("world.lend laps (ms/tick): {}", wline.join(", "));
                    log::info!(
                        "block entities {} tickers-ish, asleep etc; chunks {}",
                        self.world.block_entities.len(),
                        self.world.chunks.len()
                    );
                    log::info!("regions laps (ms/tick): {}", line.join(", "));
                }
                log::info!(
                    "regions: split+lend {:.1} ms, run {:.1} ms, land {:.1} ms (avg of 200 ticks, {} regions)",
                    avg(0),
                    avg(1),
                    avg(2),
                    regions.count
                );
            }
        }
        results.into_iter().collect::<Result<Vec<()>>>()?;
        Ok(true)
    }

    /// One region game's phases of the tick: the level's entities and blocks in its chunks, around its players.
    pub(super) fn tick_region_phases(&mut self) -> Result<()> {
        self.begin_ordered_ticks();
        self.tick_blocks()?;
        self.tick_paintings();
        self.tick_item_frames();
        self.tick_armor_stands()?;
        self.tick_cushions();
        self.tick_boats();
        self.tick_carts();
        self.tick_thunder();
        self.tick_lecterns();
        self.tick_bolts();
        self.tick_fishing();
        self.tick_tridents();
        self.tick_rockets();
        self.tick_ominous_item_spawners();
        self.tick_items();
        self.hoppers_catch_items();
        self.touch_arrows();
        self.tick_mobs()?;
        self.recheck_golem_openers();
        self.run_ordered_ticks(horizon_client::join_seq::JoinSeq::MAX, false)?;
        self.tick_experience();
        self.tick_moving_pistons()?;
        self.tick_block_entities()?;
        Ok(())
    }
}

/// Time spent in the region phases by name, summed (`lap`) and logged every 200 ticks.
static LAPS: std::sync::Mutex<Vec<(&'static str, u64)>> = std::sync::Mutex::new(Vec::new());

/// The time since `t`, added to `name`'s; `t` starts again.
fn lap(t: &mut std::time::Instant, name: &'static str) {
    let us = t.elapsed().as_micros() as u64;
    *t = std::time::Instant::now();
    let mut laps = LAPS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match laps.iter_mut().find(|(n, _)| *n == name) {
        Some(entry) => entry.1 += us,
        None => laps.push((name, us)),
    }
}

/// `v`'s items whose place a region owns, moved into that region's list; the rest stay.
fn split<T>(v: &mut Vec<T>, regions: &Regions, at: impl Fn(&T) -> (f64, f64)) -> Vec<Vec<T>> {
    let places: Vec<Option<usize>> = v
        .iter()
        .map(|item| {
            let (x, z) = at(item);
            regions.of_block(x, z)
        })
        .collect();
    let mut sizes = vec![0usize; regions.count];
    for r in places.iter().flatten() {
        sizes[*r] += 1;
    }
    let mut out: Vec<Vec<T>> = sizes.iter().map(|&n| Vec::with_capacity(n)).collect();
    let mut kept = Vec::new();
    for (item, place) in v.drain(..).zip(places) {
        match place {
            Some(r) => out[r].push(item),
            None => kept.push(item),
        }
    }
    *v = kept;
    out
}

/// `v`'s entities at the end of `list`, as they are (they joined a level already: their order is kept).
fn append_all<T>(list: &mut JoinList<T>, mut v: Vec<T>) {
    list.append(&mut v);
}

/// A list of entities as a plain vector, the list left empty.
fn drain<T>(list: &mut JoinList<T>) -> Vec<T> {
    std::mem::take(list).into()
}

/// A map's entries whose block a region owns, moved into that region's map.
fn split_map<K: std::hash::Hash + Eq + Copy, V>(
    m: &mut HashMap<K, V>,
    regions: &Regions,
    at: impl Fn(&K) -> (i32, i32),
) -> Vec<HashMap<K, V>> {
    let mut out: Vec<HashMap<K, V>> = (0..regions.count).map(|_| HashMap::new()).collect();
    for (k, v) in m.extract_if(|k, _| regions.of_chunk(at(k)).is_some()) {
        let r = regions.of_chunk(at(&k)).expect("filtered");
        out[r].insert(k, v);
    }
    out
}

fn fx_split<K: std::hash::Hash + Eq + Copy, V>(
    m: &mut FxHashMap<K, V>,
    regions: &Regions,
    at: impl Fn(&K) -> (i32, i32),
) -> Vec<FxHashMap<K, V>> {
    let mut out: Vec<FxHashMap<K, V>> = (0..regions.count).map(|_| Default::default()).collect();
    for (k, v) in m.extract_if(|k, _| regions.of_chunk(at(k)).is_some()) {
        let r = regions.of_chunk(at(&k)).expect("filtered");
        out[r].insert(k, v);
    }
    out
}

fn block_chunk(p: (i32, i32, i32)) -> (i32, i32) {
    (p.0 >> 4, p.2 >> 4)
}
fn array_chunk(p: &[i32; 3]) -> (i32, i32) {
    (p[0] >> 4, p[2] >> 4)
}

/// One region's part of a level, split out of the server's game ([`Game::split_level`]).
pub(super) struct RegionPart {
    /// The chunks of the part already scanned for creaking hearts.
    scanned: horizon_client::fast_hash::FxHashSet<(i32, i32)>,
    world: horizon_client::world::World,
    ticks: horizon_world::level::Ticks,
    sections: horizon_client::random_tick_sections::RandomTickSections,
    sequences: horizon_client::random_sequences::RandomSequences,
    mobs: Vec<Mob>,
    dropped: Vec<super::ItemEntity>,
    orbs: Vec<super::ExperienceOrb>,
    projectiles: Vec<horizon_client::projectiles::Projectile>,
    falling: Vec<super::FallingBlock>,
    paintings: Vec<horizon_client::entities::Painting>,
    item_frames: Vec<horizon_client::entities::ItemFrame>,
    armor_stands: Vec<horizon_client::entities::ArmorStand>,
    cushions: Vec<horizon_client::entities::Cushion>,
    others: Vec<super::commands::OtherEntity>,
    boats: Vec<super::boats::Boat>,
    carts: Vec<super::minecarts::Minecart>,
    bolts: Vec<super::lightning::Bolt>,
    crystals: Vec<super::dragon::Crystal>,
    breath_clouds: Vec<super::dragon::Breath>,
    tridents: Vec<super::trident::ThrownTrident>,
    rockets: Vec<super::fireworks::Rocket>,
    ominous_spawners: Vec<super::ominous_item_spawner::OminousItemSpawner>,
    tnts: Vec<super::tnt::PrimedTnt>,
    hoppers: HashMap<(i32, i32, i32), super::machines::HopperState>,
    jukebox_sounds: HashMap<(i32, i32, i32), u64>,
    lid_openers: HashMap<(i32, i32, i32), i32>,
    knot_ids: HashMap<[i32; 3], u32>,
    lectern_pulses: Vec<((i32, i32, i32), i32)>,
    anims: horizon_client::block_entity_anim::BlockEntityAnims,
}

/// A level split into its regions' parts, and the chunks each region holds (for the block ticks' return).
pub(super) struct LevelSplit {
    pub parts: Vec<RegionPart>,
    pub chunks: Vec<Vec<ChunkPos>>,
}

impl Game<'_> {
    /// The level in place split into `regions`' parts: each region's chunks, block entities, ticks and
    /// entities moved out of this game's level; what no region owns stays.
    pub(super) fn split_level(&mut self, regions: &Regions) -> LevelSplit {
        let n = regions.count;
        let of_chunk = |key: (i32, i32)| regions.of_chunk(key);
        let mut chunks: Vec<Vec<ChunkPos>> = vec![Vec::new(); n];
        for &(x, z) in self.world.chunks.keys() {
            if let Some(r) = of_chunk((x, z)) {
                chunks[r].push(ChunkPos::new(x, z));
            }
        }
        let seeds: Vec<i64> = (0..n)
            .map(|_| horizon_core::random::RandomSource::next_long(&mut self.world.random))
            .collect();
        let tick_seeds: Vec<i32> = seeds.iter().map(|s| *s as i32).collect();
        let mut lt = std::time::Instant::now();
        // The world's chunks, the scheduled ticks and the random-tick sections move on threads of their own at
        // once (each touches only its own part of the level), the entities are sorted meanwhile on this one.
        let (world, block_ticks, sections_of) = (
            &mut self.world,
            &mut self.block_ticks,
            &mut self.random_tick_sections,
        );
        let (chunks_ref, seeds_ref, tick_seeds_ref) = (&chunks, &seeds, &tick_seeds);
        let (worlds, ticks, sections) = std::thread::scope(|scope| {
            let worlds = scope.spawn(move || {
                let mut t = std::time::Instant::now();
                let w = world.lend(n, of_chunk, seeds_ref);
                lap(&mut t, "thread.world.lend");
                w
            });
            let ticks = scope.spawn(move || {
                let mut t = std::time::Instant::now();
                let r = block_ticks.lend(chunks_ref, tick_seeds_ref);
                lap(&mut t, "thread.ticks.lend");
                r
            });
            let mut t = std::time::Instant::now();
            let sections = sections_of.lend(n, of_chunk);
            lap(&mut t, "thread.sections.lend");
            match (worlds.join(), ticks.join()) {
                (Ok(w), Ok(t)) => (w, t, sections),
                (Err(e), _) | (_, Err(e)) => std::panic::resume_unwind(e),
            }
        });
        lap(&mut lt, "split.world+ticks+sections");
        let mut sequences = self.rand_sequences.split(n + 1);
        // The level's own sequences stay with the level (the first of the split).
        self.rand_sequences = sequences.remove(0);
        let v3 = |p: &horizon_core::geometry::Vec3| (p.x, p.z);
        let a3 = |p: &[f64; 3]| (p[0], p[2]);
        let b3 = |p: &[i32; 3]| (f64::from(p[0]), f64::from(p[2]));
        let mut mobs = split(&mut drain(&mut self.mobs), regions, |m| v3(&m.position));
        let mut dropped = split(&mut drain(&mut self.dropped), regions, |d| v3(&d.position));
        let mut orbs = split(&mut drain(&mut self.orbs), regions, |o| v3(&o.position));
        let mut projectiles = split(&mut drain(&mut self.creatures.projectiles), regions, |p| {
            v3(&p.position)
        });
        let mut falling = split(&mut drain(&mut self.falling), regions, |f| v3(&f.position));
        lap(&mut lt, "split.mobs+items+orbs+proj+falling");
        let mut paintings = split(&mut self.paintings, regions, |p| b3(&p.pos));
        let mut item_frames = split(&mut self.item_frames, regions, |f| b3(&f.pos));
        let mut armor_stands = split(&mut drain(&mut self.armor_stands), regions, |s| {
            v3(&s.position)
        });
        let mut cushions = split(&mut self.cushions, regions, |c| v3(&c.position));
        let mut others = split(&mut self.others, regions, |o| v3(&o.position));
        let mut boats = split(&mut drain(&mut self.boats), regions, |b| v3(&b.position));
        let mut carts = split(&mut drain(&mut self.carts), regions, |c| v3(&c.position));
        let mut bolts = split(&mut self.bolts, regions, |b| a3(&b.at));
        let mut crystals = split(&mut self.crystals, regions, |c| v3(&c.position));
        let mut breath_clouds = split(&mut self.breath_clouds, regions, |b| a3(&b.at));
        let mut tridents = split(&mut drain(&mut self.tridents), regions, |t| v3(&t.position));
        let mut rockets = split(&mut drain(&mut self.rockets), regions, |r| v3(&r.position));
        let mut ominous_spawners = split(&mut self.ominous_spawners, regions, |o| a3(&o.position));
        let mut tnts = split(&mut drain(&mut self.tnts), regions, |t| v3(&t.position));
        lap(&mut lt, "split.other entities");
        let mut hoppers = split_map(&mut self.hoppers, regions, |p| block_chunk(*p));
        let mut jukebox_sounds = split_map(&mut self.jukebox_sounds, regions, |p| block_chunk(*p));
        let mut lid_openers = split_map(&mut self.lid_openers, regions, |p| block_chunk(*p));
        let mut knot_ids = split_map(&mut self.knot_ids, regions, array_chunk);
        let mut scanned = self.creaking.split_scanned(n, |key| regions.of_chunk(key));
        let mut lectern_pulses: Vec<Vec<((i32, i32, i32), i32)>> =
            (0..n).map(|_| Vec::new()).collect();
        self.lectern_pulses
            .retain(|&(p, t)| match regions.of_chunk(block_chunk(p)) {
                Some(r) => {
                    lectern_pulses[r].push((p, t));
                    false
                }
                None => true,
            });
        let a = &mut self.be_anims;
        let mut chests = fx_split(&mut a.chests, regions, array_chunk);
        let mut books = fx_split(&mut a.books, regions, array_chunk);
        let mut shulkers = fx_split(&mut a.shulkers, regions, array_chunk);
        let mut bells = fx_split(&mut a.bells, regions, array_chunk);
        let mut conduits = fx_split(&mut a.conduits, regions, array_chunk);
        let mut skulls = fx_split(&mut a.skulls, regions, array_chunk);
        let mut pots = fx_split(&mut a.pots, regions, array_chunk);
        let mut gateway_servers = fx_split(&mut a.gateway_servers, regions, array_chunk);
        let mut conduit_servers = fx_split(&mut a.conduit_servers, regions, array_chunk);
        lap(&mut lt, "split.maps+anims");
        let mut parts = Vec::with_capacity(n);
        for (r, ((world, ticks), sections)) in
            worlds.into_iter().zip(ticks).zip(sections).enumerate()
        {
            parts.push(RegionPart {
                world,
                ticks,
                sections,
                sequences: sequences.remove(0),
                mobs: std::mem::take(&mut mobs[r]),
                dropped: std::mem::take(&mut dropped[r]),
                orbs: std::mem::take(&mut orbs[r]),
                projectiles: std::mem::take(&mut projectiles[r]),
                falling: std::mem::take(&mut falling[r]),
                paintings: std::mem::take(&mut paintings[r]),
                item_frames: std::mem::take(&mut item_frames[r]),
                armor_stands: std::mem::take(&mut armor_stands[r]),
                cushions: std::mem::take(&mut cushions[r]),
                others: std::mem::take(&mut others[r]),
                boats: std::mem::take(&mut boats[r]),
                carts: std::mem::take(&mut carts[r]),
                bolts: std::mem::take(&mut bolts[r]),
                crystals: std::mem::take(&mut crystals[r]),
                breath_clouds: std::mem::take(&mut breath_clouds[r]),
                tridents: std::mem::take(&mut tridents[r]),
                rockets: std::mem::take(&mut rockets[r]),
                ominous_spawners: std::mem::take(&mut ominous_spawners[r]),
                tnts: std::mem::take(&mut tnts[r]),
                hoppers: std::mem::take(&mut hoppers[r]),
                jukebox_sounds: std::mem::take(&mut jukebox_sounds[r]),
                lid_openers: std::mem::take(&mut lid_openers[r]),
                knot_ids: std::mem::take(&mut knot_ids[r]),
                lectern_pulses: std::mem::take(&mut lectern_pulses[r]),
                scanned: std::mem::take(&mut scanned[r]),
                anims: horizon_client::block_entity_anim::BlockEntityAnims {
                    chests: std::mem::take(&mut chests[r]),
                    books: std::mem::take(&mut books[r]),
                    shulkers: std::mem::take(&mut shulkers[r]),
                    bells: std::mem::take(&mut bells[r]),
                    conduits: std::mem::take(&mut conduits[r]),
                    skulls: std::mem::take(&mut skulls[r]),
                    pots: std::mem::take(&mut pots[r]),
                    gateway_servers: std::mem::take(&mut gateway_servers[r]),
                    conduit_servers: std::mem::take(&mut conduit_servers[r]),
                    ..Default::default()
                },
            });
        }
        lap(&mut lt, "split.parts");
        LevelSplit { parts, chunks }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_far_apart_are_separate_regions() {
        let players = [(0, 0), (400, 0)];
        let r = Regions::new(players, &players, 0);
        assert_eq!(r.count, 2);
        assert_ne!(r.of_chunk((0, 0)), r.of_chunk((400, 0)));
        assert_eq!(r.of_chunk((200, 0)), None);
    }

    #[test]
    fn ticking_sections_two_apart_merge_three_apart_do_not() {
        // Sections are 4 chunks wide: chunk 0 is section 0, chunk 8 is section 2, chunk 12 is section 3.
        let two = Regions::new([(0, 0), (8, 0)], &[(0, 0), (8, 0)], 0);
        assert_eq!(two.count, 1);
        let three = Regions::new([(0, 0), (12, 0)], &[(0, 0), (12, 0)], 0);
        assert_eq!(three.count, 2);
    }

    #[test]
    fn a_ticking_section_owns_the_ring_around_it() {
        let r = Regions::new([(0, 0)], &[(0, 0)], 0);
        // Section 0 and its eight neighbours: chunks -4..=7 on each side.
        assert_eq!(r.of_chunk((-4, 7)), Some(0));
        assert_eq!(r.of_chunk((7, -4)), Some(0));
        assert_eq!(r.of_chunk((8, 0)), None);
        assert_eq!(r.of_chunk((-5, 0)), None);
    }

    #[test]
    fn a_region_without_a_player_is_dropped() {
        let r = Regions::new([(0, 0), (400, 0)], &[(0, 0)], 0);
        assert_eq!(r.count, 1);
        assert_eq!(r.of_chunk((400, 0)), None);
    }

    #[test]
    fn views_that_touch_merge_and_numbering_is_stable() {
        // Two players 20 chunks apart with a view of 8 chunks: their views reach 4 sections each way and touch.
        let players = [(0, 0), (20, 0)];
        let r = Regions::new([], &players, 8);
        assert_eq!(r.count, 1);
        let again = Regions::new([], &players, 8);
        assert_eq!(r.firsts, again.firsts);
        assert_eq!(r.of_block(-3.5, 0.0), r.of_block(0.0, 0.0));
    }
}
