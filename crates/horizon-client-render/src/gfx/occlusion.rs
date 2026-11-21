//! `SectionOcclusionGraph`: which sections the camera can see through open
//! faces. Breadth-first from the camera section through the faces each
//! section's VisGraph connects, never stepping back towards the camera. As in
//! vanilla's `runUpdates`, a section leads on through whatever any of the faces
//! it was entered through connects to, and beyond 60 blocks a step is taken
//! only if the line from the section towards the camera stays in sections the
//! search has reached.
//!
//! As in vanilla, the full search runs off the render thread: a worker keeps
//! its own copy of every section's connectivity and searches over dense
//! arrays around the camera. Frames use the last finished graph and never wait
//! for the next one. A section meshed in between propagates into the graph the
//! frame has (vanilla's partial update), so what it opens shows at once; what
//! it closes goes at the next full search.
//!
//! GPU culling (gfx/gpu_culling.rs) answers as `contains` does from a copy of
//! the graph (`Reach`): the cells it reached, kept as bits too, sent again
//! whenever the generation moves, which it does whenever an answer may change.
use super::SectionKey;
use crate::fast_hash::FxHashSet;
use crate::mesher::ALL_VISIBLE;
use glam::DVec3;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

/// Vanilla's `Direction` order: down, up, north, south, west, east.
const STEPS: [(i32, i32, i32); 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];
/// In `Graph::directions`: the cell has a node.
const REACHED: u8 = 1 << 7;
/// `MINIMUM_ADVANCED_CULLING_SECTION_DISTANCE`: `blockToSectionCoord(60)`.
const DISTANT_SECTIONS: i32 = 60 >> 4;
/// `MINIMUM_ADVANCED_CULLING_DISTANCE` (60 blocks), squared.
const DISTANT_SQR: f64 = 3600.0;
/// `CEILED_SECTION_DIAGONAL`: `ceil(sqrt(3) * 16)`.
const SECTION_DIAGONAL: f64 = 28.0;
/// In `Graph::rays`: the line from this corner stays in reached sections.
const SEES: u8 = u8::MAX;
/// Edits kept before a full resend.
const EDIT_CAP: usize = 1 << 16;

/// VisGraph connectivity per column, one entry per section (bottom first).
/// A missing column or section connects every face.
struct Columns {
    /// The lowest block y of section 0 (`Level.getMinY`).
    min_y: i32,
    sections: i32,
    map: HashMap<(i32, i32), Box<[u64]>>,
}

impl Columns {
    fn new(min_y: i32, sections: i32) -> Self {
        Self {
            min_y,
            sections,
            map: HashMap::new(),
        }
    }
    /// `SectionPos.of`: the section holding `pos`.
    fn section_of(&self, pos: DVec3) -> SectionKey {
        (
            (pos.x.floor() as i32) >> 4,
            (pos.y.floor() as i32 - self.min_y) >> 4,
            (pos.z.floor() as i32) >> 4,
        )
    }
    /// Whether the section's connectivity changed.
    fn set(&mut self, key: SectionKey, vis: u64) -> bool {
        if !(0..self.sections).contains(&key.1) {
            return false;
        }
        let column = match self.map.get_mut(&(key.0, key.2)) {
            Some(column) => column,
            None if vis == ALL_VISIBLE => return false,
            None => self
                .map
                .entry((key.0, key.2))
                .or_insert_with(|| vec![ALL_VISIBLE; self.sections as usize].into()),
        };
        let slot = &mut column[key.1 as usize];
        if *slot == vis {
            return false;
        }
        *slot = vis;
        if vis == ALL_VISIBLE && column.iter().all(|&v| v == ALL_VISIBLE) {
            self.map.remove(&(key.0, key.2));
        }
        true
    }
    fn get(&self, key: SectionKey) -> u64 {
        self.map
            .get(&(key.0, key.2))
            .and_then(|column| column.get(usize::try_from(key.1).ok()?))
            .copied()
            .unwrap_or(ALL_VISIBLE)
    }
    fn apply(&mut self, edit: Edit) {
        match edit {
            Edit::Set(key, vis) => {
                self.set(key, vis);
            }
            Edit::Clear => self.map.clear(),
        }
    }
}

#[derive(Clone, Copy)]
enum Edit {
    Set(SectionKey, u64),
    Clear,
}

/// One search's result over the box around the camera: vanilla's
/// `GraphStorage.sectionToNodeMap`, as dense arrays.
#[derive(Default)]
struct Graph {
    from: SectionKey,
    radius: i32,
    /// The world: lowest block y and height in sections.
    min_y: i32,
    sections: i32,
    /// The searched box: lowest section y and its height in sections.
    y0: i32,
    height: i32,
    /// Per cell: `REACHED`, and the directions travelled from the camera to
    /// it (`Node.directions`).
    directions: Vec<u8>,
    /// Per cell: the faces it was entered through, one bit per face
    /// (`Node.sourceDirections`, as the faces rather than the steps); none for
    /// the camera's.
    entered: Vec<u8>,
    /// `REACHED` of each cell again, a bit each at `index` (32 a word, lowest
    /// bit first): the GPU's copy (`Occlusion::reach`), written with it
    /// (`set_reached`).
    bits: Vec<u32>,
    /// Whether any section in the searched top layer was reached. Sections
    /// above a clamped box use this conservative sky-band answer.
    sky_reached: bool,
    /// Per section corner, during a full search: `SEES`, or how far the line
    /// from it towards the camera is known to stay in reached sections (the
    /// first sample that was not). Sections are only ever added during a
    /// search, so a line that stays in reached sections keeps doing so, and
    /// one that didn't need only be checked again from where it failed.
    rays: Vec<u8>,
}

impl Graph {
    fn index(&self, key: SectionKey) -> Option<usize> {
        let (dx, dz, dy) = (key.0 - self.from.0, key.2 - self.from.2, key.1 - self.y0);
        let side = 2 * self.radius + 1;
        if dx.abs() > self.radius || dz.abs() > self.radius || !(0..self.height).contains(&dy) {
            return None;
        }
        Some((((dz + self.radius) * side + dx + self.radius) * self.height + dy) as usize)
    }
    fn key(&self, index: usize) -> SectionKey {
        let side = 2 * self.radius + 1;
        let i = index as i32;
        let column = i / self.height;
        (
            self.from.0 + column % side - self.radius,
            self.y0 + i % self.height,
            self.from.2 + column / side - self.radius,
        )
    }
    fn contains(&self, key: SectionKey) -> bool {
        if let Some(i) = self.index(key) {
            self.directions[i] & REACHED != 0
        } else if (0..=self.sections).contains(&key.1)
            && key.1 >= self.y0 + self.height
            && (key.0 - self.from.0).abs() <= self.radius
            && (key.2 - self.from.2).abs() <= self.radius
        {
            // Everything above the highest non-air section is open sky. A
            // full search can reach that layer from a neighbouring column,
            // so checking only this column's top cell can hide an upper
            // section that is visible through the open layer. Once any top
            // cell is reached, conservatively keep the whole sky band visible.
            self.sky_reached
        } else {
            false
        }
    }
    /// Cell `i` is reached, having travelled `directions` (`REACHED` among
    /// them): the one writer of `REACHED`, so `bits` say what `directions` do.
    fn set_reached(&mut self, i: usize, directions: u8) {
        debug_assert!(directions & REACHED != 0);
        self.directions[i] = directions;
        self.bits[i / 32] |= 1 << (i % 32);
    }
    /// The full search from the camera (vanilla's `scheduleFullUpdate`).
    fn search(
        &mut self,
        columns: &Columns,
        camera: DVec3,
        radius: i32,
        vis: &mut Vec<u64>,
        queue: &mut VecDeque<u32>,
    ) {
        let from = columns.section_of(camera);
        self.from = from;
        self.radius = radius;
        self.min_y = columns.min_y;
        self.sections = columns.sections;
        self.y0 = -1;
        if from.1 >= columns.sections {
            self.height = columns.sections + 2;
        } else {
            let mut max_non_air_y = 0;
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    let Some(column) = columns.map.get(&(from.0 + dx, from.2 + dz)) else {
                        continue;
                    };
                    for (y, &v) in column.iter().enumerate().rev() {
                        let y = y as i32;
                        if y <= max_non_air_y {
                            break;
                        }
                        if v != ALL_VISIBLE {
                            max_non_air_y = y;
                            break;
                        }
                    }
                }
            }
            let top_y = max_non_air_y.max(from.1);
            self.height = (top_y + 3).max(3).min(columns.sections + 2);
        }
        let side = (2 * radius + 1) as usize;
        let size = side * side * self.height as usize;
        self.directions.clear();
        self.directions.resize(size, 0);
        self.bits.clear();
        self.bits.resize(size.div_ceil(32), 0);
        self.entered.clear();
        self.entered.resize(size, 0);
        self.sky_reached = false;
        self.rays.clear();
        self.rays
            .resize((side + 1) * (side + 1) * (self.height + 1) as usize, 0);
        vis.clear();
        vis.resize(size, ALL_VISIBLE);
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let Some(column) = columns.map.get(&(from.0 + dx, from.2 + dz)) else {
                    continue;
                };
                let base = self
                    .index((from.0 + dx, 0, from.2 + dz))
                    .expect("column inside the searched box");
                let len = (column.len() as i32).min(self.height - 1) as usize;
                vis[base..base + len].copy_from_slice(&column[..len]);
            }
        }
        queue.clear();
        if (0..columns.sections).contains(&from.1) {
            let start = self.index(from).expect("camera inside the searched box");
            self.set_reached(start, REACHED);
            queue.push_back(start as u32);
        } else {
            self.seed_outside(camera, queue);
        }
        self.propagate(queue, camera, true, |i, _| vis[i]);
    }
    /// `initializeQueueForFullUpdate` with the camera above or below the
    /// world (no render section there): every section of the world's top
    /// layer (bottom layer from below) is entered through its outer face, its
    /// travelled directions pointing away from the camera's column, sorted
    /// by the squared distance from the camera's block to the section's
    /// centre (a stable sort over vanilla's x-then-z loop). Vanilla seeds only
    /// columns in view distance; this search covers its whole box, as it does
    /// from inside the world.
    fn seed_outside(&mut self, camera: DVec3, queue: &mut VecDeque<u32>) {
        // STEPS indexes: down, up, north, south, west, east.
        let (y, travel) = if self.from.1 < 0 {
            (0, 1)
        } else {
            (self.sections - 1, 0)
        };
        let block = (camera.x.floor() as i64, camera.z.floor() as i64);
        let mut seeds = Vec::with_capacity(((2 * self.radius + 1) as usize).pow(2));
        for dx in -self.radius..=self.radius {
            for dz in -self.radius..=self.radius {
                let mut directions = REACHED | 1 << travel;
                directions |= match dx.signum() {
                    1 => 1 << 5,
                    -1 => 1 << 4,
                    _ => 0,
                };
                directions |= match dz.signum() {
                    1 => 1 << 3,
                    -1 => 1 << 2,
                    _ => 0,
                };
                // The y offset is the same for every seed.
                let centre = |section: i32| i64::from(section) * 16 + 8;
                let (ox, oz) = (
                    block.0 - centre(self.from.0 + dx),
                    block.1 - centre(self.from.2 + dz),
                );
                seeds.push((ox * ox + oz * oz, dx, dz, directions));
            }
        }
        seeds.sort_by_key(|seed| seed.0);
        for (_, dx, dz, directions) in seeds {
            let i = self
                .index((self.from.0 + dx, y, self.from.2 + dz))
                .expect("seed inside the searched box");
            self.set_reached(i, directions);
            // Entered through the face opposite the travel (`sourceDirection`).
            self.entered[i] = 1 << (travel ^ 1);
            queue.push_back(i as u32);
        }
    }
    /// Vanilla's `runUpdates`: take nodes from the queue and step through
    /// every face their entry faces connect to, away from the camera. A step
    /// into a section already reached adds the face it enters through.
    /// `full`: the search from `camera` (`rays` holds its lines), not a
    /// partial update. Whether it reached a cell it had not.
    fn propagate(
        &mut self,
        queue: &mut VecDeque<u32>,
        camera: DVec3,
        full: bool,
        vis: impl Fn(usize, SectionKey) -> u64,
    ) -> bool {
        let mut grew = false;
        let eye = (
            (camera.x.floor() as i32) >> 4,
            (camera.y.floor() as i32 - self.min_y) >> 4,
            (camera.z.floor() as i32) >> 4,
        );
        // `SectionPos.center` of the camera's section.
        let centre = [eye.0 * 16 + 8, eye.1 * 16 + self.min_y + 8, eye.2 * 16 + 8];
        while let Some(index) = queue.pop_front() {
            let i = index as usize;
            let key = self.key(i);
            if key.1 == self.y0 + self.height - 1 {
                self.sky_reached = true;
            }
            let directions = self.directions[i];
            let entered = self.entered[i];
            let exits = if entered == 0 {
                0x3f
            } else {
                let vis = vis(i, key);
                (0..6)
                    .filter(|face| entered & (1 << face) != 0)
                    .fold(0, |exits, face| exits | (vis >> (face * 6)) as u8 & 0x3f)
            };
            let distant = (key.0 - eye.0).abs() > DISTANT_SECTIONS
                || (key.1 - eye.1).abs() > DISTANT_SECTIONS
                || (key.2 - eye.2).abs() > DISTANT_SECTIONS;
            // The distant test once per axis: both steps along an axis start
            // from the same corner. While the camera is in the section the
            // graph was searched from, every step leads away from it or
            // sideways, so no point of the line falls in a neighbour this loop
            // adds. A partial update after the camera moved tests every step.
            let reuse = eye == self.from;
            let mut seen = [None; 3];
            for (d, step) in STEPS.iter().enumerate() {
                if exits & (1 << d) == 0 || directions & (1 << (d ^ 1)) != 0 {
                    continue;
                }
                let next = (key.0 + step.0, key.1 + step.1, key.2 + step.2);
                if next.1 < -1 || next.1 > self.sections {
                    continue;
                }
                let Some(n) = self.index(next) else { continue };
                if distant {
                    let sees = match seen[d / 2] {
                        Some(sees) if reuse => sees,
                        _ => self.sees_camera(key, d, centre, camera, full),
                    };
                    seen[d / 2] = Some(sees);
                    if !sees {
                        continue;
                    }
                }
                if self.directions[n] & REACHED == 0 {
                    self.set_reached(n, directions | 1 << d);
                    self.entered[n] = 1 << (d ^ 1);
                    queue.push_back(n as u32);
                    grew = true;
                } else {
                    self.entered[n] |= 1 << (d ^ 1);
                }
            }
        }
        grew
    }
    /// Vanilla's distant step test: from the corner of `key` that the step `d`
    /// picks, walk towards the camera in section diagonals while farther than
    /// 60 blocks. Every point inside the world has to fall in a section the
    /// search has reached. Same arithmetic as JOML's `Vector3d` (no FMA).
    /// `full`: remember the answer per corner in `rays`.
    fn sees_camera(
        &mut self,
        key: SectionKey,
        d: usize,
        centre: [i32; 3],
        camera: DVec3,
        full: bool,
    ) -> bool {
        let origin = [key.0 * 16, key.1 * 16 + self.min_y, key.2 * 16];
        // The step's axis in x, y, z order: down/up, north/south, west/east.
        let axis = [1, 2, 0][d / 2];
        let far = |c: usize| {
            i32::from(if c == axis {
                centre[c] > origin[c]
            } else {
                centre[c] < origin[c]
            })
        };
        let corner = (key.0 + far(0), key.1 + far(1), key.2 + far(2));
        let slot = full.then(|| {
            let side = 2 * self.radius + 2;
            let (x, z) = (
                corner.0 - self.from.0 + self.radius,
                corner.2 - self.from.2 + self.radius,
            );
            ((z * side + x) * (self.height + 1) + corner.1 - self.y0) as usize
        });
        let known = slot.map_or(0, |slot| self.rays[slot]);
        if known == SEES {
            return true;
        }
        let failed = self.walk(corner, camera, known);
        if let Some(slot) = slot {
            self.rays[slot] = failed.unwrap_or(SEES);
        }
        failed.is_none()
    }
    /// The line from the section corner `corner` towards the camera; `None`
    /// if every sample is in a reached section, else the first one that is
    /// not (counted from 1). The first `known - 1` samples are known to be
    /// reached: they are stepped over (the same additions, so the same
    /// points) without looking them up.
    fn walk(&self, corner: SectionKey, camera: DVec3, known: u8) -> Option<u8> {
        let mut check = [
            f64::from(corner.0 * 16),
            f64::from(corner.1 * 16 + self.min_y),
            f64::from(corner.2 * 16),
        ];
        let camera = [camera.x, camera.y, camera.z];
        let to = [
            camera[0] - check[0],
            camera[1] - check[1],
            camera[2] - check[2],
        ];
        let inverse = 1.0 / (to[0] * to[0] + (to[1] * to[1] + to[2] * to[2])).sqrt();
        let step = to.map(|t| t * inverse * SECTION_DIAGONAL);
        let (min_y, max_y) = (
            f64::from(self.min_y),
            f64::from(self.min_y + self.sections * 16 - 1),
        );
        let far = |p: &[f64; 3]| {
            let (x, y, z) = (p[0] - camera[0], p[1] - camera[1], p[2] - camera[2]);
            x * x + (y * y + z * z) > DISTANT_SQR
        };
        let mut sample = 0u8;
        while sample + 1 < known {
            for (c, s) in check.iter_mut().zip(step) {
                *c += s;
            }
            sample += 1;
        }
        while far(&check) {
            for (c, s) in check.iter_mut().zip(step) {
                *c += s;
            }
            sample = sample.saturating_add(1);
            if check[1] > max_y || check[1] < min_y {
                break;
            }
            let section = (
                floor(check[0]) >> 4,
                (floor(check[1]) - self.min_y) >> 4,
                floor(check[2]) >> 4,
            );
            if !self.contains(section) {
                // A count too long to keep (`SEES`) starts over next time.
                return Some(if sample == SEES { 0 } else { sample });
            }
        }
        None
    }
}

/// The graph `Occlusion::contains` reads, for GPU culling's copy
/// (gfx/gpu_culling.rs): its searched box (`Graph::index`), the level's height
/// in sections, whether the search reached the box's top layer (the sky band
/// above a clamped box), and its cells' `REACHED` as bits.
#[derive(Clone, Copy)]
pub struct Reach<'a> {
    /// `Occlusion::generation` when it was taken.
    pub generation: u64,
    pub from_x: i32,
    pub from_z: i32,
    pub radius: i32,
    pub y0: i32,
    pub height: i32,
    pub sections: i32,
    pub sky_reached: bool,
    pub bits: &'a [u32],
}

#[cfg(test)]
impl Reach<'_> {
    /// gpu_culling.wgsl's `reached`, statement for statement: what
    /// `Graph::contains` answers.
    pub fn contains(&self, key: SectionKey) -> bool {
        let r = self.radius;
        let (dx, dz, dy) = (key.0 - self.from_x, key.2 - self.from_z, key.1 - self.y0);
        if dx < -r || dx > r || dz < -r || dz > r || dy < 0 {
            return false;
        }
        if dy >= self.height {
            return self.sky_reached && (0..=self.sections).contains(&key.1);
        }
        let i = (((dz + r) * (2 * r + 1) + dx + r) * self.height + dy) as usize;
        (self.bits[i / 32] >> (i % 32)) & 1 != 0
    }
}

/// `Mth.floor` for coordinates inside the world, without a libm call (the
/// x86-64 baseline has no rounding instruction).
fn floor(x: f64) -> i32 {
    let i = x as i32;
    i - i32::from(f64::from(i) > x)
}

struct Job {
    edits: Vec<Edit>,
    camera: DVec3,
    radius: i32,
    /// Buffers to reuse.
    graph: Graph,
}

/// The thread that runs full searches over its own copy of the columns.
struct Worker {
    jobs: Sender<Job>,
    done: Receiver<(Graph, Vec<Edit>)>,
}

impl Worker {
    fn spawn(min_y: i32, sections: i32) -> Option<Self> {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("occlusion".into())
            .spawn(move || {
                let mut columns = Columns::new(min_y, sections);
                let (mut vis, mut queue) = (Vec::new(), VecDeque::new());
                while let Ok(mut job) = job_rx.recv() {
                    for edit in job.edits.drain(..) {
                        columns.apply(edit);
                    }
                    job.graph
                        .search(&columns, job.camera, job.radius, &mut vis, &mut queue);
                    if done_tx.send((job.graph, job.edits)).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { jobs, done })
    }
}

/// A new value for `Occlusion::generation`, unique in the process: a copy made
/// from an older `Occlusion` (another level's) never matches. As gfx.rs
/// `next_block_entity_revision`.
fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub struct Occlusion {
    /// The render thread's copy, for partial updates (and searches when the
    /// worker is gone).
    columns: Columns,
    /// Edits the worker has not been sent yet.
    edits: Vec<Edit>,
    /// A full search is due: sections changed since the last one started.
    dirty: bool,
    /// The last finished search; `None` until one finishes, or after the
    /// columns were cleared.
    graph: Option<Graph>,
    spare: Graph,
    worker: Option<Worker>,
    /// The camera section and radius of the running search.
    running: Option<(SectionKey, i32)>,
    /// The running search started before the columns were cleared: drop it.
    discard: bool,
    /// The camera section and radius `graph`'s search started from (or is
    /// starting from).
    searched: Option<(SectionKey, i32)>,
    /// Sections changed since `graph`'s search started, to propagate from.
    changed: Vec<SectionKey>,
    /// Sections changed since the running search started: they propagate into
    /// its graph when it lands.
    changed_late: Vec<SectionKey>,
    vis: Vec<u64>,
    queue: VecDeque<u32>,
    /// Changes whenever `contains` may answer differently (`generation`).
    generation: u64,
    /// Set when edits were dropped; the next search resends every column.
    resync: bool,
}

impl Occlusion {
    /// The world's lowest block y and its height in sections.
    pub fn new(min_y: i32, sections: i32) -> Self {
        Self {
            columns: Columns::new(min_y, sections),
            edits: Vec::new(),
            dirty: true,
            graph: None,
            spare: Graph::default(),
            worker: Worker::spawn(min_y, sections),
            running: None,
            discard: false,
            searched: None,
            changed: Vec::new(),
            changed_late: Vec::new(),
            vis: Vec::new(),
            queue: VecDeque::new(),
            generation: next_generation(),
            resync: false,
        }
    }
    pub fn set(&mut self, key: SectionKey, vis: u64) {
        if self.columns.set(key, vis) {
            self.changed(key, vis);
        }
    }
    fn changed(&mut self, key: SectionKey, vis: u64) {
        self.dirty = true;
        if self.edits.len() >= EDIT_CAP || self.changed.len() >= EDIT_CAP {
            // nobody drains these with cave culling off
            self.resync = true;
            self.edits.clear();
            self.changed.clear();
            self.changed_late.clear();
        }
        if self.resync {
            return;
        }
        self.edits.push(Edit::Set(key, vis));
        self.changed.push(key);
        if self.running.is_some() {
            self.changed_late.push(key);
        }
    }
    fn resync_edits(&mut self) {
        self.resync = false;
        self.edits.clear();
        self.edits.push(Edit::Clear);
        for (&(x, z), column) in &self.columns.map {
            for (y, &vis) in column.iter().enumerate() {
                if vis != ALL_VISIBLE {
                    self.edits.push(Edit::Set((x, y as i32, z), vis));
                }
            }
        }
    }
    /// Every section of the column `x`, `z` goes (sees through every face).
    pub fn remove_column(&mut self, x: i32, z: i32) {
        let Some(column) = self.columns.map.remove(&(x, z)) else {
            return;
        };
        for (y, &vis) in column.iter().enumerate() {
            if vis != ALL_VISIBLE {
                self.changed((x, y as i32, z), ALL_VISIBLE);
            }
        }
    }
    /// `remove_column` for each of `gone`. When they are every column it holds (a
    /// teleport drops the whole view), the old graph says nothing about what arrives
    /// next: it starts over as `clear` does instead of seeing through each removed
    /// section one edit at a time (some 90 000 at render distance 32).
    pub fn remove_columns(&mut self, gone: &FxHashSet<(i32, i32)>) {
        if self.columns.map.keys().all(|column| gone.contains(column)) {
            self.clear();
            return;
        }
        for &(x, z) in gone {
            self.remove_column(x, z);
        }
    }
    pub fn clear(&mut self) {
        self.columns.map.clear();
        self.edits.clear();
        self.edits.push(Edit::Clear);
        self.resync = false;
        self.dirty = true;
        // Another world: the old graph says nothing about it. Without one, every
        // section was visible already, and stays so.
        if let Some(graph) = self.graph.take() {
            self.spare = graph;
            self.generation = next_generation();
        }
        self.searched = None;
        self.discard = self.running.is_some();
        self.changed.clear();
        self.changed_late.clear();
    }
    /// Whether `key` is visible from the camera. Without a usable graph
    /// (before the first search lands, or with the camera somewhere the graph
    /// never reached) every section is.
    pub fn contains(&self, key: SectionKey) -> bool {
        self.graph.as_ref().is_none_or(|g| g.contains(key))
    }
    /// Whether the current graph knows that `key` is visible from
    /// `camera_section`. Unknown is returned when the graph is absent, was
    /// searched from another section, or does not cover the queried section.
    pub fn visible_for_camera(&self, key: SectionKey, camera_section: SectionKey) -> Option<bool> {
        let graph = self.graph.as_ref()?;
        if graph.from != camera_section {
            return None;
        }
        let dx = i64::from(key.0) - i64::from(graph.from.0);
        let dz = i64::from(key.2) - i64::from(graph.from.2);
        if dx.abs() > i64::from(graph.radius)
            || dz.abs() > i64::from(graph.radius)
            || key.1 < graph.y0
            || key.1 > graph.sections
        {
            return None;
        }
        Some(graph.contains(key))
    }
    /// Changes whenever `contains` may answer differently: a search landed, a
    /// partial update reached more sections, or the graph went.
    #[cfg(any(test, debug_assertions))]
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// The graph `contains` reads; `None` while every section counts as
    /// visible.
    pub fn reach(&self) -> Option<Reach<'_>> {
        self.graph.as_ref().map(|g| Reach {
            generation: self.generation,
            from_x: g.from.0,
            from_z: g.from.2,
            radius: g.radius,
            y0: g.y0,
            height: g.height,
            sections: g.sections,
            sky_reached: g.sky_reached,
            bits: &g.bits,
        })
    }
    /// The bits `reach` exports are the cells' `REACHED`, cell for cell.
    #[cfg(any(test, debug_assertions))]
    pub fn reach_is_exact(&self) -> bool {
        self.graph.as_ref().is_none_or(|g| {
            g.bits.len() == g.directions.len().div_ceil(32)
                && g.directions
                    .iter()
                    .enumerate()
                    .all(|(i, &d)| (d & REACHED != 0) == ((g.bits[i / 32] >> (i % 32)) & 1 != 0))
        })
    }
    /// Take a finished search, propagate changed sections into the graph,
    /// and start a search when sections changed or the camera moved to
    /// another section. Never waits for the worker.
    pub fn update(&mut self, camera: DVec3, radius: i32) {
        let from = self.columns.section_of(camera);
        self.collect(false);
        self.propagate_changes(camera);
        if self.running.is_none() && (self.dirty || self.searched != Some((from, radius))) {
            self.start(camera, radius);
        }
        // A graph that never reached the camera's section hides everything
        // around it: until the new one lands, cull by the frustum alone.
        if self
            .graph
            .as_ref()
            .is_some_and(|g| g.from != from && !g.contains(from))
            && let Some(graph) = self.graph.take()
        {
            self.spare = graph;
            self.generation = next_generation();
        }
    }
    fn start(&mut self, camera: DVec3, radius: i32) {
        let from = self.columns.section_of(camera);
        self.dirty = false;
        self.searched = Some((from, radius));
        if self.resync {
            self.resync_edits();
        }
        let graph = std::mem::take(&mut self.spare);
        if let Some(worker) = &self.worker {
            let job = Job {
                edits: std::mem::take(&mut self.edits),
                camera,
                radius,
                graph,
            };
            if worker.jobs.send(job).is_ok() {
                self.running = Some((from, radius));
                self.changed_late.clear();
                return;
            }
            log::warn!("Horizon: the occlusion worker stopped; culling on the render thread");
            self.worker = None;
        }
        self.edits.clear();
        let mut graph = Graph::default();
        graph.search(
            &self.columns,
            camera,
            radius,
            &mut self.vis,
            &mut self.queue,
        );
        self.land(graph);
    }
    fn collect(&mut self, wait: bool) {
        let Some(worker) = &self.worker else { return };
        if self.running.is_none() {
            return;
        }
        let result = if wait {
            worker.done.recv().map_err(|_| TryRecvError::Disconnected)
        } else {
            worker.done.try_recv()
        };
        match result {
            Ok((graph, edits)) => {
                self.running = None;
                if self.edits.is_empty() {
                    self.edits = edits;
                }
                if std::mem::take(&mut self.discard) {
                    self.spare = graph;
                    return;
                }
                self.land(graph);
                self.changed.clear();
                self.changed.append(&mut self.changed_late);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                // The edits it held are lost with it: search here from now on.
                self.worker = None;
                self.running = None;
                self.discard = false;
                self.dirty = true;
            }
        }
    }
    fn land(&mut self, graph: Graph) {
        if let Some(old) = self.graph.replace(graph) {
            self.spare = old;
        }
        self.generation = next_generation();
    }
    /// Vanilla's `runPartialUpdate`: what changed sections now connect to.
    fn propagate_changes(&mut self, camera: DVec3) {
        let Some(graph) = &mut self.graph else {
            self.changed.clear();
            return;
        };
        self.queue.clear();
        for key in self.changed.drain(..) {
            if let Some(i) = graph.index(key)
                && graph.directions[i] & REACHED != 0
            {
                self.queue.push_back(i as u32);
            }
        }
        let columns = &self.columns;
        if graph.propagate(&mut self.queue, camera, false, |_, key| columns.get(key)) {
            self.generation = next_generation();
        }
    }
    /// Wait for the running search: tests, and A/B timing of the old
    /// blocking search.
    #[cfg(test)]
    fn settle(&mut self, camera: DVec3, radius: i32) {
        self.update(camera, radius);
        self.collect(true);
        self.propagate_changes(camera);
    }
}

#[cfg(test)]
mod tests;
