//! The rays' far field (docs/raytracing.md): the terrain's heights around the camera, past the
//! voxel grid (`voxels`) as far as the distant terrain goes, for the light of the sun or the moon
//! and of the sky where the grid ends (shadow_rays.wgsl `far_light`, `far_sky`).
//! One texel stands for 2x2 blocks: the top of the highest of them, from the resident sections'
//! blocks (`Voxels::column_tops`) and, beyond them, the drawn distant tiles' cells
//! (`Lod::drawn_columns`). `SIZE` texels a side, 2048 blocks, centred on the camera and addressed
//! toroidally as the grid is (a texel's slot is its world texel modulo `SIZE`), with a pyramid of
//! maxima in the mip levels, so a march crosses open air a whole coarse texel a step.
//!
//! The rays don't march it pixel by pixel: a compute pass (far_field.wgsl) works out, for every
//! texel, the height under which the terrain shades it from the light and how much sky it sees,
//! when the heights or the light change, and every view (the screen, each VR eye) reads that.
//!
//! It is written incrementally, a chunk column's 8x8 texels at a time (the window's first texel a
//! multiple of 8, so a slot block is one chunk column): a distant tile that came, went or changed
//! its cells rewrites the chunk columns it covers, a resident column whose tops changed or that
//! left its own, and a recentre the columns the window moved onto, in the slots of those it left
//! (as `voxels` does for its grid). It used to rebuild all of it, 3.3 ms of CPU measured on the
//! RTX 4090, every 500 ms in flight (the distant tiles' revision changes all the time there).
//! A pass takes what changed at most every `REBUILD_STEP` and writes `BUDGET` chunk columns a
//! frame of it; span `far_heights`.

use crate::chunk::MIN_Y;
use crate::fast_hash::{FxHashMap, FxHashSet};
use crate::lod::{Column, TileKey};
use glam::DVec3;
use std::time::{Duration, Instant};

/// Texels a side of the finest level (a power of two).
pub const SIZE: usize = 1024;
/// Blocks a texel side, as a shift.
const SHIFT: i32 = 1;
/// Mip levels, down to one texel.
pub const LEVELS: u32 = SIZE.trailing_zeros() + 1;
/// The window moves when the camera has gone this many texels from its centre.
const RECENTER: i32 = 64;
/// A change is written at most this often (and the far field's pass run again).
const REBUILD_STEP: Duration = Duration::from_millis(500);
/// A chunk column's texels a side, and the window's chunk columns a side.
const CHUNK_TEXELS: i32 = 16 >> SHIFT;
const CHUNKS: usize = SIZE / CHUNK_TEXELS as usize;
/// The first level where a chunk column is under one texel: from it the levels (64x64 texels and
/// less, 11 KB) are reduced and uploaded whole.
const WHOLE_FROM: usize = CHUNK_TEXELS.trailing_zeros() as usize + 1;
/// Chunk columns a pass writes a frame: the 16384 of a full rebuild took 3.3 ms, so about 0.4 ms.
/// A recentre's columns (up to 2032 for 64 texels diagonally) count in it but are all written in
/// their frame, the shader's window moving with them.
const BUDGET: usize = 2048;

/// A drawn distant tile's cells' heights (`i16::MIN` none).
struct FarTile {
    origin: [i32; 2],
    stride: i32,
    cells: usize,
    heights: Vec<i16>,
}
impl FarTile {
    /// The chunk columns it covers, inclusive.
    fn chunks(&self) -> [i32; 4] {
        let size = self.stride * self.cells as i32;
        let [x0, z0] = self.origin;
        [x0 >> 4, z0 >> 4, (x0 + size - 1) >> 4, (z0 + size - 1) >> 4]
    }
}

/// A set of the window's chunk columns, by slot (`(z & 127) * 128 + (x & 127)`).
#[derive(Clone)]
struct ChunkSet(Box<[u64]>);
impl Default for ChunkSet {
    fn default() -> Self {
        Self(vec![0; CHUNKS * CHUNKS / 64].into_boxed_slice())
    }
}
impl ChunkSet {
    fn slot(x: i32, z: i32) -> usize {
        let mask = CHUNKS as i32 - 1;
        ((z & mask) as usize) * CHUNKS + (x & mask) as usize
    }
    fn insert(&mut self, i: usize) {
        self.0[i / 64] |= 1 << (i % 64);
    }
    fn contains(&self, i: usize) -> bool {
        (self.0[i / 64] >> (i % 64)) & 1 == 1
    }
    fn is_empty(&self) -> bool {
        self.0.iter().all(|&w| w == 0)
    }
    fn len(&self) -> usize {
        self.0.iter().map(|w| w.count_ones() as usize).sum()
    }
    fn fill(&mut self) {
        self.0.fill(u64::MAX);
    }
    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(i, &w)| {
            let mut w = w;
            std::iter::from_fn(move || {
                (w != 0).then(|| {
                    let bit = w.trailing_zeros() as usize;
                    w &= w - 1;
                    i * 64 + bit
                })
            })
        })
    }
    /// Takes `other`'s members out of this set.
    fn remove_all(&mut self, other: &Self) {
        for (w, o) in self.0.iter_mut().zip(other.0.iter()) {
            *w &= !o;
        }
    }
    /// Moves up to `n` members of this set (in slot order) into `into`.
    fn take_into(&mut self, n: usize, into: &mut Self) {
        let mut left = n;
        for (w, o) in self.0.iter_mut().zip(into.0.iter_mut()) {
            if left == 0 {
                break;
            }
            while *w != 0 && left > 0 {
                let bit = *w & w.wrapping_neg();
                *w &= !bit;
                *o |= bit;
                left -= 1;
            }
        }
    }
    /// The set as rectangles of slots `[x, z, w, h]`: each row's runs, a run continuing the one
    /// above it when it spans the same slots (a recentre's strip is one rectangle).
    fn rects(&self) -> Vec<[usize; 4]> {
        let mut done = Vec::new();
        let mut open: Vec<[usize; 4]> = Vec::new();
        for z in 0..CHUNKS {
            let mut next = Vec::new();
            let mut x = 0;
            while x < CHUNKS {
                if !self.contains(z * CHUNKS + x) {
                    x += 1;
                    continue;
                }
                let from = x;
                while x < CHUNKS && self.contains(z * CHUNKS + x) {
                    x += 1;
                }
                match open.iter().position(|r| r[0] == from && r[2] == x - from) {
                    Some(i) => {
                        let mut r = open.swap_remove(i);
                        r[3] += 1;
                        next.push(r);
                    }
                    None => next.push([from, z, x - from, 1]),
                }
            }
            done.append(&mut open);
            open = next;
        }
        done.append(&mut open);
        done
    }
}

#[derive(Default)]
pub struct FarHeights {
    /// Each resident chunk column's block columns' tops (`Voxels::column_tops`).
    tops: FxHashMap<(i32, i32), Box<[i16; 256]>>,
    /// Chunk columns whose sections changed: their tops are read again.
    stale: FxHashSet<(i32, i32)>,
    far: FxHashMap<TileKey, FarTile>,
    far_revision: Option<u64>,
    /// The window's chunk columns to write again, at the next pass.
    dirty: ChunkSet,
    /// Those the pass under way has still to write.
    pass: ChunkSet,
    /// The pyramid, finest first, slots toroidal.
    levels: Vec<Vec<i16>>,
    texture: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// The window's first texel (world texel coordinates, multiples of `CHUNK_TEXELS`), once built.
    origin: Option<[i32; 2]>,
    built_at: Option<Instant>,
    /// The highest top in the window.
    top: i16,
    /// The far field's pass and what it writes (`prepare`).
    field: Option<Field>,
    /// The heights changed since the pass last ran.
    uploaded: bool,
    /// The light the pass last ran for.
    lit_for: Option<[f32; 3]>,
}

/// far_field.wgsl's pipelines, its uniform, its outputs (the light's shadow heights, the sky
/// seen toward eight directions) and its group.
struct Field {
    light: wgpu::ComputePipeline,
    sky: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    views: [wgpu::TextureView; 3],
    group: wgpu::BindGroup,
}

/// What a frame wrote (`FarHeights::advance`): the slot rectangles of chunk columns, all of them
/// when the texture is to be written whole.
#[derive(Default)]
struct Writes {
    rects: Vec<[usize; 4]>,
    whole: bool,
}

/// Marks in `set` the chunk columns of `range` (inclusive, `FarTile::chunks`) in the window.
fn mark(set: &mut ChunkSet, origin: Option<[i32; 2]>, range: [i32; 4]) {
    let Some(origin) = origin else {
        // The first build writes everything.
        return;
    };
    let (ox, oz) = (origin[0] / CHUNK_TEXELS, origin[1] / CHUNK_TEXELS);
    let n = CHUNKS as i32;
    for z in range[1].max(oz)..=range[3].min(oz + n - 1) {
        for x in range[0].max(ox)..=range[2].min(ox + n - 1) {
            set.insert(ChunkSet::slot(x, z));
        }
    }
}

impl FarHeights {
    /// Section column (x, z) changed: meshed again or gone.
    pub fn changed(&mut self, x: i32, z: i32) {
        self.stale.insert((x, z));
    }

    pub fn clear(&mut self) {
        self.tops.clear();
        self.stale.clear();
        self.far.clear();
        self.far_revision = None;
        self.dirty = ChunkSet::default();
        self.pass = ChunkSet::default();
        // Built again whole at the next pass, the far field off until then.
        self.origin = None;
    }

    /// The distant terrain's drawn tiles, when they changed (`revision`, `Lod::revision`): the
    /// chunk columns under the tiles that came, went or changed their cells are to be written.
    pub fn set_far<'a>(
        &mut self,
        revision: u64,
        tiles: impl Iterator<Item = (TileKey, &'a [Column])>,
    ) {
        if self.far_revision == Some(revision) {
            return;
        }
        self.far_revision = Some(revision);
        let height = |c: &Column| c.surface.map_or(i16::MIN, |s| s.height as i16);
        let mut old = std::mem::take(&mut self.far);
        for (key, columns) in tiles {
            if self.far.contains_key(&key) {
                continue;
            }
            let tile = match old.remove(&key) {
                Some(tile)
                    if tile.heights.len() == columns.len()
                        && tile
                            .heights
                            .iter()
                            .zip(columns)
                            .all(|(&h, c)| h == height(c)) =>
                {
                    tile
                }
                _ => {
                    let origin = key.origin();
                    let tile = FarTile {
                        origin: [origin[0], origin[2]],
                        stride: key.stride(),
                        cells: key.cells(),
                        heights: columns.iter().map(height).collect(),
                    };
                    mark(&mut self.dirty, self.origin, tile.chunks());
                    tile
                }
            };
            self.far.insert(key, tile);
        }
        for tile in old.values() {
            mark(&mut self.dirty, self.origin, tile.chunks());
        }
    }

    /// Writes what changed and returns the shader's `far`: the window's first texel (x, z), 1
    /// once built, the highest top.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: DVec3,
        voxels: &super::voxels::Voxels,
    ) -> [f32; 4] {
        crate::spans::time("far_heights", || {
            let writes = self.advance(camera, Instant::now(), |x, z| voxels.column_tops(x, z));
            if writes.whole || !writes.rects.is_empty() {
                self.upload(device, queue, &writes);
            }
        });
        match self.origin {
            Some(o) if self.texture.is_some() => [o[0] as f32, o[1] as f32, 1.0, self.top as f32],
            _ => [0.0; 4],
        }
    }

    /// The CPU's part of `update`: at most every `REBUILD_STEP`, a pass takes what changed (the
    /// stale columns' tops read again, the window moved on); then it writes `BUDGET` chunk columns
    /// a frame until it is through.
    fn advance(
        &mut self,
        camera: DVec3,
        now: Instant,
        column_tops: impl Fn(i32, i32) -> [i16; 256],
    ) -> Writes {
        let mut set = ChunkSet::default();
        let mut whole = false;
        if self.pass.is_empty() {
            if self
                .built_at
                .is_some_and(|t| now.duration_since(t) < REBUILD_STEP)
            {
                return Writes::default();
            }
            for (x, z) in std::mem::take(&mut self.stale) {
                let tops = column_tops(x, z);
                let changed = if tops.iter().all(|&t| t == MIN_Y as i16) {
                    // The tiles it hid show again.
                    self.tops.remove(&(x, z)).is_some()
                } else if self.tops.get(&(x, z)).is_none_or(|old| **old != tops) {
                    self.tops.insert((x, z), Box::new(tops));
                    true
                } else {
                    // Meshed again with the same tops (most edits), nothing to write.
                    false
                };
                if changed {
                    mark(&mut self.dirty, self.origin, [x, z, x, z]);
                }
            }
            let center = [
                (camera.x.floor() as i32) >> SHIFT,
                (camera.z.floor() as i32) >> SHIFT,
            ];
            let half = SIZE as i32 / 2;
            let moved = self.origin.is_none_or(|o| {
                (o[0] + half - center[0]).abs() > RECENTER
                    || (o[1] + half - center[1]).abs() > RECENTER
            });
            if moved {
                let origin = [
                    (center[0] - half).div_euclid(CHUNK_TEXELS) * CHUNK_TEXELS,
                    (center[1] - half).div_euclid(CHUNK_TEXELS) * CHUNK_TEXELS,
                ];
                let n = CHUNKS as i32;
                let (nx, nz) = (origin[0] / CHUNK_TEXELS, origin[1] / CHUNK_TEXELS);
                match self.origin {
                    Some(old) if !self.levels.is_empty() => {
                        // The columns the window moved onto, in the slots of those it left.
                        let (ox, oz) = (old[0] / CHUNK_TEXELS, old[1] / CHUNK_TEXELS);
                        for z in nz..nz + n {
                            for x in nx..nx + n {
                                let kept = (ox..ox + n).contains(&x) && (oz..oz + n).contains(&z);
                                if !kept {
                                    set.insert(ChunkSet::slot(x, z));
                                }
                            }
                        }
                    }
                    _ => {
                        set.fill();
                        whole = true;
                    }
                }
                self.origin = Some(origin);
            }
            self.pass = std::mem::take(&mut self.dirty);
            self.pass.remove_all(&set);
            if set.is_empty() && self.pass.is_empty() {
                return Writes::default();
            }
        }
        let urgent = !set.is_empty();
        self.pass
            .take_into(BUDGET.saturating_sub(set.len()), &mut set);
        self.write(&set);
        if urgent || self.pass.is_empty() {
            self.uploaded = true;
        }
        if self.pass.is_empty() {
            self.built_at = Some(now);
        }
        Writes {
            rects: if whole { Vec::new() } else { set.rects() },
            whole,
        }
    }

    /// Writes the chunk columns of `set` from the tiles and the tops, and the maxima over them.
    fn write(&mut self, set: &ChunkSet) {
        let Some(origin) = self.origin else {
            return;
        };
        if self.levels.is_empty() {
            self.levels = (0..LEVELS)
                .map(|l| vec![MIN_Y as i16; (SIZE >> l) * (SIZE >> l)])
                .collect();
        }
        let n = CHUNKS as i32;
        let (ox, oz) = (origin[0] / CHUNK_TEXELS, origin[1] / CHUNK_TEXELS);
        let level0 = &mut self.levels[0];
        // The resident chunk columns from their tops, the others cleared for the tiles.
        let mut from_tiles = ChunkSet::default();
        for slot in set.iter() {
            let (sx, sz) = ((slot % CHUNKS) as i32, (slot / CHUNKS) as i32);
            let chunk = (ox + (sx - ox).rem_euclid(n), oz + (sz - oz).rem_euclid(n));
            match self.tops.get(&chunk) {
                Some(tops) => {
                    chunk_texels(level0, origin, chunk, tops);
                }
                None => {
                    for dz in 0..CHUNK_TEXELS as usize {
                        let row = (sz as usize * CHUNK_TEXELS as usize + dz) * SIZE
                            + sx as usize * CHUNK_TEXELS as usize;
                        level0[row..row + CHUNK_TEXELS as usize].fill(MIN_Y as i16);
                    }
                    from_tiles.insert(slot);
                }
            }
        }
        // The distant tiles over those, the coarsest stretching over many columns.
        if !from_tiles.is_empty() {
            for tile in self.far.values() {
                let [x0, z0, x1, z1] = tile.chunks();
                for cz in z0.max(oz)..=z1.min(oz + n - 1) {
                    for cx in x0.max(ox)..=x1.min(ox + n - 1) {
                        if from_tiles.contains(ChunkSet::slot(cx, cz)) {
                            tile_texels(level0, tile, (cx, cz));
                        }
                    }
                }
            }
        }
        // The maxima over them, level by level. The slots stay toroidal: a coarse texel's four
        // are its world texel's children's slots.
        for l in 1..WHOLE_FROM {
            let w = (CHUNK_TEXELS >> l) as usize;
            for slot in set.iter() {
                let (sx, sz) = (slot % CHUNKS, slot / CHUNKS);
                for y in sz * w..sz * w + w {
                    for x in sx * w..sx * w + w {
                        reduce(&mut self.levels, l, x, y);
                    }
                }
            }
        }
        for l in WHOLE_FROM..LEVELS as usize {
            let side = SIZE >> l;
            for y in 0..side {
                for x in 0..side {
                    reduce(&mut self.levels, l, x, y);
                }
            }
        }
        self.top = self.levels[LEVELS as usize - 1][0];
    }

    /// The far field's textures (shadow_rays.wgsl `far_light_map`, `far_sky_map`,
    /// `far_sky_map2`), once the pass has run.
    pub fn views(&self) -> Option<[&wgpu::TextureView; 3]> {
        let field = self.field.as_ref().filter(|_| self.lit_for.is_some())?;
        Some([&field.views[0], &field.views[1], &field.views[2]])
    }

    /// Runs the far field's pass into `encoder` where the heights or the light (toward it, unit)
    /// changed since it last ran: once a frame at most, whatever the views.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        light: [f32; 3],
    ) {
        let (Some((_, heights)), Some(origin)) = (&self.texture, self.origin) else {
            return;
        };
        let field = self
            .field
            .get_or_insert_with(|| Field::new(device, heights));
        let sky = std::mem::take(&mut self.uploaded);
        if !sky && self.lit_for == Some(light) {
            return;
        }
        let params: [f32; 8] = [
            origin[0] as f32,
            origin[1] as f32,
            f32::from(self.top),
            0.0,
            light[0],
            light[1],
            light[2],
            0.0,
        ];
        queue.write_buffer(&field.params, 0, bytemuck::cast_slice(&params));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("RT far field"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &field.group, &[]);
        let groups = (SIZE as u32).div_ceil(8);
        pass.set_pipeline(&field.light);
        pass.dispatch_workgroups(groups, groups, 1);
        if sky {
            pass.set_pipeline(&field.sky);
            pass.dispatch_workgroups(groups, groups, 1);
        }
        drop(pass);
        super::gpu_passes::stamp(encoder, "rays: far field");
        self.lit_for = Some(light);
    }

    /// Uploads what `writes` says was written: its rectangles at the levels where a chunk column
    /// is a texel or more, the coarser levels whole.
    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, writes: &Writes) {
        let created = self.texture.is_none();
        let (texture, _) = self.texture.get_or_insert_with(|| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Far heights"),
                size: wgpu::Extent3d {
                    width: SIZE as u32,
                    height: SIZE as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: LEVELS,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R16Sint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            (texture, view)
        });
        let write = |l: usize, [x, y, w, h]: [usize; 4]| {
            let side = SIZE >> l;
            let level = &self.levels[l];
            let rect: Vec<i16> = if w == side {
                level[y * side..(y + h) * side].to_vec()
            } else {
                (y..y + h)
                    .flat_map(|row| &level[row * side + x..row * side + x + w])
                    .copied()
                    .collect()
            };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: l as u32,
                    origin: wgpu::Origin3d {
                        x: x as u32,
                        y: y as u32,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&rect),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w as u32 * 2),
                    rows_per_image: Some(h as u32),
                },
                wgpu::Extent3d {
                    width: w as u32,
                    height: h as u32,
                    depth_or_array_layers: 1,
                },
            );
        };
        for (l, rect) in writes.regions(created) {
            write(l, rect);
        }
    }
}

impl Writes {
    /// Each level's texel rectangles `[x, y, w, h]` to upload (`whole` too when the texture is
    /// new).
    fn regions(&self, whole: bool) -> Vec<(usize, [usize; 4])> {
        let mut out = Vec::new();
        for l in 0..LEVELS as usize {
            let side = SIZE >> l;
            if self.whole || whole || l >= WHOLE_FROM {
                out.push((l, [0, 0, side, side]));
            } else {
                let t = (CHUNK_TEXELS >> l) as usize;
                for &[x, z, w, h] in &self.rects {
                    out.push((l, [x * t, z * t, w * t, h * t]));
                }
            }
        }
        out
    }
}

/// Chunk column `chunk`'s tops (`tops`) into its texels of `level0` (the window from texel
/// `origin`), each the highest of its 2x2 block columns; false when it is off the window.
fn chunk_texels(
    level0: &mut [i16],
    origin: [i32; 2],
    chunk: (i32, i32),
    tops: &[i16; 256],
) -> bool {
    let n = SIZE as i32;
    let (tx0, tz0) = ((chunk.0 * 16) >> SHIFT, (chunk.1 * 16) >> SHIFT);
    let side = 16 >> SHIFT;
    if tx0 + side <= origin[0]
        || tz0 + side <= origin[1]
        || tx0 >= origin[0] + n
        || tz0 >= origin[1] + n
    {
        return false;
    }
    for dz in 0..side {
        for dx in 0..side {
            let mut h = MIN_Y as i16;
            for i in 0..1 << SHIFT {
                for j in 0..1 << SHIFT {
                    let x = (dx << SHIFT) + j;
                    let z = (dz << SHIFT) + i;
                    h = h.max(tops[(z * 16 + x) as usize]);
                }
            }
            let slot = (((tz0 + dz) & (n - 1)) * n + ((tx0 + dx) & (n - 1))) as usize;
            level0[slot] = h;
        }
    }
    true
}

/// `tile`'s cells over chunk column `chunk`'s texels of `level0`, each the highest of what is
/// there and the cells over it: a cell covers the texels of its blocks.
fn tile_texels(level0: &mut [i16], tile: &FarTile, chunk: (i32, i32)) {
    let n = SIZE as i32;
    let s = tile.stride;
    let [x0, z0] = tile.origin;
    let (bx0, bz0) = (chunk.0 * 16, chunk.1 * 16);
    let cells = |b0: i32, o: i32| {
        let from = (b0 - o).div_euclid(s).max(0) as usize;
        let to = ((b0 + 16 - o + s - 1).div_euclid(s).max(0) as usize).min(tile.cells);
        from..to
    };
    let (ct0, ct1) = (bx0 >> SHIFT, ((bx0 + 16) >> SHIFT) - 1);
    let (rt0, rt1) = (bz0 >> SHIFT, ((bz0 + 16) >> SHIFT) - 1);
    for cz in cells(bz0, z0) {
        let bz = z0 + cz as i32 * s;
        let tz0 = (bz >> SHIFT).max(rt0);
        let tz1 = ((bz + s - 1) >> SHIFT).min(rt1);
        for cx in cells(bx0, x0) {
            let h = tile.heights[cz * tile.cells + cx];
            if h == i16::MIN {
                continue;
            }
            let bx = x0 + cx as i32 * s;
            let tx0 = (bx >> SHIFT).max(ct0);
            let tx1 = ((bx + s - 1) >> SHIFT).min(ct1);
            for tz in tz0..=tz1 {
                for tx in tx0..=tx1 {
                    let i = ((tz & (n - 1)) * n + (tx & (n - 1))) as usize;
                    level0[i] = level0[i].max(h);
                }
            }
        }
    }
}

/// Level `l`'s texel in slot (x, y): the highest of its four in the level below.
fn reduce(levels: &mut [Vec<i16>], l: usize, x: usize, y: usize) {
    let (fine, coarse) = levels.split_at_mut(l);
    let fine = &fine[l - 1];
    let side = SIZE >> l;
    let f = |dx: usize, dy: usize| fine[(2 * y + dy) * 2 * side + 2 * x + dx];
    coarse[0][y * side + x] = f(0, 0).max(f(1, 0)).max(f(0, 1)).max(f(1, 1));
}

impl Field {
    fn new(device: &wgpu::Device, heights: &wgpu::TextureView) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RT far field"),
            source: wgpu::ShaderSource::Wgsl(include_str!("far_field.wgsl").into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let output = |format| wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT far field"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Sint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(2, output(wgpu::TextureFormat::Rg32Float)),
                entry(3, output(wgpu::TextureFormat::Rgba8Unorm)),
                entry(4, output(wgpu::TextureFormat::Rgba8Unorm)),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RT far field"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("RT far field"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RT far field"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let views = [
            wgpu::TextureFormat::Rg32Float,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ]
        .map(|format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("RT far field"),
                    size: wgpu::Extent3d {
                        width: SIZE as u32,
                        height: SIZE as u32,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::STORAGE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
        let view = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RT far field"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                view(1, heights),
                view(2, &views[0]),
                view(3, &views[1]),
                view(4, &views[2]),
            ],
        });
        Self {
            light: pipeline("cs_light"),
            sky: pipeline("cs_sky"),
            params,
            views,
            group,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lod::Surface;
    use horizon_world::registry::BlockStateId;

    /// The full rebuild the incremental writes replaced, kept as their reference: every texel
    /// from the tiles, then the resident chunk columns over them, then the maxima.
    fn rebuilt(far: &FarHeights, origin: [i32; 2]) -> Vec<Vec<i16>> {
        let n = SIZE as i32;
        let mask = n - 1;
        let mut levels: Vec<Vec<i16>> = (0..LEVELS)
            .map(|l| vec![MIN_Y as i16; (SIZE >> l) * (SIZE >> l)])
            .collect();
        let level0 = &mut levels[0];
        let slot = |tx: i32, tz: i32| ((tz & mask) * n + (tx & mask)) as usize;
        let window = [origin[0], origin[1], origin[0] + n, origin[1] + n];
        for tile in far.far.values() {
            let size = tile.stride * tile.cells as i32;
            let [x0, z0] = tile.origin;
            if (x0 + size) >> SHIFT <= window[0]
                || (z0 + size) >> SHIFT <= window[1]
                || x0 >> SHIFT >= window[2]
                || z0 >> SHIFT >= window[3]
            {
                continue;
            }
            for cz in 0..tile.cells {
                let bz = z0 + cz as i32 * tile.stride;
                let tz0 = (bz >> SHIFT).max(window[1]);
                let tz1 = ((bz + tile.stride - 1) >> SHIFT).min(window[3] - 1);
                for cx in 0..tile.cells {
                    let h = tile.heights[cz * tile.cells + cx];
                    if h == i16::MIN {
                        continue;
                    }
                    let bx = x0 + cx as i32 * tile.stride;
                    let tx0 = (bx >> SHIFT).max(window[0]);
                    let tx1 = ((bx + tile.stride - 1) >> SHIFT).min(window[2] - 1);
                    for tz in tz0..=tz1 {
                        for tx in tx0..=tx1 {
                            let s = slot(tx, tz);
                            level0[s] = level0[s].max(h);
                        }
                    }
                }
            }
        }
        for (&chunk, tops) in &far.tops {
            chunk_texels(level0, origin, chunk, tops);
        }
        for l in 1..LEVELS as usize {
            let side = SIZE >> l;
            for y in 0..side {
                for x in 0..side {
                    reduce(&mut levels, l, x, y);
                }
            }
        }
        levels
    }

    /// Runs passes until the one under way is through, applying each frame's uploads to a copy of
    /// the texture; returns the frames it took.
    fn settle(
        far: &mut FarHeights,
        gpu: &mut Vec<Vec<i16>>,
        camera: DVec3,
        now: &mut Instant,
        world: &FxHashMap<(i32, i32), [i16; 256]>,
    ) -> usize {
        let tops = |x: i32, z: i32| world.get(&(x, z)).copied().unwrap_or([MIN_Y as i16; 256]);
        *now += REBUILD_STEP;
        let mut frames = 0;
        loop {
            let writes = far.advance(camera, *now, tops);
            let created = gpu.is_empty();
            if created && !far.levels.is_empty() {
                *gpu = far.levels.iter().map(|l| vec![0; l.len()]).collect();
            }
            for (l, [x, y, w, h]) in writes.regions(created) {
                let side = SIZE >> l;
                for row in y..y + h {
                    let r = row * side + x..row * side + x + w;
                    gpu[l][r.clone()].copy_from_slice(&far.levels[l][r]);
                }
            }
            frames += 1;
            if far.pass.is_empty() {
                return frames;
            }
            *now += Duration::from_millis(16);
        }
    }

    fn column(height: i16) -> Column {
        Column {
            surface: (height != i16::MIN).then_some(Surface {
                height: i32::from(height),
                state: BlockStateId(0),
                water: None,
                biome: 0,
            }),
            spans: Vec::new(),
        }
    }

    /// A small xorshift, the test's terrain.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: i32) -> i32 {
            (self.next() % n as u64) as i32
        }
    }

    #[test]
    fn a_tile_and_a_chunk_fill_their_texels_and_the_pyramid_keeps_the_maxima() {
        let mut far = FarHeights::default();
        let key = TileKey {
            x: -2,
            z: -2,
            level: 1,
        };
        // A 32-block tile from (-64, -64) of 16 cells, 2 blocks each: its first cell's texel
        // (-32, -32) at 100, the next at none, the second row's first at 80.
        let mut columns: Vec<Column> = (0..256).map(|_| column(i16::MIN)).collect();
        columns[0] = column(100);
        columns[16] = column(80);
        columns[17] = column(90);
        far.set_far(1, [(key, columns.as_slice())].into_iter());
        let mut tops = [MIN_Y as i16; 256];
        tops[3 * 16 + 5] = 120;
        let world = [((0, 0), tops)].into_iter().collect();
        far.changed(0, 0);
        let (mut gpu, mut now) = (Vec::new(), Instant::now());
        settle(&mut far, &mut gpu, DVec3::ZERO, &mut now, &world);
        assert_eq!(far.origin, Some([-512, -512]));
        let n = SIZE as i32;
        let at = |level: usize, tx: i32, tz: i32| {
            let side = n >> level;
            far.levels[level][((tz & (side - 1)) * side + (tx & (side - 1))) as usize]
        };
        assert_eq!(at(0, -32, -32), 100);
        assert_eq!(at(0, -31, -32), MIN_Y as i16);
        assert_eq!(at(0, -32, -31), 80);
        assert_eq!(at(0, -31, -31), 90);
        // Block (5, 3) is texel (2, 1), the max of its 2x2 blocks.
        assert_eq!(at(0, 2, 1), 120);
        assert_eq!(at(0, 3, 1), MIN_Y as i16);
        assert_eq!(at(1, 1, 0), 120);
        assert_eq!(at(3, -4, -4), 100);
        assert_eq!(far.top, 120);
        assert!(gpu == far.levels);
    }

    /// Flying over a synthetic terrain, the distant tiles coming, going and changing, chunk
    /// columns meshed and leaving, the window moving: the incremental writes leave the pyramid,
    /// and the texture's copy, as a full rebuild makes them.
    #[test]
    fn the_incremental_writes_match_a_full_rebuild() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut far = FarHeights::default();
        let (mut gpu, mut now) = (Vec::new(), Instant::now());
        let mut world: FxHashMap<(i32, i32), [i16; 256]> = FxHashMap::default();
        let mut tiles: FxHashMap<TileKey, Vec<Column>> = FxHashMap::default();
        let new_tile = |rng: &mut Rng, key: TileKey| -> Vec<Column> {
            let cells = key.cells();
            (0..cells * cells)
                .map(|_| {
                    let h = rng.below(300) - 64;
                    column(if h < -40 { i16::MIN } else { h as i16 })
                })
                .collect()
        };
        let mut camera = DVec3::new(100.0, 70.0, -50.0);
        for step in 0..24u64 {
            // Some tiles come, at every level the LOD draws, some go, some change.
            for _ in 0..12 {
                let level = rng.below(8) as u8;
                let size = 16 << level;
                let reach = 1400 / size + 1;
                let key = TileKey {
                    x: (camera.x as i32).div_euclid(size) + rng.below(2 * reach) - reach,
                    z: (camera.z as i32).div_euclid(size) + rng.below(2 * reach) - reach,
                    level,
                };
                let columns = new_tile(&mut rng, key);
                tiles.insert(key, columns);
            }
            let keys: Vec<TileKey> = tiles.keys().copied().collect();
            for key in keys {
                match rng.below(10) {
                    0 => {
                        tiles.remove(&key);
                    }
                    1 => {
                        let cells = tiles[&key].len();
                        let i = rng.below(cells as i32) as usize;
                        tiles.get_mut(&key).unwrap()[i] = column(rng.below(200) as i16);
                    }
                    _ => {}
                }
            }
            far.set_far(step, tiles.iter().map(|(k, c)| (*k, c.as_slice())));
            // Chunk columns around the camera meshed, changed, gone.
            let (cx, cz) = ((camera.x as i32) >> 4, (camera.z as i32) >> 4);
            for _ in 0..40 {
                let chunk = (cx + rng.below(40) - 20, cz + rng.below(40) - 20);
                if rng.below(4) == 0 {
                    world.remove(&chunk);
                } else {
                    let tops = world.entry(chunk).or_insert([MIN_Y as i16; 256]);
                    for _ in 0..8 {
                        tops[rng.below(256) as usize] = (rng.below(320) - 64) as i16;
                    }
                }
                far.changed(chunk.0, chunk.1);
            }
            settle(&mut far, &mut gpu, camera, &mut now, &world);
            let origin = far.origin.expect("built");
            assert!(far.levels == rebuilt(&far, origin), "step {step}");
            assert_eq!(far.top, far.levels[LEVELS as usize - 1][0]);
            assert!(gpu == far.levels, "the texture's copy, step {step}");
            // Flying on, 150 blocks a step, now and then far away.
            camera += if step == 15 {
                DVec3::new(-5000.0, 0.0, 3000.0)
            } else {
                DVec3::new(150.0, 0.0, 90.0)
            };
        }
    }

    /// A tile's change writes the chunk columns under it alone, and a big change is spread over
    /// frames, `BUDGET` columns each.
    #[test]
    fn a_change_writes_its_columns_and_a_pass_keeps_to_its_budget() {
        let mut far = FarHeights::default();
        let (mut gpu, mut now) = (Vec::new(), Instant::now());
        let world = FxHashMap::default();
        far.set_far(0, std::iter::empty());
        settle(&mut far, &mut gpu, DVec3::ZERO, &mut now, &world);
        // One level 2 tile, 64 blocks: 4x4 chunk columns.
        let key = TileKey {
            x: 3,
            z: -2,
            level: 2,
        };
        let columns: Vec<Column> = (0..256).map(|i| column(i as i16)).collect();
        far.set_far(1, [(key, columns.as_slice())].into_iter());
        assert_eq!(far.dirty.len(), 16);
        now += REBUILD_STEP;
        let writes = far.advance(DVec3::ZERO, now, |_, _| [MIN_Y as i16; 256]);
        assert!(!writes.whole);
        assert_eq!(writes.rects, vec![[12, 120, 4, 4]]);
        // Four level 9 tiles, 8192 blocks each: the whole window, over 8 frames.
        let columns: Vec<Column> = (0..32 * 32).map(|_| column(70)).collect();
        let keys = [(-1, -1), (0, -1), (-1, 0), (0, 0)].map(|(x, z)| TileKey { x, z, level: 9 });
        far.set_far(2, keys.iter().map(|&k| (k, columns.as_slice())));
        let frames = settle(&mut far, &mut gpu, DVec3::ZERO, &mut now, &world);
        assert_eq!(frames, CHUNKS * CHUNKS / BUDGET);
        assert!(far.levels == rebuilt(&far, far.origin.unwrap()));
        assert!(gpu == far.levels);
    }
}
