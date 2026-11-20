//! Far meshes share the terrain arenas and passes, with their actual bounds
//! rather than the 16³ boxes used by the near-section residency walk.
use super::{Cull, Frustum, Renderer, SectionGpu, SectionKey, ShadowCull};
use crate::{
    fast_hash::{FxHashMap, FxHashSet},
    lod::{MAX_LEVEL, MeshCutouts, TileKey},
    mesher::SectionMesh,
};
use glam::{DVec3, Vec3};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

/// Upload state moves with the meshes when a new renderer takes over the world.
#[derive(Default)]
pub(super) struct Uploads {
    pub pending: Option<Pending>,
    /// Completed empty tiles can satisfy another mesh's finer cutout.
    empty: FxHashSet<TileKey>,
}

/// Finished tile meshes not uploaded yet, the tiles they replace and the
/// columns they leave out.
#[derive(Default)]
pub(super) struct Pending {
    meshes: FxHashMap<TileKey, SectionMesh>,
    /// The order they go up in: as handed over (`Lod::update`: finer tiles
    /// first, so no tile clipped around a finer one is drawn before it).
    order: VecDeque<TileKey>,
    retry_at: FxHashMap<TileKey, Instant>,
    removed: FxHashSet<TileKey>,
    cutout: Option<FxHashSet<(i32, i32)>>,
    needs_finish: bool,
}

impl Pending {
    /// Takes in the next job, with the same result as applying both in
    /// turn: newer meshes replace waiting ones, tiles meshed again are not
    /// removed, and tiles removed now drop their waiting meshes.
    fn add(
        &mut self,
        meshes: Vec<(TileKey, SectionMesh)>,
        removed: Vec<TileKey>,
        cutout: Option<FxHashSet<(i32, i32)>>,
    ) {
        for (key, mesh) in meshes {
            self.removed.remove(&key);
            self.retry_at.remove(&key);
            if self.meshes.insert(key, mesh).is_none() {
                self.order.push_back(key);
            }
        }
        for key in removed {
            self.meshes.remove(&key);
            self.retry_at.remove(&key);
            self.removed.insert(key);
        }
        if cutout.is_some() {
            self.cutout = cutout;
        }
        self.needs_finish = true;
    }

    fn take_next(&mut self, now: Instant) -> Option<(TileKey, SectionMesh)> {
        for _ in 0..self.order.len() {
            let key = self.order.pop_front().expect("queued tile");
            if self.retry_at.get(&key).is_some_and(|&due| now < due) {
                self.order.push_back(key);
            } else if let Some(mesh) = self.meshes.remove(&key) {
                self.retry_at.remove(&key);
                return Some((key, mesh));
            }
        }
        None
    }

    fn retry(&mut self, key: TileKey, mesh: SectionMesh, now: Instant) {
        // Try other tiles first and leave time for the arena to reclaim space.
        self.retry_at.insert(key, now + Duration::from_millis(250));
        self.meshes.insert(key, mesh);
        self.order.push_back(key);
    }
}

/// Two tiles overlap when one is the other or its ancestor.
#[derive(Default)]
struct TileCoverage {
    keys: FxHashSet<TileKey>,
    ancestors: FxHashSet<TileKey>,
}

impl TileCoverage {
    fn new(keys: impl IntoIterator<Item = TileKey>) -> Self {
        let mut coverage = Self::default();
        for key in keys {
            coverage.insert(key);
        }
        coverage
    }

    fn insert(&mut self, mut key: TileKey) {
        self.keys.insert(key);
        while key.level < MAX_LEVEL && self.ancestors.insert(key.parent()) {
            key = key.parent();
        }
    }

    fn overlaps(&self, mut key: TileKey) -> bool {
        !self.keys.is_empty()
            && (self.ancestors.contains(&key)
                || (key.level..=MAX_LEVEL).any(|_| {
                    let contains = self.keys.contains(&key);
                    key = key.parent();
                    contains
                }))
    }
}

/// Small, its meshes boxed: `draw_frame` walks every tile each frame (thousands at long far
/// distances), and that walk is bound by the memory it reads.
pub(super) struct Tile {
    /// None for an empty replacement whose finer cutouts are still uploading.
    pub mesh: Option<Box<SectionGpu>>,
    info: MeshInfo,
    /// When it began to fade in; none while the rest of its job uploads.
    shown: Option<Instant>,
    /// When its replacements began to fade in; it goes once they have.
    dying: Option<Instant>,
    /// Dying with nothing to replace it (out of range): it fades out.
    alone: bool,
    /// Its mesh before near chunks took some of its columns, drawn in its
    /// place until they have faded in (from the job's end, the instant), so
    /// no hole shows through them.
    previous: Option<Box<StandIn>>,
    /// Last published shadow mesh when the world is holding a different stand-in.
    shadow_previous: Option<Box<StandIn>>,
    /// A finished upload still waiting to start its world handoff.
    awaits_publication: bool,
    /// Keep the published shadow while a new mesh or its finer cutouts are still uploading.
    shadow_waits: bool,
    /// Its slot in occlusion culling's results (`cull::TILE`), kept while
    /// meshes of the same tile replace one another.
    pub cull_slot: u32,
}

struct StandIn {
    mesh: SectionGpu,
    info: MeshInfo,
    since: Option<Instant>,
    casts_shadow: bool,
}

pub(super) struct MeshInfo {
    bounds: Bounds,
    cutouts: Option<Arc<MeshCutouts>>,
}

#[derive(Default)]
struct Handoff {
    shown: Option<Instant>,
    previous: Option<Box<StandIn>>,
    shadow_previous: Option<Box<StandIn>>,
}

/// How long a tile takes to fade in over the ones it replaces.
pub(super) const FADE: Duration = Duration::from_millis(600);

/// A tile or section fading in: 0 drawn whole, else 1 + `level` * 129 +
/// `shown`, `shown` 1 to 63 sixty-fourths of `FADE` (`fade_out_code`: 65 to
/// 128, fading out). Chunk by chunk (columns of its own grid, wider for
/// coarse tiles: 2^`level` blocks), each starting in turn over the first half
/// and fading over the second, as a fine ordered pattern fixed to the terrain
/// (terrain.wgsl's `faded`). It is drawn a hair toward the camera, the newest
/// most, over the solid tiles it replaces, which stay until it is whole
/// (`upload_lods`): no pixel fought over, no hole. None: nothing to draw yet.
pub(super) fn fade_code(shown: Option<Instant>, level: u8, now: Instant) -> Option<u32> {
    let faded = now.saturating_duration_since(shown?).as_secs_f32() / FADE.as_secs_f32();
    let shown = (faded * 64.0) as u32 + 1;
    Some(if shown >= 64 {
        0
    } else {
        1 + u32::from(level) * 129 + shown
    })
}
/// A section past the render distance that far terrain is coming in under:
/// whole, a fading-out code at its start (terrain.wgsl's `fade_depth` draws
/// leaving sections in front of far terrain).
pub(super) const HELD: u32 = 1 + 64;
/// `fade_code` for one leaving since `since`, chunk by chunk as it came, over
/// what stays behind it; once `FADE` has passed it draws nothing.
pub(super) fn fade_out_code(since: Instant, level: u8, now: Instant) -> u32 {
    let faded = now.saturating_duration_since(since).as_secs_f32() / FADE.as_secs_f32();
    1 + u32::from(level) * 129 + 64 + ((faded * 64.0) as u32 + 1).min(64)
}

#[derive(Clone, Copy, Default)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}

impl Bounds {
    fn mesh(mesh: &SectionMesh) -> Option<Self> {
        let [min, max] = mesh.bounds.or_else(|| mesh.compute_bounds())?;
        Some(Self { min, max })
    }

    /// In view: the plan alone decides which tiles are in range (a circle),
    /// so a tile leaving it fades out (`upload_lods`). Cut here at the range,
    /// tiles popped out as the player moved, before the plan dropped them.
    fn visible(self, origin: DVec3, frustum: &Frustum) -> Option<u64> {
        let min = origin + self.min.as_dvec3();
        let max = origin + self.max.as_dvec3();
        if !frustum.intersects(min.as_vec3(), max.as_vec3()) {
            return None;
        }
        Some(((min + max) * 0.5).length_squared().to_bits())
    }
}

/// A tile's instance: like a section's, with the UV step of merged faces.
/// A fading tile's step is negated past its `fade_code`, which the shaders
/// read back as the whole part (they take the step as `fract(abs(w))`).
pub(super) fn instance(origin: [f32; 3], fade: u32) -> [f32; 4] {
    let step = 1.0 / crate::mesher::TILED_UV_STEPS;
    [
        origin[0],
        origin[1],
        origin[2],
        if fade == 0 {
            step
        } else {
            -(fade as f32 + step)
        },
    ]
}

pub(super) struct Visible {
    pub key: TileKey,
    pub distance: u64,
    pub origin: [f32; 3],
    pub fade: u32,
    /// Replaced, standing until its replacements are whole: its water goes
    /// after theirs (`Renderer::render`'s water tiers), so depth leaves it only
    /// where they have none yet.
    pub old: bool,
}

pub(super) struct ShadowTile {
    pub key: TileKey,
    pub origin: [f32; 3],
    pub height: f32,
}

#[derive(Default)]
pub(super) struct ShadowTiles {
    pub tiles: Vec<ShadowTile>,
    keys: FxHashSet<TileKey>,
    levels: u32,
    near_cutouts: FxHashSet<(i32, i32)>,
}

impl ShadowTiles {
    pub fn consider(
        &mut self,
        key: TileKey,
        tile: &Tile,
        camera: DVec3,
        cull: &ShadowCull,
        light: Vec3,
        lods: &FxHashMap<TileKey, Tile>,
    ) {
        if tile.dying.is_some() && !tile.alone && !retired_shadow_needed(lods, key) {
            return;
        }
        let Some((mesh, info)) = tile.shadow() else {
            return;
        };
        if tile.shadow_waits
            && let Some(cutouts) = &info.cutouts
        {
            self.near_cutouts.extend(cutouts.near.iter().copied());
        }
        let origin = (DVec3::from(key.origin().map(f64::from)) - camera).as_vec3();
        let (min, max) = (origin + info.bounds.min, origin + info.bounds.max);
        if cull.intersects(min, max) && mesh.solid > 0 {
            self.tiles.push(ShadowTile {
                key,
                origin: origin.to_array(),
                height: ((min + max) * 0.5).dot(light),
            });
        }
    }

    pub fn finish(&mut self, lods: &FxHashMap<TileKey, Tile>) {
        // A finished child can arrive before its parent's mesh is clipped
        // around it. The parent owns that part until it leaves it out.
        self.tiles
            .sort_unstable_by_key(|v| std::cmp::Reverse(v.key.level));
        let (keys, levels) = (&mut self.keys, &mut self.levels);
        self.tiles.retain(|v| {
            if Self::covering(keys, *levels, v.key).any(|parent| {
                let (_, info) = lods[&parent].shadow().expect("shadow tile");
                !info
                    .cutouts
                    .as_ref()
                    .is_some_and(|c| c.excludes_tile(v.key))
            }) {
                return false;
            }
            keys.insert(v.key);
            *levels |= 1 << v.key.level;
            true
        });
        self.tiles.sort_unstable_by(|a, b| {
            b.height
                .total_cmp(&a.height)
                .then_with(|| a.key.cmp(&b.key))
        });
    }

    pub fn clear(&mut self) {
        self.tiles.clear();
        self.keys.clear();
        self.levels = 0;
        self.near_cutouts.clear();
    }

    pub fn includes_retiring_column(&self, x: i32, z: i32) -> bool {
        self.near_cutouts.contains(&(x, z))
    }

    /// Meshes of one global cutout can finish in different frames, so look
    /// at the geometry that made it to the shadow list in this frame.
    pub fn casts_section(
        &self,
        (x, _, z): SectionKey,
        section: &SectionGpu,
        lods: &FxHashMap<TileKey, Tile>,
    ) -> bool {
        (section.leaving.is_none() || self.includes_retiring_column(x, z))
            && !Self::covering(&self.keys, self.levels, TileKey { x, z, level: 0 }).any(|key| {
                let (_, info) = lods[&key].shadow().expect("shadow tile");
                !info
                    .cutouts
                    .as_ref()
                    .is_some_and(|c| c.excludes_column(x, z))
            })
    }

    fn covering(
        keys: &FxHashSet<TileKey>,
        levels: u32,
        child: TileKey,
    ) -> impl Iterator<Item = TileKey> + '_ {
        let mut levels = levels & !((1 << child.level) - 1);
        std::iter::from_fn(move || {
            while levels != 0 {
                let level = levels.trailing_zeros() as u8;
                levels &= levels - 1;
                let shift = level - child.level;
                let key = TileKey {
                    x: child.x >> shift,
                    z: child.z >> shift,
                    level,
                };
                if keys.contains(&key) {
                    return Some(key);
                }
            }
            None
        })
    }
}

/// A retired child can still fill a hole in the published parent while that
/// parent's new mesh is waiting for a different finer tile.
fn retired_shadow_needed(lods: &FxHashMap<TileKey, Tile>, key: TileKey) -> bool {
    waiting_parent_cuts(lods, key, |cutouts| cutouts.excludes_tile(key))
}

/// A resident section's visual fade may finish while the published parent
/// still leaves its column out of the shadow map.
pub(super) fn retired_near_needed(lods: &FxHashMap<TileKey, Tile>, (x, _, z): SectionKey) -> bool {
    waiting_parent_cuts(lods, TileKey { x, z, level: 0 }, |cutouts| {
        cutouts.near.contains(&(x, z))
    })
}

fn waiting_parent_cuts(
    lods: &FxHashMap<TileKey, Tile>,
    key: TileKey,
    cuts: impl Fn(&MeshCutouts) -> bool,
) -> bool {
    let mut parent = key;
    while parent.level < MAX_LEVEL {
        parent = parent.parent();
        if lods.get(&parent).is_some_and(|tile| {
            tile.shadow_waits
                && tile.dying.is_none()
                && tile
                    .shadow()
                    .is_some_and(|(_, info)| info.cutouts.as_deref().is_some_and(&cuts))
        }) {
            return true;
        }
    }
    false
}

/// Publishes the ready parts of a renderer upload batch. A meshed parent can leave out a
/// child whose own worker job has yet to finish. Keep the published shadow
/// until that child arrives, even after the visual stand-in has faded.
fn finish_uploads(
    lods: &mut FxHashMap<TileKey, Tile>,
    empty: &mut FxHashSet<TileKey>,
    free_slots: &mut Vec<u32>,
    removed: &FxHashSet<TileKey>,
    unfinished: &TileCoverage,
    now: Instant,
    start_showing: impl Fn(TileKey) -> Instant,
) -> Vec<SectionGpu> {
    let mut waiting: Vec<_> = lods
        .iter()
        .filter_map(|(&key, tile)| tile.shadow_waits.then_some(key))
        .collect();
    // A finer mesh can itself have cutouts. Resolve those first, even when
    // the worker returned all the meshes in the same batch.
    waiting.sort_unstable_by_key(|key| key.level);
    let mut blocked = FxHashSet::default();
    for key in waiting {
        let tile = &lods[&key];
        if tile.dying.is_some()
            || removed.contains(&key)
            || unfinished.overlaps(key)
            || !tile.finer_ready(|child| {
                !removed.contains(&child)
                    && lods.get(&child).map_or_else(
                        || empty.contains(&child),
                        |tile| tile.dying.is_none() && !blocked.contains(&child),
                    )
            })
        {
            blocked.insert(key);
        }
    }
    let mut released = Vec::new();
    lods.retain(|&key, tile| {
        if removed.contains(&key) {
            return true;
        }
        if unfinished.overlaps(key) {
            if tile.awaits_publication
                && let Some(previous) = tile.previous.as_deref_mut()
            {
                // A second mesh can inherit a stand-in from an earlier fade.
                // Keep it through the new fade once the held handoff publishes.
                previous.since = None;
            }
            return true;
        }
        tile.shown.get_or_insert_with(|| start_showing(key));
        released.extend(tile.finish_upload(now, !blocked.contains(&key)));
        if tile.mesh.is_some() || tile.shadow_waits {
            return true;
        }
        empty.insert(key);
        free_slots.push(tile.cull_slot);
        released.extend(tile.previous.take().map(|previous| previous.mesh));
        false
    });
    released
}

impl Tile {
    /// The mesh drawn this frame: the previous one while it stands in.
    pub fn drawn(&self) -> &SectionGpu {
        self.previous.as_deref().map_or_else(
            || self.mesh.as_deref().expect("visible tile has a mesh"),
            |p| &p.mesh,
        )
    }
    pub fn drawn_mut(&mut self) -> &mut SectionGpu {
        match self.previous.as_deref_mut() {
            Some(p) => &mut p.mesh,
            None => self.mesh.as_deref_mut().expect("visible tile has a mesh"),
        }
    }
    /// The box occlusion culling tests (`cull::Cull`), around its origin: its
    /// mesh's bounds grown by a block, while that mesh is the one drawn
    /// whole (no stand-in, no fade). A fading tile's dither moves its depth
    /// toward the camera (terrain.wgsl's `fade_depth`), out of the box.
    pub fn cull_bounds(&self, fade: u32) -> Option<[[f32; 3]; 2]> {
        (fade == 0 && self.previous.is_none()).then(|| {
            [
                (self.info.bounds.min - 1.0).to_array(),
                (self.info.bounds.max + 1.0).to_array(),
            ]
        })
    }
    /// Whether any mesh it holds is in the compact layout: a stand-in drawn in its place can
    /// hide a compact replacement (`Renderer::follow_packed_layout`).
    pub fn holds_packed(&self) -> bool {
        self.mesh.as_ref().is_some_and(|mesh| mesh.packed)
            || self.previous.as_ref().is_some_and(|p| p.mesh.packed)
            || self.shadow_previous.as_ref().is_some_and(|p| p.mesh.packed)
    }
    /// All meshes it holds in the arena.
    pub fn meshes_mut(&mut self) -> impl Iterator<Item = &mut SectionGpu> {
        self.mesh
            .as_deref_mut()
            .into_iter()
            .chain(self.previous.as_deref_mut().map(|p| &mut p.mesh))
            .chain(self.shadow_previous.as_deref_mut().map(|p| &mut p.mesh))
    }

    /// Shadow depth has no cross-fade. Keep the old mesh until the whole
    /// upload is ready, then cast only the replacement even while both draw.
    pub fn shadow(&self) -> Option<(&SectionGpu, &MeshInfo)> {
        if self.shadow_waits {
            let previous = self
                .shadow_previous
                .as_deref()
                .or_else(|| self.previous.as_deref().filter(|p| p.casts_shadow))?;
            return Some((&previous.mesh, &previous.info));
        }
        self.shown
            .and(self.mesh.as_deref())
            .map(|mesh| (mesh, &self.info))
    }

    /// Keeps the currently published shadow mesh through a replacement
    /// upload, even if the world is still fading over an older mesh.
    fn handoff(self, keep: bool, shadows: bool, release: impl FnOnce(SectionGpu)) -> Handoff {
        let mut handoff = Handoff {
            shown: self.shown,
            previous: self.previous,
            shadow_previous: self.shadow_previous,
        };
        let Some(mesh) = self.mesh else {
            return handoff;
        };
        let old = StandIn {
            mesh: *mesh,
            info: self.info,
            since: None,
            casts_shadow: shadows && !self.shadow_waits && self.shown.is_some(),
        };
        if handoff.previous.is_none() && keep {
            handoff.previous = Some(Box::new(old));
        } else if old.casts_shadow {
            handoff.shadow_previous = Some(Box::new(old));
        } else {
            release(old.mesh);
        }
        handoff
    }

    fn finer_ready(&self, ready: impl Fn(TileKey) -> bool) -> bool {
        self.info
            .cutouts
            .as_ref()
            .is_none_or(|c| c.finer.iter().copied().all(ready))
    }

    fn finish_upload(&mut self, now: Instant, finer_ready: bool) -> Option<SectionGpu> {
        self.awaits_publication = false;
        if let Some(previous) = self.previous.as_deref_mut() {
            previous.since.get_or_insert(now);
        }
        if !finer_ready {
            return None;
        }
        self.shadow_waits = false;
        if let Some(previous) = self.previous.as_deref_mut() {
            previous.casts_shadow = false;
        }
        self.shadow_previous.take().map(|previous| previous.mesh)
    }

    fn release_visible_stand_in(&mut self, now: Instant) -> Option<SectionGpu> {
        if self.awaits_publication {
            return None;
        }
        let since = self.previous.as_deref()?.since?;
        if now - since < FADE {
            return None;
        }
        let previous = self.previous.take();
        if self.shadow_waits && previous.as_ref().is_some_and(|p| p.casts_shadow) {
            self.shadow_previous = previous;
            None
        } else {
            previous.map(|p| p.mesh)
        }
    }

    pub fn visible(
        &self,
        key: TileKey,
        camera: DVec3,
        frustum: &Frustum,
        now: Instant,
    ) -> Option<Visible> {
        if self.mesh.is_none() || self.dying.is_some_and(|since| now - since >= FADE) {
            return None;
        }
        let fade = match self.dying {
            Some(since) if self.alone => fade_out_code(since, key.level, now),
            _ => fade_code(self.shown, key.level, now)?,
        };
        let origin = DVec3::from(key.origin().map(f64::from)) - camera;
        Some(Visible {
            key,
            distance: self.info.bounds.visible(origin, frustum)?,
            origin: origin.as_vec3().to_array(),
            fade,
            old: self.dying.is_some() || self.previous.is_some(),
        })
    }
}

impl Renderer {
    /// A tile meshed again (finer detail) takes its place at once, but for
    /// one meshed around near chunks or finer tiles that are new (`keep`):
    /// its old mesh stands in while they fade in. A new one fades in once the
    /// overlapping uploads are ready (`upload_lods`).
    fn upload_lod(&mut self, key: TileKey, mesh: &SectionMesh, keep: bool) -> bool {
        let shadows = self.executor.is_some_and(|caps| caps.shadow);
        let split = self.translucent_arena.is_some();
        let (bounds, uploaded, replace_now) =
            if mesh.solid.is_empty() && mesh.translucent.is_empty() {
                (Bounds::default(), None, false)
            } else {
                let bounds = Bounds::mesh(mesh).expect("nonempty tile has bounds");
                // An ordinary replacement can take over the same ranges. A tile
                // with a visual or shadow handoff needs both meshes alive.
                let old = self.lods.get(&key).filter(|old| {
                    !keep
                        && old.shown.is_some()
                        && old.dying.is_none()
                        && old.previous.is_none()
                        && old.shadow_previous.is_none()
                        && !old.shadow_waits
                        && (!shadows || (old.info.cutouts.is_none() && mesh.lod_cutouts.is_none()))
                });
                let ranges = old
                    .and_then(|old| old.mesh.as_deref())
                    .map(|mesh| mesh.ranges(split));
                let replace_now = ranges.is_some();
                let Some(uploaded) = self.upload_mesh(mesh, ranges.unwrap_or_default()) else {
                    return false;
                };
                (bounds, Some(Box::new(uploaded)), replace_now)
            };
        if let Some(copies) = &mut self.capture_copies {
            if uploaded.is_some() {
                copies.tile(key, mesh);
            } else {
                copies.forget_tile(key);
            }
        }
        self.lod_uploads.empty.remove(&key);
        let old = self.lods.remove(&key);
        let old_slot = old.as_ref().map(|tile| tile.cull_slot);
        let Handoff {
            shown,
            previous,
            shadow_previous,
        } = match old {
            Some(old) if replace_now => {
                let mesh = *old.mesh.expect("ordinary replacement has an old mesh");
                let ranges = uploaded.as_deref().expect("new mesh").ranges(split);
                self.release_mesh_except(mesh, ranges);
                Handoff {
                    shown: old.shown,
                    ..Handoff::default()
                }
            }
            Some(old) if old.dying.is_none() => {
                old.handoff(keep && uploaded.is_some(), shadows, |mesh| {
                    self.release_mesh(mesh)
                })
            }
            Some(old) if shadows && retired_shadow_needed(&self.lods, key) => Handoff {
                shown: None,
                ..old.handoff(false, true, |mesh| self.release_mesh(mesh))
            },
            Some(old) => {
                self.release_tile(old);
                Handoff::default()
            }
            None => Handoff::default(),
        };
        let cull_slot = old_slot.unwrap_or_else(|| {
            self.lod_cull_free.pop().unwrap_or_else(|| {
                self.lod_cull_next += 1;
                self.lod_cull_next - 1
            })
        });
        self.lods.insert(
            key,
            Tile {
                mesh: uploaded,
                info: MeshInfo {
                    bounds,
                    cutouts: mesh.lod_cutouts.clone(),
                },
                shown,
                dying: None,
                alone: false,
                previous,
                shadow_previous,
                awaits_publication: !replace_now,
                shadow_waits: shadows && !replace_now,
                cull_slot,
            },
        );
        true
    }

    fn release_tile(&mut self, tile: Tile) {
        if let Some(mesh) = tile.mesh {
            self.release_mesh(*mesh);
        }
        for previous in tile.previous.into_iter().chain(tile.shadow_previous) {
            self.release_mesh(previous.mesh);
        }
    }

    pub fn remove_lod(&mut self, key: TileKey) {
        self.lod_uploads.empty.remove(&key);
        if let Some(copies) = &mut self.capture_copies {
            copies.forget_tile(key);
        }
        if let Some(tile) = self.lods.remove(&key) {
            self.lod_cull_free.push(tile.cull_slot);
            self.release_tile(tile);
        }
    }

    pub fn clear_lods(&mut self) {
        if let Some(copies) = &mut self.capture_copies {
            copies.clear_tiles();
        }
        self.lod_uploads = Uploads::default();
        for (_, tile) in std::mem::take(&mut self.lods) {
            self.release_tile(tile);
        }
        self.lod_cull_free.clear();
        self.lod_cull_next = 0;
        self.set_lod_cutout(Default::default());
        // No far terrain left to fade out over. Drained after `set_lod_cutout`, which
        // moves the sections far terrain still held (`lod_retiring`) onto this list:
        // `release_left` runs only from `upload_lods`, and the game stops calling that
        // once far terrain is off, so a section left here would draw its unloaded
        // column past the render distance for the rest of the renderer's life.
        for (key, since) in std::mem::take(&mut self.lod_leaving) {
            if self
                .sections
                .find(key)
                .is_some_and(|s| s.leaving == Some(since))
            {
                self.release_section(key);
            }
        }
    }

    /// Queues a finished mesh job (`Lod::update`, `Lod::removed`,
    /// `Lod::take_cutout`) for `upload_lods`, joining one still uploading
    /// (`Pending::add`).
    pub fn queue_lods(
        &mut self,
        meshes: Vec<(TileKey, SectionMesh)>,
        removed: Vec<TileKey>,
        cutout: Option<FxHashSet<(i32, i32)>>,
    ) {
        if meshes.is_empty() && removed.is_empty() && cutout.is_none() {
            return;
        }
        self.lod_uploads
            .pending
            .get_or_insert_default()
            .add(meshes, removed, cutout);
    }

    /// Uploads queued tile meshes for about `budget` (at least one), then
    /// fades the ready tiles in over the ones they replace.
    pub fn upload_lods(&mut self, budget: Duration) {
        let start = Instant::now();
        if self
            .lod_uploads
            .pending
            .as_ref()
            .is_some_and(|p| p.needs_finish && p.retry_at.is_empty())
        {
            // Far tiles fill the arenas too: large translucent pages wait for them.
            self.shrink_gate.wait(start, super::SHRINK_SETTLE);
        }
        self.release_left(start);
        let faded: Vec<_> = self
            .lods
            .iter()
            .filter(|&(&key, tile)| {
                tile.dying.is_some_and(|d| start - d >= FADE)
                    && !retired_shadow_needed(&self.lods, key)
            })
            .map(|(&key, _)| key)
            .collect();
        for key in faded {
            self.remove_lod(key);
        }
        let stood_in: Vec<_> = self
            .lods
            .values_mut()
            .filter_map(|tile| tile.release_visible_stand_in(start))
            .collect();
        for previous in stood_in {
            self.release_mesh(previous);
        }
        // The columns near chunks take in this job, and the tiles holding the
        // job's new finer ones: a tile meshed again around them keeps its
        // old mesh until they are in, so no hole shows through them.
        let mut around_new = FxHashSet::default();
        let new = self
            .lod_uploads
            .pending
            .iter()
            .flat_map(|pending| pending.meshes.keys())
            .filter(|key| !self.lods.contains_key(key))
            .chain(
                self.lods
                    .iter()
                    .filter(|(_, tile)| tile.mesh.is_some() && tile.shown.is_none())
                    .map(|(key, _)| key),
            );
        for &key in new {
            let mut key = key;
            while key.level < MAX_LEVEL && around_new.insert(key.parent()) {
                key = key.parent();
            }
        }
        let taken: Vec<_> = self
            .lod_uploads
            .pending
            .iter()
            .flat_map(|pending| pending.cutout.iter().flatten())
            .filter(|column| !self.lod_cutout.contains(column))
            .copied()
            .collect();
        while let Some(pending) = &mut self.lod_uploads.pending {
            // Keys meshed again keep their place; removed ones are skipped.
            let Some((key, mesh)) = pending.take_next(start) else {
                break;
            };
            let [x, _, z] = key.origin();
            let size = key.size();
            let keep = mesh.keep
                || around_new.contains(&key)
                || taken.iter().any(|&(cx, cz)| {
                    (x..x + size).contains(&(cx * 16)) && (z..z + size).contains(&(cz * 16))
                });
            let uploaded = self.upload_lod(key, &mesh, keep);
            let pending = self.lod_uploads.pending.as_mut().expect("pending tiles");
            if uploaded {
                pending.needs_finish = true;
            } else {
                pending.retry(key, mesh, start);
            }
            if start.elapsed() >= budget {
                if pending.retry_at.is_empty() {
                    return;
                }
                // Settle disjoint work even if new uploads keep filling the
                // frame budget while the blocked tiles wait for space.
                break;
            }
        }
        if self
            .lod_uploads
            .pending
            .as_ref()
            .is_some_and(|pending| pending.needs_finish)
        {
            let pending = self.lod_uploads.pending.take().expect("pending tiles");
            self.finish_lod_uploads(pending, start);
        }
    }

    fn finish_lod_uploads(&mut self, mut pending: Pending, now: Instant) {
        let unfinished = TileCoverage::new(pending.meshes.keys().copied());
        let replaced = TileCoverage::new(
            pending
                .removed
                .iter()
                .copied()
                .filter(|key| self.lods.contains_key(key)),
        );
        let cutout = pending.cutout.as_ref().unwrap_or(&self.lod_cutout);
        let kept = TileCoverage::new(
            self.lods
                .iter()
                .filter(|(key, tile)| {
                    tile.mesh.is_some() && tile.shown.is_none() && !pending.removed.contains(key)
                })
                .map(|(&key, _)| key)
                .chain(cutout.iter().map(|&(x, z)| TileKey { x, z, level: 0 })),
        );

        // Fade over the tiles this one replaces. Where nothing stood before,
        // show the new tile whole so its fade cannot open holes.
        let whole = now.checked_sub(FADE).unwrap_or(now);
        let released = finish_uploads(
            &mut self.lods,
            &mut self.lod_uploads.empty,
            &mut self.lod_cull_free,
            &pending.removed,
            &unfinished,
            now,
            |key| if replaced.overlaps(key) { now } else { whole },
        );
        for mesh in released {
            self.release_mesh(mesh);
        }

        // A refused child can hold up the whole parent that would replace an
        // old sibling, even if the parent is already visible but cannot cast.
        let mut withheld = TileCoverage::default();
        let mut unpublished_shadows = TileCoverage::default();
        for (&key, tile) in &self.lods {
            if tile.dying.is_none() && !pending.removed.contains(&key) {
                if tile.awaits_publication || (tile.shadow_waits && unfinished.overlaps(key)) {
                    withheld.insert(key);
                }
                if tile.shadow_waits {
                    unpublished_shadows.insert(key);
                }
            }
        }
        let overlaps_withheld = |key| unfinished.overlaps(key) || withheld.overlaps(key);
        pending.removed.retain(|&key| {
            if overlaps_withheld(key) || unpublished_shadows.overlaps(key) {
                return true;
            }
            self.lod_uploads.empty.remove(&key);
            if let Some(tile) = self.lods.get_mut(&key)
                && tile.dying.is_none()
            {
                tile.dying = Some(now);
                // If new terrain or near chunks overlap, fade them in over
                // this tile; a second fade-out pattern would open holes.
                tile.alone = !kept.overlaps(key);
            }
            false
        });

        if let Some(cutout) = pending.cutout.take() {
            if pending.meshes.is_empty() {
                self.set_lod_cutout(cutout);
            } else {
                // A column keeps its old cutout until the tile replacing it
                // can be shown and cast, including a tile held by a retry.
                let mut ready = cutout.clone();
                for &(x, z) in self.lod_cutout.symmetric_difference(&cutout) {
                    if overlaps_withheld(TileKey { x, z, level: 0 }) {
                        if self.lod_cutout.contains(&(x, z)) {
                            ready.insert((x, z));
                        } else {
                            ready.remove(&(x, z));
                        }
                    }
                }
                if ready != cutout {
                    pending.cutout = Some(cutout);
                }
                if ready != self.lod_cutout {
                    self.set_lod_cutout(ready);
                }
            }
        }
        if !pending.meshes.is_empty() || !pending.removed.is_empty() {
            pending.needs_finish = false;
            self.lod_uploads.pending = Some(pending);
        }
    }

    /// The tiles drawn for good, not fading out (`render-capture`).
    pub(super) fn standing_lods(&self) -> FxHashSet<TileKey> {
        self.lods
            .iter()
            .filter(|(_, tile)| tile.mesh.is_some() && tile.dying.is_none())
            .map(|(&key, _)| key)
            .collect()
    }

    /// The chunk columns the resident tiles now leave out (`Lod::take_cutout`).
    /// Removed sections they no longer leave out are released.
    pub fn set_lod_cutout(&mut self, cutout: crate::fast_hash::FxHashSet<(i32, i32)>) {
        self.lod_cutout = cutout;
        let released: Vec<_> = self
            .lod_retiring
            .iter()
            .copied()
            .filter(|key| !self.lod_cutout.contains(&(key.0, key.2)))
            .collect();
        for key in released {
            self.lod_retiring.remove(&key);
            self.leave_section(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        external::{ShadowView, shadow_box},
        native_shadow, pack_uniforms,
        tests::section,
    };
    use super::*;
    use crate::mesher::TerrainVertex;

    fn tile(solid: u32, y: f32, shown: Option<Instant>) -> Tile {
        Tile {
            mesh: Some(Box::new(section(solid, 1, None))),
            info: MeshInfo {
                bounds: Bounds {
                    min: Vec3::new(1.0, y, 1.0),
                    max: Vec3::new(15.0, y + 4.0, 15.0),
                },
                cutouts: None,
            },
            shown,
            dying: None,
            alone: false,
            previous: None,
            shadow_previous: None,
            awaits_publication: shown.is_none(),
            shadow_waits: shown.is_none(),
            cull_slot: 0,
        }
    }

    fn collect_shadow(tiles: &FxHashMap<TileKey, Tile>, camera: DVec3) -> ShadowTiles {
        let light = Vec3::Y;
        let settings = horizon_shader_api::manifest::Shadow {
            distance: 128.0,
            ..Default::default()
        };
        let shadow = native_shadow::camera(&settings);
        let cull = ShadowCull {
            light: shadow_box(&ShadowView {
                cull: pack_uniforms::shadow_culling_at(light, camera, &shadow),
                light,
                distance: None,
                unculled: None,
                every_face: false,
                view_casters: false,
                depth_planes: true,
            }),
            casters: None,
            reach: None,
        };
        let mut shadow = ShadowTiles::default();
        for (&key, tile) in tiles {
            shadow.consider(key, tile, camera, &cull, light, tiles);
        }
        shadow.finish(tiles);
        shadow
    }

    #[test]
    fn native_shadows_take_solid_lod_in_the_light_even_off_view() {
        let now = Instant::now();
        let camera = DVec3::new(0.5, 80.0, 0.5);
        let key = |x| TileKey { x, z: 0, level: 0 };
        let off_view = key(3);
        let tiles = [
            (off_view, tile(1, 74.0, Some(now))),
            (key(10), tile(1, 74.0, Some(now))), // outside the light box
            (key(5), tile(0, 74.0, Some(now))),  // translucent only
            (key(6), tile(1, 74.0, None)),       // still uploading
        ]
        .into_iter()
        .collect::<FxHashMap<_, _>>();
        let view = Frustum::boxed(Vec3::splat(-128.0), Vec3::new(0.0, 128.0, 128.0));
        assert!(
            tiles[&off_view]
                .visible(off_view, camera, &view, now)
                .is_none()
        );
        let casters = collect_shadow(&tiles, camera);
        assert_eq!(
            casters
                .tiles
                .iter()
                .map(|tile| tile.key)
                .collect::<Vec<_>>(),
            [off_view]
        );
        let detail = section(1, 0, None);
        assert!(
            casters.casts_section((0, 8, 1), &detail, &tiles),
            "ordinary full detail casts"
        );
    }

    #[test]
    fn shadow_handoff_uses_the_mesh_cutouts_even_when_the_global_cutout_is_still_pending() {
        let now = Instant::now();
        let camera = DVec3::new(0.5, 80.0, 0.5);
        let parent = TileKey {
            x: 2,
            z: 0,
            level: 1,
        };
        let child = TileKey {
            x: 4,
            z: 0,
            level: 0,
        };
        let near = (5, 0);
        let near_key = (near.0, 8, near.1);
        let detail = section(1, 0, None);
        let mut tiles = [
            (parent, tile(1, 74.0, Some(now))),
            (child, tile(2, 74.0, Some(now))),
        ]
        .into_iter()
        .collect::<FxHashMap<_, _>>();
        tiles.get_mut(&parent).unwrap().info.bounds.max.x = 31.0;

        // The child arrived alone. The parent's still-published mesh covers both columns.
        let casters = collect_shadow(&tiles, camera);
        assert_eq!(
            casters
                .tiles
                .iter()
                .map(|tile| tile.key)
                .collect::<Vec<_>>(),
            [parent]
        );
        assert!(!casters.casts_section(near_key, &detail, &tiles));

        // The replacement parent clips the uploaded child and one full-detail column.
        // Other meshes in the planner's global cutout can still be in flight.
        tiles.get_mut(&parent).unwrap().info.cutouts = Some(Arc::new(MeshCutouts {
            near: vec![near],
            finer: vec![child],
        }));
        let casters = collect_shadow(&tiles, camera);
        assert!(casters.keys.contains(&parent));
        assert!(casters.keys.contains(&child));
        assert!(casters.casts_section(near_key, &detail, &tiles));
        assert!(!casters.casts_section((child.x, 8, child.z), &detail, &tiles));

        let mut leaving = detail;
        leaving.leaving = Some(now);
        assert!(!casters.casts_section(near_key, &leaving, &tiles));
    }

    #[test]
    fn a_first_clipped_parent_waits_for_the_child_to_cast() {
        let now = Instant::now();
        let child = TileKey {
            x: 4,
            z: 0,
            level: 0,
        };
        let parent = child.parent();
        let mut clipped = tile(2, 74.0, None);
        clipped.info.cutouts = Some(Arc::new(MeshCutouts {
            near: Vec::new(),
            finer: vec![child],
        }));
        let mut tiles = [(parent, clipped)].into_iter().collect();
        let mut empty = FxHashSet::default();
        let removed = FxHashSet::default();
        finish_uploads(
            &mut tiles,
            &mut empty,
            &mut Vec::new(),
            &removed,
            &TileCoverage::default(),
            now,
            |_| now,
        );
        assert!(tiles[&parent].shown.is_some());
        assert!(tiles[&parent].shadow().is_none(), "child has not arrived");

        tiles.insert(child, tile(3, 74.0, None));
        finish_uploads(
            &mut tiles,
            &mut empty,
            &mut Vec::new(),
            &removed,
            &TileCoverage::default(),
            now,
            |_| now,
        );
        let casters = collect_shadow(&tiles, DVec3::new(0.5, 80.0, 0.5));
        assert_eq!(casters.keys, [parent, child].into_iter().collect());
    }

    #[test]
    fn finished_tiles_can_publish_beside_a_waiting_upload() {
        let now = Instant::now();
        let child = TileKey {
            x: 4,
            z: 0,
            level: 0,
        };
        let parent = child.parent();
        let disjoint = TileKey { x: 8, ..child };
        let mut tiles = [
            (parent, tile(1, 74.0, None)),
            (disjoint, tile(1, 74.0, None)),
        ]
        .into_iter()
        .collect();
        let unfinished = TileCoverage::new([child]);
        finish_uploads(
            &mut tiles,
            &mut FxHashSet::default(),
            &mut Vec::new(),
            &FxHashSet::default(),
            &unfinished,
            now,
            |_| now,
        );
        assert!(tiles[&parent].shown.is_none());
        assert!(tiles[&parent].shadow().is_none());
        assert!(tiles[&disjoint].shown.is_some());
        assert!(tiles[&disjoint].shadow().is_some());
    }

    #[test]
    fn a_held_handoff_restarts_its_inherited_world_fade() {
        let started = Instant::now();
        let expired = started + FADE;
        let child = TileKey {
            x: 4,
            z: 0,
            level: 0,
        };
        let parent = child.parent();
        let old = tile(1, 74.0, Some(started));
        let mut next = tile(2, 74.0, Some(started));
        next.awaits_publication = true;
        next.previous = Some(Box::new(StandIn {
            mesh: *old.mesh.unwrap(),
            info: old.info,
            since: Some(started),
            casts_shadow: false,
        }));
        let mut tiles = [(parent, next)].into_iter().collect();
        let mut empty = FxHashSet::default();
        let mut slots = Vec::new();
        let removed = FxHashSet::default();
        finish_uploads(
            &mut tiles,
            &mut empty,
            &mut slots,
            &removed,
            &TileCoverage::new([child]),
            expired,
            |_| expired,
        );
        finish_uploads(
            &mut tiles,
            &mut empty,
            &mut slots,
            &removed,
            &TileCoverage::default(),
            expired,
            |_| expired,
        );
        let parent = tiles.get_mut(&parent).unwrap();
        assert!(parent.release_visible_stand_in(expired).is_none());
        assert_eq!(parent.drawn().solid, 1);
        assert!(parent.release_visible_stand_in(expired + FADE).is_some());
        assert_eq!(parent.drawn().solid, 2);
    }

    #[test]
    fn an_empty_replacement_releases_the_old_caster_when_its_child_finishes() {
        let now = Instant::now();
        let camera = DVec3::new(0.5, 80.0, 0.5);
        let child = TileKey {
            x: 4,
            z: 0,
            level: 0,
        };
        let parent = child.parent();
        for child_empty in [false, true] {
            let Handoff {
                shown,
                previous,
                shadow_previous,
            } = tile(1, 74.0, Some(now)).handoff(false, true, |_| {
                panic!("empty replacement dropped the live shadow at upload");
            });
            let mut replacement = Tile {
                mesh: None,
                shown,
                previous,
                shadow_previous,
                cull_slot: 7,
                ..tile(0, 0.0, None)
            };
            replacement.info.cutouts = Some(Arc::new(MeshCutouts {
                near: Vec::new(),
                finer: vec![child],
            }));
            let mut tiles = [(parent, replacement)].into_iter().collect();
            let mut empty = FxHashSet::default();
            let mut slots = Vec::new();
            let removed = FxHashSet::default();
            let expired = now + FADE;
            assert!(
                finish_uploads(
                    &mut tiles,
                    &mut empty,
                    &mut slots,
                    &removed,
                    &TileCoverage::default(),
                    expired,
                    |_| now
                )
                .is_empty()
            );
            assert!(empty.is_empty() && slots.is_empty());
            assert_eq!(
                collect_shadow(&tiles, camera).keys,
                [parent].into_iter().collect()
            );

            let mut child_tile = tile(3, 74.0, None);
            if child_empty {
                child_tile.mesh = None;
            }
            child_tile.cull_slot = 8;
            tiles.insert(child, child_tile);
            let released = finish_uploads(
                &mut tiles,
                &mut empty,
                &mut slots,
                &removed,
                &TileCoverage::default(),
                expired,
                |_| now,
            );
            assert_eq!(
                released.iter().map(|mesh| mesh.solid).collect::<Vec<_>>(),
                [1]
            );
            assert!(empty.contains(&parent));
            assert_eq!(empty.contains(&child), child_empty);
            assert!(slots.contains(&7));
            assert_eq!(
                collect_shadow(&tiles, camera).keys.contains(&child),
                !child_empty
            );
            assert!(!tiles.contains_key(&parent));
        }
    }

    #[test]
    fn a_second_upload_keeps_the_last_published_shadow_through_the_first_fade() {
        let now = Instant::now();
        let mut current = tile(1, 74.0, Some(now));
        let replace = |solid: u32, old: Tile| {
            let Handoff {
                shown,
                previous,
                shadow_previous,
            } = old.handoff(true, true, |_| {
                panic!("published mesh released before its replacement finished");
            });
            Tile {
                shown,
                previous,
                shadow_previous,
                ..tile(solid, 74.0, None)
            }
        };
        current = replace(2, current);
        assert_eq!(current.shadow().unwrap().0.solid, 1);
        assert!(current.finish_upload(now, true).is_none());
        assert_eq!(current.shadow().unwrap().0.solid, 2);
        assert_eq!(current.drawn().solid, 1);

        current = replace(3, current);
        assert_eq!(current.shadow().unwrap().0.solid, 2);
        assert_eq!(current.drawn().solid, 1);
        assert_eq!(
            current
                .finish_upload(now + Duration::from_millis(10), true)
                .unwrap()
                .solid,
            2
        );
        assert_eq!(current.shadow().unwrap().0.solid, 3);
    }

    #[test]
    fn parent_shadow_waits_for_its_finer_mesh_even_after_the_visual_fade() {
        let now = Instant::now();
        let camera = DVec3::new(0.5, 80.0, 0.5);
        let parent = TileKey {
            x: 2,
            z: 0,
            level: 1,
        };
        let outgoing = TileKey {
            x: 4,
            z: 0,
            level: 0,
        };
        let incoming = TileKey {
            x: 5,
            z: 0,
            level: 0,
        };
        let clip = |child| {
            Some(Arc::new(MeshCutouts {
                near: Vec::new(),
                finer: vec![child],
            }))
        };
        let mut old = tile(1, 74.0, Some(now));
        old.info.cutouts = clip(outgoing);
        let Handoff {
            shown,
            previous,
            shadow_previous,
        } = old.handoff(true, true, |_| {
            panic!("the old parent must stay in the arena");
        });
        let mut new = Tile {
            shown,
            previous,
            shadow_previous,
            ..tile(2, 74.0, None)
        };
        new.info.cutouts = clip(incoming);
        let mut tiles = [(parent, new), (outgoing, tile(1, 74.0, Some(now)))]
            .into_iter()
            .collect();
        let mut removed = FxHashSet::default();
        removed.insert(outgoing);
        let mut empty = FxHashSet::default();

        // The worker delivered the clipped parent in its own batch. The old child has
        // left the plan, but the old parent still leaves that part of its footprint out.
        assert!(
            finish_uploads(
                &mut tiles,
                &mut empty,
                &mut Vec::new(),
                &removed,
                &TileCoverage::default(),
                now,
                |_| now
            )
            .is_empty()
        );
        tiles.get_mut(&outgoing).unwrap().dying = Some(now);
        assert_eq!(tiles[&parent].shadow().unwrap().0.solid, 1);
        assert!(collect_shadow(&tiles, camera).keys.contains(&outgoing));
        let later = now + FADE;
        assert!(
            tiles
                .get_mut(&parent)
                .unwrap()
                .release_visible_stand_in(later)
                .is_none()
        );
        assert_eq!(tiles[&parent].drawn().solid, 2);
        assert_eq!(tiles[&parent].shadow().unwrap().0.solid, 1);

        // The child really uploads. One batch publishes the parent and new child, and the
        // old child is no longer needed even if its GPU mesh has yet to be reclaimed.
        tiles.insert(incoming, tile(3, 74.0, None));
        let released = finish_uploads(
            &mut tiles,
            &mut empty,
            &mut Vec::new(),
            &FxHashSet::default(),
            &TileCoverage::default(),
            later,
            |_| later,
        );
        assert_eq!(released.iter().map(|s| s.solid).collect::<Vec<_>>(), [1]);
        assert_eq!(tiles[&parent].shadow().unwrap().0.solid, 2);
        let casters = collect_shadow(&tiles, camera);
        assert_eq!(casters.keys, [parent, incoming].into_iter().collect());
    }

    #[test]
    fn a_waiting_parent_partitions_its_published_near_cutouts() {
        let now = Instant::now();
        let camera = DVec3::new(0.5, 80.0, 0.5);
        let parent = TileKey {
            x: 1,
            z: 0,
            level: 2,
        };
        let single = TileKey {
            x: 4,
            z: 2,
            level: 0,
        };
        let coarse = TileKey {
            x: 2,
            z: 0,
            level: 1,
        };
        let missing = TileKey {
            x: 7,
            z: 2,
            level: 0,
        };
        let held_section = (5, 8, 0);
        let mut parent_mesh = tile(2, 74.0, Some(now));
        parent_mesh.info.cutouts = Some(Arc::new(MeshCutouts {
            near: Vec::new(),
            finer: vec![single, coarse, missing],
        }));
        let mut previous = tile(1, 74.0, Some(now));
        previous.info.cutouts = Some(Arc::new(MeshCutouts {
            near: vec![(single.x, single.z), (held_section.0, held_section.2)],
            finer: Vec::new(),
        }));
        let Handoff {
            previous,
            shadow_previous,
            ..
        } = previous.handoff(true, true, |_| {
            panic!("the old parent must stay in the arena");
        });
        parent_mesh.previous = previous;
        parent_mesh.shadow_previous = shadow_previous;
        parent_mesh.shadow_waits = true;
        let mut tiles = [
            (parent, parent_mesh),
            (single, tile(3, 74.0, Some(now))),
            (coarse, tile(4, 74.0, Some(now))),
        ]
        .into_iter()
        .collect();
        let mut retiring = section(1, 0, None);
        retiring.leaving = Some(now);
        let later = now + FADE;
        let removed = FxHashSet::default();
        let mut uploads = Uploads::default();
        assert!(
            finish_uploads(
                &mut tiles,
                &mut uploads.empty,
                &mut Vec::new(),
                &removed,
                &TileCoverage::default(),
                now,
                |_| now
            )
            .is_empty()
        );
        assert!(retiring.past_lod_fade(later));
        assert!(retired_near_needed(&tiles, held_section));
        let casters = collect_shadow(&tiles, camera);
        assert_eq!(casters.keys, [parent, single].into_iter().collect());
        assert!(
            casters.casts_section(held_section, &retiring, &tiles),
            "the partly clipped coarse tile cannot replace this column yet"
        );
        assert!(
            !casters.casts_section((single.x, 8, single.z), &retiring, &tiles),
            "the level-zero replacement already owns this column"
        );

        // An empty child satisfies the cutout, including after the renderer moves its
        // upload state as one value in `take_world`.
        uploads.empty.insert(missing);
        let mut rebuilt = Uploads::default();
        std::mem::swap(&mut uploads, &mut rebuilt);
        finish_uploads(
            &mut tiles,
            &mut rebuilt.empty,
            &mut Vec::new(),
            &removed,
            &TileCoverage::default(),
            later,
            |_| later,
        );
        assert!(!retired_near_needed(&tiles, held_section));
        let casters = collect_shadow(&tiles, camera);
        assert_eq!(casters.keys, [parent, single, coarse].into_iter().collect());
        assert!(
            !casters.casts_section(held_section, &retiring, &tiles),
            "the published coarse mesh owns this column"
        );
    }

    #[test]
    fn tiles_fade_in_on_their_own_grid() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        // terrain.wgsl's `faded` reads the level and sixty-fourths back.
        let decode = |code: u32| ((code - 1) / 129, (code - 1) % 129);
        assert_eq!(fade_code(None, 0, at(0)), None, "waits for its job");
        assert_eq!(fade_code(Some(start), 3, at(0)).map(decode), Some((3, 1)));
        let mut last = 0;
        for ms in [0, 100, 300, 500, 590] {
            let (level, shown) = decode(fade_code(Some(start), 10, at(ms)).unwrap());
            assert_eq!(level, 10);
            assert!(shown > last && shown < 64, "{shown} at {ms} ms");
            last = shown;
        }
        assert_eq!(fade_code(Some(start), 10, at(600)), Some(0), "whole");
        // Fading out: 65 to 128, then nothing drawn (128) until released.
        let mut last = 64;
        for ms in [0, 100, 300, 500, 590] {
            let (level, out) = decode(fade_out_code(start, 3, at(ms)));
            assert_eq!(level, 3);
            assert!(out > last && out < 128, "{out} at {ms} ms");
            last = out;
        }
        assert_eq!(decode(fade_out_code(start, 3, at(700))), (3, 128));
        // A section's largest code keeps its UV step exact (terrain_uv).
        let w = 129.0f32 + 1.0 / 32768.0;
        assert_eq!(w.fract(), 1.0 / 32768.0);
    }

    #[test]
    fn joined_jobs_apply_as_if_in_turn() {
        let key = |x| TileKey { x, z: 0, level: 0 };
        let mut pending = Pending::default();
        // The first job meshes 1 and 2 and retires 3 and 4.
        pending.add(
            vec![
                (key(1), SectionMesh::default()),
                (key(2), SectionMesh::default()),
            ],
            vec![key(3), key(4)],
            Some([(0, 0)].into_iter().collect()),
        );
        // The second meshes 3 again and retires 2.
        pending.add(vec![(key(3), SectionMesh::default())], vec![key(2)], None);
        let mut meshes: Vec<_> = pending.meshes.keys().map(|k| k.x).collect();
        meshes.sort_unstable();
        let mut removed: Vec<_> = pending.removed.iter().map(|k| k.x).collect();
        removed.sort_unstable();
        assert_eq!((meshes, removed), (vec![1, 3], vec![2, 4]));
        assert!(pending.cutout.as_ref().is_some_and(|c| c.contains(&(0, 0))));
        // They go up in the order handed over, each once.
        pending.add(vec![(key(1), SectionMesh::default())], vec![], None);
        let order: Vec<_> = pending
            .order
            .iter()
            .filter(|k| pending.meshes.contains_key(k))
            .map(|k| k.x)
            .collect();
        assert_eq!(order, [1, 3]);
    }

    #[test]
    fn a_refused_upload_waits_its_turn_and_is_superseded_by_new_work() {
        let mut pending = Pending::default();
        let now = Instant::now();
        let first = TileKey {
            x: 2,
            z: 0,
            level: 0,
        };
        let second = TileKey { x: 3, ..first };
        let solid = TerrainVertex::tiled([1.0; 3], [0.0; 2], 0, 0, 0);
        pending.add(
            vec![
                (
                    first,
                    SectionMesh {
                        solid: vec![solid; 4],
                        ..Default::default()
                    },
                ),
                (second, SectionMesh::default()),
            ],
            vec![],
            None,
        );
        let (key, mesh) = pending.take_next(now).unwrap();
        assert_eq!(key, first);
        pending.retry(key, mesh, now);
        assert_eq!(pending.take_next(now).unwrap().0, second);
        assert!(pending.take_next(now).is_none());
        let later = now + Duration::from_secs(1);
        let (key, mesh) = pending.take_next(later).unwrap();
        assert_eq!(key, first);
        assert_eq!(mesh.solid.len(), 4);
        pending.retry(key, mesh, later);
        pending.add(vec![(first, SectionMesh::default())], vec![], None);
        assert!(pending.take_next(later).unwrap().1.solid.is_empty());
        assert!(pending.take_next(later + Duration::from_secs(1)).is_none());
    }

    #[test]
    fn large_tile_bounds_include_water_and_do_not_cull_by_origin() {
        let vertex = |pos| TerrainVertex::tiled(pos, [0.0; 2], 0, 0, 0);
        let mesh = SectionMesh {
            solid: vec![vertex([0.0, -64.0, 0.0]), vertex([1024.0, 100.0, 1024.0])],
            translucent: vec![vertex([512.0, 120.0, 512.0])],
            bounds: Some([Vec3::new(0.0, -64.0, 0.0), Vec3::new(1024.0, 120.0, 1024.0)]),
            ..Default::default()
        };
        let bounds = Bounds::mesh(&mesh).unwrap();
        assert_eq!(bounds.min, Vec3::new(0.0, -64.0, 0.0));
        assert_eq!(bounds.max, Vec3::new(1024.0, 120.0, 1024.0));
        let view = Frustum {
            planes: super::super::frustum(
                super::super::world_projection(70.0, 16.0 / 9.0, 8192.0)
                    * crate::camera::view_matrix(0.0, 0.0),
            ),
        };
        // The tile origin is far behind, but its far edge reaches the camera.
        // Negative world origins therefore remain visible.
        assert!(
            bounds
                .visible(DVec3::new(-1000.0, -80.0, -512.0), &view)
                .is_some()
        );
        // Wholly behind the camera: out of view.
        assert!(
            bounds
                .visible(DVec3::new(-512.0, -80.0, -2000.0), &view)
                .is_none()
        );
        assert!(Bounds::mesh(&SectionMesh::default()).is_none());
    }
}
