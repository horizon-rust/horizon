//! RT lighting's cloud geometry, as RTX Remix's clouds are: a mesh, not a volume. A density field
//! of the noise `rt_clouds` makes (the shape, the weather map) is polygonized over square tiles of
//! the slab 700 to 1300 blocks up, by naive surface nets (marching cubes' dual: one vertex a
//! crossing cell, a quad a crossing edge, smooth normals from the field's gradient), and handed to
//! the renderer as ordinary vertex and index buffers (`RtCloudMesh::tiles`): the main session puts
//! them in the ray structures and the lighting. For now `RtCloudMesh::draw` shades them with the
//! sun and the sky over the world, as a soft translucent surface.
//!
//! The wind is a move of the tiles, not a new mesh: tiles are made in cloud space, which drifts
//! over the world at the wind's speed (`origin`), so a tile is only made again when the camera
//! reaches it, or its level of detail changes (32 blocks a cell near the camera, 64 farther out).
//! The meshing runs on a worker thread; the renderer collects what is done each frame.

use super::rt_clouds::{Noise, WIND};
use super::upload::Uploads;
use glam::{DVec3, Mat4, Vec3};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

/// The slab, in blocks (rt_clouds.wgsl `BASE` and `TOP`).
pub const BASE: f32 = 700.0;
pub const TOP: f32 = 1300.0;
/// A tile's side, and how many tiles reach out from the camera's (a square): 6.5 kilometres.
const TILE: f32 = 1024.0;
const RADIUS: i32 = 6;
/// The tiles within this many of the camera's are meshed finely.
const NEAR: i32 = 1;
/// A cell's side near the camera and farther out, in blocks.
const FINE: f32 = 32.0;
const COARSE: f32 = 64.0;
/// Where the density field is the surface (the density of the soft edge, not its core).
const ISO: f32 = 0.08;
/// The tiles the shader's table holds.
const MAX_TILES: usize = 256;
/// Tiles asked of the worker at once.
const IN_FLIGHT: usize = 6;

/// A mesh vertex: its place in the tile (from the tile's origin) and its outward normal.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

/// A tile of cloud space: its index in x and z, and the cells' side (`FINE` or `COARSE`) as a
/// whole number of blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub x: i32,
    pub z: i32,
    pub cell: u32,
}

/// A tile's buffers for the renderer: where its origin is in the world now (vertices are from it),
/// vertex buffers of `Vertex`, and `u32` triangle indices.
pub struct CloudTile<'a> {
    pub key: TileKey,
    pub origin: DVec3,
    pub vertices: &'a wgpu::Buffer,
    pub indices: &'a wgpu::Buffer,
    pub vertex_count: u32,
    pub index_count: u32,
}

struct Tile {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    vertex_count: u32,
    index_count: u32,
}

struct Made {
    key: TileKey,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    view_proj: [[f32; 4]; 4],
    light: [f32; 4],
    light_color: [f32; 4],
    sun: [f32; 4],
    tiles: [[f32; 4]; MAX_TILES],
}

/// A frame's light for the clouds (`rt_clouds::Params`' share).
pub struct Light {
    /// Toward the light the clouds are lit by.
    pub direction: Vec3,
    /// Its colour, in paper whites.
    pub color: [f32; 3],
    /// Toward the sun (the sky table's azimuth).
    pub sun: Vec3,
    /// The Earth's curvature the slab bends by: 1 / its diameter, 0 off.
    pub curvature: f32,
}

pub struct RtCloudMesh {
    tiles: HashMap<TileKey, Tile>,
    pending: HashSet<TileKey>,
    requests: mpsc::Sender<TileKey>,
    done: mpsc::Receiver<Made>,
    /// The cloud space's drift over the world, in blocks (x, z): the tiles' origins are moved
    /// by it.
    drift: [f64; 2],
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

/// The tile index containing cloud-space `v`.
fn tile_of(v: f64) -> i32 {
    (v / f64::from(TILE)).floor() as i32
}

/// The cells' side for a tile `distance` tiles from the camera's.
fn cell_for(distance: i32) -> u32 {
    (if distance <= NEAR { FINE } else { COARSE }) as u32
}

impl RtCloudMesh {
    pub fn new(device: &wgpu::Device) -> Self {
        let noise = Noise::shared();
        let (requests, wanted) = mpsc::channel::<TileKey>();
        let (made, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("cloud-mesher".into())
            .spawn(move || {
                while let Ok(key) = wanted.recv() {
                    let (vertices, indices) = mesh_tile(&noise, key);
                    if made
                        .send(Made {
                            key,
                            vertices,
                            indices,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("the cloud mesher's thread");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RT lighting's cloud mesh"),
            source: wgpu::ShaderSource::Wgsl(include_str!("rt_cloud_mesh.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT lighting's cloud mesh"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("RT lighting's cloud mesh"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("RT lighting's cloud mesh"),
                    bind_group_layouts: &[Some(&layout)],
                    ..Default::default()
                }),
            ),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_cloud"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            // The world's depth, read only (the rays read it as the solid terrain's): reversed,
            // so nearer is greater.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_cloud"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: super::tonemap::SCENE_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RT lighting's cloud mesh"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            tiles: HashMap::new(),
            pending: HashSet::new(),
            requests,
            done,
            drift: [0.0; 2],
            pipeline,
            layout,
            uniform,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Cloud mesh"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        }
    }

    /// Once a frame, before the views draw: takes the tiles the worker has finished, asks for the
    /// nearest missing ones, drops the ones out of reach, and moves the cloud space by the wind
    /// (`time` in seconds).
    pub fn update(&mut self, device: &wgpu::Device, camera: DVec3, time: f64) {
        use wgpu::util::DeviceExt;
        self.drift = [WIND[0] * time, WIND[1] * time];
        let (cx, cz) = (
            tile_of(camera.x - self.drift[0]),
            tile_of(camera.z - self.drift[1]),
        );
        let distance = |k: &TileKey| (k.x - cx).abs().max((k.z - cz).abs());
        while let Ok(made) = self.done.try_recv() {
            self.pending.remove(&made.key);
            if distance(&made.key) > RADIUS || made.indices.is_empty() {
                // Out of reach by now, or empty sky: a tile with nothing is kept as one.
                if made.indices.is_empty() && distance(&made.key) <= RADIUS {
                    self.tiles.insert(made.key, Self::empty(device));
                }
                continue;
            }
            // Where the GPU traces rays, the hybrid builds the tile's structure from these buffers
            // (gfx/hw_rays.rs).
            let input = if device
                .features()
                .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
            {
                wgpu::BufferUsages::BLAS_INPUT
            } else {
                wgpu::BufferUsages::empty()
            };
            let tile = Tile {
                vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Cloud tile vertices"),
                    contents: bytemuck::cast_slice(&made.vertices),
                    usage: wgpu::BufferUsages::VERTEX
                        | wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | input,
                }),
                indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Cloud tile indices"),
                    contents: bytemuck::cast_slice(&made.indices),
                    usage: wgpu::BufferUsages::INDEX
                        | wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | input,
                }),
                vertex_count: made.vertices.len() as u32,
                index_count: made.indices.len() as u32,
            };
            // A tile made again for another level of detail replaces the old one.
            self.tiles
                .retain(|k, _| !(k.x == made.key.x && k.z == made.key.z));
            self.tiles.insert(made.key, tile);
        }
        self.tiles.retain(|k, _| distance(k) <= RADIUS);
        let mut missing: Vec<TileKey> = Vec::new();
        for x in cx - RADIUS..=cx + RADIUS {
            for z in cz - RADIUS..=cz + RADIUS {
                let d = (x - cx).abs().max((z - cz).abs());
                let key = TileKey {
                    x,
                    z,
                    cell: cell_for(d),
                };
                if !self.tiles.contains_key(&key) && !self.pending.contains(&key) {
                    missing.push(key);
                }
            }
        }
        missing.sort_by_key(distance);
        for key in missing {
            if self.pending.len() >= IN_FLIGHT {
                break;
            }
            if self.requests.send(key).is_ok() {
                self.pending.insert(key);
            }
        }
    }

    fn empty(device: &wgpu::Device) -> Tile {
        let buffer = |usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Cloud tile (empty)"),
                size: 4,
                usage,
                mapped_at_creation: false,
            })
        };
        Tile {
            vertices: buffer(wgpu::BufferUsages::VERTEX),
            indices: buffer(wgpu::BufferUsages::INDEX),
            vertex_count: 0,
            index_count: 0,
        }
    }

    /// The tiles with something in them, for the ray structures and the lighting: each tile's
    /// buffers and where its origin is in the world now (the wind moves it).
    pub fn tiles(&self) -> impl Iterator<Item = CloudTile<'_>> {
        let drift = self.drift;
        self.tiles
            .iter()
            .filter(|(_, t)| t.index_count > 0)
            .map(move |(key, t)| CloudTile {
                key: *key,
                origin: DVec3::new(
                    f64::from(key.x) * f64::from(TILE) + drift[0],
                    f64::from(BASE),
                    f64::from(key.z) * f64::from(TILE) + drift[1],
                ),
                vertices: &t.vertices,
                indices: &t.indices,
                vertex_count: t.vertex_count,
                index_count: t.index_count,
            })
    }

    /// The triangles the tiles hold now.
    #[allow(dead_code)] // For the ray structures' budget (the main session).
    pub fn triangles(&self) -> u32 {
        self.tiles.values().map(|t| t.index_count / 3).sum()
    }

    /// Draws the tiles over `target` (the scene of the view whose depth is `depth`), shaded by
    /// the sun's light and the sky.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        view_proj: Mat4,
        camera: DVec3,
        light: &Light,
        sky: &wgpu::TextureView,
    ) {
        let tiles: Vec<CloudTile<'_>> = self.tiles().take(MAX_TILES).collect();
        if tiles.is_empty() {
            return;
        }
        let mut table = [[0.0; 4]; MAX_TILES];
        for (slot, tile) in table.iter_mut().zip(&tiles) {
            let o = tile.origin - camera;
            *slot = [o.x as f32, o.y as f32, o.z as f32, 0.0];
        }
        let uniform = Uniform {
            view_proj: view_proj.to_cols_array_2d(),
            light: [
                light.direction.x,
                light.direction.y,
                light.direction.z,
                light.curvature,
            ],
            light_color: [light.color[0], light.color[1], light.color[2], 0.0],
            sun: [light.sun.x, light.sun.y, light.sun.z, 0.0],
            tiles: table,
        };
        uploads.buffer(encoder, &self.uniform, 0, bytemuck::bytes_of(&uniform));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RT lighting's cloud mesh"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(sky),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("RT lighting's cloud mesh"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        for (i, tile) in tiles.iter().enumerate() {
            pass.set_vertex_buffer(0, tile.vertices.slice(..));
            pass.set_index_buffer(tile.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..tile.index_count, 0, i as u32..i as u32 + 1);
        }
    }
}

fn saturate(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn smoothstep(lo: f32, hi: f32, v: f32) -> f32 {
    let t = saturate((v - lo) / (hi - lo));
    t * t * (3.0 - 2.0 * t)
}

/// Trilinear lookup in a tiling `n`-texel cube of four bytes a texel, at `uvw` in tile units.
fn cube(data: &[u8], n: usize, uvw: Vec3) -> [f32; 4] {
    let p = uvw * n as f32 - 0.5;
    let base = p.floor();
    let f = p - base;
    let b = base.as_ivec3();
    let mut out = [0.0; 4];
    for k in 0..2 {
        for j in 0..2 {
            for i in 0..2 {
                let w = (if i == 1 { f.x } else { 1.0 - f.x })
                    * (if j == 1 { f.y } else { 1.0 - f.y })
                    * (if k == 1 { f.z } else { 1.0 - f.z });
                let at = |v: i32| v.rem_euclid(n as i32) as usize;
                let o = ((at(b.z + k) * n + at(b.y + j)) * n + at(b.x + i)) * 4;
                for (c, slot) in out.iter_mut().enumerate() {
                    *slot += w * f32::from(data[o + c]) / 255.0;
                }
            }
        }
    }
    out
}

/// The weather map's two channels at cloud-space (`x`, `z`), bilinear and tiling.
fn weather(data: &[u8], n: usize, x: f32, z: f32) -> [f32; 2] {
    let p = Vec3::new(x, 0.0, z) / super::rt_clouds::COVERAGE_TILE * n as f32 - 0.5;
    let (bx, bz) = (p.x.floor(), p.z.floor());
    let (fx, fz) = (p.x - bx, p.z - bz);
    let mut out = [0.0; 2];
    for j in 0..2 {
        for i in 0..2 {
            let w = (if i == 1 { fx } else { 1.0 - fx }) * (if j == 1 { fz } else { 1.0 - fz });
            let at = |v: f32, d: i32| (v as i32 + d).rem_euclid(n as i32) as usize;
            let o = (at(bz, j) * n + at(bx, i)) * 2;
            out[0] += w * f32::from(data[o]) / 255.0;
            out[1] += w * f32::from(data[o + 1]) / 255.0;
        }
    }
    out
}

/// The cloud density at cloud-space (`x`, `y`, `z`), as rt_clouds.wgsl `density` without the
/// erosion (the cells are coarser than it): 0 in the clear, rising inside a cloud.
pub fn density(noise: &Noise, x: f64, y: f32, z: f64) -> f32 {
    let wrap = super::rt_clouds::WRAP;
    let (x, z) = (x.rem_euclid(wrap) as f32, z.rem_euclid(wrap) as f32);
    let h = (y - BASE) / (TOP - BASE);
    let w = weather(&noise.weather, super::rt_clouds::WEATHER as usize, x, z);
    let cover = saturate((w[0] - 0.58) / 0.30);
    if cover <= 0.0 {
        return 0.0;
    }
    let top = (0.3 + 0.7 * cover) * (0.8 + 0.2 * w[1]);
    let s = cube(
        &noise.shape,
        super::rt_clouds::SHAPE as usize,
        Vec3::new(x / 4096.0, y / 3072.0, z / 4096.0),
    );
    let fbm = s[1] * 0.625 + s[2] * 0.25 + s[3] * 0.125;
    // The base is not flat: it lifts where the noise is thin.
    let lift = 0.2 * (1.0 - fbm);
    let profile = smoothstep(lift, lift + 0.12, h) * (1.0 - smoothstep(top * 0.55, top, h));
    if profile <= 0.0 {
        return 0.0;
    }
    let base = saturate((s[0] - (fbm - 1.0)) / (2.0 - fbm)) * profile;
    let base = saturate((base - (1.0 - cover * 0.95)) / (cover * 0.95)) * cover;
    if base <= 0.0 {
        return 0.0;
    }
    // The erosion's coarsest octaves (the cells resolve no finer): billows on the edges.
    let d = cube(
        &noise.detail,
        super::rt_clouds::DETAIL as usize,
        Vec3::new(x, y, z) / 512.0,
    );
    let erosion = d[0] * 0.625 + d[1] * 0.375;
    let modifier = erosion + (1.0 - 2.0 * erosion) * saturate(h * 6.0);
    saturate((base - modifier * 0.25) / (1.0 - modifier * 0.25))
}

/// The mesh of tile `key`: vertices relative to the tile's origin (its corner, at the slab's
/// base) and triangles, outward facing.
pub fn mesh_tile(noise: &Noise, key: TileKey) -> (Vec<Vertex>, Vec<u32>) {
    let cell = key.cell as f32;
    let n = [
        (TILE / cell) as i32,
        ((TOP - BASE) / cell).ceil() as i32,
        (TILE / cell) as i32,
    ];
    let origin = (
        f64::from(key.x) * f64::from(TILE),
        f64::from(key.z) * f64::from(TILE),
    );
    let at = |p: Vec3| {
        density(
            noise,
            origin.0 + f64::from(p.x),
            BASE + p.y,
            origin.1 + f64::from(p.z),
        )
    };
    // Samples from one cell before the tile to one after it, so a neighbour's cells meet this
    // tile's exactly: the same samples make the same vertices on both sides.
    let dims = [n[0] + 2, n[1] + 2, n[2] + 2];
    let sample_at = |x: i32, y: i32, z: i32| ((z + 1) * dims[1] + (y + 1)) * dims[0] + (x + 1);
    let mut samples = vec![0.0f32; (dims[0] * dims[1] * dims[2]) as usize];
    for z in -1..=n[2] {
        for y in -1..=n[1] {
            for x in -1..=n[0] {
                // The slab's ends are empty by construction: a closed surface.
                let edge = y < 0 || y >= n[1];
                samples[sample_at(x, y, z) as usize] = if edge {
                    -ISO
                } else {
                    at(Vec3::new(x as f32, y as f32, z as f32) * cell) - ISO
                };
            }
        }
    }
    let cells = [n[0] + 1, n[1] + 1, n[2] + 1];
    let cell_at =
        |x: i32, y: i32, z: i32| (((z + 1) * cells[1] + (y + 1)) * cells[0] + (x + 1)) as usize;
    let mut vertex_of = vec![u32::MAX; (cells[0] * cells[1] * cells[2]) as usize];
    let mut vertices: Vec<Vertex> = Vec::new();
    const CORNERS: [[i32; 3]; 8] = [
        [0, 0, 0],
        [1, 0, 0],
        [0, 1, 0],
        [1, 1, 0],
        [0, 0, 1],
        [1, 0, 1],
        [0, 1, 1],
        [1, 1, 1],
    ];
    const EDGES: [[usize; 2]; 12] = [
        [0, 1],
        [2, 3],
        [4, 5],
        [6, 7],
        [0, 2],
        [1, 3],
        [4, 6],
        [5, 7],
        [0, 4],
        [1, 5],
        [2, 6],
        [3, 7],
    ];
    for z in -1..n[2] {
        for y in -1..n[1] {
            for x in -1..n[0] {
                let value: [f32; 8] = std::array::from_fn(|c| {
                    let o = CORNERS[c];
                    samples[sample_at(x + o[0], y + o[1], z + o[2]) as usize]
                });
                let inside = value.iter().filter(|v| **v > 0.0).count();
                if inside == 0 || inside == 8 {
                    continue;
                }
                let mut sum = Vec3::ZERO;
                let mut crossings = 0.0;
                for [a, b] in EDGES {
                    let (va, vb) = (value[a], value[b]);
                    if (va > 0.0) != (vb > 0.0) {
                        let t = va / (va - vb);
                        let pa = Vec3::from(CORNERS[a].map(|v| v as f32));
                        let pb = Vec3::from(CORNERS[b].map(|v| v as f32));
                        sum += pa + (pb - pa) * t;
                        crossings += 1.0;
                    }
                }
                let local = (Vec3::new(x as f32, y as f32, z as f32) + sum / crossings) * cell;
                // The normal: against the density's gradient, a cell's width around.
                let e = cell * 0.5;
                let gradient = Vec3::new(
                    at(local + Vec3::X * e) - at(local - Vec3::X * e),
                    at(local + Vec3::Y * e) - at(local - Vec3::Y * e),
                    at(local + Vec3::Z * e) - at(local - Vec3::Z * e),
                );
                let normal = (-gradient).normalize_or(Vec3::Y);
                vertex_of[cell_at(x, y, z)] = vertices.len() as u32;
                vertices.push(Vertex {
                    position: local.to_array(),
                    normal: normal.to_array(),
                });
            }
        }
    }
    // A quad for each edge from a sample of this tile that crosses the surface, joining the four
    // cells around it, wound to face away from the inside.
    let mut indices: Vec<u32> = Vec::new();
    for z in 0..n[2] {
        for y in 0..n[1] {
            for x in 0..n[0] {
                let here = samples[sample_at(x, y, z) as usize] > 0.0;
                for axis in 0..3 {
                    let mut next = [x, y, z];
                    next[axis] += 1;
                    if (samples[sample_at(next[0], next[1], next[2]) as usize] > 0.0) == here {
                        continue;
                    }
                    let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
                    let corner = |db: i32, dc: i32| {
                        let mut p = [x, y, z];
                        p[b] += db;
                        p[c] += dc;
                        vertex_of[cell_at(p[0], p[1], p[2])]
                    };
                    let quad = [corner(-1, -1), corner(0, -1), corner(0, 0), corner(-1, 0)];
                    if quad.contains(&u32::MAX) {
                        continue;
                    }
                    if here {
                        indices.extend([quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
                    } else {
                        indices.extend([quad[0], quad[2], quad[1], quad[0], quad[3], quad[2]]);
                    }
                }
            }
        }
    }
    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_is_a_closed_outward_surface() {
        let noise = Noise::shared();
        // Find a tile with clouds in it.
        let (key, vertices, indices) = (-4..4)
            .flat_map(|x| (-4..4).map(move |z| TileKey { x, z, cell: 32 }))
            .map(|key| {
                let (v, i) = mesh_tile(&noise, key);
                (key, v, i)
            })
            .find(|(_, _, i)| i.len() > 600)
            .expect("some cloud in 64 tiles");
        assert_eq!(indices.len() % 3, 0);
        assert!(indices.iter().all(|i| (*i as usize) < vertices.len()));
        // The winding agrees with the normals: a triangle's normal points the way its
        // vertices' do, for nearly all of them.
        let agreeing = indices
            .chunks(3)
            .filter(|t| {
                let p = |i: u32| Vec3::from(vertices[i as usize].position);
                let n = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
                let avg = t
                    .iter()
                    .map(|i| Vec3::from(vertices[*i as usize].normal))
                    .sum::<Vec3>();
                n.dot(avg) >= 0.0
            })
            .count();
        let share = agreeing as f32 / (indices.len() / 3) as f32;
        assert!(share > 0.9, "{key:?}: {share}");
        // A tile stays within its footprint, a cell beyond it at most.
        assert!(
            vertices
                .iter()
                .all(|v| v.position.iter().all(|c| *c >= -64.0 && *c <= TILE + 64.0))
        );
    }

    /// The whole ring of tiles around a camera: their triangles and the time they take (run with
    /// `--ignored --nocapture`).
    #[test]
    #[ignore = "a measurement"]
    fn measure_the_ring() {
        let noise = Noise::shared();
        let started = std::time::Instant::now();
        let (mut triangles, mut vertices) = (0, 0);
        for x in -RADIUS..=RADIUS {
            for z in -RADIUS..=RADIUS {
                let d = x.abs().max(z.abs());
                let (v, i) = mesh_tile(
                    &noise,
                    TileKey {
                        x,
                        z,
                        cell: cell_for(d),
                    },
                );
                triangles += i.len() / 3;
                vertices += v.len();
            }
        }
        println!(
            "{triangles} triangles, {vertices} vertices, {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn neighbours_share_their_seam() {
        let noise = Noise::shared();
        let a = mesh_tile(
            &noise,
            TileKey {
                x: 0,
                z: 0,
                cell: 64,
            },
        );
        let b = mesh_tile(
            &noise,
            TileKey {
                x: 1,
                z: 0,
                cell: 64,
            },
        );
        // Every vertex of a one on the seam is also in the other, at the same world place.
        let seam = |v: &Vertex, shift: f32| (v.position[0] + shift - TILE).abs() < 1e-3;
        let in_a: Vec<_> = a.0.iter().filter(|v| v.position[0] > TILE - 1.0).collect();
        for v in in_a.iter().filter(|v| seam(v, 0.0)) {
            assert!(b.0.iter().any(|w| {
                (w.position[0] + TILE - v.position[0]).abs() < 1e-3
                    && (w.position[1] - v.position[1]).abs() < 1e-3
                    && (w.position[2] - v.position[2]).abs() < 1e-3
            }));
        }
    }
}
