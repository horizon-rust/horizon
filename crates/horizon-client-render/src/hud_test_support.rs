//! Pack textures and atlas reads for tests: the engine's and the pack formats' (feature
//! `test-support`).
use crate::hud::{HudAtlas, Rect};
use std::fs;
use std::path::Path;

/// Write a PNG `width x height` whose `frames` stacked rows are solid
/// colours, then its mcmeta beside it.
pub fn write_strip(path: &Path, width: u32, height: u32, frames: &[[u8; 4]], mcmeta: Option<&str>) {
    let file = std::io::BufWriter::new(fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for frame in frames {
        let rows = height / frames.len() as u32;
        pixels.extend((0..width * rows).flat_map(|_| *frame));
    }
    writer.write_image_data(&pixels).unwrap();
    writer.finish().unwrap();
    if let Some(mcmeta) = mcmeta {
        let meta = path.with_extension("png.mcmeta");
        fs::write(meta, mcmeta).unwrap();
    }
}

/// The atlas's texel at (`x`, `y`) inside `rect`.
pub fn texel_at(atlas: &HudAtlas, rect: Rect, x: u32, y: u32) -> [u8; 4] {
    let px = (rect.u0 * atlas.width as f32).round() as u32 + x;
    let py = (rect.v0 * atlas.height as f32).round() as u32 + y;
    let at = ((py * atlas.width + px) * 4) as usize;
    atlas.pixels[at..at + 4].try_into().unwrap()
}
