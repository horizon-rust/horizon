//! HDR captures of RT lighting's graded scene (the light the HDR output shows, 1 = paper white):
//! an F2 screenshot's HDR10 PNG beside its SDR one, and Shift+F2's (or `horizon-ctl record`'s)
//! video, a few seconds of frames at half size, for looking at motion and flicker frame by frame.
//!
//! The PNGs are HDR10 as the PNG specification's third edition (2025) defines it: 16 bits a
//! channel, BT.2020 primaries, the PQ curve, said by a `cICP` chunk, so a browser or Windows'
//! Photos shows them in HDR on an HDR display.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use crate::Result;

/// The graded light (1 = paper white, BT.709 linear) as an HDR10 PNG, `paper_white` nits a 1.
pub fn write_pq_png(
    path: &Path,
    [width, height]: [u32; 2],
    rgb: &[[f32; 3]],
    paper_white: f32,
) -> Result<()> {
    let mut data = Vec::with_capacity(rgb.len() * 6);
    for &c in rgb {
        for v in to_bt2020(c) {
            data.extend_from_slice(&pq(v * paper_white).to_be_bytes());
        }
    }
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(path)?),
        width,
        height,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Sixteen);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header()?;
    // cICP: BT.2020 primaries (9), PQ (16), RGB (0), full range (1); before the image data.
    writer.write_chunk(png::chunk::ChunkType(*b"cICP"), &[9, 16, 0, 1])?;
    writer.write_image_data(&data)?;
    Ok(())
}

/// BT.709 linear light into BT.2020's primaries (ITU-R BT.2087).
fn to_bt2020([r, g, b]: [f32; 3]) -> [f32; 3] {
    [
        0.6274 * r + 0.3293 * g + 0.0433 * b,
        0.0691 * r + 0.9195 * g + 0.0114 * b,
        0.0164 * r + 0.0880 * g + 0.8956 * b,
    ]
}

/// SMPTE ST 2084's PQ code for `nits`, 16 bits.
fn pq(nits: f32) -> u16 {
    let y = (nits.max(0.0) / 10_000.0).min(1.0);
    let (m1, m2) = (0.159_301_76, 78.843_75);
    let (c1, c2, c3) = (0.835_937_5, 18.851_563, 18.6875);
    let p = y.powf(m1);
    let v = ((c1 + c2 * p) / (1.0 + c3 * p)).powf(m2);
    (v * 65535.0).round() as u16
}

/// Half the size: each 2x2 block averaged (an odd last row or column dropped).
pub fn half(rgb: &[[f32; 3]], [width, height]: [u32; 2]) -> (Vec<[f32; 3]>, [u32; 2]) {
    let (w, h) = ((width / 2).max(1), (height / 2).max(1));
    let mut out = Vec::with_capacity((w * h) as usize);
    let at = |x: u32, y: u32| rgb[(y.min(height - 1) * width + x.min(width - 1)) as usize];
    for y in 0..h {
        for x in 0..w {
            let [a, b, c, d] = [
                at(2 * x, 2 * y),
                at(2 * x + 1, 2 * y),
                at(2 * x, 2 * y + 1),
                at(2 * x + 1, 2 * y + 1),
            ];
            out.push(std::array::from_fn(|i| (a[i] + b[i] + c[i] + d[i]) * 0.25));
        }
    }
    (out, [w, h])
}

/// One recorded frame, for the writer thread.
struct Frame {
    index: u32,
    millis: f64,
    rgb: Vec<[f32; 3]>,
    size: [u32; 2],
    paper_white: f32,
}

/// A video being recorded: frames left, and the thread writing them (`frame_NNNN.png`, and
/// `frames.txt`, each frame's time since the start in milliseconds, written last).
pub struct Recording {
    left: u32,
    index: u32,
    start: Instant,
    seconds: f32,
    sender: Option<mpsc::Sender<Frame>>,
}

impl Recording {
    /// `seconds` of frames into `dir` (made), at the frame rate the game runs at while it records.
    pub fn new(dir: PathBuf, seconds: f32) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let (sender, receiver) = mpsc::channel::<Frame>();
        let out = dir.clone();
        std::thread::Builder::new()
            .name("Horizon video capture".into())
            .spawn(move || {
                let mut times = String::from("frame\tms\n");
                for f in receiver {
                    let path = out.join(format!("frame_{:04}.png", f.index));
                    if let Err(e) = write_pq_png(&path, f.size, &f.rgb, f.paper_white) {
                        log::warn!("Horizon: video frame {}: {e}", path.display());
                    }
                    times.push_str(&format!("{}\t{:.2}\n", f.index, f.millis));
                }
                let _ = std::fs::write(out.join("frames.txt"), times);
                log::info!("Horizon: video written to {}", out.display());
            })?;
        Ok(Self {
            // At most 120 frames a second: the game hardly reaches it while each is read back.
            left: (seconds.clamp(0.1, 30.0) * 120.0) as u32,
            index: 0,
            start: Instant::now(),
            seconds: seconds.clamp(0.1, 30.0),
            sender: Some(sender),
        })
    }

    /// Whether it still takes frames.
    pub fn wants(&self) -> bool {
        self.left > 0 && self.start.elapsed().as_secs_f32() < self.seconds
    }

    /// This frame's graded light (full size), halved and handed to the writer.
    pub fn push(&mut self, rgb: &[[f32; 3]], size: [u32; 2], paper_white: f32) {
        if !self.wants() {
            self.sender = None;
            return;
        }
        let (rgb, size) = half(rgb, size);
        if let Some(sender) = &self.sender {
            let _ = sender.send(Frame {
                index: self.index,
                millis: self.start.elapsed().as_secs_f64() * 1000.0,
                rgb,
                size,
                paper_white,
            });
        }
        self.index += 1;
        self.left -= 1;
        if self.left == 0 {
            // The writer finishes the queue and writes `frames.txt`.
            self.sender = None;
        }
    }

    /// Whether it is done (the writer may still be writing its last frames).
    pub fn done(&self) -> bool {
        self.sender.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_white_and_the_peak_land_on_their_pq_codes() {
        // 100 nits is about 51 % of PQ's range, 1000 nits about 75 %, 10000 all of it.
        assert!((f32::from(pq(100.0)) / 65535.0 - 0.508).abs() < 0.002);
        assert!((f32::from(pq(1000.0)) / 65535.0 - 0.752).abs() < 0.002);
        assert_eq!(pq(10_000.0), 65535);
        assert_eq!(pq(0.0), 0);
    }

    #[test]
    fn white_stays_white_in_bt2020() {
        let w = to_bt2020([1.0, 1.0, 1.0]);
        assert!(w.iter().all(|v| (v - 1.0).abs() < 1e-3), "{w:?}");
    }

    #[test]
    fn halving_averages_each_block() {
        let rgb = vec![[0.0; 3], [1.0; 3], [1.0; 3], [0.0; 3]];
        let (out, size) = half(&rgb, [2, 2]);
        assert_eq!(size, [1, 1]);
        assert_eq!(out, vec![[0.5; 3]]);
    }
}
