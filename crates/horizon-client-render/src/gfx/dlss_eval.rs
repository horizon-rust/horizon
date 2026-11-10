//! DLSS Ray Reconstruction's evaluation on the desktop window (docs/rt-dlss-rr.md, stages 4 and 5),
//! behind the `dlss-rr` feature and `HORIZON_RT_DLSS_RR=1`: the frame state around horizon-rtx's
//! `dlss_rr::Session`.
//!
//! On a frame RR takes (`Renderer::render`'s `rr_now`), the window's view is traced raw (the
//! kernel's sample of this frame alone, `ShadowRays::set_raw_view`), the world is drawn with this
//! module's jitter, and where the TAA resolve would run, the session evaluates RR on the scene, the
//! world's depth and the RT guides (`rt_guides`), writing `output`, which is then drawn back over
//! the scene: bloom, the tone mapping, the hand and the HUD follow as after the resolve. Any NGX
//! failure turns RR off for the rest of the run (the caller drops the session).

use glam::DVec3;

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

/// The camera moving farther than this in a frame is a cut (a teleport, a dimension change): RR's
/// history is reset, as TAA's is (`taa::CUT_DISTANCE`).
const CUT_DISTANCE: f64 = 8.0;

/// The window's RR state between frames.
#[derive(Default)]
pub struct DlssEval {
    /// RR's output (a storage image at the world's size) and its view, which also stands for the
    /// last resolved frame (the screen-space reflections' `scene`, TAA's `last_color`).
    output: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// The output drawn back over the scene.
    blitter: Option<wgpu::util::TextureBlitter>,
    /// Frames evaluated: where the jitter is in its sequence.
    frame: u32,
    /// The camera of the last frame evaluated; none after a frame without RR (a reset next).
    last_camera: Option<DVec3>,
    last_time: Option<std::time::Instant>,
    /// Whether the sun pass's bounce pass was told never to rest (`SunPass::set_never_rest`).
    pub never_rest: bool,
}

impl DlssEval {
    /// The jitter the world is drawn with this frame, in pixels, image moved right and down (as
    /// `taa::jitter`, Halton(2, 3) over 16 phases, which also covers DLSS's 8 phases at DLAA).
    pub fn jitter(&self) -> [f32; 2] {
        if jitter_on() {
            super::taa::jitter(self.frame)
        } else {
            [0.0; 2]
        }
    }

    /// RR's last output, while frames are evaluated.
    pub fn last_output(&self) -> Option<wgpu::TextureView> {
        self.last_camera?;
        self.output.as_ref().map(|(_, v)| v.clone())
    }

    /// A frame went without RR: the next evaluation resets the history.
    pub fn skipped(&mut self) {
        self.last_camera = None;
        self.last_time = None;
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

    /// Evaluates RR for this frame over `color` (the scene, `size`), which then holds RR's output
    /// unless `show_input`. `Err` is NGX's (or a missing or mis-sized input): turn RR off.
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
        camera: DVec3,
    ) -> Result<(), horizon_rtx::dlss_rr::Error> {
        let size = [color.texture().width(), color.texture().height()];
        session.resize(size);
        let output = self.output(device, size);
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
            blitter.copy(device, encoder, &output, color);
        }
        self.frame = self.frame.wrapping_add(1);
        self.last_camera = Some(camera);
        self.last_time = Some(now);
        Ok(())
    }
}
