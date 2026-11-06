//! The wind map (`horizon_client::gfx::wind`) of a saved world, drawn as a weather chart: the
//! terrain shaded by its heights, the sea in blue, and an arrow every 32 blocks for the wind
//! there, its length and colour by its speed against the prevailing wind's (blue calmer, white
//! the same, red faster), the valleys that confine it tinted violet. No game: it reads the
//! region files' height maps (MOTION_BLOCKING) and only reads them.
//!
//! ```sh
//! cargo run --release -p horizon-client --example wind_map -- \
//!     --world <save folder> [--center X,Z] [--size 1024] [--wind DEGREES,M_S] --out wind.png
//! ```
//! `--synthetic` in place of `--world` draws a made-up terrain (hills, a valley, a pass, a sea).
//! `--wind`: where the wind blows toward, in degrees from east (+x) toward south (+z), and its
//! speed (default 30,3: a breeze toward the south-east).
use horizon_client::gfx::wind;
use horizon_core::position::ChunkPos;
use horizon_world::nbt::{Limits, Tag};
use horizon_world::region::RegionFile;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MIN_Y: i32 = -64;

struct Args {
    world: Option<PathBuf>,
    center: [i32; 2],
    size: usize,
    wind: [f32; 2],
    out: PathBuf,
}

fn args() -> Result<Args, String> {
    let mut world = None;
    let mut center = [0, 0];
    let mut size = 1024usize;
    let mut wind = [30.0f32, 3.0];
    let mut out = PathBuf::from("wind_map.png");
    let mut it = std::env::args().skip(1);
    let pair = |v: Option<String>| -> Result<[f64; 2], String> {
        let v = v.ok_or("a value is missing")?;
        let mut p = v.split(',').map(|x| x.trim().parse::<f64>());
        match (p.next(), p.next()) {
            (Some(Ok(a)), Some(Ok(b))) => Ok([a, b]),
            _ => Err(format!("expected two numbers: {v}")),
        }
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--world" => world = it.next().map(PathBuf::from),
            "--synthetic" => world = Some(PathBuf::new()),
            "--center" => center = pair(it.next())?.map(|v| v as i32),
            "--size" => {
                size = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--size takes a number")?
            }
            "--wind" => wind = pair(it.next())?.map(|v| v as f32),
            "--out" => out = it.next().map(PathBuf::from).ok_or("--out takes a path")?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if !size.is_power_of_two() || size < 64 {
        return Err("--size must be a power of two, 64 or more".into());
    }
    let (deg, speed) = (wind[0].to_radians(), wind[1]);
    Ok(Args {
        world: world
            .map(|w| (!w.as_os_str().is_empty()).then_some(w))
            .ok_or("--world <save folder> (or --synthetic) is needed")?,
        center,
        size,
        wind: [speed * deg.cos(), speed * deg.sin()],
        out,
    })
}

/// A chunk's MOTION_BLOCKING heights (the top of what blocks motion: the ground, the trees),
/// `x + 16 z`, in blocks; `None` if it has none.
fn chunk_heights(root: &Tag) -> Option<[i32; 256]> {
    let root = match root.get("Level") {
        Some(level @ Tag::Compound(_)) => level,
        _ => root,
    };
    let min_y = match root.get("yPos") {
        Some(Tag::Int(y)) => y * 16,
        Some(Tag::Byte(y)) => i32::from(*y) * 16,
        _ => MIN_Y,
    };
    let maps = root.get("Heightmaps")?;
    let longs = match maps
        .get("MOTION_BLOCKING")
        .or_else(|| maps.get("WORLD_SURFACE"))
    {
        Some(Tag::LongArray(l)) => l,
        _ => return None,
    };
    // Since 1.16: 9 bits a column, 7 to a long, none across two.
    let mut out = [MIN_Y; 256];
    for (i, h) in out.iter_mut().enumerate() {
        let long = *longs.get(i / 7)? as u64;
        let v = (long >> ((i % 7) * 9)) & 0x1ff;
        *h = min_y + v as i32 - 1;
    }
    Some(out)
}

/// The blocks' heights over the `size` x `size` blocks from `first` (`i32::MIN` where no chunk
/// is saved).
fn heights(world: &Path, first: [i32; 2], size: usize) -> Vec<i32> {
    let mut out = vec![i32::MIN; size * size];
    let mut regions: HashMap<(i32, i32), Option<RegionFile>> = HashMap::new();
    let (c0, c1) = (
        [first[0] >> 4, first[1] >> 4],
        [
            (first[0] + size as i32 - 1) >> 4,
            (first[1] + size as i32 - 1) >> 4,
        ],
    );
    let mut found = 0;
    for cz in c0[1]..=c1[1] {
        for cx in c0[0]..=c1[0] {
            let key = (cx >> 5, cz >> 5);
            let region = regions.entry(key).or_insert_with(|| {
                RegionFile::open(
                    world
                        .join("region")
                        .join(format!("r.{}.{}.mca", key.0, key.1)),
                )
                .ok()
            });
            let Some(region) = region else { continue };
            let Ok(Some(record)) = region.read(ChunkPos::new(cx, cz)) else {
                continue;
            };
            let Some(h) = record
                .nbt(Limits::default())
                .ok()
                .and_then(|root| chunk_heights(&root.tag))
            else {
                continue;
            };
            found += 1;
            for z in 0..16 {
                for x in 0..16 {
                    let (bx, bz) = (cx * 16 + x - first[0], cz * 16 + z - first[1]);
                    if (0..size as i32).contains(&bx) && (0..size as i32).contains(&bz) {
                        out[bz as usize * size + bx as usize] = h[(x + 16 * z) as usize];
                    }
                }
            }
        }
    }
    eprintln!("{found} chunks read");
    out
}

/// A made-up terrain `n` blocks a side: rolling hills, a ridge across with a pass, a valley
/// along z, a sea in a corner.
fn synthetic(n: usize) -> Vec<i32> {
    let s = n as f32;
    (0..n * n)
        .map(|k| {
            let (x, z) = ((k % n) as f32 / s, (k / n) as f32 / s);
            let hills =
                8.0 * ((x * 23.0).sin() * (z * 19.0).cos()) + 5.0 * ((x * 51.0 + z * 37.0).sin());
            let valley = 60.0 * (((x - 0.45) / 0.12).powi(2)).min(1.0);
            let pass = 1.0 - 0.7 * (-((z - 0.5) / 0.06).powi(2)).exp();
            let ridge =
                70.0 * (-((z - 0.5) / 0.08).powi(2)).exp() * pass * (x - 0.3).max(0.0) / 0.7;
            let sea = 45.0 * ((x + z - 1.45) / 0.3).clamp(0.0, 1.0);
            (70.0 + hills + valley + ridge - sea) as i32
        })
        .collect()
}

/// The colour of the ground at height `h` (`None`: not saved), shaded by `shade` (0 to 2).
fn ground(h: i32, shade: f32) -> [f32; 3] {
    if h == i32::MIN {
        return [0.12, 0.12, 0.12];
    }
    if h <= 62 {
        let deep = ((62 - h) as f32 / 30.0).clamp(0.0, 1.0);
        return [0.18 - 0.1 * deep, 0.36 - 0.15 * deep, 0.62 - 0.2 * deep];
    }
    let stops = [
        (63.0, [0.36, 0.56, 0.30]),
        (90.0, [0.55, 0.64, 0.36]),
        (130.0, [0.62, 0.52, 0.38]),
        (170.0, [0.58, 0.56, 0.54]),
        (220.0, [0.95, 0.95, 0.97]),
    ];
    let hf = h as f32;
    let mut c = stops[stops.len() - 1].1;
    for w in stops.windows(2) {
        if hf <= w[1].0 {
            let t = ((hf - w[0].0) / (w[1].0 - w[0].0)).clamp(0.0, 1.0);
            c = [0, 1, 2].map(|k| w[0].1[k] + (w[1].1[k] - w[0].1[k]) * t);
            break;
        }
    }
    c.map(|v| (v * shade).clamp(0.0, 1.0))
}

struct Canvas {
    size: usize,
    rgb: Vec<[f32; 3]>,
}
impl Canvas {
    fn blend(&mut self, x: i32, y: i32, c: [f32; 3], a: f32) {
        if x < 0 || y < 0 || x >= self.size as i32 || y >= self.size as i32 {
            return;
        }
        let p = &mut self.rgb[y as usize * self.size + x as usize];
        *p = [0, 1, 2].map(|k| p[k] + (c[k] - p[k]) * a);
    }
    /// A line `width` pixels wide from `a` to `b`.
    fn line(&mut self, a: [f32; 2], b: [f32; 2], width: f32, c: [f32; 3]) {
        let (x0, x1) = (a[0].min(b[0]) - width, a[0].max(b[0]) + width);
        let (y0, y1) = (a[1].min(b[1]) - width, a[1].max(b[1]) + width);
        let d = [b[0] - a[0], b[1] - a[1]];
        let len2 = (d[0] * d[0] + d[1] * d[1]).max(1e-6);
        for y in y0.floor() as i32..=y1.ceil() as i32 {
            for x in x0.floor() as i32..=x1.ceil() as i32 {
                let p = [x as f32 + 0.5 - a[0], y as f32 + 0.5 - a[1]];
                let t = ((p[0] * d[0] + p[1] * d[1]) / len2).clamp(0.0, 1.0);
                let q = [p[0] - d[0] * t, p[1] - d[1] * t];
                let dist = (q[0] * q[0] + q[1] * q[1]).sqrt();
                let cover = (width * 0.5 + 0.5 - dist).clamp(0.0, 1.0);
                if cover > 0.0 {
                    self.blend(x, y, c, cover);
                }
            }
        }
    }
    /// An arrow from `at` along `v` (pixels), outlined in black.
    fn arrow(&mut self, at: [f32; 2], v: [f32; 2], c: [f32; 3]) {
        let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
        if len < 1.0 {
            return;
        }
        let u = [v[0] / len, v[1] / len];
        let tail = [at[0] - v[0] * 0.5, at[1] - v[1] * 0.5];
        let tip = [at[0] + v[0] * 0.5, at[1] + v[1] * 0.5];
        let head = (len * 0.35).clamp(4.0, 9.0);
        let wings = [
            [
                tip[0] - u[0] * head - u[1] * head * 0.6,
                tip[1] - u[1] * head + u[0] * head * 0.6,
            ],
            [
                tip[0] - u[0] * head + u[1] * head * 0.6,
                tip[1] - u[1] * head - u[0] * head * 0.6,
            ],
        ];
        for (w, col) in [(4.6, [0.0; 3]), (2.4, c)] {
            self.line(tail, tip, w, col);
            self.line(tip, wings[0], w, col);
            self.line(tip, wings[1], w, col);
        }
    }
}

/// Blue (calmer than the prevailing wind) to white (as strong) to red (faster).
fn speed_colour(ratio: f32) -> [f32; 3] {
    if ratio < 1.0 {
        let t = ((1.0 - ratio) / 0.6).clamp(0.0, 1.0);
        [1.0 - 0.8 * t, 1.0 - 0.5 * t, 1.0]
    } else {
        let t = ((ratio - 1.0) / 0.8).clamp(0.0, 1.0);
        [1.0, 1.0 - 0.75 * t, 1.0 - 0.85 * t]
    }
}

fn main() -> Result<(), String> {
    let a = args()?;
    let n = a.size;
    let first = [a.center[0] - n as i32 / 2, a.center[1] - n as i32 / 2];
    let blocks = match &a.world {
        Some(world) => heights(world, first, n),
        None => synthetic(n),
    };
    if blocks.iter().all(|&h| h == i32::MIN) {
        return Err(format!("no saved chunk around {:?}", a.center));
    }
    // The far field's level 2: the highest of each 8x8 blocks.
    let cell = wind::CELL as usize;
    let side = n / cell;
    let mut coarse = vec![MIN_Y as i16; side * side];
    for (k, c) in coarse.iter_mut().enumerate() {
        let (i, j) = (k % side, k / side);
        let mut top = i32::MIN;
        for z in 0..cell {
            for x in 0..cell {
                top = top.max(blocks[(j * cell + z) * n + i * cell + x]);
            }
        }
        if top != i32::MIN {
            *c = top as i16;
        }
    }
    let map = wind::terrain_map(&coarse, side, [0, 0]);
    let texel = |i: usize, j: usize| {
        let k = (j.min(side - 1) * side + i.min(side - 1)) * 2;
        wind::unpack([map[k], map[k + 1]])
    };
    // The terrain, lit from the north-west.
    let at = |x: i32, z: i32| {
        let h = blocks[(z.clamp(0, n as i32 - 1) as usize) * n + x.clamp(0, n as i32 - 1) as usize];
        if h == i32::MIN {
            63.0
        } else {
            h.max(62) as f32
        }
    };
    let mut canvas = Canvas {
        size: n,
        rgb: vec![[0.0; 3]; n * n],
    };
    for z in 0..n as i32 {
        for x in 0..n as i32 {
            let (dx, dz) = (at(x + 1, z) - at(x - 1, z), at(x, z + 1) - at(x, z - 1));
            let shade = (1.0 - 0.08 * (dx + dz)).clamp(0.55, 1.4);
            let t = texel(x as usize / cell, z as usize / cell);
            let mut c = ground(blocks[z as usize * n + x as usize], shade);
            // The valleys that confine the wind, violet.
            let v = 0.35 * t[3].clamp(0.0, 1.0);
            c = [
                c[0] + (0.55 - c[0]) * v,
                c[1] + (0.25 - c[1]) * v,
                c[2] + (0.75 - c[2]) * v,
            ];
            canvas.rgb[z as usize * n + x as usize] = c;
        }
    }
    let calm = (a.wind[0] * a.wind[0] + a.wind[1] * a.wind[1])
        .sqrt()
        .max(1e-3);
    let step = 32usize;
    for z in (step / 2..n).step_by(step) {
        for x in (step / 2..n).step_by(step) {
            let h = blocks[z * n + x];
            let y = if h == i32::MIN { 63.0 } else { h as f32 + 2.0 };
            let w = wind::bend(texel(x / cell, z / cell), a.wind, y);
            let speed = (w[0] * w[0] + w[1] * w[1]).sqrt();
            let ratio = speed / calm;
            let len = (18.0 * ratio).clamp(6.0, 28.0);
            let v = [w[0] / speed.max(1e-3) * len, w[1] / speed.max(1e-3) * len];
            canvas.arrow([x as f32, z as f32], v, speed_colour(ratio));
        }
    }
    let file = std::fs::File::create(&a.out).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), n as u32, n as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = canvas
        .rgb
        .iter()
        .flat_map(|c| c.map(|v| (v.clamp(0.0, 1.0).powf(1.0 / 1.1) * 255.0).round() as u8))
        .collect();
    writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
    eprintln!("wrote {}", a.out.display());
    Ok(())
}
