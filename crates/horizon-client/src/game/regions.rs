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
use horizon_core::fast_hash::FxHashMap;

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
