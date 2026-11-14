//! What loads first: the areas kept loaded (`Ranges`) and how a column ranks in them, the
//! columns ahead of the player first (streaming.rs may not grow).

use super::*;

impl Ranges<'_> {
    /// `distance`, or the band's nearer priority close to it.
    pub(super) fn priority(&self, x: i32, z: i32) -> i64 {
        let own = self.distance(x, z);
        match self.band {
            Some(band) if band.within(x, z, 4) => own.min(band.priority(x, z)),
            _ => own,
        }
    }
    pub(super) fn player_distance(&self, x: i32, z: i32) -> i64 {
        let d = |c: (i32, i32)| {
            let (dx, dz) = (i64::from(x - c.0), i64::from(z - c.1));
            dx * dx + dz * dz
        };
        // The player's own area, also among the extra ones on a server, ranks by `ahead`.
        let around = |c: (i32, i32)| match self.ahead {
            Some(ahead) if c == self.center => ahead_distance(x - c.0, z - c.1, ahead),
            _ => d(c),
        };
        if let Some(cover) = self.cover {
            return around(self.center).min(cover.nearest_sq(x, z));
        }
        self.extra
            .iter()
            .map(|(c, _)| around(*c))
            .fold(around(self.center), i64::min)
    }

    /// How far past the view distance chunks are generated, and how far around
    /// a finished chunk (`near_finished`): decorated chunks need their
    /// neighbours generated too.
    pub(super) fn margins(decorates: bool) -> (i32, i32) {
        if decorates { (3, 2) } else { (1, 1) }
    }
    /// Chunks generation asks for: loaded from the save, or generated as protos.
    pub(super) fn requested(&self, x: i32, z: i32, decorates: bool) -> bool {
        let (margin, reach) = Self::margins(decorates);
        self.in_range(x, z, margin) || self.near_finished(x, z, reach)
    }
    /// Finished columns kept when they arrive: within two of the view distance
    /// or next to a finished chunk, and every column generation asked for (a
    /// saved one loads finished; dropping it had it asked for and loaded again
    /// every frame).
    pub(super) fn chunk_kept(&self, x: i32, z: i32, decorates: bool) -> bool {
        self.in_range(x, z, 2) || self.near_finished(x, z, 1) || self.requested(x, z, decorates)
    }
    /// Columns unloaded: past every column kept.
    pub(super) fn unloaded(&self, x: i32, z: i32, decorates: bool) -> bool {
        !self.in_range(x, z, 3) && !self.near_finished(x, z, Self::margins(decorates).1)
    }
    /// Whether a chunk finished here (`in_range(_, _, 1)`) lies within `reach`
    /// chunks of (x, z) on both axes: its decoration needs its 3x3 neighbours
    /// decorated, which need their own 3x3 neighbours generated. The ranges are
    /// circles, so a fixed margin alone misses the corners of those squares
    /// (render distance 2 never finished the chunks next to the player).
    pub(super) fn near_finished(&self, x: i32, z: i32, reach: i32) -> bool {
        let gap = |d: i32| i64::from((d.abs() - reach).max(0));
        let (dx, dz) = (gap(x - self.center.0), gap(z - self.center.1));
        let r = i64::from(self.radius + 1);
        // The extra areas are squares: their finished square grown by `reach`.
        (self.radius >= 0 && dx * dx + dz * dz <= r * r)
            || self.in_extra(x, z, 1 + reach)
            || self.in_band(x, z, 1 + reach)
    }
    pub(super) fn in_range(&self, x: i32, z: i32, margin: i32) -> bool {
        self.in_main_range(x, z, margin)
            || self.in_extra(x, z, margin)
            || self.in_band(x, z, margin)
    }
    /// Within `margin` of the pregeneration band.
    pub(super) fn in_band(&self, x: i32, z: i32, margin: i32) -> bool {
        self.band.is_some_and(|b| b.within(x, z, margin))
    }
    /// Within `margin` of an extra area's square (Chebyshev distance, as
    /// vanilla's ticket levels spread).
    pub(super) fn in_extra(&self, x: i32, z: i32, margin: i32) -> bool {
        if let Some(cover) = self.cover
            && (0..=ExtraCover::REACH).contains(&margin)
        {
            return cover.within(x, z, margin);
        }
        self.extra
            .iter()
            .any(|&(c, radius)| (x - c.0).abs().max((z - c.1).abs()) <= radius + margin)
    }
    /// The columns meshed for drawing: those vanilla draws around the
    /// player (`in_view_distance`), and forced ones.
    pub(super) fn in_view(&self, x: i32, z: i32) -> bool {
        in_view_distance(self.center, self.radius, x, z) || self.is_forced(x, z)
    }
    pub(super) fn in_main_range(&self, x: i32, z: i32, margin: i32) -> bool {
        let r = i64::from(self.radius + margin);
        let (dx, dz) = (i64::from(x - self.center.0), i64::from(z - self.center.1));
        if self.radius >= 0 && dx * dx + dz * dz <= r * r {
            return true;
        }
        self.near_forced(x, z, margin)
    }
}
impl Streaming {
    /// `ahead`, turned to the middle of its sector (`ahead_sector`), so priorities change
    /// only when the player turns past one; `None` with `HORIZON_STREAM_AHEAD=0`.
    pub(super) fn ranges_ahead(&self) -> Option<[f32; 2]> {
        let sector = ahead_sector(self.ahead.filter(|_| ahead_on())?);
        let angle = sector as f32 * std::f32::consts::TAU / AHEAD_SECTORS as f32;
        Some([angle.cos(), angle.sin()])
    }
}
/// `HORIZON_STREAM_AHEAD=0`: chunks load nearest first all around, as before; by default the
/// columns where the player looks or flies go first (`ahead_distance`).
pub(super) fn ahead_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("HORIZON_STREAM_AHEAD").map_or(true, |v| v != "0"))
}
pub(super) const AHEAD_SECTORS: i32 = 16;
/// The sector of a direction (x, z): sixteen around.
pub(super) fn ahead_sector([x, z]: [f32; 2]) -> i32 {
    let turn = z.atan2(x) / std::f32::consts::TAU;
    ((turn * AHEAD_SECTORS as f32).round() as i32).rem_euclid(AHEAD_SECTORS)
}
/// The squared distance a column (dx, dz) from the player ranks by, with the player facing
/// `ahead`: as it is within 60 degrees of it and in the ring around the player, up to four times
/// it behind. A flight outruns generation: with every column at its own distance, the ones
/// behind and to the sides took the workers' time while those ahead, which the player sees,
/// were still drawn as distant terrain. Behind the player a column now waits for those ahead at
/// twice its distance.
pub(super) fn ahead_distance(dx: i32, dz: i32, [ax, az]: [f32; 2]) -> i64 {
    let (x, z) = (i64::from(dx), i64::from(dz));
    let squared = x * x + z * z;
    if squared <= 8 {
        return squared;
    }
    let cos = (dx as f32 * ax + dz as f32 * az) / (squared as f32).sqrt();
    let k = ((0.5 - cos).max(0.0) * 8.0) as i64;
    squared * (4 + k) / 4
}
#[cfg(test)]
mod ahead_tests {
    use super::*;
    #[test]
    fn the_columns_ahead_rank_before_those_behind() {
        let east = [1.0, 0.0];
        // The ring around the player as it is; ahead as it is; behind at four times.
        assert_eq!(ahead_distance(-2, 2, east), 8);
        assert_eq!(ahead_distance(10, 0, east), 100);
        assert_eq!(ahead_distance(-10, 0, east), 400);
        // Within 60 degrees: as ahead; to the side: between.
        assert_eq!(ahead_distance(10, 5, east), 125);
        let side = ahead_distance(0, 10, east);
        assert!(side > 100 && side < 400, "{side}");
        // Twenty columns ahead go before eleven behind.
        assert!(ahead_distance(20, 0, east) < ahead_distance(-11, 0, east));
        assert_eq!(ahead_sector([1.0, 0.0]), 0);
        assert_eq!(ahead_sector([0.0, 1.0]), 4);
        assert_eq!(ahead_sector([1.0, -0.01]), 0);
    }
}
