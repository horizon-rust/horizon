//! Horizon culling of the distant terrain (`HORIZON_LOD_HORIZON=1`, off by default): a distant
//! tile whose every point lies under the skyline that nearer distant terrain draws, seen from
//! the eye, is left out of the frame's draws (the depth prepass, the world, both eyes in VR),
//! with what it costs the CPU to list.
//!
//! The skyline is kept per direction around the eye (`BINS` sectors of a pseudo-angle, cheap to
//! compute and in the same order as the true angle): each sector holds the highest elevation
//! (rise over run) under which a line of sight is sure to meet an occluder (`lod::Occluder`, a
//! box of a tile drawn solid and opaque from `OCCLUDER_FLOOR` to its top at every column) that
//! fills the whole sector. Tiles are tested nearest first, against the occluders wholly nearer
//! than they are, over every sector they touch (and one more on each side): a tile is hidden
//! only where the elevation of its highest point, seen at its nearest, is under the skyline in
//! all of them.
//!
//! Why the picture stays the same: a line of sight from the eye to a point of the tile (both at
//! or above the floor) stays above the floor; crossing an occluder's sector under the
//! occluder's top, it is between the floor and the top while over the occluder, where every
//! column is drawn solid. The occluders' tops sink with the Earth's curvature as drawn
//! (`earth_drop`, at their farthest), the tested tiles are taken unsunk (higher), and the eye is
//! taken a little higher than it is. Off when the eye is under the floor or under cover
//! (`Renderer::set_horizon_open`): a cave mouth or a window looks out under the skyline.
use super::lod::earth_drop;
use crate::lod::Occluder;
use glam::DVec3;

/// Sectors around the eye: about a fifth of a degree each.
const BINS: usize = 2048;
/// The eye is taken this much higher than the camera (blocks): the head's bob, the VR eyes.
const EYE_MARGIN: f64 = 0.25;
const FLOOR: f64 = crate::lod::OCCLUDER_FLOOR as f64;

/// The angle of (x, z) from the +x axis, as a number from 0 to 4 that grows with it (a quarter
/// turn per unit): the "diamond angle", no trigonometry.
fn pseudo_angle(x: f64, z: f64) -> f64 {
    if z >= 0.0 {
        if x >= 0.0 {
            z / (x + z)
        } else {
            1.0 - x / (-x + z)
        }
    } else if x < 0.0 {
        2.0 - z / (-x - z)
    } else {
        3.0 + x / (x - z)
    }
}

/// The pseudo-angles a rectangle relative to the eye covers, as (start, width), `None` where it
/// holds or touches the eye (it then covers half the turn or more).
fn arc(min: [f64; 2], max: [f64; 2]) -> Option<(f64, f64)> {
    if min[0] <= 0.0 && max[0] >= 0.0 && min[1] <= 0.0 && max[1] >= 0.0 {
        return None;
    }
    let first = pseudo_angle(min[0], min[1]);
    let (mut low, mut high) = (0.0f64, 0.0f64);
    for (x, z) in [(max[0], min[1]), (min[0], max[1]), (max[0], max[1])] {
        let mut offset = pseudo_angle(x, z) - first;
        if offset > 2.0 {
            offset -= 4.0;
        } else if offset < -2.0 {
            offset += 4.0;
        }
        low = low.min(offset);
        high = high.max(offset);
    }
    (high - low < 2.0).then_some(((first + low).rem_euclid(4.0), high - low))
}

/// The nearest and farthest horizontal distances from the eye to a rectangle relative to it.
fn distances(min: [f64; 2], max: [f64; 2]) -> (f64, f64) {
    let near = |lo: f64, hi: f64| lo.max(0.0).max(-hi);
    let far = |lo: f64, hi: f64| lo.abs().max(hi.abs());
    let length = |x: f64, z: f64| (x * x + z * z).sqrt();
    (
        length(near(min[0], max[0]), near(min[1], max[1])),
        length(far(min[0], max[0]), far(min[1], max[1])),
    )
}

/// The elevation of a height `rise` above the eye seen from horizontal distances `near` to
/// `far`: at its highest (`highest`) or lowest.
fn elevation(rise: f64, near: f64, far: f64, highest: bool) -> f64 {
    if (rise >= 0.0) == highest {
        rise / near.max(1e-3)
    } else {
        rise / far
    }
}

/// Distance bands of the occluders (`band`): eight to each doubling of the distance.
const BANDS: usize = 8 * 17;
fn band(far: f64) -> usize {
    ((far.max(1.0).log2() * 8.0) as usize).min(BANDS - 1)
}
/// The farthest an occluder of band `band` is.
fn band_end(band: usize) -> f64 {
    if band == BANDS - 1 {
        f64::INFINITY
    } else {
        ((band + 1) as f64 / 8.0).exp2()
    }
}
/// Runs `f` over the sectors `first` to `last` (wrapping), while it returns true; whether it
/// did for all of them.
fn for_sectors(
    sectors: &mut [f64],
    first: i64,
    last: i64,
    mut f: impl FnMut(&mut f64) -> bool,
) -> bool {
    let bins = BINS as i64;
    let start = first.rem_euclid(bins);
    let end = start + (last - first).min(bins - 1);
    if end < bins {
        sectors[start as usize..=end as usize].iter_mut().all(f)
    } else {
        sectors[start as usize..].iter_mut().all(&mut f)
            && sectors[..=(end - bins) as usize].iter_mut().all(f)
    }
}

struct Skyline {
    first: i64,
    last: i64,
    elevation: f64,
}

/// A tested tile: its index in the visible list, nearest distance, sectors and elevation.
struct Tested {
    index: usize,
    near: f64,
    first: i64,
    last: i64,
    elevation: f64,
}

#[derive(Default)]
pub(super) struct Horizon {
    sectors: Vec<f64>,
    bands: Vec<Vec<Skyline>>,
    /// Occluders that joined the last skyline, for the log.
    skylines: usize,
    tested: Vec<Tested>,
    hidden: Vec<bool>,
    /// Tiles listed and hidden since the last log line, and when it was.
    counted: (u64, u64),
    logged: Option<std::time::Instant>,
}

impl Horizon {
    /// Marks in `hidden` (resized to `tiles`) the tiles under the skyline of `occluders`, for an
    /// eye at `eye` (world); `curved` is the flat radius of the Earth's curvature where it is
    /// drawn. `tiles` yields each tile's world box (min, max). Returns the count hidden.
    pub fn cull<'a>(
        &mut self,
        eye: DVec3,
        curved: Option<f64>,
        occluders: impl Iterator<Item = &'a Occluder>,
        tiles: impl ExactSizeIterator<Item = (DVec3, DVec3)>,
    ) -> &[bool] {
        let count = tiles.len();
        self.hidden.clear();
        self.hidden.resize(count, false);
        let eye = eye + DVec3::Y * EYE_MARGIN;
        if eye.y < FLOOR {
            return &self.hidden;
        }
        let scale = BINS as f64 / 4.0;
        // The tiles to test first: the occluders farther than all of them are not needed.
        self.tested.clear();
        for (index, (min, max)) in tiles.enumerate() {
            if min.y < FLOOR {
                continue;
            }
            let rel_min = [min.x - eye.x, min.z - eye.z];
            let rel_max = [max.x - eye.x, max.z - eye.z];
            let Some((start, width)) = arc(rel_min, rel_max) else {
                continue;
            };
            let (near, far) = distances(rel_min, rel_max);
            self.tested.push(Tested {
                index,
                near,
                first: (start * scale).floor() as i64 - 1,
                last: ((start + width) * scale).floor() as i64 + 1,
                elevation: elevation(max.y - eye.y, near, far, true),
            });
        }
        self.tested
            .sort_unstable_by(|a, b| a.near.total_cmp(&b.near));
        let farthest = self.tested.last().map_or(0.0, |t| t.near);
        // The occluders by distance band (`band`): a band joins the skyline once a tested tile
        // is at least as far as the band's end, so no sort.
        for band in &mut self.bands {
            band.clear();
        }
        self.bands.resize_with(BANDS, Vec::new);
        for o in occluders {
            let min = [f64::from(o.min[0]) - eye.x, f64::from(o.min[1]) - eye.z];
            let max = [f64::from(o.max[0]) - eye.x, f64::from(o.max[1]) - eye.z];
            let (near, far) = distances(min, max);
            if far > farthest {
                continue;
            }
            let Some((start, width)) = arc(min, max) else {
                continue;
            };
            // The sectors it fills whole.
            let first = (start * scale).ceil() as i64;
            let last = ((start + width) * scale).floor() as i64 - 1;
            if last < first {
                continue;
            }
            let sunk = curved.map_or(0.0, |flat| earth_drop(far, flat));
            let rise = f64::from(o.hi) - sunk - eye.y;
            self.bands[band(far)].push(Skyline {
                first,
                last,
                elevation: elevation(rise, near, far, false),
            });
        }
        self.sectors.clear();
        self.sectors.resize(BINS, f64::NEG_INFINITY);
        let mut joined = 0;
        let mut hidden = 0;
        for t in &self.tested {
            while joined < BANDS && band_end(joined) <= t.near {
                for s in &self.bands[joined] {
                    for_sectors(&mut self.sectors, s.first, s.last, |sector| {
                        *sector = sector.max(s.elevation);
                        true
                    });
                }
                joined += 1;
            }
            if joined > 0
                && for_sectors(&mut self.sectors, t.first, t.last, |sector| {
                    *sector > t.elevation
                })
            {
                self.hidden[t.index] = true;
                hidden += 1;
            }
        }
        self.skylines = self.bands[..joined].iter().map(Vec::len).sum();
        self.counted.0 += count as u64;
        self.counted.1 += hidden;
        let now = std::time::Instant::now();
        if self
            .logged
            .is_none_or(|at| now - at >= std::time::Duration::from_secs(10))
        {
            let (listed, hidden) = std::mem::take(&mut self.counted);
            if listed > 0 {
                log::info!(
                    "Horizon: distant tiles under the skyline: {:.1} % of {} listed (10 s), {} occluders",
                    100.0 * hidden as f64 / listed as f64,
                    listed,
                    self.skylines,
                );
            }
            self.logged = Some(now);
        }
        &self.hidden
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pseudo_angle_grows_with_the_angle() {
        let mut last = -1.0;
        for step in 0..360 {
            let a = f64::from(step).to_radians();
            let p = pseudo_angle(a.cos(), a.sin());
            assert!(p > last && (0.0..4.0).contains(&p), "{step}: {p}");
            last = p;
        }
    }

    #[test]
    fn arcs_wrap_and_the_eye_inside_has_none() {
        assert!(arc([-1.0, -1.0], [1.0, 1.0]).is_none());
        // Straddling the +x axis (pseudo-angle 0): from just under 4 round to just over 0.
        let (start, width) = arc([10.0, -1.0], [12.0, 1.0]).unwrap();
        assert!(start > 3.8 && width < 0.2 && (start + width) > 4.0);
        let (start, width) = arc([10.0, 1.0], [12.0, 3.0]).unwrap();
        assert!(start > 0.0 && start + width < 0.5 && width > 0.0);
    }

    fn occluder(x: [i32; 2], z: [i32; 2], hi: i16) -> Occluder {
        Occluder {
            min: [x[0], z[0]],
            max: [x[1], z[1]],
            lo: crate::lod::OCCLUDER_FLOOR,
            hi,
        }
    }

    fn tile(x: [f64; 2], z: [f64; 2], y: [f64; 2]) -> (DVec3, DVec3) {
        (DVec3::new(x[0], y[0], z[0]), DVec3::new(x[1], y[1], z[1]))
    }

    #[test]
    fn a_ridge_hides_the_low_tiles_behind_it_not_the_high_ones() {
        // The eye at y 70 looking along +x; a ridge 200 blocks wide at x 1000..1200, 150 high.
        let eye = DVec3::new(0.0, 70.0, 0.0);
        let ridge = [occluder([1000, 1200], [-200, 200], 150)];
        let mut h = Horizon::default();
        let tiles = [
            // Low, right behind it: hidden.
            tile([1300.0, 1400.0], [-50.0, 50.0], [60.0, 120.0]),
            // A mountain behind it, rising above the line over the ridge: seen.
            tile([2000.0, 2100.0], [-50.0, 50.0], [60.0, 250.0]),
            // In front of the ridge: seen.
            tile([500.0, 600.0], [-50.0, 50.0], [60.0, 80.0]),
            // Beside it, out of its sectors: seen.
            tile([1300.0, 1400.0], [800.0, 900.0], [60.0, 80.0]),
            // Reaching under the floor (the sea's bottom): never hidden.
            tile([1300.0, 1400.0], [-50.0, 50.0], [20.0, 80.0]),
        ];
        let hidden = h.cull(eye, None, ridge.iter(), tiles.into_iter()).to_vec();
        assert_eq!(hidden, [true, false, false, false, false]);
        // Seen from above the ridge's top, nothing is.
        let high = DVec3::new(0.0, 400.0, 0.0);
        let hidden = h.cull(high, None, ridge.iter(), tiles.into_iter()).to_vec();
        assert_eq!(hidden, [false; 5]);
        // Nor from under the floor.
        let low = DVec3::new(0.0, 30.0, 0.0);
        let hidden = h.cull(low, None, ridge.iter(), tiles.into_iter()).to_vec();
        assert_eq!(hidden, [false; 5]);
    }

    #[test]
    fn a_tile_overlapping_the_occluders_distance_is_not_hidden_by_it() {
        let eye = DVec3::new(0.0, 70.0, 0.0);
        let ridge = [occluder([1000, 1200], [-200, 200], 150)];
        let mut h = Horizon::default();
        // Its nearest point is nearer than the ridge's farthest: tested before the ridge counts.
        let tiles = [tile([1100.0, 1150.0], [-20.0, 20.0], [60.0, 70.0])];
        assert_eq!(h.cull(eye, None, ridge.iter(), tiles.into_iter()), [false]);
    }

    #[test]
    fn a_line_of_sight_to_any_corner_of_a_hidden_tile_crosses_an_occluder_under_its_top() {
        // Brute force: random ridges and tiles; for each tile called hidden, every line from
        // the eye to its top corners passes over some occluder's footprint below its top.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as i32
        };
        let mut h = Horizon::default();
        for _ in 0..200 {
            let eye = DVec3::new(0.0, f64::from(50 + next(100)), 0.0);
            let occluders: Vec<_> = (0..40)
                .map(|_| {
                    let (x, z) = (next(4000) - 2000, next(4000) - 2000);
                    let size = 32 << next(4);
                    occluder([x, x + size], [z, z + size], (60 + next(200)) as i16)
                })
                .collect();
            let tiles: Vec<_> = (0..200)
                .map(|_| {
                    let (x, z) = (f64::from(next(8000) - 4000), f64::from(next(8000) - 4000));
                    let size = f64::from(16 << next(5));
                    let low = f64::from(48 + next(40));
                    tile(
                        [x, x + size],
                        [z, z + size],
                        [low, low + f64::from(next(150))],
                    )
                })
                .collect();
            let hidden = h
                .cull(eye, None, occluders.iter(), tiles.iter().copied())
                .to_vec();
            for (t, _) in tiles.iter().zip(&hidden).filter(|(_, h)| **h) {
                let (min, max) = t;
                for corner in [
                    DVec3::new(min.x, max.y, min.z),
                    DVec3::new(max.x, max.y, min.z),
                    DVec3::new(min.x, max.y, max.z),
                    DVec3::new(max.x, max.y, max.z),
                ] {
                    let blocked = occluders.iter().any(|o| {
                        // Sample the line densely: some point over the footprint, under the top.
                        (1..2000).any(|i| {
                            let p = eye + (corner - eye) * (f64::from(i) / 2000.0);
                            p.x >= f64::from(o.min[0])
                                && p.x <= f64::from(o.max[0])
                                && p.z >= f64::from(o.min[1])
                                && p.z <= f64::from(o.max[1])
                                && p.y < f64::from(o.hi)
                        })
                    });
                    assert!(blocked, "eye {eye}, tile {min} {max}, corner {corner}");
                }
            }
        }
    }

    #[test]
    #[ignore = "a timing, not a check"]
    fn time_a_far_plan_of_occluders() {
        // Occluders laid as the far plan's tiles are: rings of tiles of each level, 4 to 8 of
        // their sizes away, 8 cells to an occluder.
        let mut occluders = Vec::new();
        let mut tiles = Vec::new();
        for level in 0..11 {
            let size = 16 << level;
            let block = if level >= 5 { size / 2 } else { size };
            for tz in -8..8 {
                for tx in -8..8 {
                    let ring = i32::max(i32::abs(tx * 2 + 1), i32::abs(tz * 2 + 1));
                    if !(7..16).contains(&ring) || level > 0 && ring < 8 {
                        continue;
                    }
                    let (x, z) = (tx * size, tz * size);
                    tiles.push(tile(
                        [f64::from(x), f64::from(x + size)],
                        [f64::from(z), f64::from(z + size)],
                        [60.0, 140.0],
                    ));
                    for bz in (0..size).step_by(block as usize) {
                        for bx in (0..size).step_by(block as usize) {
                            let hi = 70 + ((x + bx + z + bz).rem_euclid(97)) as i16;
                            occluders.push(occluder(
                                [x + bx, x + bx + block],
                                [z + bz, z + bz + block],
                                hi,
                            ));
                        }
                    }
                }
            }
        }
        let mut h = Horizon::default();
        let started = std::time::Instant::now();
        let mut hidden = 0;
        for _ in 0..100 {
            hidden = h
                .cull(
                    DVec3::new(10.0, 80.0, 10.0),
                    Some(512.0),
                    occluders.iter(),
                    tiles.iter().copied(),
                )
                .iter()
                .filter(|h| **h)
                .count();
        }
        eprintln!(
            "{} occluders, {} tiles ({hidden} hidden): {:?} a frame",
            occluders.len(),
            tiles.len(),
            started.elapsed() / 100
        );
    }
}
