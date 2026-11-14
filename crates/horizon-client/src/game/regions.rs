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
use super::region_profile::Step;
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
    /// While one resident region is split out again alone: the only region that owns anything (`usize::MAX`:
    /// all of them). Read by the server's thread only.
    only: std::sync::atomic::AtomicUsize,
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
            only: std::sync::atomic::AtomicUsize::new(usize::MAX),
        }
    }

    /// From now on only region `k` (or all of them, `None`) owns chunks: a split of the level then moves out
    /// that region's part alone.
    fn restrict(&self, k: Option<usize>) {
        self.only.store(k.unwrap_or(usize::MAX), Ordering::Relaxed);
    }

    /// The region that owns chunk `(x, z)`.
    #[inline]
    pub(super) fn of_chunk(&self, (x, z): (i32, i32)) -> Option<usize> {
        let (sx, sz) = (x >> SECTION_SHIFT, z >> SECTION_SHIFT);
        let r = match &self.grid {
            Some(g) => g.get(sx, sz),
            None => self.owned.get(&(sx, sz)).copied(),
        };
        match self.only.load(Ordering::Relaxed) {
            usize::MAX => r,
            k => r.filter(|&r| r == k),
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

/// The most block changes one region lights in its tick; the rest wait for the next.
const REGION_RELIGHT_PER_TICK: usize = 1024;

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
    /// Each level's resident regions: their games keep their parts of the level and their players.
    pub resident: std::collections::HashMap<Dim, ResidentSet>,
    /// The regions just came home for nothing the server's thread has to do (their age, a player near the edge
    /// of its region): the level is split again in the same tick, instead of ticking whole on this thread.
    resplit: bool,
    /// Where the server thread's tick time goes while regions are resident.
    pub(super) prof: super::region_profile::TickProfile,
}

/// A level's regions that stay on their threads between ticks.
pub(super) struct ResidentSet {
    games: Vec<Slot>,
    /// Each region's chunks when it was lent.
    chunks: Vec<Vec<ChunkPos>>,
    /// All of `chunks`, for the streamer: lent columns are loaded, only not in the server's world.
    lent: Arc<horizon_client::fast_hash::FxHashSet<(i32, i32)>>,
    /// The layout they were lent in: what their players' views must stay inside.
    layout: Arc<Regions>,
    /// Ticks since they were all lent: the set comes home whole and is split anew every few periods.
    age: u32,
}

impl ResidentSet {
    /// The chunks of the regions that are out on their threads, for the streamer.
    fn rebuild_lent(&mut self) {
        self.lent = Arc::new(
            self.chunks
                .iter()
                .zip(&self.games)
                .filter(|(_, s)| !s.landed)
                .flat_map(|(c, _)| c.iter().map(|c| (c.x, c.z)))
                .collect(),
        );
    }
}

/// One resident region: its game when it is home, or the channel its thread sends it back on while it ticks.
struct Slot {
    game: Option<Box<Game<'static>>>,
    running: Option<
        std::sync::mpsc::Receiver<(Box<Game<'static>>, std::result::Result<(), String>, f32)>,
    >,
    /// Where its players stood when it was last home: the areas the server keeps loaded for them.
    places: Vec<(i32, i32)>,
    /// When its tick started, and when the watchdog last told of it.
    since: std::time::Instant,
    told: Option<std::time::Instant>,
    /// How long its last tick took (ms) and how many chunks it held then.
    took: f32,
    chunks: usize,
    /// Ticks since this region was lent: it comes home alone at `resident_period`, each at its own phase.
    age: u32,
    /// Its part of the level and its players are home, ticking with the server's thread this tick; it is lent
    /// again at the start of the next.
    landed: bool,
    /// Lent again: its game takes the tick's settings, as one just lent.
    fresh: bool,
    /// The connections of its players.
    conns: Vec<u32>,
    /// Its players' sleep counts as it last reported them.
    sleep: (i64, i64, i64),
}

impl Slot {
    fn new(game: Box<Game<'static>>) -> Self {
        Self {
            game: Some(game),
            running: None,
            places: Vec::new(),
            since: std::time::Instant::now(),
            told: None,
            took: 0.0,
            chunks: 0,
            age: 0,
            landed: false,
            fresh: false,
            conns: Vec::new(),
            sleep: (0, 0, 0),
        }
    }
    /// Takes the game in if its thread finished (waiting until `until`, or for good when None); false while it
    /// still runs.
    fn home(&mut self, until: Option<std::time::Instant>) -> std::result::Result<bool, String> {
        let Some(rx) = self.running.as_ref() else {
            return Ok(true);
        };
        let got = match until {
            None => rx
                .recv()
                .map_err(|_| std::sync::mpsc::TryRecvError::Disconnected),
            Some(t) => {
                match rx.recv_timeout(t.saturating_duration_since(std::time::Instant::now())) {
                    Ok(b) => Ok(b),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        Err(std::sync::mpsc::TryRecvError::Empty)
                    }
                    Err(_) => Err(std::sync::mpsc::TryRecvError::Disconnected),
                }
            }
        };
        match got {
            Ok((game, result, took)) => {
                self.running = None;
                self.took = took;
                self.chunks = game.world.chunks.len();
                self.places = game
                    .lan
                    .as_ref()
                    .map(|h| h.hub.guests.iter().map(|g| g.chunk()).collect())
                    .unwrap_or_default();
                self.game = Some(game);
                result.map(|()| true)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(false),
            Err(_) => {
                self.running = None;
                Err("a region thread stopped".to_owned())
            }
        }
    }
}

/// How long a tick waits for its regions (`HORIZON_REGION_DEADLINE_MS`, 50): one that runs longer keeps going
/// on its own thread and its players are ticked again once it is home, so a slow region does not hold the others.
fn region_deadline() -> std::time::Duration {
    static MS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    std::time::Duration::from_millis(*MS.get_or_init(|| {
        std::env::var("HORIZON_REGION_DEADLINE_MS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(50)
    }))
}

/// Ticks between the resident regions' returns (`HORIZON_RESIDENT_TICKS`, 20 by default).
fn resident_period() -> u32 {
    static N: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("HORIZON_RESIDENT_TICKS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(20)
            .max(1)
    })
}

/// Ticks of `resident_period` between the resident set coming home whole to be split anew.
const RESIDENT_REFRESH: u32 = 10;

/// Which resident regions have to come home: all of them, or those named (with the reason).
enum Land {
    All(&'static str),
    Slots(Vec<(usize, &'static str)>),
}

/// Whether region game `w` has something for the server's thread, so its region must come back: commands, menus
/// of native mods, a player dead or leaving.
fn wants_land(w: &Game<'_>) -> bool {
    if !w.level_teleports.is_empty() || !w.server_trips.is_empty() {
        return true;
    }
    let Some(h) = w.lan.as_ref() else {
        return true;
    };
    !h.commands.is_empty()
        || !h.quiet.is_empty()
        || !h.hub.native_menu_events.is_empty()
        || h.hub
            .guests
            .iter()
            .any(|g| g.dead || g.health <= 0.0 || !g.left_reason.is_empty())
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
        self.copy_clock_from(from);
        self.tickets = from.tickets.clone();
    }

    /// [`Self::copy_tick_settings_from`] but the ticket levels, which a resident region keeps for its own players.
    pub(super) fn copy_clock_from(&mut self, from: &Game<'_>) {
        self.ui.rules = from.ui.rules.clone();
        self.ui.difficulty = from.ui.difficulty;
        self.clock = from.clock.clone();
        self.ticks = from.ticks;
        self.border = from.border.clone();
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
        let world = &mut self.world;
        std::thread::scope(|scope| {
            let back = scope.spawn(move || world.take_back(region_world));
            self.block_ticks.take_back(region_ticks, chunks);
            self.random_tick_sections.take_back(region_sections);
            if let Err(e) = back.join() {
                std::panic::resume_unwind(e);
            }
        });
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
        self.level_teleports.append(&mut w.level_teleports);
        self.server_trips.append(&mut w.server_trips);
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
    /// The world phases of this tick by regions, on threads of their own. With players in two or more separate
    /// areas of the level, it is split into regions: each region's chunks, entities and players move into a
    /// region game that keeps them (a resident region) and ticks them on a thread of its own, its players' packets
    /// and network turn included. They come back every `resident_period` ticks, or when something needs the
    /// server's thread (a chat line or command, a join or leave, a death, a player leaving its region).
    /// Returns `false`: what no region owns, and the whole level in a tick the regions are back, ticks in the
    /// serial phases on this thread.
    pub(super) fn tick_world_regions(&mut self) -> Result<bool> {
        // The End is one region while its dragon fight runs: the fight belongs to the server's thread.
        let fighting = self.dim == Dim::End && !(self.fight.killed || self.fight.previously_killed);
        if !region_threads_on() || self.regions.worker || fighting {
            return Ok(false);
        }
        let dim = self.dim;
        let mut entered = std::time::Instant::now();
        if let Some(set) = self.regions.resident.remove(&dim) {
            self.regions.prof.note_resident();
            if let Some(set) = self.resident_tick(set, false)? {
                self.regions.resident.insert(dim, set);
                return Ok(false);
            }
            if !std::mem::take(&mut self.regions.resplit) {
                return Ok(false);
            }
            entered = std::time::Instant::now();
        }
        let Some(host) = self.lan.as_ref() else {
            return Ok(false);
        };
        let view = host.hub.config.view_distance.clamp(2, 32);
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
        // One chunk wider than a player's view, as `residents_must_land` asks for: a player standing still
        // never counts as having left its region.
        let regions = Regions::new(ticking, &players, view + 1);
        if regions.count < 2 {
            return Ok(false);
        }
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
        self.regions.idle.insert(dim, extra);
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
        let mut host = self.lan.take().expect("checked above");
        let mut split_guests = host
            .hub
            .split_guests(regions.count, |conn| region_of_conn.get(&conn).copied());
        let mut trackers = host
            .hub
            .lend_trackers(regions.count, |chunk| regions.of_chunk(chunk))
            .into_iter();
        let split = self.split_level(&regions);
        let regions_count = regions.count;
        let mut set = ResidentSet {
            games: Vec::with_capacity(regions.count),
            chunks: Vec::with_capacity(regions.count),
            lent: Default::default(),
            layout: Arc::new(regions),
            age: 0,
        };
        for ((part, mut w), (chunks, guests)) in split.parts.into_iter().zip(games).zip(
            split
                .chunks
                .into_iter()
                .zip(split_guests.iter_mut().map(std::mem::take)),
        ) {
            let mob_ids: Vec<u32> = part.mobs.iter().map(|m| m.id).collect();
            self.lend_region(&mut w, part);
            // The level's own settings: a game made for another dimension earlier is this one's now.
            w.copy_world_settings_from(self);
            w.refill_entity_ids(&pool);
            let conns: Vec<u32> = guests.iter().map(|g| g.conn).collect();
            host.hub.away.extend(conns.iter().copied());
            let mut shell = w.lan.take().unwrap_or_else(|| host.region_shell());
            host.lend_region(&mut shell, &conns, &mob_ids, dim, guests);
            if let Some(tracker) = trackers.next() {
                shell.hub.set_tracker(tracker);
            }
            w.lan = Some(shell);
            let mut slot = Slot::new(w);
            slot.conns = conns;
            slot.fresh = true;
            // Each region comes home at its own tick of the period.
            slot.age = (set.games.len() as u32 * resident_period()) / regions_count as u32;
            set.games.push(slot);
            set.chunks.push(chunks);
        }
        set.lent = Arc::new(set.chunks.iter().flatten().map(|c| (c.x, c.z)).collect());
        self.lan = Some(host);
        self.regions.prof.note_resident();
        self.regions.prof.add(Step::Split, entered);
        if let Some(set) = self.resident_tick(set, true)? {
            self.regions.resident.insert(dim, set);
        }
        Ok(false)
    }

    /// One tick of the resident regions `set`, on threads of their own; `None` once they are back (the tick ran
    /// by landing them, or one of them failed).
    fn resident_tick(&mut self, mut set: ResidentSet, first: bool) -> Result<Option<ResidentSet>> {
        if !first {
            // The regions that came home alone last tick are lent again.
            let relending = std::time::Instant::now();
            let mut relent = false;
            for k in 0..set.games.len() {
                if set.games[k].landed {
                    self.relend_slot(&mut set, k);
                    relent = true;
                }
            }
            if relent {
                self.regions.prof.add(Step::Split, relending);
            }
            match self.residents_must_land(&set) {
                Some(Land::All(why)) => {
                    self.regions.prof.landed_because(why);
                    self.regions.resplit = matches!(why, "age" | "left section");
                    let landing = std::time::Instant::now();
                    horizon_client::spans::time("regions_land", || self.land_resident_set(set));
                    self.regions.prof.add(Step::Land, landing);
                    return Ok(None);
                }
                Some(Land::Slots(list)) => {
                    let landing = std::time::Instant::now();
                    for (k, why) in list {
                        self.regions.prof.landed_because(why);
                        self.land_slot(&mut set, k);
                    }
                    self.regions.prof.add(Step::Land, landing);
                }
                None => {}
            }
        }
        self.streaming.lent = Some(Arc::clone(&set.lent));
        let pool = Arc::clone(
            self.regions
                .entity_pool
                .get_or_insert_with(|| Arc::new(AtomicU32::new(self.next_entity + 1))),
        );
        let started = std::time::Instant::now();
        // A region still ticking from an earlier tick is left to finish; the others start.
        let mut failed: Option<String> = None;
        let mut launched = Vec::new();
        for (k, slot) in set.games.iter_mut().enumerate() {
            match slot.home(Some(started)) {
                Err(e) => {
                    failed.get_or_insert(e);
                    continue;
                }
                Ok(false) => {
                    // The region watchdog: a region tick past 5 s is told of every 5 s.
                    if slot.since.elapsed() >= std::time::Duration::from_secs(5)
                        && slot
                            .told
                            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(5))
                    {
                        slot.told = Some(started);
                        let at = slot.places.first().copied().unwrap_or_default();
                        log::warn!(
                            "region {k} near chunk {},{} has ticked for {:.1} s ({} players)",
                            at.0,
                            at.1,
                            slot.since.elapsed().as_secs_f32(),
                            slot.places.len()
                        );
                    }
                    continue;
                }
                Ok(true) => {}
            }
            if slot.landed {
                continue;
            }
            let Some(mut w) = slot.game.take() else {
                continue;
            };
            let copying = std::time::Instant::now();
            if first || std::mem::take(&mut slot.fresh) {
                w.copy_tick_settings_from(self);
            } else {
                w.copy_clock_from(self);
            }
            w.refill_entity_ids(&pool);
            w.streaming.sky_light = self.streaming.sky_light;
            w.world.game_time = self.world.game_time;
            w.world.day_time = self.world.day_time;
            w.world.weather = self.world.weather;
            if let (Some(shell), Some(host)) = (w.lan.as_mut(), self.lan.as_mut()) {
                shell.hub.copy_region_settings(&host.hub);
                host.hub.give_away(&mut shell.hub);
                shell.hub.wake_request |= host.hub.wake_request;
            }
            self.regions.prof.add(Step::Copy, copying);
            let spawning = std::time::Instant::now();
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            slot.running = Some(rx);
            slot.since = started;
            slot.told = None;
            let spawned = super::region_pool::run(move || {
                let began = std::time::Instant::now();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    w.tick_region_phases().map_err(|e| e.to_string())
                }))
                .unwrap_or_else(|_| Err("a region thread panicked".to_owned()));
                let _ = tx.send((w, result, began.elapsed().as_secs_f32() * 1000.0));
            });
            self.regions.prof.add(Step::Spawn, spawning);
            if !spawned {
                slot.running = None;
                failed.get_or_insert("a region thread could not start".to_owned());
            } else {
                launched.push(k);
            }
        }
        set.age += 1;
        for slot in set.games.iter_mut().filter(|s| !s.landed) {
            slot.age += 1;
        }
        let waiting = std::time::Instant::now();
        // Waited for until the deadline: what runs past it carries on by itself.
        let deadline = started + region_deadline();
        horizon_client::spans::time("regions_wait", || {
            for k in launched {
                if let Err(e) = set.games[k].home(Some(deadline)) {
                    failed.get_or_insert(e);
                }
            }
        });
        self.regions.prof.add(Step::Wait, waiting);
        super::health::record_regions(
            self.dim.id(),
            set.games
                .iter()
                .map(|s| super::health::RegionStat {
                    level: self.dim.id().to_owned(),
                    at: s.places.first().map_or((0, 0), |c| (c.0 << 4, c.1 << 4)),
                    chunks: s.chunks,
                    players: s.places.len(),
                    ms: if s.running.is_some() {
                        s.since.elapsed().as_secs_f32() * 1000.0
                    } else {
                        s.took
                    },
                    late: s.running.is_some(),
                })
                .collect(),
        );
        // The sleep counts the regions reported, for the server's night skip; the wake-up went out with them.
        for slot in set.games.iter_mut().filter(|s| !s.landed) {
            if let Some(w) = slot.game.as_ref()
                && let Some(shell) = w.lan.as_ref()
            {
                slot.sleep = shell.hub.away_sleep;
            }
        }
        let (mut a, mut s, mut d) = (0, 0, 0);
        for slot in set.games.iter().filter(|s| !s.landed) {
            a += slot.sleep.0;
            s += slot.sleep.1;
            d += slot.sleep.2;
        }
        if let Some(host) = self.lan.as_mut() {
            host.hub.away_sleep = (a, s, d);
            host.hub.wake_request = false;
        }
        // Where its players stand: the areas the server keeps loaded for them.
        if let Some(host) = self.lan.as_mut() {
            host.hub.away_places = set
                .games
                .iter()
                .filter(|s| !s.landed)
                .flat_map(|s| s.places.iter().copied())
                .collect();
        }
        if let Some(e) = failed {
            let landing = std::time::Instant::now();
            self.land_resident_set(set);
            self.regions.prof.add(Step::Land, landing);
            return Err(e.into());
        }
        Ok(Some(set))
    }

    /// Whether resident regions have to come back before the next tick: all of them, or only some.
    fn residents_must_land(&self, set: &ResidentSet) -> Option<Land> {
        let Some(host) = self.lan.as_ref() else {
            return Some(Land::All("no host"));
        };
        // The whole set is split anew now and then: the layout follows the players and the ticking chunks.
        if set.age >= resident_period() * RESIDENT_REFRESH {
            return Some(Land::All("age"));
        }
        let slot_of = |conn: u32| set.games.iter().position(|s| s.conns.contains(&conn));
        let mut slots: Vec<(usize, &'static str)> = Vec::new();
        let mut add = |k: usize, why: &'static str| {
            if !slots.iter().any(|s| s.0 == k) {
                slots.push((k, why));
            }
        };
        match host.hub.away_server_conns() {
            None => return Some(Land::All("packet")),
            Some(conns) => {
                for c in conns {
                    match slot_of(c) {
                        Some(k) => add(k, "packet"),
                        None => return Some(Land::All("packet")),
                    }
                }
            }
        }
        // A player who is in the server's hands and belongs to no region.
        if host
            .hub
            .guests
            .iter()
            .any(|g| host.hub.here(g) && slot_of(g.conn).is_none())
        {
            return Some(Land::All("guest here"));
        }
        if !self.level_teleports.is_empty() {
            return Some(Land::All("teleport"));
        }
        let reach = host.hub.config.view_distance.clamp(2, 32) + 1;
        for (k, s) in set.games.iter().enumerate() {
            if s.landed {
                continue;
            }
            let Some(w) = s.game.as_ref() else {
                continue;
            };
            let left = w.lan.as_ref().is_none_or(|h| {
                h.hub.guests.iter().any(|g| {
                    let (cx, cz) = g.chunk();
                    ((cx - reach) >> SECTION_SHIFT..=(cx + reach) >> SECTION_SHIFT).any(|sx| {
                        ((cz - reach) >> SECTION_SHIFT..=(cz + reach) >> SECTION_SHIFT)
                            .any(|sz| set.layout.owned.get(&(sx, sz)) != Some(&k))
                    })
                })
            });
            if left {
                return Some(Land::All("left section"));
            }
            if wants_land(w) {
                add(k, "region wants");
            } else if s.age >= resident_period() {
                add(k, "region age");
            }
        }
        (!slots.is_empty()).then_some(Land::Slots(slots))
    }

    /// Region `k` alone back in the server's level: it ticks there with the server's thread this tick, and is
    /// lent again at the start of the next ([`Game::relend_slot`]). The other regions stay out.
    fn land_slot(&mut self, set: &mut ResidentSet, k: usize) {
        let slot = &mut set.games[k];
        let _ = slot.home(None);
        let Some(mut w) = slot.game.take() else {
            return;
        };
        let mut host = self.lan.take();
        self.take_region_back(&mut w, &set.chunks[k]);
        if let (Some(host), Some(shell)) = (host.as_mut(), w.lan.as_mut()) {
            host.hub.park_tracker(&mut shell.hub);
            host.take_region_back(shell);
        }
        if let Some(host) = host.as_mut() {
            host.hub.release_away_conns(&slot.conns);
            host.hub.restore_guest_order();
        }
        slot.game = Some(w);
        slot.landed = true;
        self.lan = host;
        set.rebuild_lent();
        self.streaming.lent = Some(Arc::clone(&set.lent));
    }

    /// Region `k`, landed alone, split out of the level again in the layout the set has: its chunks (the ones
    /// loaded since too), its entities and its players, moved back into its game.
    fn relend_slot(&mut self, set: &mut ResidentSet, k: usize) {
        let dim = self.dim;
        let Some(mut w) = set.games[k].game.take() else {
            return;
        };
        let Some(mut host) = self.lan.take() else {
            set.games[k].game = Some(w);
            return;
        };
        let pool = Arc::clone(
            self.regions
                .entity_pool
                .get_or_insert_with(|| Arc::new(AtomicU32::new(self.next_entity + 1))),
        );
        let layout = Arc::clone(&set.layout);
        layout.restrict(Some(k));
        let in_region: HashMap<u32, usize> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g))
            .filter_map(|g| Some((g.conn, layout.of_chunk(g.chunk())?)))
            .collect();
        let guests = host
            .hub
            .split_guests(layout.count, |conn| in_region.get(&conn).copied())
            .into_iter()
            .nth(k)
            .unwrap_or_default();
        let tracker = host
            .hub
            .lend_trackers(layout.count, |chunk| layout.of_chunk(chunk))
            .into_iter()
            .nth(k);
        let split = self.split_level(&layout);
        layout.restrict(None);
        let part = split
            .parts
            .into_iter()
            .nth(k)
            .expect("a part for the region");
        let chunks = split.chunks.into_iter().nth(k).unwrap_or_default();
        let mob_ids: Vec<u32> = part.mobs.iter().map(|m| m.id).collect();
        self.lend_region(&mut w, part);
        w.copy_world_settings_from(self);
        w.refill_entity_ids(&pool);
        let conns: Vec<u32> = guests.iter().map(|g| g.conn).collect();
        host.hub.away.extend(conns.iter().copied());
        let mut shell = w.lan.take().unwrap_or_else(|| host.region_shell());
        host.lend_region(&mut shell, &conns, &mob_ids, dim, guests);
        if let Some(tracker) = tracker {
            shell.hub.set_tracker(tracker);
        }
        w.lan = Some(shell);
        self.lan = Some(host);
        let slot = &mut set.games[k];
        slot.game = Some(w);
        slot.landed = false;
        slot.fresh = true;
        slot.conns = conns;
        slot.age = 0;
        set.chunks[k] = chunks;
        set.rebuild_lent();
        self.streaming.lent = Some(Arc::clone(&set.lent));
    }

    /// The resident regions of the level in place back: their parts of the level and their players.
    fn land_resident_set(&mut self, set: ResidentSet) {
        let dim = self.dim;
        self.streaming.lent = None;
        let mut host = self.lan.take();
        let mut back = Vec::with_capacity(set.games.len());
        for (mut slot, chunks) in set.games.into_iter().zip(set.chunks) {
            // One still ticking is waited for.
            let _ = slot.home(None);
            let Some(mut w) = slot.game.take() else {
                continue;
            };
            if slot.landed {
                // Home already, alone.
                back.push(w);
                continue;
            }
            self.take_region_back(&mut w, &chunks);
            if let (Some(host), Some(shell)) = (host.as_mut(), w.lan.as_mut()) {
                host.hub.park_tracker(&mut shell.hub);
                host.take_region_back(shell);
            }
            back.push(w);
        }
        if let Some(host) = host.as_mut() {
            host.hub.release_away();
            host.hub.away_places.clear();
            host.hub.restore_guest_order();
        }
        self.lan = host;
        self.regions.idle.entry(dim).or_default().extend(back);
    }

    /// Every resident region of every level back (before a save or a stop).
    pub(super) fn land_residents(&mut self) {
        let home = self.dim;
        let dims: Vec<Dim> = self.regions.resident.keys().copied().collect();
        for dim in dims {
            if dim != self.dim && self.enter_level(None, dim).is_err() {
                continue;
            }
            if let Some(set) = self.regions.resident.remove(&dim) {
                self.land_resident_set(set);
            }
        }
        if self.dim != home {
            let _ = self.enter_level(None, home);
        }
    }

    /// One region game's phases of the tick: the level's entities and blocks in its chunks, around its players.
    pub(super) fn tick_region_phases(&mut self) -> Result<()> {
        self.level_pass = super::levels::Pass {
            first: false,
            last: false,
            running: Some(true),
            packets_drained: false,
        };
        self.update_chunk_tickets();
        // Its players' packets first, before its block ticks, as vanilla.
        self.tick_lan_packets();
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
        // Its players' turn: their own ticks, entities, chunks and block changes, from its own world.
        self.tick_lan(false);
        // Sculk in its own chunks: the vibrations its players and mobs make, and the sensors, shriekers and
        // catalysts that answer them.
        self.tick_sculk_entities();
        self.tick_sculk_blocks()?;
        // Sleep: the night is skipped by the server, which sums what the regions report; a skipped night wakes
        // this region's players too.
        if self
            .lan
            .as_mut()
            .is_some_and(|h| std::mem::take(&mut h.hub.wake_request))
        {
            self.wake_guests();
        }
        if let Some(h) = self.lan.as_mut() {
            h.hub.away_sleep = (0, 0, 0);
        }
        let counts = self.sleep_counts();
        if let Some(h) = self.lan.as_mut() {
            h.hub.away_sleep = counts;
        }
        // The light its block changes made, here and not on the server thread: coalesced, and a bounded
        // amount per tick, the rest carried to the next tick.
        let sky_light = self.streaming.sky_light;
        self.streaming
            .relight_region_edits(&mut self.world, sky_light, REGION_RELIGHT_PER_TICK);
        Ok(())
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
        // The world's chunks, the scheduled ticks and the random-tick sections move on threads of their own at
        // once (each touches only its own part of the level), the entities are sorted meanwhile on this one.
        let (world, block_ticks, sections_of) = (
            &mut self.world,
            &mut self.block_ticks,
            &mut self.random_tick_sections,
        );
        let (chunks_ref, seeds_ref, tick_seeds_ref) = (&chunks, &seeds, &tick_seeds);
        let (worlds, ticks, sections) = std::thread::scope(|scope| {
            let worlds = scope.spawn(move || world.lend(n, of_chunk, seeds_ref));
            let ticks = scope.spawn(move || block_ticks.lend(chunks_ref, tick_seeds_ref));
            let sections = sections_of.lend(n, of_chunk);
            match (worlds.join(), ticks.join()) {
                (Ok(w), Ok(t)) => (w, t, sections),
                (Err(e), _) | (_, Err(e)) => std::panic::resume_unwind(e),
            }
        });
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
