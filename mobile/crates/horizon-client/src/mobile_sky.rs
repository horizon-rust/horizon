//! The desktop launcher's night sky (`launcher/src/sky.rs`), drawn as rectangles for the mobile
//! launcher: the sky's bands in steps, stars that twinkle in steps, three layers of clouds drifting
//! a texel at a time, a sun cut in slices and two skylines. The art and its numbers are the
//! desktop's; only the drawing differs (rectangles instead of baked images).
use horizon_theme::night::{self, Night};
use std::sync::LazyLock;

/// Logical pixels per texel: the sky's bands, the stars and the clouds; the sun and skylines.
const TEXEL: f32 = 4.;
const LAND: f32 = 6.;
/// Each stop of the sky's share of the window's height, top to bottom.
const SHARES: [f32; 6] = [0.2, 0.18, 0.17, 0.15, 0.15, 0.15];

/// Clouds: each layer's sprites, placed `(x, y)` in texels on a strip `period` texels wide that
/// repeats across the window, the strip's top as a share of the window's height, and its speed in
/// texels a second. `#` is the cloud, `=` its lit band, `-` its shaded underside.
struct CloudLayer {
    period: usize,
    top: f32,
    speed: f32,
    clouds: &'static [(usize, usize, &'static [&'static str])],
}

const PUFF: &[&str] = &[
    "..............############..................",
    "......##########################............",
    "..##########################################",
    "============================================",
    ".....----------------------------------.....",
];
const LONG: &[&str] = &[
    "......................##############............................",
    "..............##################################................",
    "....############################################################",
    "==============================================================..",
    "..........------------------------------------------............",
    "..............................#################.................",
    "..............................=================.................",
];
const WISP: &[&str] = &[
    "......##################......",
    "==============================",
];
const BANK: &[&str] = &[
    "............##########........................................",
    "......##########################........##############........",
    "..##################################..########################",
    "====================================..========================",
    "......-------------------------.........---------------.......",
];

const CLOUDS: [CloudLayer; 3] = [
    CloudLayer {
        period: 300,
        top: 0.36,
        speed: 0.5,
        clouds: &[(20, 0, WISP), (140, 3, PUFF), (230, 1, WISP)],
    },
    CloudLayer {
        period: 380,
        top: 0.52,
        speed: 1.,
        clouds: &[(30, 1, BANK), (200, 0, PUFF), (300, 4, WISP)],
    },
    CloudLayer {
        period: 460,
        top: 0.64,
        speed: 2.,
        clouds: &[(60, 0, LONG), (290, 2, PUFF)],
    },
];
#[cfg(test)]
const CLOUD_ROWS: usize = 10;

/// A skyline: runs of `(width, height)` in texels, repeating every `period` (their widths' sum),
/// with sprites stood on top of the run at `x`. `#` is solid.
struct Skyline {
    runs: &'static [(usize, usize)],
    props: &'static [(usize, &'static [&'static str])],
    rows: usize,
}

const TREE: &[&str] = &[".###.", "#####", "#####", ".###.", "..#..", "..#.."];
const PINE: &[&str] = &["..#..", ".###.", "..#..", ".###.", "#####", "..#.."];
const POST: &[&str] = &[".#.", "###", ".#.", ".#.", ".#."];
const TOWER: &[&str] = &["#.#", "###", "###", "###", "###", "###", "###"];

const FAR: Skyline = Skyline {
    runs: &[
        (18, 8),
        (10, 10),
        (6, 13),
        (14, 11),
        (22, 9),
        (8, 12),
        (12, 14),
        (16, 11),
        (20, 8),
        (10, 10),
        (14, 12),
        (24, 9),
        (12, 11),
        (18, 8),
    ],
    props: &[(32, TOWER), (118, PINE), (176, TOWER)],
    rows: 20,
};

const NEAR: Skyline = Skyline {
    runs: &[
        (12, 7),
        (6, 9),
        (10, 8),
        (4, 11),
        (14, 10),
        (8, 8),
        (6, 6),
        (16, 7),
        (5, 10),
        (9, 12),
        (7, 9),
        (12, 7),
        (10, 8),
        (6, 11),
        (11, 9),
        (9, 7),
        (15, 7),
    ],
    props: &[
        (5, TREE),
        (36, POST),
        (60, PINE),
        (95, TREE),
        (124, POST),
        (139, TREE),
    ],
    rows: 18,
};

/// The sun: its radius in texels, how far it stands above the near skyline's foot, and where
/// across the window (a share of its width).
const SUN_RADIUS: usize = 15;
const SUN_RISE: usize = 8;
const SUN_X: f32 = 0.72;

struct Colors {
    sun: [u32; 3],
    far: u32,
    near: u32,
    /// Clouds far to near, and each layer's lit band.
    cloud: [u32; 3],
    lit: [u32; 3],
    shade: u32,
    star: u32,
}

const NIGHT_ART: Colors = Colors {
    sun: [0xffe28c, 0xffbf66, 0xff944f],
    far: 0x3a1f5e,
    near: 0x150f2e,
    cloud: [0x3b2869, 0x6e3a7e, 0xc96a86],
    lit: [0x5a3f8a, 0x9c5a92, 0xf7a98a],
    shade: 0x2a1a4c,
    star: 0xf4f2ff,
};

#[allow(dead_code)]
const DAWN_ART: Colors = Colors {
    sun: [0xfffbea, 0xffeac2, 0xffd3a1],
    far: 0xbaa8de,
    near: 0x8c79c6,
    cloud: [0xeef0fb, 0xfbeef3, 0xfff1e4],
    lit: [0xffffff; 3],
    shade: 0xd9cbe8,
    star: 0xffffff,
};

const STAR_LEVELS: [f32; 4] = [0.95, 0.6, 0.3, 0.6];
/// The clouds' speeds and these periods keep every change on a half-second beat, so the changes of
/// different layers share frames: two a second at most.
const STAR_PERIODS: [f32; 4] = [4., 6., 8., 10.];

/// A rectangle to fill: x, y, width, height in dp, colour 0xRRGGBB, opacity 0..=255.
pub type Rect = (f32, f32, f32, f32, u32, u8);

fn step(t: f32, period: f32, group: usize) -> usize {
    // Groups start a quarter period apart, which keeps their steps on the beat.
    let phase = group as f32 * 0.25;
    ((t / period + phase).fract() * STAR_LEVELS.len() as f32) as usize % STAR_LEVELS.len()
}

/// The sky's bands, `(top, height, colour)` on whole texels: each stop of the sky in four steps
/// toward the next, the stops sharing the height as [`SHARES`] says.
pub fn bands(n: &Night, h: f32) -> Vec<(f32, f32, u32)> {
    const STEPS: usize = 4;
    let rows = (h / TEXEL).ceil();
    let mut out = Vec::with_capacity(SHARES.len() * STEPS);
    let mut at = 0.;
    for (i, share) in SHARES.iter().enumerate() {
        let span = (share * rows).round().max(STEPS as f32);
        let Some(&next) = n.sky.get(i + 1) else {
            out.push((at * TEXEL, (h / TEXEL - at).max(span) * TEXEL, n.sky[i]));
            break;
        };
        let mut from = at;
        for k in 0..STEPS {
            let to = (at + span * (k + 1) as f32 / STEPS as f32).round();
            let c = night::over(next, k as f32 / STEPS as f32, n.sky[i]);
            out.push((from * TEXEL, (to - from) * TEXEL, c));
            from = to;
        }
        at += span;
    }
    out
}

struct Star {
    /// Column in texels, row as a share of the upper half of the window.
    x: f32,
    y: f32,
    group: usize,
    big: bool,
}

/// The stars, the same on every start: a fixed field wide enough for any window, thinned toward
/// the horizon.
fn stars() -> &'static [Star] {
    static STARS: LazyLock<Vec<Star>> = LazyLock::new(|| {
        let mut rng = 0x2545_f491u32;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            (rng >> 8) as f32 / (1 << 24) as f32
        };
        (0..240)
            .filter_map(|i| {
                let (x, y) = ((next() * 960.).floor(), next());
                let keep = next() > y * y;
                let big = next() < 0.08;
                keep.then_some(Star {
                    x,
                    y,
                    group: i % STAR_PERIODS.len(),
                    big,
                })
            })
            .collect()
    });
    &STARS
}

/// A sprite of `#`-art as rectangles, one per run of the same colour in a row, at `(x, y)` dp
/// with `texel` dp per mark. `paint` picks each mark's colour.
fn sprite_rects(
    out: &mut Vec<Rect>,
    (x, y): (f32, f32),
    texel: f32,
    rows: &[&str],
    paint: impl Fn(u8) -> Option<u32>,
) {
    for (dy, row) in rows.iter().enumerate() {
        let bytes = row.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let Some(c) = paint(bytes[i]) else {
                i += 1;
                continue;
            };
            let mut j = i + 1;
            while j < bytes.len() && paint(bytes[j]) == Some(c) {
                j += 1;
            }
            out.push((
                x + i as f32 * texel,
                y + dy as f32 * texel,
                (j - i) as f32 * texel,
                texel,
                c,
                255,
            ));
            i = j;
        }
    }
}

/// The whole sky for a window of `w` by `h` dp, `t` seconds into the animation.
pub fn paint(n: &Night, w: f32, h: f32, t: f32) -> Vec<Rect> {
    let art = &NIGHT_ART;
    let mut out: Vec<Rect> = Vec::new();
    for (top, height, c) in bands(n, h) {
        out.push((0., top, w, height, c, 255));
    }
    for (g, period) in STAR_PERIODS.iter().enumerate() {
        let level = (STAR_LEVELS[step(t, *period, g)] * 255.) as u8;
        for star in stars().iter().filter(|s| s.group == g) {
            let (x, y) = (star.x * TEXEL, star.y * h * 0.5);
            if x >= w {
                continue;
            }
            let size = if star.big { TEXEL / 2. } else { TEXEL / 4. };
            out.push((
                x.round(),
                y.round(),
                size.max(1.),
                size.max(1.),
                art.star,
                level,
            ));
        }
    }
    for (depth, layer) in CLOUDS.iter().enumerate() {
        let strip = layer.period as f32 * TEXEL;
        let shift = (t * layer.speed).floor() * TEXEL % strip;
        let top = (h * layer.top).round();
        let mut x = -shift;
        while x < w {
            for &(cx, cy, sprite) in layer.clouds {
                let at = (x + cx as f32 * TEXEL, top + cy as f32 * TEXEL);
                if at.0 > w || at.0 + sprite[0].len() as f32 * TEXEL < 0. {
                    continue;
                }
                sprite_rects(&mut out, at, TEXEL, sprite, |m| match m {
                    b'#' => Some(art.cloud[depth]),
                    b'=' => Some(art.lit[depth]),
                    b'-' => Some(art.shade),
                    _ => None,
                });
            }
            x += strip;
        }
    }
    // The sun, cut by the horizon, its lower half in slices.
    let foot = h;
    let sun_x = (w * SUN_X - (SUN_RADIUS * 2) as f32 * LAND / 2.).round();
    let sun_y = foot - (SUN_RISE + SUN_RADIUS) as f32 * LAND;
    let r = SUN_RADIUS as f32;
    for y in 0..SUN_RADIUS {
        let dy = r - y as f32 - 0.5;
        let half = (r * r - dy * dy).sqrt().round() as usize;
        let depth = y as f32 / r;
        if depth > 0.5 && (SUN_RADIUS - y).is_multiple_of(3) {
            continue;
        }
        let c = art.sun[if depth < 0.4 {
            0
        } else if depth < 0.7 {
            1
        } else {
            2
        }];
        out.push((
            sun_x + (SUN_RADIUS - half) as f32 * LAND,
            sun_y + y as f32 * LAND,
            (half * 2) as f32 * LAND,
            LAND,
            c,
            255,
        ));
    }
    // The two skylines: the far one starts half a period in, so they never repeat in step.
    for (line, colour, lift) in [(&FAR, art.far, 2.), (&NEAR, art.near, 0.)] {
        let period_t: usize = line.runs.iter().map(|r| r.0).sum();
        let period = period_t as f32 * LAND;
        let top = foot - (line.rows as f32 + lift) * LAND;
        let mut x = if lift > 0. { -period / 2. } else { 0. };
        while x < w {
            let mut col = 0usize;
            let mut tops = vec![0usize; period_t];
            for &(run, height) in line.runs {
                for t in &mut tops[col..col + run] {
                    *t = line.rows - height;
                }
                col += run;
            }
            // Columns of the same height merge into one rectangle.
            let mut i = 0;
            while i < period_t {
                let mut j = i + 1;
                while j < period_t && tops[j] == tops[i] {
                    j += 1;
                }
                let y0 = top + tops[i] as f32 * LAND;
                out.push((
                    x + i as f32 * LAND,
                    y0,
                    (j - i) as f32 * LAND,
                    (line.rows - tops[i]) as f32 * LAND + lift * LAND,
                    colour,
                    255,
                ));
                i = j;
            }
            for &(px, sprite) in line.props {
                let width = sprite[0].len();
                let ground = tops[px..px + width]
                    .iter()
                    .copied()
                    .min()
                    .unwrap_or(line.rows);
                let y = ground.saturating_sub(sprite.len());
                sprite_rects(
                    &mut out,
                    (x + px as f32 * LAND, top + y as f32 * LAND),
                    LAND,
                    sprite,
                    |m| (m == b'#').then_some(colour),
                );
            }
            x += period;
        }
        if lift > 0. {
            out.push((0., foot - lift * LAND, w, lift * LAND, art.far, 255));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use night::NIGHT;

    #[test]
    fn the_bands_cover_the_window_on_whole_texels() {
        for h in [600., 777., 1080.] {
            let mut at = 0.;
            for (top, height, _) in bands(&NIGHT, h) {
                assert_eq!(top, at);
                assert_eq!(top % TEXEL, 0.);
                at += height;
            }
            assert!(at >= h);
        }
    }

    #[test]
    fn the_sky_paints_within_a_sane_number_of_rectangles_and_moves() {
        for (w, h) in [(1040., 700.), (412., 915.), (2560., 1440.)] {
            let a = paint(&NIGHT, w, h, 0.0);
            assert!(
                !a.is_empty() && a.len() < 6000,
                "{} rectangles at {w}x{h}",
                a.len()
            );
            let b = paint(&NIGHT, w, h, 20.0);
            assert_ne!(a.len() + a.iter().map(|r| r.0 as usize).sum::<usize>(), 0);
            assert!(a != b, "the clouds and stars move");
        }
    }

    #[test]
    fn the_art_fits_its_tiles() {
        for layer in &CLOUDS {
            for (x, y, sprite) in layer.clouds {
                assert!(y + sprite.len() <= CLOUD_ROWS);
                assert!(x + sprite[0].len() <= layer.period);
            }
        }
    }
}
