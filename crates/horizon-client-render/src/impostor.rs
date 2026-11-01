//! Impostors: far entities drawn as one camera-facing picture each, for the mobs and players
//! kept far past the entity render distance. A model is baked
//! from 8 views around it by 4 strips of camera elevation into 64 px tiles of a page-array
//! atlas (alpha cut, edge bleed, mips), tiles are kept by least recent use in a budget of MB, and
//! a quad per entity picks its tile with hysteresis (a view is kept
//! while the camera is within 30.5 degrees of it).
//!
//! The baking is a small software rasteriser of the model's own quads (`mesher::Vertex`,
//! as the entity pipeline draws them) over the CPU copy of the entity atlas: no render pass,
//! no GPU state, and the cost is the game's to budget per frame. The GPU side is
//! `gfx/impostors.rs`; everything here is plain data and tests.
use crate::{fast_hash::FxHashMap, mesher::Vertex as Quad};
use std::hash::Hash;

/// A tile's side in pixels, and the side a model is rasterised at before it is halved into one
/// (so its edges are antialiased).
pub const TILE: usize = 64;
const BAKE: usize = 2 * TILE;
/// The share of a tile a model's bounding sphere fills: 4 px of clear border on each side, so
/// the tile's mips and bilinear taps never reach the next tile.
const INNER: f32 = 56.0;
/// Levels of a tile's mip chain (64, 32, 16, 8 px).
pub const MIPS: usize = 4;
/// Bytes of one tile with its mips.
pub const TILE_BYTES: usize = {
    let mut bytes = 0;
    let mut m = 0;
    while m < MIPS {
        bytes += (TILE >> m) * (TILE >> m) * 4;
        m += 1;
    }
    bytes
};
/// Views around a model, 45 degrees apart, and strips of camera elevation (degrees above the
/// horizon; the camera looks down at a model below it).
pub const VIEWS: u8 = 8;
pub const STRIPS: u8 = 4;
pub const STRIP_ELEVATIONS: [f32; STRIPS as usize] = [-30.0, 0.0, 30.0, 60.0];
/// A view is kept while the camera is within this many degrees of it, rather than the nearest
/// one's 22.5; a strip while within this of its elevation.
const VIEW_HOLD: f32 = 30.5;
const STRIP_HOLD: f32 = 20.0;
/// Tiles along a page's side, a page's side in pixels, and its bytes with the mips of every tile.
pub const PAGE_TILES: u32 = 16;
pub const PAGE: u32 = PAGE_TILES * TILE as u32;
pub const PAGE_BYTES: u64 = {
    let mut bytes = 0u64;
    let mut m = 0;
    while m < MIPS {
        bytes += ((PAGE >> m) as u64) * ((PAGE >> m) as u64) * 4;
        m += 1;
    }
    bytes
};
/// The atlas's budget in MB, its pages and its tiles.
pub const BUDGET_MB: u64 = 32;
pub const PAGES: u32 = (BUDGET_MB * 1024 * 1024 / PAGE_BYTES) as u32;
pub const SLOTS: u32 = PAGES * PAGE_TILES * PAGE_TILES;

/// Where a slot is: its page, and its tile's column and row in it.
pub fn slot_origin(slot: u32) -> (u32, u32, u32) {
    let per_page = PAGE_TILES * PAGE_TILES;
    let i = slot % per_page;
    (slot / per_page, i % PAGE_TILES, i / PAGE_TILES)
}

/// The tiles in the atlas, by key, least recently used out first. A slot is handed out for a
/// tile about to be baked; tiles used this frame are never taken for another.
pub struct Slots<K> {
    map: FxHashMap<K, (u32, u64)>,
    free: Vec<u32>,
    clock: u64,
}

impl<K: Eq + Hash + Clone> Slots<K> {
    pub fn new(capacity: u32) -> Self {
        Self {
            map: FxHashMap::default(),
            free: (0..capacity).rev().collect(),
            clock: 1,
        }
    }
    /// A new frame: what it uses is safe from eviction until the next.
    pub fn begin_frame(&mut self) {
        self.clock += 1;
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    /// The tile's slot, used now.
    pub fn get(&mut self, key: &K) -> Option<u32> {
        let entry = self.map.get_mut(key)?;
        entry.1 = self.clock;
        Some(entry.0)
    }
    /// A slot for the tile `key`: a free one, or the least recently used tile's from before this
    /// frame. None when every tile was used this frame.
    pub fn insert(&mut self, key: K) -> Option<u32> {
        if let Some(entry) = self.map.get_mut(&key) {
            entry.1 = self.clock;
            return Some(entry.0);
        }
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => {
                let (victim, slot) = self
                    .map
                    .iter()
                    .filter(|(_, (_, used))| *used < self.clock)
                    .min_by_key(|(_, (_, used))| *used)
                    .map(|(key, (slot, _))| (key.clone(), *slot))?;
                self.map.remove(&victim);
                slot
            }
        };
        self.map.insert(key, (slot, self.clock));
        Some(slot)
    }
}

fn wrap(degrees: f32) -> f32 {
    let mut a = degrees.rem_euclid(360.0);
    if a >= 180.0 {
        a -= 360.0;
    }
    a
}

/// The view that shows a model from the camera at `azimuth` degrees around it (0 is the side it
/// faces, the way a baked model faces), keeping `previous` while the camera is still near it.
pub fn pick_view(azimuth: f32, previous: Option<u8>) -> u8 {
    if let Some(p) = previous
        && wrap(azimuth - f32::from(p) * 45.0).abs() <= VIEW_HOLD
    {
        return p;
    }
    ((azimuth.rem_euclid(360.0) / 45.0).round() as i32 % i32::from(VIEWS)) as u8
}

/// The strip that shows a model from a camera `elevation` degrees above it, kept likewise.
pub fn pick_strip(elevation: f32, previous: Option<u8>) -> u8 {
    if let Some(p) = previous
        && (elevation - STRIP_ELEVATIONS[usize::from(p)]).abs() <= STRIP_HOLD
    {
        return p;
    }
    let mut best = 0;
    for (i, e) in STRIP_ELEVATIONS.iter().enumerate() {
        if (elevation - e).abs() < (elevation - STRIP_ELEVATIONS[best]).abs() {
            best = i;
        }
    }
    best as u8
}

/// Past this many blocks, or under this many pixels tall, a model is drawn as an impostor; it
/// is a model again below 0.95 of either.
pub const MODEL_DISTANCE: f64 = 128.0;
pub const MIN_PIXELS: f32 = 10.0;
const SWITCH_HOLD: f64 = 0.95;

/// Whether an entity `distance` blocks away and `pixels` tall on the screen is drawn as an
/// impostor, given whether it was last frame.
pub fn use_impostor(distance: f64, pixels: f32, was_impostor: bool) -> bool {
    if was_impostor {
        distance > MODEL_DISTANCE * SWITCH_HOLD
            || f64::from(pixels) < f64::from(MIN_PIXELS) / SWITCH_HOLD
    } else {
        distance > MODEL_DISTANCE || pixels < MIN_PIXELS
    }
}

/// A model's size, from its geometry: the height of the centre of the sphere around its vertical axis
/// that holds it all, and that sphere's radius. The tile fits it, so a tile's picture has the same
/// scale from every view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub center_y: f32,
    pub radius: f32,
}

/// Whether the quads are ones an impostor shows: the ordinary model's, not shadows, cracks,
/// glint overlays and the like.
fn shown(v: &Quad) -> bool {
    v.layer & 0xffff == 0
}

/// The fit of a model's quads (`mesher::Vertex`, four to a quad, relative to the model's origin).
pub fn fit_of(quads: &[Quad]) -> Option<Fit> {
    let vertices = || quads.iter().filter(|v| shown(v));
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for v in vertices() {
        lo = lo.min(v.pos[1]);
        hi = hi.max(v.pos[1]);
    }
    if lo > hi {
        return None;
    }
    let center_y = (lo + hi) / 2.0;
    let radius = vertices()
        .map(|v| {
            let p = v.pos;
            (p[0] * p[0] + p[2] * p[2] + (p[1] - center_y) * (p[1] - center_y)).sqrt()
        })
        .fold(0.0f32, f32::max)
        .max(0.2);
    Some(Fit { center_y, radius })
}

/// The entity atlas's pixels (RGBA8, row by row), as a model's UVs index them.
#[derive(Clone, Copy)]
pub struct AtlasPixels<'a> {
    pub pixels: &'a [u8],
    pub width: u32,
    pub height: u32,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A baked view's camera: its right and up (screen) axes and the direction to it, for a camera
/// `azimuth` degrees around a model facing +z (measured from +z toward +x) and `elevation`
/// above it.
fn basis(azimuth: f32, elevation: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    let to_camera = [e.cos() * a.sin(), e.sin(), e.cos() * a.cos()];
    let flat = (to_camera[0] * to_camera[0] + to_camera[2] * to_camera[2])
        .sqrt()
        .max(1e-6);
    // The camera looks along `-to_camera`; its right is that crossed with up.
    let right = [to_camera[2] / flat, 0.0, -to_camera[0] / flat];
    let f = [-to_camera[0], -to_camera[1], -to_camera[2]];
    let up = [
        right[1] * f[2] - right[2] * f[1],
        right[2] * f[0] - right[0] * f[2],
        right[0] * f[1] - right[1] * f[0],
    ];
    (right, up, to_camera)
}

/// `entity.wgsl`'s `entity_light`: the shade of a face by its normal (the game's cardinal light).
fn entity_light(n: [f32; 3]) -> f32 {
    fn unit(v: [f32; 3]) -> [f32; 3] {
        let l = dot(v, v).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    }
    let l0 = unit([0.2, 1.0, -0.7]);
    let l1 = unit([-0.2, 1.0, 0.7]);
    ((dot(l0, n).max(0.0) + dot(l1, n).max(0.0)) * 0.6 + 0.4).min(1.0)
}

fn tint(color: u32) -> [f32; 3] {
    [
        (color & 255) as f32 / 255.0,
        (color >> 8 & 255) as f32 / 255.0,
        (color >> 16 & 255) as f32 / 255.0,
    ]
}

/// Bakes a model's view into a tile with its mips (`TILE_BYTES`, RGBA8, the mips one after another):
/// `quads` as the entity pipeline draws them (camera at the model's origin, four vertices to a
/// quad, facing +z), seen from `azimuth` and `elevation` (degrees, as `basis` has them) and
/// fitted by `fit`. Texels under alpha 26 are cut; the clear pixels next to a model take its
/// colours, so a bilinear tap at an edge doesn't darken it.
pub fn bake(quads: &[Quad], atlas: AtlasPixels, azimuth: f32, elevation: f32, fit: Fit) -> Vec<u8> {
    let (right, up, to_camera) = basis(azimuth, elevation);
    let scale = BAKE as f32 * (INNER / TILE as f32) / (2.0 * fit.radius);
    let half = BAKE as f32 / 2.0;
    // Screen x, screen y (up is positive), and nearness, of a model point.
    let project = |p: [f32; 3]| {
        let q = [p[0], p[1] - fit.center_y, p[2]];
        (
            half + dot(q, right) * scale,
            half + dot(q, up) * scale,
            dot(q, to_camera),
        )
    };
    let mut color = vec![[0.0f32; 3]; BAKE * BAKE];
    let mut depth = vec![f32::MIN; BAKE * BAKE];
    for quad in quads.as_chunks::<4>().0 {
        if !shown(&quad[0]) {
            continue;
        }
        let corners = quad.map(|v| project(v.pos));
        let tints = quad.map(|v| tint(v.color));
        // The face's shade: lit by its normal, as the shader does for a lit quad, or kept as baked.
        let [nx, ny, nz, w] = quad[0].normal;
        let normal = [
            f32::from(nx) / 127.0,
            f32::from(ny) / 127.0,
            f32::from(nz) / 127.0,
        ];
        let two_sided = w.unsigned_abs() >= 64;
        // A model that culls its back faces (entitySolid).
        let culls = w == 1;
        for triangle in [[0, 1, 2], [0, 2, 3]] {
            let [a, b, c] = triangle.map(|i| corners[i]);
            let edge = |p: (f32, f32, f32), q: (f32, f32, f32), x: f32, y: f32| {
                (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0)
            };
            let area = edge(a, b, c.0, c.1);
            if area.abs() < 1e-6 || (culls && area <= 0.0) {
                continue;
            }
            let front = area > 0.0;
            let shade = if two_sided {
                let facing = if front || w > 0 {
                    normal
                } else {
                    normal.map(|v| -v)
                };
                entity_light(facing)
            } else {
                1.0
            };
            let lo =
                |f: fn(f32, f32) -> f32, g: fn(&(f32, f32, f32)) -> f32| f(f(g(&a), g(&b)), g(&c));
            let x0 = lo(f32::min, |p| p.0).floor().max(0.0) as usize;
            let y0 = lo(f32::min, |p| p.1).floor().max(0.0) as usize;
            let x1 = (lo(f32::max, |p| p.0).ceil().max(0.0) as usize).min(BAKE - 1);
            let y1 = (lo(f32::max, |p| p.1).ceil().max(0.0) as usize).min(BAKE - 1);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    // Barycentric weights; a pixel just outside an edge still counts, so
                    // neighbouring triangles leave no cracks.
                    let l = [
                        edge(b, c, px, py) / area,
                        edge(c, a, px, py) / area,
                        edge(a, b, px, py) / area,
                    ];
                    if l.iter().any(|&w| w < -1e-4) {
                        continue;
                    }
                    let near = l[0] * a.2 + l[1] * b.2 + l[2] * c.2;
                    // The screen's y runs up here; the image's rows run down.
                    let row = BAKE - 1 - y;
                    let at = row * BAKE + x;
                    if near <= depth[at] {
                        continue;
                    }
                    let ids = triangle;
                    let mix = |f: &dyn Fn(usize) -> f32| {
                        l[0] * f(ids[0]) + l[1] * f(ids[1]) + l[2] * f(ids[2])
                    };
                    let u = mix(&|i| quad[i].uv[0]);
                    let v = mix(&|i| quad[i].uv[1]);
                    let tx =
                        ((u * atlas.width as f32).floor().max(0.0) as u32).min(atlas.width - 1);
                    let ty =
                        ((v * atlas.height as f32).floor().max(0.0) as u32).min(atlas.height - 1);
                    let i = (ty as usize * atlas.width as usize + tx as usize) * 4;
                    let Some(texel) = atlas.pixels.get(i..i + 4) else {
                        continue;
                    };
                    if texel[3] < 26 {
                        continue;
                    }
                    depth[at] = near;
                    color[at] = std::array::from_fn(|k| {
                        let t = mix(&|i| tints[i][k]);
                        (f32::from(texel[k]) / 255.0 * t * shade).clamp(0.0, 1.0)
                    });
                }
            }
        }
    }
    finish(&color, &depth)
}

/// The tile from the model rasterised at twice its size: halved with the cut and the bleed, then its mips.
fn finish(color: &[[f32; 3]], depth: &[f32]) -> Vec<u8> {
    let mut base = vec![[0u8; 4]; TILE * TILE];
    for y in 0..TILE {
        for x in 0..TILE {
            let (mut sum, mut n) = ([0.0f32; 3], 0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let i = (2 * y + dy) * BAKE + 2 * x + dx;
                if depth[i] > f32::MIN {
                    n += 1;
                    for k in 0..3 {
                        sum[k] += color[i][k];
                    }
                }
            }
            // Alpha is cut at 26 of 255: a quarter of a pixel's samples keeps it whole.
            if n > 0 {
                let rgb = sum.map(|s| (s / n as f32 * 255.0).round() as u8);
                base[y * TILE + x] = [rgb[0], rgb[1], rgb[2], 255];
            }
        }
    }
    bleed(&mut base);
    let mut out = Vec::with_capacity(TILE_BYTES);
    let mut level = base;
    let mut size = TILE;
    loop {
        out.extend(level.iter().flatten());
        if size == TILE >> (MIPS - 1) {
            break;
        }
        level = halve(&level, size);
        size /= 2;
    }
    out
}

/// The clear pixels within two of the model take the average colour of the model's pixels
/// around them (alpha stays clear).
fn bleed(tile: &mut [[u8; 4]]) {
    let mut filled: Vec<bool> = tile.iter().map(|p| p[3] > 0).collect();
    for _ in 0..2 {
        let mut next = filled.clone();
        for y in 0..TILE {
            for x in 0..TILE {
                if filled[y * TILE + x] {
                    continue;
                }
                let (mut sum, mut n) = ([0u32; 3], 0);
                for (dx, dy) in [
                    (-1i32, -1i32),
                    (0, -1),
                    (1, -1),
                    (-1, 0),
                    (1, 0),
                    (-1, 1),
                    (0, 1),
                    (1, 1),
                ] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if !(0..TILE as i32).contains(&nx) || !(0..TILE as i32).contains(&ny) {
                        continue;
                    }
                    let j = ny as usize * TILE + nx as usize;
                    if filled[j] {
                        n += 1;
                        for k in 0..3 {
                            sum[k] += u32::from(tile[j][k]);
                        }
                    }
                }
                if n > 0 {
                    let i = y * TILE + x;
                    tile[i] = [
                        (sum[0] / n) as u8,
                        (sum[1] / n) as u8,
                        (sum[2] / n) as u8,
                        0,
                    ];
                    next[i] = true;
                }
            }
        }
        filled = next;
    }
}

/// The next mip: 2x2 pixels to one, the colour weighted by alpha.
fn halve(tile: &[[u8; 4]], size: usize) -> Vec<[u8; 4]> {
    let half = size / 2;
    let mut out = vec![[0u8; 4]; half * half];
    for y in 0..half {
        for x in 0..half {
            let (mut rgb, mut weight, mut alpha, mut plain) = ([0u32; 3], 0u32, 0u32, [0u32; 3]);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = tile[(2 * y + dy) * size + 2 * x + dx];
                alpha += u32::from(p[3]);
                weight += u32::from(p[3]);
                for k in 0..3 {
                    rgb[k] += u32::from(p[k]) * u32::from(p[3]);
                    plain[k] += u32::from(p[k]);
                }
            }
            out[y * half + x] = if weight > 0 {
                [
                    (rgb[0] / weight) as u8,
                    (rgb[1] / weight) as u8,
                    (rgb[2] / weight) as u8,
                    (alpha / 4) as u8,
                ]
            } else {
                [
                    (plain[0] / 4) as u8,
                    (plain[1] / 4) as u8,
                    (plain[2] / 4) as u8,
                    0,
                ]
            };
        }
    }
    out
}

/// One tile's pixels on their way to the atlas: its slot and its mips (`TILE_BYTES`).
#[derive(Clone, Debug)]
pub struct TileUpload {
    pub slot: u32,
    pub pixels: Vec<u8>,
}

/// One corner of an impostor's quad, camera-relative (see `gfx/impostors.rs`): its place, its UV
/// in its page and the page.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub page: u32,
}

/// The quad of an impostor: `center` is where the model's fit centre is, camera-relative, the
/// camera's `right` and `up` axes face it to the camera, and `radius` is the fit's.
pub fn quad(
    center: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    radius: f32,
    slot: u32,
) -> [Vertex; 4] {
    // The whole tile spans its picture's sphere plus the clear border.
    let h = radius * TILE as f32 / INNER;
    let (page, tx, ty) = slot_origin(slot);
    let at = |a: f32, b: f32| -> [f32; 3] {
        std::array::from_fn(|i| center[i] + right[i] * h * a + up[i] * h * b)
    };
    let (u0, u1) = (
        (tx * TILE as u32) as f32 / PAGE as f32,
        ((tx + 1) * TILE as u32) as f32 / PAGE as f32,
    );
    let (v0, v1) = (
        (ty * TILE as u32) as f32 / PAGE as f32,
        ((ty + 1) * TILE as u32) as f32 / PAGE as f32,
    );
    [
        Vertex {
            pos: at(-1.0, 1.0),
            uv: [u0, v0],
            page,
        },
        Vertex {
            pos: at(1.0, 1.0),
            uv: [u1, v0],
            page,
        },
        Vertex {
            pos: at(1.0, -1.0),
            uv: [u1, v1],
            page,
        },
        Vertex {
            pos: at(-1.0, -1.0),
            uv: [u0, v1],
            page,
        },
    ]
}

/// What the renderer draws of impostors in a frame: the tiles baked since the last one, and the quads.
#[derive(Clone, Copy, Default)]
pub struct Draw<'a> {
    pub uploads: &'a [TileUpload],
    pub vertices: &'a [Vertex],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_atlas_holds_whole_pages_in_its_budget() {
        assert_eq!(TILE_BYTES, 21760);
        assert_eq!(PAGE_BYTES, 5_570_560);
        assert!(u64::from(PAGES) * PAGE_BYTES <= BUDGET_MB * 1024 * 1024);
        assert!(PAGES >= 1);
        assert_eq!(slot_origin(0), (0, 0, 0));
        assert_eq!(slot_origin(17), (0, 1, 1));
        assert_eq!(slot_origin(256), (1, 0, 0));
    }

    #[test]
    fn the_least_recently_used_tile_goes_first_but_never_one_used_this_frame() {
        let mut slots = Slots::new(2);
        assert_eq!(slots.insert('a'), Some(0));
        slots.begin_frame();
        assert_eq!(slots.insert('b'), Some(1));
        // Both used this frame: nothing to take.
        assert_eq!(slots.get(&'a'), Some(0));
        assert_eq!(slots.insert('c'), None);
        // Later frames use only `b`: `a` is the least recently used and goes.
        slots.begin_frame();
        assert_eq!(slots.get(&'b'), Some(1));
        slots.begin_frame();
        assert_eq!(slots.get(&'b'), Some(1));
        assert_eq!(slots.insert('c'), Some(0));
        assert_eq!(slots.get(&'a'), None);
        assert_eq!(slots.len(), 2);
    }

    #[test]
    fn a_view_is_kept_while_the_camera_is_near_it() {
        assert_eq!(pick_view(0.0, None), 0);
        assert_eq!(pick_view(50.0, None), 1);
        // 25 degrees past view 0 would be view 1 afresh, but 0 is kept up to 30.5.
        assert_eq!(pick_view(25.0, Some(0)), 0);
        assert_eq!(pick_view(31.0, Some(0)), 1);
        // Around the circle.
        assert_eq!(pick_view(359.0, None), 0);
        assert_eq!(pick_view(-20.0, Some(0)), 0);
        assert_eq!(pick_view(-40.0, Some(0)), 7);
        assert_eq!(pick_view(340.0, Some(7)), 7);
    }

    #[test]
    fn a_strip_is_kept_with_a_margin() {
        assert_eq!(pick_strip(0.0, None), 1);
        assert_eq!(pick_strip(80.0, None), 3);
        assert_eq!(pick_strip(-80.0, None), 0);
        assert_eq!(pick_strip(16.0, None), 2);
        assert_eq!(pick_strip(16.0, Some(1)), 1, "strip 1 holds to 20 degrees");
        assert_eq!(pick_strip(25.0, Some(1)), 2);
    }

    #[test]
    fn the_switch_has_a_margin_both_ways() {
        // A model goes to an impostor past 128 blocks, and comes back under 121.6.
        assert!(!use_impostor(127.0, 50.0, false));
        assert!(use_impostor(129.0, 50.0, false));
        assert!(use_impostor(125.0, 50.0, true));
        assert!(!use_impostor(120.0, 50.0, true));
        // And by size: under 10 px, back above 10.5.
        assert!(use_impostor(50.0, 9.0, false));
        assert!(use_impostor(50.0, 10.2, true));
        assert!(!use_impostor(50.0, 10.6, true));
    }

    fn quad_at(z: f32, w: f32, color: u32) -> [Quad; 4] {
        let vertex = |x: f32, y: f32, uv: [f32; 2]| Quad {
            pos: [x, y, z],
            uv,
            layer: 0,
            color,
            light: 0,
            normal: [0, 0, 0, 0],
            glint_uv: [0.0; 2],
        };
        [
            vertex(-w, 2.0, [0.1, 0.1]),
            vertex(w, 2.0, [0.4, 0.1]),
            vertex(w, 0.0, [0.4, 0.4]),
            vertex(-w, 0.0, [0.1, 0.4]),
        ]
    }

    /// An atlas of 10x10 px that is red in its top left and clear elsewhere.
    fn atlas() -> (Vec<u8>, u32) {
        let mut pixels = vec![0u8; 10 * 10 * 4];
        for y in 0..5 {
            for x in 0..5 {
                let i = (y * 10 + x) * 4;
                pixels[i..i + 4].copy_from_slice(&[200, 0, 0, 255]);
            }
        }
        (pixels, 10)
    }

    fn pixel(tile: &[u8], x: usize, y: usize) -> [u8; 4] {
        let i = (y * TILE + x) * 4;
        tile[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn a_quad_bakes_into_the_middle_of_a_tile() {
        let (pixels, size) = atlas();
        let atlas = AtlasPixels {
            pixels: &pixels,
            width: size,
            height: size,
        };
        let quads = quad_at(0.0, 0.5, u32::MAX);
        let fit = fit_of(&quads).unwrap();
        assert_eq!(fit.center_y, 1.0);
        let tile = bake(&quads, atlas, 0.0, 0.0, fit);
        assert_eq!(tile.len(), TILE_BYTES);
        // The picture is red in the middle, clear in the border of 4 px and the corners.
        let middle = pixel(&tile, 32, 32);
        assert!(
            middle[0] > 150 && middle[1] < 5 && middle[3] == 255,
            "{middle:?}"
        );
        for (x, y) in [(0, 0), (63, 63), (2, 32), (32, 2), (61, 32)] {
            assert_eq!(pixel(&tile, x, y)[3], 0, "{x},{y}");
        }
        // The clear pixels beside it have its colour (bleed), under bilinear taps.
        let beside = pixel(&tile, 8, 32);
        assert_eq!(beside[3], 0);
    }

    #[test]
    fn a_nearer_quad_hides_a_farther_one_and_a_view_from_behind_swaps_them() {
        let (pixels, size) = atlas();
        let atlas = AtlasPixels {
            pixels: &pixels,
            width: size,
            height: size,
        };
        let mut quads = Vec::new();
        quads.extend(quad_at(0.5, 0.5, u32::MAX));
        // A green-tinted quad behind the red one (nearer to a camera at +z is greater z).
        quads.extend(quad_at(-0.5, 0.5, 0x0000_ff00));
        let fit = fit_of(&quads).unwrap();
        let front = bake(&quads, atlas, 0.0, 0.0, fit);
        let behind = bake(&quads, atlas, 180.0, 0.0, fit);
        let a = pixel(&front, 32, 32);
        let b = pixel(&behind, 32, 32);
        // The plain quad shows its red texel; from behind the green-tinted one is nearer, and
        // its tint zeroes the red texel.
        assert!(a[0] > 150, "{a:?}");
        assert!(b[0] < 5 && b[3] == 255, "{b:?}");
    }

    #[test]
    fn mips_follow_the_base_and_keep_a_clear_tile_clear() {
        let tile = finish(&vec![[0.0; 3]; BAKE * BAKE], &vec![f32::MIN; BAKE * BAKE]);
        assert_eq!(tile.len(), TILE_BYTES);
        assert!(tile.chunks(4).all(|p| p[3] == 0));
        let mut solid = vec![[1.0f32; 3]; BAKE * BAKE];
        solid.iter_mut().for_each(|c| *c = [0.5, 0.25, 1.0]);
        let tile = finish(&solid, &vec![1.0; BAKE * BAKE]);
        // Every mip level is the same flat colour.
        let mut offset = 0;
        for m in 0..MIPS {
            let size = TILE >> m;
            let p = &tile[offset..offset + 4];
            assert_eq!(p, [128, 64, 255, 255], "mip {m}");
            offset += size * size * 4;
        }
    }

    #[test]
    fn a_quad_spans_its_tile_in_the_page() {
        let v = quad([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.56, 17);
        // Slot 17 is the tile at column 1, row 1 of page 0; the picture is as wide as its sphere
        // plus the border: 2 * 0.56 * 64 / 56 across.
        assert_eq!(v[0].page, 0);
        assert!((v[1].pos[0] - v[0].pos[0] - 1.28).abs() < 1e-5);
        assert_eq!(v[0].uv, [64.0 / 1024.0, 64.0 / 1024.0]);
        assert_eq!(v[2].uv, [128.0 / 1024.0, 128.0 / 1024.0]);
        // Top left is the picture's top row.
        assert!(v[0].pos[1] > v[3].pos[1]);
    }

    #[test]
    fn a_view_looks_from_the_side_it_names() {
        // Azimuth 0 looks at the model's +z face from +z: right is +x.
        let (right, up, to) = basis(0.0, 0.0);
        assert!(
            (right[0] - 1.0).abs() < 1e-6
                && (up[1] - 1.0).abs() < 1e-6
                && (to[2] - 1.0).abs() < 1e-6
        );
        // Azimuth 90 is on the +x side: its right is -z.
        let (right, _, to) = basis(90.0, 0.0);
        assert!((to[0] - 1.0).abs() < 1e-6 && (right[2] + 1.0).abs() < 1e-6);
        // Looking down from above tilts the up axis away from the camera.
        let (_, up, _) = basis(0.0, 30.0);
        assert!(up[2] < 0.0 && up[1] > 0.0);
    }
}
