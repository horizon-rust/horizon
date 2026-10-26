//! EXPERIMENTAL OpenXR for Horizon (research spike, off by default).
//!
//! Opens a session on the active OpenXR runtime (any runtime: SteamVR, Monado, Meta,
//! Virtual Desktop...) through `XR_KHR_vulkan_enable2`, and creates the wgpu device on the
//! Vulkan instance and device the runtime asks for, so Horizon's wgpu renderer can draw
//! straight into the headset's swapchain images. The OpenXR loader (`openxr_loader.dll`,
//! `libopenxr_loader.so`) is loaded at run time: without it, nothing here runs.
//!
//! The unsafe interop lives in this crate because horizon-client forbids unsafe code.

use ash::vk::{self, Handle};
use openxr as xr;

mod controls;
pub use controls::Controls;
/// The OpenXR types in this crate's API (poses, field of view).
pub use openxr;

const VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;

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

/// One eye this frame: where it is (stage space, metres, y up) and what it sees.
#[derive(Clone, Copy, Debug)]
pub struct Eye {
    pub pose: xr::Posef,
    pub fov: xr::Fovf,
}

/// A flat panel the runtime composites in the world (`Xr::create_quad`): where its centre
/// is (stage space) and its width and height in metres.
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    pub pose: xr::Posef,
    pub size: [f32; 2],
}

/// A frame the headset wants drawn: draw each eye into `eye_texture`, then `end_frame`.
pub struct Frame {
    pub eyes: [Eye; 2],
    /// The head (between the eyes, stage space).
    pub head: xr::Posef,
    /// The time the runtime will show this frame, for poses and animations.
    pub display_time: xr::Time,
    /// Where to show the panel drawn into `quad_texture` this frame; `None` hides it.
    pub quad: Option<Quad>,
    images: [usize; 2],
    quad_image: Option<usize>,
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
    quad: Option<EyeChain>,
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
    /// Each eye's recommended image size, in pixels.
    pub size: [u32; 2],
    /// The panel's image size, in pixels (`create_quad`).
    pub quad_size: [u32; 2],
    pub runtime: String,
    pub system_name: String,
    running: bool,
    /// The runtime ended the session (headset off, SteamVR closed).
    pub exit: bool,
}

impl Xr {
    /// Opens the runtime, the Vulkan instance and device it asks for, and wgpu on top of
    /// them. `request` gets the adapter and returns the features and limits the device must
    /// have (the renderer's own request, which it can only work out from the adapter);
    /// `flags` are the instance's, as the renderer would create it.
    pub fn new(
        app: &str,
        flags: wgpu::InstanceFlags,
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
        let blend = xr
            .enumerate_environment_blend_modes(system, VIEW_TYPE)
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
        let views = xr
            .enumerate_view_configuration_views(system, VIEW_TYPE)
            .map_err(err("views"))?;
        let size = [
            views[0].recommended_image_rect_width,
            views[0].recommended_image_rect_height,
        ];

        // SAFETY: every handle is created below, from one Vulkan entry, with the extensions and
        // features wgpu-hal asks for: what `from_raw` and `device_from_raw` require.
        let (instance, adapter, device, queue, session, frame_wait, frame_stream) =
            unsafe { open_vulkan(&xr, system, api, flags, request)? };
        let stage = session
            .create_reference_space(xr::ReferenceSpaceType::STAGE, xr::Posef::IDENTITY)
            .or_else(|_| {
                session.create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)
            })
            .map_err(err("reference space"))?;
        let view = session
            .create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)
            .map_err(err("view space"))?;
        let actions = controls::Actions::new(&xr, &session)
            .map_err(|e| log::warn!("Horizon VR: no controllers ({e})"))
            .ok();
        Ok(Self {
            eyes: Vec::new(),
            quad: None,
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
            size,
            quad_size: [0, 0],
            runtime,
            system_name,
            running: false,
            exit: false,
        })
    }

    /// Creates one swapchain per eye whose images the renderer draws into through views in
    /// `format`: an 8-bit format without sRGB (Horizon writes gamma-encoded colour, like
    /// vanilla). The images themselves are the sRGB twin, so the runtime reads them right.
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
        self.eyes = eyes;
        Ok(())
    }

    /// A panel of `size` pixels for `Frame::quad`, in the eyes' format (`create_swapchains`).
    pub fn create_quad(
        &mut self,
        size: [u32; 2],
        format: wgpu::TextureFormat,
    ) -> Result<(), Error> {
        let (vk_format, srgb) = srgb_twin(format)?;
        self.quad = Some(self.chain(size, vk_format, srgb)?);
        self.quad_size = size;
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
                        if self.session.begin(VIEW_TYPE).is_ok() {
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
        let (_, views) = self
            .session
            .locate_views(VIEW_TYPE, state.predicted_display_time, &self.stage)
            .map_err(err("xrLocateViews"))?;
        let head = self
            .view
            .locate(&self.stage, state.predicted_display_time)
            .map(|l| l.pose)
            .unwrap_or(xr::Posef::IDENTITY);
        let mut images = [0; 2];
        for (i, eye) in self.eyes.iter_mut().enumerate() {
            images[i] = eye
                .swapchain
                .acquire_image()
                .map_err(err("xrAcquireSwapchainImage"))? as usize;
            eye.swapchain
                .wait_image(xr::Duration::INFINITE)
                .map_err(err("xrWaitSwapchainImage"))?;
        }
        Ok(Some(Frame {
            eyes: [0, 1].map(|i| Eye {
                pose: views[i].pose,
                fov: views[i].fov,
            }),
            head,
            display_time: state.predicted_display_time,
            quad: None,
            images,
            quad_image: None,
        }))
    }

    /// Whether the panel has an image to show again without drawing a new one.
    pub fn quad_ready(&self) -> bool {
        self.quad.as_ref().is_some_and(|c| c.released)
    }

    /// The panel's image for `frame` (acquired on the first call), when there is a panel.
    pub fn quad_texture(&mut self, frame: &mut Frame) -> Result<Option<wgpu::Texture>, Error> {
        let Some(chain) = &mut self.quad else {
            return Ok(None);
        };
        let image = match frame.quad_image {
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
                frame.quad_image = Some(image);
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

    /// Hands `frame` to the runtime. Call after the eyes' commands are submitted.
    pub fn end_frame(&mut self, frame: Frame) -> Result<(), Error> {
        for eye in &mut self.eyes {
            eye.swapchain
                .release_image()
                .map_err(err("xrReleaseSwapchainImage"))?;
        }
        // A panel not drawn this frame shows the image released last (OpenXR keeps it).
        let quad = match (&mut self.quad, frame.quad_image) {
            (Some(chain), Some(_)) => {
                chain
                    .swapchain
                    .release_image()
                    .map_err(err("xrReleaseSwapchainImage"))?;
                chain.released = true;
                frame.quad.map(|q| (q, &chain.swapchain))
            }
            (Some(chain), None) if chain.released => frame.quad.map(|q| (q, &chain.swapchain)),
            _ => None,
        };
        let rect = xr::Rect2Di {
            offset: xr::Offset2Di { x: 0, y: 0 },
            extent: xr::Extent2Di {
                width: self.size[0] as i32,
                height: self.size[1] as i32,
            },
        };
        let views = [0, 1].map(|i| {
            xr::CompositionLayerProjectionView::new()
                .pose(frame.eyes[i].pose)
                .fov(frame.eyes[i].fov)
                .sub_image(
                    xr::SwapchainSubImage::new()
                        .swapchain(&self.eyes[i].swapchain)
                        .image_rect(rect),
                )
        });
        let layer = xr::CompositionLayerProjection::new()
            .space(&self.stage)
            .views(&views);
        let panel = quad.map(|(q, swapchain)| {
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
                                width: self.quad_size[0] as i32,
                                height: self.quad_size[1] as i32,
                            },
                        }),
                )
        });
        let layers: Vec<&xr::CompositionLayerBase<xr::Vulkan>> = match &panel {
            Some(panel) => vec![&layer, panel],
            None => vec![&layer],
        };
        self.frame_stream
            .end(frame.display_time, self.blend, &layers)
            .map_err(err("xrEndFrame"))
    }

    /// The OpenXR instance, for actions (controllers) later on.
    pub fn instance(&self) -> &xr::Instance {
        &self.xr
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
);

/// The Vulkan instance and device through the runtime (`xrCreateVulkan*KHR`), wgpu on them,
/// and the session.
unsafe fn open_vulkan(
    xr: &xr::Instance,
    system: xr::SystemId,
    api: u32,
    flags: wgpu::InstanceFlags,
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
    let (open, raw_device) = {
        let hal_adapter = unsafe { adapter.as_hal::<wgpu_hal::api::Vulkan>() }
            .ok_or_else(|| Error("not a Vulkan adapter".into()))?;
        let device_extensions = hal_adapter.required_device_extensions(features);
        let device_names: Vec<*const std::ffi::c_char> =
            device_extensions.iter().map(|e| e.as_ptr()).collect();
        let mut phd_features = hal_adapter.physical_device_features(&device_extensions, features);
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        let device_create = phd_features.add_to_device_create(
            vk::DeviceCreateInfo::default()
                .queue_create_infos(&queues)
                .enabled_extension_names(&device_names),
        );
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
        (open, raw_device)
    };
    let (device, queue) = unsafe {
        adapter.create_device_from_hal(
            open,
            &wgpu::DeviceDescriptor {
                label: Some("Horizon XR device"),
                required_features: features,
                required_limits: limits,
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
}
