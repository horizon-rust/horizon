//! Opacity micromaps for the cutout quads (docs/rtx-4090-handoff.md, "Next" 3; DXR 1.2's and
//! `VK_EXT_opacity_micromap`'s opacity micromaps): what each micro-triangle of a triangle shows of
//! its sprite, worked out once on the CPU, so the hardware itself skips the transparent texels of
//! leaves, plants and iron bars and confirms the opaque ones, and runs the alpha test (any-hit, or
//! a ray query's candidate) only where a micro-triangle straddles both.
//!
//! A triangle's micromap at subdivision level `L` splits it into `4^L` micro-triangles along its
//! barycentrics (u the weight of its second vertex, v of its third, as the hit's), in the order of
//! the specification's space-filling curve (`curve_index`, its reference code), each a state in
//! two bits (4-state format): transparent, opaque, or unknown (the shader decides). A state is
//! known only where every texel the micro-triangle overlaps agrees (alpha at least half: opaque,
//! as the kernel's `hw_opaque` tests it), so the picture is the any-hit's exactly.
//!
//! Pure CPU work, tested here; the builds are in the raw section structures.

/// The specification's states (`VkOpacityMicromapStateEXT`).
pub const TRANSPARENT: u8 = 0;
pub const OPAQUE: u8 = 1;
pub const UNKNOWN_TRANSPARENT: u8 = 2;
pub const UNKNOWN_OPAQUE: u8 = 3;

/// The finest level made: `4^8` micro-triangles, 16 KB a triangle in four states.
pub const MAX_LEVEL: u32 = 8;

/// A sprite's alpha test as bits, `width` x `height` texels, row-major from the top: set where
/// the texel is opaque (alpha at least 128 of 255, `hw_opaque`'s `texel.a >= 0.5`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlphaMask {
    pub width: u32,
    pub height: u32,
    bits: Vec<u64>,
}

impl AlphaMask {
    /// From RGBA8 pixels (`width * height * 4` bytes).
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Self {
        let n = (width * height) as usize;
        let mut bits = vec![0u64; n.div_ceil(64)];
        for (i, p) in rgba.chunks_exact(4).take(n).enumerate() {
            if p[3] >= 128 {
                bits[i / 64] |= 1 << (i % 64);
            }
        }
        Self {
            width,
            height,
            bits,
        }
    }

    /// From a test over each texel.
    pub fn from_fn(width: u32, height: u32, opaque: impl Fn(u32, u32) -> bool) -> Self {
        let mut bits = vec![0u64; ((width * height) as usize).div_ceil(64)];
        for y in 0..height {
            for x in 0..width {
                if opaque(x, y) {
                    let i = (y * width + x) as usize;
                    bits[i / 64] |= 1 << (i % 64);
                }
            }
        }
        Self {
            width,
            height,
            bits,
        }
    }

    pub fn opaque(&self, x: u32, y: u32) -> bool {
        let i = (y * self.width + x) as usize;
        self.bits[i / 64] >> (i % 64) & 1 == 1
    }

    /// The texel the shader reads at `uv` (`clamp(floor(uv * size))`).
    pub fn texel_at(&self, uv: [f32; 2]) -> (u32, u32) {
        let x = (uv[0] * self.width as f32).floor() as i64;
        let y = (uv[1] * self.height as f32).floor() as i64;
        (
            x.clamp(0, i64::from(self.width) - 1) as u32,
            y.clamp(0, i64::from(self.height) - 1) as u32,
        )
    }
}

/// The specification's reference `BarycentricsToSpaceFillingCurveIndex`: the micro-triangle of a
/// level-`level` micromap holding barycentrics `(u, v)`.
pub fn curve_index(u: f32, v: f32, level: u32) -> u32 {
    let u = u.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let side = 1u32 << level;
    let fu = u * side as f32;
    let fv = v * side as f32;
    let mut iu = fu as u32;
    let mut iv = fv as u32;
    let uf = fu - iu as f32;
    let vf = fv - iv as f32;
    if iu >= side {
        iu = side - 1;
    }
    if iv >= side {
        iv = side - 1;
    }
    let iuv = iu + iv;
    if iuv >= side {
        iu -= iuv - side + 1;
    }
    let mut iw = !(iu.wrapping_add(iv));
    if uf + vf >= 1.0 && iuv < side - 1 {
        iw = iw.wrapping_sub(1);
    }
    let mut b0 = !(iu ^ iw);
    b0 &= side - 1;
    let t = (iu ^ iv) & b0;
    let mut f = t;
    f ^= f >> 1;
    f ^= f >> 2;
    f ^= f >> 4;
    f ^= f >> 8;
    let mut b1 = ((f ^ iu) & !b0) | t;
    b0 = (b0 | (b0 << 8)) & 0x00ff_00ff;
    b0 = (b0 | (b0 << 4)) & 0x0f0f_0f0f;
    b0 = (b0 | (b0 << 2)) & 0x3333_3333;
    b0 = (b0 | (b0 << 1)) & 0x5555_5555;
    b1 = (b1 | (b1 << 8)) & 0x00ff_00ff;
    b1 = (b1 | (b1 << 4)) & 0x0f0f_0f0f;
    b1 = (b1 | (b1 << 2)) & 0x3333_3333;
    b1 = (b1 | (b1 << 1)) & 0x5555_5555;
    b0 | (b1 << 1)
}

/// The micro-triangles of level `level`, each as its three corners' barycentrics `(u, v)`: the
/// upright ones of the uniform grid and the inverted ones between them, in no particular order
/// (`curve_index` of the centroid places each).
pub fn micro_triangles(level: u32) -> Vec<[[f32; 2]; 3]> {
    let n = 1u32 << level;
    let s = 1.0 / n as f32;
    let mut out = Vec::with_capacity((n * n) as usize);
    for i in 0..n {
        for j in 0..n - i {
            let (u, v) = (i as f32 * s, j as f32 * s);
            out.push([[u, v], [u + s, v], [u, v + s]]);
            if i + j + 1 < n {
                out.push([[u + s, v], [u + s, v + s], [u, v + s]]);
            }
        }
    }
    out
}

/// What a triangle's micromap comes to: one state for all of it (a special index, no data), or
/// its states (four a byte, micro-triangle `k` in bits `2 * (k % 4)` of byte `k / 4`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Micromap {
    Uniform(u8),
    States { level: u32, data: Vec<u8> },
}

impl Micromap {
    /// The `VkOpacityMicromapSpecialIndexEXT` of a uniform one.
    pub fn special_index(&self) -> Option<i32> {
        match self {
            Self::Uniform(TRANSPARENT) => Some(-1),
            Self::Uniform(OPAQUE) => Some(-2),
            Self::Uniform(UNKNOWN_TRANSPARENT) => Some(-3),
            Self::Uniform(_) => Some(-4),
            Self::States { .. } => None,
        }
    }

    /// Micro-triangle `k`'s state.
    pub fn state(&self, k: u32) -> u8 {
        match self {
            Self::Uniform(s) => *s,
            Self::States { data, .. } => data[(k / 4) as usize] >> (2 * (k % 4)) & 3,
        }
    }
}

/// The level fine enough for a triangle whose texture coordinates `uv` span its sprite: about a
/// micro-triangle edge a texel along its longest edge (exact on a block face's full sprite).
pub fn level_for(uv: [[f32; 2]; 3], mask: &AlphaMask) -> u32 {
    let texels = |a: [f32; 2], b: [f32; 2]| {
        let dx = (a[0] - b[0]) * mask.width as f32;
        let dy = (a[1] - b[1]) * mask.height as f32;
        dx.abs().max(dy.abs())
    };
    let longest = texels(uv[0], uv[1])
        .max(texels(uv[1], uv[2]))
        .max(texels(uv[2], uv[0]));
    let mut level = 0;
    while level < MAX_LEVEL && ((1u32 << level) as f32) < longest - 1e-3 {
        level += 1;
    }
    level
}

/// Whether triangle `t` and the rectangle `[x0, x1] x [y0, y1]` overlap with some area (touching
/// alone does not count: a ray meeting exactly an edge is a measure-zero case the hardware's own
/// precision decides anyway).
fn triangle_meets_rect(t: [[f32; 2]; 3], x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    let (min_x, max_x) = (
        t[0][0].min(t[1][0]).min(t[2][0]),
        t[0][0].max(t[1][0]).max(t[2][0]),
    );
    let (min_y, max_y) = (
        t[0][1].min(t[1][1]).min(t[2][1]),
        t[0][1].max(t[1][1]).max(t[2][1]),
    );
    if max_x <= x0 || min_x >= x1 || max_y <= y0 || min_y >= y1 {
        return false;
    }
    // The triangle's edges as separating axes.
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    for e in 0..3 {
        let a = t[e];
        let b = t[(e + 1) % 3];
        let c = t[(e + 2) % 3];
        let normal = [b[1] - a[1], a[0] - b[0]];
        let side = |p: [f32; 2]| normal[0] * (p[0] - a[0]) + normal[1] * (p[1] - a[1]);
        let inside = side(c);
        if inside == 0.0 {
            continue;
        }
        if corners.iter().all(|&p| side(p) * inside <= 0.0) {
            return false;
        }
    }
    true
}

/// How far inside a texel's edges a micro-triangle must reach to count as over it, in texels.
const EDGE: f32 = 1e-4;

/// The micromap of a triangle whose corners have texture coordinates `uv` (its first, second and
/// third vertices) on the sprite `mask`, at `level`.
pub fn micromap(uv: [[f32; 2]; 3], mask: &AlphaMask, level: u32) -> Micromap {
    let level = level.min(MAX_LEVEL);
    let count = 1u32 << (2 * level);
    let mut data = vec![0u8; (count as usize).div_ceil(4)];
    let (w, h) = (mask.width as f32, mask.height as f32);
    let at = |b: [f32; 2]| {
        let a = 1.0 - b[0] - b[1];
        [
            (uv[0][0] * a + uv[1][0] * b[0] + uv[2][0] * b[1]) * w,
            (uv[0][1] * a + uv[1][1] * b[0] + uv[2][1] * b[1]) * h,
        ]
    };
    let mut seen = [false; 4];
    for corners in micro_triangles(level) {
        let c = [
            (corners[0][0] + corners[1][0] + corners[2][0]) / 3.0,
            (corners[0][1] + corners[1][1] + corners[2][1]) / 3.0,
        ];
        let k = curve_index(c[0], c[1], level);
        // The micro-triangle in texels, and the texels it overlaps (clamped as the shader reads).
        let t = corners.map(at);
        let lo_x = t.iter().map(|p| p[0]).fold(f32::MAX, f32::min).floor() as i64;
        let hi_x = t.iter().map(|p| p[0]).fold(f32::MIN, f32::max).floor() as i64;
        let lo_y = t.iter().map(|p| p[1]).fold(f32::MAX, f32::min).floor() as i64;
        let hi_y = t.iter().map(|p| p[1]).fold(f32::MIN, f32::max).floor() as i64;
        let (mut any_opaque, mut any_clear) = (false, false);
        for y in lo_y..=hi_y {
            for x in lo_x..=hi_x {
                // The texel's area, shrunk by a hair so that float rounding at a shared edge
                // doesn't count a neighbour the micro-triangle only touches.
                let (fx, fy) = (x as f32 + EDGE, y as f32 + EDGE);
                if !triangle_meets_rect(t, fx, fy, fx + 1.0 - 2.0 * EDGE, fy + 1.0 - 2.0 * EDGE) {
                    continue;
                }
                let tx = x.clamp(0, i64::from(mask.width) - 1) as u32;
                let ty = y.clamp(0, i64::from(mask.height) - 1) as u32;
                if mask.opaque(tx, ty) {
                    any_opaque = true;
                } else {
                    any_clear = true;
                }
            }
        }
        let state = match (any_opaque, any_clear) {
            (true, false) => OPAQUE,
            (false, true) => TRANSPARENT,
            (true, true) => UNKNOWN_OPAQUE,
            // Degenerate (no area met): the shader decides.
            (false, false) => UNKNOWN_OPAQUE,
        };
        seen[state as usize] = true;
        data[(k / 4) as usize] |= state << (2 * (k % 4));
    }
    match seen {
        [true, false, false, false] => Micromap::Uniform(TRANSPARENT),
        [false, true, false, false] => Micromap::Uniform(OPAQUE),
        [false, false, false, true] => Micromap::Uniform(UNKNOWN_OPAQUE),
        _ => Micromap::States { level, data },
    }
}

/// The share of unknown micro-triangles above which a finer level is tried (`fitted`).
const UNKNOWN_SHARE: f32 = 0.25;

/// The micromap of a triangle at the first level from `level_for` that leaves at most a quarter
/// of it to the shader, two levels finer at most (a sprite mapped at an angle or in part: its
/// micro-triangles straddle texels at one a texel).
pub fn fitted(uv: [[f32; 2]; 3], mask: &AlphaMask) -> Micromap {
    let first = level_for(uv, mask);
    let mut best = micromap(uv, mask, first);
    for level in first + 1..=(first + 2).min(MAX_LEVEL) {
        if best.unknown_share() <= UNKNOWN_SHARE {
            break;
        }
        best = micromap(uv, mask, level);
    }
    best
}

impl Micromap {
    /// The share of its micro-triangles the shader decides.
    pub fn unknown_share(&self) -> f32 {
        match self {
            Self::Uniform(s) => f32::from(u8::from(*s >= UNKNOWN_TRANSPARENT)),
            Self::States { level, .. } => {
                let n = 1u32 << (2 * level);
                (0..n)
                    .filter(|&k| self.state(k) >= UNKNOWN_TRANSPARENT)
                    .count() as f32
                    / n as f32
            }
        }
    }
}

/// What a sprite layer is to the alpha test, for [`Cache::section`].
#[derive(Clone, Copy)]
pub enum Layer<'a> {
    /// Taken as opaque everywhere (opaque leaves, sprites past the first array, a sprite with no
    /// transparent texel): the hardware confirms its hits.
    Opaque,
    /// Animated: its alpha changes, the shader decides.
    Changing,
    /// A still cutout, its alpha test.
    Mask(&'a AlphaMask),
}

/// A section's micromaps, for one micromap build and its geometries: the states of its distinct
/// triangles (`data`, each at its `triangles` offset), and for each alpha-tested geometry the
/// index of each of its triangles' (a special index where it is uniform).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SectionMicromaps {
    pub data: Vec<u8>,
    /// `VkMicromapTriangleEXT`: the data offset, then the level in the low half and the format
    /// (4-state, 2) in the high half.
    pub triangles: Vec<[u32; 2]>,
    /// `VkMicromapUsageEXT`: how many triangles are at each level.
    pub usage: Vec<(u32, u32)>,
    /// Each alpha-tested geometry (its place among the section's solid geometries) and its
    /// triangles' indices.
    pub geometries: Vec<(u32, Vec<i32>)>,
}

/// The `VkOpacityMicromapFormatEXT` of four states.
pub const FORMAT_4_STATE: u32 = 2;

impl SectionMicromaps {
    /// How many of its triangles' states are left to the shader, of how many (for the log).
    pub fn unknown(&self) -> (u32, u32) {
        let mut unknown = 0;
        let mut total = 0;
        for (_, indices) in &self.geometries {
            for &i in indices {
                total += 1;
                unknown += u32::from(i == -3 || i == -4);
            }
        }
        (unknown, total)
    }
}

/// Triangles' micromaps by sprite layer and texture coordinates: a section's leaves are the same
/// few quads over and over, as are the next section's.
#[derive(Default)]
pub struct Cache {
    known: std::collections::HashMap<(u16, [u32; 6]), Micromap>,
}

/// How many triangles the cache keeps before it starts again (each a few hundred bytes at most).
const CACHE_LIMIT: usize = 1 << 16;

impl Cache {
    /// The micromap of a triangle with texture coordinates `uv` on sprite `layer`.
    pub fn triangle(&mut self, layer: u16, uv: [[f32; 2]; 3], kind: Layer<'_>) -> Micromap {
        let mask = match kind {
            Layer::Opaque => return Micromap::Uniform(OPAQUE),
            Layer::Changing => return Micromap::Uniform(UNKNOWN_OPAQUE),
            Layer::Mask(mask) => mask,
        };
        let key = (
            layer,
            [
                uv[0][0].to_bits(),
                uv[0][1].to_bits(),
                uv[1][0].to_bits(),
                uv[1][1].to_bits(),
                uv[2][0].to_bits(),
                uv[2][1].to_bits(),
            ],
        );
        if let Some(m) = self.known.get(&key) {
            return m.clone();
        }
        if self.known.len() >= CACHE_LIMIT {
            self.known.clear();
        }
        let m = fitted(uv, mask);
        self.known.insert(key, m.clone());
        m
    }

    /// Forget every triangle (the sprites changed: another resource pack).
    pub fn clear(&mut self) {
        self.known.clear();
    }

    /// The micromaps of a section's alpha-tested geometries: each `(first, count)` quads, quad
    /// `q`'s layer and corners' texture coordinates from `quad(q)`, its two triangles corners 0,
    /// 1, 2 and 0, 2, 3 (gfx.rs `quad_indices`), the layer's alpha test from `layer`. `None`
    /// where every triangle is uniformly opaque or unknown, which no micromap improves on.
    pub fn section<'a>(
        &mut self,
        geometries: &[(u32, u32, u32)],
        quad: impl Fn(u32) -> (u16, [[f32; 2]; 4]),
        layer: impl Fn(u16) -> Layer<'a>,
    ) -> Option<SectionMicromaps> {
        let mut out = SectionMicromaps::default();
        let mut distinct: std::collections::HashMap<(u32, Vec<u8>), i32> = Default::default();
        let mut levels = std::collections::BTreeMap::<u32, u32>::new();
        let mut helps = false;
        for &(geometry, first, count) in geometries {
            let mut indices = Vec::with_capacity(count as usize * 2);
            for q in first..first + count {
                let (l, uv) = quad(q);
                let kind = layer(l);
                for tri in [[uv[0], uv[1], uv[2]], [uv[0], uv[2], uv[3]]] {
                    let m = self.triangle(l, tri, kind);
                    let index = match m {
                        Micromap::Uniform(_) => m.special_index().unwrap_or(-4),
                        Micromap::States { level, data } => {
                            let next = out.triangles.len() as i32;
                            *distinct
                                .entry((level, data))
                                .or_insert_with_key(|(level, data)| {
                                    out.triangles.push([
                                        out.data.len() as u32,
                                        level | FORMAT_4_STATE << 16,
                                    ]);
                                    out.data.extend_from_slice(data);
                                    *levels.entry(*level).or_default() += 1;
                                    next
                                })
                        }
                    };
                    helps |= index >= 0 || index == -1 || index == -2;
                    indices.push(index);
                }
            }
            out.geometries.push((geometry, indices));
        }
        if !helps {
            return None;
        }
        out.usage = levels.into_iter().map(|(level, n)| (n, level)).collect();
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn centroid(t: [[f32; 2]; 3]) -> [f32; 2] {
        [
            (t[0][0] + t[1][0] + t[2][0]) / 3.0,
            (t[0][1] + t[1][1] + t[2][1]) / 3.0,
        ]
    }

    #[test]
    fn the_curve_numbers_every_micro_triangle_once() {
        for level in 0..=7 {
            let tris = micro_triangles(level);
            assert_eq!(tris.len(), 1 << (2 * level));
            let mut seen = vec![false; tris.len()];
            for t in tris {
                let c = centroid(t);
                let k = curve_index(c[0], c[1], level) as usize;
                assert!(k < seen.len(), "level {level}: index {k}");
                assert!(!seen[k], "level {level}: index {k} twice");
                seen[k] = true;
            }
        }
    }

    #[test]
    fn the_curve_places_any_point_in_the_micro_triangle_holding_it() {
        // Points inside a micro-triangle (not on its edges) take its centroid's index.
        let level = 4;
        for t in micro_triangles(level) {
            let c = centroid(t);
            let k = curve_index(c[0], c[1], level);
            for corner in t {
                let p = [
                    c[0] + 0.8 * (corner[0] - c[0]),
                    c[1] + 0.8 * (corner[1] - c[1]),
                ];
                assert_eq!(curve_index(p[0], p[1], level), k);
            }
        }
    }

    #[test]
    fn whole_sprites_are_uniform() {
        let full = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        let opaque = AlphaMask::from_fn(16, 16, |_, _| true);
        let clear = AlphaMask::from_fn(16, 16, |_, _| false);
        assert_eq!(micromap(full, &opaque, 4), Micromap::Uniform(OPAQUE));
        assert_eq!(micromap(full, &clear, 4), Micromap::Uniform(TRANSPARENT));
        assert_eq!(Micromap::Uniform(OPAQUE).special_index(), Some(-2));
        assert_eq!(Micromap::Uniform(TRANSPARENT).special_index(), Some(-1));
    }

    #[test]
    fn a_block_face_s_triangle_at_its_texels_level_knows_every_state() {
        // A quad's triangles over a whole 16x16 sprite (corners 0, 1, 2 and 0, 2, 3), a
        // checkerboard: every micro-triangle lies in one texel.
        let mask = AlphaMask::from_fn(16, 16, |x, y| (x + y) % 2 == 0);
        let quad = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        for tri in [[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]] {
            assert_eq!(level_for(tri, &mask), 4);
            let m = micromap(tri, &mask, 4);
            for k in 0..256 {
                let s = m.state(k);
                assert!(s == OPAQUE || s == TRANSPARENT, "micro-triangle {k}: {s}");
            }
        }
    }

    #[test]
    fn a_known_state_is_the_alpha_test_s_answer_wherever_a_ray_meets_it() {
        // A leaf-like sprite under a rotated, partial mapping: every point's known state agrees
        // with the texel the shader would read there.
        let mask = AlphaMask::from_fn(16, 16, |x, y| (x * 7 + y * 3) % 5 < 2 || x == y);
        let uv = [[0.9, 0.05], [0.2, 0.3], [0.6, 0.95]];
        let m = fitted(uv, &mask);
        let level = match &m {
            Micromap::States { level, .. } => *level,
            Micromap::Uniform(_) => panic!("a busy sprite is not uniform"),
        };
        assert!(
            level > level_for(uv, &mask),
            "an angled mapping takes a finer level"
        );
        let mut known = 0;
        let mut rng = 12345u32;
        let mut next = || {
            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (rng >> 8) as f32 / 16_777_216.0
        };
        for _ in 0..20_000 {
            let (mut u, mut v) = (next(), next());
            if u + v > 1.0 {
                (u, v) = (1.0 - u, 1.0 - v);
            }
            let k = curve_index(u, v, level);
            let s = m.state(k);
            if s == UNKNOWN_OPAQUE || s == UNKNOWN_TRANSPARENT {
                continue;
            }
            known += 1;
            let a = 1.0 - u - v;
            let p = [
                uv[0][0] * a + uv[1][0] * u + uv[2][0] * v,
                uv[0][1] * a + uv[1][1] * u + uv[2][1] * v,
            ];
            let (x, y) = mask.texel_at(p);
            assert_eq!(s == OPAQUE, mask.opaque(x, y), "at {p:?}");
        }
        assert!(known > 12_000, "only {known} points met a known state");
    }

    #[test]
    fn an_alpha_mask_reads_rgba_s_half_alpha_as_the_shader_does() {
        let rgba = [0, 0, 0, 127, 0, 0, 0, 128];
        let mask = AlphaMask::from_rgba(2, 1, &rgba);
        assert!(!mask.opaque(0, 0));
        assert!(mask.opaque(1, 0));
    }

    #[test]
    fn a_section_s_leaves_share_their_micromaps() {
        let leaf = AlphaMask::from_fn(16, 16, |x, y| (x + y) % 3 != 0);
        let quad = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let mut cache = Cache::default();
        // Ten leaf quads, then an opaque leaf (layer 7), in one geometry (its place: 1).
        let section = cache
            .section(
                &[(1, 0, 11)],
                |q| (if q < 10 { 3 } else { 7 }, quad),
                |l| {
                    if l == 3 {
                        Layer::Mask(&leaf)
                    } else {
                        Layer::Opaque
                    }
                },
            )
            .expect("leaves gain from micromaps");
        assert_eq!(
            section.triangles.len(),
            2,
            "one micromap a triangle of the quad"
        );
        assert_eq!(section.usage, vec![(2, 4)]);
        assert_eq!(section.data.len(), 2 * 64);
        let (geometry, indices) = &section.geometries[0];
        assert_eq!(*geometry, 1);
        assert_eq!(indices.len(), 22);
        assert_eq!(&indices[..4], &[0, 1, 0, 1]);
        assert_eq!(&indices[20..], &[-2, -2]);
        assert_eq!(section.triangles[1], [64, 4 | FORMAT_4_STATE << 16]);
        // Animated sprites alone: nothing to gain.
        assert_eq!(
            cache.section(&[(0, 0, 2)], |_| (5, quad), |_| Layer::Changing),
            None
        );
    }
}
