//! DLSS Ray Reconstruction (NGX's "DLSS-D" feature) on Vulkan, behind the `dlss-rr` feature
//! ([docs/rt-dlss-rr.md](../../../docs/rt-dlss-rr.md)): one learned pass in place of RT lighting's
//! accumulation, denoiser and TAA, which can also upscale. Nothing here runs unless the game asks
//! for it (`wanted`: `HORIZON_RT_DLSS_RR=1`), and a failure anywhere is an `Err` the renderer
//! answers with its own denoiser.
//!
//! There are no official Rust bindings. `ffi` declares by hand the C functions of the SDK's
//! `nvsdk_ngx_vk.h` and `nvsdk_ngx_params.h` (exported by the static `nvsdk_ngx` library that
//! build.rs links), the structures of `nvsdk_ngx_defs.h`/`nvsdk_ngx_defs_vk.h`, and the parameter
//! names the SDK's inline helpers (`nvsdk_ngx_helpers_dlssd_vk.h`, `NGX_VULKAN_CREATE_DLSSD_EXT1`,
//! `NGX_VULKAN_EVALUATE_DLSSD_EXT`) set, which `DlssRr::new` and `DlssRr::evaluate` set the same
//! way. The static library loads NVIDIA's runtime `nvngx_dlssd.dll` (never in this repository) from
//! the folders `Ngx::init` is given. What was checked against the SDK's public headers and what
//! was not is listed in the doc.

use ash::vk;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::Arc;
use wgpu::hal::api::Vulkan;

/// Whether DLSS Ray Reconstruction is asked for: `HORIZON_RT_DLSS_RR=1` (off by default).
pub fn wanted() -> bool {
    std::env::var("HORIZON_RT_DLSS_RR").is_ok_and(|v| v == "1")
}

/// The quality mode the window's feature is made in: `HORIZON_DLSS_RR_QUALITY`
/// (`quality`, `balanced`, `performance`, `ultra`; unset or `dlaa` is DLAA, render size = output
/// size). An unknown value is logged and taken as DLAA.
pub fn quality_wanted() -> Quality {
    let Ok(v) = std::env::var("HORIZON_DLSS_RR_QUALITY") else {
        return Quality::Dlaa;
    };
    Quality::parse(&v).unwrap_or_else(|| {
        log::warn!(
            "DLSS-RR: HORIZON_DLSS_RR_QUALITY={v:?} is not dlaa, quality, balanced, performance \
             or ultra: DLAA"
        );
        Quality::Dlaa
    })
}

/// Why NGX or the feature could not be used: the call and its `NVSDK_NGX_Result`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub call: &'static str,
    pub result: u32,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed (NGX result {:#010x})", self.call, self.result)
    }
}

impl std::error::Error for Error {}

fn check(call: &'static str, result: ffi::Result) -> Result<(), Error> {
    if ffi::failed(result) {
        Err(Error { call, result })
    } else {
        Ok(())
    }
}

/// The hand-written declarations (see the module's comment). Names follow the SDK's C names.
#[allow(non_snake_case, non_camel_case_types, dead_code)]
pub mod ffi {
    use ash::vk;
    use std::ffi::{c_char, c_int, c_uint, c_ulonglong, c_void};

    /// `wchar_t`: UTF-16 on Windows, UTF-32 elsewhere.
    #[cfg(windows)]
    pub type WChar = u16;
    #[cfg(not(windows))]
    pub type WChar = u32;

    /// `NVSDK_NGX_Result`: failures are `0xBAD0_0000` in the top 12 bits.
    pub type Result = u32;
    pub const RESULT_SUCCESS: Result = 0x1;
    pub const RESULT_FAIL: Result = 0xBAD0_0000;
    /// `NVSDK_NGX_Result_FAIL_OutOfDate`: what the optimal-settings helper answers when the
    /// capability parameters hold no callback (an old DLL or driver).
    pub const RESULT_FAIL_OUT_OF_DATE: Result = 0xBAD0_000C;
    /// `NVSDK_NGX_FAILED`.
    pub const fn failed(r: Result) -> bool {
        r & 0xFFF0_0000 == 0xBAD0_0000
    }

    /// `NVSDK_NGX_Version_API` of the SDK the declarations were checked against (310.x).
    pub const VERSION_API: c_int = 0x15;

    /// `NVSDK_NGX_Feature`.
    pub const FEATURE_SUPER_SAMPLING: c_int = 1;
    pub const FEATURE_RAY_RECONSTRUCTION: c_int = 13;

    /// `NVSDK_NGX_EngineType`.
    pub const ENGINE_CUSTOM: c_int = 0;

    /// `NVSDK_NGX_PerfQuality_Value`.
    pub const PERF_MAX_PERF: c_int = 0;
    pub const PERF_BALANCED: c_int = 1;
    pub const PERF_MAX_QUALITY: c_int = 2;
    pub const PERF_ULTRA_PERFORMANCE: c_int = 3;
    pub const PERF_ULTRA_QUALITY: c_int = 4;
    pub const PERF_DLAA: c_int = 5;

    /// `NVSDK_NGX_DLSS_Feature_Flags`.
    pub const FLAG_IS_HDR: c_int = 1 << 0;
    pub const FLAG_MV_LOW_RES: c_int = 1 << 1;
    pub const FLAG_MV_JITTERED: c_int = 1 << 2;
    pub const FLAG_DEPTH_INVERTED: c_int = 1 << 3;
    pub const FLAG_AUTO_EXPOSURE: c_int = 1 << 6;

    /// `NVSDK_NGX_DLSS_Denoise_Mode`, `_Roughness_Mode`, `_Depth_Type`.
    pub const DENOISE_DL_UNIFIED: c_int = 1;
    pub const ROUGHNESS_UNPACKED: c_uint = 0;
    pub const ROUGHNESS_PACKED: c_uint = 1;
    pub const DEPTH_LINEAR: c_uint = 0;
    pub const DEPTH_HW: c_uint = 1;

    /// `NVSDK_NGX_Feature_Support_Result`: 0 supported, else a mask of why not.
    pub const FEATURE_SUPPORTED: c_int = 0;

    /// `NVSDK_NGX_Resource_VK_Type`.
    pub const RESOURCE_VK_IMAGEVIEW: c_int = 0;
    pub const RESOURCE_VK_BUFFER: c_int = 1;

    /// `NVSDK_NGX_Parameter`, `NVSDK_NGX_Handle`: opaque.
    #[repr(C)]
    pub struct Parameter {
        _opaque: [u8; 0],
    }
    #[repr(C)]
    pub struct Handle {
        _opaque: [u8; 0],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct ImageViewInfo_VK {
        pub ImageView: vk::ImageView,
        pub Image: vk::Image,
        pub SubresourceRange: vk::ImageSubresourceRange,
        pub Format: vk::Format,
        pub Width: c_uint,
        pub Height: c_uint,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct BufferInfo_VK {
        pub Buffer: vk::Buffer,
        pub SizeInBytes: c_uint,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub union ResourceUnion_VK {
        pub ImageViewInfo: ImageViewInfo_VK,
        pub BufferInfo: BufferInfo_VK,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Resource_VK {
        pub Resource: ResourceUnion_VK,
        pub Type: c_int,
        pub ReadWrite: bool,
    }

    #[repr(C)]
    pub struct PathListInfo {
        pub Path: *const *const WChar,
        pub Length: c_uint,
    }

    pub type AppLogCallback =
        Option<unsafe extern "C" fn(message: *const c_char, level: c_int, source: c_int)>;

    #[repr(C)]
    pub struct LoggingInfo {
        pub LoggingCallback: AppLogCallback,
        /// `NVSDK_NGX_Logging_Level`: 0 off, 1 on, 2 verbose.
        pub MinimumLoggingLevel: c_int,
        pub DisableOtherLoggingSinks: bool,
    }

    #[repr(C)]
    pub struct FeatureCommonInfo {
        pub PathListInfo: PathListInfo,
        pub InternalData: *mut c_void,
        pub LoggingInfo: LoggingInfo,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct ProjectIdDescription {
        pub ProjectId: *const c_char,
        pub EngineType: c_int,
        pub EngineVersion: *const c_char,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub union IdentifierValue {
        pub ProjectDesc: ProjectIdDescription,
        pub ApplicationId: c_ulonglong,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Application_Identifier {
        /// 0 an application id, 1 a project id.
        pub IdentifierType: c_int,
        pub v: IdentifierValue,
    }

    #[repr(C)]
    pub struct FeatureDiscoveryInfo {
        pub SDKVersion: c_int,
        pub FeatureID: c_int,
        pub Identifier: Application_Identifier,
        pub ApplicationDataPath: *const WChar,
        pub FeatureInfo: *const FeatureCommonInfo,
    }

    #[repr(C)]
    pub struct FeatureRequirement {
        pub FeatureSupported: c_int,
        pub MinHWArchitecture: c_uint,
        pub MinOSVersion: [c_char; 255],
    }

    /// `PFN_NVSDK_NGX_DLSS_GetOptimalSettingsCallback`, the `DLSSDOptimalSettingsCallback`
    /// capability parameter: the SDK's inline `NGX_DLSSD_GET_OPTIMAL_SETTINGS` calls it on the
    /// capability parameters after setting `Width`, `Height`, `PerfQualityValue` and `RTXValue`
    /// (`Ngx::optimal_settings` does the same).
    pub type OptimalSettingsCallback = unsafe extern "C" fn(*mut Parameter) -> Result;

    // NVSDK_CONV is __cdecl on MSVC (the only convention on x86-64) and nothing elsewhere.
    unsafe extern "C" {
        pub fn NVSDK_NGX_VULKAN_Init_with_ProjectID(
            InProjectId: *const c_char,
            InEngineType: c_int,
            InEngineVersion: *const c_char,
            InApplicationDataPath: *const WChar,
            InInstance: vk::Instance,
            InPD: vk::PhysicalDevice,
            InDevice: vk::Device,
            InGIPA: Option<vk::PFN_vkGetInstanceProcAddr>,
            InGDPA: Option<vk::PFN_vkGetDeviceProcAddr>,
            InFeatureInfo: *const FeatureCommonInfo,
            InSDKVersion: c_int,
        ) -> Result;
        pub fn NVSDK_NGX_VULKAN_Init(
            InApplicationId: c_ulonglong,
            InApplicationDataPath: *const WChar,
            InInstance: vk::Instance,
            InPD: vk::PhysicalDevice,
            InDevice: vk::Device,
            InGIPA: Option<vk::PFN_vkGetInstanceProcAddr>,
            InGDPA: Option<vk::PFN_vkGetDeviceProcAddr>,
            InFeatureInfo: *const FeatureCommonInfo,
            InSDKVersion: c_int,
        ) -> Result;
        pub fn NVSDK_NGX_VULKAN_Shutdown1(InDevice: vk::Device) -> Result;
        pub fn NVSDK_NGX_VULKAN_GetCapabilityParameters(
            OutParameters: *mut *mut Parameter,
        ) -> Result;
        pub fn NVSDK_NGX_VULKAN_AllocateParameters(OutParameters: *mut *mut Parameter) -> Result;
        pub fn NVSDK_NGX_VULKAN_DestroyParameters(InParameters: *mut Parameter) -> Result;
        pub fn NVSDK_NGX_VULKAN_GetScratchBufferSize(
            InFeatureId: c_int,
            InParameters: *const Parameter,
            OutSizeInBytes: *mut usize,
        ) -> Result;
        pub fn NVSDK_NGX_VULKAN_CreateFeature1(
            InDevice: vk::Device,
            InCmdList: vk::CommandBuffer,
            InFeatureID: c_int,
            InParameters: *mut Parameter,
            OutHandle: *mut *mut Handle,
        ) -> Result;
        pub fn NVSDK_NGX_VULKAN_ReleaseFeature(InHandle: *mut Handle) -> Result;
        pub fn NVSDK_NGX_VULKAN_EvaluateFeature_C(
            InCmdList: vk::CommandBuffer,
            InFeatureHandle: *const Handle,
            InParameters: *const Parameter,
            InCallback: Option<unsafe extern "C" fn(progress: f32, cancel: *mut bool)>,
        ) -> Result;
        pub fn NVSDK_NGX_VULKAN_GetFeatureRequirements(
            Instance: vk::Instance,
            PhysicalDevice: vk::PhysicalDevice,
            FeatureDiscoveryInfo: *const FeatureDiscoveryInfo,
            OutSupported: *mut FeatureRequirement,
        ) -> Result;
        pub fn NVSDK_NGX_VULKAN_GetFeatureInstanceExtensionRequirements(
            FeatureDiscoveryInfo: *const FeatureDiscoveryInfo,
            OutExtensionCount: *mut u32,
            OutExtensionProperties: *mut *mut vk::ExtensionProperties,
        ) -> Result;
        pub fn NVSDK_NGX_VULKAN_GetFeatureDeviceExtensionRequirements(
            Instance: vk::Instance,
            PhysicalDevice: vk::PhysicalDevice,
            FeatureDiscoveryInfo: *const FeatureDiscoveryInfo,
            OutExtensionCount: *mut u32,
            OutExtensionProperties: *mut *mut vk::ExtensionProperties,
        ) -> Result;

        pub fn NVSDK_NGX_Parameter_SetI(p: *mut Parameter, name: *const c_char, v: c_int);
        pub fn NVSDK_NGX_Parameter_SetUI(p: *mut Parameter, name: *const c_char, v: c_uint);
        pub fn NVSDK_NGX_Parameter_SetF(p: *mut Parameter, name: *const c_char, v: f32);
        pub fn NVSDK_NGX_Parameter_SetULL(p: *mut Parameter, name: *const c_char, v: c_ulonglong);
        pub fn NVSDK_NGX_Parameter_SetVoidPointer(
            p: *mut Parameter,
            name: *const c_char,
            v: *mut c_void,
        );
        pub fn NVSDK_NGX_Parameter_GetI(
            p: *mut Parameter,
            name: *const c_char,
            out: *mut c_int,
        ) -> Result;
        pub fn NVSDK_NGX_Parameter_GetUI(
            p: *mut Parameter,
            name: *const c_char,
            out: *mut c_uint,
        ) -> Result;
        pub fn NVSDK_NGX_Parameter_GetVoidPointer(
            p: *mut Parameter,
            name: *const c_char,
            out: *mut *mut c_void,
        ) -> Result;
    }

    /// The parameter names (`NVSDK_NGX_Parameter_*`) the DLSS-D helpers set, as C strings.
    pub mod names {
        use std::ffi::CStr;
        pub const WIDTH: &CStr = c"Width";
        pub const HEIGHT: &CStr = c"Height";
        pub const OUT_WIDTH: &CStr = c"OutWidth";
        pub const OUT_HEIGHT: &CStr = c"OutHeight";
        pub const PERF_QUALITY_VALUE: &CStr = c"PerfQualityValue";
        pub const CREATE_FLAGS: &CStr = c"DLSS.Feature.Create.Flags";
        pub const ENABLE_OUTPUT_SUBRECTS: &CStr = c"DLSS.Enable.Output.Subrects";
        pub const CREATION_NODE_MASK: &CStr = c"CreationNodeMask";
        pub const VISIBILITY_NODE_MASK: &CStr = c"VisibilityNodeMask";
        pub const DENOISE_MODE: &CStr = c"DLSS.Denoise.Mode";
        pub const ROUGHNESS_MODE: &CStr = c"DLSS.Roughness.Mode";
        pub const USE_HW_DEPTH: &CStr = c"DLSS.Use.HW.Depth";
        pub const RR_PRESET_DLAA: &CStr = c"RayReconstruction.Hint.Render.Preset.DLAA";
        pub const RR_PRESET_QUALITY: &CStr = c"RayReconstruction.Hint.Render.Preset.Quality";
        pub const RR_PRESET_BALANCED: &CStr = c"RayReconstruction.Hint.Render.Preset.Balanced";
        pub const RR_PRESET_PERFORMANCE: &CStr =
            c"RayReconstruction.Hint.Render.Preset.Performance";

        pub const COLOR: &CStr = c"Color";
        pub const OUTPUT: &CStr = c"Output";
        pub const DEPTH: &CStr = c"Depth";
        pub const MOTION_VECTORS: &CStr = c"MotionVectors";
        pub const JITTER_X: &CStr = c"Jitter.Offset.X";
        pub const JITTER_Y: &CStr = c"Jitter.Offset.Y";
        pub const RESET: &CStr = c"Reset";
        pub const MV_SCALE_X: &CStr = c"MV.Scale.X";
        pub const MV_SCALE_Y: &CStr = c"MV.Scale.Y";
        pub const RENDER_SUBRECT_WIDTH: &CStr = c"DLSS.Render.Subrect.Dimensions.Width";
        pub const RENDER_SUBRECT_HEIGHT: &CStr = c"DLSS.Render.Subrect.Dimensions.Height";
        pub const OUTPUT_SUBRECT_X: &CStr = c"DLSS.Output.Subrect.Base.X";
        pub const OUTPUT_SUBRECT_Y: &CStr = c"DLSS.Output.Subrect.Base.Y";
        pub const TRANSPARENCY_MASK: &CStr = c"TransparencyMask";
        pub const EXPOSURE_TEXTURE: &CStr = c"ExposureTexture";
        pub const BIAS_CURRENT_COLOR_MASK: &CStr = c"DLSS.Input.Bias.Current.Color.Mask";
        pub const PRE_EXPOSURE: &CStr = c"DLSS.Pre.Exposure";
        pub const EXPOSURE_SCALE: &CStr = c"DLSS.Exposure.Scale";
        pub const FRAME_TIME_DELTA_MS: &CStr = c"FrameTimeDeltaInMsec";
        pub const DIFFUSE_ALBEDO: &CStr = c"DLSS.Input.DiffuseAlbedo";
        pub const SPECULAR_ALBEDO: &CStr = c"DLSS.Input.SpecularAlbedo";
        pub const NORMALS: &CStr = c"GBuffer.Normals";
        pub const ROUGHNESS: &CStr = c"GBuffer.Roughness";
        pub const SPECULAR_HIT_DISTANCE: &CStr = c"DLSSD.SpecularHitDistance";
        pub const SPECULAR_MOTION_VECTORS: &CStr = c"GBuffer.SpecularMvec";
        pub const WORLD_TO_VIEW: &CStr = c"WorldToViewMatrix";
        pub const VIEW_TO_CLIP: &CStr = c"ViewToClipMatrix";
        pub const TONEMAPPER_TYPE: &CStr = c"TonemapperType";
        pub const INDICATOR_INVERT_X: &CStr = c"DLSS.Indicator.Invert.X.Axis";
        pub const INDICATOR_INVERT_Y: &CStr = c"DLSS.Indicator.Invert.Y.Axis";
        /// The inputs' sub-rectangle origins, which `NGX_VULKAN_EVALUATE_DLSSD_EXT` always sets
        /// (0, 0 for whole images), in the SDK's own spellings.
        pub const SUBRECT_BASES: [(&CStr, &CStr); 9] = [
            (
                c"DLSS.Input.Color.Subrect.Base.X",
                c"DLSS.Input.Color.Subrect.Base.Y",
            ),
            (
                c"DLSS.Input.Depth.Subrect.Base.X",
                c"DLSS.Input.Depth.Subrect.Base.Y",
            ),
            (
                c"DLSS.Input.MV.Subrect.Base.X",
                c"DLSS.Input.MV.Subrect.Base.Y",
            ),
            (
                c"DLSS.Input.Translucency.Subrect.Base.X",
                c"DLSS.Input.Translucency.Subrect.Base.Y",
            ),
            (
                c"DLSS.Input.Bias.Current.Color.Subrect.Base.X",
                c"DLSS.Input.Bias.Current.Color.Subrect.Base.Y",
            ),
            (
                c"DLSS.Input.DiffuseAlbedo.Subrect.Base.X",
                c"DLSS.Input.DiffuseAlbedo.Subrect.Base.Y",
            ),
            (
                c"DLSS.Input.SpecularAlbedo.Subrect.Base.X",
                c"DLSS.Input.SpecularAlbedo.Subrect.Base.Y",
            ),
            (
                c"DLSS.Input.Normals.Subrect.Base.X",
                c"DLSS.Input.Normals.Subrect.Base.Y",
            ),
            (
                c"DLSS.Input.Roughness.Subrect.Base.X",
                c"DLSS.Input.Roughness.Subrect.Base.Y",
            ),
        ];

        pub const RR_AVAILABLE: &CStr = c"SuperSamplingDenoising.Available";
        pub const RR_NEEDS_UPDATED_DRIVER: &CStr = c"SuperSamplingDenoising.NeedsUpdatedDriver";
        pub const RR_MIN_DRIVER_MAJOR: &CStr = c"SuperSamplingDenoising.MinDriverVersionMajor";
        pub const RR_MIN_DRIVER_MINOR: &CStr = c"SuperSamplingDenoising.MinDriverVersionMinor";
        pub const RR_FEATURE_INIT_RESULT: &CStr = c"SuperSamplingDenoising.FeatureInitResult";
        pub const RR_OPTIMAL_SETTINGS: &CStr = c"DLSSDOptimalSettingsCallback";
        pub const RTX_VALUE: &CStr = c"RTXValue";
        pub const DYNAMIC_MAX_WIDTH: &CStr = c"DLSS.Get.Dynamic.Max.Render.Width";
        pub const DYNAMIC_MAX_HEIGHT: &CStr = c"DLSS.Get.Dynamic.Max.Render.Height";
        pub const DYNAMIC_MIN_WIDTH: &CStr = c"DLSS.Get.Dynamic.Min.Render.Width";
        pub const DYNAMIC_MIN_HEIGHT: &CStr = c"DLSS.Get.Dynamic.Min.Render.Height";
    }
}

/// Horizon's NGX project id (`NVSDK_NGX_VULKAN_Init_with_ProjectID`), a fixed UUID with the
/// custom engine type; `HORIZON_DLSS_PROJECT_ID` overrides it (an id NVIDIA assigns, for a release).
const PROJECT_ID: &CStr = c"6f1d0c1e-5c55-4b6a-9d7e-3a0b8d0e4c21";
const ENGINE_VERSION: &CStr = c"1.0";

/// What NGX needs to know of Horizon before the instance or the device exists.
struct Discovery {
    project: CString,
    data_path: Vec<ffi::WChar>,
    paths: Vec<Vec<ffi::WChar>>,
}

impl Discovery {
    fn new(dll_dirs: &[std::path::PathBuf], data_path: &std::path::Path) -> Self {
        let project = std::env::var("HORIZON_DLSS_PROJECT_ID")
            .ok()
            .and_then(|v| CString::new(v).ok())
            .unwrap_or_else(|| PROJECT_ID.to_owned());
        Self {
            project,
            data_path: wide(data_path),
            paths: dll_dirs.iter().map(|d| wide(d)).collect(),
        }
    }

    /// Calls `f` with the discovery info for `feature` (the pointers live as long as `self`).
    fn with<R>(&self, feature: i32, f: impl FnOnce(&ffi::FeatureDiscoveryInfo) -> R) -> R {
        let paths: Vec<*const ffi::WChar> = self.paths.iter().map(|p| p.as_ptr()).collect();
        let common = self.common(&paths);
        let info = ffi::FeatureDiscoveryInfo {
            SDKVersion: ffi::VERSION_API,
            FeatureID: feature,
            Identifier: ffi::Application_Identifier {
                IdentifierType: 1,
                v: ffi::IdentifierValue {
                    ProjectDesc: ffi::ProjectIdDescription {
                        ProjectId: self.project.as_ptr(),
                        EngineType: ffi::ENGINE_CUSTOM,
                        EngineVersion: ENGINE_VERSION.as_ptr(),
                    },
                },
            },
            ApplicationDataPath: self.data_path.as_ptr(),
            FeatureInfo: &common,
        };
        f(&info)
    }

    fn common(&self, paths: &[*const ffi::WChar]) -> ffi::FeatureCommonInfo {
        ffi::FeatureCommonInfo {
            PathListInfo: ffi::PathListInfo {
                Path: paths.as_ptr(),
                Length: paths.len() as u32,
            },
            InternalData: std::ptr::null_mut(),
            LoggingInfo: ffi::LoggingInfo {
                LoggingCallback: Some(log_callback),
                MinimumLoggingLevel: ngx_log_level(),
                DisableOtherLoggingSinks: false,
            },
        }
    }
}

/// NGX's own logging level (`NVSDK_NGX_Logging_Level`): `HORIZON_DLSS_RR_LOG=2` verbose, `=0` off,
/// else on.
fn ngx_log_level() -> i32 {
    match std::env::var("HORIZON_DLSS_RR_LOG").as_deref() {
        Ok("0") => 0,
        Ok("2") => 2,
        _ => 1,
    }
}

/// NGX's log, into Horizon's: at debug level, or info with `HORIZON_DLSS_RR_LOG` set to 1 or 2.
unsafe extern "C" fn log_callback(message: *const c_char, _level: i32, _source: i32) {
    if !message.is_null() {
        // Safety: NGX hands a NUL-terminated string valid for the call.
        let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
        if matches!(
            std::env::var("HORIZON_DLSS_RR_LOG").as_deref(),
            Ok("1" | "2")
        ) {
            log::info!("NGX: {}", text.trim_end());
        } else {
            log::debug!("NGX: {}", text.trim_end());
        }
    }
}

/// `path` as a NUL-terminated `wchar_t` string.
fn wide(path: &std::path::Path) -> Vec<ffi::WChar> {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    let mut out: Vec<ffi::WChar> = text.encode_utf16().collect();
    #[cfg(not(windows))]
    let mut out: Vec<ffi::WChar> = text.chars().map(u32::from).collect();
    out.push(0);
    out
}

/// The extension names of an NGX requirement list.
///
/// # Safety
/// `list` and `count` come from one successful NGX requirement call.
unsafe fn names(list: *const vk::ExtensionProperties, count: u32) -> Vec<CString> {
    if list.is_null() || count == 0 {
        return Vec::new();
    }
    // Safety: NGX's array of `count` properties, owned by NGX.
    unsafe { std::slice::from_raw_parts(list, count as usize) }
        .iter()
        .filter_map(|e| e.extension_name_as_c_str().ok().map(CStr::to_owned))
        .collect()
}

/// The Vulkan instance extensions Ray Reconstruction needs, to add to the instance before it is
/// created (wgpu-hal `Instance::init_with_callback`, or the OpenXR runtime's
/// `xrCreateVulkanInstanceKHR` in VR). `dll_dirs` are where `nvngx_dlssd.dll` may be.
pub fn required_instance_extensions(
    dll_dirs: &[std::path::PathBuf],
    data_path: &std::path::Path,
) -> Result<Vec<CString>, Error> {
    let discovery = Discovery::new(dll_dirs, data_path);
    discovery.with(ffi::FEATURE_RAY_RECONSTRUCTION, |info| {
        let mut count = 0u32;
        let mut list = std::ptr::null_mut();
        // Safety: a query; the discovery info's pointers live for the call.
        check(
            "NVSDK_NGX_VULKAN_GetFeatureInstanceExtensionRequirements",
            unsafe {
                ffi::NVSDK_NGX_VULKAN_GetFeatureInstanceExtensionRequirements(
                    info, &mut count, &mut list,
                )
            },
        )?;
        // Safety: what the call above returned.
        Ok(unsafe { names(list, count) })
    })
}

/// The device extensions Ray Reconstruction needs on `physical` (to add in wgpu-hal's
/// `open_with_callback`, or to `xrCreateVulkanDeviceKHR`'s create info in VR), once the GPU
/// supports the feature (`NVSDK_NGX_VULKAN_GetFeatureRequirements`); `Err` on another GPU.
///
/// # Safety
/// `physical` is a physical device of `instance`, which is alive.
pub unsafe fn required_device_extensions(
    instance: vk::Instance,
    physical: vk::PhysicalDevice,
    dll_dirs: &[std::path::PathBuf],
    data_path: &std::path::Path,
) -> Result<Vec<CString>, Error> {
    let discovery = Discovery::new(dll_dirs, data_path);
    discovery.with(ffi::FEATURE_RAY_RECONSTRUCTION, |info| {
        // Safety: the caller's handles, a zeroed out-structure.
        let mut requirement: ffi::FeatureRequirement = unsafe { std::mem::zeroed() };
        check("NVSDK_NGX_VULKAN_GetFeatureRequirements", unsafe {
            ffi::NVSDK_NGX_VULKAN_GetFeatureRequirements(instance, physical, info, &mut requirement)
        })?;
        if requirement.FeatureSupported != ffi::FEATURE_SUPPORTED {
            return Err(Error {
                call: "Ray Reconstruction is not supported on this GPU or driver",
                result: requirement.FeatureSupported as u32,
            });
        }
        let mut count = 0u32;
        let mut list = std::ptr::null_mut();
        check(
            "NVSDK_NGX_VULKAN_GetFeatureDeviceExtensionRequirements",
            unsafe {
                ffi::NVSDK_NGX_VULKAN_GetFeatureDeviceExtensionRequirements(
                    instance, physical, info, &mut count, &mut list,
                )
            },
        )?;
        Ok(unsafe { names(list, count) })
    })
}

/// Extension names for wgpu-hal's callbacks, which take `&'static CStr`: leaked, once per
/// instance or device creation (a few dozen bytes for the process's life).
pub fn leak(names: Vec<CString>) -> Vec<&'static CStr> {
    names
        .into_iter()
        .map(|n| &*Box::leak(n.into_boxed_c_str()))
        .collect()
}

/// The devices NGX is initialised on.
static INITIALISED: std::sync::Mutex<Vec<vk::Device>> = std::sync::Mutex::new(Vec::new());

/// NGX on one Vulkan device: initialised once, shut down when the last feature using it goes.
pub struct Ngx {
    /// Keeps the Vulkan device alive until NGX is shut down on it (`Drop`).
    wgpu_device: wgpu::Device,
    device: vk::Device,
    caps: *mut ffi::Parameter,
}

// The handles are used from the render thread only; NGX allows calls from any one thread at a time.
unsafe impl Send for Ngx {}
unsafe impl Sync for Ngx {}

/// NGX's render sizes for one output size and quality mode (`NGX_DLSSD_GET_OPTIMAL_SETTINGS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptimalSettings {
    pub render: [u32; 2],
    /// The range dynamic resolution may use (both `render` when NGX gives none).
    pub min: [u32; 2],
    pub max: [u32; 2],
}

impl Ngx {
    /// NGX on `device`'s Vulkan device, with Ray Reconstruction available on it; `Err` when the
    /// device is not Vulkan, the DLL is missing, the GPU or driver lacks the feature, or init
    /// fails. `dll_dirs` are searched for `nvngx_dlssd.dll`; `data_path` takes NGX's logs.
    pub fn init(
        device: &wgpu::Device,
        dll_dirs: &[std::path::PathBuf],
        data_path: &std::path::Path,
    ) -> Result<Arc<Self>, Error> {
        let not_vulkan = Error {
            call: "a Vulkan device",
            result: ffi::RESULT_FAIL,
        };
        let (raw_device, instance, physical, gipa, gdpa) = {
            // Safety: read and copied, dropped before wgpu takes the device back.
            let hal = unsafe { device.as_hal::<Vulkan>() }.ok_or(not_vulkan)?;
            let shared = hal.shared_instance();
            (
                hal.raw_device().handle(),
                shared.raw_instance().handle(),
                hal.raw_physical_device(),
                shared.entry().static_fn().get_instance_proc_addr,
                // What dlss_wgpu passes too: the instance's own vkGetDeviceProcAddr.
                shared.raw_instance().fp_v1_0().get_device_proc_addr,
            )
        };
        // One NGX a device: `Shutdown1` on one would end the other's.
        let mut initialised = INITIALISED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if initialised.contains(&raw_device) {
            return Err(Error {
                call: "NGX is already initialised on this device",
                result: ffi::RESULT_FAIL,
            });
        }
        let discovery = Discovery::new(dll_dirs, data_path);
        let paths: Vec<*const ffi::WChar> = discovery.paths.iter().map(|p| p.as_ptr()).collect();
        let common = discovery.common(&paths);
        // Safety: the device's own handles and loader; the strings live for the call.
        check("NVSDK_NGX_VULKAN_Init_with_ProjectID", unsafe {
            ffi::NVSDK_NGX_VULKAN_Init_with_ProjectID(
                discovery.project.as_ptr(),
                ffi::ENGINE_CUSTOM,
                ENGINE_VERSION.as_ptr(),
                discovery.data_path.as_ptr(),
                instance,
                physical,
                raw_device,
                Some(gipa),
                Some(gdpa),
                &common,
                ffi::VERSION_API,
            )
        })?;
        let mut caps = std::ptr::null_mut();
        // Safety: NGX initialised above.
        let got = unsafe { ffi::NVSDK_NGX_VULKAN_GetCapabilityParameters(&mut caps) };
        // From here on, dropping `ngx` shuts NGX down again.
        initialised.push(raw_device);
        drop(initialised);
        let ngx = Arc::new(Self {
            wgpu_device: device.clone(),
            device: raw_device,
            caps,
        });
        check("NVSDK_NGX_VULKAN_GetCapabilityParameters", got)?;
        let available = ngx.get_i(ffi::names::RR_AVAILABLE).unwrap_or(0);
        if available == 0 {
            let driver = ngx.get_i(ffi::names::RR_NEEDS_UPDATED_DRIVER).unwrap_or(0);
            if driver != 0 {
                log::warn!(
                    "DLSS-RR: the driver is too old for Ray Reconstruction (NGX asks for {}.{} or newer)",
                    ngx.get_ui(ffi::names::RR_MIN_DRIVER_MAJOR).unwrap_or(0),
                    ngx.get_ui(ffi::names::RR_MIN_DRIVER_MINOR).unwrap_or(0),
                );
            }
            let result = ngx
                .get_i(ffi::names::RR_FEATURE_INIT_RESULT)
                .map_or(ffi::RESULT_FAIL, |r| r as u32);
            return Err(Error {
                call: if driver != 0 {
                    "Ray Reconstruction needs a newer driver"
                } else {
                    "Ray Reconstruction is not available (SuperSamplingDenoising.Available = 0; \
                     is nvngx_dlssd.dll in one of the folders above?)"
                },
                result,
            });
        }
        Ok(ngx)
    }

    fn get_i(&self, name: &CStr) -> Option<i32> {
        let mut v = 0;
        // Safety: the capability parameters NGX returned, alive until `Drop`.
        let r = unsafe { ffi::NVSDK_NGX_Parameter_GetI(self.caps, name.as_ptr(), &mut v) };
        (!ffi::failed(r)).then_some(v)
    }

    fn get_ui(&self, name: &CStr) -> Option<u32> {
        let mut v = 0;
        // Safety: as `get_i`.
        let r = unsafe { ffi::NVSDK_NGX_Parameter_GetUI(self.caps, name.as_ptr(), &mut v) };
        (!ffi::failed(r)).then_some(v)
    }

    /// The render size NGX advises for `output` at `quality`: what the SDK's inline
    /// `NGX_DLSSD_GET_OPTIMAL_SETTINGS` does, on the capability parameters.
    pub fn optimal_settings(
        &self,
        output: [u32; 2],
        quality: Quality,
    ) -> Result<OptimalSettings, Error> {
        use ffi::names as n;
        let mut callback = std::ptr::null_mut();
        // Safety: the capability parameters, alive until `Drop`.
        let r = unsafe {
            ffi::NVSDK_NGX_Parameter_GetVoidPointer(
                self.caps,
                n::RR_OPTIMAL_SETTINGS.as_ptr(),
                &mut callback,
            )
        };
        if ffi::failed(r) || callback.is_null() {
            return Err(Error {
                call: "DLSSDOptimalSettingsCallback (none: an old DLL or driver)",
                result: ffi::RESULT_FAIL_OUT_OF_DATE,
            });
        }
        // Safety: NGX's function pointer under that key has this type (the SDK's helper casts it
        // the same way).
        let callback =
            unsafe { std::mem::transmute::<*mut c_void, ffi::OptimalSettingsCallback>(callback) };
        // Safety: the capability parameters; the callback reads and writes them.
        unsafe {
            let p = self.caps;
            ffi::NVSDK_NGX_Parameter_SetUI(p, n::WIDTH.as_ptr(), output[0]);
            ffi::NVSDK_NGX_Parameter_SetUI(p, n::HEIGHT.as_ptr(), output[1]);
            ffi::NVSDK_NGX_Parameter_SetI(p, n::PERF_QUALITY_VALUE.as_ptr(), quality.value());
            // The helper sets it too: "some older DLSS dlls still expect this value to be set".
            ffi::NVSDK_NGX_Parameter_SetI(p, n::RTX_VALUE.as_ptr(), 0);
            check("DLSSDOptimalSettingsCallback", callback(p))?;
        }
        let pair = |w: &CStr, h: &CStr| Some([self.get_ui(w)?, self.get_ui(h)?]);
        let render = pair(n::OUT_WIDTH, n::OUT_HEIGHT).ok_or(Error {
            call: "DLSSDOptimalSettingsCallback (no OutWidth/OutHeight)",
            result: ffi::RESULT_FAIL,
        })?;
        Ok(OptimalSettings {
            render,
            min: pair(n::DYNAMIC_MIN_WIDTH, n::DYNAMIC_MIN_HEIGHT).unwrap_or(render),
            max: pair(n::DYNAMIC_MAX_WIDTH, n::DYNAMIC_MAX_HEIGHT).unwrap_or(render),
        })
    }
}

impl Drop for Ngx {
    fn drop(&mut self) {
        // NGX's work on the GPU (feature setup) ends before it shuts down.
        let _ = self.wgpu_device.poll(wgpu::PollType::wait_indefinitely());
        // Safety: the parameters and the device NGX was initialised on; every feature is
        // released first (each holds an `Arc` of this), and `wgpu_device` keeps the VkDevice
        // alive until after `Shutdown1`.
        unsafe {
            if !self.caps.is_null() {
                ffi::NVSDK_NGX_VULKAN_DestroyParameters(self.caps);
            }
            let r = ffi::NVSDK_NGX_VULKAN_Shutdown1(self.device);
            INITIALISED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|d| *d != self.device);
            if ffi::failed(r) {
                log::warn!("DLSS-RR: NVSDK_NGX_VULKAN_Shutdown1 failed (NGX result {r:#010x})");
            } else {
                log::info!("DLSS-RR: NGX shut down");
            }
        }
    }
}

/// A quality mode: the share of the output's pixels the rays are traced at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// Input = output (denoising and anti-aliasing only).
    Dlaa,
    Quality,
    Balanced,
    Performance,
    UltraPerformance,
}

impl Quality {
    pub const ALL: [Self; 5] = [
        Self::Dlaa,
        Self::Quality,
        Self::Balanced,
        Self::Performance,
        Self::UltraPerformance,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Dlaa => "DLAA",
            Self::Quality => "Quality",
            Self::Balanced => "Balanced",
            Self::Performance => "Performance",
            Self::UltraPerformance => "Ultra Performance",
        }
    }

    /// `HORIZON_DLSS_RR_QUALITY`'s spelling of a mode, case and blanks ignored ("" is DLAA).
    pub fn parse(v: &str) -> Option<Self> {
        match v
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_', ' '], "")
            .as_str()
        {
            "" | "dlaa" => Some(Self::Dlaa),
            "quality" => Some(Self::Quality),
            "balanced" => Some(Self::Balanced),
            "performance" => Some(Self::Performance),
            "ultra" | "ultraperformance" => Some(Self::UltraPerformance),
            _ => None,
        }
    }

    fn value(self) -> i32 {
        match self {
            Self::Dlaa => ffi::PERF_DLAA,
            Self::Quality => ffi::PERF_MAX_QUALITY,
            Self::Balanced => ffi::PERF_BALANCED,
            Self::Performance => ffi::PERF_MAX_PERF,
            Self::UltraPerformance => ffi::PERF_ULTRA_PERFORMANCE,
        }
    }

    /// The input size DLSS's published scale factors give for `output` (1, 1/1.5, 1/1.72, 1/2,
    /// 1/3 a side), rounded up: what `Settings::render` defaults to when NGX's optimal settings
    /// are not asked.
    pub fn render_size(self, output: [u32; 2]) -> [u32; 2] {
        let inverse = match self {
            Self::Dlaa => 1.0,
            Self::Quality => 1.5,
            Self::Balanced => 1.0 / 0.58,
            Self::Performance => 2.0,
            Self::UltraPerformance => 3.0,
        };
        output.map(|v| ((v as f64 / inverse).ceil() as u32).max(1))
    }
}

/// How a feature instance is made: one per view (the window, each VR eye), as each has its own
/// history.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// The rays' (and the G-buffer's) size, at most `output`.
    pub render: [u32; 2],
    /// The denoised, upscaled image's size.
    pub output: [u32; 2],
    pub quality: Quality,
    /// Horizon's depth is reversed Z (`GreaterEqual`, Depth32Float): `DepthInverted`.
    pub depth_inverted: bool,
    /// The colour is linear HDR (the scene before the exposure): `IsHDR`.
    pub hdr: bool,
    /// The motion vectors are at the render size (`MVLowRes`) and include the jitter
    /// (`MVJittered`).
    pub mv_low_res: bool,
    pub mv_jittered: bool,
    /// NGX computes the exposure itself (`AutoExposure`) instead of reading `Inputs::exposure`.
    pub auto_exposure: bool,
    /// Roughness rides in the normals' w (`DLSS.Roughness.Mode` packed), with no roughness image.
    pub roughness_packed: bool,
    /// The depth is the hardware depth buffer, not a linear view depth.
    pub hardware_depth: bool,
    /// Output to a sub-rectangle of the output image (`Inputs::output_offset`): VR's sharp zone.
    pub output_subrects: bool,
}

/// One image NGX reads or writes: the raw handles of a wgpu texture and its view.
#[derive(Clone, Copy, Debug)]
pub struct Image {
    pub view: vk::ImageView,
    pub image: vk::Image,
    pub format: vk::Format,
    pub size: [u32; 2],
    pub aspect: vk::ImageAspectFlags,
}

impl Image {
    /// `view`'s handles (the whole of its texture's first mip and layer), `None` off Vulkan.
    ///
    /// # Safety
    /// The handles are used only while `view` and its texture live (for the frame that records
    /// the evaluation), and the texture is in the layout NGX expects when the commands run
    /// (`DlssRr::evaluate`).
    pub unsafe fn of(adapter: &wgpu::Adapter, view: &wgpu::TextureView) -> Option<Self> {
        let texture = view.texture();
        let format = texture.format();
        // Safety: read only, dropped at once.
        let raw_format = unsafe { adapter.as_hal::<Vulkan>() }?.texture_format_as_raw(format);
        // Safety: the view's and the texture's own handles, alive as long as they are.
        let raw_view = unsafe { view.as_hal::<Vulkan>()?.raw_handle() };
        let raw_image = unsafe { texture.as_hal::<Vulkan>()?.raw_handle() };
        let aspect = if format.is_depth_stencil_format() {
            vk::ImageAspectFlags::DEPTH
        } else {
            vk::ImageAspectFlags::COLOR
        };
        Some(Self {
            view: raw_view,
            image: raw_image,
            format: raw_format,
            size: [texture.width(), texture.height()],
            aspect,
        })
    }

    fn resource(&self, read_write: bool) -> ffi::Resource_VK {
        ffi::Resource_VK {
            Resource: ffi::ResourceUnion_VK {
                ImageViewInfo: ffi::ImageViewInfo_VK {
                    ImageView: self.view,
                    Image: self.image,
                    SubresourceRange: vk::ImageSubresourceRange {
                        aspect_mask: self.aspect,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    },
                    Format: self.format,
                    Width: self.size[0],
                    Height: self.size[1],
                },
            },
            Type: ffi::RESOURCE_VK_IMAGEVIEW,
            ReadWrite: read_write,
        }
    }
}

/// Everything one evaluation reads, for one view and one frame (docs/rt-dlss-rr.md, "Inputs").
/// Images are at `render` size unless said otherwise; `None` leaves an optional input out.
#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    /// This frame's noisy radiance, one sample a pixel, *not* accumulated, with the albedo
    /// multiplied in (the lit scene as the world pass composes it), linear HDR.
    pub color: Image,
    /// The denoised image, `output` size, written (a storage image: `GENERAL` layout).
    pub output: Image,
    /// The depth (reversed-Z hardware depth with `Settings::hardware_depth`).
    pub depth: Image,
    /// Per-pixel motion in pixels toward last frame's position (`RG16F`/`RG32F`), the camera's and
    /// the entities' own.
    pub motion_vectors: Image,
    /// The diffuse albedo (the block's texel colour, linear) and the specular albedo (the
    /// preintegrated environment BRDF, not F0).
    pub diffuse_albedo: Image,
    pub specular_albedo: Image,
    /// World- or view-space normals (xyz, unit), roughness in w with `Settings::roughness_packed`.
    pub normals: Image,
    /// Linear roughness, unless packed.
    pub roughness: Option<Image>,
    /// The specular ray's hit distance (with `world_to_view` and `view_to_clip`), or specular
    /// motion vectors: one of the two guides the reflections.
    pub specular_hit_distance: Option<Image>,
    pub specular_motion_vectors: Option<Image>,
    /// A 1x1 exposure, unless `Settings::auto_exposure`.
    pub exposure: Option<Image>,
    /// Pixels to trust this frame's colour more (particles, the Eye of Aldhani's meteors).
    pub bias_current_color_mask: Option<Image>,
    pub transparency_mask: Option<Image>,
    /// The projection's jitter this frame, in render pixels (TAA's `jitter`, x right, y down).
    pub jitter: [f32; 2],
    /// What turns the motion vectors' values into render pixels (1, 1 when already in pixels).
    pub mv_scale: [f32; 2],
    /// The part of the inputs used (the render size, or less with dynamic resolution).
    pub render_size: [u32; 2],
    /// Where the output's sub-rectangle starts, with `Settings::output_subrects`.
    pub output_offset: [u32; 2],
    /// The history is wrong: a teleport, a dimension change, a new world, a resize.
    pub reset: bool,
    pub pre_exposure: f32,
    pub exposure_scale: f32,
    pub frame_time_ms: f32,
    /// Row-major world-to-view and view-to-clip matrices, which the hit distance guide needs.
    pub world_to_view: Option<[f32; 16]>,
    pub view_to_clip: Option<[f32; 16]>,
}

/// One Ray Reconstruction feature instance (one view's history), released on drop.
pub struct DlssRr {
    /// Keeps NGX initialised until this feature is released (`Ngx`'s drop shuts it down).
    _ngx: Arc<Ngx>,
    handle: *mut ffi::Handle,
    params: *mut ffi::Parameter,
    settings: Settings,
}

unsafe impl Send for DlssRr {}

impl DlssRr {
    /// Makes the feature for `settings`, recording its setup into `cmd` (a command buffer of
    /// `ngx`'s device in the recording state, submitted before the first `evaluate`).
    ///
    /// # Safety
    /// `cmd` is recording, on the device `ngx` was initialised on.
    pub unsafe fn new(
        ngx: &Arc<Ngx>,
        cmd: vk::CommandBuffer,
        settings: Settings,
    ) -> Result<Self, Error> {
        let mut params = std::ptr::null_mut();
        // Safety: NGX initialised (`ngx`).
        check("NVSDK_NGX_VULKAN_AllocateParameters", unsafe {
            ffi::NVSDK_NGX_VULKAN_AllocateParameters(&mut params)
        })?;
        let mut feature = Self {
            _ngx: ngx.clone(),
            handle: std::ptr::null_mut(),
            params,
            settings,
        };
        let mut flags = 0;
        for (on, flag) in [
            (settings.hdr, ffi::FLAG_IS_HDR),
            (settings.mv_low_res, ffi::FLAG_MV_LOW_RES),
            (settings.mv_jittered, ffi::FLAG_MV_JITTERED),
            (settings.depth_inverted, ffi::FLAG_DEPTH_INVERTED),
            (settings.auto_exposure, ffi::FLAG_AUTO_EXPOSURE),
        ] {
            if on {
                flags |= flag;
            }
        }
        use ffi::names as n;
        // What NGX_VULKAN_CREATE_DLSSD_EXT1 sets, in its order.
        // Safety: the parameters allocated above, names NUL-terminated.
        unsafe {
            let p = feature.params;
            ffi::NVSDK_NGX_Parameter_SetUI(p, n::CREATION_NODE_MASK.as_ptr(), 1);
            ffi::NVSDK_NGX_Parameter_SetUI(p, n::VISIBILITY_NODE_MASK.as_ptr(), 1);
            ffi::NVSDK_NGX_Parameter_SetUI(p, n::WIDTH.as_ptr(), settings.render[0]);
            ffi::NVSDK_NGX_Parameter_SetUI(p, n::HEIGHT.as_ptr(), settings.render[1]);
            ffi::NVSDK_NGX_Parameter_SetUI(p, n::OUT_WIDTH.as_ptr(), settings.output[0]);
            ffi::NVSDK_NGX_Parameter_SetUI(p, n::OUT_HEIGHT.as_ptr(), settings.output[1]);
            ffi::NVSDK_NGX_Parameter_SetI(
                p,
                n::PERF_QUALITY_VALUE.as_ptr(),
                settings.quality.value(),
            );
            ffi::NVSDK_NGX_Parameter_SetI(p, n::CREATE_FLAGS.as_ptr(), flags);
            ffi::NVSDK_NGX_Parameter_SetI(
                p,
                n::ENABLE_OUTPUT_SUBRECTS.as_ptr(),
                i32::from(settings.output_subrects),
            );
            ffi::NVSDK_NGX_Parameter_SetI(p, n::DENOISE_MODE.as_ptr(), ffi::DENOISE_DL_UNIFIED);
            ffi::NVSDK_NGX_Parameter_SetUI(
                p,
                n::ROUGHNESS_MODE.as_ptr(),
                if settings.roughness_packed {
                    ffi::ROUGHNESS_PACKED
                } else {
                    ffi::ROUGHNESS_UNPACKED
                },
            );
            ffi::NVSDK_NGX_Parameter_SetUI(
                p,
                n::USE_HW_DEPTH.as_ptr(),
                if settings.hardware_depth {
                    ffi::DEPTH_HW
                } else {
                    ffi::DEPTH_LINEAR
                },
            );
            check("NVSDK_NGX_VULKAN_CreateFeature1", {
                ffi::NVSDK_NGX_VULKAN_CreateFeature1(
                    ngx.device,
                    cmd,
                    ffi::FEATURE_RAY_RECONSTRUCTION,
                    p,
                    &mut feature.handle,
                )
            })?;
        }
        Ok(feature)
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// Records the evaluation into `cmd`. The inputs must already be in the layouts NGX reads
    /// (`SHADER_READ_ONLY_OPTIMAL` for colour images, `DEPTH_STENCIL_READ_ONLY_OPTIMAL` for depth:
    /// wgpu's `TextureUses::RESOURCE`) and the output in `GENERAL` (`STORAGE_READ_WRITE`), with
    /// their writes made visible: `encoder.transition_resources` on the wgpu encoder submitted
    /// just before `cmd` does both (docs/rt-dlss-rr.md, "Recording").
    ///
    /// # Safety
    /// `cmd` is recording on `ngx`'s device; every image of `inputs` lives until it has run.
    pub unsafe fn evaluate(&self, cmd: vk::CommandBuffer, inputs: &Inputs) -> Result<(), Error> {
        use ffi::names as n;
        let p = self.params;
        // The resources stay on this stack frame until EvaluateFeature returns, which reads them.
        let color = inputs.color.resource(false);
        let output = inputs.output.resource(true);
        let depth = inputs.depth.resource(false);
        let motion = inputs.motion_vectors.resource(false);
        let diffuse = inputs.diffuse_albedo.resource(false);
        let specular = inputs.specular_albedo.resource(false);
        let normals = inputs.normals.resource(false);
        let optional = |i: &Option<Image>| i.map(|i| i.resource(false));
        let roughness = optional(&inputs.roughness);
        let hit_distance = optional(&inputs.specular_hit_distance);
        let specular_motion = optional(&inputs.specular_motion_vectors);
        let exposure = optional(&inputs.exposure);
        let bias = optional(&inputs.bias_current_color_mask);
        let transparency = optional(&inputs.transparency_mask);
        let mut world_to_view = inputs.world_to_view;
        let mut view_to_clip = inputs.view_to_clip;
        let ptr = |r: &ffi::Resource_VK| r as *const ffi::Resource_VK as *mut c_void;
        let opt_ptr = |r: &Option<ffi::Resource_VK>| r.as_ref().map_or(std::ptr::null_mut(), ptr);
        let matrix = |m: &mut Option<[f32; 16]>| {
            m.as_mut()
                .map_or(std::ptr::null_mut(), |m| m.as_mut_ptr().cast::<c_void>())
        };
        let or_one = |v: f32| if v == 0.0 { 1.0 } else { v };
        // What NGX_VULKAN_EVALUATE_DLSSD_EXT sets, with its defaults (a 0 scale or exposure is 1).
        // Safety: the feature's parameters; every pointer lives until the call below returns.
        unsafe {
            let set = |name: &CStr, v: *mut c_void| {
                ffi::NVSDK_NGX_Parameter_SetVoidPointer(p, name.as_ptr(), v)
            };
            set(n::COLOR, ptr(&color));
            set(n::OUTPUT, ptr(&output));
            set(n::DEPTH, ptr(&depth));
            set(n::MOTION_VECTORS, ptr(&motion));
            set(n::DIFFUSE_ALBEDO, ptr(&diffuse));
            set(n::SPECULAR_ALBEDO, ptr(&specular));
            set(n::NORMALS, ptr(&normals));
            set(n::ROUGHNESS, opt_ptr(&roughness));
            set(n::SPECULAR_HIT_DISTANCE, opt_ptr(&hit_distance));
            set(n::SPECULAR_MOTION_VECTORS, opt_ptr(&specular_motion));
            set(n::EXPOSURE_TEXTURE, opt_ptr(&exposure));
            set(n::BIAS_CURRENT_COLOR_MASK, opt_ptr(&bias));
            set(n::TRANSPARENCY_MASK, opt_ptr(&transparency));
            set(n::WORLD_TO_VIEW, matrix(&mut world_to_view));
            set(n::VIEW_TO_CLIP, matrix(&mut view_to_clip));
            ffi::NVSDK_NGX_Parameter_SetF(p, n::JITTER_X.as_ptr(), inputs.jitter[0]);
            ffi::NVSDK_NGX_Parameter_SetF(p, n::JITTER_Y.as_ptr(), inputs.jitter[1]);
            ffi::NVSDK_NGX_Parameter_SetF(p, n::MV_SCALE_X.as_ptr(), or_one(inputs.mv_scale[0]));
            ffi::NVSDK_NGX_Parameter_SetF(p, n::MV_SCALE_Y.as_ptr(), or_one(inputs.mv_scale[1]));
            ffi::NVSDK_NGX_Parameter_SetI(p, n::RESET.as_ptr(), i32::from(inputs.reset));
            ffi::NVSDK_NGX_Parameter_SetUI(
                p,
                n::RENDER_SUBRECT_WIDTH.as_ptr(),
                inputs.render_size[0],
            );
            ffi::NVSDK_NGX_Parameter_SetUI(
                p,
                n::RENDER_SUBRECT_HEIGHT.as_ptr(),
                inputs.render_size[1],
            );
            if self.settings.output_subrects {
                ffi::NVSDK_NGX_Parameter_SetUI(
                    p,
                    n::OUTPUT_SUBRECT_X.as_ptr(),
                    inputs.output_offset[0],
                );
                ffi::NVSDK_NGX_Parameter_SetUI(
                    p,
                    n::OUTPUT_SUBRECT_Y.as_ptr(),
                    inputs.output_offset[1],
                );
            }
            ffi::NVSDK_NGX_Parameter_SetF(p, n::PRE_EXPOSURE.as_ptr(), or_one(inputs.pre_exposure));
            ffi::NVSDK_NGX_Parameter_SetF(
                p,
                n::EXPOSURE_SCALE.as_ptr(),
                or_one(inputs.exposure_scale),
            );
            ffi::NVSDK_NGX_Parameter_SetF(p, n::FRAME_TIME_DELTA_MS.as_ptr(), inputs.frame_time_ms);
            // The rest the helper sets: whole images (no input sub-rectangles), the string tone
            // mapper (0, the helper's default), the dev DLL's overlay axes as they are.
            for (x, y) in n::SUBRECT_BASES {
                ffi::NVSDK_NGX_Parameter_SetUI(p, x.as_ptr(), 0);
                ffi::NVSDK_NGX_Parameter_SetUI(p, y.as_ptr(), 0);
            }
            ffi::NVSDK_NGX_Parameter_SetUI(p, n::TONEMAPPER_TYPE.as_ptr(), 0);
            ffi::NVSDK_NGX_Parameter_SetI(p, n::INDICATOR_INVERT_X.as_ptr(), 0);
            ffi::NVSDK_NGX_Parameter_SetI(p, n::INDICATOR_INVERT_Y.as_ptr(), 0);
            check("NVSDK_NGX_VULKAN_EvaluateFeature_C", {
                ffi::NVSDK_NGX_VULKAN_EvaluateFeature_C(cmd, self.handle, p, None)
            })
        }
    }

    /// Records the evaluation into a command buffer of its own, after `encoder`'s commands: the
    /// pattern `RawTlas::build` and `sun` use (wgpu records an encoder's commands at `finish`, so
    /// raw commands can't share one). Transitions `images` to what NGX reads (`RESOURCE`) and
    /// `outputs` to storage in `encoder`, ends it into `split`, records the evaluation between two
    /// full memory barriers, and leaves a fresh encoder in `encoder` for what follows; submit
    /// `split` in order before it. dlss_wgpu records the same transitions, then the evaluation in
    /// the same encoder (its wgpu records eagerly); NGX leaves each image in the layout it found,
    /// which wgpu's tracker still believes.
    ///
    /// # Safety
    /// `inputs` holds the handles of `images` and `outputs` (`Image::of`), alive for the frame.
    pub unsafe fn record(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        split: &mut Vec<wgpu::CommandBuffer>,
        images: &[&wgpu::Texture],
        outputs: &[&wgpu::Texture],
        inputs: &Inputs,
    ) -> Result<(), Error> {
        // The device's functions, for the barriers (copied out: no wgpu lock is held while the
        // encoder below is recorded).
        // Safety: read only, dropped at once.
        let raw_device = unsafe { device.as_hal::<Vulkan>() }.map(|d| d.raw_device().clone());
        let Some(raw_device) = raw_device else {
            return Err(Error {
                call: "a Vulkan device",
                result: ffi::RESULT_FAIL,
            });
        };
        let transitions = images
            .iter()
            .map(|t| wgpu::TextureTransition {
                texture: *t,
                selector: None,
                state: wgpu::TextureUses::RESOURCE,
            })
            .chain(outputs.iter().map(|t| wgpu::TextureTransition {
                texture: *t,
                selector: None,
                state: wgpu::TextureUses::STORAGE_READ_WRITE,
            }));
        encoder.transition_resources(
            std::iter::empty::<wgpu::BufferTransition<&wgpu::Buffer>>(),
            transitions,
        );
        let before = std::mem::replace(
            encoder,
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Horizon frame, after DLSS Ray Reconstruction"),
            }),
        );
        split.push(before.finish());
        let mut raw = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Horizon RTX: DLSS Ray Reconstruction"),
        });
        let mut result = Ok(());
        // NGX records its own barriers, but which stages wgpu's transitions above wait for
        // (shader stages) and which stages wgpu's next barriers start from are not NGX's: a full
        // memory barrier on each side covers whatever NGX runs (compute, copies), at the cost of
        // two barriers a frame.
        let full = [vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)];
        // Safety: raw commands on the encoder's own command buffer, which holds nothing else;
        // the transitions above put every image where NGX expects it.
        unsafe {
            raw.as_hal_mut::<Vulkan, _, _>(|hal| {
                result = match hal {
                    Some(hal) => {
                        let cb = hal.raw_handle();
                        let barrier = || {
                            raw_device.cmd_pipeline_barrier(
                                cb,
                                vk::PipelineStageFlags::ALL_COMMANDS,
                                vk::PipelineStageFlags::ALL_COMMANDS,
                                vk::DependencyFlags::empty(),
                                &full,
                                &[],
                                &[],
                            )
                        };
                        barrier();
                        let evaluated = self.evaluate(cb, inputs);
                        barrier();
                        evaluated
                    }
                    None => Err(Error {
                        call: "a Vulkan command encoder",
                        result: ffi::RESULT_FAIL,
                    }),
                };
            });
        }
        split.push(raw.finish());
        result
    }
}

impl Drop for DlssRr {
    fn drop(&mut self) {
        // Safety: the feature and parameters this made; the GPU is done with the feature when the
        // renderer drops it (it is dropped after the device's idle wait, or with the frame's
        // resources it was evaluated on).
        unsafe {
            if !self.handle.is_null() {
                ffi::NVSDK_NGX_VULKAN_ReleaseFeature(self.handle);
            }
            if !self.params.is_null() {
                ffi::NVSDK_NGX_VULKAN_DestroyParameters(self.params);
            }
        }
    }
}

// Stages 1 and 5 (docs/rt-dlss-rr.md §6): NGX's extensions enabled when the window's instance and
// device are made, NGX initialised on that device, one feature instance at the world's render size
// (DLAA), made again when that size changes and released before NGX shuts down, and evaluated each
// frame in place of the TAA resolve (`Session::evaluate`). Every step logs one line starting
// "DLSS-RR:", and every failure leaves the renderer as it is without RR.

/// Where NGX looks for its runtime DLL, and where it writes its own logs.
#[derive(Clone, Debug)]
pub struct Paths {
    pub dll_dirs: Vec<std::path::PathBuf>,
    pub data_path: std::path::PathBuf,
}

/// The start of the feature DLL's file name: `nvngx_dlssd.dll` on Windows,
/// `libnvidia-ngx-dlssd.so.<version>` on Linux.
#[cfg(windows)]
const RUNTIME_DLL: &str = "nvngx_dlssd";
#[cfg(not(windows))]
const RUNTIME_DLL: &str = "libnvidia-ngx-dlssd";

impl Paths {
    /// The executable's folder, then `DLSS_RUNTIME_DIR` when set; NGX's logs go to `data_path`
    /// (made if missing).
    pub fn new(data_path: std::path::PathBuf) -> Self {
        let mut dll_dirs = Vec::new();
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
        {
            dll_dirs.push(dir);
        }
        if let Some(dir) = std::env::var_os("DLSS_RUNTIME_DIR").filter(|d| !d.is_empty()) {
            dll_dirs.push(dir.into());
        }
        let _ = std::fs::create_dir_all(&data_path);
        Self {
            dll_dirs,
            data_path,
        }
    }

    /// Logs the folders NGX is told to search (`FeatureCommonInfo::PathListInfo`) and whether
    /// the runtime DLL is in each.
    pub fn log(&self) {
        for dir in &self.dll_dirs {
            match runtime_dll_in(dir) {
                Some(file) => log::info!(
                    "DLSS-RR: NGX searches {} for the runtime: {} found",
                    dir.display(),
                    file.to_string_lossy()
                ),
                None => log::info!(
                    "DLSS-RR: NGX searches {} for the runtime: no {RUNTIME_DLL}* there",
                    dir.display()
                ),
            }
        }
        if std::env::var_os("DLSS_RUNTIME_DIR").is_none_or(|d| d.is_empty()) {
            log::info!(
                "DLSS-RR: DLSS_RUNTIME_DIR is not set: only the executable's folder is searched"
            );
        }
        log::info!(
            "DLSS-RR: NGX's data path (its own logs): {}",
            self.data_path.display()
        );
    }
}

/// The feature DLL's name in `dir`, if it is there.
fn runtime_dll_in(dir: &std::path::Path) -> Option<std::ffi::OsString> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let name = entry.file_name();
        name.to_string_lossy()
            .to_ascii_lowercase()
            .starts_with(RUNTIME_DLL)
            .then_some(name)
    })
}

/// Names for the log: `a, b, c`, or `none`.
fn list<S: AsRef<CStr>>(names: &[S]) -> String {
    if names.is_empty() {
        return "none".to_owned();
    }
    names
        .iter()
        .map(|n| n.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The window's instance, made as `wgpu::Instance::new(make())` makes it, with NGX's instance
/// extensions enabled too. When wgpu already enables every one (expected:
/// `VK_KHR_get_physical_device_properties2`-class names; unverified), that is the instance
/// returned, unchanged. Otherwise it is made again through wgpu-hal's `init_with_callback` with
/// the missing names added, as dlss_wgpu does; then wgpu-core's flags are the environment's
/// (`Instance::from_hal`), so release builds validate indirect draws unless
/// `WGPU_VALIDATION_INDIRECT_CALL=0`. Any failure keeps wgpu's own instance, and RR stays off.
pub fn create_instance(
    make: impl Fn() -> wgpu::InstanceDescriptor,
    paths: &Paths,
) -> wgpu::Instance {
    paths.log();
    let instance = wgpu::Instance::new(make());
    let wanted = match required_instance_extensions(&paths.dll_dirs, &paths.data_path) {
        Ok(wanted) => wanted,
        Err(e) => {
            log::warn!(
                "DLSS-RR: off: NGX's instance extensions are unknown ({e}); the renderer stays as it is"
            );
            return instance;
        }
    };
    log::info!(
        "DLSS-RR: NGX asks for {} instance extension(s): {}",
        wanted.len(),
        list(&wanted)
    );
    let (missing, offered) = {
        // Safety: read only, dropped before the instance is used or dropped.
        let Some(hal) = (unsafe { instance.as_hal::<Vulkan>() }) else {
            log::warn!("DLSS-RR: off: the instance has no Vulkan backend");
            return instance;
        };
        let shared = hal.shared_instance();
        let missing: Vec<CString> = wanted
            .iter()
            .filter(|w| !shared.extensions().contains(&w.as_c_str()))
            .cloned()
            .collect();
        // Safety: the instance's own loader.
        let available = unsafe { shared.entry().enumerate_instance_extension_properties(None) }
            .unwrap_or_default();
        let offered = missing.iter().all(|m| {
            available
                .iter()
                .any(|a| a.extension_name_as_c_str() == Ok(m.as_c_str()))
        });
        (missing, offered)
    };
    if missing.is_empty() {
        log::info!("DLSS-RR: wgpu's Vulkan instance already enables all of them");
        return instance;
    }
    if !offered {
        log::warn!(
            "DLSS-RR: off: the Vulkan loader does not offer every instance extension NGX needs \
             (missing from wgpu's instance: {})",
            list(&missing)
        );
        return instance;
    }
    drop(instance);
    let desc = make();
    let hal_desc = wgpu::hal::InstanceDescriptor {
        name: "wgpu",
        flags: desc.flags,
        memory_budget_thresholds: desc.memory_budget_thresholds,
        backend_options: desc.backend_options.clone(),
        telemetry: None,
        // Only OpenGL reads the display; the surface takes the window's own handles.
        display: None,
    };
    let extra = leak(missing.clone());
    // Safety: what `Instance::new` does for Vulkan, with the extra names, each offered by the
    // loader (checked above).
    let made = unsafe {
        wgpu::hal::vulkan::Instance::init_with_callback(
            &hal_desc,
            Some(Box::new(
                move |args: wgpu::hal::vulkan::CreateInstanceCallbackArgs| {
                    for name in extra {
                        if !args.extensions.contains(&name) {
                            args.extensions.push(name);
                        }
                    }
                },
            )),
        )
    };
    match made {
        Ok(hal) => {
            log::info!(
                "DLSS-RR: the Vulkan instance is made with {} added (wgpu's own path cannot add \
                 instance extensions; indirect draws are now validated unless \
                 WGPU_VALIDATION_INDIRECT_CALL=0)",
                list(&missing)
            );
            // Safety: a Vulkan instance wgpu-hal made itself.
            unsafe { wgpu::Instance::from_hal::<Vulkan>(hal) }
        }
        Err(e) => {
            log::warn!(
                "DLSS-RR: off: making the Vulkan instance with {} failed ({e}); the renderer \
                 stays as it is",
                list(&missing)
            );
            wgpu::Instance::new(make())
        }
    }
}

/// NGX's device extensions for `adapter`'s GPU, to enable when the device is made
/// (horizon-hdr's `open_vulkan_device_with`). Empty, so that the device is made exactly as
/// without RR, when the GPU or driver lacks Ray Reconstruction, the query fails, or the driver
/// does not offer one of the names.
pub fn device_extensions(adapter: &wgpu::Adapter, paths: &Paths) -> Vec<&'static CStr> {
    let (instance, physical, available) = {
        // Safety: read only, dropped at once; the handles stay valid while `adapter` lives.
        let Some(hal) = (unsafe { adapter.as_hal::<Vulkan>() }) else {
            log::warn!("DLSS-RR: off: the adapter is not Vulkan");
            return Vec::new();
        };
        let raw = hal.shared_instance().raw_instance();
        // Safety: the adapter's own physical device on its own instance.
        let available =
            unsafe { raw.enumerate_device_extension_properties(hal.raw_physical_device()) }
                .unwrap_or_default();
        (raw.handle(), hal.raw_physical_device(), available)
    };
    // Safety: the adapter's physical device on its live instance.
    let wanted = match unsafe {
        required_device_extensions(instance, physical, &paths.dll_dirs, &paths.data_path)
    } {
        Ok(wanted) => wanted,
        Err(e) => {
            log::warn!("DLSS-RR: off on this GPU: {e}; the renderer stays as it is");
            return Vec::new();
        }
    };
    log::info!(
        "DLSS-RR: Ray Reconstruction is supported on this GPU and driver \
         (NVSDK_NGX_VULKAN_GetFeatureRequirements); NGX asks for {} device extension(s): {}",
        wanted.len(),
        list(&wanted)
    );
    let absent: Vec<&CString> = wanted
        .iter()
        .filter(|w| {
            !available
                .iter()
                .any(|a| a.extension_name_as_c_str() == Ok(w.as_c_str()))
        })
        .collect();
    if !absent.is_empty() {
        log::warn!(
            "DLSS-RR: off: the driver does not offer {}; the device is made without NGX's extensions",
            list(&absent)
        );
        return Vec::new();
    }
    leak(wanted)
}

/// Whether the device was made with every extension NGX asks for (logged, enabled or not).
fn extensions_enabled(device: &wgpu::Device, paths: &Paths) -> bool {
    // Safety: read only, dropped before the device is used again.
    let Some(hal) = (unsafe { device.as_hal::<Vulkan>() }) else {
        log::warn!("DLSS-RR: off: the device is not Vulkan");
        return false;
    };
    let shared = hal.shared_instance();
    let report = |kind: &str, wanted: Result<Vec<CString>, Error>, enabled: &[&CStr]| {
        let wanted = match wanted {
            Ok(wanted) => wanted,
            Err(e) => {
                log::warn!("DLSS-RR: off: NGX's {kind} extensions are unknown ({e})");
                return false;
            }
        };
        let (on, off): (Vec<&CString>, Vec<&CString>) =
            wanted.iter().partition(|w| enabled.contains(&w.as_c_str()));
        log::info!(
            "DLSS-RR: {kind} extensions NGX needs: enabled [{}], not enabled [{}]",
            list(&on),
            list(&off)
        );
        if !off.is_empty() {
            log::warn!(
                "DLSS-RR: off: the {kind} was made without {}; the renderer stays as it is",
                list(&off)
            );
        }
        off.is_empty()
    };
    let instance_ok = report(
        "instance",
        required_instance_extensions(&paths.dll_dirs, &paths.data_path),
        shared.extensions(),
    );
    // Safety: the device's own physical device on its live instance.
    let device_ok = instance_ok
        && report(
            "device",
            unsafe {
                required_device_extensions(
                    shared.raw_instance().handle(),
                    hal.raw_physical_device(),
                    &paths.dll_dirs,
                    &paths.data_path,
                )
            },
            hal.enabled_device_extensions(),
        );
    instance_ok && device_ok
}

/// Ray Reconstruction on the desktop window: NGX on the window's device and one feature instance,
/// evaluated once a frame (`evaluate`). In DLAA (render size = output size) it follows the world's
/// size; in an upscaling mode (`quality_wanted`) its output is the window's size and its render
/// size NGX's optimal one for it (`render_size`), which the renderer draws the world at. Dropping
/// it releases the feature and, with the last `Arc<Ngx>`, shuts NGX down, before the device can go
/// (both hold it).
pub struct Session {
    feature: Option<DlssRr>,
    ngx: Arc<Ngx>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    quality: Quality,
    /// The inputs' size and the output's (equal in DLAA).
    render: [u32; 2],
    output: [u32; 2],
    /// The feature was just made: its first evaluation resets (it has no history yet).
    fresh: bool,
    /// Evaluations so far, for the first one's log line.
    evaluated: u64,
}

/// One frame's images for `Session::evaluate`, at the session's render size but for `output`
/// (its output size): wgpu views whose raw Vulkan handles NGX is given (`Image::of`).
pub struct Frame<'a> {
    /// What the raw handles' formats are looked up with.
    pub adapter: &'a wgpu::Adapter,
    /// The noisy scene, linear HDR (`IsHDR`), before the exposure (NGX's `AutoExposure`).
    pub color: &'a wgpu::TextureView,
    /// The hardware depth, reversed Z (`DepthInverted`).
    pub depth: &'a wgpu::TextureView,
    /// Render pixels toward last frame's position, unjittered (`MVJittered` 0, scale 1, 1).
    pub motion: &'a wgpu::TextureView,
    pub diffuse_albedo: &'a wgpu::TextureView,
    pub specular_albedo: &'a wgpu::TextureView,
    /// World-space normals, roughness in w (`DLSS.Roughness.Mode` packed).
    pub normals: &'a wgpu::TextureView,
    /// The denoised scene, a storage image (Rgba16Float).
    pub output: &'a wgpu::TextureView,
    /// The projection's jitter this frame in render pixels, as NGX takes it.
    pub jitter: [f32; 2],
    /// The history is wrong (a camera cut, frames without RR before this one).
    pub reset: bool,
    pub frame_time_ms: f32,
}

impl Session {
    /// NGX on `device` (or `previous`'s, when the renderer is rebuilt on the same device) and a
    /// feature in `quality` for a `size` window; `None`, logged, when anything fails.
    pub fn start(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        quality: Quality,
        paths: &Paths,
        previous: Option<&Session>,
    ) -> Option<Self> {
        let ngx = match previous.filter(|p| p.device == *device) {
            Some(previous) => previous.ngx.clone(),
            None => {
                if !extensions_enabled(device, paths) {
                    return None;
                }
                match Ngx::init(device, &paths.dll_dirs, &paths.data_path) {
                    Ok(ngx) => {
                        log::info!(
                            "DLSS-RR: NGX initialised (project id, custom engine); Ray \
                             Reconstruction is available (SuperSamplingDenoising.Available = 1)"
                        );
                        ngx
                    }
                    Err(e) => {
                        log::warn!(
                            "DLSS-RR: off: {e}; RT lighting keeps its accumulation, denoiser and TAA"
                        );
                        return None;
                    }
                }
            }
        };
        log_optimal(&ngx, size, true);
        let mut session = Self {
            feature: None,
            ngx,
            device: device.clone(),
            queue: queue.clone(),
            quality,
            render: size,
            output: size,
            fresh: true,
            evaluated: 0,
        };
        session.render = session.render_for(size)?;
        session.create();
        session.feature.is_some().then_some(session)
    }

    /// The render size for a `size` output: itself in DLAA, NGX's optimal one otherwise; `None`,
    /// logged, when NGX gives none (an old DLL or driver).
    fn render_for(&self, size: [u32; 2]) -> Option<[u32; 2]> {
        if self.quality == Quality::Dlaa {
            return Some(size);
        }
        match self.ngx.optimal_settings(size, self.quality) {
            Ok(o) if !o.render.contains(&0) && o.render[0] <= size[0] && o.render[1] <= size[1] => {
                Some(o.render)
            }
            Ok(o) => {
                log::warn!(
                    "DLSS-RR: off: NGX's render size for {} at {}x{} is {}x{}; back to RT \
                     lighting's accumulation, denoiser and TAA at full resolution",
                    self.quality.name(),
                    size[0],
                    size[1],
                    o.render[0],
                    o.render[1]
                );
                None
            }
            Err(e) => {
                log::warn!(
                    "DLSS-RR: off: no render size for {} at {}x{} ({e}); back to RT lighting's \
                     accumulation, denoiser and TAA at full resolution",
                    self.quality.name(),
                    size[0],
                    size[1]
                );
                None
            }
        }
    }

    /// Whether a feature instance exists for the current size.
    pub fn active(&self) -> bool {
        self.feature.is_some()
    }

    /// The quality mode the feature is made in.
    pub fn quality(&self) -> Quality {
        self.quality
    }

    /// Whether the feature upscales (any mode but DLAA): the world is drawn at `render_size` and
    /// the output is the window's size (`resize` with the window's).
    pub fn upscales(&self) -> bool {
        self.quality != Quality::Dlaa
    }

    /// The size of the inputs (the world's, the rays', the guides').
    pub fn render_size(&self) -> [u32; 2] {
        self.render
    }

    /// The size of the output (the render size in DLAA).
    pub fn output_size(&self) -> [u32; 2] {
        self.output
    }

    /// Records this frame's evaluation (docs/rt-dlss-rr.md §3) after `encoder`'s commands, in
    /// command buffers of its own pushed to `split` (`DlssRr::record`), and leaves a new encoder
    /// in `encoder`. `Err` when the feature is missing, an input is not the render size or the
    /// output not the output size, an image is not Vulkan, or NGX refuses the evaluation: the
    /// caller turns RR off then.
    pub fn evaluate(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        split: &mut Vec<wgpu::CommandBuffer>,
        frame: &Frame<'_>,
    ) -> Result<(), Error> {
        let Some(feature) = &self.feature else {
            return Err(Error {
                call: "a feature instance (none was made)",
                result: ffi::RESULT_FAIL,
            });
        };
        let views = [
            frame.color,
            frame.depth,
            frame.motion,
            frame.diffuse_albedo,
            frame.specular_albedo,
            frame.normals,
            frame.output,
        ];
        let mut images = Vec::with_capacity(views.len());
        for (i, view) in views.into_iter().enumerate() {
            let texture = view.texture();
            let (size, call) = if i == 6 {
                (self.output, "an output of the feature's output size")
            } else {
                (self.render, "an input of the feature's render size")
            };
            if [texture.width(), texture.height()] != size {
                return Err(Error {
                    call,
                    result: ffi::RESULT_FAIL,
                });
            }
            // Safety: the views live for the frame the evaluation is recorded and submitted in;
            // `record` transitions them to the layouts NGX reads.
            let image = unsafe { Image::of(frame.adapter, view) }.ok_or(Error {
                call: "a Vulkan texture",
                result: ffi::RESULT_FAIL,
            })?;
            images.push(image);
        }
        let reset = frame.reset || self.fresh;
        let inputs = Inputs {
            color: images[0],
            depth: images[1],
            motion_vectors: images[2],
            diffuse_albedo: images[3],
            specular_albedo: images[4],
            normals: images[5],
            output: images[6],
            roughness: None,
            specular_hit_distance: None,
            specular_motion_vectors: None,
            exposure: None,
            bias_current_color_mask: None,
            transparency_mask: None,
            jitter: frame.jitter,
            mv_scale: [1.0, 1.0],
            render_size: self.render,
            output_offset: [0, 0],
            reset,
            pre_exposure: 1.0,
            exposure_scale: 1.0,
            frame_time_ms: frame.frame_time_ms,
            world_to_view: None,
            view_to_clip: None,
        };
        let read: Vec<&wgpu::Texture> = views[..6].iter().map(|v| v.texture()).collect();
        let written = [frame.output.texture()];
        // Safety: `inputs` holds the handles of these very textures, alive for the frame.
        unsafe {
            feature.record(&self.device, encoder, split, &read, &written, &inputs)?;
        }
        self.fresh = false;
        if self.evaluated == 0 {
            let sizes = if self.upscales() {
                format!(
                    "{}x{} -> {}x{} ({}",
                    self.render[0],
                    self.render[1],
                    self.output[0],
                    self.output[1],
                    self.quality.name()
                )
            } else {
                format!("{}x{} (DLAA", self.render[0], self.render[1])
            };
            log::info!(
                "DLSS-RR: first evaluation recorded at {sizes}, reset {}, jitter {:.3}, {:.3}); \
                 RT lighting's accumulation, denoiser and TAA are skipped for the window",
                if reset { "on" } else { "off" },
                frame.jitter[0],
                frame.jitter[1],
            );
        }
        self.evaluated += 1;
        Ok(())
    }

    /// Follows the output's size (the world's in DLAA, the window's when upscaling): the feature
    /// is released and made again for `size`. A render size NGX does not give leaves the session
    /// without a feature (`active` false), logged.
    pub fn resize(&mut self, size: [u32; 2]) {
        if size == self.output || size.contains(&0) {
            return;
        }
        self.release();
        self.output = size;
        log_optimal(&self.ngx, size, false);
        let Some(render) = self.render_for(size) else {
            return;
        };
        self.render = render;
        self.create();
        self.fresh = true;
    }

    fn describe(&self) -> String {
        let [w, h] = self.render;
        if self.upscales() {
            format!(
                "{}: render {w}x{h} -> output {}x{}",
                self.quality.name(),
                self.output[0],
                self.output[1]
            )
        } else {
            format!("{w}x{h}")
        }
    }

    fn create(&mut self) {
        // What the later stages feed (docs/rt-dlss-rr.md §3); the evaluation stage may change
        // these, which only means making the feature again.
        let settings = Settings {
            render: self.render,
            output: self.output,
            quality: self.quality,
            depth_inverted: true,
            hdr: true,
            mv_low_res: true,
            mv_jittered: false,
            auto_exposure: true,
            roughness_packed: true,
            hardware_depth: true,
            output_subrects: false,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Horizon RTX: DLSS Ray Reconstruction setup"),
            });
        let mut made = Err(Error {
            call: "a Vulkan command encoder",
            result: ffi::RESULT_FAIL,
        });
        // Safety: the encoder's own command buffer, open and recording (wgpu-core begins it
        // for `as_hal_mut`) on the device NGX was initialised on, holding nothing else; it is
        // submitted at once, before any evaluation.
        unsafe {
            encoder.as_hal_mut::<Vulkan, _, _>(|hal| {
                if let Some(hal) = hal {
                    made = DlssRr::new(&self.ngx, hal.raw_handle(), settings);
                }
            });
        }
        self.queue.submit([encoder.finish()]);
        let what = self.describe();
        match made {
            Ok(feature) => {
                if self.upscales() {
                    log::info!(
                        "DLSS-RR: feature instance created for {what} (upscaling, \
                         PerfQualityValue {}, jitter over {} phases)",
                        self.quality.value(),
                        jitter_phases(self.render, self.output)
                    );
                } else {
                    log::info!(
                        "DLSS-RR: feature instance created for {what} (DLAA: render size = \
                         output size)"
                    );
                }
                self.feature = Some(feature);
            }
            Err(e) => log::warn!(
                "DLSS-RR: off: creating the feature instance for {what} failed: {e}; the \
                 renderer stays as it is"
            ),
        }
    }

    fn release(&mut self) {
        if let Some(feature) = self.feature.take() {
            // Its setup commands have run before it is released.
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
            drop(feature);
            log::info!("DLSS-RR: feature instance for {} released", self.describe());
        }
    }
}

/// The jitter's phases before it repeats for a `render` input and an `output`: NVIDIA's guide's
/// 8 × (output / render)² (the scale on each axis), at least 32 as its RR guide asks.
pub fn jitter_phases(render: [u32; 2], output: [u32; 2]) -> u32 {
    let ratio = |i: usize| output[i] as f64 / render[i].max(1) as f64;
    ((8.0 * ratio(0) * ratio(1)).ceil() as u32).max(32)
}

impl Drop for Session {
    fn drop(&mut self) {
        // The feature first, then (fields) the `Arc<Ngx>`, whose last drop shuts NGX down, then
        // the device.
        self.release();
    }
}

/// Logs NGX's render size for a `size` output at each quality mode (info at start, debug on
/// resize).
fn log_optimal(ngx: &Ngx, size: [u32; 2], info: bool) {
    let sizes: Vec<String> = Quality::ALL
        .iter()
        .map(|&q| match ngx.optimal_settings(size, q) {
            Ok(o) if o.min == o.max => {
                format!("{} {}x{}", q.name(), o.render[0], o.render[1])
            }
            Ok(o) => format!(
                "{} {}x{} (dynamic {}x{} to {}x{})",
                q.name(),
                o.render[0],
                o.render[1],
                o.min[0],
                o.min[1],
                o.max[0],
                o.max[1]
            ),
            Err(e) => format!("{} unknown ({e})", q.name()),
        })
        .collect();
    let level = if info {
        log::Level::Info
    } else {
        log::Level::Debug
    };
    log::log!(
        level,
        "DLSS-RR: NGX's render sizes for a {}x{} output: {}",
        size[0],
        size[1],
        sizes.join(", ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_is_told_by_its_top_bits() {
        assert!(!ffi::failed(ffi::RESULT_SUCCESS));
        assert!(ffi::failed(ffi::RESULT_FAIL));
        // NVSDK_NGX_Result_FAIL_FeatureNotSupported and its kin keep the code in the low bits.
        assert!(ffi::failed(0xBAD0_0001));
        assert!(!ffi::failed(0x0000_0002));
    }

    #[test]
    fn the_resource_is_laid_out_as_the_sdk_declares_it() {
        // ImageView, Image (8 each), the subresource range (5 x 4), the format and the size
        // (3 x 4): 48 bytes; the type (4) and the flag (1) after the union, padded to 8.
        assert_eq!(std::mem::size_of::<ffi::ImageViewInfo_VK>(), 48);
        assert_eq!(std::mem::size_of::<ffi::Resource_VK>(), 56);
        assert_eq!(std::mem::offset_of!(ffi::Resource_VK, Type), 48);
        assert_eq!(std::mem::offset_of!(ffi::Resource_VK, ReadWrite), 52);
    }

    #[test]
    fn the_quality_modes_scale_as_published() {
        let eye = [4400, 3520];
        assert_eq!(Quality::Dlaa.render_size(eye), eye);
        assert_eq!(Quality::Performance.render_size(eye), [2200, 1760]);
        assert_eq!(Quality::Quality.render_size(eye), [2934, 2347]);
        assert_eq!(Quality::UltraPerformance.render_size(eye), [1467, 1174]);
    }

    #[test]
    fn the_quality_switch_names_each_mode() {
        assert_eq!(Quality::parse(""), Some(Quality::Dlaa));
        assert_eq!(Quality::parse(" DLAA "), Some(Quality::Dlaa));
        assert_eq!(Quality::parse("Quality"), Some(Quality::Quality));
        assert_eq!(Quality::parse("balanced"), Some(Quality::Balanced));
        assert_eq!(Quality::parse("performance"), Some(Quality::Performance));
        assert_eq!(Quality::parse("ultra"), Some(Quality::UltraPerformance));
        assert_eq!(
            Quality::parse("ultra_performance"),
            Some(Quality::UltraPerformance)
        );
        assert_eq!(Quality::parse("fast"), None);
    }

    #[test]
    fn the_jitter_has_more_phases_the_more_it_upscales() {
        // NVIDIA's 8 × scale², never under RR's 32.
        assert_eq!(jitter_phases([2580, 1080], [2580, 1080]), 32);
        assert_eq!(jitter_phases([1720, 720], [2580, 1080]), 32);
        assert_eq!(jitter_phases([1290, 540], [2580, 1080]), 32);
        assert_eq!(jitter_phases([860, 360], [2580, 1080]), 72);
        assert_eq!(jitter_phases([640, 360], [2560, 1440]), 128);
    }

    #[test]
    fn paths_end_in_a_nul() {
        let w = wide(std::path::Path::new("ab"));
        assert_eq!(w, vec![97, 98, 0]);
    }
}
