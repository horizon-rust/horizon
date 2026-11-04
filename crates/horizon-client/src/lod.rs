//! Bounded voxel-column terrain for distances beyond the resident chunk area.
//! Far tiles never enter World, lighting, simulation or the section cave graph.
use crate::{
    blocks::BlockFacts,
    chunk::{Chunk, MIN_Y},
    fast_hash::{FxHashMap, FxHashSet},
    mesher::SectionMesh,
    streaming::{LodSource, WorkerContext},
};
mod mesh;
pub mod store;
use horizon_world::registry::BlockStateId;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

/// Cells on a side of a tile up to `FAR_LEVEL`, and of a coarser one (`TileKey::cells`):
/// coarse tiles have more, finer cells for the same tile count (the CPU's work goes by tiles,
/// not cells). From `FAR_LEVEL` a cell is a chunk or wider.
const CELLS: usize = 16;
const FAR_CELLS: usize = 32;
const FAR_LEVEL: u8 = 5;
const MAX_SAVED: usize = 1024;
/// How many resident chunks may wait for the worker's summary at once
/// (`Lod::observing`).
pub const OBSERVE_AHEAD: usize = 8;
const SAMPLE_JOBS: usize = 4;
/// While the near terrain loads on a machine with four threads or fewer, far
/// terrain starts one sample job at most this often and no detail: it would
/// otherwise take the cores the near chunks and the frame need.
const HELD_SAMPLE_STEP: std::time::Duration = std::time::Duration::from_millis(500);
/// Arrivals (samples, detail, finished meshes) select again at most this often: each
/// selection walks the whole quadtree, about a millisecond at a large distance, and samples
/// arrive nearly every frame while the far terrain fills in. A replan selects at once.
const SELECT_STEP: std::time::Duration = std::time::Duration::from_millis(50);
/// The main thread's time a frame for the far terrain, its update and its uploads together
/// (`upload_budget`). Measured on an RTX 4090 flying at 100 blocks/s with 1024 chunks, the
/// LOD stages took about 10 ms of a slow frame before the selection, the plan and the
/// sampling choice left the main thread or became incremental.
pub const FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(2);
/// What the uploads keep however long the update took: tiles keep arriving.
pub const UPLOAD_FLOOR: std::time::Duration = std::time::Duration::from_micros(500);
/// The time `Renderer::upload_lods` may take once `Lod::update` (and gathering the covered
/// columns) took `spent` of this frame: what remains of `FRAME_BUDGET`, at least `UPLOAD_FLOOR`.
pub fn upload_budget(spent: std::time::Duration) -> std::time::Duration {
    FRAME_BUDGET.saturating_sub(spent).max(UPLOAD_FLOOR)
}
/// EXPERIMENTAL `HORIZON_LOD_DETAIL=N`: distant cells N times smaller on each side (N squared
/// as many tiles and cells), for measuring how far distant detail can go; 1 without it.
fn detail_scale() -> i32 {
    static SCALE: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| {
        std::env::var("HORIZON_LOD_DETAIL")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n: &i32| (1..=8).contains(&n))
            .unwrap_or(1)
    })
}
/// The coarsest tile level: `Plan` roots cover at most 1024 chunks around.
pub(crate) const MAX_LEVEL: u8 = 10;
/// Top solid face and optional fluid face, in absolute block coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub height: i32,
    pub state: BlockStateId,
    pub water: Option<(i32, BlockStateId)>,
    pub biome: u16,
}
/// A half-open material run; gaps between runs remain air.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub min: i16,
    pub max: i16,
    pub state: BlockStateId,
    pub biome: u16,
    pub light: u16,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Column {
    pub surface: Option<Surface>,
    pub spans: Vec<Span>,
}
impl Column {
    /// Reduce an entire horizontal footprint, rather than inflating its corner
    /// column (a single leaf used to become a whole distant tree-sized slab).
    pub(crate) fn reduced(columns: &[Column; 256], x: usize, z: usize, width: usize) -> Self {
        if width == 1 {
            return columns[z * 16 + x].clone();
        }
        let mut events = Vec::new();
        for dz in z..z + width {
            for dx in x..x + width {
                for span in &columns[dz * 16 + dx].spans {
                    let material = (span.state, span.biome, span.light);
                    events.push((span.min, material, 1));
                    events.push((span.max, material, -1));
                }
            }
        }
        events.sort_unstable_by_key(|event| event.0);
        let mut result = Self::default();
        let mut votes = Vec::<((BlockStateId, u16), i32)>::new();
        let mut sky = [0i32; 16];
        let mut block = [0i32; 16];
        let mut occupied = 0;
        let mut bottom = MIN_Y as i16;
        for (y, material, change) in events {
            if bottom < y {
                if occupied * 2 >= (width * width) as i32
                    && let Some(&((state, biome), _)) =
                        votes.iter().max_by_key(|&(material, count)| {
                            (*count, std::cmp::Reverse((material.0.0, material.1)))
                        })
                {
                    // Lighting must not split votes for the same material.
                    // Reduction can expose buried faces; retain the brightest
                    // actual light in this occupied footprint, per channel.
                    let light = (sky.iter().rposition(|&n| n > 0).unwrap_or(0) << 4)
                        | (block.iter().rposition(|&n| n > 0).unwrap_or(0) << 12);
                    result.push(Span {
                        min: bottom,
                        max: y,
                        state,
                        biome,
                        light: light as u16,
                    });
                }
                bottom = y;
            }
            occupied += change;
            sky[usize::from((material.2 >> 4) & 15)] += change;
            block[usize::from((material.2 >> 12) & 15)] += change;
            let material = (material.0, material.1);
            if let Some(vote) = votes.iter_mut().find(|vote| vote.0 == material) {
                vote.1 += change;
            } else {
                votes.push((material, change));
            }
        }
        result
    }
    pub fn from_surface(surface: Option<Surface>) -> Self {
        let mut column = Self {
            surface,
            spans: Vec::new(),
        };
        if let Some(s) = surface {
            if s.height > MIN_Y {
                column.spans.push(Span {
                    min: MIN_Y as i16,
                    max: s.height as i16,
                    state: s.state,
                    biome: s.biome,
                    light: 240,
                });
            }
            if let Some((height, state)) = s.water {
                column.spans.push(Span {
                    min: s.height as i16,
                    max: height as i16,
                    state,
                    biome: s.biome,
                    light: 240,
                });
            }
        }
        column
    }
    pub(crate) fn push(&mut self, span: Span) {
        if let Some(last) = self.spans.last_mut()
            && last.max == span.min
            && last.state == span.state
            && last.biome == span.biome
            && last.light == span.light
        {
            last.max = span.max;
        } else {
            self.spans.push(span);
        }
    }
    /// `kinds`: `state_kinds`, per block state.
    fn from_chunk(chunk: &Chunk, kinds: &[u8], x: i32, z: i32) -> Self {
        let kind = |state: BlockStateId| kinds.get(state.0 as usize).copied().unwrap_or(0);
        let face_light = |y: i32| {
            let mut sky = 0;
            let mut block = 0;
            for (dx, dy, dz) in [
                (0, 0, 0),
                (-1, 0, 0),
                (1, 0, 0),
                (0, -1, 0),
                (0, 1, 0),
                (0, 0, -1),
                (0, 0, 1),
            ] {
                if !(0..16).contains(&(x + dx)) || !(0..16).contains(&(z + dz)) {
                    continue;
                }
                let value = chunk.light(x + dx, y + dy, z + dz);
                sky = sky.max(value & 0xf0);
                block = block.max(value & 15);
            }
            sky | block
        };
        let mut column = Self::default();
        for (sy, section) in chunk.sections.iter().enumerate() {
            let min = MIN_Y + sy as i32 * 16;
            let uniform = section.blocks.bits() == 0
                && section.biomes.bits() == 0
                && matches!(section.light, crate::chunk::Light::Uniform(_));
            let uniform = uniform
                && face_light(min) == chunk.light(x, min, z)
                && face_light(min + 15) == chunk.light(x, min, z);
            let step = if uniform { 16 } else { 1 };
            for offset in (0..16).step_by(step) {
                let y = min + offset;
                let state = BlockStateId(chunk.get(x, y, z));
                let kind = kind(state);
                if kind & KEPT == 0 {
                    continue;
                }
                let light = face_light(y);
                column.push(Span {
                    min: y as i16,
                    max: (y + step as i32) as i16,
                    state,
                    biome: chunk.biome(x, y, z),
                    light: u16::from(light & 0xf0) | (u16::from((light & 15).max(kind >> 4)) << 12),
                });
            }
        }
        let mut water = None;
        for span in column.spans.iter().rev() {
            if kind(span.state) & FLUID != 0 {
                water.get_or_insert((i32::from(span.max), span.state));
            } else {
                column.surface = Some(Surface {
                    height: i32::from(span.max),
                    state: span.state,
                    water,
                    biome: span.biome,
                });
                break;
            }
        }
        if column.surface.is_none()
            && let Some((height, state)) = water
        {
            column.surface = Some(Surface {
                height: MIN_Y,
                state: BlockStateId(
                    kinds.iter().position(|&kind| kind & AIR != 0).unwrap_or(0) as u32
                ),
                water: Some((height, state)),
                biome: chunk.biome(x, height - 1, z),
            });
        }
        column
    }
}
/// A chunk's 256 columns (`Column::from_chunk`), on the far-terrain worker.
fn summarize(chunk: &Chunk, kinds: &[u8]) -> Arc<[Column; 256]> {
    Arc::new(std::array::from_fn(|i| {
        Column::from_chunk(chunk, kinds, (i & 15) as i32, (i >> 4) as i32)
    }))
}
/// `state_kinds` bits: kept in columns, a fluid, the air state; the light
/// emission is in the high nibble.
const KEPT: u8 = 1;
const FLUID: u8 = 2;
const AIR: u8 = 4;
/// What `Column::from_chunk` needs of each block state, read from the facts
/// once: matching their names for every block made observing a chunk slow.
fn state_kinds(facts: &BlockFacts) -> Vec<u8> {
    (0..facts.states.len())
        .map(|i| {
            let state = BlockStateId(i as u32);
            let f = facts.get(state);
            let fluid = matches!(&*f.name, "minecraft:water" | "minecraft:lava");
            let kept = !facts.is_air(state)
                && (f.solid_render || f.full_collision || f.name.ends_with("_leaves") || fluid);
            (u8::from(kept) * KEPT)
                | (u8::from(fluid) * FLUID)
                | (u8::from(state == facts.air) * AIR)
                | (f.emission.min(15) << 4)
        })
        .collect()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub x: i32,
    pub z: i32,
    pub level: u8,
}
impl TileKey {
    pub fn size(self) -> i32 {
        16 << self.level
    }
    /// Cells on a side (`CELLS`, `FAR_CELLS`).
    pub fn cells(self) -> usize {
        if self.level >= FAR_LEVEL {
            FAR_CELLS
        } else {
            CELLS
        }
    }
    /// A cell's width in blocks.
    pub fn stride(self) -> i32 {
        self.size() / self.cells() as i32
    }
    pub fn origin(self) -> [i32; 3] {
        [self.x * self.size(), 0, self.z * self.size()]
    }
    fn parent(self) -> Self {
        Self {
            x: self.x.div_euclid(2),
            z: self.z.div_euclid(2),
            level: self.level + 1,
        }
    }
    fn children(self) -> [Self; 4] {
        std::array::from_fn(|i| Self {
            x: self.x * 2 + (i & 1) as i32,
            z: self.z * 2 + (i >> 1) as i32,
            level: self.level - 1,
        })
    }
    fn distance(self, camera: [i32; 2]) -> i32 {
        let [x, _, z] = self.origin();
        (x - camera[0])
            .max(camera[0] - x - self.size())
            .max(z - camera[1])
            .max(camera[1] - z - self.size())
            .max(0)
    }
    fn in_view(self, camera: [i32; 2], yaw: f32) -> bool {
        let [x, _, z] = self.origin();
        let (sin, cos) = yaw.sin_cos();
        let mut forward = f32::NEG_INFINITY;
        let mut left = f32::NEG_INFINITY;
        let mut right = f32::NEG_INFINITY;
        for (dx, dz) in [
            (0, 0),
            (self.size(), 0),
            (0, self.size()),
            (self.size(), self.size()),
        ] {
            let (x, z) = ((x + dx - camera[0]) as f32, (z + dz - camera[1]) as f32);
            let f = -x * sin + z * cos;
            let side = x * cos + z * sin;
            forward = forward.max(f);
            left = left.max(f * 0.866 + side * 0.5);
            right = right.max(f * 0.866 - side * 0.5);
        }
        forward > 0. && left > 0. && right > 0.
    }
}
/// Whether `tile` overlaps `other` or shares part of an edge with it (corners don't count).
fn touches(tile: TileKey, other: TileKey) -> bool {
    let [x, _, z] = tile.origin();
    let size = tile.size();
    let [ox, _, oz] = other.origin();
    let os = other.size();
    (x < ox + os && ox < x + size && z < oz + os && oz < z + size)
        || (((x + size == ox) || (ox + os == x)) && z < oz + os && oz < z + size)
        || (((z + size == oz) || (oz + os == z)) && x < ox + os && ox < x + size)
}
/// The tiles of `tiles` that touch one of `changed`: for each changed tile, only the keys of
/// each level around it are looked up (scanning `tiles` where that range is larger), instead of
/// every tile against every changed one.
fn touching(tiles: &FxHashSet<TileKey>, changed: &[TileKey]) -> FxHashSet<TileKey> {
    let mut out = FxHashSet::default();
    for &other in changed {
        let [ox, _, oz] = other.origin();
        let os = other.size();
        for level in 0..=MAX_LEVEL {
            let size = 16 << level;
            // Keys whose square reaches [ox - size, ox + os + size) on each axis.
            let (x0, x1) = ((ox - size).div_euclid(size), (ox + os).div_euclid(size));
            let (z0, z1) = ((oz - size).div_euclid(size), (oz + os).div_euclid(size));
            let range = i64::from(x1 - x0 + 1) * i64::from(z1 - z0 + 1);
            if range > tiles.len() as i64 {
                out.extend(
                    tiles
                        .iter()
                        .copied()
                        .filter(|t| t.level == level && touches(*t, other)),
                );
                continue;
            }
            for z in z0..=z1 {
                for x in x0..=x1 {
                    let key = TileKey { x, z, level };
                    if tiles.contains(&key) && touches(key, other) {
                        out.insert(key);
                    }
                }
            }
        }
    }
    out
}
#[derive(Clone, Default)]
struct Plan {
    roots: Vec<TileKey>,
    nodes: FxHashSet<TileKey>,
    split: FxHashSet<TileKey>,
    elevation: i32,
}
impl Plan {
    #[cfg(test)]
    fn new(camera: [i32; 2], detail: i32, distance: u32) -> Self {
        Self::with_previous(camera, detail, distance, &Self::default(), 0, false)
    }
    /// `reduced`: Reduced far detail (`graphics::FarDetail`), each level from
    /// half the distance.
    fn with_previous(
        camera: [i32; 2],
        detail: i32,
        distance: u32,
        previous: &Self,
        elevation: i32,
        reduced: bool,
    ) -> Self {
        let range = distance.min(1024) as i32 * 16;
        let mut level = 0;
        while (16 << level) < range {
            level += 1;
        }
        let size = 16 << level;
        let mut result = Self {
            elevation,
            ..Self::default()
        };
        for z in (camera[1] - range).div_euclid(size)..=(camera[1] + range).div_euclid(size) {
            for x in (camera[0] - range).div_euclid(size)..=(camera[0] + range).div_euclid(size) {
                let key = TileKey { x, z, level };
                result.roots.push(key);
                result.visit(key, camera, detail, range, previous, reduced);
            }
        }
        result
    }
    fn visit(
        &mut self,
        key: TileKey,
        camera: [i32; 2],
        detail: i32,
        range: i32,
        previous: &Self,
        reduced: bool,
    ) {
        if key.distance(camera) > range {
            return;
        }
        self.nodes.insert(key);
        // Keep distant cells small on screen; finest tiles retain individual
        // blocks. Reduced keeps them twice as large (a quarter of the cells).
        let split = (if previous.split.contains(&key) { 10 } else { 8 }) * detail_scale();
        let threshold = key.size() * if reduced { split / 2 } else { split };
        if key.level > 0
            && (key.distance(camera) < detail * 16 + 16
                || key.distance(camera).max(self.elevation) < threshold)
        {
            self.split.insert(key);
            for child in key.children() {
                self.visit(child, camera, detail, range, previous, reduced);
            }
        }
    }
    fn select(
        &self,
        key: TileKey,
        ready: &FxHashMap<TileKey, impl Sized>,
        out: &mut Vec<TileKey>,
    ) -> bool {
        if !self.nodes.contains(&key) {
            return true;
        }
        if self.split.contains(&key) {
            // Visit every child: a missing sibling must not hide ready detail.
            let complete = key
                .children()
                .into_iter()
                .map(|child| self.select(child, ready, out))
                .fold(true, |complete, child| complete & child);
            if complete {
                return true;
            }
            if ready.contains_key(&key) {
                // Its mesh is clipped around every selected finer tile.
                out.push(key);
                return true;
            }
            false
        } else if ready.contains_key(&key) {
            out.push(key);
            true
        } else {
            false
        }
    }
    fn is_leaf(&self, key: &TileKey) -> bool {
        self.nodes.contains(key) && !self.split.contains(key)
    }
}
/// What the far plan's thread is asked for at each chunk crossing (`Plan::with_previous`).
struct PlanRequest {
    camera: [i32; 2],
    detail: i32,
    distance: u32,
    elevation: i32,
    reduced: bool,
}
/// A new plan and how it differs from the one before it, so the main thread updates what
/// depends on the plan by the tiles that changed. Building the plan (about 12000 nodes at
/// 1024 chunks, 0.5 ms) and filtering every sample by it (another 0.5 ms) on the main thread
/// at each chunk crossing was part of the 1.6 ms `lod_plan` and `lod_select` of a slow frame.
struct Replan {
    plan: Arc<Plan>,
    change: PlanChange,
}
#[derive(Debug, Default, PartialEq)]
struct PlanChange {
    /// Nodes of the old plan the new one has not.
    gone: Vec<TileKey>,
    /// Leaves of the new plan that were not leaves of the old one.
    leaves: Vec<TileKey>,
    /// Leaves of the old plan the new one splits.
    split: Vec<TileKey>,
}
impl PlanChange {
    fn between(old: &Plan, new: &Plan) -> Self {
        let mut change = Self {
            gone: old
                .nodes
                .iter()
                .copied()
                .filter(|key| !new.nodes.contains(key))
                .collect(),
            leaves: new
                .nodes
                .iter()
                .copied()
                .filter(|key| !new.split.contains(key) && !old.is_leaf(key))
                .collect(),
            split: old
                .nodes
                .iter()
                .copied()
                .filter(|key| !old.split.contains(key) && new.split.contains(key))
                .collect(),
        };
        // Hash order means nothing; sorted, the lists compare in tests.
        change.gone.sort_unstable();
        change.leaves.sort_unstable();
        change.split.sort_unstable();
        change
    }
}
/// The plan's leaves with no samples yet (`Lod::ready`), kept up to date as samples come and
/// the plan changes: `request_samples` picks among them. It scanned every node of the plan
/// for them on each frame a sample slot was free (nearly all of them in flight), 2 ms of a
/// slow frame at 1024 chunks (`lod_wanted`).
#[derive(Debug, Default, PartialEq)]
struct Missing(FxHashSet<TileKey>);
impl Missing {
    #[cfg(test)]
    fn new<V>(plan: &Plan, ready: &FxHashMap<TileKey, V>) -> Self {
        Self(
            plan.nodes
                .iter()
                .copied()
                .filter(|key| !plan.split.contains(key) && !ready.contains_key(key))
                .collect(),
        )
    }
    /// After the plan changed by `change`, `ready` already without the gone nodes.
    fn replan<V>(&mut self, change: &PlanChange, ready: &FxHashMap<TileKey, V>) {
        for key in change.gone.iter().chain(&change.split) {
            self.0.remove(key);
        }
        self.0.extend(
            change
                .leaves
                .iter()
                .copied()
                .filter(|key| !ready.contains_key(key)),
        );
    }
}
/// The children of `active`'s tiles that `plan` splits: the broad refinement
/// `request_samples` alternates with nearby leaves. Kept with the plan and the active tiles
/// they came from (`Lod::fallback`) rather than found on every frame (a lookup per active
/// tile, about 9000 at 1024 chunks).
fn fallback_children(plan: &Plan, active: &FxHashSet<TileKey>) -> Vec<TileKey> {
    let mut children: Vec<_> = active
        .iter()
        .filter(|parent| plan.split.contains(parent))
        .flat_map(|parent| parent.children())
        .filter(|child| plan.nodes.contains(child))
        .collect();
    children.sort_unstable();
    children
}
/// The tiles to draw (`Plan::select`, less the covered finest ones) and those of them to mesh:
/// `dirty`, the new ones and those touching a tile that came or went. `None` when nothing
/// changed. On the far terrain's worker: walking the quadtree, comparing the sets and finding
/// the touching tiles took about 1 ms of the main thread at 1024 chunks.
fn choose<V>(
    plan: &Plan,
    ready: &FxHashMap<TileKey, V>,
    covered: &FxHashSet<(i32, i32)>,
    active: &FxHashSet<TileKey>,
    mut dirty: FxHashSet<TileKey>,
) -> Option<(FxHashSet<TileKey>, FxHashSet<TileKey>)> {
    let mut selected = Vec::new();
    for &key in &plan.roots {
        plan.select(key, ready, &mut selected);
    }
    // Their normal meshes already provide coverage. Empty stand-ins
    // must not become hundreds of redundant fallback clipping masks.
    selected.retain(|key| key.level != 0 || !covered.contains(&(key.x, key.z)));
    let next: FxHashSet<_> = selected.iter().copied().collect();
    let changed: Vec<_> = active.symmetric_difference(&next).copied().collect();
    dirty.extend(touching(&next, &changed));
    dirty.extend(next.difference(active).copied());
    dirty.retain(|key| next.contains(key));
    (!dirty.is_empty() || next != *active).then_some((next, dirty))
}
/// The bounds in blocks of `covered`'s columns: [x0, z0, x1, z1], half-open.
fn covered_bounds(covered: &FxHashSet<(i32, i32)>) -> Option<[i32; 4]> {
    covered
        .iter()
        .fold(None, |bounds, &column| Some(extend_bounds(bounds, column)))
}
fn extend_bounds(bounds: Option<[i32; 4]>, (x, z): (i32, i32)) -> [i32; 4] {
    match bounds {
        None => [x * 16, z * 16, (x + 1) * 16, (z + 1) * 16],
        Some([x0, z0, x1, z1]) => [
            x0.min(x * 16),
            z0.min(z * 16),
            x1.max((x + 1) * 16),
            z1.max((z + 1) * 16),
        ],
    }
}
/// Whether the column lies on an edge of `bounds`: removing it may shrink them.
fn on_bounds_edge(bounds: Option<[i32; 4]>, (x, z): (i32, i32)) -> bool {
    bounds.is_none_or(|[x0, z0, x1, z1]| {
        x * 16 == x0 || z * 16 == z0 || (x + 1) * 16 == x1 || (z + 1) * 16 == z1
    })
}
/// The full-detail columns (`Lod::update`), as the renderer's complete columns in the render
/// distance change.
pub enum Covered {
    /// None changed.
    Same,
    /// All of them, after the render area moved.
    All(FxHashSet<(i32, i32)>),
    /// Columns that became covered (`true`) or not, with columns unchanged allowed.
    Changed(Vec<((i32, i32), bool)>),
}
/// A finished tile mesh and its bounds, relative to the tile's origin (`mesh_bounds`), found
/// on the worker: a pass over every vertex of each tile cost the uploads' frame time.
pub struct TileMesh {
    pub mesh: SectionMesh,
    pub bounds: Option<(glam::Vec3, glam::Vec3)>,
}
/// The box of a mesh's vertices, `None` for an empty mesh.
pub fn mesh_bounds(mesh: &SectionMesh) -> Option<(glam::Vec3, glam::Vec3)> {
    let mut positions = mesh
        .solid
        .iter()
        .chain(&mesh.translucent)
        .map(|v| glam::Vec3::from(v.pos));
    let first = positions.next()?;
    Some(positions.fold((first, first), |(min, max), pos| {
        (min.min(pos), max.max(pos))
    }))
}
/// Visited chunks' summaries (`Lod::saved`) and their reductions.
type Saved = FxHashMap<(i32, i32), Arc<[Column; 256]>>;
type Reduced = FxHashMap<(i32, i32), Arc<ReducedColumns>>;
/// Each tile's samples (`Lod::ready`).
type Ready = FxHashMap<TileKey, Arc<Vec<Column>>>;
/// What a mesh job reads, as it was when the job began: the worker gathers
/// every tile's inputs from it (gathering on the render thread stalled the
/// frame, tens of milliseconds for a large job on a 4-thread machine).
struct Snapshot {
    active: Arc<FxHashSet<TileKey>>,
    samples: Ready,
    saved: Arc<Saved>,
    reduced: Arc<Reduced>,
}
/// A selection and mesh job (`Lod::select`): the main thread's state, shared. The main
/// thread changes it copy-on-write (`Arc::make_mut`), so sending it costs nothing; cloning
/// the samples, the visited summaries and the covered columns for each job, and building the
/// selection, were about 2 ms of the main thread per job at 1024 chunks.
struct Selection {
    plan: Arc<Plan>,
    ready: Arc<Ready>,
    active: Arc<FxHashSet<TileKey>>,
    dirty: FxHashSet<TileKey>,
    covered: Arc<FxHashSet<(i32, i32)>>,
    bounds: Option<[i32; 4]>,
    saved: Arc<Saved>,
    reduced: Arc<Reduced>,
}
impl Selection {
    /// On the worker: `None` when asked to stop.
    fn run(
        self,
        mesher: &mut mesh::Mesher,
        context: &WorkerContext,
        stop: &AtomicBool,
    ) -> Option<Done> {
        let Self {
            plan,
            ready,
            active,
            dirty,
            covered,
            bounds,
            saved,
            reduced,
        } = self;
        let Some((next, dirty)) = choose(&plan, &ready, &covered, &active, dirty) else {
            return Some(Done::Unchanged);
        };
        let mut finer = Lod::finer(&next, &dirty);
        let samples = next
            .iter()
            .filter_map(|key| Some((*key, ready.get(key)?.clone())))
            .collect();
        // The main thread changes the samples nearly every frame: held during the meshing,
        // the first change would copy the whole map.
        drop(ready);
        let removed = active.difference(&next).copied().collect();
        let fallback = fallback_children(&plan, &next);
        let snapshot = Snapshot {
            active: Arc::new(next),
            samples,
            saved,
            reduced,
        };
        let mut meshes = Vec::with_capacity(dirty.len());
        for key in dirty {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let finer = finer.remove(&key).unwrap_or_default();
            let input = snapshot.mesh_input(key, finer);
            let mut mesh = mesher.mesh(
                input.key,
                &input.columns(),
                &covered,
                bounds,
                context,
                &input.neighbors,
                &input.finer,
            );
            mesh.sort_solid();
            let bounds = mesh_bounds(&mesh);
            meshes.push((input.key, TileMesh { mesh, bounds }));
        }
        Some(Done::Mesh {
            active: snapshot.active,
            removed,
            meshes,
            covered: (*covered).clone(),
            fallback: (plan, fallback),
        })
    }
}
struct MeshInput {
    key: TileKey,
    /// The tile's samples, with `visited` replacing some of them (`columns`).
    samples: Arc<Vec<Column>>,
    visited: Vec<(usize, Column)>,
    neighbors: [Vec<mesh::EdgeColumn>; 4],
    finer: Vec<TileKey>,
}
struct ReducedColumns {
    columns: Arc<[Column; 256]>,
    levels: [Vec<Column>; 4],
}
impl MeshInput {
    /// Built on the worker: cloning every tile's samples on the render
    /// thread stalled it for tens of milliseconds after a move.
    fn columns(&self) -> Vec<Column> {
        let mut columns = (*self.samples).clone();
        for (index, column) in &self.visited {
            columns[*index] = column.clone();
        }
        columns
    }
}
impl ReducedColumns {
    fn new(columns: Arc<[Column; 256]>) -> Self {
        let levels = std::array::from_fn(|level| {
            let width = 2 << level;
            (0..16 / width)
                .flat_map(|z| {
                    let columns = &columns;
                    (0..16 / width)
                        .map(move |x| Column::reduced(columns, x * width, z * width, width))
                })
                .collect()
        });
        Self { columns, levels }
    }
    fn get(&self, x: usize, z: usize, width: usize) -> &Column {
        if width == 1 {
            &self.columns[z * 16 + x]
        } else {
            &self.levels[width.trailing_zeros() as usize - 1]
                [(z / width) * (16 / width) + x / width]
        }
    }
}
enum Job {
    Sample(TileKey),
    /// Summarize the chunk `Shared::observed` holds for this column.
    Observe((i32, i32)),
    RequestDetail {
        chunks: Vec<(i32, i32)>,
        center: [i32; 2],
    },
    Select(Box<Selection>),
}
enum Done {
    Detail((i32, i32), ReducedColumns),
    /// A visited chunk's summary (`Job::Observe`).
    Observed((i32, i32), ReducedColumns),
    DetailRequested,
    Sample {
        key: TileKey,
        samples: Vec<Column>,
    },
    /// The selected tiles (`active`), those they replace and the new meshes.
    Mesh {
        active: Arc<FxHashSet<TileKey>>,
        removed: Vec<TileKey>,
        meshes: Vec<(TileKey, TileMesh)>,
        covered: FxHashSet<(i32, i32)>,
        /// `fallback_children` of `active` under this plan.
        fallback: (Arc<Plan>, Vec<TileKey>),
    },
    /// A selection that changed nothing.
    Unchanged,
}
struct Shared {
    jobs: Mutex<VecDeque<Job>>,
    /// The latest chunk to summarize per column, queued as `Job::Observe`
    /// once: edits made before the worker gets to it are summarized once.
    observed: Mutex<FxHashMap<(i32, i32), Arc<Chunk>>>,
    /// `state_kinds`, from the first observed chunk's facts.
    kinds: std::sync::OnceLock<Vec<u8>>,
    wake: Condvar,
    stop: AtomicBool,
}
/// One background worker and at most two outstanding sample jobs. Only the
/// current view's quadtree and a bounded set of visited summaries are retained.
pub struct Lod {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
    results: mpsc::Receiver<Done>,
    #[cfg(test)]
    context: Arc<WorkerContext>,
    meshing: bool,
    /// Something `select` reads changed since it last ran.
    reselect: bool,
    /// The plan or the covered columns changed: covered leaves need stand-ins.
    replanned: bool,
    invalidated: FxHashSet<(i32, i32)>,
    /// The area last sent to the plan's thread.
    area: Option<([i32; 2], i32, u32, i32, bool)>,
    /// The latest plan from its thread (`Replan`).
    plan: Arc<Plan>,
    plan_requests: Option<mpsc::Sender<PlanRequest>>,
    plans: mpsc::Receiver<Replan>,
    plan_thread: Option<thread::JoinHandle<()>>,
    /// The plan's leaves without samples.
    missing: Missing,
    /// `fallback_children` and the plan and active tiles they are of.
    fallback: Option<(Arc<Plan>, Arc<FxHashSet<TileKey>>, Vec<TileKey>)>,
    /// Reduced far detail (`set_reduced_detail`).
    reduced_detail: bool,
    pending: FxHashSet<TileKey>,
    sample_nearby: bool,
    detail_masks: FxHashMap<TileKey, [u64; 4]>,
    detail_request_pending: bool,
    detail_request_at: std::time::Instant,
    /// Four threads or fewer: far work waits for the near terrain (`hold`).
    small: bool,
    /// The near terrain is loading (`set_near_loading`).
    near_loading: bool,
    /// When `request_samples` last started a job while held.
    held_sample_at: std::time::Instant,
    /// When `select` last walked the quadtree (`SELECT_STEP`).
    selected_at: std::time::Instant,
    /// Each tile's samples, shared with mesh jobs in flight. Changed through
    /// `insert_ready`, which keeps `missing`.
    ready: Arc<Ready>,
    active: Arc<FxHashSet<TileKey>>,
    dirty: FxHashSet<TileKey>,
    removed: Vec<TileKey>,
    covered: Arc<FxHashSet<(i32, i32)>>,
    covered_bounds: Option<[i32; 4]>,
    /// The covered columns the active meshes leave out, when a mesh job
    /// has finished since the last `take_cutout`.
    cutout: Option<FxHashSet<(i32, i32)>>,
    saved: Arc<Saved>,
    reduced: Arc<Reduced>,
    saved_order: VecDeque<(i32, i32)>,
    /// Mesh jobs finished: the drawn tiles changed (`revision`).
    revision: u64,
}
impl Lod {
    pub fn new(source: Arc<dyn LodSource>, context: Arc<WorkerContext>) -> Self {
        let shared = Arc::new(Shared {
            jobs: Mutex::new(VecDeque::new()),
            observed: Mutex::new(FxHashMap::default()),
            kinds: std::sync::OnceLock::new(),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let worker = shared.clone();
        let mesh_context = context.clone();
        let (tx, results) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("Far terrain".into())
            .spawn(move || {
                let mut sampler = source.sampler();
                let count = thread::available_parallelism()
                    .map_or(1, |n| n.get() / 4)
                    .clamp(1, 3);
                let mut sampling = Vec::new();
                for _ in 0..count {
                    let Some(mut sample) = sampler.fork() else {
                        break;
                    };
                    let shared = worker.clone();
                    let tx = tx.clone();
                    sampling.push(
                        thread::Builder::new()
                            .name("Far sampling".into())
                            .spawn(move || {
                                loop {
                                    let key = {
                                        let mut jobs = shared.jobs.lock().unwrap();
                                        loop {
                                            if shared.stop.load(Ordering::Relaxed) {
                                                return;
                                            }
                                            if let Some(index) = jobs
                                                .iter()
                                                .position(|job| matches!(job, Job::Sample(_)))
                                            {
                                                let Some(Job::Sample(key)) = jobs.remove(index)
                                                else {
                                                    unreachable!()
                                                };
                                                break key;
                                            }
                                            jobs = shared.wake.wait(jobs).unwrap();
                                        }
                                    };
                                    let [x, _, z] = key.origin();
                                    let samples = sample.grid(x, z, key.stride(), key.cells());
                                    if shared.stop.load(Ordering::Relaxed) {
                                        return;
                                    }
                                    if tx.send(Done::Sample { key, samples }).is_err() {
                                        return;
                                    }
                                }
                            })
                            .expect("far sampling worker"),
                    );
                }
                let mut mesher = mesh::Mesher::default();
                'work: loop {
                    for (key, columns) in sampler.drain_details() {
                        if tx
                            .send(Done::Detail(key, ReducedColumns::new(columns)))
                            .is_err()
                        {
                            break 'work;
                        }
                    }
                    let job = {
                        let mut jobs = worker.jobs.lock().unwrap();
                        if (jobs.is_empty()
                            || (!sampling.is_empty()
                                && jobs.iter().all(|job| matches!(job, Job::Sample(_)))))
                            && !worker.stop.load(Ordering::Relaxed)
                        {
                            jobs = worker
                                .wake
                                .wait_timeout(jobs, std::time::Duration::from_millis(100))
                                .unwrap()
                                .0;
                        }
                        if worker.stop.load(Ordering::Relaxed) {
                            break;
                        }
                        if sampling.is_empty() {
                            jobs.pop_front()
                        } else {
                            jobs.iter()
                                .position(|job| !matches!(job, Job::Sample(_)))
                                .and_then(|index| jobs.remove(index))
                        }
                    };
                    let Some(job) = job else { continue };
                    let done = match job {
                        Job::Observe(key) => {
                            let chunk = worker.observed.lock().unwrap().remove(&key);
                            let (Some(chunk), Some(kinds)) = (chunk, worker.kinds.get()) else {
                                continue;
                            };
                            let columns = summarize(&chunk, kinds);
                            sampler.remember(key, &columns);
                            Done::Observed(key, ReducedColumns::new(columns))
                        }
                        Job::RequestDetail { chunks, center } => {
                            sampler.request_detail(&chunks, center);
                            Done::DetailRequested
                        }
                        Job::Sample(key) => {
                            let [x, _, z] = key.origin();
                            let samples = sampler.grid(x, z, key.stride(), key.cells());
                            Done::Sample { key, samples }
                        }
                        Job::Select(selection) => {
                            let Some(done) =
                                selection.run(&mut mesher, &mesh_context, &worker.stop)
                            else {
                                break 'work;
                            };
                            done
                        }
                    };
                    if tx.send(done).is_err() {
                        break;
                    }
                }
                worker.stop.store(true, Ordering::Relaxed);
                worker.wake.notify_all();
                for handle in sampling {
                    let _ = handle.join();
                }
            })
            .expect("far terrain worker");
        // The plan's own thread: a mesh job can hold the worker for tens of milliseconds, and
        // a chunk crossing's plan should not wait for it. Every plan is built from the one
        // before it, in order (`Plan::with_previous` keeps detail across its thresholds).
        let (plan_requests, requests) = mpsc::channel::<PlanRequest>();
        let (replans, plans) = mpsc::channel();
        let plan_thread = thread::Builder::new()
            .name("Far plan".into())
            .spawn(move || {
                let mut previous = Arc::new(Plan::default());
                while let Ok(r) = requests.recv() {
                    let plan = Arc::new(Plan::with_previous(
                        r.camera,
                        r.detail,
                        r.distance,
                        &previous,
                        r.elevation,
                        r.reduced,
                    ));
                    let change = PlanChange::between(&previous, &plan);
                    previous = plan.clone();
                    if replans.send(Replan { plan, change }).is_err() {
                        return;
                    }
                }
            })
            .expect("far plan thread");
        Self {
            shared,
            thread: Some(handle),
            results,
            #[cfg(test)]
            context,
            meshing: false,
            reselect: true,
            replanned: true,
            invalidated: FxHashSet::default(),
            area: None,
            reduced_detail: false,
            plan: Arc::default(),
            plan_requests: Some(plan_requests),
            plans,
            plan_thread: Some(plan_thread),
            missing: Missing::default(),
            fallback: None,
            pending: FxHashSet::default(),
            sample_nearby: true,
            detail_masks: FxHashMap::default(),
            detail_request_pending: false,
            detail_request_at: std::time::Instant::now(),
            small: thread::available_parallelism().map_or(1, |n| n.get()) <= 4,
            near_loading: false,
            held_sample_at: std::time::Instant::now(),
            selected_at: std::time::Instant::now(),
            ready: Arc::default(),
            active: Arc::default(),
            dirty: FxHashSet::default(),
            removed: Vec::new(),
            covered: Arc::default(),
            covered_bounds: None,
            cutout: None,
            saved: Arc::default(),
            reduced: Arc::default(),
            saved_order: VecDeque::new(),
            revision: 0,
        }
    }
    /// The solid ground at block column (x, z): its top face's height and
    /// its block, from the finest tile sampled there. For aiming past the
    /// resident chunks (`game.raycast`); water is passed, as the view's ray
    /// passes it.
    pub fn ground(&self, x: i32, z: i32) -> Option<(i32, BlockStateId)> {
        self.surface(x, z).map(|s| (s.height, s.state))
    }
    /// The surface at block column (x, z), its water included, from the finest tile sampled
    /// there (`None` where none is).
    pub fn surface(&self, x: i32, z: i32) -> Option<Surface> {
        (0..=MAX_LEVEL).find_map(|level| {
            let size = 16 << level;
            let key = TileKey {
                x: x.div_euclid(size),
                z: z.div_euclid(size),
                level,
            };
            let cells = key.cells();
            let columns = self.ready.get(&key).filter(|c| c.len() == cells * cells)?;
            let cell = key.stride();
            let (cx, cz) = ((x - key.x * size) / cell, (z - key.z * size) / cell);
            columns[cz as usize * cells + cx as usize].surface
        })
    }
    /// Changes whenever the drawn tiles do (a mesh job finished).
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Each drawn tile and its cells' columns, for the rays' far field (gfx/heightmap.rs).
    pub fn drawn_columns(&self) -> impl Iterator<Item = (TileKey, &[Column])> {
        self.active.iter().filter_map(|&key| {
            let columns = self.ready.get(&key)?;
            (columns.len() == key.cells() * key.cells()).then_some((key, columns.as_slice()))
        })
    }
    pub fn stats(&self) -> (usize, usize) {
        (self.active.len(), self.ready.len())
    }
    pub fn progress(&self) -> (usize, usize) {
        let mut desired = 0;
        let mut sampled = 0;
        for key in &self.plan.nodes {
            if !self.plan.split.contains(key) {
                desired += 1;
                sampled += usize::from(self.ready.contains_key(key));
            }
        }
        (desired, sampled)
    }
    pub fn refined_samples(&self) -> usize {
        self.active
            .iter()
            .filter_map(|key| self.detail_masks.get(key))
            .flat_map(|mask| mask.iter())
            .map(|bits| bits.count_ones() as usize)
            .sum()
    }
    pub fn removed(&mut self) -> Vec<TileKey> {
        std::mem::take(&mut self.removed)
    }
    /// The full-detail columns the active meshes leave out, once per finished
    /// mesh job. Until a column leaves this set, nothing distant stands in for
    /// it: its full-detail terrain must still be drawn, in or past the render
    /// distance, or a hole opens there.
    pub fn take_cutout(&mut self) -> Option<FxHashSet<(i32, i32)>> {
        self.cutout.take()
    }
    pub fn invalidate_chunk(&mut self, x: i32, z: i32) {
        if self.meshing {
            self.invalidated.insert((x, z));
        }
        // The tiles touching the chunk: a few per level, not every active one.
        for level in 0..=MAX_LEVEL {
            let size = 16 << level;
            for kz in (z * 16).div_euclid(size) - 1..=((z + 1) * 16).div_euclid(size) {
                for kx in (x * 16).div_euclid(size) - 1..=((x + 1) * 16).div_euclid(size) {
                    let key = TileKey {
                        x: kx,
                        z: kz,
                        level,
                    };
                    let [ox, _, oz] = key.origin();
                    if x * 16 <= ox + size
                        && (x + 1) * 16 >= ox
                        && z * 16 <= oz + size
                        && (z + 1) * 16 >= oz
                        && self.active.contains(&key)
                    {
                        self.dirty.insert(key);
                    }
                }
            }
        }
    }
    /// Capture actual block columns, including canopy gaps and material
    /// layers: the worker summarizes the chunk (`summarize`, a few
    /// milliseconds a chunk), then `finish_observed` keeps it.
    pub fn observe(&mut self, chunk: Arc<Chunk>, facts: &BlockFacts) {
        self.shared.kinds.get_or_init(|| state_kinds(facts));
        let key = (chunk.x, chunk.z);
        if self
            .shared
            .observed
            .lock()
            .unwrap()
            .insert(key, chunk)
            .is_none()
        {
            self.shared
                .jobs
                .lock()
                .unwrap()
                .push_back(Job::Observe(key));
            self.shared.wake.notify_all();
        }
    }
    /// Chunks waiting for the worker to summarize them. A resident chunk
    /// held here is copied by its next edit (`Arc::make_mut`), so callers
    /// hand over a few at a time (`OBSERVE_AHEAD`).
    pub fn observing(&self) -> usize {
        self.shared.observed.lock().unwrap().len()
    }
    /// Keeps a visited chunk's summary, unless it is unchanged.
    fn finish_observed(&mut self, key: (i32, i32), columns: ReducedColumns) {
        if self
            .saved
            .get(&key)
            .is_some_and(|saved| **saved == *columns.columns)
        {
            return;
        }
        // Copied here only if a mesh job still reads them (`Arc::make_mut`): once a job at most.
        let saved = Arc::make_mut(&mut self.saved);
        if !saved.contains_key(&key) {
            while saved.len() >= MAX_SAVED {
                if let Some(old) = self.saved_order.pop_front() {
                    saved.remove(&old);
                    Arc::make_mut(&mut self.reduced).remove(&old);
                }
            }
            self.saved_order.push_back(key);
        }
        saved.insert(key, columns.columns.clone());
        self.finish_detail(key, columns);
    }
    /// Sets a tile's samples; the only way `ready` gains one, so `missing` stays right.
    fn insert_ready(&mut self, key: TileKey, samples: Arc<Vec<Column>>) {
        Arc::make_mut(&mut self.ready).insert(key, samples);
        self.missing.0.remove(&key);
    }
    fn apply_fine(&mut self, (cx, cz): (i32, i32), columns: &[Column; 256]) {
        let fine = TileKey {
            x: cx,
            z: cz,
            level: 0,
        };
        if self.plan.nodes.contains(&fine) || self.ready.contains_key(&fine) {
            self.insert_ready(fine, Arc::new(columns.to_vec()));
            self.reselect = true;
            self.detail_masks.insert(fine, [u64::MAX; 4]);
        }
    }
    fn apply_detail(&mut self, (cx, cz): (i32, i32), columns: &ReducedColumns) {
        self.apply_fine((cx, cz), &columns.columns);
        // A single generated chunk cannot describe a larger cell's area
        // (strides over 16 blocks, levels over 4); at each level up to that,
        // only the tile holding the chunk overlaps it.
        for level in 0..=4u8 {
            let size = 16 << level;
            let key = TileKey {
                x: (cx * 16).div_euclid(size),
                z: (cz * 16).div_euclid(size),
                level,
            };
            if !self.ready.contains_key(&key) {
                continue;
            }
            let Some(samples) = Arc::make_mut(&mut self.ready).get_mut(&key) else {
                continue;
            };
            let [ox, _, oz] = key.origin();
            let stride = key.size() / 16;
            let x0 = ((cx * 16 - ox + stride - 1).div_euclid(stride)).clamp(0, 16);
            let x1 = (((cx + 1) * 16 - ox + stride - 1).div_euclid(stride)).clamp(0, 16);
            let z0 = ((cz * 16 - oz + stride - 1).div_euclid(stride)).clamp(0, 16);
            let z1 = (((cz + 1) * 16 - oz + stride - 1).div_euclid(stride)).clamp(0, 16);
            if x0 == x1 || z0 == z1 {
                continue;
            }
            let samples = Arc::make_mut(samples);
            let mask = self.detail_masks.entry(key).or_default();
            for z in z0..z1 {
                for x in x0..x1 {
                    let index = (z * 16 + x) as usize;
                    samples[index] = columns
                        .get(
                            (ox + x * stride).rem_euclid(16) as usize,
                            (oz + z * stride).rem_euclid(16) as usize,
                            stride as usize,
                        )
                        .clone();
                    mask[index / 64] |= 1 << (index % 64);
                }
            }
        }
    }
    fn finish_sample(&mut self, key: TileKey, samples: Vec<Column>) {
        self.pending.remove(&key);
        if self.plan.nodes.contains(&key) && samples.len() == key.cells() * key.cells() {
            let mut samples = samples;
            if let (Some(old), Some(mask)) = (self.ready.get(&key), self.detail_masks.get(&key)) {
                for i in 0..256 {
                    if mask[i / 64] & (1 << (i % 64)) != 0 {
                        samples[i] = old[i].clone();
                    }
                }
            }
            self.insert_ready(key, Arc::new(samples));
            self.reselect = true;
        }
    }
    fn finish_detail(&mut self, key: (i32, i32), columns: ReducedColumns) {
        if let Some(visited) = self.saved.get(&key).cloned()
            && !Arc::ptr_eq(&visited, &columns.columns)
        {
            // Its latest reduction is queued; never restore old edits or seed data.
            self.apply_fine(key, &visited);
            return;
        }
        self.apply_detail(key, &columns);
        if self.saved.contains_key(&key) {
            Arc::make_mut(&mut self.reduced).insert(key, Arc::new(columns));
        }
        self.invalidate_chunk(key.0, key.1);
    }
    /// Whether the near terrain is still loading: on small machines far
    /// terrain then holds back (`HELD_SAMPLE_STEP`).
    pub fn set_near_loading(&mut self, loading: bool) {
        self.near_loading = loading;
    }
    /// Reduced far detail (`graphics::FarDetail::Reduced`): the next update
    /// replans when it changes.
    pub fn set_reduced_detail(&mut self, reduced: bool) {
        self.reduced_detail = reduced;
    }
    fn held(&self) -> bool {
        self.small && self.near_loading
    }
    fn request_detail(&mut self, camera: [i32; 2]) {
        if self.held()
            || self.detail_request_pending
            || self.detail_request_at.elapsed() < std::time::Duration::from_millis(250)
        {
            return;
        }
        self.detail_request_at = std::time::Instant::now();
        let mut keys: Vec<_> = self
            .active
            .iter()
            .filter(|key| {
                key.level <= 1
                    && !self.plan.split.contains(key)
                    && !(key.level == 0 && self.covered.contains(&(key.x, key.z)))
                    && self.detail_masks.get(key) != Some(&[u64::MAX; 4])
            })
            .copied()
            .collect();
        keys.sort_unstable_by_key(|key| (key.distance(camera), *key));
        let mut chunks = FxHashSet::default();
        for key in keys {
            let [ox, _, oz] = key.origin();
            let stride = key.size() / 16;
            let mask = self.detail_masks.get(&key).copied().unwrap_or_default();
            for index in 0..256 {
                if mask[index / 64] & (1 << (index % 64)) == 0 {
                    let chunk = (
                        (ox + (index as i32 % 16) * stride).div_euclid(16),
                        (oz + (index as i32 / 16) * stride).div_euclid(16),
                    );
                    if self.covered.contains(&chunk) {
                        self.detail_masks.entry(key).or_default()[index / 64] |= 1 << (index % 64);
                    } else {
                        chunks.insert(chunk);
                    }
                }
            }
            if chunks.len() >= 32 {
                break;
            }
        }
        let mut chunks: Vec<_> = chunks.into_iter().collect();
        chunks.sort_unstable_by_key(|&(x, z)| {
            let dx = i64::from(x * 16 + 8 - camera[0]);
            let dz = i64::from(z * 16 + 8 - camera[1]);
            dx * dx + dz * dz
        });
        chunks.truncate(32);
        if !chunks.is_empty() {
            self.detail_request_pending = true;
            self.shared
                .jobs
                .lock()
                .unwrap()
                .push_back(Job::RequestDetail {
                    chunks,
                    center: camera,
                });
            self.shared.wake.notify_all();
        }
    }
    pub fn update(
        &mut self,
        camera: [f64; 3],
        yaw: f32,
        detail: i32,
        distance: u32,
        covered: Covered,
    ) -> Vec<(TileKey, TileMesh)> {
        let elevation =
            (((camera[1] - f64::from(MIN_Y + crate::chunk::HEIGHT)).clamp(0., 16384.)) as i32 / 32)
                * 32;
        let camera = [camera[0].floor() as i32, camera[2].floor() as i32];
        // Fill the view ahead first; a nearest-only ring spends most work out
        // of sight while visible forests remain undecorated noise terrain.
        let ahead = (detail * 16 + 128).max(elevation * 2).min(4096) as f32;
        let yaw = yaw.to_radians();
        let focus = [
            camera[0] - (yaw.sin() * ahead) as i32,
            camera[1] + (yaw.cos() * ahead) as i32,
        ];
        // Replan at chunk boundaries, not on every camera movement: the plan's thread
        // builds it, and it is taken in (`replan`) a frame or so later.
        let area = (
            [camera[0].div_euclid(16), camera[1].div_euclid(16)],
            detail,
            distance,
            elevation,
            self.reduced_detail,
        );
        if self.area != Some(area) {
            self.area = Some(area);
            if let Some(requests) = &self.plan_requests {
                let _ = requests.send(PlanRequest {
                    camera,
                    detail,
                    distance,
                    elevation,
                    reduced: self.reduced_detail,
                });
            }
        }
        let replan = crate::spans::time("lod_plan", || {
            let mut replan = false;
            while let Ok(next) = self.plans.try_recv() {
                self.replan(next);
                replan = true;
            }
            replan
        });
        let mut meshes = Vec::new();
        crate::spans::time("lod_results", || self.receive(&mut meshes));
        crate::spans::time("lod_covered", || self.cover(covered));
        if !self.meshing && (replan || self.selected_at.elapsed() >= SELECT_STEP) {
            crate::spans::time("lod_select", || self.select());
        }
        crate::spans::time("lod_request", || self.request_detail(focus));
        crate::spans::time("lod_wanted", || {
            self.request_samples(camera, focus, yaw, detail, elevation)
        });
        meshes
    }
    /// Takes in a plan from the plan's thread: what depends on the plan changes by the tiles
    /// that changed (`PlanChange`), not by a pass over every node and sample.
    fn replan(&mut self, Replan { plan, change }: Replan) {
        self.plan = plan;
        // Every sample is of a node of the plan (`finish_sample`, `apply_fine`, `cover`).
        if change.gone.iter().any(|key| self.ready.contains_key(key)) {
            let ready = Arc::make_mut(&mut self.ready);
            for key in &change.gone {
                ready.remove(key);
            }
        }
        // Not only nodes: `request_detail` marks the active tiles, of an older plan.
        self.detail_masks
            .retain(|key, _| self.plan.nodes.contains(key));
        self.missing.replan(&change, &self.ready);
        // Complete resident columns need no duplicate noise work.
        for &key in &change.leaves {
            if key.level == 0
                && self.covered.contains(&(key.x, key.z))
                && !self.ready.contains_key(&key)
            {
                self.insert_ready(key, Arc::new(vec![Column::default(); CELLS * CELLS]));
            }
        }
        self.reselect = true;
    }
    fn receive(&mut self, meshes: &mut Vec<(TileKey, TileMesh)>) {
        while let Ok(done) = self.results.try_recv() {
            match done {
                Done::Detail(key, columns) => {
                    crate::spans::time("lod_detail", || self.finish_detail(key, columns))
                }
                Done::Observed(key, columns) => {
                    crate::spans::time("lod_detail", || self.finish_observed(key, columns))
                }
                Done::DetailRequested => self.detail_request_pending = false,
                Done::Sample { key, samples } => self.finish_sample(key, samples),
                Done::Mesh {
                    active,
                    removed,
                    meshes: completed,
                    covered,
                    fallback: (plan, fallback),
                } => {
                    self.cutout = Some(covered);
                    // Retire parents only when the entire replacement is ready.
                    self.removed.extend(removed);
                    if Arc::ptr_eq(&plan, &self.plan) {
                        self.fallback = Some((plan, active.clone(), fallback));
                    }
                    self.active = active;
                    self.revision += 1;
                    self.meshing = false;
                    self.reselect = true;
                    for (x, z) in std::mem::take(&mut self.invalidated) {
                        self.invalidate_chunk(x, z);
                    }
                    meshes.extend(completed);
                }
                Done::Unchanged => {
                    self.meshing = false;
                    for (x, z) in std::mem::take(&mut self.invalidated) {
                        self.invalidate_chunk(x, z);
                    }
                }
            }
        }
    }
    /// Takes in the full-detail columns that changed. In flight a column completes or leaves
    /// nearly every frame: the set is changed by those columns, not rebuilt and compared.
    fn cover(&mut self, covered: Covered) {
        let changes = match covered {
            Covered::Same => Vec::new(),
            Covered::All(all) if all == *self.covered => Vec::new(),
            Covered::All(all) => self
                .covered
                .symmetric_difference(&all)
                .map(|&column| (column, all.contains(&column)))
                .collect(),
            Covered::Changed(changes) => changes,
        };
        let mut shrunk = false;
        for (column, now) in changes {
            if self.covered.contains(&column) == now {
                continue;
            }
            // Copied only if the mesh job in flight still reads it.
            let set = Arc::make_mut(&mut self.covered);
            if now {
                set.insert(column);
                self.covered_bounds = Some(extend_bounds(self.covered_bounds, column));
            } else {
                set.remove(&column);
                shrunk |= on_bounds_edge(self.covered_bounds, column);
            }
            self.invalidate_chunk(column.0, column.1);
            // Complete resident columns need no duplicate noise work. Their real
            // summaries are retained when the full-detail chunks leave the view.
            let key = TileKey {
                x: column.0,
                z: column.1,
                level: 0,
            };
            if now && self.plan.nodes.contains(&key) && !self.ready.contains_key(&key) {
                self.insert_ready(key, Arc::new(vec![Column::default(); CELLS * CELLS]));
            }
            self.reselect = true;
        }
        if shrunk {
            self.covered_bounds = covered_bounds(&self.covered);
        }
        if std::mem::take(&mut self.replanned) {
            let fine: Vec<_> = self
                .plan
                .nodes
                .iter()
                .copied()
                .filter(|key| {
                    key.level == 0
                        && self.covered.contains(&(key.x, key.z))
                        && !self.ready.contains_key(key)
                })
                .collect();
            for key in fine {
                self.insert_ready(key, Arc::new(vec![Column::default(); CELLS * CELLS]));
            }
            self.reselect = true;
        }
    }
    /// Chooses the tiles to draw and meshes those that changed, when the
    /// plan, the samples, the covered columns or the active tiles changed: both on the
    /// worker (`Selection`), which answers `Done::Unchanged` when nothing did.
    fn select(&mut self) {
        if !std::mem::take(&mut self.reselect) && self.dirty.is_empty() {
            return;
        }
        self.selected_at = std::time::Instant::now();
        self.meshing = true;
        self.shared
            .jobs
            .lock()
            .unwrap()
            .push_front(Job::Select(Box::new(Selection {
                plan: self.plan.clone(),
                ready: self.ready.clone(),
                active: self.active.clone(),
                dirty: std::mem::take(&mut self.dirty),
                covered: self.covered.clone(),
                bounds: self.covered_bounds,
                saved: self.saved.clone(),
                reduced: self.reduced.clone(),
            })));
        self.shared.wake.notify_all();
    }
    fn request_samples(
        &mut self,
        camera: [i32; 2],
        focus: [i32; 2],
        yaw: f32,
        detail: i32,
        elevation: i32,
    ) {
        // Every sample slot busy: no tile can be requested this frame.
        let free = if self.held() {
            usize::from(
                self.pending.is_empty() && self.held_sample_at.elapsed() >= HELD_SAMPLE_STEP,
            )
        } else {
            SAMPLE_JOBS.saturating_sub(self.pending.len())
        };
        if free == 0 {
            return;
        }
        if self.held() {
            self.held_sample_at = std::time::Instant::now();
        }
        // Load desired nearby leaves directly, alongside broad fallback coverage.
        // Waiting for every ancestor/sibling made full detail take minutes to appear.
        let mut wanted = Vec::new();
        for &root in &self.plan.roots {
            if self.plan.nodes.contains(&root) && !self.ready.contains_key(&root) {
                wanted.push((0, 0, root));
            }
        }
        // The leaves without samples, the same as a pass over the plan's nodes finds.
        for &key in &self.missing.0 {
            wanted.push((
                if key.in_view(camera, yaw) { 1 } else { 3 },
                i64::from(key.distance(focus)) * 4 + i64::from(key.distance(camera)),
                key,
            ));
        }
        let current = self.fallback.as_ref().is_some_and(|(plan, active, _)| {
            Arc::ptr_eq(plan, &self.plan) && Arc::ptr_eq(active, &self.active)
        });
        if !current {
            self.fallback = Some((
                self.plan.clone(),
                self.active.clone(),
                fallback_children(&self.plan, &self.active),
            ));
        }
        let children = self.fallback.as_ref().map_or(&[][..], |(_, _, c)| c);
        let fallback = children
            .iter()
            .filter(|child| !self.ready.contains_key(child) && !self.pending.contains(child))
            .map(|&child| {
                // Covered near terrain bounds projected error. Using
                // zero distance for a parent touching the camera kept
                // refining it while huge visible walls waited elsewhere.
                let priority = i64::from(child.distance(camera).max((detail * 16).max(elevation)))
                    * 1024
                    / i64::from(child.size());
                (
                    if child.in_view(camera, yaw) { 2 } else { 4 },
                    priority,
                    child,
                )
            })
            .min();
        // Only the first `free` keys not already pending are taken. A key is
        // listed at most twice (as a root and as a node), so the best
        // `2 * (SAMPLE_JOBS + pending)` entries hold them, in order.
        let best = 2 * (SAMPLE_JOBS + self.pending.len());
        if wanted.len() > best {
            wanted.select_nth_unstable(best);
            wanted.truncate(best);
        }
        wanted.sort_unstable();
        // Alternate one nearby final tile with one broad refinement tile.
        if let Some(fallback) = fallback {
            let index = if self.sample_nearby {
                wanted
                    .iter()
                    .position(|(_, _, k)| !self.pending.contains(k))
                    .map_or(0, |i| i + 1)
            } else {
                0
            };
            wanted.insert(index, fallback);
        }
        let mut unique = FxHashSet::default();
        let wanted: Vec<_> = wanted
            .into_iter()
            .map(|(_, _, key)| key)
            .filter(|key| !self.pending.contains(key) && unique.insert(*key))
            .collect();
        if !wanted.is_empty() {
            let mut jobs = self.shared.jobs.lock().unwrap();
            for key in wanted.into_iter().take(free) {
                self.pending.insert(key);
                self.sample_nearby = !self.sample_nearby;
                jobs.push_back(Job::Sample(key));
            }
            drop(jobs);
            self.shared.wake.notify_all();
        }
    }
    /// The active tiles inside each of `keys` (finer, so each a descendant).
    fn finer(
        active: &FxHashSet<TileKey>,
        keys: &FxHashSet<TileKey>,
    ) -> FxHashMap<TileKey, Vec<TileKey>> {
        let mut finer: FxHashMap<TileKey, Vec<TileKey>> = FxHashMap::default();
        for &tile in active {
            let mut parent = tile;
            while parent.level < MAX_LEVEL {
                parent = parent.parent();
                if keys.contains(&parent) {
                    finer.entry(parent).or_default().push(tile);
                }
            }
        }
        finer
    }
    /// After a test set the plan or the samples directly: `missing` found again.
    #[cfg(test)]
    fn edited(&mut self) {
        self.missing = Missing::new(&self.plan, &self.ready);
    }
    #[cfg(test)]
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            active: self.active.clone(),
            samples: (*self.ready).clone(),
            saved: self.saved.clone(),
            reduced: self.reduced.clone(),
        }
    }
    #[cfg(test)]
    fn mesh(&self, key: TileKey) -> SectionMesh {
        let finer = Self::finer(&self.active, &[key].into_iter().collect())
            .remove(&key)
            .unwrap_or_default();
        let input = self.snapshot().mesh_input(key, finer);
        mesh::Mesher::default().mesh(
            key,
            &input.columns(),
            &self.covered,
            self.covered_bounds,
            &self.context,
            &input.neighbors,
            &input.finer,
        )
    }
}
impl Snapshot {
    fn visited_column(&self, wx: i32, wz: i32, width: i32) -> Option<&Column> {
        let key = (wx.div_euclid(16), wz.div_euclid(16));
        if width == 1 {
            self.saved
                .get(&key)
                .map(|columns| &columns[(wz.rem_euclid(16) * 16 + wx.rem_euclid(16)) as usize])
        } else if width <= 16 {
            self.reduced.get(&key).map(|columns| {
                columns.get(
                    wx.rem_euclid(16) as usize,
                    wz.rem_euclid(16) as usize,
                    width as usize,
                )
            })
        } else {
            None
        }
    }
    fn neighbor(&self, wx: i32, wz: i32) -> Option<(Column, i32)> {
        for level in 0..=MAX_LEVEL {
            let size = 16 << level;
            let key = TileKey {
                x: wx.div_euclid(size),
                z: wz.div_euclid(size),
                level,
            };
            if !self.active.contains(&key) {
                continue;
            }
            let stride = key.stride();
            let [ox, _, oz] = key.origin();
            let x = (wx - ox) / stride;
            let z = (wz - oz) / stride;
            let (sx, sz) = (ox + x * stride, oz + z * stride);
            if let Some(column) = self.visited_column(sx, sz, stride) {
                return Some((column.clone(), stride));
            }
            let cells = key.cells() as i32;
            return self
                .samples
                .get(&key)
                .map(|columns| (columns[(z * cells + x) as usize].clone(), stride));
        }
        None
    }
    fn mesh_input(&self, key: TileKey, finer: Vec<TileKey>) -> MeshInput {
        let (stride, cells) = (key.stride(), key.cells());
        let [ox, _, oz] = key.origin();
        let mut visited = Vec::new();
        for z in 0..cells {
            for x in 0..cells {
                let (wx, wz) = (ox + x as i32 * stride, oz + z as i32 * stride);
                if let Some(column) = self.visited_column(wx, wz, stride) {
                    visited.push((z * cells + x, column.clone()));
                }
            }
        }
        let neighbors = std::array::from_fn(|direction| {
            let mut edge = Vec::new();
            let mut offset = 0;
            while offset < key.size() {
                let (wx, wz) = match direction {
                    0 => (ox - 1, oz + offset),
                    1 => (ox + key.size(), oz + offset),
                    2 => (ox + offset, oz - 1),
                    _ => (ox + offset, oz + key.size()),
                };
                let (column, neighbor_stride) = self
                    .neighbor(wx, wz)
                    .unwrap_or_else(|| (Column::default(), stride));
                let coordinate = if direction < 2 { wz } else { wx };
                let width = (neighbor_stride - coordinate.rem_euclid(neighbor_stride))
                    .min(key.size() - offset);
                edge.push(mesh::EdgeColumn {
                    start: offset,
                    end: offset + width,
                    column,
                });
                offset += width;
            }
            edge
        });
        MeshInput {
            key,
            samples: self.samples[&key].clone(),
            visited,
            neighbors,
            finer,
        }
    }
}
impl Drop for Lod {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // Its channel closed, the plan's thread ends.
        self.plan_requests = None;
        if let Some(thread) = self.plan_thread.take() {
            let _ = thread.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::TerrainVertex;
    #[test]
    fn reduced_far_detail_only_merges_tiles_away_from_the_near_terrain() {
        let leaves = |plan: &Plan| {
            plan.nodes
                .iter()
                .filter(|k| !plan.split.contains(k))
                .count()
        };
        for camera in [[0, 0], [100, -40]] {
            for distance in [64, 128, 512, 1024] {
                let full = Plan::new(camera, 12, distance);
                let reduced = Plan::with_previous(camera, 12, distance, &Plan::default(), 0, true);
                // Never finer than Full anywhere, and as fine next to the
                // full-detail area, whose edge the finest tiles match.
                assert!(reduced.split.is_subset(&full.split));
                assert_eq!(reduced.roots, full.roots);
                for key in reduced.nodes.iter().filter(|k| !reduced.split.contains(k)) {
                    if key.distance(camera) < 12 * 16 + 16 {
                        assert_eq!(key.level, 0, "{key:?}");
                    }
                }
                let (full, reduced) = (leaves(&full), leaves(&reduced));
                assert!(
                    reduced * 2 < full,
                    "{reduced} of {full} tiles at {distance}"
                );
            }
        }
    }
    #[test]
    fn far_plan_is_bounded_and_near_tiles_align_with_chunks() {
        for camera in [[0, 0], [-1, -17], [1023, -4097]] {
            for distance in [64, 256, 1024] {
                let plan = Plan::new(camera, 12, distance);
                assert!(plan.nodes.len() < 12000, "{} nodes", plan.nodes.len());
                assert!(plan.roots.iter().all(|key| key.level <= MAX_LEVEL));
                for key in &plan.nodes {
                    if !plan.split.contains(key) && key.distance(camera) < 12 * 16 {
                        assert_eq!(key.level, 0);
                    }
                }
                let cells = plan
                    .nodes
                    .iter()
                    .filter(|k| !plan.split.contains(k))
                    .count();
                assert!(cells < 10000);
            }
        }
        assert_eq!(
            TileKey {
                x: -1,
                z: -2,
                level: 1
            }
            .origin(),
            [-32, 0, -64]
        );
    }
    #[test]
    fn aerial_distance_avoids_unseen_full_detail_outside_the_near_area() {
        let ground = Plan::new([0, 0], 12, 1024);
        let high = Plan::with_previous([0, 0], 12, 1024, &Plan::default(), 1024, false);
        assert!(high.nodes.len() < ground.nodes.len());
        assert!(
            high.nodes.contains(&TileKey {
                x: 0,
                z: 0,
                level: 0
            }),
            "near full-detail footprint stays exact"
        );
        assert!(
            !high.nodes.contains(&TileKey {
                x: 20,
                z: 0,
                level: 0
            }),
            "far terrain viewed from high above needs distance-appropriate detail"
        );
    }
    #[test]
    fn late_parent_keeps_ready_detail_and_only_fills_missing_children() {
        let parent = TileKey {
            x: -1,
            z: 0,
            level: 1,
        };
        let mut plan = Plan::default();
        plan.roots.push(parent);
        plan.nodes.insert(parent);
        plan.split.insert(parent);
        plan.nodes.extend(parent.children());
        let mut ready: FxHashMap<TileKey, Vec<Column>> = FxHashMap::default();
        ready.insert(parent, vec![]);
        ready.insert(parent.children()[0], vec![]);
        let mut selected = Vec::new();
        ready.remove(&parent);
        assert!(!plan.select(parent, &ready, &mut selected));
        assert_eq!(selected, [parent.children()[0]]);
        ready.insert(parent, vec![]);
        selected.clear();
        assert!(plan.select(parent, &ready, &mut selected));
        assert_eq!(selected, [parent.children()[0], parent]);
        for child in parent.children() {
            ready.insert(child, vec![]);
        }
        selected.clear();
        assert!(plan.select(parent, &ready, &mut selected));
        assert_eq!(selected, parent.children());
    }
    #[test]
    fn detail_survives_movement_across_a_split_threshold() {
        let before = Plan::new([0, 0], 2, 256);
        let after = Plan::with_previous([16, 0], 2, 256, &before, 0, false);
        let mut retained = 0;
        for key in &before.split {
            if key.distance([16, 0]) < key.size() * 10 {
                assert!(after.split.contains(key));
                retained += 1;
            }
        }
        assert!(retained > 100);
        assert!(after.nodes.len() < 16000);
    }
    #[test]
    fn reduced_material_votes_ignore_light_and_retain_exposure() {
        let mut columns: [Column; 256] = std::array::from_fn(|_| Column::default());
        for i in 0..16 {
            columns[(i / 4) * 16 + i % 4].spans.push(Span {
                min: 60,
                max: 64,
                state: BlockStateId(if i < 10 { 2 } else { 1 }),
                biome: 0,
                light: if i < 10 { (i as u16 + 6) << 4 } else { 0 },
            });
        }
        let coarse = Column::reduced(&columns, 0, 0, 4);
        assert_eq!(coarse.spans.len(), 1);
        assert_eq!(
            coarse.spans[0].state,
            BlockStateId(2),
            "majority grass must beat dark dirt"
        );
        assert_eq!(coarse.spans[0].light, 240, "retain sampled sky exposure");
        for column in &mut columns {
            for span in &mut column.spans {
                span.light = 0;
            }
        }
        assert_eq!(
            Column::reduced(&columns, 0, 0, 4).spans[0].light,
            0,
            "unlit caves stay unlit"
        );
        columns[0].spans[0].light = 7 << 12;
        assert_eq!(
            Column::reduced(&columns, 0, 0, 4).spans[0].light,
            7 << 12,
            "block light does not invent skylight"
        );
    }
    #[test]
    fn coarse_voxels_reduce_the_area_instead_of_inflating_one_leaf() {
        let span = |min, max, state| Span {
            min,
            max,
            state: BlockStateId(state),
            biome: 0,
            light: 240,
        };
        let mut columns = std::array::from_fn(|_| Column {
            surface: None,
            spans: vec![span(0, 4, 1)],
        });
        columns[0].spans.push(span(8, 10, 2));
        assert_eq!(Column::reduced(&columns, 0, 0, 4).spans, [span(0, 4, 1)]);
        for z in 0..4 {
            for x in 0..4 {
                columns[z * 16 + x].spans.push(span(12, 14, 3));
            }
        }
        assert_eq!(
            Column::reduced(&columns, 0, 0, 4).spans,
            [span(0, 4, 1), span(12, 14, 3)]
        );
        assert_eq!(Column::reduced(&columns, 0, 0, 1), columns[0]);
        let reduced = ReducedColumns::new(Arc::new(columns));
        assert_eq!(
            reduced.get(0, 0, 1).spans,
            [span(0, 4, 1), span(8, 10, 2), span(12, 14, 3)]
        );
        assert_eq!(reduced.get(0, 0, 4).spans, [span(0, 4, 1), span(12, 14, 3)]);
        assert_eq!(reduced.get(4, 8, 4).spans, [span(0, 4, 1)]);
        assert_eq!(reduced.get(0, 0, 16).spans, [span(0, 4, 1)]);
    }
    #[test]
    #[ignore = "requires prepared models/textures in HORIZON_TEST_RESOURCES"]
    fn real_mesh_clips_near_chunks_and_preserves_water_and_edited_surfaces() -> crate::Result<()> {
        use crate::{
            assets::Assets, biome::Biomes, light::LightTables, mesher::MeshTables,
            testgen::TestTerrain,
        };
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let assets = Arc::new(Assets::load(&root)?);
        let render = Arc::new(assets.reload(&root)?);
        let facts = BlockFacts::load(&root, assets.registry)?;
        let biomes = Arc::new(Biomes::load(&root)?);
        let tables = Arc::new(MeshTables::new(
            &assets,
            &render,
            &facts,
            crate::models::load_visibility(&root, assets.registry)?,
        )?);
        let context = Arc::new(WorkerContext {
            light: Arc::new(LightTables::load(&root, assets.registry, &facts)?),
            tables,
            assets: assets.clone(),
            render,
            biomes: biomes.clone(),
        });
        let source = Arc::new(TestTerrain::new(assets.registry, &biomes)?);
        let mut lod = Lod::new(source, context);
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let key = TileKey {
            x: -1,
            z: -1,
            level: 5,
        };
        Arc::make_mut(&mut lod.ready).insert(
            key,
            Arc::new(vec![
                Column::from_surface(Some(Surface {
                    height: 70,
                    state: stone,
                    water: Some((80, water)),
                    biome: biomes.id("minecraft:plains"),
                }));
                CELLS * CELLS
            ]),
        );
        // A chunk covers only one quarter of this parent's 32² cell. Both
        // opaque terrain and water must leave precisely that 16² footprint.
        Arc::make_mut(&mut lod.covered).insert((-1, -2));
        lod.covered_bounds = Some([-16, -32, 0, -16]);
        let mesh = lod.mesh(key);
        let top_area = |vertices: &[TerrainVertex], y: f32| {
            vertices
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|quad| quad.iter().all(|v| (v.pos[1] - y).abs() < 0.001))
                .map(|quad| {
                    let a = glam::Vec3::from(quad[0].pos);
                    let b = glam::Vec3::from(quad[1].pos);
                    let c = glam::Vec3::from(quad[2].pos);
                    let normal = (b - a).cross(c - a);
                    assert!(normal.y > 0.0, "surface must face the camera from above");
                    let center = (a + c) * 0.5;
                    let [ox, _, oz] = key.origin();
                    let chunk = (
                        ((center.x as i32) + ox).div_euclid(16),
                        ((center.z as i32) + oz).div_euclid(16),
                    );
                    assert!(
                        !lod.covered.contains(&chunk),
                        "LOD overlaps a full-detail chunk"
                    );
                    normal.y
                })
                .sum::<f32>()
        };
        let expected = (key.size() * key.size() - 16 * 16) as f32;
        assert_eq!(top_area(&mesh.solid, 70.0), expected);
        assert_eq!(top_area(&mesh.translucent, 80.0 - 1.0 / 9.0), expected);
        assert!(mesh.translucent.iter().all(|v| v.light == 240));
        let side = |axis: usize, plane: f32, low: f32, high: f32| {
            let other = if axis == 0 { 2 } else { 0 };
            mesh.solid
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|quad| {
                    quad.iter().all(|v| v.pos[axis] == plane)
                        && quad.iter().map(|v| v.pos[1]).fold(f32::INFINITY, f32::min)
                            == MIN_Y as f32
                        && quad
                            .iter()
                            .map(|v| v.pos[1])
                            .fold(f32::NEG_INFINITY, f32::max)
                            == 70.0
                        && quad
                            .iter()
                            .map(|v| v.pos[other])
                            .fold(f32::INFINITY, f32::min)
                            == low
                        && quad
                            .iter()
                            .map(|v| v.pos[other])
                            .fold(f32::NEG_INFINITY, f32::max)
                            == high
                })
                .map(move |quad| {
                    let a = glam::Vec3::from(quad[0].pos);
                    (glam::Vec3::from(quad[1].pos) - a).cross(glam::Vec3::from(quad[2].pos) - a)
                        [axis]
                })
                .collect::<Vec<_>>()
        };
        // The two exposed edges face the missing full-detail chunk and seal
        // the height mismatch all the way down. Shared subpieces add no walls.
        let east_cut = side(0, 496.0, 480.0, 496.0);
        assert_eq!(east_cut.len(), 1);
        assert!(east_cut[0] > 0.0);
        let north_cut = side(2, 496.0, 496.0, 512.0);
        assert_eq!(north_cut.len(), 1);
        assert!(north_cut[0] < 0.0);
        assert!(side(0, 496.0, 496.0, 512.0).is_empty());

        // Block edits do not refresh Chunk.heights; observation must scan the
        // actual blocks, including builds above the old heightmap. The worker
        // summarizes the chunk; its result is kept on receipt.
        let receive_observed = |lod: &mut Lod| {
            let Done::Observed(key, columns) = lod
                .results
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
            else {
                panic!("expected the observed chunk's summary");
            };
            lod.finish_observed(key, columns);
        };
        let mut chunk = Chunk::empty(-1, -1);
        chunk.set(0, 120, 0, stone.0);
        chunk.set(4, 54, 4, stone.0);
        for y in 55..64 {
            chunk.set(4, y, 4, water.0);
        }
        lod.observe(Arc::new(chunk.clone()), &facts);
        receive_observed(&mut lod);
        let observed = &lod.saved[&(-1, -1)];
        assert_eq!(observed[0].surface.unwrap().height, 121);
        assert_eq!(observed[0].surface.unwrap().state, stone);
        assert_eq!(observed[68].surface.unwrap().height, 55);
        assert_eq!(observed[68].surface.unwrap().water, Some((64, water)));

        let fine = TileKey {
            x: -1,
            z: -1,
            level: 0,
        };
        Arc::make_mut(&mut lod.ready).insert(fine, Arc::new(vec![Column::default(); 256]));
        lod.finish_detail(
            (-1, -1),
            ReducedColumns::new(Arc::new(std::array::from_fn(|_| Column::default()))),
        );
        assert_eq!(
            lod.ready[&fine][0].surface.unwrap().height,
            121,
            "late seed generation cannot overwrite a visited build"
        );
        assert_eq!(lod.detail_masks[&fine], [u64::MAX; 4]);
        Arc::make_mut(&mut lod.plan).nodes.insert(fine);
        lod.pending.insert(fine);
        lod.finish_sample(fine, vec![Column::default(); 256]);
        assert_eq!(
            lod.ready[&fine][0].surface.unwrap().height,
            121,
            "a late coarse sampling job cannot erase completed detail"
        );
        assert_eq!(lod.detail_masks[&fine], [u64::MAX; 4]);
        Arc::make_mut(&mut lod.plan).nodes.remove(&fine);

        // The worker's summary comes with the reusable pyramid; a newer
        // observation replaces both, and an unchanged one changes nothing.
        assert!(Arc::ptr_eq(
            &lod.reduced[&(-1, -1)].columns,
            &lod.saved[&(-1, -1)]
        ));
        assert!(
            lod.snapshot()
                .visited_column(-16, -16, 2)
                .unwrap()
                .spans
                .is_empty()
        );
        let old = lod.saved[&(-1, -1)].clone();
        lod.observe(Arc::new(chunk.clone()), &facts);
        receive_observed(&mut lod);
        assert!(Arc::ptr_eq(&lod.saved[&(-1, -1)], &old));
        chunk.set(0, 130, 0, stone.0);
        lod.observe(Arc::new(chunk.clone()), &facts);
        receive_observed(&mut lod);
        assert_eq!(lod.ready[&fine][0].surface.unwrap().height, 131);
        assert!(Arc::ptr_eq(
            &lod.reduced[&(-1, -1)].columns,
            &lod.saved[&(-1, -1)]
        ));
        // A seed reduction arriving after the edit cannot restore old data.
        lod.finish_detail((-1, -1), ReducedColumns::new(old));
        assert_eq!(lod.ready[&fine][0].surface.unwrap().height, 131);

        // Exercise the actual worker/update seam: a parent stays displayed
        // until all four child meshes can replace it in the same update.
        let camera = [-256.0, 100.0, -256.0];
        lod.area = Some(([-16, -16], 2, 64, 0, false));
        Arc::make_mut(&mut lod.plan).roots = vec![key];
        Arc::make_mut(&mut lod.plan).nodes.insert(key);
        // As a replan does (`update` only selects after a change).
        lod.reselect = true;
        let await_meshes = |lod: &mut Lod| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let meshes = lod.update(camera, 0.0, 2, 64, Covered::Same);
                if !meshes.is_empty() {
                    return meshes;
                }
                assert!(std::time::Instant::now() < deadline, "mesh worker stalled");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        let uploaded = await_meshes(&mut lod);
        assert_eq!(uploaded.len(), 1);
        assert_eq!(uploaded[0].0, key);
        assert!(lod.active.contains(&key));
        Arc::make_mut(&mut lod.plan).split.insert(key);
        for child in key.children() {
            Arc::make_mut(&mut lod.plan).nodes.insert(child);
            let samples = lod.ready[&key].clone();
            Arc::make_mut(&mut lod.ready).insert(child, samples);
        }
        lod.reselect = true;
        assert!(lod.update(camera, 0.0, 2, 64, Covered::Same).is_empty());
        assert!(
            lod.active.contains(&key),
            "parent retired before child meshes"
        );
        assert!(lod.removed().is_empty());
        let uploaded = await_meshes(&mut lod);
        assert_eq!(uploaded.len(), 4);
        assert!(
            key.children()
                .iter()
                .all(|child| lod.active.contains(child))
        );
        assert_eq!(lod.removed(), [key]);

        let covered_fine = TileKey {
            x: -1,
            z: -2,
            level: 0,
        };
        let mut plan = Plan::default();
        plan.roots.push(covered_fine);
        plan.nodes.insert(covered_fine);
        lod.plan = Arc::new(plan);
        Arc::make_mut(&mut lod.ready).insert(covered_fine, Arc::new(vec![Column::default(); 256]));
        Arc::make_mut(&mut lod.covered).insert((-1, -2));
        lod.edited();
        lod.reselect = true;
        lod.replanned = true;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            lod.update(camera, 0., 2, 64, Covered::Same);
            if !lod.meshing && lod.active.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "covered normal chunk remained an active LOD mask"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(lod.progress(), (1, 1));

        // Keep one sample outstanding, as on ordinary frames. Completing just
        // one job at a time must still let both loading queues make progress.
        lod.shared.stop.store(true, Ordering::Relaxed);
        lod.shared.wake.notify_all();
        lod.thread.take().unwrap().join().unwrap();
        while lod.results.try_recv().is_ok() {}
        lod.shared.jobs.lock().unwrap().clear();
        let mut plan = Plan::new([-256, -256], 2, 64);
        let parent = TileKey {
            x: -2,
            z: -2,
            level: 3,
        };
        plan.roots = vec![parent];
        lod.plan = Arc::new(plan);
        Arc::make_mut(&mut lod.ready).clear();
        Arc::make_mut(&mut lod.ready).insert(parent, Arc::new(vec![Column::default(); 256]));
        lod.active = Arc::new([parent].into_iter().collect());
        lod.edited();
        lod.pending = (0..SAMPLE_JOBS - 1)
            .map(|i| TileKey {
                x: 999 + i as i32,
                z: 999,
                level: 0,
            })
            .collect();
        lod.sample_nearby = true;
        lod.detail_request_pending = true;
        lod.meshing = true;
        lod.update(camera, 0.0, 2, 64, Covered::All(FxHashSet::default()));
        let Some(Job::Sample(nearby)) = lod.shared.jobs.lock().unwrap().pop_front() else {
            panic!("nearby detail was not scheduled");
        };
        assert_eq!(nearby.level, 0);
        assert!(
            nearby.origin()[2] > -256 + 64,
            "detail should load ahead of the view"
        );
        lod.finish_sample(nearby, vec![Column::default(); 256]);
        lod.update(camera, 0.0, 2, 64, Covered::All(FxHashSet::default()));
        let Some(Job::Sample(fallback)) = lod.shared.jobs.lock().unwrap().pop_front() else {
            panic!("fallback refinement was starved");
        };
        assert!(fallback.level > 0);
        lod.pending.remove(&fallback);
        lod.sample_nearby = true;
        lod.update(camera, 90.0, 2, 64, Covered::All(FxHashSet::default()));
        let Some(Job::Sample(turned)) = lod.shared.jobs.lock().unwrap().pop_front() else {
            panic!("turned view was not scheduled");
        };
        assert!(
            turned.origin()[0] < -256 - 64,
            "loading priority should follow a turn"
        );
        lod.shared.jobs.lock().unwrap().clear();
        let near = TileKey {
            x: -4,
            z: -4,
            level: 2,
        };
        let far = TileKey {
            x: 0,
            z: 0,
            level: 6,
        };
        let mut plan = Plan {
            roots: vec![near, far],
            ..Plan::default()
        };
        Arc::make_mut(&mut lod.ready).clear();
        for parent in [near, far] {
            plan.nodes.insert(parent);
            plan.split.insert(parent);
            plan.nodes.extend(parent.children());
            Arc::make_mut(&mut lod.ready).insert(parent, Arc::new(vec![Column::default(); 256]));
        }
        lod.plan = Arc::new(plan);
        lod.active = Arc::new([near, far].into_iter().collect());
        lod.edited();
        lod.shared.jobs.lock().unwrap().clear();
        lod.pending = (0..SAMPLE_JOBS - 1)
            .map(|i| TileKey {
                x: 999 + i as i32,
                z: 999,
                level: 0,
            })
            .collect();
        lod.sample_nearby = false;
        lod.update(camera, 0., 2, 64, Covered::All(FxHashSet::default()));
        let Some(Job::Sample(broad)) = lod.shared.jobs.lock().unwrap().pop_front() else {
            panic!("projected-error refinement missing")
        };
        assert_eq!(
            broad.level, 5,
            "a near tile touching the camera must not starve a huge visible distant tile"
        );
        lod.shared.jobs.lock().unwrap().clear();
        lod.active = Arc::new((0..40).map(|x| TileKey { x, z: 0, level: 0 }).collect());
        Arc::make_mut(&mut lod.plan).split.clear();
        lod.detail_masks.clear();
        lod.covered = Arc::new([(0, 0)].into_iter().collect());
        lod.detail_request_pending = false;
        lod.detail_request_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
        lod.request_detail([0, 0]);
        let Some(Job::RequestDetail { chunks, .. }) = lod.shared.jobs.lock().unwrap().pop_front()
        else {
            panic!("detail batch was not requested");
        };
        assert_eq!(
            chunks,
            (1..=32).map(|x| (x, 0)).collect::<Vec<_>>(),
            "one unfinished fine tile must not starve neighbouring requests"
        );
        Ok(())
    }
}

#[cfg(test)]
mod touching_tests {
    use super::*;

    #[test]
    fn touching_finds_what_every_pair_would() {
        let mut tiles = FxHashSet::default();
        let mut seed = 12345u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            seed >> 8
        };
        for _ in 0..600 {
            let level = (next() % 5) as u8;
            let span = 64 >> level.min(5);
            tiles.insert(TileKey {
                x: (next() % (span * 2)) as i32 - span as i32,
                z: (next() % (span * 2)) as i32 - span as i32,
                level,
            });
        }
        let changed: Vec<TileKey> = tiles.iter().copied().step_by(37).collect();
        let brute: FxHashSet<TileKey> = tiles
            .iter()
            .copied()
            .filter(|t| changed.iter().any(|c| touches(*t, *c)))
            .collect();
        assert_eq!(touching(&tiles, &changed), brute);
        assert!(!brute.is_empty());
    }
}

#[cfg(test)]
mod incremental_tests {
    use super::*;

    /// A small deterministic generator for the tests' random choices.
    struct Lcg(u32);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            self.0 >> 8
        }
    }

    #[test]
    fn plans_applied_by_their_changes_match_rebuilding_along_a_flight() {
        let mut random = Lcg(7);
        let mut plan = Plan::default();
        let mut ready: FxHashMap<TileKey, ()> = FxHashMap::default();
        let mut missing = Missing::default();
        // Flying east and climbing, a chunk a step, as `Lod::update` replans.
        for step in 0..48 {
            let camera = [step * 16 + 3, -step * 5];
            let elevation = (step / 8) * 32;
            let next = Plan::with_previous(camera, 4, 64, &plan, elevation, false);
            let change = PlanChange::between(&plan, &next);
            // What `Lod::replan` does with the change, against a pass over everything.
            let mut kept = ready.clone();
            kept.retain(|key, _| next.nodes.contains(key));
            for key in &change.gone {
                ready.remove(key);
            }
            assert_eq!(ready, kept, "step {step}");
            missing.replan(&change, &ready);
            plan = next;
            assert_eq!(missing, Missing::new(&plan, &ready), "step {step}");
            // Samples arrive for some of the leaves still missing (`insert_ready`).
            let mut leaves: Vec<_> = missing.0.iter().copied().collect();
            leaves.sort_unstable();
            for key in leaves {
                if random.next().is_multiple_of(3) {
                    ready.insert(key, ());
                    missing.0.remove(&key);
                }
            }
            assert_eq!(missing, Missing::new(&plan, &ready), "step {step}");
        }
        assert!(ready.len() > 100);
    }

    #[test]
    fn fallback_children_are_the_split_active_tiles_children_in_the_plan() {
        let plan = Plan::new([40, -70], 4, 128);
        let active: FxHashSet<_> = plan.nodes.iter().copied().step_by(3).collect();
        let mut brute = Vec::new();
        for &parent in &active {
            if plan.split.contains(&parent) {
                for child in parent.children() {
                    if plan.nodes.contains(&child) {
                        brute.push(child);
                    }
                }
            }
        }
        brute.sort_unstable();
        assert!(!brute.is_empty());
        assert_eq!(fallback_children(&plan, &active), brute);
    }

    #[test]
    fn a_selection_meshes_what_changed_and_nothing_when_nothing_did() {
        let plan = Plan::new([0, 0], 2, 64);
        let ready: FxHashMap<TileKey, ()> = plan.nodes.iter().map(|&k| (k, ())).collect();
        let covered = FxHashSet::default();
        let none = FxHashSet::default();
        let (active, dirty) = choose(&plan, &ready, &covered, &none, FxHashSet::default())
            .expect("a first selection");
        // Every leaf is ready: the leaves are drawn, all of them new.
        let leaves: FxHashSet<_> = plan
            .nodes
            .iter()
            .copied()
            .filter(|k| !plan.split.contains(k))
            .collect();
        assert_eq!(active, leaves);
        assert_eq!(dirty, leaves);
        // The same again: nothing to do, and a dirty tile no longer drawn is dropped.
        let gone = TileKey {
            x: 9999,
            z: 0,
            level: 0,
        };
        assert!(
            choose(
                &plan,
                &ready,
                &covered,
                &active,
                [gone].into_iter().collect()
            )
            .is_none()
        );
        // A dirty drawn tile is meshed alone.
        let tile = *leaves.iter().next().unwrap();
        let (same, dirty) = choose(
            &plan,
            &ready,
            &covered,
            &active,
            [tile].into_iter().collect(),
        )
        .unwrap();
        assert_eq!(
            (same, dirty),
            (active.clone(), [tile].into_iter().collect())
        );
        // A covered finest tile leaves the selection; its neighbours mesh again.
        let fine = TileKey {
            x: 0,
            z: 0,
            level: 0,
        };
        assert!(active.contains(&fine));
        let covered: FxHashSet<_> = [(0, 0)].into_iter().collect();
        let (next, dirty) = choose(&plan, &ready, &covered, &active, FxHashSet::default()).unwrap();
        assert!(!next.contains(&fine));
        assert_eq!(next.len() + 1, active.len());
        assert!(dirty.contains(&TileKey { x: 1, ..fine }));
        assert!(dirty.contains(&TileKey { x: -1, ..fine }));
        assert!(dirty.iter().all(|key| touches(*key, fine)));
    }

    #[test]
    fn covered_bounds_kept_by_change_match_a_full_pass() {
        let mut random = Lcg(99);
        let mut covered = FxHashSet::default();
        let mut bounds = None;
        for _ in 0..2000 {
            let column = (
                (random.next() % 13) as i32 - 6,
                (random.next() % 9) as i32 - 4,
            );
            // As `Lod::cover` does.
            if random.next().is_multiple_of(2) {
                if covered.insert(column) {
                    bounds = Some(extend_bounds(bounds, column));
                }
            } else if covered.remove(&column) && on_bounds_edge(bounds, column) {
                bounds = covered_bounds(&covered);
            }
            assert_eq!(bounds, covered_bounds(&covered));
        }
    }

    #[test]
    fn the_uploads_get_what_the_update_leaves() {
        let ms = std::time::Duration::from_millis;
        assert_eq!(upload_budget(ms(0)), FRAME_BUDGET);
        assert_eq!(
            upload_budget(std::time::Duration::from_micros(300)),
            ms(2) - std::time::Duration::from_micros(300)
        );
        assert_eq!(upload_budget(ms(5)), UPLOAD_FLOOR);
    }

    #[test]
    fn mesh_bounds_cover_both_passes() {
        use crate::mesher::TerrainVertex;
        let vertex = |pos| TerrainVertex::tiled(pos, [0.0; 2], 0, 0, 0);
        let mesh = SectionMesh {
            solid: vec![vertex([3.0, -64.0, 2.0]), vertex([16.0, 70.0, 1.0])],
            translucent: vec![vertex([8.0, 80.0, 9.0])],
            ..Default::default()
        };
        assert_eq!(
            mesh_bounds(&mesh),
            Some((
                glam::Vec3::new(3.0, -64.0, 1.0),
                glam::Vec3::new(16.0, 80.0, 9.0)
            ))
        );
        assert_eq!(mesh_bounds(&SectionMesh::default()), None);
    }
}
