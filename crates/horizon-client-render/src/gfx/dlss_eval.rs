//! DLSS Ray Reconstruction's evaluation on the desktop window (docs/rt-dlss-rr.md, stages 4 to 6),
//! behind the `dlss-rr` feature and `HORIZON_RT_DLSS_RR=1`: the frame state around horizon-rtx's
//! `dlss_rr::Session`.
//!
//! On a frame RR takes (`Renderer::render`'s `rr_now`), the window's view is traced raw (the
//! kernel's sample of this frame alone, `ShadowRays::set_raw_view`), the world is drawn with this
//! module's jitter, and where the TAA resolve would run, the session evaluates RR on the scene, the
//! world's depth and the RT guides (`rt_guides`), writing `output`, which is then drawn back over
//! the scene: bloom, the tone mapping, the hand and the HUD follow as after the resolve. Any NGX
//! failure turns RR off for the rest of the run (the caller drops the session).
//!
//! In an upscaling mode (`HORIZON_DLSS_RR_QUALITY`, stage 6) the world is drawn at the session's
//! render size (the world post's target, `world_post::Post::set_render_size`), and `output`, at
//! the window's size, is drawn onto the window's scene in place of the world post's copy.
//!
//! The guides are made just before the evaluation from the world's own depth and this module's
//! jitter (`ShadowRays::run_guides`), with the entities' own motion drawn over that depth by
//! frame generation's `ObjectMotion`, which this module feeds every frame RR runs.

use glam::{DVec3, Mat4};

/// `HORIZON_DLSS_RR_SHOW=input`: the scene keeps the raw noisy input RR was given (RR still runs,
/// its output unused), to tell a fault in the input from one in RR.
pub fn show_input() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("HORIZON_DLSS_RR_SHOW").is_ok_and(|v| v.trim().eq_ignore_ascii_case("input"))
    })
}

/// `HORIZON_DLSS_RR_JITTER=0`: the world is drawn without a jitter (and RR told so), to tell what
/// the jitter does from what RR does.
fn jitter_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("HORIZON_DLSS_RR_JITTER").is_ok_and(|v| v == "0"))
}

/// `HORIZON_DLSS_RR_JITTER_SIGN=x|y|xy`: the axes whose jitter NGX is given negated. Horizon's
/// jitter is how far the image moves, right and down; NGX's sign convention against that is
/// unverified (docs/rt-dlss-rr.md §9): with the wrong sign a still camera's image swims.
fn jitter_sign() -> [f32; 2] {
    static SIGN: std::sync::OnceLock<[f32; 2]> = std::sync::OnceLock::new();
    *SIGN.get_or_init(|| {
        let v = std::env::var("HORIZON_DLSS_RR_JITTER_SIGN")
            .unwrap_or_default()
            .to_ascii_lowercase();
        [
            if v.contains('x') { -1.0 } else { 1.0 },
            if v.contains('y') { -1.0 } else { 1.0 },
        ]
    })
}

/// The jitter's positions before it repeats at DLAA: NVIDIA's RR guide asks for at least 32 (its
/// Programming Guide's minimum is 8 at DLAA), Halton(2, 3) as DLSS was trained on. Upscaling takes
/// more (`horizon_rtx::dlss_rr::jitter_phases`).
const PHASES: u32 = 32;

/// The camera moving farther than this in a frame is a cut (a teleport, a dimension change): RR's
/// history is reset, as TAA's is (`taa::CUT_DISTANCE`).
const CUT_DISTANCE: f64 = 8.0;

/// The window's RR state between frames.
#[derive(Default)]
pub struct DlssEval {
    /// RR's output (a storage image at the session's output size) and its view, which also stands
    /// for the last resolved frame (the screen-space reflections' `scene`, TAA's `last_color`; they
    /// read it by screen position, whatever its size).
    output: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// The output drawn back over the scene.
    blitter: Option<wgpu::util::TextureBlitter>,
    /// Frames evaluated: where the jitter is in its sequence.
    frame: u32,
    /// The jitter's phases (`set_sizes`; 0 before: `PHASES`).
    phases: u32,
    /// The camera of the last frame evaluated and its unjittered view projection; none after a
    /// frame without RR (a reset next).
    last_camera: Option<DVec3>,
    last_view_proj: Option<Mat4>,
    /// The entities' own motion vectors (`track_entities`, `object_motion`).
    objects: Option<super::frame_gen::motion::ObjectMotion>,
    last_time: Option<std::time::Instant>,
    /// Whether the sun pass's bounce pass was told never to rest (`SunPass::set_never_rest`).
    pub never_rest: bool,
}

impl DlssEval {
    /// The jitter the world is drawn with this frame, in pixels, image moved right and down (as
    /// `taa::jitter`, Halton(2, 3), over `PHASES`).
    pub fn jitter(&self) -> [f32; 2] {
        if jitter_on() {
            super::taa::jitter_of(self.frame, self.phases.max(PHASES))
        } else {
            [0.0; 2]
        }
    }

    /// The session's render and output sizes this frame, before `jitter`: the jitter's phases
    /// grow with the upscaling (32 at DLAA, as before).
    pub fn set_sizes(&mut self, render: [u32; 2], output: [u32; 2]) {
        self.phases = horizon_rtx::dlss_rr::jitter_phases(render, output);
    }

    /// RR's last output, while frames are evaluated.
    pub fn last_output(&self) -> Option<wgpu::TextureView> {
        self.last_camera?;
        self.output.as_ref().map(|(_, v)| v.clone())
    }

    /// A frame went without RR: the next evaluation resets the history.
    pub fn skipped(&mut self) {
        self.last_camera = None;
        self.last_view_proj = None;
        self.last_time = None;
        if let Some(objects) = &mut self.objects {
            objects.reset();
        }
    }

    /// This frame's entity vertices (relative to `camera`) and their entity ids
    /// (`Frame::entity_motion`), matched to last frame's for their own motion; `atlas`: the
    /// entities' textures. Every frame RR runs, before `object_motion`.
    pub fn track_entities(
        &mut self,
        device: &wgpu::Device,
        atlas: &wgpu::TextureView,
        entities: &[crate::mesher::Vertex],
        spans: &[(u32, u64)],
        camera: DVec3,
    ) {
        self.objects
            .get_or_insert_with(|| {
                super::frame_gen::motion::ObjectMotion::new(device, atlas.clone())
            })
            .track(entities, spans, camera);
    }

    /// The entities' own motion over the world's `depth` (drawn with `jittered`; `view_proj` the
    /// same without the jitter, from `camera`), for the guides: none on a frame after a reset or
    /// without tracked entities.
    #[allow(clippy::too_many_arguments)]
    pub fn object_motion(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        jittered: Mat4,
        view_proj: Mat4,
        camera: DVec3,
    ) -> Option<wgpu::TextureView> {
        let (last, last_camera) = self.last_view_proj.zip(self.last_camera)?;
        let objects = self.objects.as_mut()?;
        // Last frame's projection from this frame's camera, where the tracked vertices' last
        // positions are.
        let previous = last * Mat4::from_translation((camera - last_camera).as_vec3());
        let size = [depth.texture().width(), depth.texture().height()];
        Some(objects.draw(
            device, queue, encoder, depth, size, jittered, view_proj, previous,
        ))
    }

    fn output(&mut self, device: &wgpu::Device, size: [u32; 2]) -> wgpu::TextureView {
        if self
            .output
            .as_ref()
            .is_none_or(|(t, _)| [t.width(), t.height()] != size)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("DLSS Ray Reconstruction output"),
                size: wgpu::Extent3d {
                    width: size[0].max(1),
                    height: size[1].max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: super::tonemap::SCENE_FORMAT,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.output = Some((texture, view));
        }
        self.output.as_ref().expect("made above").1.clone()
    }

    /// Evaluates RR for this frame over `color` (the world's scene) and draws its output onto
    /// `target` unless `show_input`: `color` itself at DLAA, whose session follows `color`'s size;
    /// the window's scene when the session upscales (its sizes set by the caller, `resize` with
    /// the window's). `Err` is NGX's (or a missing or mis-sized input): turn RR off.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate(
        &mut self,
        session: &mut horizon_rtx::dlss_rr::Session,
        device: &wgpu::Device,
        adapter: &wgpu::Adapter,
        encoder: &mut wgpu::CommandEncoder,
        split: &mut Vec<wgpu::CommandBuffer>,
        guides: &super::rt_guides::Guides,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        target: &wgpu::TextureView,
        camera: DVec3,
        view_proj: Mat4,
    ) -> Result<(), horizon_rtx::dlss_rr::Error> {
        if !session.upscales() {
            session.resize([color.texture().width(), color.texture().height()]);
        }
        let output = self.output(device, session.output_size());
        let reset = self
            .last_camera
            .is_none_or(|last| (camera - last).length() > CUT_DISTANCE);
        let now = std::time::Instant::now();
        let frame_time_ms = self
            .last_time
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32() * 1000.0);
        let sign = jitter_sign();
        let jitter = self.jitter();
        session.evaluate(
            encoder,
            split,
            &horizon_rtx::dlss_rr::Frame {
                adapter,
                color,
                depth,
                motion: &guides.motion,
                diffuse_albedo: &guides.diffuse_albedo,
                specular_albedo: &guides.specular_albedo,
                normals: &guides.normals,
                output: &output,
                jitter: [jitter[0] * sign[0], jitter[1] * sign[1]],
                reset,
                frame_time_ms,
            },
        )?;
        if !show_input() {
            let blitter = self.blitter.get_or_insert_with(|| {
                wgpu::util::TextureBlitter::new(device, super::tonemap::SCENE_FORMAT)
            });
            blitter.copy(device, encoder, &output, target);
        }
        self.frame = self.frame.wrapping_add(1);
        self.last_camera = Some(camera);
        self.last_view_proj = Some(view_proj);
        self.last_time = Some(now);
        Ok(())
    }
}
