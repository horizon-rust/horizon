//! HDR metadata for the swapchain: the Vulkan calls the game crate, which
//! forbids unsafe code, can't make itself. wgpu sets no HDR metadata, and
//! without it a compositor may clip or tone-map everything above 1.0 (80 nits)
//! of an scRGB swapchain.

#[cfg(target_os = "linux")]
pub mod wayland;

/// Opens a Vulkan device with `VK_EXT_hdr_metadata` too, when the GPU offers
/// it, so [`set_metadata`] can tell the compositor the real peak brightness.
/// `None` leaves opening to wgpu.
#[cfg(not(target_os = "macos"))]
pub fn open_vulkan_device(
    adapter: &wgpu::Adapter,
    desc: &wgpu::DeviceDescriptor<'_>,
) -> Option<Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError>> {
    open(adapter, desc, &[]).ok().flatten()
}

/// [`open_vulkan_device`] with `extra` device extensions too (DLSS Ray
/// Reconstruction's NGX list, docs/rt-dlss-rr.md), each enabled only where the
/// GPU offers it. With any, the device is opened here even on a GPU without
/// `VK_EXT_hdr_metadata`; when that fails, it is opened as without them. With
/// none, this is [`open_vulkan_device`].
#[cfg(not(target_os = "macos"))]
pub fn open_vulkan_device_with(
    adapter: &wgpu::Adapter,
    desc: &wgpu::DeviceDescriptor<'_>,
    extra: &[&'static std::ffi::CStr],
) -> Option<Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError>> {
    if extra.is_empty() {
        return open_vulkan_device(adapter, desc);
    }
    let failure = match open(adapter, desc, extra) {
        Ok(Some(Ok(opened))) => return Some(Ok(opened)),
        Ok(Some(Err(e))) => e.to_string(),
        Ok(None) => return open_vulkan_device(adapter, desc),
        Err(e) => e,
    };
    log::warn!(
        "Horizon renderer: the Vulkan device with {} extra extension(s) failed ({failure}); \
         opening it without them",
        extra.len()
    );
    open_vulkan_device(adapter, desc)
}

/// `open_vulkan_device`'s body, with `extra` extensions; `Err` when wgpu-hal's
/// open fails (which `open_vulkan_device` answers with `None`).
#[cfg(not(target_os = "macos"))]
fn open(
    adapter: &wgpu::Adapter,
    desc: &wgpu::DeviceDescriptor<'_>,
    extra: &[&'static std::ffi::CStr],
) -> Result<Option<Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError>>, String> {
    use wgpu::hal::api::Vulkan;
    let open = {
        // Safety: only read and opened from here; dropped before wgpu takes it back.
        let Some(hal) = (unsafe { adapter.as_hal::<Vulkan>() }) else {
            return Ok(None);
        };
        let instance = hal.shared_instance().raw_instance();
        // Safety: the adapter's own physical device on its own instance.
        let Ok(extensions) =
            (unsafe { instance.enumerate_device_extension_properties(hal.raw_physical_device()) })
        else {
            return Ok(None);
        };
        let offers = |name: &std::ffi::CStr| {
            extensions
                .iter()
                .any(|e| e.extension_name_as_c_str() == Ok(name))
        };
        let hdr = offers(ash::ext::hdr_metadata::NAME);
        if !hdr && extra.is_empty() {
            return Ok(None);
        }
        let extra: Vec<&'static std::ffi::CStr> =
            extra.iter().copied().filter(|&e| offers(e)).collect();
        // The RTX 4090 build's shader execution reordering (horizon-rtx `sun`,
        // docs/rtx-4090.md): the extension and its feature where the driver offers them and the
        // device gets ray tracing pipelines (wgpu's EXPERIMENTAL_RAY_TRACING_PIPELINES).
        let reorder = offers(ash::nv::ray_tracing_invocation_reorder::NAME)
            && desc
                .required_features
                .contains(wgpu::Features::EXPERIMENTAL_RAY_TRACING_PIPELINES);
        let mut reorder_features =
            ash::vk::PhysicalDeviceRayTracingInvocationReorderFeaturesNV::default()
                .ray_tracing_invocation_reorder(true);
        // The RTX 4090 build's opacity micromaps (horizon-rtx `omm`, `HORIZON_RT_OMM=0` turns them off): the
        // extension, its feature and what it builds on, where the driver offers them and the
        // device traces rays.
        let micromaps = !std::env::var("HORIZON_RT_OMM").is_ok_and(|v| v == "0")
            && offers(ash::ext::opacity_micromap::NAME)
            && desc
                .required_features
                .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
        let synchronization2 = micromaps && offers(ash::khr::synchronization2::NAME);
        let mut micromap_features =
            ash::vk::PhysicalDeviceOpacityMicromapFeaturesEXT::default().micromap(true);
        // Safety: what wgpu's own request_device does, with one or two more extensions.
        unsafe {
            hal.open_with_callback(
                desc.required_features,
                &desc.required_limits,
                &desc.memory_hints,
                Some(Box::new(
                    |args: wgpu::hal::vulkan::CreateDeviceCallbackArgs| {
                        if hdr {
                            args.extensions.push(ash::ext::hdr_metadata::NAME);
                        }
                        if reorder {
                            args.extensions
                                .push(ash::nv::ray_tracing_invocation_reorder::NAME);
                            *args.create_info = args.create_info.push_next(&mut reorder_features);
                        }
                        if micromaps {
                            args.extensions.push(ash::ext::opacity_micromap::NAME);
                            if synchronization2
                                && !args.extensions.contains(&ash::khr::synchronization2::NAME)
                            {
                                args.extensions.push(ash::khr::synchronization2::NAME);
                            }
                            *args.create_info = args.create_info.push_next(&mut micromap_features);
                        }
                        for name in extra {
                            if !args.extensions.contains(&name) {
                                args.extensions.push(name);
                            }
                        }
                    },
                )),
            )
        }
        .map_err(|e| e.to_string())?
    };
    // Safety: opened from this adapter with exactly `desc`'s features.
    Ok(Some(unsafe { adapter.create_device_from_hal(open, desc) }))
}

/// wgpu's opt-in to its `EXPERIMENTAL_*` features, which the crates that forbid unsafe code
/// can't give: hardware ray queries, asked for only when the player turns ray tracing on
/// (`gfx/ray_query.rs` in horizon-client-render).
pub fn experimental_features() -> wgpu::ExperimentalFeatures {
    // Safety: wgpu warns that its experimental features may have bugs that reach undefined
    // behaviour through safe calls. Horizon accepts that only on request, for ray queries.
    unsafe { wgpu::ExperimentalFeatures::enabled() }
}

/// Tells the compositor, through `VK_EXT_hdr_metadata`, that the swapchain's
/// content is BT.709, peaks at `peak` nits and is no darker than `black`. Needs a
/// device from `open_vulkan_device` and a configured surface; does nothing otherwise.
pub fn set_metadata(device: &wgpu::Device, surface: &wgpu::Surface<'_>, peak: f32, black: f32) {
    #[cfg(not(target_os = "macos"))]
    {
        use wgpu::hal::api::Vulkan;
        // Safety: read-only access to handles wgpu keeps alive while these guards live.
        let (Some(hal_device), Some(hal_surface)) =
            (unsafe { device.as_hal::<Vulkan>() }, unsafe {
                surface.as_hal::<Vulkan>()
            })
        else {
            return;
        };
        if !hal_device
            .enabled_device_extensions()
            .contains(&ash::ext::hdr_metadata::NAME)
        {
            return;
        }
        let Some(swapchain) = hal_surface.raw_native_swapchain() else {
            return;
        };
        let loader = ash::ext::hdr_metadata::Device::new(
            hal_device.shared_instance().raw_instance(),
            hal_device.raw_device(),
        );
        // Safety: a live swapchain of this device, one metadata entry for it.
        unsafe { loader.set_hdr_metadata(&[swapchain], &[metadata(peak, black)]) };
    }
    #[cfg(target_os = "macos")]
    let _ = (device, surface, peak, black);
}

#[cfg(not(target_os = "macos"))]
fn metadata(peak: f32, black: f32) -> ash::vk::HdrMetadataEXT<'static> {
    let xy = |x, y| ash::vk::XYColorEXT { x, y };
    ash::vk::HdrMetadataEXT::default()
        .display_primary_red(xy(0.64, 0.33))
        .display_primary_green(xy(0.30, 0.60))
        .display_primary_blue(xy(0.15, 0.06))
        .white_point(xy(0.3127, 0.3290))
        .max_luminance(peak)
        .min_luminance(black)
        .max_content_light_level(peak)
        // Unknown (CTA-861.3): the game can't know a frame's average ahead of it, and
        // sky or snow can average above paper white; a display may dim against a low one.
        .max_frame_average_light_level(0.0)
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    #[test]
    fn the_frame_average_is_sent_as_unknown() {
        let m = super::metadata(1000.0, 0.05);
        assert_eq!(m.max_frame_average_light_level, 0.0);
        assert_eq!(m.max_content_light_level, 1000.0);
        assert_eq!(m.max_luminance, 1000.0);
        assert_eq!(m.min_luminance, 0.05);
    }
}

/// wgpu's experimental features, for RT lighting's hardware ray queries
/// (`Features::EXPERIMENTAL_RAY_QUERY`, the client's `gfx/hw_rays.rs`): the
/// token wgpu asks for before it opens a device with them, which the client,
/// forbidding unsafe code, can't make itself.
pub fn ray_query_features() -> wgpu::ExperimentalFeatures {
    // Safety: only asked for with ray queries, which the client turns off with
    // HORIZON_HW_RT=0; the API may change between wgpu versions, not misbehave.
    unsafe { wgpu::ExperimentalFeatures::enabled() }
}
