//! OpenAL Soft sources, shared static buffers and bounded streaming queues.
//! Native handles and the haptics callback's lifetime stay in this crate.
#![deny(unsafe_op_in_unsafe_fn)]

use libloading::Library;
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

const ALC_CONNECTED: i32 = 0x313;
const ALC_DEFAULT_ALL_DEVICES_SPECIFIER: i32 = 0x1012;
const AL_BUFFERS_PROCESSED: i32 = 0x1016;
const STREAM_BUFFERS: usize = 3;
const ALC_FREQUENCY: i32 = 0x1007;
const ALC_DEVICE_SPECIFIER: i32 = 0x1005;
const ALC_ALL_DEVICES_SPECIFIER: i32 = 0x1013;
const ALC_OUTPUT_MODE_SOFT: i32 = 0x19AC;
const ALC_QUAD_SOFT: i32 = 0x1503;
const ALC_HRTF_SOFT: i32 = 0x1992;
const ALC_NUM_HRTF_SPECIFIERS_SOFT: i32 = 0x1994;
const ALC_HRTF_ID_SOFT: i32 = 0x1996;
const ALC_OUTPUT_LIMITER_SOFT: i32 = 0x199a;
const AL_BUFFER: i32 = 0x1009;
const AL_SOURCE_RELATIVE: i32 = 0x0202;
const AL_DIRECT_CHANNELS_SOFT: i32 = 0x1033;
const AL_FORMAT_STEREO_FLOAT32: i32 = 0x10011;
const AL_FORMAT_QUAD32: i32 = 0x1206;
const AL_FORMAT_MONO_FLOAT32: i32 = 0x10010;
const AL_GAIN: i32 = 0x100a;
const AL_PITCH: i32 = 0x1003;
const AL_POSITION: i32 = 0x1004;
const AL_ORIENTATION: i32 = 0x100f;
const AL_LOOPING: i32 = 0x1007;
const AL_SOURCE_STATE: i32 = 0x1010;
const AL_PLAYING: i32 = 0x1012;
const AL_STOPPED: i32 = 0x1014;
const AL_SOURCE_DISTANCE_MODEL: i32 = 0xd000;
const AL_LINEAR_DISTANCE: i32 = 0xd003;
const AL_MAX_DISTANCE: i32 = 0x1023;
const AL_ROLLOFF_FACTOR: i32 = 0x1021;
const AL_REFERENCE_DISTANCE: i32 = 0x1020;
const AL_SOURCE_DISTANCE_MODEL_EXT: i32 = 0x200;

type BufferCallback = unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32;
type SetCallback = unsafe extern "C" fn(u32, i32, i32, BufferCallback, *mut c_void);
type Render = Box<dyn FnMut(&mut [f32], u32) + Send>;

macro_rules! api {
    ($($name:ident: $kind:ty = $symbol:literal),+ $(,)?) => {
        struct Api {
            $($name: $kind,)+
            _library: Library,
        }
        impl Api {
            fn load() -> Result<&'static Self, String> {
                // Native backends and thread-local context destructors can
                // retain library code after a device closes. Keep it loaded
                // for the process lifetime, including after device enumeration.
                static API: OnceLock<Result<Api, String>> = OnceLock::new();
                API.get_or_init(|| {
                    let library = load_library()?;
                    // SAFETY: these are the published OpenAL/ALC C signatures.
                    // The library outlives every copied function pointer.
                    unsafe {
                        Ok(Self {
                            $($name: *library
                                .get::<$kind>(concat!($symbol, "\0").as_bytes())
                                .map_err(|e| format!("OpenAL symbol {}: {e}", $symbol))?,)+
                            _library: library,
                        })
                    }
                }).as_ref().map_err(Clone::clone)
            }
        }
    };
}

api! {
    open_device: unsafe extern "C" fn(*const c_char) -> *mut c_void = "alcOpenDevice",
    close_device: unsafe extern "C" fn(*mut c_void) -> c_char = "alcCloseDevice",
    create_context: unsafe extern "C" fn(*mut c_void, *const i32) -> *mut c_void = "alcCreateContext",
    destroy_context: unsafe extern "C" fn(*mut c_void) = "alcDestroyContext",
    // OpenAL Soft's ALC_EXT_thread_local_context: independent streams created
    // on different game/test threads must never replace each other's context.
    make_current: unsafe extern "C" fn(*mut c_void) -> c_char = "alcSetThreadContext",
    current_context: unsafe extern "C" fn() -> *mut c_void = "alcGetThreadContext",
    device_string: unsafe extern "C" fn(*mut c_void, i32) -> *const c_char = "alcGetString",
    device_extension: unsafe extern "C" fn(*mut c_void, *const c_char) -> c_char = "alcIsExtensionPresent",
    device_integer: unsafe extern "C" fn(*mut c_void, i32, i32, *mut i32) = "alcGetIntegerv",
    extension: unsafe extern "C" fn(*const c_char) -> c_char = "alIsExtensionPresent",
    proc_address: unsafe extern "C" fn(*const c_char) -> *mut c_void = "alGetProcAddress",
    gen_buffers: unsafe extern "C" fn(i32, *mut u32) = "alGenBuffers",
    delete_buffers: unsafe extern "C" fn(i32, *const u32) = "alDeleteBuffers",
    queue_buffers: unsafe extern "C" fn(u32, i32, *const u32) = "alSourceQueueBuffers",
    unqueue_buffers: unsafe extern "C" fn(u32, i32, *mut u32) = "alSourceUnqueueBuffers",
    gen_sources: unsafe extern "C" fn(i32, *mut u32) = "alGenSources",
    delete_sources: unsafe extern "C" fn(i32, *const u32) = "alDeleteSources",
    source_integer: unsafe extern "C" fn(u32, i32, i32) = "alSourcei",
    source_float: unsafe extern "C" fn(u32, i32, f32) = "alSourcef",
    source_position: unsafe extern "C" fn(u32, i32, f32, f32, f32) = "alSource3f",
    source_state: unsafe extern "C" fn(u32, i32, *mut i32) = "alGetSourcei",
    listener_position: unsafe extern "C" fn(i32, f32, f32, f32) = "alListener3f",
    listener_orientation: unsafe extern "C" fn(i32, *const f32) = "alListenerfv",
    buffer_data: unsafe extern "C" fn(u32, i32, *const c_void, i32, i32) = "alBufferData",
    enable: unsafe extern "C" fn(i32) = "alEnable",
    play: unsafe extern "C" fn(u32) = "alSourcePlay",
    rewind: unsafe extern "C" fn(u32) = "alSourceRewind",
    pause: unsafe extern "C" fn(u32) = "alSourcePause",
    stop: unsafe extern "C" fn(u32) = "alSourceStop",
    error: unsafe extern "C" fn() -> i32 = "alGetError",
}

fn load_library() -> Result<Library, String> {
    #[cfg(target_os = "windows")]
    let names = ["OpenAL32.dll"];
    #[cfg(target_os = "macos")]
    let names = ["libopenal.1.dylib", "libopenal.dylib"];
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let names = ["libopenal.so.1", "libopenal.so"];
    let candidates = match std::env::var_os("HORIZON_OPENAL_LIBRARY") {
        Some(path) => vec![PathBuf::from(path)],
        None => {
            let mut paths = Vec::new();
            if let Ok(exe) = std::env::current_exe() {
                for directory in library_dirs(&exe) {
                    paths.extend(names.iter().map(|name| directory.join(name)));
                }
            }
            paths.extend(names.map(PathBuf::from));
            paths
        }
    };
    let mut errors = Vec::new();
    for path in candidates {
        // SAFETY: the bundled or system OpenAL library has the published C ABI.
        match unsafe { Library::new(&path) } {
            Ok(library) => return Ok(library),
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    Err(format!(
        "OpenAL Soft is unavailable ({})",
        errors.join("; ")
    ))
}

fn library_dirs(exe: &Path) -> Vec<PathBuf> {
    let Some(directory) = exe.parent() else {
        return Vec::new();
    };
    let mut dirs = vec![directory.join("native")];
    // The macOS launcher hard-links the game into this bundle, leaving the
    // signed native library in the enclosing version folder.
    if directory.ends_with("Horizon.app/Contents/MacOS")
        && let Some(version) = directory.ancestors().nth(3)
    {
        dirs.push(version.join("native"));
    }
    dirs
}

impl Api {
    fn check(&self, operation: &str) -> Result<(), String> {
        // SAFETY: callers have made their live context current.
        let error = unsafe { (self.error)() };
        if error == 0 {
            Ok(())
        } else {
            Err(format!("OpenAL {operation} failed (0x{error:04x})"))
        }
    }
}

struct CurrentContext<'a> {
    api: &'a Api,
    previous: *mut c_void,
}
impl<'a> CurrentContext<'a> {
    fn enter(api: &'a Api, context: *mut c_void) -> Result<Self, String> {
        // SAFETY: context belongs to a live OutputStream on this thread.
        unsafe {
            let previous = (api.current_context)();
            if (api.make_current)(context) == 0 {
                return Err("OpenAL could not make its output context current".into());
            }
            Ok(Self { api, previous })
        }
    }
}
impl Drop for CurrentContext<'_> {
    fn drop(&mut self) {
        // SAFETY: the previous context is still owned by its output stream.
        unsafe { (self.api.make_current)(self.previous) };
    }
}

struct CallbackState {
    render: Mutex<Render>,
    sample_rate: u32,
    channels: usize,
    failed: AtomicBool,
}

unsafe extern "C" fn render_callback(user: *mut c_void, samples: *mut c_void, bytes: i32) -> i32 {
    if user.is_null() || samples.is_null() {
        return 0;
    }
    // SAFETY: OutputStream owns this state until the native source is stopped and deleted.
    let state = unsafe { &*user.cast::<CallbackState>() };
    let frame_bytes = state.channels * size_of::<f32>();
    if bytes <= 0
        || !(bytes as usize).is_multiple_of(frame_bytes)
        || !(samples as usize).is_multiple_of(align_of::<f32>())
    {
        return 0;
    }
    // SAFETY: OpenAL supplies an aligned writable block for the stereo-f32
    // format, and OutputStream keeps this boxed state alive until deletion of
    // the source and its buffer has stopped all callbacks.
    let data = unsafe {
        std::slice::from_raw_parts_mut(samples.cast::<f32>(), bytes as usize / size_of::<f32>())
    };
    data.fill(0.0);
    if !state.failed.load(Ordering::Relaxed) {
        let result = catch_unwind(AssertUnwindSafe(|| {
            if let Ok(mut render) = state.render.try_lock() {
                render(data, state.sample_rate);
            }
        }));
        if result.is_err() {
            state.failed.store(true, Ordering::Relaxed);
            data.fill(0.0);
        }
    }
    bytes
}

/// A continuously rendered stereo output. The stream owns the callback and
/// cannot move to another thread while native context operations are in use.
/// The renderer runs on OpenAL's real-time thread: it must not block or allocate.
pub struct OutputStream {
    api: &'static Api,
    device: *mut c_void,
    context: *mut c_void,
    source: u32,
    buffer: u32,
    callback: Box<CallbackState>,
    name: String,
}
impl OutputStream {
    pub fn open(
        device_name: &str,
        render: impl FnMut(&mut [f32], u32) + Send + 'static,
    ) -> Result<Self, String> {
        Self::open_with_format(device_name, 2, render)
    }

    /// Four direct output channels, used for a USB DualSense's two actuators.
    /// Fails when OpenAL cannot confirm quad output, so haptics never leak into speakers.
    pub fn open_quad(
        device_name: &str,
        render: impl FnMut(&mut [f32], u32) + Send + 'static,
    ) -> Result<Self, String> {
        Self::open_with_format(device_name, 4, render)
    }

    fn open_with_format(
        device_name: &str,
        channels: usize,
        render: impl FnMut(&mut [f32], u32) + Send + 'static,
    ) -> Result<Self, String> {
        let api = Api::load()?;
        let name = CString::new(device_name).map_err(|_| "Invalid OpenAL device name")?;
        // SAFETY: the optional device name is NUL-terminated for this call.
        let device = unsafe {
            (api.open_device)(if device_name.is_empty() {
                ptr::null()
            } else {
                name.as_ptr()
            })
        };
        if device.is_null() {
            return Err(format!(
                "OpenAL could not open audio device '{device_name}'"
            ));
        }
        let mut stream = Self {
            api,
            device,
            context: ptr::null_mut(),
            source: 0,
            buffer: 0,
            callback: Box::new(CallbackState {
                render: Mutex::new(Box::new(render)),
                sample_rate: 48_000,
                channels,
                failed: AtomicBool::new(false),
            }),
            name: device_name.into(),
        };
        // SAFETY: all handles are owned by stream; its destructor also handles
        // partial initialization if any of the following operations fails.
        unsafe {
            if channels == 4
                && (stream.api.device_extension)(device, c"ALC_SOFT_output_mode".as_ptr()) == 0
            {
                return Err("OpenAL output mode query is unavailable".into());
            }
            let attributes = if channels == 4 {
                [
                    ALC_FREQUENCY,
                    48_000,
                    ALC_OUTPUT_MODE_SOFT,
                    ALC_QUAD_SOFT,
                    0,
                ]
            } else {
                [ALC_FREQUENCY, 48_000, 0, 0, 0]
            };
            stream.context = (stream.api.create_context)(device, attributes.as_ptr());
            if stream.context.is_null() {
                return Err("OpenAL could not create an output context".into());
            }
            let _current = CurrentContext::enter(stream.api, stream.context)?;
            for extension in [c"AL_SOFT_callback_buffer", c"AL_EXT_FLOAT32"] {
                if (stream.api.extension)(extension.as_ptr()) == 0 {
                    return Err(format!(
                        "OpenAL Soft output needs {}",
                        extension.to_string_lossy()
                    ));
                }
            }
            if channels == 4 {
                for extension in [c"AL_EXT_MCFORMATS", c"AL_SOFT_direct_channels"] {
                    if (stream.api.extension)(extension.as_ptr()) == 0 {
                        return Err(format!(
                            "OpenAL haptics need {}",
                            extension.to_string_lossy()
                        ));
                    }
                }
                let mut output_mode = 0;
                (stream.api.device_integer)(device, ALC_OUTPUT_MODE_SOFT, 1, &mut output_mode);
                if output_mode != ALC_QUAD_SOFT {
                    return Err("OpenAL device does not expose direct quad output".into());
                }
            }
            let mut rate = 0;
            (stream.api.device_integer)(device, ALC_FREQUENCY, 1, &mut rate);
            if rate <= 0 {
                return Err("OpenAL returned an invalid output sample rate".into());
            }
            stream.callback.sample_rate = rate as u32;
            let actual_name = (stream.api.device_string)(device, ALC_DEVICE_SPECIFIER);
            if !actual_name.is_null() {
                stream.name = CStr::from_ptr(actual_name).to_string_lossy().into_owned();
            }
            let address = (stream.api.proc_address)(c"alBufferCallbackSOFT".as_ptr());
            if address.is_null() {
                return Err("OpenAL callback entry point is unavailable".into());
            }
            let set_callback: SetCallback = std::mem::transmute(address);
            (stream.api.gen_buffers)(1, &mut stream.buffer);
            (stream.api.gen_sources)(1, &mut stream.source);
            stream.api.check("creating output")?;
            set_callback(
                stream.buffer,
                if channels == 4 {
                    AL_FORMAT_QUAD32
                } else {
                    AL_FORMAT_STEREO_FLOAT32
                },
                rate,
                render_callback,
                (&mut *stream.callback as *mut CallbackState).cast(),
            );
            (stream.api.source_integer)(stream.source, AL_SOURCE_RELATIVE, 1);
            if channels == 4 {
                (stream.api.source_integer)(stream.source, AL_DIRECT_CHANNELS_SOFT, 1);
            }
            (stream.api.source_integer)(stream.source, AL_BUFFER, stream.buffer as i32);
            stream.api.check("attaching callback")?;
            (stream.api.play)(stream.source);
            stream.api.check("starting output")?;
        }
        Ok(stream)
    }

    pub fn device_name(&self) -> &str {
        &self.name
    }

    pub fn sample_rate(&self) -> u32 {
        self.callback.sample_rate
    }
}
impl Drop for OutputStream {
    fn drop(&mut self) {
        // SAFETY: stop and delete the source before freeing callback state.
        // Deleting the context also stops callbacks after a device failure.
        unsafe {
            if !self.context.is_null() {
                if let Ok(mut current) = CurrentContext::enter(self.api, self.context) {
                    if self.source != 0 {
                        (self.api.stop)(self.source);
                        (self.api.delete_sources)(1, &self.source);
                    }
                    if self.buffer != 0 {
                        (self.api.delete_buffers)(1, &self.buffer);
                    }
                    if current.previous == self.context {
                        current.previous = ptr::null_mut();
                    }
                }
                (self.api.destroy_context)(self.context);
            }
            if !self.device.is_null() {
                (self.api.close_device)(self.device);
            }
        }
        if self.callback.failed.load(Ordering::Relaxed) {
            log::error!("Horizon: OpenAL renderer panicked; output was silenced");
        }
    }
}

/// Playback state of one vanilla-style OpenAL source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceState {
    Initial,
    Playing,
    Paused,
    Stopped,
}

/// Parameters applied to one source. The caller retains decoded samples;
/// OpenAL copies them into its buffer on first attachment.
pub struct VoiceSettings<'a> {
    pub samples: Option<(&'a Arc<Vec<f32>>, u32)>,
    pub state: VoiceState,
    pub gain: f32,
    pub pitch: f32,
    pub max_distance: Option<f32>,
    pub looping: bool,
    pub position: [f32; 3],
    pub relative: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct Parameters {
    gain: f32,
    pitch: f32,
    max_distance: Option<f32>,
    looping: bool,
    position: [f32; 3],
    relative: bool,
}
struct CachedBuffer {
    buffer: u32,
    samples: Weak<Vec<f32>>,
}
struct StreamBuffers {
    buffers: [u32; STREAM_BUFFERS],
    free: Vec<u32>,
    finished: bool,
}
struct Voice {
    source: u32,
    buffer: u32,
    stream: Option<StreamBuffers>,
    applied: Option<Parameters>,
    started: bool,
}

/// One OpenAL device and context with an independent AL source per sound.
pub struct NativeOutput {
    api: &'static Api,
    device: *mut c_void,
    context: *mut c_void,
    voices: HashMap<u64, Voice>,
    buffers: HashMap<(usize, u32), CachedBuffer>,
    name: String,
}

impl NativeOutput {
    pub fn open(device_name: &str) -> Result<Self, String> {
        let api = Api::load()?;
        let name = CString::new(device_name).map_err(|_| "Invalid OpenAL device name")?;
        // SAFETY: the optional device name is a valid NUL-terminated C string.
        let device = unsafe {
            (api.open_device)(if device_name.is_empty() {
                ptr::null()
            } else {
                name.as_ptr()
            })
        };
        if device.is_null() {
            return Err(format!(
                "OpenAL could not open audio device '{device_name}'"
            ));
        }
        let mut output = Self {
            api,
            device,
            context: ptr::null_mut(),
            voices: HashMap::new(),
            buffers: HashMap::new(),
            name: device_name.into(),
        };
        // SAFETY: this output owns the device and context; Drop handles partial initialization.
        unsafe {
            let mut attributes = Vec::with_capacity(7);
            if (api.device_extension)(device, c"ALC_SOFT_HRTF".as_ptr()) != 0 {
                let mut count = 0;
                (api.device_integer)(device, ALC_NUM_HRTF_SPECIFIERS_SOFT, 1, &mut count);
                if count > 0 {
                    attributes.extend([ALC_HRTF_SOFT, 0, ALC_HRTF_ID_SOFT, 0]);
                }
            }
            if (api.device_extension)(device, c"ALC_SOFT_output_limiter".as_ptr()) != 0 {
                attributes.extend([ALC_OUTPUT_LIMITER_SOFT, 1]);
            }
            attributes.push(0);
            output.context = (api.create_context)(device, attributes.as_ptr());
            if output.context.is_null() {
                return Err("OpenAL could not create an output context".into());
            }
            let _current = CurrentContext::enter(api, output.context)?;
            for extension in [
                c"AL_EXT_FLOAT32",
                c"AL_EXT_source_distance_model",
                c"AL_EXT_LINEAR_DISTANCE",
            ] {
                if (api.extension)(extension.as_ptr()) == 0 {
                    return Err(format!(
                        "OpenAL output needs {}",
                        extension.to_string_lossy()
                    ));
                }
            }
            (api.enable)(AL_SOURCE_DISTANCE_MODEL_EXT);
            api.check("configuring distance model")?;
            let all = (api.device_extension)(device, c"ALC_ENUMERATE_ALL_EXT".as_ptr()) != 0;
            let actual = (api.device_string)(
                device,
                if all {
                    ALC_ALL_DEVICES_SPECIFIER
                } else {
                    ALC_DEVICE_SPECIFIER
                },
            );
            if !actual.is_null() {
                output.name = CStr::from_ptr(actual).to_string_lossy().into_owned();
            }
        }
        output.set_listener([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0])?;
        Ok(output)
    }

    pub fn device_name(&self) -> &str {
        &self.name
    }

    fn ensure_voice(&mut self, id: u64) -> Result<(), String> {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.voices.entry(id) {
            let mut source = 0;
            // SAFETY: the caller holds this output's current context.
            unsafe { (self.api.gen_sources)(1, &mut source) };
            if let Err(error) = self.api.check("creating source") {
                if source != 0 {
                    // SAFETY: the source belongs to this context.
                    unsafe { (self.api.delete_sources)(1, &source) };
                }
                return Err(error);
            }
            entry.insert(Voice {
                source,
                buffer: 0,
                stream: None,
                applied: None,
                started: false,
            });
        }
        Ok(())
    }

    /// Recycle consumed streaming buffers. Queueing and uploads stay on the audio thread.
    pub fn stream_space(&mut self, id: u64) -> Result<usize, String> {
        let _current = CurrentContext::enter(self.api, self.context)?;
        let Some(voice) = self.voices.get_mut(&id) else {
            return Ok(STREAM_BUFFERS);
        };
        let Some(stream) = &mut voice.stream else {
            return Ok(STREAM_BUFFERS);
        };
        let mut processed = 0;
        // SAFETY: this source and all queued buffers belong to this current context.
        unsafe {
            (self.api.source_state)(voice.source, AL_BUFFERS_PROCESSED, &mut processed);
            for _ in 0..processed {
                let mut buffer = 0;
                (self.api.unqueue_buffers)(voice.source, 1, &mut buffer);
                stream.free.push(buffer);
            }
        }
        self.api.check("recycling stream buffers")?;
        Ok(stream.free.len())
    }

    /// Queue one bounded PCM block. Returns false when the three-buffer queue is full.
    pub fn queue_stream(&mut self, id: u64, samples: &[f32], rate: u32) -> Result<bool, String> {
        let api = self.api;
        let _current = CurrentContext::enter(api, self.context)?;
        self.ensure_voice(id)?;
        self.stream_space(id)?;
        let voice = self.voices.get_mut(&id).expect("voice inserted");
        if voice.stream.is_none() {
            let mut buffers = [0; STREAM_BUFFERS];
            // SAFETY: the output array is valid and this context owns the new buffers.
            unsafe { (api.gen_buffers)(STREAM_BUFFERS as i32, buffers.as_mut_ptr()) };
            if let Err(error) = api.check("creating stream buffers") {
                for buffer in buffers.into_iter().filter(|b| *b != 0) {
                    // SAFETY: no source refers to these partially created buffers.
                    unsafe { (api.delete_buffers)(1, &buffer) };
                }
                return Err(error);
            }
            voice.stream = Some(StreamBuffers {
                free: buffers.to_vec(),
                buffers,
                finished: false,
            });
        }
        let stream = voice.stream.as_mut().expect("stream inserted");
        let Some(buffer) = stream.free.last().copied() else {
            return Ok(false);
        };
        if stream.free.len() == STREAM_BUFFERS {
            let mut state = 0;
            // SAFETY: the queue is empty. Rewind only resets a stopped source's
            // state; otherwise OpenAL would mark newly queued blocks processed.
            unsafe {
                (api.source_state)(voice.source, AL_SOURCE_STATE, &mut state);
                if state == AL_STOPPED {
                    (api.rewind)(voice.source);
                }
            }
        }
        let bytes = i32::try_from(std::mem::size_of_val(samples))
            .map_err(|_| "OpenAL stream block is too large")?;
        let rate = i32::try_from(rate).map_err(|_| "OpenAL sample rate is too large")?;
        // SAFETY: OpenAL copies this live slice before returning; this free buffer is not queued.
        unsafe {
            (api.buffer_data)(
                buffer,
                AL_FORMAT_MONO_FLOAT32,
                samples.as_ptr().cast(),
                bytes,
                rate,
            );
            (api.queue_buffers)(voice.source, 1, &buffer);
        }
        api.check("queueing stream block")?;
        stream.free.pop();
        Ok(true)
    }

    pub fn finish_stream(&mut self, id: u64) {
        if let Some(stream) = self.voices.get_mut(&id).and_then(|v| v.stream.as_mut()) {
            stream.finished = true;
        }
    }

    /// Returns true when the native source has reached the end of its sound.
    pub fn sync_voice(&mut self, id: u64, settings: VoiceSettings<'_>) -> Result<bool, String> {
        let api = self.api;
        let _current = CurrentContext::enter(api, self.context)?;
        self.ensure_voice(id)?;
        self.stream_space(id)?;
        let voice = self.voices.get_mut(&id).expect("voice inserted");
        // SAFETY: this context owns every source and buffer below.
        unsafe {
            if voice.buffer == 0
                && voice.stream.is_none()
                && let Some((samples, rate)) = settings.samples
                && !samples.is_empty()
            {
                let key = (Arc::as_ptr(samples) as usize, rate);
                let cached = self.buffers.entry(key).or_insert_with(|| CachedBuffer {
                    buffer: 0,
                    samples: Arc::downgrade(samples),
                });
                // A freed Arc's allocation can be reused at the same address.
                if cached.buffer != 0 && cached.samples.upgrade().is_none() {
                    (api.delete_buffers)(1, &cached.buffer);
                    cached.buffer = 0;
                    cached.samples = Arc::downgrade(samples);
                }
                if cached.buffer == 0 {
                    let bytes = i32::try_from(std::mem::size_of_val(samples.as_slice()))
                        .map_err(|_| "OpenAL sound buffer is too large")?;
                    let rate =
                        i32::try_from(rate).map_err(|_| "OpenAL sample rate is too large")?;
                    (api.gen_buffers)(1, &mut cached.buffer);
                    (api.buffer_data)(
                        cached.buffer,
                        AL_FORMAT_MONO_FLOAT32,
                        samples.as_ptr().cast(),
                        bytes,
                        rate,
                    );
                    api.check("uploading shared sound buffer")?;
                }
                (api.source_integer)(voice.source, AL_BUFFER, cached.buffer as i32);
                api.check("attaching shared sound buffer")?;
                voice.buffer = cached.buffer;
            }
            let parameters = Parameters {
                gain: settings.gain,
                pitch: settings.pitch,
                max_distance: settings.max_distance,
                looping: settings.looping && voice.stream.is_none(),
                position: settings.position,
                relative: settings.relative,
            };
            if voice.applied != Some(parameters) {
                (api.source_float)(voice.source, AL_GAIN, settings.gain);
                (api.source_float)(voice.source, AL_PITCH, settings.pitch);
                (api.source_position)(
                    voice.source,
                    AL_POSITION,
                    settings.position[0],
                    settings.position[1],
                    settings.position[2],
                );
                (api.source_integer)(
                    voice.source,
                    AL_SOURCE_RELATIVE,
                    i32::from(settings.relative),
                );
                (api.source_integer)(voice.source, AL_LOOPING, i32::from(parameters.looping));
                if let Some(max) = settings.max_distance {
                    (api.source_integer)(
                        voice.source,
                        AL_SOURCE_DISTANCE_MODEL,
                        AL_LINEAR_DISTANCE,
                    );
                    (api.source_float)(voice.source, AL_MAX_DISTANCE, max);
                    (api.source_float)(voice.source, AL_ROLLOFF_FACTOR, 1.0);
                    (api.source_float)(voice.source, AL_REFERENCE_DISTANCE, 0.0);
                } else {
                    (api.source_integer)(voice.source, AL_SOURCE_DISTANCE_MODEL, 0);
                }
                api.check("setting source parameters")?;
                voice.applied = Some(parameters);
            }
            let mut native_state = 0;
            (api.source_state)(voice.source, AL_SOURCE_STATE, &mut native_state);
            let ready = voice
                .stream
                .as_ref()
                .map_or(voice.buffer != 0, |s| s.free.len() < STREAM_BUFFERS);
            let finished = voice
                .stream
                .as_ref()
                .map_or(voice.started && native_state == AL_STOPPED, |s| {
                    s.finished && !ready
                });
            if settings.state == VoiceState::Playing && finished {
                return Ok(true);
            }
            match settings.state {
                VoiceState::Playing if ready && native_state != AL_PLAYING => {
                    (api.play)(voice.source);
                    voice.started = true;
                }
                VoiceState::Paused if native_state == AL_PLAYING => (api.pause)(voice.source),
                VoiceState::Stopped if native_state != AL_STOPPED => (api.stop)(voice.source),
                _ => {}
            }
            api.check("updating source playback")?;
        }
        Ok(false)
    }

    pub fn remove_voice(&mut self, id: u64) {
        let Some(voice) = self.voices.remove(&id) else {
            return;
        };
        if let Ok(_current) = CurrentContext::enter(self.api, self.context) {
            // SAFETY: deleting the source detaches its buffers before their deletion.
            unsafe {
                (self.api.stop)(voice.source);
                (self.api.delete_sources)(1, &voice.source);
                if let Some(stream) = voice.stream {
                    (self.api.delete_buffers)(STREAM_BUFFERS as i32, stream.buffers.as_ptr());
                }
            }
            self.prune_buffers();
        }
    }

    fn prune_buffers(&mut self) {
        self.buffers.retain(|_, cached| {
            let keep = cached.samples.strong_count() != 0
                || self.voices.values().any(|v| v.buffer == cached.buffer);
            if !keep && cached.buffer != 0 {
                // SAFETY: no source refers to this buffer and its decoded data was evicted.
                unsafe { (self.api.delete_buffers)(1, &cached.buffer) };
            }
            keep
        });
    }

    pub fn connected(&self) -> bool {
        // SAFETY: this output owns a live device. ALC device queries need no current context.
        unsafe {
            if (self.api.device_extension)(self.device, c"ALC_EXT_disconnect".as_ptr()) == 0 {
                return true;
            }
            let mut connected = 1;
            (self.api.device_integer)(self.device, ALC_CONNECTED, 1, &mut connected);
            connected != 0
        }
    }

    pub fn set_listener(
        &mut self,
        position: [f64; 3],
        forward: [f32; 3],
        up: [f32; 3],
    ) -> Result<(), String> {
        let _current = CurrentContext::enter(self.api, self.context)?;
        let orientation = [forward[0], forward[1], forward[2], up[0], up[1], up[2]];
        // SAFETY: the context is current and the orientation array is valid for this call.
        unsafe {
            (self.api.listener_position)(
                AL_POSITION,
                position[0] as f32,
                position[1] as f32,
                position[2] as f32,
            );
            (self.api.listener_orientation)(AL_ORIENTATION, orientation.as_ptr());
        }
        self.api.check("setting listener")
    }
}

impl Drop for NativeOutput {
    fn drop(&mut self) {
        for id in self.voices.keys().copied().collect::<Vec<_>>() {
            self.remove_voice(id);
        }
        // SAFETY: sources have been removed, so the remaining cached buffers are detached.
        if let Ok(_current) = CurrentContext::enter(self.api, self.context) {
            for cached in self.buffers.values() {
                // SAFETY: this current context owns these detached buffers.
                unsafe { (self.api.delete_buffers)(1, &cached.buffer) };
            }
        }
        // SAFETY: all native sources and buffers have been deleted.
        unsafe {
            if !self.context.is_null() {
                if let Ok(mut current) = CurrentContext::enter(self.api, self.context)
                    && current.previous == self.context
                {
                    current.previous = ptr::null_mut();
                }
                (self.api.destroy_context)(self.context);
            }
            if !self.device.is_null() {
                (self.api.close_device)(self.device);
            }
        }
    }
}

/// The system default endpoint, in the same naming scheme as `output_devices`.
pub fn default_output_device() -> Option<String> {
    let api = Api::load().ok()?;
    // SAFETY: copy the library-owned string immediately, without retaining its pointer.
    unsafe {
        let all = (api.device_extension)(ptr::null_mut(), c"ALC_ENUMERATE_ALL_EXT".as_ptr()) != 0;
        let name = (api.device_string)(
            ptr::null_mut(),
            if all {
                ALC_DEFAULT_ALL_DEVICES_SPECIFIER
            } else {
                0x1004
            },
        );
        (!name.is_null()).then(|| CStr::from_ptr(name).to_string_lossy().into_owned())
    }
}

pub fn output_devices() -> Result<Vec<String>, String> {
    let api = Api::load()?;
    // SAFETY: OpenAL returns a double-NUL-terminated list valid until the next
    // enumeration. Copy each entry before returning.
    unsafe {
        let all = (api.device_extension)(ptr::null_mut(), c"ALC_ENUMERATE_ALL_EXT".as_ptr()) != 0;
        if !all && (api.device_extension)(ptr::null_mut(), c"ALC_ENUMERATION_EXT".as_ptr()) == 0 {
            return Ok(Vec::new());
        }
        let parameter = if all {
            ALC_ALL_DEVICES_SPECIFIER
        } else {
            ALC_DEVICE_SPECIFIER
        };
        let mut cursor = (api.device_string)(ptr::null_mut(), parameter);
        let mut names = Vec::new();
        while !cursor.is_null() {
            let name = CStr::from_ptr(cursor);
            if name.is_empty() {
                break;
            }
            let owned = name.to_string_lossy().into_owned();
            if !names.contains(&owned) {
                names.push(owned);
            }
            cursor = cursor.add(name.to_bytes_with_nul().len());
        }
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicUsize};

    #[test]
    fn bundled_library_is_found_beside_the_game_and_outside_its_macos_bundle() {
        assert_eq!(
            library_dirs(Path::new("version/horizon-client")),
            [PathBuf::from("version/native")]
        );
        assert_eq!(
            library_dirs(Path::new(
                "version/Horizon.app/Contents/MacOS/horizon-client"
            )),
            [
                PathBuf::from("version/Horizon.app/Contents/MacOS/native"),
                PathBuf::from("version/native"),
            ]
        );
    }

    #[test]
    fn callback_renders_interleaved_float_samples_at_the_output_rate() {
        let state = Box::new(CallbackState {
            render: Mutex::new(Box::new(|data, rate| {
                assert_eq!(rate, 48_000);
                for frame in data.as_chunks_mut::<2>().0 {
                    frame.copy_from_slice(&[0.25, -0.5]);
                }
            })),
            sample_rate: 48_000,
            channels: 2,
            failed: AtomicBool::new(false),
        });
        let mut data = [1.0_f32; 8];
        let bytes = size_of_val(&data) as i32;
        // SAFETY: the boxed state and writable sample array outlive the call.
        let written = unsafe {
            render_callback(
                (&*state as *const CallbackState).cast_mut().cast(),
                data.as_mut_ptr().cast(),
                bytes,
            )
        };
        assert_eq!(written, bytes);
        assert_eq!(data, [0.25, -0.5, 0.25, -0.5, 0.25, -0.5, 0.25, -0.5]);
    }

    #[test]
    fn callback_contains_a_renderer_panic_and_clears_the_output() {
        let state = Box::new(CallbackState {
            render: Mutex::new(Box::new(|_, _| panic!("test renderer panic"))),
            sample_rate: 48_000,
            channels: 2,
            failed: AtomicBool::new(false),
        });
        let mut data = [1.0_f32; 4];
        // SAFETY: the pointers refer to live, correctly sized callback state.
        let written = unsafe {
            render_callback(
                (&*state as *const CallbackState).cast_mut().cast(),
                data.as_mut_ptr().cast(),
                size_of_val(&data) as i32,
            )
        };
        assert_eq!(written, size_of_val(&data) as i32);
        assert_eq!(data, [0.0; 4]);
        assert!(state.failed.load(Ordering::Relaxed));
    }

    #[test]
    fn quad_callback_keeps_the_actuator_channels_separate() {
        let state = Box::new(CallbackState {
            render: Mutex::new(Box::new(|data, _| {
                for frame in data.as_chunks_mut::<4>().0 {
                    frame.copy_from_slice(&[0.0, 0.0, 0.25, -0.5]);
                }
            })),
            sample_rate: 48_000,
            channels: 4,
            failed: AtomicBool::new(false),
        });
        let mut data = [1.0_f32; 8];
        // SAFETY: the boxed state and writable sample array outlive the call.
        let written = unsafe {
            render_callback(
                (&*state as *const CallbackState).cast_mut().cast(),
                data.as_mut_ptr().cast(),
                size_of_val(&data) as i32,
            )
        };
        assert_eq!(written, size_of_val(&data) as i32);
        assert_eq!(data, [0.0, 0.0, 0.25, -0.5, 0.0, 0.0, 0.25, -0.5]);
    }

    #[test]
    #[ignore = "requires OpenAL Soft; run with ALSOFT_DRIVERS=null"]
    fn native_quad_output_renders() {
        let count = Arc::new(AtomicUsize::new(0));
        let rendered = count.clone();
        let stream = OutputStream::open_quad("", move |data, _| {
            data.fill(0.0);
            rendered.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while count.load(Ordering::Relaxed) < 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "OpenAL did not render quad output"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        drop(stream);
    }

    #[test]
    #[ignore = "requires OpenAL Soft; run with ALSOFT_DRIVERS=null"]
    fn native_source_plays_and_reports_its_end() {
        let mut output = NativeOutput::open("").unwrap();
        output
            .set_listener([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0])
            .unwrap();
        let samples = Arc::new(vec![0.0_f32; 2_400]);
        let settings = || VoiceSettings {
            samples: Some((&samples, 48_000)),
            state: VoiceState::Playing,
            gain: 0.25,
            pitch: 1.0,
            max_distance: Some(16.0),
            looping: false,
            position: [1.0, 0.0, 0.0],
            relative: false,
        };
        assert!(!output.sync_voice(1, settings()).unwrap());
        // The values vanilla's Channel writes to AL are held on this source.
        let _current = CurrentContext::enter(output.api, output.context).unwrap();
        let source = output.voices[&1].source;
        let mut model = 0;
        let mut gain = 0.0;
        // SAFETY: the current context owns this source and the output pointers are valid.
        unsafe {
            let get_source = output
                .api
                ._library
                .get::<unsafe extern "C" fn(u32, i32, *mut f32)>(b"alGetSourcef\0")
                .unwrap();
            get_source(source, AL_GAIN, &mut gain);
            (output.api.source_state)(source, AL_SOURCE_DISTANCE_MODEL, &mut model);
        }
        assert_eq!(gain, 0.25);
        assert_eq!(model, AL_LINEAR_DISTANCE);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !output.sync_voice(1, settings()).unwrap() {
            assert!(
                std::time::Instant::now() < deadline,
                "OpenAL source did not stop"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        output.remove_voice(1);
    }

    fn playing(samples: Option<(&Arc<Vec<f32>>, u32)>) -> VoiceSettings<'_> {
        VoiceSettings {
            samples,
            state: VoiceState::Playing,
            gain: 1.0,
            pitch: 1.0,
            max_distance: None,
            looping: false,
            position: [0.0; 3],
            relative: true,
        }
    }

    #[test]
    #[ignore = "requires OpenAL Soft; run with ALSOFT_DRIVERS=null"]
    fn repeated_sounds_share_one_native_buffer_until_evicted() {
        let mut output = NativeOutput::open("").unwrap();
        let samples = Arc::new(vec![0.0; 4_800]);
        output
            .sync_voice(1, playing(Some((&samples, 48_000))))
            .unwrap();
        output
            .sync_voice(2, playing(Some((&samples, 48_000))))
            .unwrap();
        assert_eq!(output.buffers.len(), 1);
        assert_eq!(output.voices[&1].buffer, output.voices[&2].buffer);
        output.remove_voice(1);
        assert_eq!(output.buffers.len(), 1);
        drop(samples);
        output.remove_voice(2);
        assert!(output.buffers.is_empty());
    }

    #[test]
    #[ignore = "requires OpenAL Soft; run with ALSOFT_DRIVERS=null"]
    fn streaming_recycles_a_bounded_queue_and_finishes_once() {
        let mut output = NativeOutput::open("").unwrap();
        let block = vec![0.0; 2_400];
        for _ in 0..STREAM_BUFFERS {
            assert!(output.queue_stream(1, &block, 48_000).unwrap());
        }
        assert!(!output.queue_stream(1, &block, 48_000).unwrap());
        output.finish_stream(1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !output.sync_voice(1, playing(None)).unwrap() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(output.stream_space(1).unwrap(), STREAM_BUFFERS);
        assert!(
            output.sync_voice(1, playing(None)).unwrap(),
            "finished streams never restart"
        );
        assert!(
            output.buffers.is_empty(),
            "streamed PCM never enters the static cache"
        );
        output.remove_voice(1);
    }

    #[test]
    #[ignore = "requires OpenAL Soft; run with ALSOFT_DRIVERS=null"]
    fn streaming_recovers_from_an_underrun_without_replaying_consumed_blocks() {
        let mut output = NativeOutput::open("").unwrap();
        let block = vec![0.0; 480];
        output.queue_stream(1, &block, 48_000).unwrap();
        output.sync_voice(1, playing(None)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while output.stream_space(1).unwrap() != STREAM_BUFFERS {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            !output.sync_voice(1, playing(None)).unwrap(),
            "decoder has not reached EOF"
        );
        output.queue_stream(1, &block, 48_000).unwrap();
        output.finish_stream(1);
        assert!(!output.sync_voice(1, playing(None)).unwrap());
        while !output.sync_voice(1, playing(None)).unwrap() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    #[ignore = "requires OpenAL Soft; run with ALSOFT_DRIVERS=null"]
    fn native_outputs_render_and_survive_replacing_the_previous_context() {
        let first_count = Arc::new(AtomicUsize::new(0));
        let count = first_count.clone();
        let first = OutputStream::open("", move |data, _| {
            data.fill(0.0);
            count.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
        assert!(first.sample_rate() > 0);
        let second_count = Arc::new(AtomicUsize::new(0));
        let count = second_count.clone();
        let second = OutputStream::open("", move |data, _| {
            data.fill(0.0);
            count.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
        drop(first);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while second_count.load(Ordering::Relaxed) < 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "OpenAL did not render"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!second.device_name().is_empty());
        drop(second);
    }

    #[test]
    #[ignore = "requires OpenAL Soft; run with ALSOFT_DRIVERS=null"]
    fn native_streams_can_be_created_on_independent_threads() {
        let ready = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    let count = Arc::new(AtomicUsize::new(0));
                    let rendered = count.clone();
                    let stream = OutputStream::open("", move |data, _| {
                        data.fill(0.0);
                        rendered.fetch_add(1, Ordering::Relaxed);
                    })
                    .unwrap();
                    ready.fetch_add(1, Ordering::Relaxed);
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                    while ready.load(Ordering::Relaxed) < 2 || count.load(Ordering::Relaxed) < 2 {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "OpenAL did not render"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    drop(stream);
                });
            }
        });
    }
}
