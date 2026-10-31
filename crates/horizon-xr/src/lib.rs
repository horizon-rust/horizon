//! EXPERIMENTAL OpenXR for Horizon (research spike, off by default).
//!
//! Opens a session on the active OpenXR runtime (any runtime: SteamVR, Monado, Meta,
//! Virtual Desktop...) through `XR_KHR_vulkan_enable2`, and creates the wgpu device on the
//! Vulkan instance and device the runtime asks for, so Horizon's wgpu renderer can draw
//! straight into the headset's swapchain images. The OpenXR loader (`openxr_loader.dll`,
//! `libopenxr_loader.so`) is loaded at run time: without it, nothing here runs.
//!
//! When the runtime has `XR_EXT_eye_gaze_interaction` and the headset tracks the eyes, each
//! frame says where they look (`Frame::gaze`); without it the gaze is `None` and the head
//! aims as before.
//!
//! The unsafe interop lives in this crate because horizon-client forbids unsafe code.

use ash::vk::{self, Handle};
use openxr as xr;

mod controls;
mod foveation;
pub use controls::Controls;
/// The OpenXR types in this crate's API (poses, field of view).
pub use openxr;

const VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;
/// Each eye wide, then each eye's inset (`XR_VARJO_quad_views`).
const QUAD_VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_QUAD_VARJO;

/// What went wrong, for the log.
#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub(crate) fn err<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> Error + '_ {
    move |e| Error(format!("{what}: {e}"))
}

/// The extensions the active runtime offers, by name (needs the loader, not a headset).
pub fn runtime_extensions() -> Result<Vec<String>, Error> {
    // SAFETY: loads the system's OpenXR loader, as `Xr::new` does.
    let entry = unsafe { xr::Entry::load(&()) }.map_err(err("no OpenXR loader"))?;
    let available = entry
        .enumerate_extensions()
        .map_err(err("OpenXR extensions"))?;
    Ok(available
        .names()
        .into_iter()
        .map(|n| String::from_utf8_lossy(n.strip_suffix(&[0]).unwrap_or(n)).into_owned())
        .collect())
}

/// One eye this frame: where it is (stage space, metres, y up) and what it sees.
#[derive(Clone, Copy, Debug)]
pub struct Eye {
    pub pose: xr::Posef,
    pub fov: xr::Fovf,
}

/// How many quad layers there can be (`Xr::create_quad`'s `index`), drawn in that order.
pub const QUADS: usize = 2;

/// A flat panel the runtime composites in the world (`Xr::create_quad`): where its centre
/// is (stage space) and its width and height in metres. Its image's top edge is along +y and
/// its front faces +z.
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    pub pose: xr::Posef,
    pub size: [f32; 2],
}

/// Where the eyes look on a frame (`XR_EXT_eye_gaze_interaction`), while the gaze is valid.
/// A streamed headset may give it coarsely: the Vision Pro through KRVR moves it in steps of
/// about 12 degrees (docs/vr.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gaze {
    /// Its direction in the head's frame (the VIEW space: -z ahead, +x right, +y up), unit.
    pub view: [f32; 3],
    /// Where it starts and its direction in the stage space (the frame's head turned onto
    /// `view`), unit.
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    /// The runtime tracks the eyes now; `false` when it only holds the last gaze.
    pub tracked: bool,
}

/// A frame the headset wants drawn: draw each eye into `eye_texture`, then `end_frame`.
pub struct Frame {
    pub eyes: [Eye; 2],
    /// With the quad views, each eye's inset (`eye_texture` 2 and 3): a narrower view drawn
    /// sharper over the eye's own, where the eyes look with gaze foveation.
    pub insets: Option<[Eye; 2]>,
    /// The head (between the eyes, stage space).
    pub head: xr::Posef,
    /// The time the runtime will show this frame, for poses and animations.
    pub display_time: xr::Time,
    /// Where to show each panel drawn into `quad_texture` this frame; `None` hides it.
    pub quads: [Option<Quad>; QUADS],
    /// Where each hand (left, right) points: the aim pose (stage space), whose -z is the
    /// ray; `None` for a hand that isn't tracked.
    pub hands: [Option<xr::Posef>; 2],
    /// Where the eyes look; `None` without eye tracking or while the gaze isn't valid.
    pub gaze: Option<Gaze>,
    /// Each view's swapchain image: the eyes, then the insets.
    images: Vec<usize>,
    quad_images: [Option<usize>; QUADS],
}

/// One eye's swapchain and its images as wgpu textures.
struct EyeChain {
    swapchain: xr::Swapchain<xr::Vulkan>,
    images: Vec<wgpu::Texture>,
    /// An image has been released: a layer may show it again without a new one.
    released: bool,
}

pub struct Xr {
    // Field order is drop order: the session goes before the device it renders with.
    eyes: Vec<EyeChain>,
    /// The eyes' shading rate images (on in stereo, `HORIZON_XR_VRS=0` turns it off), before the device.
    rates: Option<foveation::Rates>,
    quads: [Option<EyeChain>; QUADS],
    actions: Option<controls::Actions>,
    view: xr::Space,
    stage: xr::Space,
    frame_stream: xr::FrameStream<xr::Vulkan>,
    frame_wait: xr::FrameWaiter,
    session: xr::Session<xr::Vulkan>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: wgpu::Adapter,
    pub instance: wgpu::Instance,
    xr: xr::Instance,
    blend: xr::EnvironmentBlendMode,
    view_type: xr::ViewConfigurationType,
    /// Each eye's recommended image size, in pixels.
    pub size: [u32; 2],
    /// Each inset's, with the quad views (`new`'s `foveated`).
    pub inset_size: Option<[u32; 2]>,
    /// The runtime puts the insets where the eyes look (`XR_VARJO_foveated_rendering`).
    pub gaze_foveation: bool,
    /// Each panel's image size, in pixels (`create_quad`).
    pub quad_sizes: [[u32; 2]; QUADS],
    pub runtime: String,
    pub system_name: String,
    /// The runtime has eye tracking (`XR_EXT_eye_gaze_interaction`, supported by the system).
    pub eye_gaze: bool,
    /// The gaze's pitch is turned the other way (`HORIZON_XR_GAZE_INVERT_PITCH=1`): KRVR's
    /// SteamVR driver gives it upside down.
    pub invert_gaze_pitch: bool,
    /// The fragment shading rate is on (stereo, unless `HORIZON_XR_VRS=0`): its attachment's texel size.
    shading_rate: Option<[u32; 2]>,
    running: bool,
    /// The runtime ended the session (headset off, SteamVR closed).
    pub exit: bool,
}

impl Xr {
    /// Opens the runtime, the Vulkan instance and device it asks for, and wgpu on top of
    /// them. `request` gets the adapter and returns the features and limits the device must
    /// have (the renderer's own request, which it can only work out from the adapter);
    /// `flags` are the instance's, as the renderer would create it.
    ///
    /// `foveated` asks for the quad views (`XR_VARJO_quad_views`) when the runtime offers
    /// them: each eye is then drawn twice, wide at a low resolution and an inset at a high
    /// one (`Frame::insets`, `inset_size`), and the runtime puts the insets where the eyes
    /// look when it can (`XR_VARJO_foveated_rendering`: `gaze_foveation`). Without them the
    /// session is stereo, as when `foveated` is false.
    pub fn new(
        app: &str,
        flags: wgpu::InstanceFlags,
        foveated: bool,
        request: impl FnOnce(&wgpu::Adapter) -> (wgpu::Features, wgpu::Limits),
    ) -> Result<Self, Error> {
        // SAFETY: loads the system's OpenXR loader; its entry points are trusted like a driver.
        let entry = unsafe { xr::Entry::load(&()) }.map_err(|e| {
            Error(format!(
                "no OpenXR loader ({e}): put openxr_loader.dll beside the game"
            ))
        })?;
        let available = entry
            .enumerate_extensions()
            .map_err(err("OpenXR extensions"))?;
        if !available.khr_vulkan_enable2 {
            return Err(Error(
                "the OpenXR runtime has no XR_KHR_vulkan_enable2".into(),
            ));
        }
        let mut extensions = xr::ExtensionSet::default();
        extensions.khr_vulkan_enable2 = true;
        extensions.ext_eye_gaze_interaction = available.ext_eye_gaze_interaction;
        // KRVR offers the quad views only to an instance that enables the extension.
        extensions.varjo_quad_views = foveated && available.varjo_quad_views;
        extensions.varjo_foveated_rendering =
            extensions.varjo_quad_views && available.varjo_foveated_rendering;
        let xr = entry
            .create_instance(
                &xr::ApplicationInfo {
                    application_name: app,
                    application_version: 0,
                    engine_name: "Horizon",
                    engine_version: 0,
                    api_version: xr::Version::new(1, 0, 0),
                },
                &extensions,
                &[],
                &(),
            )
            .map_err(err("xrCreateInstance"))?;
        let props = xr.properties().map_err(err("runtime properties"))?;
        let runtime = format!("{} {}", props.runtime_name, props.runtime_version);
        let system = xr
            .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
            .map_err(err("no headset (xrGetSystem)"))?;
        let system_name = xr
            .system_properties(system)
            .map(|p| p.system_name)
            .unwrap_or_default();
        // SAFETY: the instance has the extension whose property struct is chained.
        let eye_gaze =
            available.ext_eye_gaze_interaction && unsafe { supports_eye_gaze(&xr, system) };
        let quad = extensions.varjo_quad_views
            && xr
                .enumerate_view_configurations(system)
                .is_ok_and(|c| c.contains(&QUAD_VIEW_TYPE));
        let view_type = if quad { QUAD_VIEW_TYPE } else { VIEW_TYPE };
        // SAFETY: as above, for XR_VARJO_foveated_rendering.
        let gaze_foveation = quad
            && extensions.varjo_foveated_rendering
            && unsafe { supports_foveated_rendering(&xr, system) };
        let blend = xr
            .enumerate_environment_blend_modes(system, view_type)
            .map_err(err("blend modes"))?
            .first()
            .copied()
            .unwrap_or(xr::EnvironmentBlendMode::OPAQUE);
        let reqs = xr
            .graphics_requirements::<xr::Vulkan>(system)
            .map_err(err("Vulkan requirements"))?;
        // wgpu prefers Vulkan 1.3; take the newest the runtime allows, at least 1.1.
        let (api, api_xr) = if reqs.max_api_version_supported >= xr::Version::new(1, 3, 0) {
            (vk::make_api_version(0, 1, 3, 0), xr::Version::new(1, 3, 0))
        } else {
            (vk::make_api_version(0, 1, 1, 0), xr::Version::new(1, 1, 0))
        };
        if api_xr < reqs.min_api_version_supported {
            return Err(Error(format!(
                "the runtime needs Vulkan {} or newer",
                reqs.min_api_version_supported
            )));
        }
        // With gaze foveation the insets' sizes are the foveated ones (narrower, so smaller).
        let sizes = if gaze_foveation {
            // SAFETY: the instance has XR_VARJO_foveated_rendering.
            unsafe { foveated_sizes(&xr, system, view_type) }.map_err(err("views"))?
        } else {
            xr.enumerate_view_configuration_views(system, view_type)
                .map_err(err("views"))?
                .iter()
                .map(|v| {
                    [
                        v.recommended_image_rect_width,
                        v.recommended_image_rect_height,
                    ]
                })
                .collect()
        };
        let size = *sizes.first().ok_or_else(|| Error("no views".into()))?;
        let inset_size = sizes.get(2).copied().filter(|_| quad);
        // Variable rate shading steered by the gaze (`foveation.rs`), unless `HORIZON_XR_VRS=0`:
        // in stereo (the quad views foveate already), on Vulkan 1.2 or newer (`vkCreateRenderPass2`).
        let vrs = inset_size.is_none()
            && api >= vk::make_api_version(0, 1, 2, 0)
            && std::env::var_os("HORIZON_XR_VRS").is_none_or(|v| v != "0");

        // SAFETY: every handle is created below, from one Vulkan entry, with the extensions and
        // features wgpu-hal asks for: what `from_raw` and `device_from_raw` require.
        let (instance, adapter, device, queue, session, frame_wait, frame_stream, shading_rate) =
            unsafe { open_vulkan(&xr, system, api, flags, vrs, request)? };
        let stage = session
            .create_reference_space(xr::ReferenceSpaceType::STAGE, xr::Posef::IDENTITY)
            .or_else(|_| {
                session.create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)
            })
            .map_err(err("reference space"))?;
        let view = session
            .create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)
            .map_err(err("view space"))?;
        let actions = controls::Actions::new(&xr, &session, eye_gaze)
            .map_err(|e| log::warn!("Horizon VR: no controllers ({e})"))
            .ok();
        let invert_gaze_pitch =
            std::env::var_os("HORIZON_XR_GAZE_INVERT_PITCH").is_some_and(|v| v == "1");
        Ok(Self {
            eyes: Vec::new(),
            rates: None,
            quads: Default::default(),
            actions,
            view,
            stage,
            frame_stream,
            frame_wait,
            session,
            device,
            queue,
            adapter,
            instance,
            xr,
            blend,
            view_type,
            size,
            inset_size,
            gaze_foveation,
            quad_sizes: [[0, 0]; QUADS],
            runtime,
            system_name,
            eye_gaze,
            invert_gaze_pitch,
            shading_rate,
            running: false,
            exit: false,
        })
    }

    /// The swapchain formats the runtime offers (Vulkan `VkFormat` values), its preferred first.
    pub fn swapchain_formats(&self) -> Result<Vec<u32>, Error> {
        self.session
            .enumerate_swapchain_formats()
            .map_err(err("swapchain formats"))
    }

    /// Creates one swapchain per eye whose images the renderer draws into through views in
    /// `format`: an 8-bit format without sRGB (Horizon writes gamma-encoded colour, like
    /// vanilla). The images themselves are the sRGB twin, so the runtime reads them right.
    /// `Rgba16Float` and `Rgb10a2Unorm`, which have no sRGB twin, are taken as they are.
    pub fn create_swapchains(&mut self, format: wgpu::TextureFormat) -> Result<(), Error> {
        let (vk_format, srgb) = srgb_twin(format)?;
        let offered = self
            .session
            .enumerate_swapchain_formats()
            .map_err(err("swapchain formats"))?;
        if !offered.contains(&(vk_format.as_raw() as u32)) {
            return Err(Error(format!("the runtime offers no {srgb:?} swapchain")));
        }
        let mut eyes = Vec::new();
        for _ in 0..2 {
            eyes.push(self.chain(self.size, vk_format, srgb)?);
        }
        if let Some(size) = self.inset_size {
            for _ in 0..2 {
                eyes.push(self.chain(size, vk_format, srgb)?);
            }
        }
        self.eyes = eyes;
        if let Some(texel) = self.shading_rate {
            // SAFETY: the device has the fragment shading rate on, for `texel`.
            self.rates = unsafe { foveation::Rates::new(&self.device, self.size, texel) }
                .map_err(|e| log::warn!("Horizon VR: no variable rate shading ({e})"))
                .ok();
            if self.rates.is_some() {
                log::info!(
                    "Horizon VR: variable rate shading, {}x{} pixels a rate texel, sharp near the gaze (HORIZON_XR_VRS=0 turns it off)",
                    texel[0],
                    texel[1]
                );
            }
        }
        Ok(())
    }

    /// The eyes are shaded coarser away from where the eyes look (`HORIZON_XR_VRS=0` turns it off).
    pub fn variable_rate_shading(&self) -> bool {
        self.rates.is_some()
    }

    /// Before `frame`'s eyes are drawn: their shading rates for where the eyes look (ahead
    /// without a gaze).
    pub fn foveate(&mut self, frame: &Frame) {
        if let Some(rates) = &mut self.rates {
            let gaze = frame.gaze.map_or([0.0, 0.0, -1.0], |g| g.view);
            rates.update(&self.queue, frame.eyes.map(|e| e.fov), gaze);
        }
    }

    /// The render passes at the eyes' size take eye `eye`'s shading rate from now on; `None`
    /// stops it (the window, the panel).
    pub fn foveate_eye(&self, eye: Option<usize>) {
        if let Some(rates) = &self.rates {
            rates.attach(&self.device, eye);
        }
    }

    /// Panel `index` (below `QUADS`), of `size` pixels, for `Frame::quads`, in the eyes'
    /// format (`create_swapchains`). Made again, it starts with no image.
    pub fn create_quad(
        &mut self,
        index: usize,
        size: [u32; 2],
        format: wgpu::TextureFormat,
    ) -> Result<(), Error> {
        let (vk_format, srgb) = srgb_twin(format)?;
        self.quads[index] = Some(self.chain(size, vk_format, srgb)?);
        self.quad_sizes[index] = size;
        Ok(())
    }

    fn chain(
        &self,
        size: [u32; 2],
        vk_format: vk::Format,
        srgb: wgpu::TextureFormat,
    ) -> Result<EyeChain, Error> {
        let unorm = srgb.remove_srgb_suffix();
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let swapchain = self
            .session
            .create_swapchain(&xr::SwapchainCreateInfo {
                create_flags: xr::SwapchainCreateFlags::EMPTY,
                usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT
                    | xr::SwapchainUsageFlags::SAMPLED
                    | xr::SwapchainUsageFlags::TRANSFER_DST
                    | xr::SwapchainUsageFlags::MUTABLE_FORMAT,
                format: vk_format.as_raw() as u32,
                sample_count: 1,
                width: size[0],
                height: size[1],
                face_count: 1,
                array_size: 1,
                mip_count: 1,
            })
            .map_err(err("xrCreateSwapchain"))?;
        let images = swapchain
            .enumerate_images()
            .map_err(err("swapchain images"))?
            .into_iter()
            // SAFETY: the runtime made these images with the size, format, usage and
            // MUTABLE_FORMAT given above, and keeps them alive as long as the swapchain.
            .map(|raw| unsafe { self.wrap_image(raw, extent, srgb, unorm) })
            .collect::<Result<_, _>>()?;
        Ok(EyeChain {
            swapchain,
            images,
            released: false,
        })
    }

    unsafe fn wrap_image(
        &self,
        raw: u64,
        size: wgpu::Extent3d,
        format: wgpu::TextureFormat,
        view: wgpu::TextureFormat,
    ) -> Result<wgpu::Texture, Error> {
        let hal = {
            let hal_device = unsafe { self.device.as_hal::<wgpu_hal::api::Vulkan>() }
                .ok_or_else(|| Error("not a Vulkan device".into()))?;
            unsafe {
                hal_device.texture_from_raw(
                    vk::Image::from_raw(raw),
                    &wgpu_hal::TextureDescriptor {
                        label: Some("XR eye"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUses::COLOR_TARGET
                            | wgpu::TextureUses::RESOURCE
                            | wgpu::TextureUses::COPY_DST,
                        memory_flags: wgpu_hal::MemoryFlags::empty(),
                        view_formats: vec![view],
                    },
                    // The runtime owns the image: wgpu never frees it.
                    Some(Box::new(|| {})),
                    wgpu_hal::vulkan::TextureMemory::External,
                )
            }
        };
        Ok(unsafe {
            self.device
                .create_texture_from_hal::<wgpu_hal::api::Vulkan>(
                    hal,
                    &wgpu::TextureDescriptor {
                        label: Some("XR eye"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[view],
                    },
                    wgpu::TextureUses::UNINITIALIZED,
                )
        })
    }

    /// Handles the runtime's events (session start and stop). Returns whether frames should
    /// be drawn now; `exit` turns true when the runtime ends the session.
    pub fn poll(&mut self) -> bool {
        let mut buffer = xr::EventDataBuffer::new();
        while let Ok(Some(event)) = self.xr.poll_event(&mut buffer) {
            match event {
                xr::Event::SessionStateChanged(e) => match e.state() {
                    xr::SessionState::READY => {
                        if self.session.begin(self.view_type).is_ok() {
                            self.running = true;
                        }
                    }
                    xr::SessionState::STOPPING => {
                        let _ = self.session.end();
                        self.running = false;
                    }
                    xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
                        self.running = false;
                        self.exit = true;
                    }
                    _ => {}
                },
                xr::Event::InstanceLossPending(_) => self.exit = true,
                _ => {}
            }
        }
        self.running && !self.eyes.is_empty()
    }

    /// Waits for the headset's next frame (this paces the game in VR). `None`: the runtime
    /// wants nothing drawn this time (the frame is already ended).
    pub fn begin_frame(&mut self) -> Result<Option<Frame>, Error> {
        let state = self.frame_wait.wait().map_err(err("xrWaitFrame"))?;
        self.frame_stream.begin().map_err(err("xrBeginFrame"))?;
        if !state.should_render {
            self.frame_stream
                .end(state.predicted_display_time, self.blend, &[])
                .map_err(err("xrEndFrame"))?;
            return Ok(None);
        }
        let views: Vec<Eye> = if self.gaze_foveation {
            // SAFETY: the instance has XR_VARJO_foveated_rendering and the session the quad
            // views.
            unsafe { self.locate_foveated(state.predicted_display_time) }
                .map_err(err("xrLocateViews"))?
        } else {
            self.session
                .locate_views(self.view_type, state.predicted_display_time, &self.stage)
                .map_err(err("xrLocateViews"))?
                .1
                .iter()
                .map(|v| Eye {
                    pose: v.pose,
                    fov: v.fov,
                })
                .collect()
        };
        if views.len() < self.eyes.len() {
            return Err(Error(format!(
                "xrLocateViews: {} views for {} swapchains",
                views.len(),
                self.eyes.len()
            )));
        }
        let head = self
            .view
            .locate(&self.stage, state.predicted_display_time)
            .map(|l| l.pose)
            .unwrap_or(xr::Posef::IDENTITY);
        let mut images = Vec::with_capacity(self.eyes.len());
        for eye in &mut self.eyes {
            images.push(
                eye.swapchain
                    .acquire_image()
                    .map_err(err("xrAcquireSwapchainImage"))? as usize,
            );
            eye.swapchain
                .wait_image(xr::Duration::INFINITE)
                .map_err(err("xrWaitSwapchainImage"))?;
        }
        let hands = match &self.actions {
            Some(actions) => actions.aims(&self.session, &self.stage, state.predicted_display_time),
            None => [None; 2],
        };
        // The gaze is located in the head's frame, and turned into the stage with the head
        // located above: one location for both, so the pitch fix applies to both alike.
        let gaze = self
            .actions
            .as_ref()
            .and_then(|a| a.gaze(&self.session, &self.view, state.predicted_display_time))
            .map(|(pose, tracked)| gaze_from(head, pose, tracked, self.invert_gaze_pitch));
        Ok(Some(Frame {
            eyes: [views[0], views[1]],
            insets: (self.eyes.len() == 4).then(|| [views[2], views[3]]),
            head,
            display_time: state.predicted_display_time,
            quads: [None; QUADS],
            hands,
            gaze,
            images,
            quad_images: [None; QUADS],
        }))
    }

    /// The quad views located with `XrViewLocateFoveatedRenderingVARJO` chained: the runtime
    /// places the insets where the eyes look.
    unsafe fn locate_foveated(&self, time: xr::Time) -> Result<Vec<Eye>, xr::sys::Result> {
        use xr::sys;
        let foveated = sys::ViewLocateFoveatedRenderingVARJO {
            ty: sys::ViewLocateFoveatedRenderingVARJO::TYPE,
            next: std::ptr::null(),
            foveated_rendering_active: sys::TRUE,
        };
        let info = sys::ViewLocateInfo {
            ty: sys::ViewLocateInfo::TYPE,
            next: &foveated as *const _ as *const _,
            view_configuration_type: self.view_type,
            display_time: time,
            space: self.stage.as_raw(),
        };
        let mut state = sys::ViewState {
            ty: sys::ViewState::TYPE,
            next: std::ptr::null_mut(),
            view_state_flags: sys::ViewStateFlags::EMPTY,
        };
        let blank = sys::View {
            ty: sys::View::TYPE,
            next: std::ptr::null_mut(),
            pose: xr::Posef::IDENTITY,
            fov: xr::Fovf::default(),
        };
        let mut views = [blank; 4];
        let mut count = 0;
        let result = unsafe {
            (self.xr.fp().locate_views)(
                self.session.as_raw(),
                &info,
                &mut state,
                views.len() as u32,
                &mut count,
                views.as_mut_ptr(),
            )
        };
        if result.into_raw() < 0 {
            return Err(result);
        }
        Ok(views[..(count as usize).min(4)]
            .iter()
            .map(|v| Eye {
                pose: v.pose,
                fov: v.fov,
            })
            .collect())
    }

    /// Whether panel `index` has an image to show again without drawing a new one.
    pub fn quad_ready(&self, index: usize) -> bool {
        self.quads[index].as_ref().is_some_and(|c| c.released)
    }

    /// Panel `index`'s image for `frame` (acquired on the first call), when it was made.
    pub fn quad_texture(
        &mut self,
        index: usize,
        frame: &mut Frame,
    ) -> Result<Option<wgpu::Texture>, Error> {
        let Some(chain) = &mut self.quads[index] else {
            return Ok(None);
        };
        let image = match frame.quad_images[index] {
            Some(image) => image,
            None => {
                let image = chain
                    .swapchain
                    .acquire_image()
                    .map_err(err("xrAcquireSwapchainImage"))? as usize;
                chain
                    .swapchain
                    .wait_image(xr::Duration::INFINITE)
                    .map_err(err("xrWaitSwapchainImage"))?;
                frame.quad_images[index] = Some(image);
                image
            }
        };
        Ok(Some(chain.images[image].clone()))
    }

    /// The controllers, synced now (`Controls::default()` without them).
    pub fn controls(&self) -> Controls {
        match &self.actions {
            Some(actions) if self.running => actions.read(&self.session),
            _ => Controls::default(),
        }
    }

    /// The image one eye (0 left, 1 right) of `frame` is drawn into.
    pub fn eye_texture(&self, frame: &Frame, eye: usize) -> &wgpu::Texture {
        &self.eyes[eye].images[frame.images[eye]]
    }

    /// The eyes' image format (`create_swapchains`): an 8-bit sRGB one, or the format given.
    pub fn eye_format(&self) -> wgpu::TextureFormat {
        self.eyes[0].images[0].format()
    }

    /// Hands `frame` to the runtime. Call after the eyes' commands are submitted.
    pub fn end_frame(&mut self, frame: Frame) -> Result<(), Error> {
        for eye in &mut self.eyes {
            eye.swapchain
                .release_image()
                .map_err(err("xrReleaseSwapchainImage"))?;
        }
        // A panel not drawn this frame shows the image released last (OpenXR keeps it).
        for (chain, image) in self.quads.iter_mut().zip(frame.quad_images) {
            if let (Some(chain), Some(_)) = (chain, image) {
                chain
                    .swapchain
                    .release_image()
                    .map_err(err("xrReleaseSwapchainImage"))?;
                chain.released = true;
            }
        }
        // The eyes, then the insets, each its whole image.
        let located: Vec<Eye> = frame
            .eyes
            .iter()
            .chain(frame.insets.iter().flatten())
            .copied()
            .collect();
        let views: Vec<_> = located
            .iter()
            .zip(&self.eyes)
            .enumerate()
            .map(|(i, (eye, chain))| {
                let size = if i < 2 {
                    self.size
                } else {
                    self.inset_size.unwrap_or(self.size)
                };
                xr::CompositionLayerProjectionView::new()
                    .pose(eye.pose)
                    .fov(eye.fov)
                    .sub_image(
                        xr::SwapchainSubImage::new()
                            .swapchain(&chain.swapchain)
                            .image_rect(xr::Rect2Di {
                                offset: xr::Offset2Di { x: 0, y: 0 },
                                extent: xr::Extent2Di {
                                    width: size[0] as i32,
                                    height: size[1] as i32,
                                },
                            }),
                    )
            })
            .collect();
        let layer = xr::CompositionLayerProjection::new()
            .space(&self.stage)
            .views(&views);
        let panels: Vec<_> = (0..QUADS)
            .filter_map(|i| {
                let chain = self.quads[i].as_ref().filter(|c| c.released)?;
                Some((frame.quads[i]?, &chain.swapchain, self.quad_sizes[i]))
            })
            .map(|(q, swapchain, size)| {
                xr::CompositionLayerQuad::new()
                    .layer_flags(xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA)
                    .space(&self.stage)
                    .eye_visibility(xr::EyeVisibility::BOTH)
                    .pose(q.pose)
                    .size(xr::Extent2Df {
                        width: q.size[0],
                        height: q.size[1],
                    })
                    .sub_image(
                        xr::SwapchainSubImage::new()
                            .swapchain(swapchain)
                            .image_rect(xr::Rect2Di {
                                offset: xr::Offset2Di { x: 0, y: 0 },
                                extent: xr::Extent2Di {
                                    width: size[0] as i32,
                                    height: size[1] as i32,
                                },
                            }),
                    )
            })
            .collect();
        let mut layers: Vec<&xr::CompositionLayerBase<xr::Vulkan>> = vec![&layer];
        layers.extend(panels.iter().map(|p| &**p));
        self.frame_stream
            .end(frame.display_time, self.blend, &layers)
            .map_err(err("xrEndFrame"))
    }

    /// The OpenXR instance, for actions (controllers) later on.
    pub fn instance(&self) -> &xr::Instance {
        &self.xr
    }
}

/// `XrSystemEyeGazeInteractionPropertiesEXT::supportsEyeGazeInteraction`: the runtime has the
/// extension, but the headset may still have no eye tracking.
unsafe fn supports_eye_gaze(xr: &xr::Instance, system: xr::SystemId) -> bool {
    unsafe {
        let mut eyes = xr::sys::SystemEyeGazeInteractionPropertiesEXT {
            ty: xr::sys::SystemEyeGazeInteractionPropertiesEXT::TYPE,
            next: std::ptr::null_mut(),
            supports_eye_gaze_interaction: xr::sys::FALSE,
        };
        let mut p = xr::sys::SystemProperties {
            ty: xr::sys::SystemProperties::TYPE,
            next: &mut eyes as *mut _ as *mut _,
            ..std::mem::zeroed()
        };
        let result = (xr.fp().get_system_properties)(xr.as_raw(), system, &mut p);
        result == xr::sys::Result::SUCCESS && eyes.supports_eye_gaze_interaction.into()
    }
}

/// `XrSystemFoveatedRenderingPropertiesVARJO::supportsFoveatedRendering`: the runtime can
/// place the insets where the eyes look.
unsafe fn supports_foveated_rendering(xr: &xr::Instance, system: xr::SystemId) -> bool {
    unsafe {
        let mut foveated = xr::sys::SystemFoveatedRenderingPropertiesVARJO {
            ty: xr::sys::SystemFoveatedRenderingPropertiesVARJO::TYPE,
            next: std::ptr::null_mut(),
            supports_foveated_rendering: xr::sys::FALSE,
        };
        let mut p = xr::sys::SystemProperties {
            ty: xr::sys::SystemProperties::TYPE,
            next: &mut foveated as *mut _ as *mut _,
            ..std::mem::zeroed()
        };
        let result = (xr.fp().get_system_properties)(xr.as_raw(), system, &mut p);
        result == xr::sys::Result::SUCCESS && foveated.supports_foveated_rendering.into()
    }
}

/// Each view's recommended image size with `XrFoveatedViewConfigurationViewVARJO` chained
/// (`foveatedRenderingActive`): the insets' sizes for their foveated, narrower field.
unsafe fn foveated_sizes(
    xr: &xr::Instance,
    system: xr::SystemId,
    ty: xr::ViewConfigurationType,
) -> Result<Vec<[u32; 2]>, xr::sys::Result> {
    use xr::sys;
    let enumerate = xr.fp().enumerate_view_configuration_views;
    let mut count = 0;
    let result = unsafe { enumerate(xr.as_raw(), system, ty, 0, &mut count, std::ptr::null_mut()) };
    if result.into_raw() < 0 {
        return Err(result);
    }
    let mut foveated: Vec<_> = (0..count)
        .map(|_| sys::FoveatedViewConfigurationViewVARJO {
            ty: sys::FoveatedViewConfigurationViewVARJO::TYPE,
            next: std::ptr::null_mut(),
            foveated_rendering_active: sys::TRUE,
        })
        .collect();
    let mut views: Vec<_> = foveated
        .iter_mut()
        .map(|f| sys::ViewConfigurationView {
            ty: sys::ViewConfigurationView::TYPE,
            next: f as *mut _ as *mut _,
            // SAFETY: plain numbers the runtime fills in.
            ..unsafe { std::mem::zeroed() }
        })
        .collect();
    let result = unsafe {
        enumerate(
            xr.as_raw(),
            system,
            ty,
            count,
            &mut count,
            views.as_mut_ptr(),
        )
    };
    if result.into_raw() < 0 {
        return Err(result);
    }
    Ok(views
        .iter()
        .take(count as usize)
        .map(|v| {
            [
                v.recommended_image_rect_width,
                v.recommended_image_rect_height,
            ]
        })
        .collect())
}

/// `v` turned by `q`.
fn rotate(q: xr::Quaternionf, v: [f32; 3]) -> [f32; 3] {
    let u = [q.x, q.y, q.z];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    // v' = v + 2w(u x v) + 2u x (u x v)
    let t = cross(u, v).map(|c| c * 2.0);
    let c = cross(u, t);
    std::array::from_fn(|i| v[i] + q.w * t[i] + c[i])
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|c| c * c).sum::<f32>().sqrt();
    if length > 1e-6 {
        v.map(|c| c / length)
    } else {
        [0.0, 0.0, -1.0]
    }
}

/// The gaze from its pose in the head's frame (`view`) and the head's pose in the stage;
/// `invert_pitch` mirrors it up and down in the head's frame.
fn gaze_from(head: xr::Posef, view: xr::Posef, tracked: bool, invert_pitch: bool) -> Gaze {
    let mut dir = normalize(rotate(view.orientation, [0.0, 0.0, -1.0]));
    if invert_pitch {
        dir[1] = -dir[1];
    }
    let p = view.position;
    let offset = rotate(head.orientation, [p.x, p.y, p.z]);
    Gaze {
        view: dir,
        origin: [
            head.position.x + offset[0],
            head.position.y + offset[1],
            head.position.z + offset[2],
        ],
        direction: normalize(rotate(head.orientation, dir)),
        tracked,
    }
}

/// The sRGB swapchain format whose images the renderer writes through `format` views.
fn srgb_twin(format: wgpu::TextureFormat) -> Result<(vk::Format, wgpu::TextureFormat), Error> {
    match format.remove_srgb_suffix() {
        wgpu::TextureFormat::Bgra8Unorm => Ok((
            vk::Format::B8G8R8A8_SRGB,
            wgpu::TextureFormat::Bgra8UnormSrgb,
        )),
        wgpu::TextureFormat::Rgba8Unorm => Ok((
            vk::Format::R8G8B8A8_SRGB,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )),
        wgpu::TextureFormat::Rgba16Float => Ok((
            vk::Format::R16G16B16A16_SFLOAT,
            wgpu::TextureFormat::Rgba16Float,
        )),
        wgpu::TextureFormat::Rgb10a2Unorm => Ok((
            vk::Format::A2B10G10R10_UNORM_PACK32,
            wgpu::TextureFormat::Rgb10a2Unorm,
        )),
        other => Err(Error(format!("no headset swapchain for {other:?}"))),
    }
}

type Opened = (
    wgpu::Instance,
    wgpu::Adapter,
    wgpu::Device,
    wgpu::Queue,
    xr::Session<xr::Vulkan>,
    xr::FrameWaiter,
    xr::FrameStream<xr::Vulkan>,
    // The fragment shading rate's texel size, when it is on.
    Option<[u32; 2]>,
);

/// The Vulkan instance and device through the runtime (`xrCreateVulkan*KHR`), wgpu on them,
/// and the session. `vrs` turns the fragment shading rate on when the GPU has it.
unsafe fn open_vulkan(
    xr: &xr::Instance,
    system: xr::SystemId,
    api: u32,
    flags: wgpu::InstanceFlags,
    vrs: bool,
    request: impl FnOnce(&wgpu::Adapter) -> (wgpu::Features, wgpu::Limits),
) -> Result<Opened, Error> {
    let vk_entry = unsafe { ash::Entry::load() }.map_err(err("Vulkan loader"))?;
    let instance_extensions = wgpu_hal::vulkan::Instance::desired_extensions(&vk_entry, api, flags)
        .map_err(err("Vulkan instance extensions"))?;
    let names: Vec<*const std::ffi::c_char> =
        instance_extensions.iter().map(|e| e.as_ptr()).collect();
    let app_info = vk::ApplicationInfo::default()
        .application_name(c"Horizon")
        .engine_name(c"Horizon")
        .api_version(api);
    let create = vk::InstanceCreateInfo::default()
        .application_info(&app_info)
        .enabled_extension_names(&names);
    #[allow(clippy::missing_transmute_annotations)]
    let get_proc = unsafe { std::mem::transmute(vk_entry.static_fn().get_instance_proc_addr) };
    let raw_instance =
        unsafe { xr.create_vulkan_instance(system, get_proc, &create as *const _ as *const _) }
            .map_err(err("xrCreateVulkanInstanceKHR"))?
            .map_err(|e| Error(format!("vkCreateInstance: {}", vk::Result::from_raw(e))))?;
    let ash_instance = unsafe {
        ash::Instance::load(
            vk_entry.static_fn(),
            vk::Instance::from_raw(raw_instance as _),
        )
    };
    let hal_instance = unsafe {
        wgpu_hal::vulkan::Instance::from_raw(
            vk_entry,
            ash_instance.clone(),
            api,
            0,
            None,
            instance_extensions,
            flags,
            Default::default(),
            false,
            None,
        )
    }
    .map_err(err("wgpu instance"))?;
    let physical = vk::PhysicalDevice::from_raw(
        unsafe { xr.vulkan_graphics_device(system, raw_instance) }
            .map_err(err("the headset's GPU"))? as _,
    );
    let exposed = hal_instance
        .expose_adapter(physical)
        .ok_or_else(|| Error("wgpu can't use the headset's GPU".into()))?;
    // wgpu-core gives an instance made from wgpu-hal its default flags plus the environment's
    // (third_party/wgpu-core): say indirect draws aren't validated when `flags` says so.
    if !flags.contains(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL)
        && std::env::var_os("WGPU_VALIDATION_INDIRECT_CALL").is_none()
    {
        // SAFETY: set once while the device opens, before the renderer's threads read it.
        unsafe { std::env::set_var("WGPU_VALIDATION_INDIRECT_CALL", "0") };
    }
    let instance = unsafe { wgpu::Instance::from_hal::<wgpu_hal::api::Vulkan>(hal_instance) };
    let adapter = unsafe { instance.create_adapter_from_hal(exposed) };
    let (features, mut limits) = request(&adapter);
    // wgpu-core gives an instance made from wgpu-hal its default flags, whatever `flags` says,
    // so indirect draws are validated, and that validation asserts buffers stay under 4 GiB.
    limits.max_buffer_size = limits.max_buffer_size.min(u64::from(u32::MAX));
    let features = features & adapter.features();
    let family = unsafe { ash_instance.get_physical_device_queue_family_properties(physical) }
        .iter()
        .position(|f| {
            f.queue_flags
                .contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
        })
        .ok_or_else(|| Error("no graphics queue".into()))? as u32;
    let (open, raw_device, shading_rate) = {
        let hal_adapter = unsafe { adapter.as_hal::<wgpu_hal::api::Vulkan>() }
            .ok_or_else(|| Error("not a Vulkan adapter".into()))?;
        let mut device_extensions = hal_adapter.required_device_extensions(features);
        // SAFETY: the instance is the one the physical device came from, Vulkan 1.2 or newer
        // when `vrs` is.
        let shading_rate = vrs
            .then(|| unsafe { foveation::texel_size(&ash_instance, physical) })
            .flatten();
        if vrs && shading_rate.is_none() {
            log::warn!(
                "Horizon VR: the GPU has no attachment shading rate: no variable rate shading"
            );
        }
        if shading_rate.is_some() {
            device_extensions.push(ash::khr::fragment_shading_rate::NAME);
        }
        let device_names: Vec<*const std::ffi::c_char> =
            device_extensions.iter().map(|e| e.as_ptr()).collect();
        let mut phd_features = hal_adapter.physical_device_features(&device_extensions, features);
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        let mut rate_features = vk::PhysicalDeviceFragmentShadingRateFeaturesKHR::default()
            .pipeline_fragment_shading_rate(true)
            .attachment_fragment_shading_rate(true);
        let mut device_create = phd_features.add_to_device_create(
            vk::DeviceCreateInfo::default()
                .queue_create_infos(&queues)
                .enabled_extension_names(&device_names),
        );
        if shading_rate.is_some() {
            device_create = device_create.push_next(&mut rate_features);
        }
        let raw_device = unsafe {
            xr.create_vulkan_device(
                system,
                get_proc,
                physical.as_raw() as _,
                &device_create as *const _ as *const _,
            )
        }
        .map_err(err("xrCreateVulkanDeviceKHR"))?
        .map_err(|e| Error(format!("vkCreateDevice: {}", vk::Result::from_raw(e))))?;
        let ash_device = unsafe {
            ash::Device::load(
                ash_instance.fp_v1_0(),
                vk::Device::from_raw(raw_device as _),
            )
        };
        let open = unsafe {
            hal_adapter.device_from_raw(
                ash_device,
                None,
                &device_extensions,
                features,
                &limits,
                &wgpu::MemoryHints::Performance,
                family,
                0,
            )
        }
        .map_err(err("wgpu device"))?;
        if let Some(texel) = shading_rate {
            // SAFETY: the device has the extension and its features, on Vulkan 1.2 or newer,
            // and `texel` is one it allows; wgpu has made no pipeline on it yet.
            unsafe { open.device.enable_fragment_shading_rate(texel) };
        }
        (open, raw_device, shading_rate)
    };
    let (device, queue) = unsafe {
        adapter.create_device_from_hal(
            open,
            &wgpu::DeviceDescriptor {
                label: Some("Horizon XR device"),
                required_features: features,
                required_limits: limits,
                // Ray queries (RT lighting's hardware rays) are wgpu's experimental API: asked for
                // only when the client asks for them.
                // The API may change between wgpu versions, not misbehave.
                experimental_features: if features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
                {
                    wgpu::ExperimentalFeatures::enabled()
                } else {
                    wgpu::ExperimentalFeatures::disabled()
                },
                ..Default::default()
            },
        )
    }
    .map_err(err("wgpu device"))?;
    let (session, frame_wait, frame_stream) = unsafe {
        xr.create_session::<xr::Vulkan>(
            system,
            &xr::vulkan::SessionCreateInfo {
                instance: raw_instance,
                physical_device: physical.as_raw() as _,
                device: raw_device,
                queue_family_index: family,
                queue_index: 0,
            },
        )
    }
    .map_err(err("xrCreateSession"))?;
    Ok((
        instance,
        adapter,
        device,
        queue,
        session,
        frame_wait,
        frame_stream,
        shading_rate,
    ))
}

/// An eye's off-axis projection in wgpu clip space with reversed depth, like Horizon's world
/// projection (`near` maps to depth 1, `far` to 0). Column-major.
pub fn projection(fov: xr::Fovf, near: f32, far: f32) -> [[f32; 4]; 4] {
    let (l, r) = (fov.angle_left.tan(), fov.angle_right.tan());
    let (d, u) = (fov.angle_down.tan(), fov.angle_up.tan());
    let (w, h) = (r - l, u - d);
    // Reversed z: depth = (near * (far - (-z))... ) chosen so z_ndc(-near) = 1, z_ndc(-far) = 0.
    let a = near / (far - near);
    let b = far * near / (far - near);
    [
        [2.0 / w, 0.0, 0.0, 0.0],
        [0.0, 2.0 / h, 0.0, 0.0],
        [(r + l) / w, (u + d) / h, a, -1.0],
        [0.0, 0.0, b, 0.0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
        let v = [p[0], p[1], p[2], 1.0];
        let mut c = [0.0; 4];
        for (col, x) in m.iter().zip(v) {
            for (row, value) in col.iter().enumerate() {
                c[row] += value * x;
            }
        }
        [c[0] / c[3], c[1] / c[3], c[2] / c[3]]
    }

    #[test]
    fn projection_maps_near_to_one_far_to_zero_and_edges_to_the_frame() {
        let fov = xr::Fovf {
            angle_left: -0.8,
            angle_right: 0.7,
            angle_up: 0.75,
            angle_down: -0.85,
        };
        let m = projection(fov, 0.05, 1000.0);
        assert!((clip(m, [0.0, 0.0, -0.05])[2] - 1.0).abs() < 1e-4);
        assert!(clip(m, [0.0, 0.0, -1000.0])[2].abs() < 1e-4);
        let left = clip(m, [fov.angle_left.tan() * 2.0, 0.0, -2.0]);
        let up = clip(m, [0.0, fov.angle_up.tan() * 2.0, -2.0]);
        assert!((left[0] + 1.0).abs() < 1e-4, "{left:?}");
        assert!((up[1] - 1.0).abs() < 1e-4, "{up:?}");
    }

    /// A rotation of `degrees` about the unit `axis`.
    fn turn(axis: [f32; 3], degrees: f32) -> xr::Quaternionf {
        let (s, c) = (degrees.to_radians() * 0.5).sin_cos();
        xr::Quaternionf {
            x: axis[0] * s,
            y: axis[1] * s,
            z: axis[2] * s,
            w: c,
        }
    }

    #[test]
    fn the_gaze_turns_with_the_head_and_its_pitch_can_be_mirrored() {
        // The head turned 90 degrees left (facing -x) at 1.6 m; the eyes look 12 degrees up.
        let head = xr::Posef {
            orientation: turn([0.0, 1.0, 0.0], 90.0),
            position: xr::Vector3f {
                x: 0.0,
                y: 1.6,
                z: 0.0,
            },
        };
        let view = xr::Posef {
            orientation: turn([1.0, 0.0, 0.0], 12.0),
            position: xr::Vector3f {
                x: 0.0,
                y: 0.0,
                z: -0.01,
            },
        };
        let g = gaze_from(head, view, true, false);
        let up = 12f32.to_radians();
        assert!((g.view[1] - up.sin()).abs() < 1e-5, "{g:?}");
        assert!((g.direction[0] + up.cos()).abs() < 1e-5, "faces -x: {g:?}");
        assert!((g.direction[1] - up.sin()).abs() < 1e-5, "{g:?}");
        assert!((g.origin[0] + 0.01).abs() < 1e-5 && (g.origin[1] - 1.6).abs() < 1e-5);
        let mirrored = gaze_from(head, view, false, true);
        assert!((mirrored.view[1] + up.sin()).abs() < 1e-5, "{mirrored:?}");
        assert!((mirrored.direction[1] + up.sin()).abs() < 1e-5);
        assert!(!mirrored.tracked);
    }
}
