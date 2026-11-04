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
/// Sections written a frame at most, nearest columns first. A chunk border crossed marks every
/// section of 16 columns (31 across a corner), 384 to 744: at four columns a frame they take a
/// few frames, the far edge of the grid a little late, where the rays' fade already blends it
/// (docs/raytracing.md). Before this, each crossing wrote them all in one frame, five
/// `write_texture` calls a section, each making its own staging buffer: the frame spikes in
/// flight.
const SECTIONS_PER_FRAME: usize = 4 * SECTIONS;
/// Frames between two rebuilds of the light cells at most, while lights keep changing (a block
/// placed or broken, a column come or gone within the grid): the rebuild is a 3 MB table and
/// as much zeroed cache, so a burst of changes pays it once.
const LIGHTS_EVERY: u32 = 8;

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
    /// Frames since the light cells were made (`LIGHTS_EVERY`).
    lights_age: u32,
}

/// A run of sections of one column, packed for the grid's textures: the toroidal slot of the
/// column, its first section and how many, and the texels of each texture in that order, the
/// sections one after another along y (`pack_run`).
struct Run {
    slot: (u32, u32),
    first: i32,
    count: u32,
    masks: [Vec<u8>; 2],
    albedo: Vec<u8>,
    bricks: Vec<u8>,
}

/// Copies section `s`'s texels (`src`, laid out as `layers` of `rows` rows of `row` bytes) into
/// a run of `n` sections (`dst`, the same layers, each `rows * n` rows): a section's rows go
/// after the sections below it in every layer, as a 3D copy of the whole run reads them.
fn interleave(
    dst: &mut [u8],
    src: &[u8],
    layers: usize,
    rows: usize,
    row: usize,
    n: usize,
    s: usize,
) {
    let layer = rows * row;
    debug_assert_eq!(src.len(), layers * layer);
    debug_assert_eq!(dst.len(), layers * layer * n);
    for z in 0..layers {
        let at = (z * rows * n + rows * s) * row;
        dst[at..at + layer].copy_from_slice(&src[z * layer..(z + 1) * layer]);
    }
}

/// The section mask's texel layout: two layers (bricks along z) of two rows (bricks along y) of
/// eight `Rgba32Uint` texels (16 bytes each, four a brick along x).
const MASK_LAYOUT: (usize, usize, usize) = (2, 2, 8 * 16);
/// The section's block colours: 16 layers of 16 rows of 16 `R16Uint` texels.
const ALBEDO_LAYOUT: (usize, usize, usize) = (16, 16, 16 * 2);
/// The section's bricks: two layers of two rows of two bytes.
const BRICKS_LAYOUT: (usize, usize, usize) = (2, 2, 2);

/// Packs sections `first..first + count` of column (x, z) for one 3D copy per texture.
fn pack_run(
    sections: &HashMap<SectionKey, Occupancy>,
    albedo: &HashMap<SectionKey, Albedo>,
    (x, z): (i32, i32),
    first: i32,
    count: u32,
) -> Run {
    let n = count as usize;
    let size = |(layers, rows, row): (usize, usize, usize)| layers * rows * row * n;
    let mut run = Run {
        slot: (
            x.rem_euclid(2 * RADIUS) as u32,
            z.rem_euclid(2 * RADIUS) as u32,
        ),
        first,
        count,
        masks: [vec![0; size(MASK_LAYOUT)], vec![0; size(MASK_LAYOUT)]],
        albedo: vec![0; size(ALBEDO_LAYOUT)],
        bricks: vec![0; size(BRICKS_LAYOUT)],
    };
    for s in 0..n {
        let key = (x, first + s as i32, z);
        let occupancy = sections.get(&key).unwrap_or(&Occupancy::Empty);
        if *occupancy == Occupancy::Empty && !albedo.contains_key(&key) {
            // Already zero.
            continue;
        }
        let (layers, rows, row) = MASK_LAYOUT;
        for (dst, mask) in run.masks.iter_mut().zip(occupancy.brick_masks()) {
            interleave(dst, bytemuck::cast_slice(&mask), layers, rows, row, n, s);
        }
        let (layers, rows, row) = BRICKS_LAYOUT;
        interleave(
            &mut run.bricks,
            &occupancy.bricks(),
            layers,
            rows,
            row,
            n,
            s,
        );
        if let Some(colours) = albedo.get(&key) {
            let (layers, rows, row) = ALBEDO_LAYOUT;
            interleave(&mut run.albedo, &colours.decode(), layers, rows, row, n, s);
        }
    }
    run
}

/// The dirty sections in range as runs of consecutive sections per column, the nearest columns
/// to `center` first, up to `budget` sections; the rest stay dirty for the next frame.
fn plan_runs(
    dirty: &mut HashSet<SectionKey>,
    center: (i32, i32),
    budget: usize,
) -> Vec<((i32, i32), i32, u32)> {
    let mut columns: HashMap<(i32, i32), Vec<i32>> = HashMap::new();
    for &(x, sy, z) in dirty.iter() {
        if in_range(center, x, z) {
            columns.entry((x, z)).or_default().push(sy);
        }
    }
    let mut order: Vec<_> = columns.keys().copied().collect();
    order.sort_by_key(|&(x, z)| {
        let (dx, dz) = (x - center.0, z - center.1);
        (dx * dx + dz * dz, x, z)
    });
    let mut runs = Vec::new();
    let mut written = 0;
    let mut kept = HashSet::new();
    for column in order {
        let mut sys = columns.remove(&column).expect("the column's sections");
        sys.sort_unstable();
        if written >= budget {
            kept.extend(sys.iter().map(|&sy| (column.0, sy, column.1)));
            continue;
        }
        written += sys.len();
        let mut run: Option<(i32, u32)> = None;
        for sy in sys {
            match &mut run {
                Some((first, count)) if *first + *count as i32 == sy => *count += 1,
                _ => {
                    if let Some((first, count)) = run.take() {
                        runs.push((column, first, count));
                    }
                    run = Some((sy, 1));
                }
            }
        }
        if let Some((first, count)) = run {
            runs.push((column, first, count));
        }
    }
    // What was out of range is dropped: the next recentre marks it again if it comes back.
    *dirty = kept;
    runs
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
    /// The flames near `camera` (within `range` blocks), for the heat haze above them.
    pub fn flames(&self, camera: DVec3, range: f64) -> Vec<super::haze::Flame> {
        super::haze::nearby(self.lights.iter(), camera, range)
    }

    /// The section `key` now has these light sources (as meshed).
    pub fn set_lights(&mut self, key: SectionKey, lights: &[[u8; 10]]) {
        let changed = if lights.is_empty() {
            self.lights.remove(&key).is_some()
        } else {
            self.lights.insert(key, lights.to_vec()).as_deref() != Some(lights)
        };
        // A light beyond the grid changes nothing in it: the recentre that brings it in marks
        // the cells dirty itself.
        if changed
            && self.in_grid(key)
            && let Some(grid) = &mut self.grid
        {
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

    /// The top of each of chunk column (x, z)'s 256 block columns (index `z * 16 + x`), from its
    /// sections' blocks: above its highest solid or cutout block (a world Y), `MIN_Y` where none.
    pub fn column_tops(&self, x: i32, z: i32) -> [i16; 256] {
        let mut tops = [crate::chunk::MIN_Y as i16; 256];
        let mut found = [false; 256];
        let mut left = 256;
        for sy in (0..SECTIONS as i32).rev() {
            let Some(occupancy) = self.sections.get(&(x, sy, z)) else {
                continue;
            };
            let base = crate::chunk::MIN_Y + sy * 16;
            for (i, top) in tops.iter_mut().enumerate() {
                if found[i] {
                    continue;
                }
                if let Some(y) = (0..16)
                    .rev()
                    .find(|&y| occupancy.get(i & 15, y, i >> 4) != crate::occupancy::EMPTY)
                {
                    *top = (base + y as i32 + 1) as i16;
                    found[i] = true;
                    left -= 1;
                }
            }
            if left == 0 {
                break;
            }
        }
        tops
    }

    pub fn remove_column(&mut self, x: i32, z: i32) {
        for sy in 0..SECTIONS as i32 {
            let key = (x, sy, z);
            if self.lights.remove(&key).is_some()
                && self.in_grid(key)
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
                lights_age: 0,
            }
        });
        true
    }

    /// Whether section `key` lies in the grid's range (anywhere, before it is centred).
    fn in_grid(&self, key: SectionKey) -> bool {
        self.grid
            .as_ref()
            .is_some_and(|g| g.center.is_none_or(|c| in_range(c, key.0, key.2)))
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
        let recentred = grid.center != Some(center);
        if recentred {
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
        // The dirty sections as runs per column, one copy a texture and run, a frame's budget
        // of them (`SECTIONS_PER_FRAME`).
        let runs = plan_runs(&mut grid.dirty, center, SECTIONS_PER_FRAME);
        let mut super_columns = HashSet::new();
        for (column, first, count) in runs {
            let (x, z) = column;
            for sy in first..first + count as i32 {
                if self.sections.contains_key(&(x, sy, z)) {
                    grid.top = grid.top.max(crate::chunk::MIN_Y + sy * 16 + 16);
                }
            }
            super_columns.insert((x >> 1, z >> 1));
            let run = pack_run(&self.sections, &self.albedo, column, first, count);
            let (sx, sz) = run.slot;
            // One 3D copy a texture: a section is `w` texels across, `r` rows and `l` layers of
            // `row` bytes a row; the run's sections follow one another along y.
            let copy = |texture: &wgpu::Texture,
                        data: &[u8],
                        (l, r, row): (usize, usize, usize),
                        w: u32| {
                let (l, r, row) = (l as u32, r as u32, row as u32);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: sx * w,
                            y: run.first as u32 * r,
                            z: sz * l,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(r * run.count),
                    },
                    wgpu::Extent3d {
                        width: w,
                        height: r * run.count,
                        depth_or_array_layers: l,
                    },
                );
            };
            for (texture, data) in grid.masks.iter().zip(&run.masks) {
                copy(texture, data, MASK_LAYOUT, 8);
            }
            copy(&grid.albedo, &run.albedo, ALBEDO_LAYOUT, 16);
            copy(&grid.bricks, &run.bricks, BRICKS_LAYOUT, 2);
        }
        // A 32-block brick holds 2x2x2 sections: anything in any of them; a column of bricks (two
        // columns of sections each way) in one copy.
        for (bx, bz) in super_columns {
            let column: Vec<u8> = (0..(SECTIONS / 2) as i32)
                .map(|by| {
                    u8::from((0..8).any(|i| {
                        let key = (bx * 2 + (i & 1), by * 2 + ((i >> 1) & 1), bz * 2 + (i >> 2));
                        self.sections.contains_key(&key)
                    }))
                })
                .collect();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &grid.supers,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: bx.rem_euclid(RADIUS) as u32,
                        y: 0,
                        z: bz.rem_euclid(RADIUS) as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &column,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(1),
                    rows_per_image: Some(column.len() as u32),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: column.len() as u32,
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
        grid.lights_age = grid.lights_age.saturating_add(1);
        if grid.lights_dirty && (recentred || grid.lights_age >= LIGHTS_EVERY) {
            grid.lights_dirty = false;
            grid.lights_age = 0;
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
            // The high nibble is the flame's kind (`haze::nearby`).
            let level = level & 15;
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
    fn a_run_packs_its_sections_one_above_the_other_in_every_layer() {
        use crate::occupancy::{CUTOUT, EMPTY, SOLID};
        // Two sections of one column: the lower one solid at one block, the upper one cutout at
        // another and coloured; a third, empty, above them.
        let mut sections = HashMap::new();
        let lower =
            Occupancy::from_fn(|x, y, z| if (x, y, z) == (1, 2, 3) { SOLID } else { EMPTY });
        let upper = Occupancy::from_fn(|x, y, z| {
            if (x, y, z) == (9, 9, 9) {
                CUTOUT
            } else {
                EMPTY
            }
        });
        sections.insert((0, 3, 0), lower.clone());
        sections.insert((0, 4, 0), upper.clone());
        let mut albedo = HashMap::new();
        let colours = Albedo::from_fn(|x, y, z| (x + y * 16 + z * 256) as u16);
        albedo.insert((0, 4, 0), colours.clone());
        let run = pack_run(&sections, &albedo, (0, 0), 3, 3);
        assert_eq!((run.slot, run.first, run.count), ((0, 0), 3, 3));
        // The masks: in each of the two layers, the run's rows are the sections' two rows each,
        // one section after the other, 128 bytes a row.
        let (lower_masks, upper_masks) = (lower.brick_masks(), upper.brick_masks());
        for (i, packed) in run.masks.iter().enumerate() {
            assert_eq!(packed.len(), 3 * 512);
            let (lo, up): (&[u8], &[u8]) = (
                bytemuck::cast_slice(&lower_masks[i]),
                bytemuck::cast_slice(&upper_masks[i]),
            );
            for z in 0..2 {
                let rows = |first: usize, n: usize| {
                    &packed[(z * 6 + first) * 128..(z * 6 + first + n) * 128]
                };
                assert_eq!(rows(0, 2), &lo[z * 256..(z + 1) * 256]);
                assert_eq!(rows(2, 2), &up[z * 256..(z + 1) * 256]);
                assert!(rows(4, 2).iter().all(|&b| b == 0));
            }
        }
        // The colours: zero for the lower and the empty section, the upper's decoded, layer by
        // layer (16 rows of 32 bytes a section).
        let decoded = colours.decode();
        assert_eq!(run.albedo.len(), 3 * 8192);
        for z in 0..16 {
            let rows = |first: usize, n: usize| {
                &run.albedo[(z * 48 + first) * 32..(z * 48 + first + n) * 32]
            };
            assert!(rows(0, 16).iter().all(|&b| b == 0));
            assert_eq!(rows(16, 16), &decoded[z * 512..(z + 1) * 512]);
            assert!(rows(32, 16).iter().all(|&b| b == 0));
        }
        // The bricks: two rows of two bytes a layer a section; the lower section's block is in
        // its brick (0, 0, 0), the upper's cutout in its brick (1, 1, 1).
        assert_eq!(run.bricks.len(), 3 * 8);
        let brick =
            |s: usize, x: usize, y: usize, z: usize| run.bricks[(z * 6 + 2 * s + y) * 2 + x];
        assert_eq!(brick(0, 0, 0, 0), 1);
        assert_eq!(brick(0, 1, 1, 1), 0);
        assert_eq!(brick(1, 1, 1, 1), 3);
        assert_eq!(brick(1, 0, 0, 0), 0);
        assert_eq!(run.bricks.iter().filter(|&&b| b != 0).count(), 2);
    }

    #[test]
    fn runs_take_the_nearest_columns_first_within_the_budget() {
        let mut dirty: HashSet<SectionKey> = HashSet::new();
        // A far column whole, a near one with sections 2, 3 and 5, and one out of range.
        dirty.extend((0..SECTIONS as i32).map(|sy| (5, sy, 0)));
        dirty.extend([(1, 2, 0), (1, 3, 0), (1, 5, 0)]);
        dirty.insert((RADIUS + 3, 0, 0));
        let runs = plan_runs(&mut dirty, (0, 0), 3);
        assert_eq!(runs, vec![((1, 0), 2, 2), ((1, 0), 5, 1)]);
        // The far column waits for the next frame (the budget was spent); the one out of range
        // is dropped.
        assert_eq!(dirty.len(), SECTIONS);
        assert!(dirty.iter().all(|&(x, _, _)| x == 5));
        let runs = plan_runs(&mut dirty, (0, 0), SECTIONS);
        assert_eq!(runs, vec![((5, 0), 0, SECTIONS as u32)]);
        assert!(dirty.is_empty());
    }

    #[test]
    fn a_light_reaches_the_cells_around_it_only() {
        // A torch (level 14) at world (33, 201, 26): section (2, 16, 1), grid around chunk (0, 0).
        let mut sections = HashMap::new();
        sections.insert(
            (2, 16, 1),
            vec![[1, 9, 10, 14, 255, 170, 90, 255, 255, 255]],
        );
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
