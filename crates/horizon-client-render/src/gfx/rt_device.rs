//! The terrain shader's sources for RT lighting and VR, and the device features they need: the
//! hardware rays', and the OpenXR runtime's own device (gfx.rs may not grow).

use super::*;

/// The features the renderer asks its device for. Mappable primary buffers: section pages
/// that no longer fit in video memory go to system memory. Shader packs' 16-bit normalized
/// buffers (render targets with the adapter's own format features) and R11F_G11F_B10F: a
/// float stand-in neither clamps nor drops NaN.
/// terrain.wgsl, its clip position `@invariant` where the world tests against a depth prepass
/// for equality (`world_prepass`) or `HORIZON_TERRAIN_INVARIANT=1` asks. b19c0339b made it
/// invariant always, the only change to the terrain's shaders between b5b5252a7 and 482620a06,
/// over which the RTX 4090's "world: phase 1" took 0.9 to 1.1 ms more and the rays' prepass 0.33
/// (one session, the time of day fixed); the variable tells whether it was that.
pub(super) fn terrain_source() -> std::borrow::Cow<'static, str> {
    static INVARIANT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let invariant = *INVARIANT.get_or_init(|| {
        world_prepass() != WorldPrepass::Off
            || std::env::var("HORIZON_TERRAIN_INVARIANT").is_ok_and(|v| v != "0")
    });
    // HDR lighting's functions ahead of terrain.wgsl (`terrain_shader`).
    let source = terrain_shader();
    if invariant {
        invariant_terrain(&source).into()
    } else {
        source.into()
    }
}
/// `source` (terrain.wgsl) with `TerrainOut`'s clip position `@invariant`.
pub(super) fn invariant_terrain(source: &str) -> String {
    let at = source
        .find("struct TerrainWire {")
        .expect("terrain.wgsl has TerrainWire");
    let (head, tail) = source.split_at(at);
    let field = "@builtin(position) clip: vec4<f32>,";
    debug_assert!(tail.contains(field));
    format!(
        "{head}{}",
        tail.replacen(field, "@builtin(position) @invariant clip: vec4<f32>,", 1)
    )
}
/// terrain.wgsl with the translucent terrain's dual-source entry (`fs_translucent_dual`).
pub(super) fn dual_source_terrain() -> String {
    format!(
        "enable dual_source_blending;\n{}{}",
        terrain_source(),
        include_str!("../terrain_dual.wgsl")
    )
}
/// terrain.wgsl with VR's two-view depth prepass entry (`vs_terrain_views`, `EyeDepths`).
pub(super) fn terrain_views() -> String {
    format!(
        "{}{}",
        terrain_source(),
        include_str!("../terrain_views.wgsl")
    )
}
/// The features every device is asked for (`surface_setup::open_adapter`, `open_xr_device`):
/// those of the adapter's the renderer's passes use.
pub(super) fn device_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    timing::GpuTimer::features(adapter)
        | pack_timing::PassTiming::features(adapter)
        | cull::Cull::features(adapter)
        | oit::features(adapter)
        | (adapter.features()
            & (wgpu::Features::DUAL_SOURCE_BLENDING
                | wgpu::Features::INDIRECT_FIRST_INSTANCE
                | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT
                | wgpu::Features::MAPPABLE_PRIMARY_BUFFERS
                | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                | wgpu::Features::RG11B10UFLOAT_RENDERABLE
                | wgpu::Features::SUBGROUP
                | wgpu::Features::VERTEX_WRITABLE_STORAGE
                | wgpu::Features::CLEAR_TEXTURE
                | wgpu::Features::TEXTURE_ATOMIC
                | wgpu::Features::FLOAT32_FILTERABLE))
}
/// RT lighting's hybrid (`hw_rays`): the hardware ray queries asked of a Vulkan device, none with
/// HORIZON_HW_RT=0.
pub(super) fn hw_rt_features() -> wgpu::Features {
    if std::env::var("HORIZON_HW_RT").is_ok_and(|v| v == "0") {
        wgpu::Features::empty()
    } else {
        wgpu::Features::EXPERIMENTAL_RAY_QUERY
    }
}
/// EXPERIMENTAL VR: the device through the OpenXR runtime (`xr`), with a surface for the
/// window on the same instance. `None` when the runtime, the headset or the surface is missing.
pub(super) fn open_xr_device(window: &Arc<Window>, offscreen: bool, layers: u32) -> Option<Opened> {
    // RT lighting's hardware rays in the headset as on the screen, where the adapter has them,
    // with the RTX build's ray tracing pipelines (horizon-xr adds the build's extensions).
    let xr = xr::open(|adapter| {
        let pipelines = if cfg!(feature = "rtx")
            && !hw_rt_features().is_empty()
            && adapter.features().contains(hw_rt_features())
        {
            wgpu::Features::EXPERIMENTAL_RAY_TRACING_PIPELINES
        } else {
            wgpu::Features::empty()
        };
        (
            device_features(adapter)
                | (adapter.features() & (hw_rt_features() | wgpu::Features::MULTIVIEW | pipelines)),
            adapter.limits(),
        )
    })?;
    describe_adapter(&xr.adapter);
    if let Err(e) = check_layers(&xr.adapter, layers) {
        log::warn!("Horizon VR: {e}; starting without VR");
        return None;
    }
    let surface = if offscreen {
        None
    } else {
        match xr.instance.create_surface(window.clone()) {
            Ok(surface) => Some(surface),
            Err(e) => {
                log::warn!(
                    "Horizon VR: no window surface on the headset's instance ({e}); starting without VR"
                );
                return None;
            }
        }
    };
    Some(Opened {
        instance: xr.instance.clone(),
        surface,
        adapter: xr.adapter.clone(),
        device: xr.device.clone(),
        queue: xr.queue.clone(),
        xr: Some(Arc::new(std::sync::Mutex::new(xr))),
        device_lost: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        // The surface is set up on the headset's device by `Renderer::new`.
        configured: None,
    })
}
/// The pack's `request` under Linear Light (`linear`): no float frame of a shader API 0.2 pack
/// or of HDR's layers, as the float scene is tone mapped into the frame (`tonemap`).
pub(super) fn linear_light_request(
    linear: bool,
    request: output::FrameRequest,
) -> output::FrameRequest {
    if linear {
        output::FrameRequest {
            linear_pack: false,
            hdr_float: false,
            ..request
        }
    } else {
        request
    }
}
