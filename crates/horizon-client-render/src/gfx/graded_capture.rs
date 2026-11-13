//! The graded scene read back: the HDR screenshot beside the PNG, and the frames a recording
//! writes as Radiance HDR (gfx.rs may not grow).

use super::*;

impl Renderer {
    /// The graded scene's rows (`tonemap::SCENE_FORMAT`, eight bytes a pixel) copied into a
    /// buffer to read back.
    pub(super) fn copy_graded(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        size: [u32; 2],
    ) -> wgpu::Buffer {
        let row = (size[0] * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon HDR screenshot"),
            size: u64::from(row) * u64::from(size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        buffer
    }
    /// RT lighting's HDR video: `seconds` of the graded scene's frames, at half size, as HDR10
    /// PNGs into `dir` (gfx/hdr_capture.rs). The game slows while it records (each frame is read
    /// back); `frames.txt` holds their times.
    pub fn start_recording(&mut self, dir: PathBuf, seconds: f32) -> Result<()> {
        self.recording = Some(hdr_capture::Recording::new(dir, seconds)?);
        Ok(())
    }
    /// Whether a video is being recorded.
    pub fn recording(&self) -> bool {
        self.recording.is_some()
    }
    /// An F2 screenshot's HDR copies beside its PNG (`path`): the graded light as a Radiance
    /// file (`.hdr`, linear, for measuring) and as an HDR10 PNG (`-hdr.png`, to look at).
    pub(super) fn write_graded_screenshot(
        &self,
        path: &std::path::Path,
        buffer: &wgpu::Buffer,
        size: [u32; 2],
    ) -> Result<()> {
        let rgb = self.read_graded(buffer, size)?;
        write_radiance(&path.with_extension("hdr"), size, &rgb)?;
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let paper_white = self.hdr.as_ref().map_or(200.0, |h| h.paper_white);
        hdr_capture::write_pq_png(
            &path.with_file_name(format!("{stem}-hdr.png")),
            size,
            &rgb,
            paper_white,
        )?;
        Ok(())
    }
    /// The graded light read back (`copy_graded`): linear BT.709, 1 = paper white.
    pub(super) fn read_graded(
        &self,
        buffer: &wgpu::Buffer,
        [width, height]: [u32; 2],
    ) -> Result<Vec<[f32; 3]>> {
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = sender.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| e.to_string())?;
        receiver
            .recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let row = (width * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) as usize;
        let mut rgb = Vec::with_capacity((width * height) as usize);
        {
            let data = buffer.get_mapped_range(..).map_err(|e| e.to_string())?;
            for y in 0..height as usize {
                for px in data[y * row..y * row + width as usize * 8]
                    .as_chunks::<8>()
                    .0
                {
                    let channel = |i: usize| {
                        let v = half_to_f32(u16::from_le_bytes([px[i * 2], px[i * 2 + 1]]));
                        if v.is_finite() { v.max(0.0) } else { 0.0 }
                    };
                    rgb.push([channel(0), channel(1), channel(2)]);
                }
            }
        }
        buffer.unmap();
        Ok(rgb)
    }
}
/// Linear light (1 = paper white) as a Radiance RGBE file, uncompressed.
pub(super) fn write_radiance(
    path: &std::path::Path,
    [width, height]: [u32; 2],
    rgb: &[[f32; 3]],
) -> Result<()> {
    use std::io::Write;
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(
        out,
        "#?RADIANCE\nFORMAT=32-bit_rle_rgbe\nEXPOSURE=1.0\n\n-Y {height} +X {width}\n"
    )?;
    for c in rgb {
        out.write_all(&rgbe(*c))?;
    }
    out.flush()?;
    Ok(())
}
/// An IEEE half float's value.
pub(super) fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((h >> 10) & 0x1f);
    let mantissa = f32::from(h & 0x3ff);
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        31 => {
            if mantissa == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}
/// A colour as Radiance's RGBE: three mantissas sharing the largest channel's exponent.
pub(super) fn rgbe([r, g, b]: [f32; 3]) -> [u8; 4] {
    let largest = r.max(g).max(b);
    if !largest.is_finite() || largest < 1e-32 {
        return [0; 4];
    }
    // largest = m * 2^e with m in [0.5, 1).
    let e = largest.log2().floor() as i32 + 1;
    let scale = 256.0 / 2f32.powi(e);
    let byte = |v: f32| (v * scale).clamp(0.0, 255.0) as u8;
    [byte(r), byte(g), byte(b), (e + 128).clamp(0, 255) as u8]
}
