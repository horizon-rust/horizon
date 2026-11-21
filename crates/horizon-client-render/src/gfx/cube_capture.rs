//! A 360-degree capture of the world (`horizon-ctl panorama`, the client host's `panorama`): the
//! six 90-degree cube faces seen from one eye position, one face per frame, each drawn as a
//! world view (`world_views.rs`: its own camera, section list and frustum, so what lies behind
//! the player is drawn) into a target that is never presented. A world view is the world pass
//! alone: the sky, terrain, entities and block entities. The HUD, the hand, screens, chat,
//! overlays and mod GUI draws are other passes that world views never run, so the capture
//! contains none of them and the window's own frames are untouched.
//!
//! The faces are read back asynchronously (`map_async`, polled on later frames), so the render
//! thread never waits for the GPU; [`equirect`] and the PNG writing belong on a worker.

use super::world_views::Target;
use super::{Renderer, copy_frame, native_render, padded_row};
use glam::{DVec3, Vec3};
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// The world view id the capture's faces draw as; mods' views use ids of their own choosing, and
/// this one is not for them.
pub(super) const VIEW_ID: u64 = 0xC0BE_CA97_0000_0001;
/// The field of view of a cube face (vanilla's panoramic camera: `Camera.calculateFov` with
/// `panoramic` answers 90).
pub const FACE_FOV: f32 = 90.0;
/// A face's side in pixels, at least and at most (the largest world view target).
pub const MIN_SIZE: u32 = 16;
pub const MAX_SIZE: u32 = crate::render_views::MAX_WORLD_VIEW_SIDE;
/// Frames a face may be asked for without being drawn (a title screen, views a mod filled up)
/// before the capture gives up.
const PATIENCE: u32 = 600;

/// One cube face: its name (in file names) and the camera turn that looks at it, in the
/// player's own yaw and pitch (yaw 0 is south, +Z, and turning right adds; pitch -90 is up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    pub name: &'static str,
    pub yaw: f32,
    pub pitch: f32,
}

/// The faces in file order: the four horizontal ones east to north, then up and down.
pub const FACES: [Face; 6] = [
    Face {
        name: "south",
        yaw: 0.0,
        pitch: 0.0,
    },
    Face {
        name: "west",
        yaw: 90.0,
        pitch: 0.0,
    },
    Face {
        name: "north",
        yaw: 180.0,
        pitch: 0.0,
    },
    Face {
        name: "east",
        yaw: -90.0,
        pitch: 0.0,
    },
    Face {
        name: "up",
        yaw: 0.0,
        pitch: -90.0,
    },
    Face {
        name: "down",
        yaw: 0.0,
        pitch: 90.0,
    },
];

impl Face {
    /// The face's camera in the world: where it looks, its right and its up, as unit vectors.
    pub fn basis(self) -> (Vec3, Vec3, Vec3) {
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let forward = Vec3::new(
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        let right = Vec3::new(-yaw.cos(), 0.0, -yaw.sin());
        (forward, right, right.cross(forward))
    }
}

/// The finished capture: six `size` by `size` RGBA8 faces, top row first, in [`FACES`] order.
#[derive(Debug)]
pub struct CubeCapture {
    pub size: u32,
    /// The player's yaw when the capture began: the middle of an equirectangular picture.
    pub center_yaw: f32,
    /// The eye position all six faces were seen from.
    pub eye: [f64; 3],
    pub faces: [Vec<u8>; 6],
}

/// The six faces as one equirectangular picture of `width` by `height` RGBA8 pixels: longitude
/// across (the middle column is `center_yaw`, turning right goes right) and latitude down (the
/// top row is straight up), each pixel bilinearly sampled from the face its direction lies in.
pub fn equirect(
    faces: &[Vec<u8>; 6],
    size: u32,
    center_yaw: f32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let bases = FACES.map(Face::basis);
    let n = size as i32;
    let texel = |face: usize, x: i32, y: i32| -> [f32; 4] {
        let (x, y) = (x.clamp(0, n - 1) as usize, y.clamp(0, n - 1) as usize);
        let at = (y * size as usize + x) * 4;
        std::array::from_fn(|c| f32::from(faces[face][at + c]))
    };
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for row in 0..height {
        let lat = (0.5 - (row as f32 + 0.5) / height as f32) * std::f32::consts::PI;
        for column in 0..width {
            let lon = ((column as f32 + 0.5) / width as f32 - 0.5) * std::f32::consts::TAU;
            let yaw = center_yaw.to_radians() + lon;
            let direction = Vec3::new(-yaw.sin() * lat.cos(), lat.sin(), yaw.cos() * lat.cos());
            // The face the direction is most along sees it inside its 90 degrees.
            let (face, ahead) = bases
                .iter()
                .map(|(forward, ..)| direction.dot(*forward))
                .enumerate()
                .fold(
                    (0, f32::MIN),
                    |best, (k, d)| if d > best.1 { (k, d) } else { best },
                );
            let (_, right, up) = bases[face];
            let u = direction.dot(right) / ahead;
            let v = direction.dot(up) / ahead;
            let x = (u + 1.0) / 2.0 * size as f32 - 0.5;
            let y = (1.0 - v) / 2.0 * size as f32 - 0.5;
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = (x - x0, y - y0);
            let (x0, y0) = (x0 as i32, y0 as i32);
            let corners = [
                (texel(face, x0, y0), (1.0 - fx) * (1.0 - fy)),
                (texel(face, x0 + 1, y0), fx * (1.0 - fy)),
                (texel(face, x0, y0 + 1), (1.0 - fx) * fy),
                (texel(face, x0 + 1, y0 + 1), fx * fy),
            ];
            for c in 0..4 {
                let sum: f32 = corners.iter().map(|(t, w)| t[c] * w).sum();
                out.push(sum.round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    out
}

/// How a capture is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Six PNGs, `<path without .png>_<face>.png`.
    Cube,
    /// One equirectangular PNG at `path`.
    Equirect,
}

/// The widest an equirectangular picture is made (twice as wide as it is tall).
pub const MAX_EQUIRECT_WIDTH: u32 = 8192;

/// A capture's files: the six faces' or the one equirectangular picture's.
pub fn files(path: &std::path::Path, format: Format) -> Vec<std::path::PathBuf> {
    let stem = path.with_extension("");
    match format {
        Format::Cube => FACES
            .iter()
            .map(|f| {
                let mut name = stem.clone().into_os_string();
                name.push(format!("_{}.png", f.name));
                name.into()
            })
            .collect(),
        Format::Equirect => vec![if path.extension().is_some() {
            path.to_path_buf()
        } else {
            path.with_extension("png")
        }],
    }
}

/// The picture's size for faces of `size`: four faces wide, two tall, at most
/// [`MAX_EQUIRECT_WIDTH`] wide.
pub fn equirect_size(size: u32) -> [u32; 2] {
    let width = (size * 4).min(MAX_EQUIRECT_WIDTH);
    [width, width / 2]
}

fn write_png(path: &std::path::Path, [w, h]: [u32; 2], rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(rgba))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes `capture` as `format` to `path` and returns the files, for a worker thread.
pub fn write(
    capture: &CubeCapture,
    path: &std::path::Path,
    format: Format,
) -> Result<Vec<std::path::PathBuf>, String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let files = files(path, format);
    match format {
        Format::Cube => {
            for (file, face) in files.iter().zip(&capture.faces) {
                write_png(file, [capture.size; 2], face)?;
            }
        }
        Format::Equirect => {
            let [w, h] = equirect_size(capture.size);
            let image = equirect(&capture.faces, capture.size, capture.center_yaw, w, h);
            write_png(&files[0], [w, h], &image)?;
        }
    }
    Ok(files)
}

/// `name` inside `root`, refused when it is not a plain relative path (a mod's destination;
/// the screenshot writer's rules).
pub fn destination(root: &std::path::Path, name: &str) -> Result<std::path::PathBuf, String> {
    super::screenshot::screenshot_destination(root, name).map_err(|e| e.to_string())
}

/// Why a capture cannot start.
pub fn check_size(size: u32, device_side: u32) -> Result<(), String> {
    if !(MIN_SIZE..=MAX_SIZE).contains(&size) {
        return Err(format!(
            "the panorama's face size must be {MIN_SIZE} to {MAX_SIZE} pixels, not {size}"
        ));
    }
    if size > device_side {
        return Err(format!(
            "this device's textures are at most {device_side} pixels a side, not {size}"
        ));
    }
    Ok(())
}

/// A face being read back.
struct Mapping {
    face: usize,
    buffer: wgpu::Buffer,
    bgra: bool,
    done: Receiver<Result<(), wgpu::BufferAsyncError>>,
}

/// The capture in progress. At most one runs at a time.
#[derive(Default)]
pub(super) struct Job {
    active: Option<Active>,
    /// The outcome, taken by `Renderer::take_cube_capture`.
    pub(super) result: Option<Result<CubeCapture, String>>,
}

struct Active {
    size: u32,
    center_yaw: f32,
    /// Where the first face was asked for: all six look from here.
    eye: Option<DVec3>,
    /// The next face to draw.
    next: usize,
    /// The face this frame drew, whose copy is read back once the frame is submitted.
    drawn: Option<(usize, wgpu::Buffer, bool)>,
    mappings: Vec<Mapping>,
    faces: [Option<Vec<u8>>; 6],
    /// Frames since the last face was drawn.
    waited: u32,
}

impl Job {
    pub(super) fn busy(&self) -> bool {
        self.active.is_some()
    }

    /// Whether this frame drew a face, which its submission must not defer.
    pub(super) fn has_drawn(&self) -> bool {
        self.active.as_ref().is_some_and(|a| a.drawn.is_some())
    }

    /// The face to draw this frame as a world view, from the eye the first one was asked from.
    pub(super) fn view(&mut self, camera: DVec3) -> Option<native_render::WorldView> {
        let active = self.active.as_mut()?;
        if active.next >= FACES.len() || active.drawn.is_some() {
            return None;
        }
        let eye = *active.eye.get_or_insert(camera);
        let face = FACES[active.next];
        Some(native_render::WorldView::new(
            VIEW_ID,
            eye,
            face.yaw,
            face.pitch,
            FACE_FOV,
            [active.size; 2],
        ))
    }

    fn fail(&mut self, error: String) {
        self.active = None;
        self.result = Some(Err(error));
    }
}

impl Renderer {
    /// Whether a capture is under way.
    pub fn cube_capture_busy(&self) -> bool {
        self.cube.busy()
    }

    /// Starts a capture of faces `size` pixels a side, the middle of an equirectangular picture
    /// being `center_yaw` (the player's yaw now). The faces draw on the next six frames.
    pub fn start_cube_capture(&mut self, size: u32, center_yaw: f32) -> Result<(), String> {
        if self.cube.busy() {
            return Err("a panorama is already being captured".into());
        }
        check_size(size, self.world_view_side)?;
        self.cube.result = None;
        self.cube.active = Some(Active {
            size,
            center_yaw,
            eye: None,
            next: 0,
            drawn: None,
            mappings: Vec::new(),
            faces: Default::default(),
            waited: 0,
        });
        Ok(())
    }

    /// The finished capture, or why it failed; once.
    pub fn take_cube_capture(&mut self) -> Option<Result<CubeCapture, String>> {
        self.cube.result.take()
    }

    /// The capture's face for this frame, asked with the mods' views (`view_sections::ask`).
    pub(super) fn cube_view(&mut self, camera: DVec3) -> Option<native_render::WorldView> {
        self.cube.view(camera)
    }

    /// The face just drawn into `target`: its copy is made in `encoder`, read back after the
    /// frame is submitted (`cube_submitted`).
    pub(super) fn cube_drawn(&mut self, encoder: &mut wgpu::CommandEncoder, target: &Target) {
        let Some(active) = self.cube.active.as_mut() else {
            return;
        };
        let bgra = match target.format().remove_srgb_suffix() {
            wgpu::TextureFormat::Bgra8Unorm => true,
            wgpu::TextureFormat::Rgba8Unorm => false,
            other => {
                let error = format!(
                    "the panorama needs an 8-bit frame, and this one is {other:?} (HDR on?)"
                );
                self.cube.fail(error);
                return;
            }
        };
        let buffer = copy_frame(&self.device, encoder, target.color(), [active.size; 2]);
        active.drawn = Some((active.next, buffer, bgra));
    }

    /// After a frame's submission: starts the face it drew reading back, and collects those
    /// that have been read. Never waits for the GPU.
    pub(super) fn cube_submitted(&mut self) {
        let Some(active) = self.cube.active.as_mut() else {
            return;
        };
        match active.drawn.take() {
            Some((face, buffer, bgra)) => {
                let (sender, done) = mpsc::channel();
                buffer.map_async(wgpu::MapMode::Read, .., move |r| {
                    let _ = sender.send(r);
                });
                active.mappings.push(Mapping {
                    face,
                    buffer,
                    bgra,
                    done,
                });
                active.next += 1;
                active.waited = 0;
            }
            None if active.next < FACES.len() => {
                active.waited += 1;
                if active.waited > PATIENCE {
                    self.cube
                        .fail("no world was drawn for the panorama's faces".into());
                    return;
                }
            }
            None => {}
        }
        if !active.mappings.is_empty() {
            let _ = self.device.poll(wgpu::PollType::Poll);
        }
        let size = active.size;
        let mut failed = None;
        active.mappings.retain_mut(|m| match m.done.try_recv() {
            Err(TryRecvError::Empty) => true,
            Ok(Ok(())) => {
                match read_face(&m.buffer, size, m.bgra) {
                    Ok(face) => active.faces[m.face] = Some(face),
                    Err(e) => failed = Some(e),
                }
                false
            }
            Ok(Err(e)) => {
                failed = Some(e.to_string());
                false
            }
            Err(TryRecvError::Disconnected) => {
                failed = Some("the panorama's readback stopped".into());
                false
            }
        });
        if let Some(error) = failed {
            self.cube.fail(error);
        } else if active.faces.iter().all(Option::is_some) {
            let faces = active
                .faces
                .each_mut()
                .map(|f| f.take().unwrap_or_default());
            let capture = CubeCapture {
                size,
                center_yaw: active.center_yaw,
                eye: active.eye.unwrap_or_default().to_array(),
                faces,
            };
            self.cube.active = None;
            self.cube.result = Some(Ok(capture));
        }
    }
}

/// A mapped face's RGBA8 pixels without the row padding.
fn read_face(buffer: &wgpu::Buffer, size: u32, bgra: bool) -> Result<Vec<u8>, String> {
    let (side, row) = (size as usize, padded_row(size) as usize);
    let mut out = Vec::with_capacity(side * side * 4);
    {
        let data = buffer.get_mapped_range(..).map_err(|e| e.to_string())?;
        for y in 0..side {
            for px in data[y * row..y * row + side * 4].as_chunks::<4>().0 {
                let (r, b) = if bgra { (px[2], px[0]) } else { (px[0], px[2]) };
                out.extend_from_slice(&[r, px[1], b, 255]);
            }
        }
    }
    buffer.unmap();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Vec3, b: [f32; 3]) {
        assert!(a.abs_diff_eq(Vec3::from(b), 1e-5), "{a:?} != {b:?}");
    }

    /// Each face looks at its own world axis, with its right and up as a camera there has them.
    #[test]
    fn faces_look_along_the_six_axes() {
        let expected = [
            // name, forward, right, up
            ("south", [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ("west", [-1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
            ("north", [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ("east", [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            ("up", [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            ("down", [0.0, -1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ];
        for (face, (name, forward, right, up)) in FACES.iter().zip(expected) {
            assert_eq!(face.name, name);
            let (f, r, u) = face.basis();
            near(f, forward);
            near(r, right);
            near(u, up);
        }
    }

    /// The faces' yaw and pitch, as the renderer's view matrix reads them, put the face's
    /// forward at -Z, its right at +X and its up at +Y.
    #[test]
    fn the_renderers_view_matrix_agrees_with_the_faces() {
        for face in FACES {
            let view = crate::camera::view_matrix_rolled(face.yaw, face.pitch, 0.0);
            let (f, r, u) = face.basis();
            near(view.transform_vector3(f), [0.0, 0.0, -1.0]);
            near(view.transform_vector3(r), [1.0, 0.0, 0.0]);
            near(view.transform_vector3(u), [0.0, 1.0, 0.0]);
        }
    }

    /// A face of one colour per quadrant, or one colour.
    fn solid(rgb: [u8; 3], size: u32) -> Vec<u8> {
        (0..size * size)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect()
    }
    fn colours() -> [Vec<u8>; 6] {
        [
            solid([10, 0, 0], 4), // south
            solid([20, 0, 0], 4), // west
            solid([30, 0, 0], 4), // north
            solid([40, 0, 0], 4), // east
            solid([50, 0, 0], 4), // up
            solid([60, 0, 0], 4), // down
        ]
    }
    fn red(image: &[u8], width: u32, x: u32, y: u32) -> u8 {
        image[((y * width + x) * 4) as usize]
    }

    /// An 8 by 4 picture centred on south: north at both edges, east on the left, south in the
    /// middle, west on the right, up on the top row and down on the bottom row.
    #[test]
    fn the_equirect_columns_run_round_the_horizon() {
        let image = equirect(&colours(), 4, 0.0, 8, 4);
        assert_eq!(image.len(), 8 * 4 * 4);
        let row: Vec<u8> = (0..8).map(|x| red(&image, 8, x, 1)).collect();
        assert_eq!(row, [30, 40, 40, 10, 10, 20, 20, 30]);
        let top: Vec<u8> = (0..8).map(|x| red(&image, 8, x, 0)).collect();
        assert_eq!(top, [50; 8]);
        let bottom: Vec<u8> = (0..8).map(|x| red(&image, 8, x, 3)).collect();
        assert_eq!(bottom, [60; 8]);
    }

    /// The middle of the picture is where the player looked.
    #[test]
    fn the_equirect_is_centred_on_the_players_yaw() {
        // facing west (yaw 90): west in the middle, north to its right, south to its left, east
        // behind
        let image = equirect(&colours(), 4, 90.0, 8, 4);
        let row: Vec<u8> = (0..8).map(|x| red(&image, 8, x, 1)).collect();
        assert_eq!(row, [40, 10, 10, 20, 20, 30, 30, 40]);
    }

    /// A face keeps its own orientation: looking a little right of south and a little up
    /// reads the top-right of the south face, and right of south is west.
    #[test]
    fn the_equirect_keeps_a_faces_orientation() {
        let size = 8u32;
        let mut south = Vec::new();
        for y in 0..size {
            for x in 0..size {
                let rgb = match (x >= size / 2, y >= size / 2) {
                    (false, false) => [1, 0, 0],
                    (true, false) => [2, 0, 0],
                    (false, true) => [3, 0, 0],
                    (true, true) => [4, 0, 0],
                };
                south.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        let mut faces = colours().map(|_| solid([99, 0, 0], size));
        faces[0] = south;
        let image = equirect(&faces, size, 0.0, 64, 32);
        // right and above the middle: top-right; left and above: top-left; below: bottom
        assert_eq!(red(&image, 64, 36, 14), 2);
        assert_eq!(red(&image, 64, 27, 14), 1);
        assert_eq!(red(&image, 64, 36, 17), 4);
        assert_eq!(red(&image, 64, 27, 17), 3);
    }

    #[test]
    fn the_face_size_is_checked() {
        assert!(check_size(1024, 8192).is_ok());
        assert_eq!(
            check_size(8, 8192).unwrap_err(),
            "the panorama's face size must be 16 to 4096 pixels, not 8"
        );
        assert_eq!(
            check_size(2048, 1024).unwrap_err(),
            "this device's textures are at most 1024 pixels a side, not 2048"
        );
    }

    /// The faces are asked for one at a time, six frames, from the first frame's eye.
    #[test]
    fn a_job_asks_for_one_face_a_frame_from_one_eye() {
        let mut job = Job {
            active: Some(Active {
                size: 64,
                center_yaw: 0.0,
                eye: None,
                next: 0,
                drawn: None,
                mappings: Vec::new(),
                faces: Default::default(),
                waited: 0,
            }),
            result: None,
        };
        let first = job.view(DVec3::new(1.0, 70.0, 2.0)).unwrap();
        assert_eq!(
            (first.yaw, first.pitch, first.fov_degrees),
            (0.0, 0.0, 90.0)
        );
        assert_eq!(first.size, [64, 64]);
        // asked again before the face is drawn: the same face, the same eye
        job.active.as_mut().unwrap().next = 4;
        let up = job.view(DVec3::new(50.0, 70.0, 50.0)).unwrap();
        assert_eq!(up.camera, DVec3::new(1.0, 70.0, 2.0));
        assert_eq!((up.yaw, up.pitch), (0.0, -90.0));
        job.active.as_mut().unwrap().next = 6;
        assert!(job.view(DVec3::ZERO).is_none());
    }
}
