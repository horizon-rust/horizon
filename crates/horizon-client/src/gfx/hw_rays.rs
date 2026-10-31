//! RT lighting's hybrid (docs/raytracing.md): the terrain as it is drawn, in hardware acceleration
//! structures, so the sun's and the light sources' shadow rays meet the exact triangles, their
//! texels alpha-tested, where the GPU traces rays (`Features::EXPERIMENTAL_RAY_QUERY`). The voxel
//! grid (`voxels`) keeps the diffuse light: the sky, global illumination, what the rays bounce off.
//!
//! Each section near the camera (`RADIUS`) gets a bottom-level structure over its solid quads,
//! read where they lie in the vertex arena with the shared quad indices: no copy, and one
//! geometry left non-opaque so every candidate hit reaches the shader's alpha test (opaque
//! texels confirm at once), and a second over its translucent quads (stained glass, water, ice),
//! which tint the light that crosses them. The top-level structure, made again each frame, holds
//! them relative to the camera, each instance's custom data its row in `instances`: where its
//! quads are in the arena, for the alpha test and the tint to read their texels.
//!
//! The moving geometry joins them as Remix's does, made again each frame (`finish`): the entities'
//! quads as drawn (a dynamic structure over the vertices copied to `dynamics`, rebuilt every frame
//! with the fast-build flag), the first-person view model (the arms and the held item) in a
//! structure of its own, and the cloud tiles (`rt_cloud_mesh`), one structure each, built once and
//! moved by the wind in the top-level structure. Their rows in `instances` say what they are (a
//! kind in the first word, which the terrain's paths read as an arena page they don't have), so
//! the shader's terrain code is left as it was.

use super::SectionKey;
use super::rt_cloud_mesh::{CloudTile, TileKey};
use crate::mesher::Vertex;
use std::collections::HashMap;

/// Sections within this many chunks of the camera's have a structure: the voxel grid's reach,
/// where the rays run.
pub const RADIUS: i32 = super::voxels::RADIUS;
/// The vertex arena's pages the rays read (shadow_rays_hw.wgsl `ARENA_PAGES`, bound at 16 and 40
/// on): quads on a later page are taken as opaque, without their texels.
pub const ARENA_PAGES: usize = 8;
/// The bindings of the arena's pages after the first (16).
pub const ARENA_BINDINGS: [u32; ARENA_PAGES - 1] = [40, 41, 42, 43, 44, 45, 46];
/// Instances the top-level structure holds at most (a 17x17-chunk square is about 7000 sections).
const MAX_INSTANCES: u32 = 16384;
/// Of them, those kept for the moving geometry: the entities, the view model and the cloud tiles
/// (13 by 13 at most).
const DYNAMIC_INSTANCES: u32 = 256;
/// What an instance is, in the first word of its row (shadow_rays_hw.wgsl `DYN_ENTITY`,
/// `DYN_CLOUD`); below these it is a terrain section's arena page.
const KIND_ENTITY: u32 = 0xffff_fff0;
const KIND_CLOUD: u32 = 0xffff_fff1;
/// The clouds' ray mask bit: the bounce rays leave it out (shadow_rays_hw.wgsl `NO_CLOUDS`), since
/// what they hit is lit from the voxel grid and the cache, which know nothing of the sky's slab.
const CLOUD_MASK: u8 = 0x04;
/// Cloud structures built a frame at most.
const CLOUDS_PER_FRAME: usize = 32;
/// Entity vertices the dynamic structures take at most, so a crowd stays bounded.
const MAX_DYNAMIC_VERTICES: usize = 1 << 19;
/// How far the clouds drift before their instances are placed again, in blocks (a shadow a block
/// off, 700 blocks up, is far under what the eye sees).
const CLOUD_DRIFT: f64 = 1.0;
/// Bottom-level structures built a frame at most, so arriving in a new place spreads them out.
const BUILDS_PER_FRAME: usize = 128;

/// A section's structure and where its quads were when it was built: its page, first vertex,
/// solid quads, and which of its geometries is the translucent one (2: none).
/// A stale structure's page (`HwRays::forget`): past every arena page, so the rays find no quads
/// of it to read (shadow_rays_hw.wgsl `hw_quad`) and take its hits as opaque.
const PAGE_GONE: u32 = u32::MAX;
/// How many frames a stale structure stands in for a section's new mesh at most.
const STALE_FRAMES: u32 = 30;
struct Built {
    blas: wgpu::Blas,
    page: u32,
    start: u64,
    solid: u32,
    translucent_geometry: u32,
}

pub struct HwRays {
    built: HashMap<SectionKey, Built>,
    /// Sections meshed again whose old structure stands in until the new one is built, and the
    /// frames it has stood in: the section never leaves the rays between its two meshes (it
    /// would let the light through it for a frame, the world flickering as a block is placed).
    stale: HashMap<SectionKey, u32>,
    pub tlas: wgpu::Tlas,
    /// Where each instance's quads are, by its custom data (shadow_rays_hw.wgsl `instances`):
    /// the page, the solid quads' first vertex, the translucent ones', the translucent geometry.
    pub instances: wgpu::Buffer,
    /// Instances set in `tlas` last frame.
    used: usize,
    /// Where the instances are placed from (a block, `ANCHOR_REACH` from the camera at most): the
    /// top-level structure is made again only when it moves or the sections change, not each
    /// frame the camera moves (the rays add the camera's offset from it, `offset`).
    anchor: Option<glam::DVec3>,
    /// The sections changed since the top-level structure was made.
    changed: bool,
    /// Instances the terrain's sections take (the moving ones follow), and whether their slots
    /// changed since the top-level structure was made.
    terrain: usize,
    terrain_changed: bool,
    /// The moving geometry's vertices (`mesher::Vertex`: the entities', then the view model's),
    /// which the shader reads and the structures build from, and how many fit.
    pub dynamics: wgpu::Buffer,
    capacity: usize,
    /// Structures for the entities and the view model, and the quads each was made to hold.
    entities: Option<(wgpu::Blas, u32)>,
    hand: Option<(wgpu::Blas, u32)>,
    /// Each cloud tile's structure, and where the first tile was when the instances were placed.
    clouds: HashMap<TileKey, wgpu::Blas>,
    cloud_origin: Option<glam::DVec3>,
    /// Moving instances set in `tlas` last frame, and the view model's vertices moved to the world.
    moving: usize,
    hand_scratch: Vec<Vertex>,
}

/// The moving geometry of a frame, as drawn, for `HwRays::finish`.
pub struct Dynamic<'a> {
    /// The entities' quads, four vertices each, relative to the camera.
    pub entities: &'a [Vertex],
    /// The view model's quads, relative to the camera as the hand's own projection sees them.
    pub hand: &'a [Vertex],
    /// Takes the view model's vertices to where the world's projection shows what the hand's
    /// shows (Remix's view model perspective correction): the bob, then the fields of view's ratio.
    pub hand_to_world: glam::Mat4,
    /// The cloud tiles to put in the rays, when the mesh clouds are drawn.
    pub clouds: Vec<CloudTile<'a>>,
}

/// What the rays bind of the hybrid, owned (`Renderer::hw_bindings`).
pub struct Bindings {
    pub tlas: wgpu::Tlas,
    /// The arena's first `ARENA_PAGES` pages, the missing ones the first again.
    pub arena: [wgpu::Buffer; ARENA_PAGES],
    pub sprites: wgpu::TextureView,
    pub instances: wgpu::Buffer,
    pub dynamics: wgpu::Buffer,
    pub atlas: wgpu::TextureView,
}

impl Bindings {
    pub fn hardware(&self) -> super::shadow_rays::Hardware<'_> {
        super::shadow_rays::Hardware {
            tlas: &self.tlas,
            arena: &self.arena,
            sprites: &self.sprites,
            instances: &self.instances,
            dynamics: &self.dynamics,
            atlas: &self.atlas,
        }
    }
}

/// A triangle geometry of `quads` quads (four vertices, two triangles each).
fn quads_geometry(
    quads: u32,
    flags: wgpu::AccelerationStructureGeometryFlags,
) -> wgpu::BlasTriangleGeometrySizeDescriptor {
    wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: quads * 4,
        index_format: Some(wgpu::IndexFormat::Uint32),
        index_count: Some(quads * 6),
        flags,
    }
}

/// The build of a moving structure from the quads of `vertices` from vertex `first`.
fn moving_entry<'a>(
    blas: &'a wgpu::Blas,
    size: &'a wgpu::BlasTriangleGeometrySizeDescriptor,
    vertices: &'a wgpu::Buffer,
    indices: &'a wgpu::Buffer,
    first: u32,
) -> wgpu::BlasBuildEntry<'a> {
    wgpu::BlasBuildEntry {
        blas,
        geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
            size,
            vertex_buffer: vertices,
            first_vertex: first,
            vertex_stride: std::mem::size_of::<Vertex>() as u64,
            index_buffer: Some(indices),
            first_index: Some(0),
            transform_buffer: None,
            transform_buffer_offset: None,
        }]),
    }
}

/// Makes `slot`'s structure again for `quads` quads when it is missing or too small (a power of
/// two, so a growing crowd does not remake it every frame).
fn ensure_moving(
    device: &wgpu::Device,
    slot: &mut Option<(wgpu::Blas, u32)>,
    quads: u32,
    label: &'static str,
) {
    if slot.as_ref().is_some_and(|(_, held)| *held >= quads) {
        return;
    }
    let held = quads.next_power_of_two().max(64);
    let blas = device.create_blas(
        &wgpu::CreateBlasDescriptor {
            label: Some(label),
            // Built again each frame: the fast build, as Remix's dynamic structures.
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        },
        wgpu::BlasGeometrySizeDescriptors::Triangles {
            descriptors: vec![quads_geometry(
                held,
                wgpu::AccelerationStructureGeometryFlags::empty(),
            )],
        },
    );
    *slot = Some((blas, held));
}

/// How far the camera may move from the anchor before the instances are placed again, in blocks
/// (the rays' precision stays well under a millimetre this near).
const ANCHOR_REACH: f64 = 64.0;

/// A section to build: its key, its arena page and first vertex, its solid quads and its
/// translucent ones (after them).
pub struct ToBuild {
    pub key: SectionKey,
    pub page: u32,
    pub start: u64,
    pub quads: u32,
    pub translucent: u32,
}

impl HwRays {
    /// Whether `device` traces hardware rays (and `HORIZON_HW_RT` isn't 0).
    pub fn supported(device: &wgpu::Device) -> bool {
        device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
    }

    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            built: HashMap::new(),
            stale: HashMap::new(),
            tlas: device.create_tlas(&wgpu::CreateTlasDescriptor {
                label: Some("RT lighting's terrain"),
                max_instances: MAX_INSTANCES,
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            }),
            instances: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("RT lighting's terrain instances"),
                size: u64::from(MAX_INSTANCES) * 16,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            used: 0,
            anchor: None,
            changed: true,
            terrain: 0,
            terrain_changed: true,
            dynamics: Self::dynamics_buffer(device, 1024),
            capacity: 1024,
            entities: None,
            hand: None,
            clouds: HashMap::new(),
            cloud_origin: None,
            moving: 0,
            hand_scratch: Vec::new(),
        }
    }

    /// A buffer for `vertices` moving vertices.
    fn dynamics_buffer(device: &wgpu::Device, vertices: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RT lighting's moving geometry"),
            size: (vertices * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::BLAS_INPUT
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// The camera's offset from the instances' anchor, for the rays (`hw_light`'s origin).
    pub fn offset(&self, camera: glam::DVec3) -> [f32; 3] {
        self.anchor
            .map_or(glam::Vec3::ZERO, |a| (camera - a).as_vec3())
            .to_array()
    }

    /// Section `key` was meshed again or went: its structure goes, made again if still near.
    pub fn forget(&mut self, key: SectionKey) {
        if let Some(built) = self.built.get_mut(&key) {
            // Its vertices are freed: the old structure's quads are taken as opaque, without
            // their texels (`PAGE_GONE`), until the new mesh's structure replaces it.
            built.page = PAGE_GONE;
            self.stale.insert(key, 0);
            self.changed = true;
        }
    }

    /// Whether `key` is within `RADIUS` chunks of chunk `center`.
    pub fn near(center: (i32, i32), key: SectionKey) -> bool {
        (key.0 - center.0).abs() <= RADIUS && (key.2 - center.1).abs() <= RADIUS
    }

    /// Whether section `key` already has its structure (a stale one, standing in, doesn't count).
    pub fn has(&self, key: SectionKey) -> bool {
        self.built.contains_key(&key) && !self.stale.contains_key(&key)
    }

    /// How many more sections this frame may build.
    pub fn budget(&self) -> usize {
        BUILDS_PER_FRAME
    }

    /// Builds `todo`'s structures from `arena`'s pages and the quad `indices`, drops those far from
    /// chunk `center`, and makes the top-level structure again around `camera`.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        todo: &[ToBuild],
        pages: impl Fn(u32) -> wgpu::Buffer,
        indices: &wgpu::Buffer,
        center: (i32, i32),
        camera: glam::DVec3,
    ) {
        let before = self.built.len();
        // A stale structure no new mesh replaced within a few frames (the section went empty,
        // or out of range) goes.
        let mut gone = Vec::new();
        for (key, frames) in &mut self.stale {
            *frames += 1;
            if *frames > STALE_FRAMES {
                gone.push(*key);
            }
        }
        for key in gone {
            self.stale.remove(&key);
            self.built.remove(&key);
        }
        for t in todo {
            self.stale.remove(&t.key);
        }
        self.built.retain(|key, _| Self::near(center, *key));
        self.changed |= self.built.len() != before || !todo.is_empty();
        // A section's geometries: its solid quads, then its translucent ones (stained glass, water,
        // ice), which tint the light instead of stopping it; each once a ray (no duplicate any-hit).
        let geometry = |quads: u32, translucent: bool| wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count: quads * 4,
            index_format: Some(wgpu::IndexFormat::Uint32),
            index_count: Some(quads * 6),
            flags: if translucent {
                wgpu::AccelerationStructureGeometryFlags::NO_DUPLICATE_ANY_HIT_INVOCATION
            } else {
                wgpu::AccelerationStructureGeometryFlags::empty()
            },
        };
        let sizes: Vec<Vec<_>> = todo
            .iter()
            .map(|t| {
                [(t.quads, false), (t.translucent, true)]
                    .into_iter()
                    .filter(|(quads, _)| *quads > 0)
                    .map(|(quads, translucent)| geometry(quads, translucent))
                    .collect()
            })
            .collect();
        let buffers: Vec<_> = todo.iter().map(|t| pages(t.page)).collect();
        let mut entries = Vec::with_capacity(todo.len());
        for (t, size) in todo.iter().zip(&sizes) {
            let blas = device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("RT lighting's section"),
                    flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: size.clone(),
                },
            );
            self.built.insert(
                t.key,
                Built {
                    blas,
                    page: t.page,
                    start: t.start,
                    solid: t.quads,
                    translucent_geometry: match (t.quads, t.translucent) {
                        (_, 0) => 2,
                        (0, _) => 0,
                        _ => 1,
                    },
                },
            );
        }
        for ((t, size), buffer) in todo.iter().zip(&sizes).zip(&buffers) {
            let firsts = [
                (t.quads, t.start),
                (t.translucent, t.start + u64::from(t.quads) * 4),
            ];
            let firsts = firsts.into_iter().filter(|(quads, _)| *quads > 0);
            entries.push(wgpu::BlasBuildEntry {
                blas: &self.built[&t.key].blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(
                    size.iter()
                        .zip(firsts)
                        .map(|(size, (_, first))| wgpu::BlasTriangleGeometry {
                            size,
                            vertex_buffer: buffer,
                            first_vertex: first as u32,
                            vertex_stride: std::mem::size_of::<crate::mesher::TerrainVertex>()
                                as u64,
                            index_buffer: Some(indices),
                            first_index: Some(0),
                            transform_buffer: None,
                            transform_buffer_offset: None,
                        })
                        .collect(),
                ),
            });
        }
        let moved = self
            .anchor
            .is_none_or(|a| (camera - a).abs().max_element() > ANCHOR_REACH);
        // The top-level structure is made in `finish`, with the moving geometry.
        encoder.build_acceleration_structures(entries.iter(), std::iter::empty());
        if !(moved || self.changed) {
            return;
        }
        self.terrain_changed = true;
        if moved {
            self.anchor = Some(camera.floor());
        }
        self.changed = false;
        let anchor = self.anchor.unwrap_or(camera);
        let mut n = 0usize;
        let mut table = Vec::with_capacity(self.built.len().min(MAX_INSTANCES as usize));
        for (&(x, sy, z), built) in &self.built {
            if n as u32 >= MAX_INSTANCES - DYNAMIC_INSTANCES {
                break;
            }
            let origin = glam::DVec3::new(
                f64::from(x * 16),
                f64::from(crate::chunk::MIN_Y + sy * 16),
                f64::from(z * 16),
            ) - anchor;
            let o = origin.as_vec3();
            let first = built.start as u32;
            table.push([
                built.page,
                first,
                first + built.solid * 4,
                built.translucent_geometry,
            ]);
            self.tlas[n] = Some(wgpu::TlasInstance::new(
                &built.blas,
                [1.0, 0.0, 0.0, o.x, 0.0, 1.0, 0.0, o.y, 0.0, 0.0, 1.0, o.z],
                n as u32,
                0xff,
            ));
            n += 1;
        }
        for i in n..self.used {
            self.tlas[i] = None;
        }
        self.used = n;
        self.terrain = n;
        if !table.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&table));
        }
    }

    /// Puts the frame's moving geometry in the rays (after `update`, once the entities are drawn)
    /// and makes the top-level structure if the terrain, the moving geometry or the clouds changed.
    /// `indices` are the quad indices (`Renderer::indices`), long enough for the entities' and the
    /// view model's quads.
    pub fn finish(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        dynamic: &Dynamic,
        indices: &wgpu::Buffer,
        camera: glam::DVec3,
    ) {
        let Some(anchor) = self.anchor else {
            return;
        };
        // `HORIZON_HW_DYNAMIC=0` leaves the moving geometry out (the terrain alone, to compare).
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let enabled =
            *ENABLED.get_or_init(|| std::env::var("HORIZON_HW_DYNAMIC").map_or(true, |v| v != "0"));
        let none = Dynamic {
            entities: &[],
            hand: &[],
            hand_to_world: dynamic.hand_to_world,
            clouds: Vec::new(),
        };
        let dynamic = if enabled { dynamic } else { &none };
        // The vertices: the entities', then the view model's moved to where the world sees them.
        let entities =
            &dynamic.entities[..dynamic.entities.len().min(MAX_DYNAMIC_VERTICES) / 4 * 4];
        let hand_room = MAX_DYNAMIC_VERTICES - entities.len();
        let hand = &dynamic.hand[..dynamic.hand.len().min(hand_room) / 4 * 4];
        self.hand_scratch.clear();
        self.hand_scratch.extend(hand.iter().map(|v| {
            let at = dynamic
                .hand_to_world
                .transform_point3(glam::Vec3::from(v.pos));
            let n = glam::Vec3::new(
                f32::from(v.normal[0]),
                f32::from(v.normal[1]),
                f32::from(v.normal[2]),
            );
            let n = dynamic.hand_to_world.transform_vector3(n);
            // The face normal stays in its bytes (the fourth, the lighting mode, is left alone).
            let n = (n / n.abs().max_element().max(1.0e-6) * 127.0).round();
            Vertex {
                pos: at.to_array(),
                normal: [n.x as i8, n.y as i8, n.z as i8, v.normal[3]],
                ..*v
            }
        }));
        let total = entities.len() + self.hand_scratch.len();
        if total > self.capacity {
            self.capacity = total.next_power_of_two();
            self.dynamics = Self::dynamics_buffer(device, self.capacity);
        }
        if !entities.is_empty() {
            queue.write_buffer(&self.dynamics, 0, bytemuck::cast_slice(entities));
        }
        if !self.hand_scratch.is_empty() {
            queue.write_buffer(
                &self.dynamics,
                std::mem::size_of_val(entities) as u64,
                bytemuck::cast_slice(&self.hand_scratch),
            );
        }
        // The cloud tiles: a structure each, made when the tile comes (a tile with another level of
        // detail has another key) and dropped with it; the wind only moves its instance.
        let before = self.clouds.len();
        self.clouds
            .retain(|key, _| dynamic.clouds.iter().any(|t| t.key == *key));
        let mut changed = self.clouds.len() != before;
        let mut fresh = Vec::new();
        for tile in &dynamic.clouds {
            if self.clouds.contains_key(&tile.key) || fresh.len() >= CLOUDS_PER_FRAME {
                continue;
            }
            let size = wgpu::BlasTriangleGeometrySizeDescriptor {
                vertex_format: wgpu::VertexFormat::Float32x3,
                vertex_count: tile.vertex_count,
                index_format: Some(wgpu::IndexFormat::Uint32),
                index_count: Some(tile.index_count),
                // Opaque for the rays: no alpha test, a hit is a hit.
                flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
            };
            let blas = device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("RT lighting's cloud tile"),
                    flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: vec![size.clone()],
                },
            );
            self.clouds.insert(tile.key, blas);
            fresh.push((tile, size));
            changed = true;
        }
        let origin = dynamic.clouds.first().map(|t| t.origin);
        if let (Some(now), Some(placed)) = (origin, self.cloud_origin)
            && (now - placed).abs().max_element() > CLOUD_DRIFT
        {
            changed = true;
        }
        let entity_quads = (entities.len() / 4) as u32;
        let hand_quads = (self.hand_scratch.len() / 4) as u32;
        if !(changed || entity_quads + hand_quads > 0 || self.moving > 0 || self.terrain_changed) {
            return;
        }
        // The moving structures, built again from this frame's vertices.
        if entity_quads > 0 {
            ensure_moving(
                device,
                &mut self.entities,
                entity_quads,
                "RT lighting's entities",
            );
        }
        if hand_quads > 0 {
            ensure_moving(
                device,
                &mut self.hand,
                hand_quads,
                "RT lighting's view model",
            );
        }
        let flags = wgpu::AccelerationStructureGeometryFlags::empty();
        let entity_size = quads_geometry(entity_quads, flags);
        let hand_size = quads_geometry(hand_quads, flags);
        let mut entries: Vec<wgpu::BlasBuildEntry<'_>> = Vec::new();
        if let (true, Some((blas, _))) = (entity_quads > 0, &self.entities) {
            entries.push(moving_entry(blas, &entity_size, &self.dynamics, indices, 0));
        }
        if let (true, Some((blas, _))) = (hand_quads > 0, &self.hand) {
            entries.push(moving_entry(
                blas,
                &hand_size,
                &self.dynamics,
                indices,
                entities.len() as u32,
            ));
        }
        for (tile, size) in &fresh {
            entries.push(wgpu::BlasBuildEntry {
                blas: &self.clouds[&tile.key],
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![
                    wgpu::BlasTriangleGeometry {
                        size,
                        vertex_buffer: tile.vertices,
                        first_vertex: 0,
                        vertex_stride: std::mem::size_of::<super::rt_cloud_mesh::Vertex>() as u64,
                        index_buffer: Some(tile.indices),
                        first_index: Some(0),
                        transform_buffer: None,
                        transform_buffer_offset: None,
                    },
                ]),
            });
        }
        // Their instances follow the terrain's: the entities and the view model where the camera's
        // offset from the anchor puts them (their vertices are relative to the camera), each cloud
        // tile where the wind has taken it. Each row says what the instance is (the shader reads
        // it as a kind where terrain rows hold an arena page), its first vertex, and 2: no
        // translucent geometry.
        let offset = (camera - anchor).as_vec3();
        let mut slots: Vec<(&wgpu::Blas, glam::Vec3, [u32; 4], u8)> = Vec::new();
        if let (true, Some((blas, _))) = (entity_quads > 0, &self.entities) {
            slots.push((blas, offset, [KIND_ENTITY, 0, 0, 2], 0xff));
        }
        if let (true, Some((blas, _))) = (hand_quads > 0, &self.hand) {
            let first = entities.len() as u32;
            slots.push((blas, offset, [KIND_ENTITY, first, 0, 2], 0xff));
        }
        for tile in &dynamic.clouds {
            if let Some(blas) = self.clouds.get(&tile.key) {
                let at = (tile.origin - anchor).as_vec3();
                slots.push((blas, at, [KIND_CLOUD, 0, 0, 2], CLOUD_MASK));
            }
        }
        let mut rows = Vec::with_capacity(slots.len());
        let mut n = self.terrain;
        for (blas, at, row, mask) in slots {
            if n as u32 >= MAX_INSTANCES {
                break;
            }
            let transform = [
                1.0, 0.0, 0.0, at.x, 0.0, 1.0, 0.0, at.y, 0.0, 0.0, 1.0, at.z,
            ];
            self.tlas[n] = Some(wgpu::TlasInstance::new(blas, transform, n as u32, mask));
            rows.push(row);
            n += 1;
        }
        for i in n..self.used {
            self.tlas[i] = None;
        }
        self.moving = n - self.terrain;
        self.used = n;
        self.cloud_origin = origin;
        self.terrain_changed = false;
        if !rows.is_empty() {
            queue.write_buffer(
                &self.instances,
                (self.terrain * 16) as u64,
                bytemuck::cast_slice(&rows),
            );
        }
        encoder.build_acceleration_structures(entries.iter(), [&self.tlas]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_copies_of_the_water_waves_are_the_same() {
        let waves = |text: &'static str| {
            let start = text.find("fn water_hash(").expect("the sea state");
            let end = text.find("fn water_waves(").expect("the waves");
            // Whatever line endings the checkout gave each file.
            text[start..end].replace('\r', "")
        };
        let shown = waves(include_str!("../terrain.wgsl"));
        assert_eq!(shown, waves(include_str!("shadow_rays_hw.wgsl")));
        assert_eq!(shown, waves(include_str!("froxels_hw.wgsl")));
    }

    #[test]
    fn the_shaders_read_as_many_arena_pages_as_are_bound() {
        for text in [
            include_str!("shadow_rays_hw.wgsl"),
            include_str!("froxels_hw.wgsl"),
        ] {
            assert!(text.contains(&format!("const ARENA_PAGES: u32 = {ARENA_PAGES}u;")));
            assert!(text.contains("@group(0) @binding(16) var<storage, read> arena: array<u32>;"));
            for (page, binding) in ARENA_BINDINGS.iter().enumerate() {
                let page = page + 1;
                assert!(text.contains(&format!(
                    "@group(0) @binding({binding}) var<storage, read> arena{page}: array<u32>;"
                )));
                assert!(
                    text.contains(&format!("case {page}u: {{ return arena{page}[i]; }}"))
                        || page == ARENA_PAGES - 1
                );
            }
        }
    }
}
