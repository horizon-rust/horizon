//! Solid draws grouped by vertex arena page within distance bands.
//!
//! Sections sit in arena pages of 256 MiB, so render distance 32 spreads
//! them over half a dozen pages, and a list sorted only by distance switches
//! page every few sections: about a thousand indirect multi-draws of two or
//! three entries a frame. Mesa's Intel driver (and wgpu-core, for each
//! vertex buffer change) pays per call, and below four entries Mesa unrolls
//! the call on the CPU entry by entry. Grouping the sections of one band of
//! distance by page keeps the list front to back to within the band (what
//! the depth test needs to reject hidden fragments early) with one call per
//! page and band.
//!
//! Past `NEAR` the bands widen with distance, half an octave each: distant
//! terrain reaches thousands of blocks, and 64-block bands there split its
//! tiles into thousands of calls a frame (wgpu records each one, twice in VR)
//! for fragments the near terrain mostly hides already.
//!
//! Translucent draws go back to front exactly within `NEAR`; past it, each
//! band's draws go by page (`back_to_front`), back to front within one. Far
//! water rarely lies behind other translucent faces, and without this the
//! pages alternating along the list made about three thousand calls.
//!
//! For solid draws the picture does not change: solid draws write depth and are tested
//! against it, so their order only decides which of two fragments at the
//! same depth stays, and sections do not share faces.

/// The width of a band, in blocks, up to `NEAR`.
const BAND: f64 = 64.0;
const NEAR: f64 = 256.0;

/// The band of a draw `blocks` from the camera.
fn band(blocks: f64) -> u64 {
    if blocks < NEAR {
        (blocks / BAND) as u64
    } else {
        (NEAR / BAND) as u64 + ((blocks / NEAR).log2() * 2.0) as u64
    }
}

/// The sort key of a translucent draw `distance` from the camera (squared, as bits; `index`
/// breaks ties) in `pages`: farthest first, exactly within `NEAR`, by band and page past it.
pub(super) fn back_to_front(
    distance: u64,
    index: usize,
    pages: (u32, Option<u32>),
) -> impl Ord + use<> {
    use std::cmp::Reverse;
    let blocks = f64::from_bits(distance).sqrt();
    let group = if blocks < NEAR {
        (0u8, distance)
    } else {
        (1, band(blocks))
    };
    (Reverse(group), pages, Reverse(distance), Reverse(index))
}

/// One section's draws: `start..start + len` of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Run {
    key: u64,
    pub start: u32,
    pub len: u32,
    /// The caller's data, carried along (occlusion culling's test inputs).
    pub tag: [u32; 3],
}

#[derive(Default)]
pub(super) struct PageRuns {
    runs: Vec<Run>,
    scratch: Vec<([u32; 5], (u32, Option<u32>))>,
}

impl PageRuns {
    pub fn clear(&mut self) {
        self.runs.clear();
    }

    /// The draws `start..start + len` of a section `distance` from the
    /// camera (squared, as bits) in arena `page`; runs come nearest first.
    pub fn push(&mut self, distance: u64, page: u32, start: usize, len: usize, tag: [u32; 3]) {
        if len > 0 {
            let band = band(f64::from_bits(distance).sqrt());
            self.runs.push(Run {
                key: band << 32 | u64::from(page),
                start: start as u32,
                len: len as u32,
                tag,
            });
        }
    }

    /// Reorders `draws[base..]` and their `pages`, which the pushed runs
    /// cover in order, so that each band's runs in one page are adjacent
    /// (in distance order); returns the runs at their new places.
    pub fn regroup(
        &mut self,
        draws: &mut [[u32; 5]],
        pages: &mut [(u32, Option<u32>)],
        base: usize,
    ) -> &[Run] {
        let page = |r: &Run| r.key as u32;
        if self.runs.windows(2).all(|w| page(&w[0]) == page(&w[1])) {
            return &self.runs;
        }
        // Stable: within a band and page the runs stay nearest first.
        self.runs.sort_by_key(|r| r.key);
        self.scratch.clear();
        for r in &mut self.runs {
            let (s, e) = (r.start as usize, (r.start + r.len) as usize);
            r.start = (base + self.scratch.len()) as u32;
            self.scratch
                .extend(draws[s..e].iter().copied().zip(pages[s..e].iter().copied()));
        }
        for (i, &(draw, p)) in self.scratch.iter().enumerate() {
            draws[base + i] = draw;
            pages[base + i] = p;
        }
        &self.runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(blocks: f64) -> u64 {
        (blocks * blocks).to_bits()
    }

    /// Runs of (distance in blocks, page, draws), as the list builder pushes them.
    fn list(runs: &[(f64, u32, u32)]) -> (PageRuns, Vec<[u32; 5]>, Vec<(u32, Option<u32>)>) {
        let mut grouped = PageRuns::default();
        let (mut draws, mut pages) = (Vec::new(), Vec::new());
        for (n, &(distance, page, count)) in runs.iter().enumerate() {
            let start = draws.len();
            for k in 0..count {
                draws.push([n as u32, k, 0, 0, 0]);
                pages.push((page, None));
            }
            grouped.push(
                bits(distance),
                page,
                start,
                count as usize,
                [n as u32, 0, 0],
            );
        }
        (grouped, draws, pages)
    }

    #[test]
    fn a_band_draws_each_page_once_keeping_the_order_inside_it() {
        let (mut grouped, mut draws, mut pages) = list(&[
            (10.0, 1, 2),
            (20.0, 0, 1),
            (30.0, 1, 3),
            (40.0, 0, 2),
            (70.0, 1, 1),
        ]);
        let runs: Vec<_> = grouped
            .regroup(&mut draws, &mut pages, 0)
            .iter()
            .map(|r| (r.tag[0], r.start, r.len))
            .collect();
        assert_eq!(
            runs,
            [(1, 0, 1), (3, 1, 2), (0, 3, 2), (2, 5, 3), (4, 8, 1)]
        );
        let order: Vec<_> = draws.iter().map(|d| (d[0], d[1])).collect();
        assert_eq!(
            order,
            [
                (1, 0),
                (3, 0),
                (3, 1),
                (0, 0),
                (0, 1),
                (2, 0),
                (2, 1),
                (2, 2),
                (4, 0)
            ]
        );
        let pages: Vec<_> = pages.iter().map(|p| p.0).collect();
        assert_eq!(pages, [0, 0, 0, 1, 1, 1, 1, 1, 1]);
    }

    #[test]
    fn bands_widen_past_near_and_stay_in_order() {
        assert_eq!([0.0, 63.0, 64.0, 255.0].map(band), [0, 0, 1, 3]);
        assert_eq!(band(256.0), 4);
        assert!(band(16384.0) <= 16);
        let mut last = 0;
        for d in 0..20000 {
            let b = band(f64::from(d));
            assert!(b >= last && b <= last + 1, "{d}");
            last = b;
        }
    }

    #[test]
    fn translucent_draws_go_back_to_front_by_page_only_far_away() {
        // (blocks, index, page)
        let mut draws = [
            (10.0, 0, 1),
            (20.0, 1, 0),
            (30.0, 2, 1),
            (1000.0, 3, 1),
            (1001.0, 4, 0),
            (1002.0, 5, 1),
        ];
        draws.sort_by_key(|&(d, i, p)| back_to_front(bits(d), i, (p, None)));
        let order: Vec<_> = draws.iter().map(|d| d.1).collect();
        // Far: one band, page 0 then page 1 (farthest first in it); near: exactly.
        assert_eq!(order, [4, 5, 3, 2, 1, 0]);
    }

    #[test]
    fn bands_stay_in_distance_order() {
        let (mut grouped, mut draws, mut pages) =
            list(&[(10.0, 1, 1), (100.0, 0, 1), (200.0, 0, 1)]);
        grouped.regroup(&mut draws, &mut pages, 0);
        let order: Vec<_> = draws.iter().map(|d| d[0]).collect();
        assert_eq!(order, [0, 1, 2]);
    }

    #[test]
    fn one_page_and_an_offset_list() {
        let (mut grouped, mut draws, mut pages) = list(&[(10.0, 3, 2), (90.0, 3, 1)]);
        let before = draws.clone();
        assert_eq!(grouped.regroup(&mut draws, &mut pages, 0)[1].start, 2);
        assert_eq!(draws, before);
        // A list after other draws: starts are absolute.
        let mut grouped = PageRuns::default();
        let mut draws = vec![[9; 5], [7, 0, 0, 0, 0], [8, 0, 0, 0, 0]];
        let mut pages = vec![(5, None), (1, None), (0, None)];
        grouped.push(bits(5.0), 1, 1, 1, [0; 3]);
        grouped.push(bits(6.0), 0, 2, 1, [1, 0, 0]);
        let runs = grouped.regroup(&mut draws, &mut pages, 1).to_vec();
        assert_eq!(draws, [[9; 5], [8, 0, 0, 0, 0], [7, 0, 0, 0, 0]]);
        assert_eq!(pages, [(5, None), (0, None), (1, None)]);
        assert_eq!((runs[0].tag[0], runs[0].start), (1, 1));
        // Empty runs are left out.
        grouped.clear();
        grouped.push(bits(1.0), 0, 0, 0, [0; 3]);
        assert!(grouped.regroup(&mut [], &mut [], 0).is_empty());
    }
}
