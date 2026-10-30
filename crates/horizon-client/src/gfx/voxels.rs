//! Ray-traced shadows' voxel grid (docs/raytracing.md, approach A): the blocks around the camera
//! as bit masks, 512 bits an 8x8x8 brick for the solid blocks and as many for the cutout ones
//! (`crate::occupancy`), that terrain.wgsl walks toward the sun or the moon. It covers `RADIUS` chunks each side of the camera's chunk and the
//! world's whole height, addressed toroidally: when the camera crosses a chunk border only the
//! columns that came into range are written, and a changed block costs its section's 4 KiB.
//!
//! Two more textures, one texel an 8x8x8 brick and one a 32x32x32 one, say which hold anything:
//! a ray crosses the air above the ground 32 blocks a step, then 8, and only walks blocks where
//! there are some.
//!
//! Every resident section's occupancy is kept (two bits a block, uniform sections nothing), so a
//! column coming back into range is written without meshing it again.

use super::SectionKey;
use crate::chunk::SECTIONS;
use crate::occupancy::{Albedo, Occupancy};
use glam::DVec3;
use std::collections::{HashMap, HashSet};

/// Chunks covered on each side of the camera's chunk.
pub const RADIUS: i32 = 8;
/// The grid's width and depth in blocks.
pub const WIDTH: u32 = (2 * RADIUS * 16) as u32;
const HEIGHT: u32 = SECTIONS as u32 * 16;

struct Grid {
    /// The solid and the cutout masks: four `Rgba32Uint` texels a brick along x.
    masks: [wgpu::Texture; 2],
    mask_views: [wgpu::TextureView; 2],
    bricks: wgpu::Texture,
    bricks_view: wgpu::TextureView,
    supers: wgpu::Texture,
    supers_view: wgpu::TextureView,
    /// RT lighting's bounced light and shapes: each block's map colour and shape id (`Albedo`), a
    /// texel a block; and the shapes by id (`occupancy::shapes`), as many as written.
    albedo: wgpu::Texture,
    albedo_view: wgpu::TextureView,
    shapes: wgpu::Buffer,
    shapes_written: usize,
    /// The camera's chunk the grid is centred on, once written.
    center: Option<(i32, i32)>,
    /// Sections to write, in range or not yet.
    dirty: HashSet<SectionKey>,
    /// Above the highest block written in range (a world Y): a ray going up past it is lit.
    top: i32,
    /// RT lighting's light sources in range (`LIGHT_*`), and for each 8-block cell of the grid
    /// the eight that light it most (indices + 1, 0 none, two to a word).
    lights: wgpu::Buffer,
    cells: wgpu::Buffer,
    /// RT lighting's NEE cache: for each cell's eight lights, the shadow rays drawn toward it and
    /// those that reached it (two words), learnt by the rays (shadow_rays.wgsl `restir_light`) and
    /// emptied when the cells' lists are made again.
    visibility: wgpu::Buffer,
    /// The light sources changed in range: the buffers are made again.
    lights_dirty: bool,
}

/// At most this many light sources in range: the nearest to the camera are kept.
pub const MAX_LIGHTS: usize = 8192;
/// Cells a side of `Grid::cells`, x, then z, then y (8 blocks each).
const CELLS: [u32; 3] = [WIDTH / 8, WIDTH / 8, HEIGHT / 8];
/// How far a light of level 15 reaches, in blocks (in proportion to the level); its light is gone
/// there (shadow_rays.wgsl `light_path`'s falloff window). Half again vanilla's 16: Remix's lights
/// fall off by the inverse square without a cut, so a lamp still lights a little past the 15
/// blocks of its level.
const LIGHT_REACH: f32 = 24.0;

/// One light source for the GPU: its centre relative to the grid's first block (y from the
/// world's bottom) and its reach; its colour (gamma-space, 0..1) and strength.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuLight {
    at: [f32; 4],
    colour: [f32; 4],
}

#[derive(Default)]
pub struct Voxels {
    sections: HashMap<SectionKey, Occupancy>,
    /// Every resident section's blocks' colours (`SectionMesh::albedo`), for RT lighting.
    albedo: HashMap<SectionKey, Albedo>,
    /// Every resident section's light sources (`SectionMesh::lights`), for RT lighting.
    lights: HashMap<SectionKey, Vec<[u8; 10]>>,
    grid: Option<Grid>,
}

fn in_range(center: (i32, i32), x: i32, z: i32) -> bool {
    (center.0 - RADIUS..center.0 + RADIUS).contains(&x)
        && (center.1 - RADIUS..center.1 + RADIUS).contains(&z)
}

impl Voxels {
    /// The section `key` now has these light sources (as meshed).
    pub fn set_lights(&mut self, key: SectionKey, lights: &[[u8; 10]]) {
        let changed = if lights.is_empty() {
            self.lights.remove(&key).is_some()
        } else {
            self.lights.insert(key, lights.to_vec()).as_deref() != Some(lights)
        };
        if changed && let Some(grid) = &mut self.grid {
            grid.lights_dirty = true;
        }
    }

    /// The section `key`'s blocks now have these colours (as meshed).
    pub fn set_albedo(&mut self, key: SectionKey, albedo: &Albedo) {
        let changed = if albedo.is_empty() {
            self.albedo.remove(&key).is_some()
        } else {
            self.albedo.insert(key, albedo.clone()).as_ref() != Some(albedo)
        };
        if changed && let Some(grid) = &mut self.grid {
            grid.dirty.insert(key);
        }
    }

    /// The section `key` now holds `occupancy` (as meshed).
    pub fn set(&mut self, key: SectionKey, occupancy: &Occupancy) {
        let changed = match occupancy {
            Occupancy::Empty => self.sections.remove(&key).is_some(),
            o => self.sections.insert(key, o.clone()).as_ref() != Some(o),
        };
        if changed && let Some(grid) = &mut self.grid {
            grid.dirty.insert(key);
        }
    }

    pub fn remove_column(&mut self, x: i32, z: i32) {
        for sy in 0..SECTIONS as i32 {
            let key = (x, sy, z);
            if self.lights.remove(&key).is_some()
                && let Some(grid) = &mut self.grid
            {
                grid.lights_dirty = true;
            }
            self.albedo.remove(&key);
            if self.sections.remove(&key).is_some()
                && let Some(grid) = &mut self.grid
            {
                grid.dirty.insert(key);
            }
        }
    }

    pub fn clear(&mut self) {
        self.sections.clear();
        self.lights.clear();
        self.albedo.clear();
        if let Some(grid) = &mut self.grid {
            grid.center = None;
            grid.dirty.clear();
        }
    }

    /// Makes or drops the grid's texture; true when that changed.
    pub fn enable(&mut self, device: &wgpu::Device, on: bool) -> bool {
        if on == self.grid.is_some() {
            return false;
        }
        self.grid = on.then(|| {
            let texture = |label, width, scale, format| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height: HEIGHT / scale,
                        depth_or_array_layers: WIDTH / scale,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            };
            let masks = [
                "Ray-traced shadows' solid blocks",
                "Ray-traced shadows' cutout blocks",
            ]
            .map(|label| texture(label, WIDTH / 2, 8, wgpu::TextureFormat::Rgba32Uint));
            let bricks = texture(
                "Ray-traced shadows' bricks",
                WIDTH / 8,
                8,
                wgpu::TextureFormat::R8Uint,
            );
            let supers = texture(
                "Ray-traced shadows' 32-block bricks",
                WIDTH / 32,
                32,
                wgpu::TextureFormat::R8Uint,
            );
            let albedo = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("RT lighting's block colours and shapes"),
                size: wgpu::Extent3d {
                    width: WIDTH,
                    height: HEIGHT,
                    depth_or_array_layers: WIDTH,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format: wgpu::TextureFormat::R16Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            Grid {
                shapes: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("RT lighting's block shapes"),
                    size: (crate::occupancy::SHAPES * 80) as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                shapes_written: 0,
                albedo_view: albedo.create_view(&Default::default()),
                albedo,
                mask_views: [0, 1].map(|i| masks[i].create_view(&Default::default())),
                masks,
                bricks_view: bricks.create_view(&Default::default()),
                bricks,
                supers_view: supers.create_view(&Default::default()),
                supers,
                center: None,
                dirty: HashSet::new(),
                top: crate::chunk::MIN_Y,
                lights: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("RT lighting's light sources"),
                    size: (MAX_LIGHTS * std::mem::size_of::<GpuLight>()) as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                cells: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("RT lighting's light cells"),
                    size: u64::from(CELLS[0] * CELLS[1] * CELLS[2]) * 16,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                visibility: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("RT lighting's NEE cache"),
                    size: u64::from(CELLS[0] * CELLS[1] * CELLS[2]) * 8 * 8,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                lights_dirty: true,
            }
        });
        true
    }

    /// RT lighting's light sources and cells, and the blocks' shapes, while the grid runs.
    pub fn light_buffers(&self) -> Option<[&wgpu::Buffer; 3]> {
        self.grid.as_ref().map(|g| [&g.lights, &g.cells, &g.shapes])
    }

    /// RT lighting's NEE cache (`Grid::visibility`), while the grid runs.
    pub fn light_visibility(&self) -> Option<&wgpu::Buffer> {
        self.grid.as_ref().map(|g| &g.visibility)
    }

    /// The grid's solid and cutout masks, its bricks, its 32-block bricks and its blocks' colours,
    /// while it runs.
    pub fn views(&self) -> Option<[&wgpu::TextureView; 5]> {
        self.grid.as_ref().map(|g| {
            [
                &g.mask_views[0],
                &g.mask_views[1],
                &g.bricks_view,
                &g.supers_view,
                &g.albedo_view,
            ]
        })
    }

    /// Centres the grid on the camera's chunk and writes what changed. Returns the grid's
    /// first block (x, z) and the Y above its highest block, for the shader, while it runs.
    pub fn update(&mut self, queue: &wgpu::Queue, camera: DVec3) -> Option<[i32; 3]> {
        let grid = self.grid.as_mut()?;
        let center = (
            (camera.x.floor() as i32) >> 4,
            (camera.z.floor() as i32) >> 4,
        );
        if grid.center != Some(center) {
            for x in center.0 - RADIUS..center.0 + RADIUS {
                for z in center.1 - RADIUS..center.1 + RADIUS {
                    if grid.center.is_none_or(|old| !in_range(old, x, z)) {
                        grid.dirty.extend((0..SECTIONS as i32).map(|sy| (x, sy, z)));
                    }
                }
            }
            grid.center = Some(center);
            grid.lights_dirty = true;
            // The highest section in range: kept up to date as sections come, never lowered
            // until the grid moves again.
            grid.top = crate::chunk::MIN_Y;
            for (x, sy, z) in self.sections.keys() {
                if in_range(center, *x, *z) {
                    grid.top = grid.top.max(crate::chunk::MIN_Y + sy * 16 + 16);
                }
            }
        }
        let width = 2 * RADIUS;
        let mut supers = HashSet::new();
        for key @ (x, sy, z) in std::mem::take(&mut grid.dirty) {
            if !in_range(center, x, z) {
                continue;
            }
            supers.insert((x >> 1, sy >> 1, z >> 1));
            let occupancy = self.sections.get(&key).unwrap_or(&Occupancy::Empty);
            if *occupancy != Occupancy::Empty {
                grid.top = grid.top.max(crate::chunk::MIN_Y + sy * 16 + 16);
            }
            let (sx, sz) = (x.rem_euclid(width) as u32, z.rem_euclid(width) as u32);
            for (texture, mask) in grid.masks.iter().zip(occupancy.brick_masks()) {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: sx * 8,
                            y: (sy * 2) as u32,
                            z: sz * 2,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytemuck::cast_slice(&mask),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(8 * 16),
                        rows_per_image: Some(2),
                    },
                    wgpu::Extent3d {
                        width: 8,
                        height: 2,
                        depth_or_array_layers: 2,
                    },
                );
            }
            let colours = self
                .albedo
                .get(&key)
                .map_or_else(|| vec![0; 8192], Albedo::decode);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &grid.albedo,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: sx * 16,
                        y: (sy * 16) as u32,
                        z: sz * 16,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &colours,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(32),
                    rows_per_image: Some(16),
                },
                wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 16,
                },
            );
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &grid.bricks,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: sx * 2,
                        y: (sy * 2) as u32,
                        z: sz * 2,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &occupancy.bricks(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(2),
                    rows_per_image: Some(2),
                },
                wgpu::Extent3d {
                    width: 2,
                    height: 2,
                    depth_or_array_layers: 2,
                },
            );
        }
        // A 32-block brick holds 2x2x2 sections: anything in any of them.
        for (bx, by, bz) in supers {
            let any = (0..8).any(|i| {
                let key = (bx * 2 + (i & 1), by * 2 + ((i >> 1) & 1), bz * 2 + (i >> 2));
                self.sections.contains_key(&key)
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &grid.supers,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: bx.rem_euclid(RADIUS) as u32,
                        y: by as u32,
                        z: bz.rem_euclid(RADIUS) as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &[u8::from(any)],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(1),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        // The shapes met since: the table only grows.
        let shapes = crate::occupancy::shapes();
        if shapes.len() > grid.shapes_written {
            queue.write_buffer(&grid.shapes, 0, bytemuck::cast_slice(&shapes));
            grid.shapes_written = shapes.len();
        }
        let origin = [(center.0 - RADIUS) * 16, (center.1 - RADIUS) * 16];
        if std::mem::take(&mut grid.lights_dirty) {
            let (lights, cells) = light_cells(&self.lights, center, origin, camera);
            log::debug!("Horizon: {} light sources in the rays' grid", lights.len());
            queue.write_buffer(&grid.lights, 0, bytemuck::cast_slice(&lights));
            queue.write_buffer(&grid.cells, 0, bytemuck::cast_slice(&cells));
            let size = u64::from(CELLS[0] * CELLS[1] * CELLS[2]) * 8 * 8;
            queue.write_buffer(&grid.visibility, 0, &vec![0; size as usize]);
        }
        Some([origin[0], origin[1], grid.top])
    }
}

/// RT lighting's light sources in range, nearest the camera first (at most `MAX_LIGHTS`), and for
/// each cell of the grid the eight (indices + 1, two to a word) whose light reaching its centre is
/// the strongest.
fn light_cells(
    sections: &HashMap<SectionKey, Vec<[u8; 10]>>,
    center: (i32, i32),
    origin: [i32; 2],
    camera: DVec3,
) -> (Vec<GpuLight>, Vec<[u32; 4]>) {
    let mut found: Vec<(f64, GpuLight)> = Vec::new();
    for (&(x, sy, z), lights) in sections {
        if !in_range(center, x, z) {
            continue;
        }
        for &[lx, ly, lz, level, r, g, b, ax, ay, az] in lights {
            // Where in the block it burns (the mesher's `light_at`), or its middle.
            let within = if ax == 255 {
                [0.5; 3]
            } else {
                [ax, ay, az].map(|v| f32::from(v) / 32.0)
            };
            let world = [
                x * 16 + i32::from(lx),
                crate::chunk::MIN_Y + sy * 16 + i32::from(ly),
                z * 16 + i32::from(lz),
            ];
            let d = DVec3::new(world[0] as f64, world[1] as f64, world[2] as f64) - camera;
            let strength = (f32::from(level) / 15.0).powi(2);
            found.push((
                d.length_squared(),
                GpuLight {
                    at: [
                        (world[0] - origin[0]) as f32 + within[0],
                        (world[1] - crate::chunk::MIN_Y) as f32 + within[1],
                        (world[2] - origin[1]) as f32 + within[2],
                        LIGHT_REACH * (f32::from(level) + 1.0) / 16.0,
                    ],
                    colour: [
                        f32::from(r) / 255.0,
                        f32::from(g) / 255.0,
                        f32::from(b) / 255.0,
                        strength,
                    ],
                },
            ));
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.truncate(MAX_LIGHTS);
    let lights: Vec<GpuLight> = found.into_iter().map(|(_, l)| l).collect();
    let [cx, cz, cy] = CELLS.map(|c| c as i32);
    let mut best = vec![[(0f32, 0u32); 8]; (cx * cz * cy) as usize];
    for (i, light) in lights.iter().enumerate() {
        let [px, py, pz, reach] = light.at;
        let cell_of = |v: f32| (v / 8.0).floor() as i32;
        for y in cell_of(py - reach).max(0)..=cell_of(py + reach).min(cy - 1) {
            for z in cell_of(pz - reach).max(0)..=cell_of(pz + reach).min(cz - 1) {
                for x in cell_of(px - reach).max(0)..=cell_of(px + reach).min(cx - 1) {
                    let c = [
                        x as f32 * 8.0 + 4.0,
                        y as f32 * 8.0 + 4.0,
                        z as f32 * 8.0 + 4.0,
                    ];
                    let d2 = (c[0] - px).powi(2) + (c[1] - py).powi(2) + (c[2] - pz).powi(2);
                    // The cell's nearest corner is up to 7 blocks nearer than its centre.
                    if d2.sqrt() > reach + 7.0 {
                        continue;
                    }
                    let score = light.colour[3] / d2.max(1.0);
                    let slots = &mut best[((y * cz + z) * cx + x) as usize];
                    if let Some(weakest) = (0..8).min_by(|&a, &b| slots[a].0.total_cmp(&slots[b].0))
                        && score > slots[weakest].0
                    {
                        slots[weakest] = (score, i as u32 + 1);
                    }
                }
            }
        }
    }
    // Two indices (16 bits each, at most `MAX_LIGHTS`) a word.
    let cells = best
        .into_iter()
        .map(|s| std::array::from_fn(|w| s[w * 2].1 | (s[w * 2 + 1].1 << 16)))
        .collect();
    (lights, cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_range_is_the_camera_chunk_and_radius_each_side() {
        assert!(in_range((0, 0), -RADIUS, RADIUS - 1));
        assert!(!in_range((0, 0), RADIUS, 0));
        assert!(!in_range((0, 0), 0, -RADIUS - 1));
        // A chunk keeps its toroidal slot as the grid moves.
        let slot = |x: i32| x.rem_euclid(2 * RADIUS);
        assert_eq!(slot(-1), slot(2 * RADIUS - 1));
    }

    #[test]
    fn a_light_reaches_the_cells_around_it_only() {
        // A torch (level 14) at world (33, 201, 26): section (2, 16, 1), grid around chunk (0, 0).
        let mut sections = HashMap::new();
        sections.insert((2, 16, 1), vec![[1, 9, 10, 14, 255, 170, 90, 255, 255, 255]]);
        let origin = [-RADIUS * 16, -RADIUS * 16];
        let (lights, cells) = light_cells(&sections, (0, 0), origin, DVec3::ZERO);
        assert_eq!(lights.len(), 1);
        let at = lights[0].at;
        assert_eq!(at[..3], [33.5 + 128.0, 265.5, 26.5 + 128.0]);
        let [cx, cz, _] = CELLS.map(|c| c as usize);
        let cell = |x: f32, y: f32, z: f32| {
            let [x, y, z] = [x, y, z].map(|v| (v / 8.0) as usize);
            cells[(y * cz + z) * cx + x]
        };
        assert_eq!(cell(at[0], at[1], at[2])[0], 1);
        assert_eq!(
            cell(at[0] + 9.0, at[1], at[2])
                .iter()
                .filter(|&&i| i == 1)
                .count(),
            1
        );
        assert_eq!(cell(at[0] + 40.0, at[1], at[2]), [0; 4]);
    }
}
