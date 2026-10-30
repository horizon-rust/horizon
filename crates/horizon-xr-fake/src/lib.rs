//! A fake OpenXR runtime, for testing Horizon's VR without a headset (a development tool:
//! players never load it). The Khronos loader loads it when `XR_RUNTIME_JSON` names its
//! manifest (`horizon-xr-fake.json`, beside the library), for that process only; nothing is
//! installed or registered.
//!
//! It is a headset the test controls: two eyes with an off-axis field of view like a real
//! one, a head and controllers moved by a script, frames paced at a fixed rate, real Vulkan
//! swapchain images on the application's own device, and on chosen frames the images each
//! layer shows are written as PNG. When the run ends it writes a report of what the
//! application did. It implements the part of OpenXR 1.0 and `XR_KHR_vulkan_enable2` that
//! Horizon uses; any other function answers `XR_ERROR_FUNCTION_UNSUPPORTED`.
//!
//! Settings (environment variables):
//! - `HORIZON_FAKE_XR_OUT`: folder for the report (`report.json`) and captures; default `.`.
//! - `HORIZON_FAKE_XR_SIZE`: each eye's recommended image, `WxH`; default `1600x1600`.
//! - `HORIZON_FAKE_XR_HZ`: display rate; default 90.
//! - `HORIZON_FAKE_XR_FRAMES`: ends the session (asks the application to exit) after this
//!   many frames; default none.
//! - `HORIZON_FAKE_XR_CAPTURE`: frame numbers to capture, `120,240`.
//! - `HORIZON_FAKE_XR_VIDEO`: frames `FIRST-LAST` of the left eye only, as `video00123.png`
//!   (one PNG per frame, to make a video of: flicker shows across frames, not in one).
//! - `HORIZON_FAKE_XR_CAPTURE_EVERY`: also capture every N frames into `latest-*.png`, so a
//!   run that ends on its own leaves its last view.
//! - `HORIZON_FAKE_XR_CAPTURE_SECONDS`: script times to capture, `8,12.5`: the first frame at
//!   or after each, into `t8-*.png`, `t12.5-*.png` (the script runs on time, not frames).
//! - `HORIZON_FAKE_XR_SCRIPT`: a script file, one step per line (`#` comments):
//!   `SECONDS head YAW PITCH [X Y Z]` moves the head there by that time (degrees, yaw
//!   positive turning left, pitch positive looking up, metres in the stage), linearly from
//!   the step before; `SECONDS input PATH VALUE [Y]` sets a binding path's value from that
//!   time (`/user/hand/right/input/a/click 1`, a stick takes `X Y`); `SECONDS pose PATH YAW
//!   PITCH [X Y Z]` moves a pose binding (`/user/hand/right/input/aim/pose`) like the head,
//!   and a pose the script never moves isn't tracked.
//! - `HORIZON_FAKE_XR_NO_EYES=1`: no eye tracking (no `XR_EXT_eye_gaze_interaction`).
//!   Otherwise the eyes' pose, `pose /user/eyes_ext/input/gaze_ext/pose YAW PITCH`, is where
//!   they look relative to the head (its position is ignored: between the eyes); before its
//!   first step the gaze isn't valid.
//! - `HORIZON_FAKE_XR_GAZE_STEP`: snaps the gaze to a grid this many degrees apart, rows
//!   shifted by half a step, as the Vision Pro streamed by KRVR does (about 12).
//! - `HORIZON_FAKE_XR_QUAD=1`: also offers the quad views (`XR_VARJO_quad_views`,
//!   `XR_VARJO_foveated_rendering`): each eye wide at `HORIZON_FAKE_XR_SIZE`, and an inset
//!   40 degrees across at `HORIZON_FAKE_XR_INSET_SIZE` (default half the eye's), centred where
//!   the eyes look when the application asks for foveation and the gaze is valid, ahead
//!   otherwise.
//!
//! A capture also writes `*-eye0-composite.png` and `*-eye1-composite.png` when quad layers
//! are shown: each eye with the quads drawn over it where they are, as a headset shows them
//! (premultiplied alpha, later layers on top, no depth).

#![allow(clippy::missing_safety_doc)]

use std::collections::{HashMap, VecDeque};
use std::ffi::{CStr, c_char};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use ash::vk::{self, Handle};
use openxr_sys as sys;
use sys::Handle as _;

mod script;
use script::Script;

type XrResult = sys::Result;

const RUNTIME_NAME: &str = "Horizon fake XR";
const SYSTEM_NAME: &str = "Horizon fake headset";
const SYSTEM_ID: u64 = 1;
const IPD: f32 = 0.063;
/// A real headset's left eye (radians); the right eye mirrors it.
const LEFT_FOV: sys::Fovf = sys::Fovf {
    angle_left: -0.96,
    angle_right: 0.78,
    angle_up: 0.85,
    angle_down: -0.90,
};
/// Formats offered, sRGB first (Vulkan's numbers).
const FORMATS: [i64; 4] = [43, 50, 37, 44];

#[derive(Default)]
struct Report {
    frames: u64,
    skipped_waits: u64,
    /// Milliseconds between successive `xrEndFrame`s.
    intervals: Vec<f32>,
    layers: HashMap<String, u64>,
    captures: Vec<String>,
    errors: Vec<String>,
    actions: Vec<String>,
    exit_requested: bool,
}

struct Swapchain {
    images: Vec<(vk::Image, vk::DeviceMemory)>,
    format: vk::Format,
    size: [u32; 2],
    next: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum SpaceKind {
    View,
    Local,
    Stage,
    /// A pose action's space (`xrCreateActionSpace`): the action's handle.
    Action(u64),
}

struct Action {
    name: String,
    ty: sys::ActionType,
    bindings: Vec<String>,
}

struct Vulkan {
    instance: ash::Instance,
    device: Option<ash::Device>,
}

#[derive(Default)]
struct State {
    instance: Option<u64>,
    vulkan: Option<Vulkan>,
    session: Option<u64>,
    binding: Option<(u32, u32)>,
    session_state: Option<sys::SessionState>,
    events: VecDeque<(u64, sys::SessionState)>,
    spaces: HashMap<u64, SpaceKind>,
    swapchains: HashMap<u64, Swapchain>,
    paths: Vec<String>,
    actions: HashMap<u64, Action>,
    next_handle: u64,
    started: Option<Instant>,
    next_frame: Option<Instant>,
    last_end: Option<Instant>,
    script: Script,
    report: Report,
    settings: Settings,
}

struct Settings {
    out: std::path::PathBuf,
    size: [u32; 2],
    period: Duration,
    frames: Option<u64>,
    capture: Vec<u64>,
    capture_every: Option<u64>,
    capture_seconds: Vec<(f32, String)>,
    video: Option<(u64, u64)>,
    /// The headset tracks the eyes.
    eyes: bool,
    /// The gaze's grid, degrees (`script::snap_gaze`).
    gaze_step: Option<f32>,
    /// The quad views are offered, and each inset's image.
    quad: bool,
    inset_size: [u32; 2],
}

impl Default for Settings {
    fn default() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        let pixels = |name: &str| {
            var(name).and_then(|s| {
                let (w, h) = s.split_once('x')?;
                Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
            })
        };
        let size: [u32; 2] = pixels("HORIZON_FAKE_XR_SIZE").unwrap_or([1600, 1600]);
        let hz: f64 = var("HORIZON_FAKE_XR_HZ")
            .and_then(|s| s.parse().ok())
            .unwrap_or(90.0);
        Self {
            out: var("HORIZON_FAKE_XR_OUT")
                .unwrap_or_else(|| ".".into())
                .into(),
            size,
            period: Duration::from_secs_f64(1.0 / hz.max(1.0)),
            frames: var("HORIZON_FAKE_XR_FRAMES").and_then(|s| s.parse().ok()),
            capture: var("HORIZON_FAKE_XR_CAPTURE")
                .map(|s| s.split(',').filter_map(|n| n.trim().parse().ok()).collect())
                .unwrap_or_default(),
            capture_seconds: var("HORIZON_FAKE_XR_CAPTURE_SECONDS")
                .map(|s| {
                    s.split(',')
                        .filter_map(|n| Some((n.trim().parse().ok()?, n.trim().to_owned())))
                        .collect()
                })
                .unwrap_or_default(),
            video: var("HORIZON_FAKE_XR_VIDEO").and_then(|s| {
                let (a, b) = s.split_once('-')?;
                Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
            }),
            capture_every: var("HORIZON_FAKE_XR_CAPTURE_EVERY")
                .and_then(|s| s.parse().ok())
                .filter(|&n| n > 0),
            eyes: var("HORIZON_FAKE_XR_NO_EYES").is_none_or(|v| v != "1"),
            gaze_step: var("HORIZON_FAKE_XR_GAZE_STEP")
                .and_then(|s| s.parse().ok())
                .filter(|&d: &f32| d > 0.0),
            quad: var("HORIZON_FAKE_XR_QUAD").is_some_and(|v| v == "1"),
            inset_size: pixels("HORIZON_FAKE_XR_INSET_SIZE").unwrap_or(size.map(|c| c / 2)),
        }
    }
}

fn state() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE
        .get_or_init(|| Mutex::new(State::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

impl State {
    fn handle(&mut self) -> u64 {
        self.next_handle += 1;
        self.next_handle
    }
    fn now(&self) -> sys::Time {
        let started = self.started.unwrap_or_else(Instant::now);
        sys::Time::from_nanos(started.elapsed().as_nanos() as i64 + 1)
    }
    fn seconds(&self) -> f32 {
        self.started.map_or(0.0, |s| s.elapsed().as_secs_f32())
    }
    fn set_session_state(&mut self, state: sys::SessionState) {
        if let Some(session) = self.session {
            self.session_state = Some(state);
            self.events.push_back((session, state));
        }
    }
    fn write_report(&self) {
        let r = &self.report;
        let mut intervals = r.intervals.clone();
        intervals.sort_by(f32::total_cmp);
        let mut last: Vec<f32> = r.intervals.iter().rev().take(300).copied().collect();
        last.sort_by(f32::total_cmp);
        let last_median = last.get(last.len() / 2).copied().unwrap_or(0.0);
        let last_p95 = last
            .get(((last.len() as f32 - 1.0) * 0.95).round().max(0.0) as usize)
            .copied()
            .unwrap_or(0.0);
        let pick = |q: f32| {
            intervals
                .get(((intervals.len() as f32 - 1.0) * q).round().max(0.0) as usize)
                .copied()
                .unwrap_or(0.0)
        };
        let list = |v: &[String]| {
            v.iter()
                .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "'")))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let layers = r
            .layers
            .iter()
            .map(|(k, v)| format!("\"{k}\": {v}"))
            .collect::<Vec<_>>()
            .join(", ");
        let json = format!(
            "{{\n  \"frames\": {},\n  \"skipped_waits\": {},\n  \"frame_ms\": {{\"median\": {:.3}, \"p95\": {:.3}, \"p99\": {:.3}, \"max\": {:.3}}},\n  \"fps\": {:.1},\n  \"last_300\": {{\"median_ms\": {:.3}, \"p95_ms\": {:.3}, \"fps\": {:.1}}},\n  \"layers\": {{{layers}}},\n  \"actions\": [{}],\n  \"captures\": [{}],\n  \"errors\": [{}],\n  \"exit_requested\": {}\n}}\n",
            r.frames,
            r.skipped_waits,
            pick(0.5),
            pick(0.95),
            pick(0.99),
            pick(1.0),
            if pick(0.5) > 0.0 {
                1000.0 / pick(0.5)
            } else {
                0.0
            },
            last_median,
            last_p95,
            if last_median > 0.0 {
                1000.0 / last_median
            } else {
                0.0
            },
            list(&r.actions),
            list(&r.captures),
            list(&r.errors),
            r.exit_requested,
        );
        let _ = std::fs::create_dir_all(&self.settings.out);
        let _ = std::fs::write(self.settings.out.join("report.json"), json);
    }
}

fn write_str(dst: &mut [c_char], s: &str) {
    let n = s.len().min(dst.len() - 1);
    for (d, b) in dst.iter_mut().zip(s.bytes().take(n)) {
        *d = b as c_char;
    }
    dst[n] = 0;
}

unsafe fn read_str(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// The two-call idiom: the count, then the items when there is room.
unsafe fn two_call<T: Copy>(capacity: u32, count: *mut u32, out: *mut T, items: &[T]) -> XrResult {
    unsafe {
        *count = items.len() as u32;
        if capacity == 0 {
            return XrResult::SUCCESS;
        }
        if (capacity as usize) < items.len() {
            return XrResult::ERROR_SIZE_INSUFFICIENT;
        }
        for (i, item) in items.iter().enumerate() {
            *out.add(i) = *item;
        }
    }
    XrResult::SUCCESS
}

// ----- the loader's entry point -----

#[unsafe(no_mangle)]
pub unsafe extern "system" fn xrNegotiateLoaderRuntimeInterface(
    loader: *const sys::NegotiateLoaderInfo,
    request: *mut sys::NegotiateRuntimeRequest,
) -> XrResult {
    unsafe {
        if loader.is_null() || request.is_null() {
            return XrResult::ERROR_INITIALIZATION_FAILED;
        }
        (*request).runtime_interface_version = 1;
        (*request).runtime_api_version = sys::Version::new(1, 0, 34);
        (*request).get_instance_proc_addr = Some(get_instance_proc_addr);
    }
    XrResult::SUCCESS
}

unsafe extern "system" fn unsupported() -> XrResult {
    XrResult::ERROR_FUNCTION_UNSUPPORTED
}

macro_rules! table {
    ($name:expr, $($xr:literal => $f:ident),* $(,)?) => {
        match $name {
            $($xr => $f as *const (),)*
            _ => unsupported as *const (),
        }
    };
}

unsafe extern "system" fn get_instance_proc_addr(
    _instance: sys::Instance,
    name: *const c_char,
    function: *mut Option<sys::pfn::VoidFunction>,
) -> XrResult {
    let name = unsafe { read_str(name) };
    let f = table!(name.as_str(),
        "xrGetInstanceProcAddr" => get_instance_proc_addr,
        "xrEnumerateApiLayerProperties" => enumerate_api_layer_properties,
        "xrEnumerateInstanceExtensionProperties" => enumerate_instance_extension_properties,
        "xrCreateInstance" => create_instance,
        "xrDestroyInstance" => destroy_instance,
        "xrGetInstanceProperties" => get_instance_properties,
        "xrPollEvent" => poll_event,
        "xrResultToString" => result_to_string,
        "xrStructureTypeToString" => structure_type_to_string,
        "xrGetSystem" => get_system,
        "xrGetSystemProperties" => get_system_properties,
        "xrEnumerateEnvironmentBlendModes" => enumerate_environment_blend_modes,
        "xrEnumerateViewConfigurations" => enumerate_view_configurations,
        "xrEnumerateViewConfigurationViews" => enumerate_view_configuration_views,
        "xrGetVulkanGraphicsRequirements2KHR" => get_vulkan_graphics_requirements,
        "xrGetVulkanGraphicsRequirementsKHR" => get_vulkan_graphics_requirements,
        "xrCreateVulkanInstanceKHR" => create_vulkan_instance,
        "xrGetVulkanGraphicsDevice2KHR" => get_vulkan_graphics_device,
        "xrCreateVulkanDeviceKHR" => create_vulkan_device,
        "xrCreateSession" => create_session,
        "xrDestroySession" => destroy_session,
        "xrEnumerateReferenceSpaces" => enumerate_reference_spaces,
        "xrCreateReferenceSpace" => create_reference_space,
        "xrDestroySpace" => destroy_space,
        "xrLocateSpace" => locate_space,
        "xrCreateActionSpace" => create_action_space,
        "xrLocateViews" => locate_views,
        "xrEnumerateSwapchainFormats" => enumerate_swapchain_formats,
        "xrCreateSwapchain" => create_swapchain,
        "xrDestroySwapchain" => destroy_swapchain,
        "xrEnumerateSwapchainImages" => enumerate_swapchain_images,
        "xrAcquireSwapchainImage" => acquire_swapchain_image,
        "xrWaitSwapchainImage" => ok_swapchain,
        "xrReleaseSwapchainImage" => ok_swapchain,
        "xrBeginSession" => begin_session,
        "xrEndSession" => end_session,
        "xrRequestExitSession" => request_exit_session,
        "xrWaitFrame" => wait_frame,
        "xrBeginFrame" => begin_frame,
        "xrEndFrame" => end_frame,
        "xrStringToPath" => string_to_path,
        "xrPathToString" => path_to_string,
        "xrCreateActionSet" => create_action_set,
        "xrDestroyActionSet" => destroy_handle,
        "xrCreateAction" => create_action,
        "xrDestroyAction" => destroy_handle,
        "xrSuggestInteractionProfileBindings" => suggest_bindings,
        "xrAttachSessionActionSets" => ok_session,
        "xrSyncActions" => ok_session,
        "xrGetActionStateBoolean" => get_action_state_boolean,
        "xrGetActionStateFloat" => get_action_state_float,
        "xrGetActionStateVector2f" => get_action_state_vector2f,
        "xrGetActionStatePose" => get_action_state_pose,
    );
    unsafe {
        // SAFETY: the loader calls each pointer with its OpenXR signature, which every entry
        // above has; `unsupported` ignores its arguments and returns, as the ABI allows.
        *function = Some(std::mem::transmute::<*const (), sys::pfn::VoidFunction>(f));
    }
    XrResult::SUCCESS
}

// ----- instance and system -----

unsafe extern "system" fn enumerate_api_layer_properties(
    _capacity: u32,
    count: *mut u32,
    _out: *mut sys::ApiLayerProperties,
) -> XrResult {
    unsafe { *count = 0 };
    XrResult::SUCCESS
}

unsafe extern "system" fn enumerate_instance_extension_properties(
    _layer: *const c_char,
    capacity: u32,
    count: *mut u32,
    out: *mut sys::ExtensionProperties,
) -> XrResult {
    let mut items = Vec::new();
    let mut offered = vec![("XR_KHR_vulkan_enable2", 2), ("XR_KHR_vulkan_enable", 8)];
    // Asked before any instance exists: the settings are read here too.
    let settings = Settings::default();
    if settings.eyes {
        offered.push(("XR_EXT_eye_gaze_interaction", 2));
    }
    if settings.quad {
        offered.push(("XR_VARJO_quad_views", 1));
        offered.push(("XR_VARJO_foveated_rendering", 3));
    }
    for (name, version) in offered {
        let mut p = sys::ExtensionProperties {
            ty: sys::StructureType::EXTENSION_PROPERTIES,
            next: std::ptr::null_mut(),
            extension_name: [0; sys::MAX_EXTENSION_NAME_SIZE],
            extension_version: version,
        };
        write_str(&mut p.extension_name, name);
        items.push(p);
    }
    unsafe { two_call(capacity, count, out, &items) }
}

unsafe extern "system" fn create_instance(
    _info: *const sys::InstanceCreateInfo,
    instance: *mut sys::Instance,
) -> XrResult {
    let mut s = state();
    if s.instance.is_some() {
        return XrResult::ERROR_LIMIT_REACHED;
    }
    s.settings = Settings::default();
    s.script = Script::load();
    let h = s.handle();
    s.instance = Some(h);
    s.started = Some(Instant::now());
    unsafe { *instance = sys::Instance::from_raw(h) };
    XrResult::SUCCESS
}

unsafe extern "system" fn destroy_instance(_instance: sys::Instance) -> XrResult {
    let mut s = state();
    s.write_report();
    // The application owns the instance and device it asked for: dropping the handles
    // leaves them alive.
    *s = State::default();
    XrResult::SUCCESS
}

unsafe extern "system" fn get_instance_properties(
    _instance: sys::Instance,
    props: *mut sys::InstanceProperties,
) -> XrResult {
    unsafe {
        (*props).runtime_version = sys::Version::new(0, 1, 0);
        write_str(&mut (*props).runtime_name, RUNTIME_NAME);
    }
    XrResult::SUCCESS
}

unsafe extern "system" fn poll_event(
    _instance: sys::Instance,
    buffer: *mut sys::EventDataBuffer,
) -> XrResult {
    let mut s = state();
    let Some((session, st)) = s.events.pop_front() else {
        return XrResult::EVENT_UNAVAILABLE;
    };
    let time = s.now();
    unsafe {
        let e = buffer as *mut sys::EventDataSessionStateChanged;
        *e = sys::EventDataSessionStateChanged {
            ty: sys::StructureType::EVENT_DATA_SESSION_STATE_CHANGED,
            next: std::ptr::null(),
            session: sys::Session::from_raw(session),
            state: st,
            time,
        };
    }
    XrResult::SUCCESS
}

unsafe extern "system" fn result_to_string(
    _i: sys::Instance,
    value: XrResult,
    buffer: *mut c_char,
) -> XrResult {
    let text = format!("{value:?}");
    unsafe {
        let dst = std::slice::from_raw_parts_mut(buffer, sys::MAX_RESULT_STRING_SIZE);
        write_str(dst, &text);
    }
    XrResult::SUCCESS
}

unsafe extern "system" fn structure_type_to_string(
    _i: sys::Instance,
    value: sys::StructureType,
    buffer: *mut c_char,
) -> XrResult {
    let text = format!("{value:?}");
    unsafe {
        let dst = std::slice::from_raw_parts_mut(buffer, sys::MAX_STRUCTURE_NAME_SIZE);
        write_str(dst, &text);
    }
    XrResult::SUCCESS
}

unsafe extern "system" fn get_system(
    _instance: sys::Instance,
    info: *const sys::SystemGetInfo,
    system: *mut sys::SystemId,
) -> XrResult {
    unsafe {
        if (*info).form_factor != sys::FormFactor::HEAD_MOUNTED_DISPLAY {
            return XrResult::ERROR_FORM_FACTOR_UNSUPPORTED;
        }
        *system = sys::SystemId::from_raw(SYSTEM_ID);
    }
    XrResult::SUCCESS
}

unsafe extern "system" fn get_system_properties(
    _instance: sys::Instance,
    _system: sys::SystemId,
    props: *mut sys::SystemProperties,
) -> XrResult {
    let (size, eyes, quad) = {
        let s = state();
        (s.settings.size, s.settings.eyes, s.settings.quad)
    };
    unsafe {
        let mut next = (*props).next as *mut sys::BaseOutStructure;
        while !next.is_null() {
            match (*next).ty {
                sys::StructureType::SYSTEM_EYE_GAZE_INTERACTION_PROPERTIES_EXT => {
                    let p = next as *mut sys::SystemEyeGazeInteractionPropertiesEXT;
                    (*p).supports_eye_gaze_interaction = eyes.into();
                }
                sys::StructureType::SYSTEM_FOVEATED_RENDERING_PROPERTIES_VARJO => {
                    let p = next as *mut sys::SystemFoveatedRenderingPropertiesVARJO;
                    (*p).supports_foveated_rendering = (quad && eyes).into();
                }
                _ => {}
            }
            next = (*next).next;
        }
        (*props).system_id = sys::SystemId::from_raw(SYSTEM_ID);
        (*props).vendor_id = 0;
        write_str(&mut (*props).system_name, SYSTEM_NAME);
        (*props).graphics_properties.max_swapchain_image_width = size[0].max(4096);
        (*props).graphics_properties.max_swapchain_image_height = size[1].max(4096);
        (*props).graphics_properties.max_layer_count = 16;
        (*props).tracking_properties.orientation_tracking = sys::TRUE;
        (*props).tracking_properties.position_tracking = sys::TRUE;
    }
    XrResult::SUCCESS
}

unsafe extern "system" fn enumerate_environment_blend_modes(
    _i: sys::Instance,
    _s: sys::SystemId,
    _v: sys::ViewConfigurationType,
    capacity: u32,
    count: *mut u32,
    out: *mut sys::EnvironmentBlendMode,
) -> XrResult {
    unsafe { two_call(capacity, count, out, &[sys::EnvironmentBlendMode::OPAQUE]) }
}

unsafe extern "system" fn enumerate_view_configurations(
    _i: sys::Instance,
    _s: sys::SystemId,
    capacity: u32,
    count: *mut u32,
    out: *mut sys::ViewConfigurationType,
) -> XrResult {
    let mut types = vec![sys::ViewConfigurationType::PRIMARY_STEREO];
    if state().settings.quad {
        types.push(sys::ViewConfigurationType::PRIMARY_QUAD_VARJO);
    }
    unsafe { two_call(capacity, count, out, &types) }
}

/// Whether a chain of input structures (`next`) has one of type `ty` whose third field, a
/// `Bool32` (`foveatedRenderingActive`), is true.
unsafe fn chained_true(mut next: *const sys::BaseInStructure, ty: sys::StructureType) -> bool {
    unsafe {
        while !next.is_null() {
            if (*next).ty == ty {
                // Both Varjo structures are { type, next, Bool32 }.
                let p = next as *const sys::ViewLocateFoveatedRenderingVARJO;
                return (*p).foveated_rendering_active.into();
            }
            next = (*next).next;
        }
    }
    false
}

unsafe extern "system" fn enumerate_view_configuration_views(
    _i: sys::Instance,
    _s: sys::SystemId,
    ty: sys::ViewConfigurationType,
    capacity: u32,
    count: *mut u32,
    out: *mut sys::ViewConfigurationView,
) -> XrResult {
    let (size, inset, quad) = {
        let s = state();
        (s.settings.size, s.settings.inset_size, s.settings.quad)
    };
    let sizes = match ty {
        sys::ViewConfigurationType::PRIMARY_STEREO => vec![size, size],
        sys::ViewConfigurationType::PRIMARY_QUAD_VARJO if quad => vec![size, size, inset, inset],
        _ => return XrResult::ERROR_VIEW_CONFIGURATION_TYPE_UNSUPPORTED,
    };
    unsafe {
        *count = sizes.len() as u32;
        if capacity == 0 {
            return XrResult::SUCCESS;
        }
        if (capacity as usize) < sizes.len() {
            return XrResult::ERROR_SIZE_INSUFFICIENT;
        }
        for (i, size) in sizes.iter().enumerate() {
            let v = &mut *out.add(i);
            // The application's chain stays: only the numbers are written.
            let foveated = i >= 2
                && chained_true(
                    v.next as *const _,
                    sys::StructureType::FOVEATED_VIEW_CONFIGURATION_VIEW_VARJO,
                );
            // Without foveation an inset covers more, at the same sharpness: a bigger image.
            let [w, h] = if i >= 2 && !foveated {
                size.map(|c| c * 3 / 2)
            } else {
                *size
            };
            v.recommended_image_rect_width = w;
            v.max_image_rect_width = w.max(4096);
            v.recommended_image_rect_height = h;
            v.max_image_rect_height = h.max(4096);
            v.recommended_swapchain_sample_count = 1;
            v.max_swapchain_sample_count = 4;
        }
    }
    XrResult::SUCCESS
}

// ----- Vulkan (XR_KHR_vulkan_enable2) -----

unsafe extern "system" fn get_vulkan_graphics_requirements(
    _i: sys::Instance,
    _s: sys::SystemId,
    reqs: *mut sys::GraphicsRequirementsVulkanKHR,
) -> XrResult {
    unsafe {
        (*reqs).min_api_version_supported = sys::Version::new(1, 1, 0);
        (*reqs).max_api_version_supported = sys::Version::new(1, 3, 0);
    }
    XrResult::SUCCESS
}

unsafe fn entry_from(get_proc: Option<sys::platform::VkGetInstanceProcAddr>) -> Option<ash::Entry> {
    let f = get_proc?;
    // SAFETY: the application's vkGetInstanceProcAddr, which has Vulkan's signature.
    let f: vk::PFN_vkGetInstanceProcAddr = unsafe { std::mem::transmute(f) };
    Some(unsafe {
        ash::Entry::from_static_fn(ash::StaticFn {
            get_instance_proc_addr: f,
        })
    })
}

unsafe extern "system" fn create_vulkan_instance(
    _i: sys::Instance,
    info: *const sys::VulkanInstanceCreateInfoKHR,
    out: *mut sys::platform::VkInstance,
    vk_result: *mut sys::platform::VkResult,
) -> XrResult {
    unsafe {
        let info = &*info;
        let Some(entry) = entry_from(info.pfn_get_instance_proc_addr) else {
            return XrResult::ERROR_VALIDATION_FAILURE;
        };
        let create = &*(info.vulkan_create_info as *const vk::InstanceCreateInfo);
        match entry.create_instance(create, None) {
            Ok(instance) => {
                *out = instance.handle().as_raw() as _;
                *vk_result = 0;
                state().vulkan = Some(Vulkan {
                    instance,
                    device: None,
                });
                XrResult::SUCCESS
            }
            Err(e) => {
                *vk_result = e.as_raw();
                XrResult::ERROR_RUNTIME_FAILURE
            }
        }
    }
}

unsafe extern "system" fn get_vulkan_graphics_device(
    _i: sys::Instance,
    _info: *const sys::VulkanGraphicsDeviceGetInfoKHR,
    out: *mut sys::platform::VkPhysicalDevice,
) -> XrResult {
    let s = state();
    let Some(v) = &s.vulkan else {
        return XrResult::ERROR_VALIDATION_FAILURE;
    };
    let devices = unsafe { v.instance.enumerate_physical_devices() }.unwrap_or_default();
    // A discrete GPU first, as a PC headset's runtime picks the one driving the display.
    let pick = devices
        .iter()
        .copied()
        .find(|&d| {
            unsafe { v.instance.get_physical_device_properties(d) }.device_type
                == vk::PhysicalDeviceType::DISCRETE_GPU
        })
        .or_else(|| devices.first().copied());
    match pick {
        Some(d) => {
            unsafe { *out = d.as_raw() as _ };
            XrResult::SUCCESS
        }
        None => XrResult::ERROR_RUNTIME_FAILURE,
    }
}

unsafe extern "system" fn create_vulkan_device(
    _i: sys::Instance,
    info: *const sys::VulkanDeviceCreateInfoKHR,
    out: *mut sys::platform::VkDevice,
    vk_result: *mut sys::platform::VkResult,
) -> XrResult {
    let mut s = state();
    let Some(v) = &mut s.vulkan else {
        return XrResult::ERROR_VALIDATION_FAILURE;
    };
    unsafe {
        let info = &*info;
        let physical = vk::PhysicalDevice::from_raw(info.vulkan_physical_device as u64);
        let create = &*(info.vulkan_create_info as *const vk::DeviceCreateInfo);
        match v.instance.create_device(physical, create, None) {
            Ok(device) => {
                *out = device.handle().as_raw() as _;
                *vk_result = 0;
                v.device = Some(device);
                XrResult::SUCCESS
            }
            Err(e) => {
                *vk_result = e.as_raw();
                XrResult::ERROR_RUNTIME_FAILURE
            }
        }
    }
}

// ----- session -----

unsafe extern "system" fn create_session(
    _i: sys::Instance,
    info: *const sys::SessionCreateInfo,
    session: *mut sys::Session,
) -> XrResult {
    let mut s = state();
    if s.session.is_some() {
        return XrResult::ERROR_LIMIT_REACHED;
    }
    let binding = unsafe { (*info).next as *const sys::GraphicsBindingVulkanKHR };
    if binding.is_null()
        || unsafe { (*binding).ty } != sys::StructureType::GRAPHICS_BINDING_VULKAN_KHR
    {
        return XrResult::ERROR_GRAPHICS_DEVICE_INVALID;
    }
    let b = unsafe { &*binding };
    s.binding = Some((b.queue_family_index, b.queue_index));
    let h = s.handle();
    s.session = Some(h);
    s.set_session_state(sys::SessionState::IDLE);
    s.set_session_state(sys::SessionState::READY);
    unsafe { *session = sys::Session::from_raw(h) };
    XrResult::SUCCESS
}

unsafe extern "system" fn destroy_session(_session: sys::Session) -> XrResult {
    let mut s = state();
    s.write_report();
    let swapchains: Vec<u64> = s.swapchains.keys().copied().collect();
    for h in swapchains {
        free_swapchain(&mut s, h);
    }
    s.session = None;
    s.session_state = None;
    XrResult::SUCCESS
}

unsafe extern "system" fn begin_session(
    _session: sys::Session,
    _info: *const sys::SessionBeginInfo,
) -> XrResult {
    let mut s = state();
    s.set_session_state(sys::SessionState::SYNCHRONIZED);
    s.set_session_state(sys::SessionState::VISIBLE);
    s.set_session_state(sys::SessionState::FOCUSED);
    XrResult::SUCCESS
}

unsafe extern "system" fn end_session(_session: sys::Session) -> XrResult {
    let mut s = state();
    s.set_session_state(sys::SessionState::IDLE);
    s.set_session_state(sys::SessionState::EXITING);
    s.write_report();
    XrResult::SUCCESS
}

unsafe extern "system" fn request_exit_session(_session: sys::Session) -> XrResult {
    let mut s = state();
    s.report.exit_requested = true;
    s.set_session_state(sys::SessionState::STOPPING);
    XrResult::SUCCESS
}

// ----- spaces and views -----

unsafe extern "system" fn enumerate_reference_spaces(
    _s: sys::Session,
    capacity: u32,
    count: *mut u32,
    out: *mut sys::ReferenceSpaceType,
) -> XrResult {
    use sys::ReferenceSpaceType as R;
    unsafe { two_call(capacity, count, out, &[R::VIEW, R::LOCAL, R::STAGE]) }
}

unsafe extern "system" fn create_reference_space(
    _session: sys::Session,
    info: *const sys::ReferenceSpaceCreateInfo,
    space: *mut sys::Space,
) -> XrResult {
    let kind = match unsafe { (*info).reference_space_type } {
        sys::ReferenceSpaceType::VIEW => SpaceKind::View,
        sys::ReferenceSpaceType::LOCAL => SpaceKind::Local,
        sys::ReferenceSpaceType::STAGE => SpaceKind::Stage,
        _ => return XrResult::ERROR_REFERENCE_SPACE_UNSUPPORTED,
    };
    let mut s = state();
    let h = s.handle();
    s.spaces.insert(h, kind);
    unsafe { *space = sys::Space::from_raw(h) };
    XrResult::SUCCESS
}

unsafe extern "system" fn destroy_space(space: sys::Space) -> XrResult {
    state().spaces.remove(&space.into_raw());
    XrResult::SUCCESS
}

fn tracked() -> sys::SpaceLocationFlags {
    sys::SpaceLocationFlags::ORIENTATION_VALID
        | sys::SpaceLocationFlags::POSITION_VALID
        | sys::SpaceLocationFlags::ORIENTATION_TRACKED
        | sys::SpaceLocationFlags::POSITION_TRACKED
}

unsafe extern "system" fn locate_space(
    space: sys::Space,
    base: sys::Space,
    _time: sys::Time,
    location: *mut sys::SpaceLocation,
) -> XrResult {
    let s = state();
    let (Some(&a), Some(&b)) = (
        s.spaces.get(&space.into_raw()),
        s.spaces.get(&base.into_raw()),
    ) else {
        return XrResult::ERROR_HANDLE_INVALID;
    };
    let (Some(space), Some(base)) = (stage_pose(&s, a), stage_pose(&s, b)) else {
        // A controller the script doesn't move: not tracked.
        unsafe {
            (*location).location_flags = sys::SpaceLocationFlags::EMPTY;
            (*location).pose = script::IDENTITY;
        }
        return XrResult::SUCCESS;
    };
    unsafe {
        (*location).location_flags = tracked();
        (*location).pose = script::compose(script::inverse(base), space);
    }
    XrResult::SUCCESS
}

/// Where a space is now in the stage (the local space is the stage here).
fn stage_pose(s: &State, kind: SpaceKind) -> Option<sys::Posef> {
    let t = s.seconds();
    match kind {
        SpaceKind::View => Some(s.script.head(t)),
        SpaceKind::Local | SpaceKind::Stage => Some(script::IDENTITY),
        SpaceKind::Action(action) => {
            let a = s.actions.get(&action)?;
            a.bindings.iter().find_map(|p| {
                if p == GAZE_PATH {
                    return gaze_pose(s, t);
                }
                s.script.pose(p, t)
            })
        }
    }
}

/// The eyes' pose binding: scripted relative to the head.
const GAZE_PATH: &str = "/user/eyes_ext/input/gaze_ext/pose";

/// Where the eyes look in the stage: the script's gaze (head relative, snapped to the grid)
/// turned with the head, from between the eyes. `None` without eye tracking or before the
/// script's first gaze step.
fn gaze_pose(s: &State, t: f32) -> Option<sys::Posef> {
    if !s.settings.eyes {
        return None;
    }
    let (yaw, pitch) = s.script.angles(GAZE_PATH, t)?;
    let (yaw, pitch) = match s.settings.gaze_step {
        Some(step) => script::snap_gaze(yaw, pitch, step),
        None => (yaw, pitch),
    };
    let head = s.script.head(t);
    Some(script::compose(head, script::pose(yaw, pitch, [0.0; 3])))
}

unsafe extern "system" fn create_action_space(
    _session: sys::Session,
    info: *const sys::ActionSpaceCreateInfo,
    space: *mut sys::Space,
) -> XrResult {
    let action = unsafe { (*info).action }.into_raw();
    let mut s = state();
    if s.actions.get(&action).map(|a| a.ty) != Some(sys::ActionType::POSE_INPUT) {
        return XrResult::ERROR_ACTION_TYPE_MISMATCH;
    }
    let h = s.handle();
    s.spaces.insert(h, SpaceKind::Action(action));
    unsafe { *space = sys::Space::from_raw(h) };
    XrResult::SUCCESS
}

unsafe extern "system" fn locate_views(
    _session: sys::Session,
    info: *const sys::ViewLocateInfo,
    view_state: *mut sys::ViewState,
    capacity: u32,
    count: *mut u32,
    out: *mut sys::View,
) -> XrResult {
    let s = state();
    let t = s.seconds();
    let head = s.script.head(t);
    let (ty, foveated) = unsafe {
        (
            (*info).view_configuration_type,
            chained_true(
                (*info).next as *const _,
                sys::StructureType::VIEW_LOCATE_FOVEATED_RENDERING_VARJO,
            ),
        )
    };
    let right = sys::Fovf {
        angle_left: -LEFT_FOV.angle_right,
        angle_right: -LEFT_FOV.angle_left,
        ..LEFT_FOV
    };
    let eye = |offset: f32, fov| sys::View {
        ty: sys::StructureType::VIEW,
        next: std::ptr::null_mut(),
        pose: script::offset(head, [offset, 0.0, 0.0]),
        fov,
    };
    let mut views = vec![eye(-IPD / 2.0, LEFT_FOV), eye(IPD / 2.0, right)];
    if ty == sys::ViewConfigurationType::PRIMARY_QUAD_VARJO && s.settings.quad {
        // Where the inset is centred, as tangents from the eye's forward (right, up).
        let looked = if foveated { gaze_pose(&s, t) } else { None }.map(|g| {
            let d = script::rotate(
                script::compose(script::inverse(head), g).orientation,
                [0.0, 0.0, -1.0],
            );
            [d[0] / -d[2].min(-1e-3), d[1] / -d[2].min(-1e-3)]
        });
        let [cx, cy] = looked.unwrap_or([0.0, 0.0]);
        for (i, wide) in [LEFT_FOV, right].into_iter().enumerate() {
            let half = INSET_HALF.to_radians().tan();
            let clamp = |c: f32, lo: f32, hi: f32| c.clamp(lo.tan() + half, hi.tan() - half);
            let (x, y) = (
                clamp(cx, wide.angle_left, wide.angle_right),
                clamp(cy, wide.angle_down, wide.angle_up),
            );
            let fov = sys::Fovf {
                angle_left: (x - half).atan(),
                angle_right: (x + half).atan(),
                angle_down: (y - half).atan(),
                angle_up: (y + half).atan(),
            };
            views.push(eye(if i == 0 { -IPD } else { IPD } / 2.0, fov));
        }
    }
    unsafe {
        (*view_state).view_state_flags = sys::ViewStateFlags::ORIENTATION_VALID
            | sys::ViewStateFlags::POSITION_VALID
            | sys::ViewStateFlags::ORIENTATION_TRACKED
            | sys::ViewStateFlags::POSITION_TRACKED;
        two_call(capacity, count, out, &views)
    }
}

/// Half an inset's width, degrees (quad views).
const INSET_HALF: f32 = 20.0;

// ----- swapchains -----

unsafe extern "system" fn enumerate_swapchain_formats(
    _s: sys::Session,
    capacity: u32,
    count: *mut u32,
    out: *mut i64,
) -> XrResult {
    unsafe { two_call(capacity, count, out, &FORMATS) }
}

unsafe extern "system" fn create_swapchain(
    _session: sys::Session,
    info: *const sys::SwapchainCreateInfo,
    out: *mut sys::Swapchain,
) -> XrResult {
    let mut s = state();
    let info = unsafe { &*info };
    let Some(device) = s.vulkan.as_ref().and_then(|v| v.device.clone()) else {
        return XrResult::ERROR_VALIDATION_FAILURE;
    };
    let instance = s.vulkan.as_ref().map(|v| v.instance.clone()).unwrap();
    let format = vk::Format::from_raw(info.format as i32);
    if !FORMATS.contains(&info.format) {
        return XrResult::ERROR_SWAPCHAIN_FORMAT_UNSUPPORTED;
    }
    let u = info.usage_flags;
    let mut usage = vk::ImageUsageFlags::TRANSFER_SRC;
    if u.contains(sys::SwapchainUsageFlags::COLOR_ATTACHMENT) {
        usage |= vk::ImageUsageFlags::COLOR_ATTACHMENT;
    }
    if u.contains(sys::SwapchainUsageFlags::SAMPLED) {
        usage |= vk::ImageUsageFlags::SAMPLED;
    }
    if u.contains(sys::SwapchainUsageFlags::TRANSFER_DST) {
        usage |= vk::ImageUsageFlags::TRANSFER_DST;
    }
    if u.contains(sys::SwapchainUsageFlags::UNORDERED_ACCESS) {
        usage |= vk::ImageUsageFlags::STORAGE;
    }
    let flags = if u.contains(sys::SwapchainUsageFlags::MUTABLE_FORMAT) {
        vk::ImageCreateFlags::MUTABLE_FORMAT
    } else {
        vk::ImageCreateFlags::empty()
    };
    let physical = {
        // The device's physical device: the one `get_vulkan_graphics_device` picked.
        let devices = unsafe { instance.enumerate_physical_devices() }.unwrap_or_default();
        devices
            .iter()
            .copied()
            .find(|&d| {
                unsafe { instance.get_physical_device_properties(d) }.device_type
                    == vk::PhysicalDeviceType::DISCRETE_GPU
            })
            .or_else(|| devices.first().copied())
    };
    let Some(physical) = physical else {
        return XrResult::ERROR_RUNTIME_FAILURE;
    };
    let memory = unsafe { instance.get_physical_device_memory_properties(physical) };
    let mut images = Vec::new();
    for _ in 0..3 {
        let create = vk::ImageCreateInfo::default()
            .flags(flags)
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: info.width,
                height: info.height,
                depth: 1,
            })
            .mip_levels(info.mip_count.max(1))
            .array_layers(info.array_size.max(1))
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let Ok(image) = (unsafe { device.create_image(&create, None) }) else {
            return XrResult::ERROR_RUNTIME_FAILURE;
        };
        let reqs = unsafe { device.get_image_memory_requirements(image) };
        let Some(kind) = (0..memory.memory_type_count).find(|&i| {
            reqs.memory_type_bits & (1 << i) != 0
                && memory.memory_types[i as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        }) else {
            return XrResult::ERROR_RUNTIME_FAILURE;
        };
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(reqs.size)
            .memory_type_index(kind);
        let Ok(mem) = (unsafe { device.allocate_memory(&alloc, None) }) else {
            return XrResult::ERROR_RUNTIME_FAILURE;
        };
        if unsafe { device.bind_image_memory(image, mem, 0) }.is_err() {
            return XrResult::ERROR_RUNTIME_FAILURE;
        }
        images.push((image, mem));
    }
    let h = s.handle();
    s.swapchains.insert(
        h,
        Swapchain {
            images,
            format,
            size: [info.width, info.height],
            next: 0,
        },
    );
    unsafe { *out = sys::Swapchain::from_raw(h) };
    XrResult::SUCCESS
}

fn free_swapchain(s: &mut State, h: u64) {
    let Some(chain) = s.swapchains.remove(&h) else {
        return;
    };
    if let Some(device) = s.vulkan.as_ref().and_then(|v| v.device.as_ref()) {
        unsafe {
            let _ = device.device_wait_idle();
            for (image, mem) in chain.images {
                device.destroy_image(image, None);
                device.free_memory(mem, None);
            }
        }
    }
}

unsafe extern "system" fn destroy_swapchain(swapchain: sys::Swapchain) -> XrResult {
    free_swapchain(&mut state(), swapchain.into_raw());
    XrResult::SUCCESS
}

unsafe extern "system" fn enumerate_swapchain_images(
    swapchain: sys::Swapchain,
    capacity: u32,
    count: *mut u32,
    out: *mut sys::SwapchainImageBaseHeader,
) -> XrResult {
    let s = state();
    let Some(chain) = s.swapchains.get(&swapchain.into_raw()) else {
        return XrResult::ERROR_HANDLE_INVALID;
    };
    let items: Vec<sys::SwapchainImageVulkanKHR> = chain
        .images
        .iter()
        .map(|(image, _)| sys::SwapchainImageVulkanKHR {
            ty: sys::StructureType::SWAPCHAIN_IMAGE_VULKAN_KHR,
            next: std::ptr::null_mut(),
            image: image.as_raw(),
        })
        .collect();
    unsafe {
        two_call(
            capacity,
            count,
            out as *mut sys::SwapchainImageVulkanKHR,
            &items,
        )
    }
}

unsafe extern "system" fn acquire_swapchain_image(
    swapchain: sys::Swapchain,
    _info: *const sys::SwapchainImageAcquireInfo,
    index: *mut u32,
) -> XrResult {
    let mut s = state();
    let Some(chain) = s.swapchains.get_mut(&swapchain.into_raw()) else {
        return XrResult::ERROR_HANDLE_INVALID;
    };
    unsafe { *index = chain.next as u32 };
    chain.next = (chain.next + 1) % chain.images.len();
    XrResult::SUCCESS
}

unsafe extern "system" fn ok_swapchain(
    _s: sys::Swapchain,
    _info: *const std::ffi::c_void,
) -> XrResult {
    XrResult::SUCCESS
}

// ----- frames -----

unsafe extern "system" fn wait_frame(
    _session: sys::Session,
    _info: *const sys::FrameWaitInfo,
    frame: *mut sys::FrameState,
) -> XrResult {
    let (deadline, period) = {
        let mut s = state();
        let period = s.settings.period;
        let now = Instant::now();
        let next = s.next_frame.map_or(now, |t| t.max(now - period));
        if next < now {
            s.report.skipped_waits += 1;
        }
        s.next_frame = Some(next + period);
        (next, period)
    };
    // Sleep without the lock: the application may call in from other threads.
    let now = Instant::now();
    if deadline > now {
        std::thread::sleep(deadline - now);
    }
    let s = state();
    let render = matches!(
        s.session_state,
        Some(sys::SessionState::VISIBLE | sys::SessionState::FOCUSED)
    );
    unsafe {
        (*frame).predicted_display_time =
            sys::Time::from_nanos(s.now().as_nanos() + period.as_nanos() as i64);
        (*frame).predicted_display_period = sys::Duration::from_nanos(period.as_nanos() as i64);
        (*frame).should_render = render.into();
    }
    XrResult::SUCCESS
}

unsafe extern "system" fn begin_frame(
    _session: sys::Session,
    _info: *const sys::FrameBeginInfo,
) -> XrResult {
    XrResult::SUCCESS
}

unsafe extern "system" fn end_frame(
    _session: sys::Session,
    info: *const sys::FrameEndInfo,
) -> XrResult {
    let mut s = state();
    let now = Instant::now();
    if let Some(last) = s.last_end.replace(now) {
        s.report.intervals.push((now - last).as_secs_f32() * 1000.0);
    }
    s.report.frames += 1;
    let frame = s.report.frames;
    let info = unsafe { &*info };
    let layers = unsafe { std::slice::from_raw_parts(info.layers, info.layer_count as usize) };
    let latest = s.settings.capture_every.is_some_and(|n| frame % n == 0);
    let now_s = s.seconds();
    let timed = s
        .settings
        .capture_seconds
        .iter()
        .position(|(t, _)| *t <= now_s)
        .map(|i| s.settings.capture_seconds.remove(i).1);
    let video = s
        .settings
        .video
        .is_some_and(|(a, b)| (a..=b).contains(&frame));
    if video {
        for &layer in layers {
            if unsafe { (*layer).ty } == sys::StructureType::COMPOSITION_LAYER_PROJECTION {
                let p = unsafe { &*(layer as *const sys::CompositionLayerProjection) };
                let sub = unsafe { (*p.views).sub_image };
                capture_image(&mut s, &sub, &format!("video{frame:05}.png"));
                break;
            }
        }
    }
    let capture = latest || s.settings.capture.contains(&frame) || timed.is_some();
    let tag = if let Some(t) = timed {
        format!("t{t}")
    } else if latest && !s.settings.capture.contains(&frame) {
        "latest".to_owned()
    } else {
        format!("frame{frame}")
    };
    // What the composite captures need: each eye and each quad.
    let mut eyes = Vec::new();
    let mut quads = Vec::new();
    for (i, &layer) in layers.iter().enumerate() {
        let ty = unsafe { (*layer).ty };
        let name = match ty {
            sys::StructureType::COMPOSITION_LAYER_PROJECTION => "projection",
            sys::StructureType::COMPOSITION_LAYER_QUAD => "quad",
            _ => "other",
        };
        *s.report.layers.entry(name.into()).or_default() += 1;
        if !capture {
            continue;
        }
        match ty {
            sys::StructureType::COMPOSITION_LAYER_PROJECTION => {
                let p = unsafe { &*(layer as *const sys::CompositionLayerProjection) };
                let views = unsafe { std::slice::from_raw_parts(p.views, p.view_count as usize) };
                for (eye, v) in views.iter().enumerate() {
                    let name = format!("{tag}-layer{i}-eye{eye}.png");
                    capture_image(&mut s, &v.sub_image, &name);
                    eyes.push((v.pose, v.fov, v.sub_image));
                }
            }
            sys::StructureType::COMPOSITION_LAYER_QUAD => {
                let q = unsafe { &*(layer as *const sys::CompositionLayerQuad) };
                let name = format!("{tag}-layer{i}-quad.png");
                capture_image(&mut s, &q.sub_image, &name);
                quads.push((q.pose, q.size, q.sub_image));
            }
            _ => {}
        }
    }
    if !quads.is_empty() {
        for (eye, view) in eyes.iter().enumerate() {
            let name = format!("{tag}-eye{eye}-composite.png");
            let result = composite(&s, view, &quads)
                .and_then(|(w, h, rgba)| write_png(&s.settings.out.join(&name), w, h, &rgba));
            match result {
                Ok(()) => {
                    if !s.report.captures.contains(&name) {
                        s.report.captures.push(name);
                    }
                }
                Err(e) => s.report.errors.push(format!("capture {name}: {e}")),
            }
        }
    }
    if s.settings.frames.is_some_and(|n| frame == n) {
        s.report.exit_requested = true;
        s.set_session_state(sys::SessionState::STOPPING);
    }
    if frame % 90 == 0 {
        s.write_report();
    }
    XrResult::SUCCESS
}

/// Copies the image a layer shows into a PNG in the output folder.
fn capture_image(s: &mut State, sub: &sys::SwapchainSubImage, name: &str) {
    let result = read_image(s, sub)
        .and_then(|(w, h, pixels)| write_png(&s.settings.out.join(name), w, h, &pixels));
    match result {
        Ok(()) => {
            if !s.report.captures.iter().any(|c| c == name) {
                s.report.captures.push(name.to_owned());
            }
        }
        Err(e) => s.report.errors.push(format!("capture {name}: {e}")),
    }
}

/// One eye's image with each quad drawn over it as a compositor would: each pixel's ray
/// meets each quad's plane (either side), later quads on top, premultiplied alpha. RGBA
/// rows, top first.
fn composite(
    s: &State,
    (pose, fov, sub): &(sys::Posef, sys::Fovf, sys::SwapchainSubImage),
    quads: &[(sys::Posef, sys::Extent2Df, sys::SwapchainSubImage)],
) -> Result<(u32, u32, Vec<u8>), String> {
    let (w, h, mut out) = read_image(s, sub)?;
    let images = quads
        .iter()
        .map(|(_, _, sub)| read_image(s, sub))
        .collect::<Result<Vec<_>, _>>()?;
    let (l, r) = (fov.angle_left.tan(), fov.angle_right.tan());
    let (d, u) = (fov.angle_down.tan(), fov.angle_up.tan());
    // Each quad's frame, with the eye's position and turn in it.
    let frames: Vec<_> = quads
        .iter()
        .map(|(q, size, _)| {
            let p = script::compose(script::inverse(*q), *pose);
            (
                p.orientation,
                [p.position.x, p.position.y, p.position.z],
                *size,
            )
        })
        .collect();
    for py in 0..h {
        for px in 0..w {
            let fx = (px as f32 + 0.5) / w as f32;
            let fy = (py as f32 + 0.5) / h as f32;
            let ray = [l + fx * (r - l), u - fy * (u - d), -1.0];
            let at = ((py * w + px) * 4) as usize;
            for ((turn, from, size), (qw, qh, image)) in frames.iter().zip(&images) {
                let dir = script::rotate(*turn, ray);
                if dir[2].abs() < 1e-6 {
                    continue;
                }
                let t = -from[2] / dir[2];
                if t <= 0.0 {
                    continue;
                }
                let x = (from[0] + t * dir[0]) / size.width + 0.5;
                let y = 0.5 - (from[1] + t * dir[1]) / size.height;
                if !(0.0..1.0).contains(&x) || !(0.0..1.0).contains(&y) {
                    continue;
                }
                let (sx, sy) = ((x * *qw as f32) as u32, (y * *qh as f32) as u32);
                let src = ((sy.min(qh - 1) * qw + sx.min(qw - 1)) * 4) as usize;
                let a = f32::from(image[src + 3]) / 255.0;
                for c in 0..3 {
                    let v = f32::from(image[src + c]) + f32::from(out[at + c]) * (1.0 - a);
                    out[at + c] = v.min(255.0) as u8;
                }
                out[at + 3] = 255;
            }
        }
    }
    Ok((w, h, out))
}

/// The image a layer shows, read back as RGBA rows (top first). The application leaves it
/// in COLOR_ATTACHMENT_OPTIMAL, as OpenXR asks for a released image.
fn read_image(s: &State, sub: &sys::SwapchainSubImage) -> Result<(u32, u32, Vec<u8>), String> {
    (|| -> Result<(u32, u32, Vec<u8>), String> {
        let chain = s
            .swapchains
            .get(&sub.swapchain.into_raw())
            .ok_or("no swapchain")?;
        let v = s.vulkan.as_ref().ok_or("no Vulkan")?;
        let device = v.device.as_ref().ok_or("no device")?;
        let (family, index) = s.binding.ok_or("no session")?;
        // The image the application released last.
        let slot = (chain.next + chain.images.len() - 1) % chain.images.len();
        let image = chain.images[slot].0;
        let [w, h] = chain.size;
        let format = chain.format;
        unsafe {
            let queue = device.get_device_queue(family, index);
            let pool = device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default().queue_family_index(family),
                    None,
                )
                .map_err(|e| e.to_string())?;
            let cmd = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(pool)
                        .command_buffer_count(1),
                )
                .map_err(|e| e.to_string())?[0];
            let bytes = u64::from(w) * u64::from(h) * 4;
            let buffer = device
                .create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(bytes)
                        .usage(vk::BufferUsageFlags::TRANSFER_DST),
                    None,
                )
                .map_err(|e| e.to_string())?;
            let reqs = device.get_buffer_memory_requirements(buffer);
            let physical = v
                .instance
                .enumerate_physical_devices()
                .map_err(|e| e.to_string())?;
            let props = v.instance.get_physical_device_memory_properties(
                physical
                    .iter()
                    .copied()
                    .find(|&d| {
                        v.instance.get_physical_device_properties(d).device_type
                            == vk::PhysicalDeviceType::DISCRETE_GPU
                    })
                    .or_else(|| physical.first().copied())
                    .ok_or("no GPU")?,
            );
            let host = (0..props.memory_type_count)
                .find(|&i| {
                    reqs.memory_type_bits & (1 << i) != 0
                        && props.memory_types[i as usize].property_flags.contains(
                            vk::MemoryPropertyFlags::HOST_VISIBLE
                                | vk::MemoryPropertyFlags::HOST_COHERENT,
                        )
                })
                .ok_or("no host memory")?;
            let memory = device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(reqs.size)
                        .memory_type_index(host),
                    None,
                )
                .map_err(|e| e.to_string())?;
            device
                .bind_buffer_memory(buffer, memory, 0)
                .map_err(|e| e.to_string())?;
            device
                .begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default())
                .map_err(|e| e.to_string())?;
            let range = vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: sub.image_array_index,
                layer_count: 1,
            };
            let barrier = |old, new, src, dst| {
                vk::ImageMemoryBarrier::default()
                    .old_layout(old)
                    .new_layout(new)
                    .src_access_mask(src)
                    .dst_access_mask(dst)
                    .image(image)
                    .subresource_range(range)
            };
            device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier(
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    vk::AccessFlags::MEMORY_WRITE,
                    vk::AccessFlags::TRANSFER_READ,
                )],
            );
            device.cmd_copy_image_to_buffer(
                cmd,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer,
                &[vk::BufferImageCopy {
                    buffer_offset: 0,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: sub.image_array_index,
                        layer_count: 1,
                    },
                    image_offset: vk::Offset3D::default(),
                    image_extent: vk::Extent3D {
                        width: w,
                        height: h,
                        depth: 1,
                    },
                }],
            );
            device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier(
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::AccessFlags::TRANSFER_READ,
                    vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE,
                )],
            );
            device.end_command_buffer(cmd).map_err(|e| e.to_string())?;
            let cmds = [cmd];
            device
                .queue_submit(
                    queue,
                    &[vk::SubmitInfo::default().command_buffers(&cmds)],
                    vk::Fence::null(),
                )
                .map_err(|e| e.to_string())?;
            device.queue_wait_idle(queue).map_err(|e| e.to_string())?;
            let ptr = device
                .map_memory(memory, 0, bytes, vk::MemoryMapFlags::empty())
                .map_err(|e| e.to_string())? as *const u8;
            let mut pixels = std::slice::from_raw_parts(ptr, bytes as usize).to_vec();
            device.unmap_memory(memory);
            device.destroy_buffer(buffer, None);
            device.free_memory(memory, None);
            device.destroy_command_pool(pool, None);
            if matches!(
                format,
                vk::Format::B8G8R8A8_SRGB | vk::Format::B8G8R8A8_UNORM
            ) {
                for p in pixels.chunks_exact_mut(4) {
                    p.swap(0, 2);
                }
            }
            Ok((w, h, pixels))
        }
    })()
}

fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())
}

// ----- paths and actions -----

unsafe extern "system" fn string_to_path(
    _i: sys::Instance,
    text: *const c_char,
    path: *mut sys::Path,
) -> XrResult {
    let text = unsafe { read_str(text) };
    let mut s = state();
    let index = match s.paths.iter().position(|p| *p == text) {
        Some(i) => i,
        None => {
            s.paths.push(text);
            s.paths.len() - 1
        }
    };
    unsafe { *path = sys::Path::from_raw(index as u64 + 1) };
    XrResult::SUCCESS
}

unsafe extern "system" fn path_to_string(
    _i: sys::Instance,
    path: sys::Path,
    capacity: u32,
    count: *mut u32,
    buffer: *mut c_char,
) -> XrResult {
    let s = state();
    let Some(text) = s.paths.get(path.into_raw().wrapping_sub(1) as usize) else {
        return XrResult::ERROR_PATH_INVALID;
    };
    let mut bytes: Vec<c_char> = text.bytes().map(|b| b as c_char).collect();
    bytes.push(0);
    unsafe { two_call(capacity, count, buffer, &bytes) }
}

unsafe extern "system" fn create_action_set(
    _i: sys::Instance,
    _info: *const sys::ActionSetCreateInfo,
    out: *mut sys::ActionSet,
) -> XrResult {
    let h = state().handle();
    unsafe { *out = sys::ActionSet::from_raw(h) };
    XrResult::SUCCESS
}

unsafe extern "system" fn create_action(
    _set: sys::ActionSet,
    info: *const sys::ActionCreateInfo,
    out: *mut sys::Action,
) -> XrResult {
    let info = unsafe { &*info };
    let name = unsafe { read_str(info.action_name.as_ptr()) };
    let mut s = state();
    let h = s.handle();
    s.report.actions.push(name.clone());
    s.actions.insert(
        h,
        Action {
            name,
            ty: info.action_type,
            bindings: Vec::new(),
        },
    );
    unsafe { *out = sys::Action::from_raw(h) };
    XrResult::SUCCESS
}

unsafe extern "system" fn destroy_handle(_h: u64) -> XrResult {
    XrResult::SUCCESS
}

unsafe extern "system" fn suggest_bindings(
    _i: sys::Instance,
    suggested: *const sys::InteractionProfileSuggestedBinding,
) -> XrResult {
    let suggested = unsafe { &*suggested };
    let bindings = unsafe {
        std::slice::from_raw_parts(
            suggested.suggested_bindings,
            suggested.count_suggested_bindings as usize,
        )
    };
    let mut s = state();
    for b in bindings {
        let path = s
            .paths
            .get(b.binding.into_raw().wrapping_sub(1) as usize)
            .cloned()
            .unwrap_or_default();
        if let Some(action) = s.actions.get_mut(&b.action.into_raw())
            && !action.bindings.contains(&path)
        {
            action.bindings.push(path);
        }
    }
    XrResult::SUCCESS
}

unsafe extern "system" fn ok_session(_s: sys::Session, _info: *const std::ffi::c_void) -> XrResult {
    XrResult::SUCCESS
}

/// The script's value on the first of the action's bindings it sets.
fn action_value(s: &State, action: sys::Action) -> Option<[f32; 2]> {
    let a = s.actions.get(&action.into_raw())?;
    let _ = (&a.name, a.ty);
    let t = s.seconds();
    a.bindings.iter().find_map(|p| s.script.input(p, t))
}

unsafe extern "system" fn get_action_state_boolean(
    _session: sys::Session,
    info: *const sys::ActionStateGetInfo,
    out: *mut sys::ActionStateBoolean,
) -> XrResult {
    let s = state();
    let v = action_value(&s, unsafe { (*info).action }).unwrap_or_default();
    unsafe {
        (*out).current_state = (v[0] > 0.5).into();
        (*out).changed_since_last_sync = sys::FALSE;
        (*out).last_change_time = s.now();
        (*out).is_active = sys::TRUE;
    }
    XrResult::SUCCESS
}

unsafe extern "system" fn get_action_state_float(
    _session: sys::Session,
    info: *const sys::ActionStateGetInfo,
    out: *mut sys::ActionStateFloat,
) -> XrResult {
    let s = state();
    let v = action_value(&s, unsafe { (*info).action }).unwrap_or_default();
    unsafe {
        (*out).current_state = v[0];
        (*out).changed_since_last_sync = sys::FALSE;
        (*out).last_change_time = s.now();
        (*out).is_active = sys::TRUE;
    }
    XrResult::SUCCESS
}

unsafe extern "system" fn get_action_state_pose(
    _session: sys::Session,
    info: *const sys::ActionStateGetInfo,
    out: *mut sys::ActionStatePose,
) -> XrResult {
    let s = state();
    let action = unsafe { (*info).action }.into_raw();
    let active = stage_pose(&s, SpaceKind::Action(action)).is_some();
    unsafe { (*out).is_active = active.into() };
    XrResult::SUCCESS
}

unsafe extern "system" fn get_action_state_vector2f(
    _session: sys::Session,
    info: *const sys::ActionStateGetInfo,
    out: *mut sys::ActionStateVector2f,
) -> XrResult {
    let s = state();
    let v = action_value(&s, unsafe { (*info).action }).unwrap_or_default();
    unsafe {
        (*out).current_state = sys::Vector2f { x: v[0], y: v[1] };
        (*out).changed_since_last_sync = sys::FALSE;
        (*out).last_change_time = s.now();
        (*out).is_active = sys::TRUE;
    }
    XrResult::SUCCESS
}
