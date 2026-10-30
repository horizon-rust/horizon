//! EXPERIMENTAL VR through OpenXR (research spike), only with `HORIZON_XR=1` on Vulkan.
//!
//! The device comes from the runtime (`horizon_xr`), so the renderer draws the headset's eyes
//! with its own pipelines: each frame, `render` first draws the world once per eye into that
//! eye's swapchain image (the offscreen path, at the headset's size, with the eye's
//! projection, without the HUD), then the window as usual, then the HUD alone onto a panel
//! the runtime shows in front of the player. The head aims: the game turns the player by
//! the head's yaw and takes its pitch (`xr_look`), and the eyes draw the world from the
//! body's yaw with the head's pose on top. With eye tracking `xr_look` also says where the
//! eyes look, for the game to target blocks with (`game/xr_gaze.rs`). The controllers come
//! in as a gamepad (`xr_pad`).
//! While a menu is open the panel stays where it opened, and a laser from the pointing hand
//! (another quad layer) shows where it meets the panel: the game moves its pointer there
//! and the trigger clicks (`xr_pointer`). On the title the eyes see the panorama as a scene
//! around the player (`backdrop`): fixed in the stage, with each eye's own view of it.
//! Not there yet: the hand, aiming with a controller in the world, room-scale movement.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use glam::{Mat4, Quat, Vec3};

use super::panorama::Backdrop;
use super::{Frame, Renderer, Result};
use crate::gamepad::{Button, PadState};

/// The session, shared with a renderer that reloads resources on the same device.
pub(super) type Shared = Arc<Mutex<horizon_xr::Xr>>;

/// The panel's width in metres, its distance, and how far (degrees) the head turns away
/// before it follows: it stays put while the player looks around it. While a menu is open it
/// doesn't follow at all.
const PANEL_WIDTH: f32 = 1.4;
const PANEL_DISTANCE: f32 = 1.5;
const PANEL_FOLLOW: f32 = 40.0;

/// The quad layers: the HUD and menus, then the laser over them.
const PANEL: usize = 0;
const LASER: usize = 1;
/// The laser's width, and its length when it misses the panel (metres).
const LASER_WIDTH: f32 = 0.006;
const LASER_REACH: f32 = 1.0;
/// The title's backdrop: the panorama painted onto a floor under where the head started and
/// a sphere around it this wide (metres). The floor is the stage's when the head started
/// above it, else vanilla's eye height below the head.
const BACKDROP_RADIUS: f32 = 30.0;
const EYE_HEIGHT: f32 = 1.62;
/// The laser's image: a soft-edged strip, brighter towards the far end.
const LASER_IMAGE: [u32; 2] = [16, 128];

/// The pointing hand in a menu (`Renderer::xr_pointer`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XrPointer {
    /// A hand is tracked and points.
    pub tracked: bool,
    /// Where its ray meets the panel, in window pixels; `None` when it misses.
    pub at: Option<[f32; 2]>,
    /// Its trigger, 0..1.
    pub trigger: f32,
    /// The left stick is clicked: put the panel in front of the head again.
    pub recentre: bool,
}

/// Where the head and the eyes aim (`Renderer::xr_look`), in vanilla's degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XrLook {
    /// The head's turn since the last call (yaw) and its pitch.
    pub turn: f32,
    pub pitch: f32,
    /// Where the eyes look: their yaw relative to the head's and their pitch. `None` without
    /// eye tracking or while the gaze isn't valid.
    pub gaze: Option<[f32; 2]>,
    /// The headset tracks the eyes.
    pub eye_tracking: bool,
}

/// What one eye's draw replaces in `draw_frame`.
#[derive(Clone, Copy)]
pub(super) struct EyeView {
    pub view_proj: Mat4,
    /// The frustum both eyes cull with: around the two of them, from the head.
    pub cull_view_proj: Mat4,
    /// The second eye: the first eye's visible sections stand (`cull_view_proj` is theirs).
    pub reuse_visibility: bool,
    /// The window's size, which the HUD is laid out for.
    pub hud_size: [u32; 2],
    /// `Globals::inverse_view`'s inverse for the sky's rays: the eye's rotation, with the
    /// off-axis frustum's centre folded in as a shear so both eyes see the same sky.
    pub view: Mat4,
    /// Half the frustum's width over half its height, and the tangent of half its height.
    pub aspect: f32,
    pub tan_half: f32,
    /// The title's backdrop from this eye.
    pub backdrop: Backdrop,
    /// How many views the frame draws (submissions in flight per frame).
    pub views: usize,
    /// Where the ray-traced shadows' rays go (`shadow_rays::Foveation`).
    pub foveation: super::shadow_rays::Foveation,
    /// The view's index for the rays' history: 1 and 2 the eyes, 3 and 4 the insets (0 the window).
    pub view_index: usize,
}

/// VR state the renderer keeps between frames.
#[derive(Default)]
pub(super) struct State {
    /// Where the head was on the first frame (stage space): the player's eye.
    origin: Option<Vec3>,
    /// The head's yaw and pitch (vanilla's degrees, yaw relative to the stage's forward) on
    /// the last frame drawn.
    head: Option<(f32, f32)>,
    /// The gaze on the last frame drawn: its yaw less the head's, and its pitch (`XrLook`).
    gaze: Option<[f32; 2]>,
    /// The head yaw the game has turned the player by so far (`xr_look`): the eyes draw the
    /// body at the player's yaw less this.
    applied_yaw: f32,
    /// The panel's yaw (radians, stage space) and centre.
    panel: Option<(f32, Vec3)>,
    /// The HUD the panel's last image shows (a hash of its vertices and size).
    panel_hud: Option<u64>,
    /// A menu is open (`xr_pointer`): the panel stays put and the laser shows.
    menu: bool,
    /// The hand that points (0 left, 1 right): the last one whose trigger was pulled.
    hand: usize,
    /// The controllers as last read (`xr_pad`).
    controls: horizon_xr::Controls,
    /// Where each hand's ray met the panel on the last frame (window pixels).
    hits: [Option<[f32; 2]>; 2],
    /// Each hand is tracked on the last frame.
    tracked: [bool; 2],
    /// The frames since the last timing line (`xr_frame_times`).
    times: Times,
    /// What this frame waited for (the headset's pacing, each view's waits, the window's
    /// image): `Renderer::last_wait` once the frame is drawn.
    pub(super) waited: Duration,
}

/// Frame times gathered for the log's timing line.
#[derive(Default)]
struct Times {
    cpu: Vec<f32>,
    gpu: f32,
    gpu_samples: u32,
}

/// The VR frames between timing lines in the log.
const TIMED_FRAMES: usize = 300;

/// Whether the player asked for VR (`HORIZON_XR=1`).
pub(super) fn wanted() -> bool {
    std::env::var_os("HORIZON_XR").is_some_and(|v| v == "1")
}

/// Opens the device through the OpenXR runtime. `None` (logged) leaves the usual device.
/// A runtime still starting (SteamVR launched by the loader, a streaming headset not yet
/// connected) fails at first, so it is tried for a while.
pub(super) fn open(
    request: impl Fn(&wgpu::Adapter) -> (wgpu::Features, wgpu::Limits),
) -> Option<horizon_xr::Xr> {
    // The same instance options as the usual Vulkan device: without wgpu's indirect draw
    // validation in release builds.
    let flags = super::instance_descriptor(wgpu::Backends::VULKAN).flags;
    // Foveated rendering (the quad views) only when asked: not yet tried in a headset.
    let foveated = std::env::var_os("HORIZON_XR_FOVEATED").is_some_and(|v| v == "1");
    let mut attempt = 1;
    loop {
        match horizon_xr::Xr::new("Horizon", flags, foveated, &request) {
            Ok(xr) => {
                log::info!(
                    "Horizon VR: {} on {}, {}x{} per eye (HORIZON_XR=1, experimental)",
                    xr.system_name,
                    xr.runtime,
                    xr.size[0],
                    xr.size[1]
                );
                match xr.inset_size {
                    Some([w, h]) => log::info!(
                        "Horizon VR: foveated, insets {w}x{h} {} (HORIZON_XR_FOVEATED=1)",
                        if xr.gaze_foveation {
                            "where the eyes look"
                        } else {
                            "fixed ahead"
                        }
                    ),
                    None if foveated => {
                        log::info!("Horizon VR: the runtime has no quad views: stereo")
                    }
                    None => {}
                }
                return Some(xr);
            }
            Err(e) if attempt < 10 && !e.0.starts_with("no OpenXR loader") => {
                log::info!("Horizon VR: {e}; trying again ({attempt}/10)");
                attempt += 1;
                std::thread::sleep(Duration::from_secs(2));
            }
            Err(e) => {
                log::warn!("Horizon VR: {e}; starting without VR");
                return None;
            }
        }
    }
}

fn vec(p: horizon_xr::openxr::Vector3f) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

fn quat(o: horizon_xr::openxr::Quaternionf) -> Quat {
    Quat::from_xyzw(o.x, o.y, o.z, o.w)
}

/// The yaw (radians, 0 looking down -z, positive turning left) a rotation faces.
fn facing(rotation: Quat) -> f32 {
    let f = rotation * Vec3::NEG_Z;
    (-f.x).atan2(-f.z)
}

/// One eye's view of the title's backdrop, centred where the head started (`origin`, stage
/// space): from the eye's own pose and field of view, so the scene stays put as the head
/// turns and both eyes see it from where they are.
fn backdrop(eye: &horizon_xr::Eye, origin: Vec3, depth_far: f32) -> Backdrop {
    let at = vec(eye.pose.position) - origin;
    let view = Mat4::from_rotation_translation(quat(eye.pose.orientation), at).inverse();
    let projection = Mat4::from_cols_array_2d(&horizon_xr::projection(eye.fov, 0.05, depth_far));
    Backdrop {
        inverse_view_proj: (projection * view).inverse(),
        eye: at,
        floor: if origin.y > 0.5 { origin.y } else { EYE_HEIGHT },
        radius: BACKDROP_RADIUS,
    }
}

/// A gaze direction (stage space) as vanilla's angles: its yaw less the head's (`forward`),
/// wrapped to ±180, and its pitch (positive down).
fn gaze_angles(dir: Vec3, forward: Vec3) -> [f32; 2] {
    let yaw = |d: Vec3| -(-d.x).atan2(-d.z).to_degrees();
    let turn = (yaw(dir) - yaw(forward) + 180.0).rem_euclid(360.0) - 180.0;
    [turn, -dir.y.clamp(-1.0, 1.0).asin().to_degrees()]
}

/// An aim pose's ray: where it starts and its direction (the pose's -z).
fn ray(pose: horizon_xr::openxr::Posef) -> (Vec3, Vec3) {
    (
        vec(pose.position),
        (quat(pose.orientation) * Vec3::NEG_Z).normalize(),
    )
}

/// Where a ray meets a quad's front: how far along it, and the point in the quad's image
/// (0..1, from the top left).
fn hit(from: Vec3, dir: Vec3, quad: horizon_xr::Quad) -> Option<(f32, [f32; 2])> {
    let rotation = quat(quad.pose.orientation);
    let centre = vec(quad.pose.position);
    let normal = rotation * Vec3::Z;
    let facing = dir.dot(normal);
    if facing > -1e-4 {
        return None;
    }
    let t = (centre - from).dot(normal) / facing;
    if t <= 0.0 {
        return None;
    }
    let local = rotation.inverse() * (from + dir * t - centre);
    let uv = [local.x / quad.size[0] + 0.5, 0.5 - local.y / quad.size[1]];
    uv.iter()
        .all(|c| (0.0..=1.0).contains(c))
        .then_some((t, uv))
}

/// The laser from `from` to `to` as a thin quad: its length along the ray, turned about it
/// to face the head (a quad is flat).
fn laser(from: Vec3, to: Vec3, head: Vec3) -> horizon_xr::Quad {
    let along = (to - from).normalize_or(Vec3::NEG_Z);
    let mid = (from + to) * 0.5;
    let towards = head - mid;
    let front = (towards - along * towards.dot(along))
        .try_normalize()
        .unwrap_or_else(|| along.any_orthonormal_vector());
    let side = along.cross(front);
    let o = Quat::from_mat3(&glam::Mat3::from_cols(side, along, front));
    horizon_xr::Quad {
        pose: horizon_xr::openxr::Posef {
            orientation: horizon_xr::openxr::Quaternionf {
                x: o.x,
                y: o.y,
                z: o.z,
                w: o.w,
            },
            position: horizon_xr::openxr::Vector3f {
                x: mid.x,
                y: mid.y,
                z: mid.z,
            },
        },
        size: [LASER_WIDTH, (to - from).length().max(1e-3)],
    }
}

impl Renderer {
    /// Draws both eyes of the headset's next frame, when it wants one; `finish_eyes` ends it.
    pub(super) fn render_eyes(&mut self, frame: &Frame) -> Result<Option<horizon_xr::Frame>> {
        let Some(shared) = self.xr.clone() else {
            return Ok(None);
        };
        let mut xr = shared.lock().map_err(|_| "VR session poisoned")?;
        if xr.exit || !xr.poll() {
            return Ok(None);
        }
        let paced = std::time::Instant::now();
        let begun = crate::spans::time("vr_wait", || xr.begin_frame());
        self.xr_state.waited = paced.elapsed();
        let Some(xr_frame) = begun? else {
            return Ok(None);
        };
        xr.foveate(&xr_frame);
        let head = quat(xr_frame.head.orientation);
        let forward = head * Vec3::NEG_Z;
        self.xr_state.head = Some((
            -facing(head).to_degrees(),
            -forward.y.clamp(-1.0, 1.0).asin().to_degrees(),
        ));
        self.xr_state.gaze = xr_frame
            .gaze
            .map(|g| gaze_angles(Vec3::from(g.direction), forward));
        let origin = *self
            .xr_state
            .origin
            .get_or_insert_with(|| vec(xr_frame.head.position));
        self.eye_lists = None;
        let body = crate::camera::view_matrix(frame.yaw - self.xr_state.applied_yaw, 0.0);
        // The eyes, then with the quad views each eye's inset, at their own sizes.
        let views: Vec<(horizon_xr::Eye, [u32; 2])> = xr_frame
            .eyes
            .iter()
            .map(|&e| (e, xr.size))
            .chain(
                xr_frame
                    .insets
                    .iter()
                    .flatten()
                    .map(|&e| (e, xr.inset_size.unwrap_or(xr.size))),
            )
            .collect();
        // One culling frustum from the head, as wide as both eyes together (the insets are
        // inside them).
        let [a, b] = xr_frame.eyes.map(|e| e.fov);
        let both = horizon_xr::openxr::Fovf {
            angle_left: a.angle_left.min(b.angle_left),
            angle_right: a.angle_right.max(b.angle_right),
            angle_up: a.angle_up.max(b.angle_up),
            angle_down: a.angle_down.min(b.angle_down),
        };
        let head_view =
            Mat4::from_rotation_translation(head, vec(xr_frame.head.position) - origin).inverse();
        let cull_view_proj =
            Mat4::from_cols_array_2d(&horizon_xr::projection(both, 0.05, frame.depth_far))
                * head_view
                * body;
        // Shader packs size their targets for one view: with two sizes a frame they would be
        // made again for every view, so the quad views draw without them for now.
        let quad = views.len() > 2;
        let packs = quad.then(|| (self.pack_post.take(), self.native.take()));
        let drawn = self.draw_views(frame, &xr, &xr_frame, &views, body, origin, cull_view_proj);
        if let Some((pack_post, native)) = packs {
            self.pack_post = pack_post;
            self.native = native;
        }
        drawn?;
        Ok(Some(xr_frame))
    }

    /// Each view of `xr_frame` into its swapchain image: the first culls and builds the draw
    /// lists, the others reuse them.
    #[allow(clippy::too_many_arguments)]
    fn draw_views(
        &mut self,
        frame: &Frame,
        xr: &horizon_xr::Xr,
        xr_frame: &horizon_xr::Frame,
        views: &[(horizon_xr::Eye, [u32; 2])],
        body: Mat4,
        origin: Vec3,
        cull_view_proj: Mat4,
    ) -> Result<()> {
        // Where the eyes look (or, without eye tracking, where the head faces), in the world's
        // camera-relative axes: the sharp zone of every eye's rays.
        let head = quat(xr_frame.head.orientation);
        let look = xr_frame
            .gaze
            .map_or(head * Vec3::NEG_Z, |g| Vec3::from(g.direction));
        let gaze = body.inverse().transform_vector3(look);
        for (i, (eye, size)) in views.iter().enumerate() {
            let rotation = quat(eye.pose.orientation);
            let head = Mat4::from_rotation_translation(rotation, vec(eye.pose.position) - origin)
                .inverse();
            let projection =
                Mat4::from_cols_array_2d(&horizon_xr::projection(eye.fov, 0.05, frame.depth_far));
            let (l, r) = (eye.fov.angle_left.tan(), eye.fov.angle_right.tan());
            let (d, u) = (eye.fov.angle_down.tan(), eye.fov.angle_up.tan());
            // The sky's ray for ndc (x, y) is (x * (r - l) / 2 + (r + l) / 2, ..., -1): the
            // centre offset is a shear of the symmetric ray.
            let shear = Mat4::from_cols(
                glam::Vec4::X,
                glam::Vec4::Y,
                glam::Vec4::new(-(r + l) * 0.5, -(u + d) * 0.5, 1.0, 0.0),
                glam::Vec4::W,
            );
            let view = shear.inverse() * Mat4::from_quat(rotation.inverse()) * body;
            let target = xr.eye_texture(xr_frame, i).clone();
            // With variable rate shading, this eye's passes are shaded coarser away from
            // where the eyes look.
            xr.foveate_eye(Some(i));
            let drawn = self.draw_eye(
                frame,
                target,
                *size,
                EyeView {
                    view_proj: projection * head * body,
                    cull_view_proj,
                    reuse_visibility: i > 0,
                    hud_size: [self.config.width, self.config.height],
                    view,
                    aspect: (r - l) / (u - d),
                    tan_half: (u - d) * 0.5,
                    backdrop: backdrop(eye, origin, frame.depth_far),
                    views: views.len(),
                    // The quad views' insets (after the two eyes) are the sharp zone.
                    foveation: super::shadow_rays::Foveation::eye((i < 2).then_some(gaze)),
                    view_index: i + 1,
                },
            );
            xr.foveate_eye(None);
            drawn?;
        }
        Ok(())
    }

    /// After the window's frame: the HUD (and any menu) onto the panel, placed in front of
    /// the player, then the frame goes to the runtime.
    pub(super) fn finish_eyes(
        &mut self,
        frame: &Frame,
        mut xr_frame: horizon_xr::Frame,
    ) -> Result<()> {
        let Some(shared) = self.xr.clone() else {
            return Ok(());
        };
        let mut xr = shared.lock().map_err(|_| "VR session poisoned")?;
        let size = [self.config.width, self.config.height];
        if xr.quad_sizes[PANEL] != size {
            xr.create_quad(PANEL, size, self.config.format)?;
        }
        // The HUD changes now and then: the panel is drawn again only when it did.
        let hud = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            bytemuck::cast_slice::<_, u8>(&frame.hud.vertices).hash(&mut h);
            size.hash(&mut h);
            h.finish()
        };
        let unchanged = self.xr_state.panel_hud == Some(hud) && xr.quad_ready(PANEL);
        if unchanged && !frame.hud.vertices.is_empty() {
            xr_frame.quads[PANEL] = Some(self.place_panel(&xr_frame, size));
        } else if !frame.hud.vertices.is_empty()
            && let Some(texture) = xr.quad_texture(PANEL, &mut xr_frame)?
        {
            self.xr_state.panel_hud = Some(hud);
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(self.config.format.remove_srgb_suffix()),
                ..Default::default()
            });
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("VR panel"),
                });
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("VR panel clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.hud.draw_range(
                &mut encoder,
                &view,
                &self.depth,
                frame.hud,
                size,
                0..frame.hud.vertices.len() as u32,
            );
            self.queue.submit([encoder.finish()]);
            xr_frame.quads[PANEL] = Some(self.place_panel(&xr_frame, size));
        }
        self.point(&mut xr, &mut xr_frame, size)?;
        xr.end_frame(xr_frame)?;
        Ok(())
    }

    /// Where each hand's ray meets the panel, and the pointing hand's laser while a menu is
    /// open.
    fn point(
        &mut self,
        xr: &mut horizon_xr::Xr,
        xr_frame: &mut horizon_xr::Frame,
        size: [u32; 2],
    ) -> Result<()> {
        let panel = xr_frame.quads[PANEL];
        let mut ends = [None; 2];
        for (i, hand) in xr_frame.hands.iter().enumerate() {
            self.xr_state.tracked[i] = hand.is_some();
            self.xr_state.hits[i] = None;
            let (Some(hand), Some(panel)) = (hand, panel) else {
                continue;
            };
            let (from, dir) = ray(*hand);
            if let Some((t, uv)) = hit(from, dir, panel) {
                self.xr_state.hits[i] = Some([uv[0] * size[0] as f32, uv[1] * size[1] as f32]);
                ends[i] = Some(from + dir * t);
            }
        }
        let hand = self.xr_state.hand;
        let Some(pose) = xr_frame.hands[hand].filter(|_| self.xr_state.menu) else {
            return Ok(());
        };
        if !xr.quad_ready(LASER) {
            self.draw_laser_image(xr, xr_frame)?;
        }
        let (from, dir) = ray(pose);
        let to = ends[hand].unwrap_or(from + dir * LASER_REACH);
        xr_frame.quads[LASER] = Some(laser(from, to, vec(xr_frame.head.position)));
        Ok(())
    }

    /// The laser's image, drawn once: the layer shows it from then on.
    fn draw_laser_image(
        &mut self,
        xr: &mut horizon_xr::Xr,
        xr_frame: &mut horizon_xr::Frame,
    ) -> Result<()> {
        if xr.quad_sizes[LASER] != LASER_IMAGE {
            xr.create_quad(LASER, LASER_IMAGE, self.config.format)?;
        }
        let Some(texture) = xr.quad_texture(LASER, xr_frame)? else {
            return Ok(());
        };
        let bgra = self.config.format.remove_srgb_suffix() == wgpu::TextureFormat::Bgra8Unorm;
        let [w, h] = LASER_IMAGE;
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                // Opaque in the middle, soft at the sides; faint at the hand (the bottom).
                let across = 1.0 - ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0).abs();
                let along = 1.0 - (y as f32 + 0.5) / h as f32;
                let a = (across * 2.0).min(1.0) * (0.25 + 0.65 * along);
                // Premultiplied: what the runtime expects without the unpremultiplied flag.
                let [r, g, b] = [190.0, 225.0, 255.0].map(|c: f32| (c * a) as u8);
                let alpha = (a * 255.0) as u8;
                pixels.extend(if bgra {
                    [b, g, r, alpha]
                } else {
                    [r, g, b, alpha]
                });
            }
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        // An empty pass leaves the image as a colour attachment, as OpenXR wants it released.
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.config.format.remove_srgb_suffix()),
            ..Default::default()
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("VR laser"),
            });
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("VR laser"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    /// The panel's place: in front of the head, below eye level, following the head's yaw
    /// once it has turned `PANEL_FOLLOW` degrees away, but never while a menu is open.
    fn place_panel(&mut self, xr_frame: &horizon_xr::Frame, size: [u32; 2]) -> horizon_xr::Quad {
        let head = quat(xr_frame.head.orientation);
        let at = vec(xr_frame.head.position);
        let yaw = facing(head);
        let away = |panel_yaw: f32| {
            use std::f32::consts::{PI, TAU};
            ((yaw - panel_yaw + PI).rem_euclid(TAU) - PI).abs()
        };
        let (panel_yaw, centre) = match self.xr_state.panel {
            Some((panel_yaw, centre))
                if self.xr_state.menu || away(panel_yaw) < PANEL_FOLLOW.to_radians() =>
            {
                (panel_yaw, centre)
            }
            _ => {
                let forward = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
                (yaw, at + forward * PANEL_DISTANCE - Vec3::Y * 0.15)
            }
        };
        self.xr_state.panel = Some((panel_yaw, centre));
        let o = Quat::from_rotation_y(panel_yaw);
        horizon_xr::Quad {
            pose: horizon_xr::openxr::Posef {
                orientation: horizon_xr::openxr::Quaternionf {
                    x: o.x,
                    y: o.y,
                    z: o.z,
                    w: o.w,
                },
                position: horizon_xr::openxr::Vector3f {
                    x: centre.x,
                    y: centre.y,
                    z: centre.z,
                },
            },
            size: [
                PANEL_WIDTH,
                PANEL_WIDTH * size[1] as f32 / size[0].max(1) as f32,
            ],
        }
    }

    /// The head's turn since the last call and its pitch, and where the eyes look, for the
    /// game to aim with; `None` outside VR or before the first frame.
    pub fn xr_look(&mut self) -> Option<XrLook> {
        let eye_tracking = self.xr.as_ref()?.lock().is_ok_and(|xr| xr.eye_gaze);
        let (yaw, pitch) = self.xr_state.head?;
        let turn = yaw - self.xr_state.applied_yaw;
        self.xr_state.applied_yaw = yaw;
        Some(XrLook {
            turn,
            pitch,
            gaze: self.xr_state.gaze,
            eye_tracking,
        })
    }

    /// Once per poll, with whether a menu is open: the pointing hand and where it points at
    /// the panel (the last frame's). A menu opening puts the panel in front of the head, and
    /// it stays there while the menu is open; outside VR `None`.
    pub fn xr_pointer(&mut self, menu: bool) -> Option<XrPointer> {
        self.xr.as_ref()?;
        let state = &mut self.xr_state;
        if menu && !state.menu {
            state.panel = None;
        }
        state.menu = menu;
        let c = state.controls;
        // The hand whose trigger is pulled alone takes the pointer.
        if c.left_trigger > 0.6 && c.right_trigger < 0.3 && state.tracked[0] {
            state.hand = 0;
        } else if (c.right_trigger > 0.6 && c.left_trigger < 0.3 && state.tracked[1])
            || !state.tracked[state.hand]
        {
            state.hand = 1;
        }
        if !state.tracked[state.hand] && state.tracked[0] {
            state.hand = 0;
        }
        let left = state.hand == 0;
        Some(XrPointer {
            tracked: state.tracked[state.hand],
            at: state.hits[state.hand],
            trigger: if left {
                c.left_trigger
            } else {
                c.right_trigger
            },
            recentre: c.left_stick_click,
        })
    }

    /// Once per VR frame drawn: the game's CPU time on it (without waiting) and the GPU times
    /// read back since the last frame (one per view, a few frames late). Every 300 frames the
    /// log says what a frame cost on each side, to tell whether VR waits for the CPU or the GPU.
    pub fn xr_frame_times(&mut self, cpu_ms: f32, gpu_ms: &[f32]) {
        let Some(shared) = &self.xr else {
            return;
        };
        let times = &mut self.xr_state.times;
        times.cpu.push(cpu_ms);
        times.gpu += gpu_ms.iter().sum::<f32>();
        times.gpu_samples += gpu_ms.len() as u32;
        if times.cpu.len() < TIMED_FRAMES {
            return;
        }
        let views = shared
            .lock()
            .map_or(2, |xr| if xr.inset_size.is_some() { 4 } else { 2 });
        let frames = times.cpu.len();
        times.cpu.sort_by(f32::total_cmp);
        let median = times.cpu[frames / 2];
        let p95 = times.cpu[(frames * 95 / 100).min(frames - 1)];
        let mean = times.cpu.iter().sum::<f32>() / frames as f32;
        // Each timed view's GPU time, scaled to a frame's views: a few go untimed.
        let per_view = times.gpu / times.gpu_samples.max(1) as f32;
        log::info!(
            "Horizon VR: {frames} frames, {views} views: cpu {mean:.2} ms a frame (median {median:.2}, p95 {p95:.2}), gpu {:.2} ms a frame ({per_view:.2} a view, {} of {} views timed)",
            per_view * views as f32,
            times.gpu_samples,
            frames * views,
        );
        *times = Times::default();
    }

    /// Puts the panel in front of the head again on the next frame.
    pub fn xr_recentre(&mut self) {
        self.xr_state.panel = None;
    }

    /// The controllers as a gamepad (Touch's layout onto Xbox's), outside VR `None`.
    pub fn xr_pad(&mut self) -> Option<PadState> {
        let xr = self.xr.as_ref()?.lock().ok()?;
        let c = xr.controls();
        self.xr_state.controls = c;
        let mut pad = PadState {
            buttons: 0,
            left: c.left_stick,
            right: c.right_stick,
            triggers: [c.left_trigger, c.right_trigger],
        };
        for (button, down) in [
            (Button::South, c.a),
            (Button::East, c.b),
            (Button::West, c.x),
            (Button::North, c.y),
            (Button::LeftBumper, c.left_grip),
            (Button::RightBumper, c.right_grip),
            (Button::LeftStick, c.left_stick_click),
            (Button::RightStick, c.right_stick_click),
            (Button::Start, c.menu),
        ] {
            pad.set(button, down);
        }
        Some(pad)
    }

    /// The window in VR: the left eye scaled onto it, and the HUD, instead of drawing the
    /// world a third time.
    pub(super) fn present_mirror(
        &mut self,
        frame: &Frame,
        xr_frame: &horizon_xr::Frame,
    ) -> Result<bool> {
        let Some(surface) = &self.surface else {
            return Ok(false);
        };
        let acquire = std::time::Instant::now();
        let acquired = surface.get_current_texture();
        self.xr_state.waited += acquire.elapsed();
        let surface_frame = match acquired {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated => {
                Self::configure(
                    &self.device,
                    surface,
                    &self.surface_config(),
                    self.hdr.as_ref(),
                );
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recreate_surface()?;
                return Ok(false);
            }
            _ => return Ok(false),
        };
        let format = self.config.format.remove_srgb_suffix();
        let eye = {
            let shared = self.xr.clone().ok_or("no VR session")?;
            let xr = shared.lock().map_err(|_| "VR session poisoned")?;
            xr.eye_texture(xr_frame, 0)
                .create_view(&wgpu::TextureViewDescriptor {
                    format: Some(format),
                    ..Default::default()
                })
        };
        let screen = surface_frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor {
                format: Some(format),
                ..Default::default()
            });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("VR mirror"),
            });
        let group = self.world_post.bind(&self.device, &eye);
        self.world_post
            .finish_with(&mut encoder, &screen, format, &group);
        self.hud.draw_range(
            &mut encoder,
            &screen,
            &self.depth,
            frame.hud,
            [self.config.width, self.config.height],
            0..frame.hud.vertices.len() as u32,
        );
        self.queue.submit([encoder.finish()]);
        self.queue.present(surface_frame);
        Ok(true)
    }

    /// One eye: `draw_frame`'s offscreen path into `target`, at the eye's size, with its view.
    fn draw_eye(
        &mut self,
        frame: &Frame,
        target: wgpu::Texture,
        size: [u32; 2],
        eye: EyeView,
    ) -> Result<bool> {
        let depth = match self.eye_depth.iter().find(|(s, _)| *s == size) {
            Some((_, depth)) => depth.clone(),
            None => {
                let mut config = self.config.clone();
                config.width = size[0];
                config.height = size[1];
                let depth = Self::depth(&self.device, &config);
                // Two sizes at most (the eyes and the insets); a new headset size replaces them.
                if self.eye_depth.len() >= 2 {
                    self.eye_depth.clear();
                }
                self.eye_depth.push((size, depth.clone()));
                depth
            }
        };
        let window = [self.config.width, self.config.height];
        let surface = self.surface.take();
        let offscreen = self.offscreen.replace(target);
        let hdr = self.hdr.take();
        let depth = std::mem::replace(&mut self.depth, depth);
        [self.config.width, self.config.height] = size;
        self.eye = Some(eye);
        let drawn = crate::spans::time("vr_eye_draw", || self.draw_frame(frame));
        self.xr_state.waited += self.last_wait;
        self.eye = None;
        [self.config.width, self.config.height] = window;
        self.depth = depth;
        self.hdr = hdr;
        self.offscreen = offscreen;
        self.surface = surface;
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(yaw: f32, centre: Vec3, size: [f32; 2]) -> horizon_xr::Quad {
        let o = Quat::from_rotation_y(yaw);
        horizon_xr::Quad {
            pose: horizon_xr::openxr::Posef {
                orientation: horizon_xr::openxr::Quaternionf {
                    x: o.x,
                    y: o.y,
                    z: o.z,
                    w: o.w,
                },
                position: horizon_xr::openxr::Vector3f {
                    x: centre.x,
                    y: centre.y,
                    z: centre.z,
                },
            },
            size,
        }
    }

    #[test]
    fn a_ray_meets_the_panel_in_image_coordinates_from_the_top_left() {
        let panel = quad(0.0, Vec3::new(0.0, 1.5, -1.5), [1.4, 0.8]);
        let (t, uv) = hit(Vec3::new(0.0, 1.5, 0.0), Vec3::NEG_Z, panel).expect("centre");
        assert!((t - 1.5).abs() < 1e-5 && (uv[0] - 0.5).abs() < 1e-5 && (uv[1] - 0.5).abs() < 1e-5);
        // Up and to the left of the centre: the image's top left quarter.
        let (_, uv) = hit(Vec3::new(-0.35, 1.7, 0.0), Vec3::NEG_Z, panel).expect("hit");
        assert!(
            (uv[0] - 0.25).abs() < 1e-5 && (uv[1] - 0.25).abs() < 1e-5,
            "{uv:?}"
        );
        // Away from it, past its edge, or from behind: no hit.
        assert!(hit(Vec3::new(0.0, 1.5, 0.0), Vec3::Z, panel).is_none());
        assert!(hit(Vec3::new(0.8, 1.5, 0.0), Vec3::NEG_Z, panel).is_none());
        assert!(hit(Vec3::new(0.0, 1.5, -3.0), Vec3::Z, panel).is_none());
        // A panel turned to the left (yaw 90 degrees) faces +x: its right is -z, so +z is
        // on the left of its image.
        let side = quad(
            std::f32::consts::FRAC_PI_2,
            Vec3::new(-1.5, 1.5, 0.0),
            [1.4, 0.8],
        );
        let (_, uv) = hit(Vec3::new(0.0, 1.5, 0.35), Vec3::NEG_X, side).expect("side");
        assert!((uv[0] - 0.25).abs() < 1e-4, "{uv:?}");
    }

    fn eye(yaw: f32, at: Vec3) -> horizon_xr::Eye {
        let o = Quat::from_rotation_y(yaw);
        horizon_xr::Eye {
            pose: horizon_xr::openxr::Posef {
                orientation: horizon_xr::openxr::Quaternionf {
                    x: o.x,
                    y: o.y,
                    z: o.z,
                    w: o.w,
                },
                position: horizon_xr::openxr::Vector3f {
                    x: at.x,
                    y: at.y,
                    z: at.z,
                },
            },
            // A real headset's left eye, wider towards the nose's other side.
            fov: horizon_xr::openxr::Fovf {
                angle_left: -1.05,
                angle_right: 0.78,
                angle_up: 0.78,
                angle_down: -0.87,
            },
        }
    }

    /// Where a stage point shows in an eye (ndc), and the cube direction that pixel samples.
    fn seen(eye: &horizon_xr::Eye, origin: Vec3, point: Vec3) -> ([f32; 2], Vec3) {
        let b = backdrop(eye, origin, 1000.0);
        let clip = b.inverse_view_proj.inverse() * (point - origin).extend(1.0);
        let ndc = [clip.x / clip.w, clip.y / clip.w];
        // `fs_backdrop`'s ray for that pixel.
        let on = b.inverse_view_proj * glam::Vec4::new(ndc[0], ndc[1], 0.5, 1.0);
        let ray = (on.truncate() / on.w - b.eye).normalize();
        (ndc, b.point(ray).normalize())
    }

    #[test]
    fn the_backdrop_stays_put_as_the_head_turns_and_each_eye_sees_it_from_where_it_is() {
        let origin = Vec3::new(0.3, 1.6, -0.2);
        // A point on the floor 3 m ahead, and one on the sphere far ahead and up.
        let near = Vec3::new(0.3, 0.0, -3.2);
        let far = origin + Vec3::new(0.0, 0.5, -1.0).normalize() * BACKDROP_RADIUS;
        for point in [near, far] {
            let (ndc, dir) = seen(&eye(0.0, origin), origin, point);
            assert!(ndc.iter().all(|c| c.abs() < 1.0), "{ndc:?} in view");
            assert!((dir - (point - origin).normalize()).length() < 1e-3);
            // Turned 20 degrees: the point moves across the image, the texel stays.
            let (turned, dir_turned) = seen(&eye(0.35, origin), origin, point);
            assert!(turned[0] - ndc[0] > 0.2, "{ndc:?} -> {turned:?}");
            assert!((dir_turned - dir).length() < 1e-3);
        }
        // The two eyes 64 mm apart: the near floor is seen from two places (it shifts much
        // more between them than the far sphere does), and still in the same texel.
        let [left, right] = [-0.032, 0.032].map(|x| eye(0.0, origin + Vec3::X * x));
        let shift = |point| {
            let (l, dl) = seen(&left, origin, point);
            let (r, dr) = seen(&right, origin, point);
            assert!((dl - dr).length() < 1e-3);
            l[0] - r[0]
        };
        let (near_shift, far_shift) = (shift(near), shift(far));
        assert!(
            near_shift > 5.0 * far_shift && far_shift > 0.0,
            "{near_shift} {far_shift}"
        );
    }

    #[test]
    fn the_backdrop_floor_is_the_stage_floor_or_eye_height_below_a_local_head() {
        let stage = backdrop(
            &eye(0.0, Vec3::new(0.0, 1.2, 0.0)),
            Vec3::new(0.0, 1.2, 0.0),
            1000.0,
        );
        assert_eq!(stage.floor, 1.2);
        let local = backdrop(&eye(0.0, Vec3::ZERO), Vec3::ZERO, 1000.0);
        assert_eq!(local.floor, EYE_HEIGHT);
        // Looking straight down from the centre meets the floor right below it.
        assert!((local.point(Vec3::NEG_Y) - Vec3::new(0.0, -EYE_HEIGHT, 0.0)).length() < 1e-4);
    }

    #[test]
    fn the_gaze_is_measured_from_the_head_in_vanillas_angles() {
        let ahead = |yaw: f32, pitch: f32| {
            let (y, p) = (yaw.to_radians(), pitch.to_radians());
            Vec3::new(y.sin() * p.cos(), p.sin(), -y.cos() * p.cos())
        };
        // Eyes 12 degrees right of the head and 10 up: vanilla's yaw grows to the right,
        // its pitch is positive looking down.
        let [turn, pitch] = gaze_angles(ahead(12.0, 10.0), ahead(0.0, 0.0));
        assert!(
            (turn - 12.0).abs() < 1e-3 && (pitch + 10.0).abs() < 1e-3,
            "{turn} {pitch}"
        );
        // Across the stage's back, the turn stays small.
        let [turn, _] = gaze_angles(ahead(-175.0, 0.0), ahead(175.0, -20.0));
        assert!((turn - 10.0).abs() < 1e-3, "{turn}");
    }

    #[test]
    fn the_laser_runs_along_the_ray_and_faces_the_head() {
        let from = Vec3::new(0.2, 1.2, -0.3);
        let to = Vec3::new(0.0, 1.45, -1.5);
        let head = Vec3::new(0.0, 1.6, 0.0);
        let q = laser(from, to, head);
        let o = quat(q.pose.orientation);
        let along = o * Vec3::Y;
        assert!((along - (to - from).normalize()).length() < 1e-4);
        assert!((q.size[1] - (to - from).length()).abs() < 1e-5);
        let centre = vec(q.pose.position);
        assert!((centre - (from + to) * 0.5).length() < 1e-5);
        assert!(
            (o * Vec3::Z).dot(head - centre) > 0.0,
            "its front faces the head"
        );
    }
}
