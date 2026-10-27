//! Far meshes share the terrain arena and passes, with their actual bounds
//! rather than the 16³ boxes used by the near-section residency walk.
use super::{Frustum, Renderer, SectionGpu};
use crate::{
    fast_hash::{FxHashMap, FxHashSet},
    lod::TileKey,
    mesher::SectionMesh,
};
use glam::{DVec3, Vec3};
use std::time::{Duration, Instant};

/// How long a tile takes to fade in, and the tiles it replaces to fade out: the two draw
/// opposite pixels of a dither pattern (`terrain.wgsl`'s `faded_out`), so detail changing
/// with distance blends instead of popping, and each pixel still shows one opaque surface
/// (no sorting, depth and shadow passes unchanged).
const FADE: Duration = Duration::from_millis(600);

/// Finished tile meshes not uploaded yet, the tiles they replace (removed
/// once every mesh is in) and the columns they leave out.
#[derive(Default)]
pub(super) struct Pending {
    meshes: FxHashMap<TileKey, SectionMesh>,
    removed: FxHashSet<TileKey>,
    cutout: Option<FxHashSet<(i32, i32)>>,
    /// Tiles uploaded for these meshes, new ones hidden until all are in.
    uploaded: Vec<TileKey>,
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
            self.meshes.insert(key, mesh);
        }
        for key in removed {
            self.meshes.remove(&key);
            self.removed.insert(key);
        }
        if cutout.is_some() {
            self.cutout = cutout;
        }
    }
}

pub(super) struct Tile {
    pub mesh: SectionGpu,
    bounds: Bounds,
    /// When it began to fade in: `None` while the job it came with uploads.
    shown: Option<Instant>,
    /// When it began to fade out, replaced.
    leaving: Option<Instant>,
}

impl Tile {
    /// Its instance's `w` (`instance`) at `now`; `None` hidden.
    fn fade(&self, now: Instant) -> Option<f32> {
        let progress = |since: Instant| (now - since).as_secs_f32() / FADE.as_secs_f32();
        match (self.leaving, self.shown) {
            (Some(since), _) => Some(4.0 + progress(since).min(0.999)),
            (None, Some(since)) if progress(since) < 1.0 => Some(2.0 + progress(since)),
            (None, Some(_)) => Some(1.0 / crate::mesher::TILED_UV_STEPS),
            (None, None) => None,
        }
    }
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}

impl Bounds {
    fn mesh(mesh: &SectionMesh) -> Option<Self> {
        let mut positions = mesh
            .solid
            .iter()
            .chain(&mesh.translucent)
            .map(|v| Vec3::from(v.pos));
        let first = positions.next()?;
        let mut bounds = Self {
            min: first,
            max: first,
        };
        for pos in positions {
            bounds.min = bounds.min.min(pos);
            bounds.max = bounds.max.max(pos);
        }
        Some(bounds)
    }

    fn visible(self, origin: DVec3, distance: f32, frustum: &Frustum) -> Option<u64> {
        let min = origin + self.min.as_dvec3();
        let max = origin + self.max.as_dvec3();
        let nearest = DVec3::ZERO.clamp(min, max);
        if nearest.x * nearest.x + nearest.z * nearest.z > f64::from(distance).powi(2)
            || !frustum.intersects(min.as_vec3(), max.as_vec3())
        {
            return None;
        }
        Some(((min + max) * 0.5).length_squared().to_bits())
    }
}

/// A tile's instance: like a section's, with the UV step of merged faces, or its fade
/// (`Tile::fade`, `terrain.wgsl`'s `terrain_uv`).
pub(super) fn instance(v: &Visible) -> [f32; 4] {
    [v.origin[0], v.origin[1], v.origin[2], v.fade]
}

pub(super) struct Visible {
    pub key: TileKey,
    pub distance: u64,
    pub origin: [f32; 3],
    fade: f32,
}

impl Tile {
    pub fn visible(
        &self,
        key: TileKey,
        camera: DVec3,
        distance: f32,
        frustum: &Frustum,
        now: Instant,
    ) -> Option<Visible> {
        let fade = self.fade(now)?;
        let origin = DVec3::from(key.origin().map(f64::from)) - camera;
        Some(Visible {
            key,
            distance: self.bounds.visible(origin, distance, frustum)?,
            origin: origin.as_vec3().to_array(),
            fade,
        })
    }
}

impl Renderer {
    /// A tile meshed again keeps showing; a new one waits for `show`.
    pub fn upload_lod(&mut self, key: TileKey, mesh: &SectionMesh) {
        let shown = self
            .lods
            .get(&key)
            .filter(|t| t.leaving.is_none())
            .and_then(|t| t.shown);
        self.remove_lod(key);
        if let Some(bounds) = Bounds::mesh(mesh)
            && let Some(mesh) = self.upload_mesh(mesh)
        {
            self.lods.insert(
                key,
                Tile {
                    mesh,
                    bounds,
                    shown,
                    leaving: None,
                },
            );
        }
    }

    pub fn remove_lod(&mut self, key: TileKey) {
        if let Some(tile) = self.lods.remove(&key) {
            self.release_mesh(tile.mesh);
        }
    }

    pub fn clear_lods(&mut self) {
        self.lod_pending = None;
        for (_, tile) in std::mem::take(&mut self.lods) {
            self.release_mesh(tile.mesh);
        }
        self.set_lod_cutout(Default::default());
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
        self.lod_pending
            .get_or_insert_default()
            .add(meshes, removed, cutout);
    }

    /// Uploads queued tile meshes for about `budget` (at least one), then,
    /// once all are in, fades them in and the tiles they replace out (`FADE`).
    pub fn upload_lods(&mut self, budget: Duration) {
        let start = Instant::now();
        let faded: Vec<_> = self
            .lods
            .iter()
            .filter(|(_, t)| t.leaving.is_some_and(|since| start - since >= FADE))
            .map(|(&k, _)| k)
            .collect();
        for key in faded {
            self.remove_lod(key);
        }
        while let Some(pending) = &mut self.lod_pending {
            let Some(&key) = pending.meshes.keys().next() else {
                let pending = self.lod_pending.take().expect("pending tiles");
                for key in pending.uploaded {
                    if let Some(tile) = self.lods.get_mut(&key) {
                        tile.shown.get_or_insert(start);
                    }
                }
                for key in pending.removed {
                    if let Some(tile) = self.lods.get_mut(&key) {
                        tile.leaving.get_or_insert(start);
                    }
                }
                if let Some(cutout) = pending.cutout {
                    self.set_lod_cutout(cutout);
                }
                return;
            };
            let mesh = pending.meshes.remove(&key).expect("pending tile");
            self.upload_lod(key, &mesh);
            if let Some(pending) = &mut self.lod_pending {
                pending.uploaded.push(key);
            }
            if start.elapsed() >= budget {
                return;
            }
        }
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
            self.release_section(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::TerrainVertex;

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
        assert!(pending.cutout.is_some_and(|c| c.contains(&(0, 0))));
    }

    #[test]
    fn large_tile_bounds_include_water_and_do_not_cull_by_origin() {
        let vertex = |pos| TerrainVertex::tiled(pos, [0.0; 2], 0, 0, 0);
        let mesh = SectionMesh {
            solid: vec![vertex([0.0, -64.0, 0.0]), vertex([1024.0, 100.0, 1024.0])],
            translucent: vec![vertex([512.0, 120.0, 512.0])],
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
        // The tile origin is farther than the range, but its far edge reaches
        // the camera. Negative world origins therefore remain visible.
        assert!(
            bounds
                .visible(DVec3::new(-1000.0, -80.0, -512.0), 256.0, &view)
                .is_some()
        );
        assert!(
            bounds
                .visible(DVec3::new(-2000.0, -80.0, -512.0), 256.0, &view)
                .is_none()
        );
        assert!(Bounds::mesh(&SectionMesh::default()).is_none());
    }
}
