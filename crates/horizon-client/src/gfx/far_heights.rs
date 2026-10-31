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

/// A drawn distant tile's cells' heights (`i16::MIN` none).
struct FarTile {
    origin: [i32; 2],
    stride: i32,
    cells: usize,
    heights: Vec<i16>,
}

#[derive(Default)]
pub struct FarHeights {
    /// Each resident chunk column's block columns' tops (`Voxels::column_tops`).
    tops: FxHashMap<(i32, i32), Box<[i16; 256]>>,
    /// Chunk columns whose sections changed: their tops are read again.
    stale: FxHashSet<(i32, i32)>,
    /// Chunk columns whose tops changed: their texels are written again, and their maxima.
    patched: FxHashSet<(i32, i32)>,
    far: Vec<FarTile>,
    far_revision: Option<u64>,
    /// The distant tiles changed, or a chunk column left: everything is written again.
    dirty: bool,
    /// The pyramid, finest first, slots toroidal.
    levels: Vec<Vec<i16>>,
    texture: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// The window's first texel (world texel coordinates), once built.
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

impl FarHeights {
    /// Section column (x, z) changed: meshed again or gone.
    pub fn changed(&mut self, x: i32, z: i32) {
        self.stale.insert((x, z));
    }

    pub fn clear(&mut self) {
        self.tops.clear();
        self.stale.clear();
        self.patched.clear();
        self.far.clear();
        self.far_revision = None;
        self.dirty = true;
    }

    /// The distant terrain's drawn tiles, when they changed (`revision`, `Lod::revision`).
    pub fn set_far<'a>(
        &mut self,
        revision: u64,
        tiles: impl Iterator<Item = (TileKey, &'a [Column])>,
    ) {
        if self.far_revision == Some(revision) {
            return;
        }
        self.far_revision = Some(revision);
        self.far = tiles
            .map(|(key, columns)| {
                let origin = key.origin();
                FarTile {
                    origin: [origin[0], origin[2]],
                    stride: key.stride(),
                    cells: key.cells(),
                    heights: columns
                        .iter()
                        .map(|c| c.surface.map_or(i16::MIN, |s| s.height as i16))
                        .collect(),
                }
            })
            .collect();
        self.dirty = true;
    }

    /// Writes what changed, at most every `REBUILD_STEP`, and returns the shader's `far`: the
    /// window's first texel (x, z), 1 once built, the highest top.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: DVec3,
        voxels: &super::voxels::Voxels,
    ) -> [f32; 4] {
        for (x, z) in self.stale.drain() {
            let tops = voxels.column_tops(x, z);
            if tops.iter().all(|&t| t == MIN_Y as i16) {
                // The tiles it hid show again.
                self.dirty |= self.tops.remove(&(x, z)).is_some();
            } else if self.tops.get(&(x, z)).is_none_or(|old| **old != tops) {
                // Meshed again with the same tops (most edits), nothing to write.
                self.tops.insert((x, z), Box::new(tops));
                self.patched.insert((x, z));
            }
        }
        let center = [
            (camera.x.floor() as i32) >> SHIFT,
            (camera.z.floor() as i32) >> SHIFT,
        ];
        let half = SIZE as i32 / 2;
        let moved = self.origin.is_none_or(|o| {
            (o[0] + half - center[0]).abs() > RECENTER || (o[1] + half - center[1]).abs() > RECENTER
        });
        let due = self.built_at.is_none_or(|t| t.elapsed() >= REBUILD_STEP);
        if (self.dirty || moved) && due {
            crate::spans::time("far heights", || {
                self.rebuild([center[0] - half, center[1] - half]);
            });
            self.upload(device, queue);
            self.patched.clear();
            self.uploaded = true;
            self.dirty = false;
            self.built_at = Some(Instant::now());
        } else if !self.patched.is_empty() && due && self.texture.is_some() {
            crate::spans::time("far heights", || {
                for chunk in std::mem::take(&mut self.patched) {
                    self.patch(queue, chunk);
                }
            });
            self.uploaded = true;
            self.built_at = Some(Instant::now());
        }
        match self.origin {
            Some(o) if self.texture.is_some() => [o[0] as f32, o[1] as f32, 1.0, self.top as f32],
            _ => [0.0; 4],
        }
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

    fn rebuild(&mut self, origin: [i32; 2]) {
        let n = SIZE as i32;
        let mask = n - 1;
        if self.levels.is_empty() {
            self.levels = (0..LEVELS)
                .map(|l| vec![0; (SIZE >> l) * (SIZE >> l)])
                .collect();
        }
        let level0 = &mut self.levels[0];
        level0.fill(MIN_Y as i16);
        let slot = |tx: i32, tz: i32| ((tz & mask) * n + (tx & mask)) as usize;
        let window = [origin[0], origin[1], origin[0] + n, origin[1] + n];
        // The distant tiles, the coarsest stretching over many texels.
        for tile in &self.far {
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
                if tz0 > tz1 {
                    continue;
                }
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
        // The resident chunks, over the tiles that stood in for them.
        for (&chunk, tops) in &self.tops {
            chunk_texels(level0, origin, chunk, tops);
        }
        // The maxima, level by level. The slots stay toroidal: a coarse texel's four are its
        // world texel's children's slots.
        for l in 1..LEVELS as usize {
            let side = SIZE >> l;
            for y in 0..side {
                for x in 0..side {
                    reduce(&mut self.levels, l, x, y);
                }
            }
        }
        self.top = self.levels[LEVELS as usize - 1][0];
        self.origin = Some(origin);
    }

    /// Chunk column `chunk`'s tops changed: its texels written again, then the maxima over them
    /// up the levels, and only those uploaded.
    fn patch(&mut self, queue: &wgpu::Queue, chunk: (i32, i32)) {
        let (Some(origin), Some((texture, _)), Some(tops)) =
            (self.origin, &self.texture, self.tops.get(&chunk))
        else {
            return;
        };
        if !chunk_texels(&mut self.levels[0], origin, chunk, tops) {
            return;
        }
        let side0 = 16 >> SHIFT;
        let (tx0, tz0) = ((chunk.0 * 16) >> SHIFT, (chunk.1 * 16) >> SHIFT);
        for l in 0..LEVELS as usize {
            let side = SIZE >> l;
            let mask = side as i32 - 1;
            let w = (side0 >> l).max(1) as usize;
            let (x0, y0) = (((tx0 >> l) & mask) as usize, ((tz0 >> l) & mask) as usize);
            let mut rect = Vec::with_capacity(w * w);
            for y in y0..y0 + w {
                for x in x0..x0 + w {
                    if l > 0 {
                        reduce(&mut self.levels, l, x, y);
                    }
                    rect.push(self.levels[l][y * side + x]);
                }
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: l as u32,
                    origin: wgpu::Origin3d {
                        x: x0 as u32,
                        y: y0 as u32,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&rect),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w as u32 * 2),
                    rows_per_image: Some(w as u32),
                },
                wgpu::Extent3d {
                    width: w as u32,
                    height: w as u32,
                    depth_or_array_layers: 1,
                },
            );
        }
        // An upper bound: lowered at the next rebuild.
        self.top = self.top.max(self.levels[LEVELS as usize - 1][0]);
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
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
        for (l, level) in self.levels.iter().enumerate() {
            let side = (SIZE >> l) as u32;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: l as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(level),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(side * 2),
                    rows_per_image: Some(side),
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
            );
        }
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

    #[test]
    fn a_tile_and_a_chunk_fill_their_texels_and_the_pyramid_keeps_the_maxima() {
        let mut far = FarHeights {
            far: vec![FarTile {
                origin: [-64, -64],
                stride: 16,
                cells: 2,
                heights: vec![100, i16::MIN, 80, 90],
            }],
            ..Default::default()
        };
        let mut tops = Box::new([MIN_Y as i16; 256]);
        tops[3 * 16 + 5] = 120;
        far.tops.insert((0, 0), tops);
        far.rebuild([-512, -512]);
        let n = SIZE as i32;
        let at = |level: usize, tx: i32, tz: i32| {
            let side = n >> level;
            far.levels[level][((tz & (side - 1)) * side + (tx & (side - 1))) as usize]
        };
        // Block (-64, -64) is texel (-32, -32); the tile's first cell spans 8x8 texels.
        assert_eq!(at(0, -32, -32), 100);
        assert_eq!(at(0, -25, -25), 100);
        assert_eq!(at(0, -24, -32), MIN_Y as i16);
        assert_eq!(at(0, -32, -24), 80);
        assert_eq!(at(0, -24, -24), 90);
        // Block (5, 3) is texel (2, 1), the max of its 2x2 blocks.
        assert_eq!(at(0, 2, 1), 120);
        assert_eq!(at(0, 3, 1), MIN_Y as i16);
        assert_eq!(at(1, 1, 0), 120);
        assert_eq!(at(3, -4, -4), 100);
        assert_eq!(far.top, 120);
    }
}
