//! Horizon Culling over the curved Earth (`horizon_chunks`), and what the rays and the aim
//! read of the sampled tiles: their surface, their columns (lod.rs may not grow).

use super::{CELLS, Column, Lod, MAX_CHUNKS, MAX_LEVEL, Surface, TileKey};
use horizon_world::registry::BlockStateId;
use std::sync::Arc;

/// The Earth's radius Earth Curvature draws the distant terrain on, in blocks
/// (`gfx::lod::earth_drop`).
pub const EARTH_RADIUS: f64 = 6_371_000.0;
/// The overworld's sea, which the curved horizon is measured from.
const SEA_LEVEL: f64 = 63.0;
/// The highest terrain a far tile can hold: the build limit, 257 blocks above
/// the sea. A peak this tall sinks under the horizon last.
const HIGHEST_TERRAIN: f64 = 320.0;
/// Horizon Culling's distances are rounded up to this many chunks, so a player
/// climbing or falling replans the far terrain every few dozen blocks, not on
/// every one.
const HORIZON_STEP: u32 = 64;
/// How far LOD Distance goes while Earth Curvature is on, in chunks: past it,
/// even a peak at the build limit seen from the build limit lies under the
/// horizon (`horizon_chunks(320, ..)`, 114 km).
pub const HORIZON_MAX_CHUNKS: u32 = 7168;

/// How far far terrain can be seen over the curved Earth from an eye at
/// `eye_y`, in chunks: past it the tallest terrain (`HIGHEST_TERRAIN`) has
/// sunk under the horizon and Horizon Culling stops planning tiles.
///
/// Measured on the Earth as it is drawn, not the real one: the loaded chunks
/// stay flat out to `flat_radius` blocks and the curvature starts there
/// (`earth_drop`), the true sphere raised by `flat_radius^2 / 2R`. The eye's
/// line grazes that surface `sqrt(2Rh - f^2)` away (or the flat chunks' edge
/// when the eye is lower), and a peak `H` tall stands over it until
/// `sqrt(2RH)` past that point. `sqrt(2Rh) + sqrt(2RH + f^2)` bounds both
/// cases from above, so no visible tile is dropped.
pub fn horizon_chunks(eye_y: f64, flat_radius: f64) -> u32 {
    let eye = (eye_y - SEA_LEVEL).max(0.0);
    let peak = HIGHEST_TERRAIN - SEA_LEVEL;
    let f2 = flat_radius * flat_radius;
    let blocks = (2.0 * EARTH_RADIUS * eye).sqrt() + (2.0 * EARTH_RADIUS * peak + f2).sqrt();
    let chunks = (blocks / 16.0).ceil().min(f64::from(MAX_CHUNKS)) as u32;
    chunks.div_ceil(HORIZON_STEP) * HORIZON_STEP
}

impl Lod {
    /// The solid ground at block column (x, z): its top face's height and
    /// its block, from the finest tile sampled there. For aiming past the
    /// resident chunks (`game.raycast`); water is passed, as the view's ray
    /// passes it.
    pub fn ground(&self, x: i32, z: i32) -> Option<(i32, BlockStateId)> {
        (0..=MAX_LEVEL).find_map(|level| {
            let size = 16 << level;
            let key = TileKey {
                x: x.div_euclid(size),
                z: z.div_euclid(size),
                level,
            };
            let columns = self.ready.get(&key).filter(|c| c.len() == CELLS * CELLS)?;
            let cell = size / CELLS as i32;
            let (cx, cz) = ((x - key.x * size) / cell, (z - key.z * size) / cell);
            let surface = columns[cz as usize * CELLS + cx as usize].surface?;
            Some((surface.height, surface.state))
        })
    }
    /// The surface at block column (x, z), its water included, from the finest tile sampled
    /// there (`None` where none is): RT lighting's natural fogs past the loaded chunks.
    pub fn surface(&self, x: i32, z: i32) -> Option<Surface> {
        (0..=MAX_LEVEL).find_map(|level| {
            let size = 16 << level;
            let key = TileKey {
                x: x.div_euclid(size),
                z: z.div_euclid(size),
                level,
            };
            let columns = self.ready.get(&key).filter(|c| c.len() == CELLS * CELLS)?;
            let cell = size / CELLS as i32;
            let (cx, cz) = ((x - key.x * size) / cell, (z - key.z * size) / cell);
            columns[cz as usize * CELLS + cx as usize].surface
        })
    }
    /// Changes whenever the drawn tiles may have (a mesh arrived).
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Each drawn tile and its cells' columns, for the rays' far field (gfx/far_heights.rs).
    pub fn drawn_columns(&self) -> impl Iterator<Item = (TileKey, &Arc<Vec<Column>>)> {
        self.active.iter().filter_map(|&key| {
            let columns = self.ready.get(&key)?;
            (columns.len() == CELLS * CELLS).then_some((key, columns))
        })
    }
}
