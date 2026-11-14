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
use super::{Dim, Game, Resources};
use horizon_client::Result;
use horizon_core::fast_hash::FxHashMap;
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
        Self {
            owned,
            count,
            firsts,
        }
    }

    /// The region that owns chunk `(x, z)`.
    pub(super) fn of_chunk(&self, (x, z): (i32, i32)) -> Option<usize> {
        self.owned
            .get(&(x >> SECTION_SHIFT, z >> SECTION_SHIFT))
            .copied()
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
}

impl Game<'_> {
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
