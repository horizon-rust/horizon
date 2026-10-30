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

use super::SectionKey;
use std::collections::HashMap;

/// Sections within this many chunks of the camera's have a structure: the voxel grid's reach,
/// where the rays run.
pub const RADIUS: i32 = super::voxels::RADIUS;
/// Instances the top-level structure holds at most (a 17x17-chunk square is about 7000 sections).
const MAX_INSTANCES: u32 = 16384;
/// Bottom-level structures built a frame at most, so arriving in a new place spreads them out.
const BUILDS_PER_FRAME: usize = 128;

/// A section's structure and where its quads were when it was built: its page, first vertex,
/// solid quads, and which of its geometries is the translucent one (2: none).
struct Built {
    blas: wgpu::Blas,
    page: u32,
    start: u64,
    solid: u32,
    translucent_geometry: u32,
}

pub struct HwRays {
    built: HashMap<SectionKey, Built>,
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
        }
    }

    /// The camera's offset from the instances' anchor, for the rays (`hw_light`'s origin).
    pub fn offset(&self, camera: glam::DVec3) -> [f32; 3] {
        self.anchor
            .map_or(glam::Vec3::ZERO, |a| (camera - a).as_vec3())
            .to_array()
    }

    /// Section `key` was meshed again or went: its structure goes, made again if still near.
    pub fn forget(&mut self, key: SectionKey) {
        if self.built.remove(&key).is_some() {
            self.changed = true;
        }
    }

    /// Whether `key` is within `RADIUS` chunks of chunk `center`.
    pub fn near(center: (i32, i32), key: SectionKey) -> bool {
        (key.0 - center.0).abs() <= RADIUS && (key.2 - center.1).abs() <= RADIUS
    }

    /// Whether section `key` already has its structure.
    pub fn has(&self, key: SectionKey) -> bool {
        self.built.contains_key(&key)
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
        if !(moved || self.changed) {
            encoder.build_acceleration_structures(entries.iter(), std::iter::empty());
            return;
        }
        if moved {
            self.anchor = Some(camera.floor());
        }
        self.changed = false;
        let anchor = self.anchor.unwrap_or(camera);
        let mut n = 0usize;
        let mut table = Vec::with_capacity(self.built.len().min(MAX_INSTANCES as usize));
        for (&(x, sy, z), built) in &self.built {
            if n as u32 >= MAX_INSTANCES {
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
        if !table.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&table));
        }
        encoder.build_acceleration_structures(entries.iter(), [&self.tlas]);
    }
}
