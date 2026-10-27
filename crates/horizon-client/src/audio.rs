//! Sound playback: vanilla's `SoundEngine` bookkeeping (`sound_engine`) over a
//! small mixer standing in for OpenAL: one channel per source with its gain,
//! pitch, linear distance attenuation (`AL_LINEAR_DISTANCE`, reference 0,
//! rolloff 1), looping, relative flag and source state, Ogg/Vorbis decoding
//! (streams on a worker thread) and OpenAL Soft's stereo panning. With sound
//! physics on (`sound_physics`, a Horizon extra), each positional channel
//! also gets EFX-style filters and reverb sends (`reverb`), worked out off
//! this thread (`acoustics`). Binaural panning and the Doppler effect
//! (`spatial`) are extras of their own.
use crate::Result;
use crate::acoustics::{Acoustics, Sink};
use crate::reverb::{self, Bus, Crossover, Split};
use crate::sound_engine::{
    Attenuation, Backend, Bound, ChannelId, InstanceId, PlayResult, SoundEngine, SoundInstance,
    Sounds, TickContext,
};
use crate::sound_events::{Sound, SoundRegistry};
use crate::sound_physics::{Environment, Request, Settings as PhysicsSettings};
use crate::spatial::{self, Binaural, Ear, Motion};
#[cfg(feature = "audio")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use horizon_core::random::{LegacyRandom, RandomSource};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

/// An OpenAL source's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Initial,
    Playing,
    Paused,
    Stopped,
}
/// An OpenAL source.
struct Channel {
    samples: Option<(Arc<Vec<f32>>, u32)>,
    position: f64,
    state: State,
    gain: f32,
    pitch: f32,
    /// `AL_LINEAR_DISTANCE` max distance, or no attenuation.
    linear: Option<f32>,
    looping: bool,
    at: [f32; 3],
    relative: bool,
    /// Sound physics' result for this channel, once worked out.
    environment: Option<Environment>,
    /// The filters' state.
    split: Split,
    /// Waiting for its environment: plays from this instant at the latest.
    hold: Option<Instant>,
    /// The sound event and `SoundSource` it plays, for re-evaluation.
    tag: Option<(Arc<str>, usize)>,
    /// The tick it was last traced at.
    traced: u64,
    /// Its velocity, for the Doppler effect.
    motion: Motion,
    /// Its ears' state, for binaural panning.
    binaural: Option<Box<Binaural>>,
    /// How it was mixed in the last buffer: the next one ramps from there.
    last: Option<Voice>,
}
impl Channel {
    fn new() -> Self {
        Channel {
            samples: None,
            position: 0.0,
            state: State::Initial,
            gain: 1.0,
            pitch: 1.0,
            linear: None,
            looping: false,
            at: [0.0; 3],
            relative: false,
            environment: None,
            split: Split::default(),
            hold: None,
            tag: None,
            traced: 0,
            motion: Motion::default(),
            binaural: None,
            last: None,
        }
    }
    /// Seconds of sound left (looping sounds never end).
    fn remaining(&self) -> f32 {
        match &self.samples {
            _ if self.looping => f32::INFINITY,
            Some((s, rate)) => {
                ((s.len() as f64 - self.position)
                    / f64::from(*rate)
                    / f64::from(self.pitch.max(0.01))) as f32
            }
            None => f32::INFINITY,
        }
    }
}
/// Horizon's spatial extras the mixer applies (`spatial`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Spatial {
    hrtf: bool,
    doppler: Option<f32>,
    whoosh: bool,
}
/// How a voice is mixed for one buffer: its pan gains, its gain on a mono
/// output, its filters and sends, and its ears when binaural.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Voice {
    pan: [f32; 2],
    mono: f32,
    wet: Option<Wet>,
    ears: Option<[Ear; 2]>,
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
impl Voice {
    /// The gains `t` of the way from `self` to `to` (the ears ramp in
    /// `Binaural::process`).
    fn toward(&self, to: &Voice, t: f32) -> Voice {
        let wet = match (self.wet, to.wet) {
            (Some(a), Some(b)) => Some(Wet {
                direct_gain: lerp(a.direct_gain, b.direct_gain, t),
                direct_hf: lerp(a.direct_hf, b.direct_hf, t),
                sends: std::array::from_fn(|k| {
                    (
                        lerp(a.sends[k].0, b.sends[k].0, t),
                        lerp(a.sends[k].1, b.sends[k].1, t),
                    )
                }),
            }),
            (_, wet) => wet,
        };
        Voice {
            pan: [
                lerp(self.pan[0], to.pan[0], t),
                lerp(self.pan[1], to.pan[1], t),
            ],
            mono: lerp(self.mono, to.mono, t),
            wet,
            ears: to.ears,
        }
    }
}
#[derive(Default)]
struct Mixer {
    channels: HashMap<ChannelId, Channel>,
    listener: [f64; 3],
    listener_yaw: f32,
    listener_pitch: f32,
    limiter_peak: f32,
    /// Sound physics' settings while it is on.
    physics_settings: Option<PhysicsSettings>,
    /// The reverb buses and their buffers, built at the output's rate.
    physics: Option<Box<MixPhysics>>,
    spatial: Spatial,
    /// The listener's velocity, for the Doppler effect.
    listener_motion: Motion,
}
/// The mixer's sound physics state.
struct MixPhysics {
    settings: PhysicsSettings,
    rate: u32,
    crossover: Crossover,
    buses: Vec<Bus>,
    /// Each bus's input for the buffer being mixed, one sample a frame.
    sends: [Vec<f32>; reverb::SENDS],
}
impl MixPhysics {
    fn new(settings: &PhysicsSettings, rate: u32) -> Self {
        Self {
            settings: settings.clone(),
            rate,
            crossover: Split::coefficient(rate),
            buses: reverb::presets(settings)
                .into_iter()
                .map(|p| Bus::new(p, rate))
                .collect(),
            sends: Default::default(),
        }
    }
}
/// Where the sound physics thread delivers: the channel's environment, which
/// also ends its hold.
struct MixerSink(Arc<Mutex<Mixer>>);
impl Sink for MixerSink {
    fn environment(&self, channel: ChannelId, environment: Environment) {
        if let Ok(mut m) = self.0.lock()
            && let Some(c) = m.channels.get_mut(&channel)
        {
            c.environment = Some(environment);
            c.hold = None;
        }
    }
}
/// A sound the player could have heard (`SoundEventListener.onPlaySound`):
/// what the subtitle overlay needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    /// The `subtitle` translation key of the sound event.
    pub subtitle: String,
    /// The instance's position (relative sounds report their raw coordinates).
    pub at: Option<[f64; 3]>,
    /// `SoundEngine.getRange`: infinite for relative and non-attenuated sounds.
    pub range: f32,
    pub looping: bool,
}
/// The mixer side of the engine (`Backend`).
pub struct MixerBackend {
    mixer: Arc<Mutex<Mixer>>,
    root: PathBuf,
    cache: Arc<Mutex<SoundCache>>,
    /// The decoder thread for static sounds, started with the first one.
    decoder: Option<std::sync::mpsc::Sender<Decode>>,
    next: ChannelId,
    live: bool,
    /// Sound physics: off unless the extra is on.
    acoustics: Acoustics,
    /// The camera is underwater (`isUnderWater`), for sound physics.
    listener_underwater: bool,
    /// Client ticks, for re-evaluating moving sounds.
    ticks: u64,
    /// The block the listener was in at the last tick.
    listener_cell: Option<[i32; 3]>,
    /// The whooshing arrows' channels, by the arrow's key.
    whooshes: HashMap<u32, ChannelId>,
    whoosh_samples: Option<Arc<Vec<f32>>>,
}
/// Decoded samples and their rate (`None`: the file did not load).
type Samples = Option<(Arc<Vec<f32>>, u32)>;
/// Decoded static sounds (`SoundBufferLibrary`'s buffers), dropped least
/// recently used first past `CACHE_BYTES`: kept forever, every sound heard in
/// a session stayed (up to 1.4 GiB of samples, villages playing hundreds).
#[derive(Default)]
struct SoundCache {
    entries: HashMap<String, (Samples, u64)>,
    bytes: usize,
    clock: u64,
}
const CACHE_BYTES: usize = 64 << 20;
impl SoundCache {
    fn get(&mut self, key: &str) -> Option<Samples> {
        self.clock += 1;
        let entry = self.entries.get_mut(key)?;
        entry.1 = self.clock;
        Some(entry.0.clone())
    }
    fn insert(&mut self, key: String, samples: Samples) {
        self.clock += 1;
        self.bytes += Self::size(&samples);
        if let Some((old, _)) = self.entries.insert(key, (samples, self.clock)) {
            self.bytes -= Self::size(&old);
        }
        while self.bytes > CACHE_BYTES && self.entries.len() > 1 {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((gone, _)) = self.entries.remove(&oldest) {
                self.bytes -= Self::size(&gone);
            }
        }
    }
    fn size(samples: &Samples) -> usize {
        samples
            .as_ref()
            .map_or(0, |(s, _)| s.len() * std::mem::size_of::<f32>())
    }
}
/// A static sound to decode for a channel waiting on it.
struct Decode {
    key: String,
    file: PathBuf,
    channel: ChannelId,
}
/// Give a channel waiting on its sound the decoded samples and start it, as
/// the buffer future attaching and playing the source. A source stopped or
/// released meanwhile stays so.
fn attach(mixer: &Mutex<Mixer>, channel: ChannelId, samples: Samples, looping: bool) {
    if let Some(samples) = samples
        && let Ok(mut m) = mixer.lock()
        && let Some(c) = m.channels.get_mut(&channel)
        && c.state == State::Initial
    {
        c.samples = Some(samples);
        c.position = 0.0;
        c.state = State::Playing;
        c.looping |= looping;
    }
}
/// The open output stream. A build without the `audio` feature (the server build, which must not
/// need libasound) never has one.
#[cfg(feature = "audio")]
type Stream = cpal::Stream;
#[cfg(not(feature = "audio"))]
type Stream = std::convert::Infallible;
pub struct Audio {
    _stream: Option<Stream>,
    pub engine: SoundEngine,
    backend: MixerBackend,
    /// Record `Heard` sounds (the subtitle overlay is listening).
    pub record_heard: bool,
    pub listener: [f64; 3],
    pub listener_yaw: f32,
    pub master: f32,
    /// Seeds of positional plays (the client level random's `nextLong`).
    random: LegacyRandom,
    /// A `SoundSource` index the next sound plays on (`/playsound`).
    source_override: Option<usize>,
    /// Positional sounds played since the last drain, kept while the world
    /// is open to the network so guests hear them too.
    pub capture: Option<Vec<(String, [f64; 3], f32, f32)>>,
    /// How many times each positional sound played (the control socket's `status`),
    /// with or without an audio device.
    pub heard: std::collections::BTreeMap<String, u32>,
    /// How many times each selection was stopped ("record *", "* minecraft:…", `*` for
    /// any), with or without an audio device, as `heard`.
    pub stopped: std::collections::BTreeMap<String, u32>,
}
fn decode(path: &Path) -> Option<(Vec<f32>, u32)> {
    use symphonia::core::{
        audio::SampleBuffer, codecs::DecoderOptions, formats::FormatOptions, io::MediaSourceStream,
        meta::MetadataOptions, probe::Hint,
    };
    let file = fs::File::open(path).ok()?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("ogg");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let mut format = probed.format;
    let track = format.default_track()?;
    let id = track.id;
    let rate = track.codec_params.sample_rate?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .ok()?;
    let mut mono = Vec::new();
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != id {
            continue;
        }
        let Ok(buffer) = decoder.decode(&packet) else {
            continue;
        };
        let spec = *buffer.spec();
        let channels = spec.channels.count().max(1);
        let mut samples = SampleBuffer::<f32>::new(buffer.capacity() as u64, spec);
        samples.copy_interleaved_ref(buffer);
        for frame in samples.samples().chunks(channels) {
            mono.push(frame.iter().sum::<f32>() / channels as f32);
        }
    }
    Some((mono, rate))
}
impl MixerBackend {
    /// Traces playing sounds again: every `sound_update_interval` ticks with
    /// `update_moving_sounds` (the mod's `tickInGameSound` hook; music discs
    /// only then get sound physics), and with `update_on_listener_move` the
    /// ones still playing for a while when the listener enters another
    /// block, at most `MOVE_BATCH` a tick.
    fn reevaluate(&mut self) {
        const MOVE_BATCH: usize = 24;
        let settings = self.acoustics.settings();
        if !settings.enabled || !self.live {
            return;
        }
        let (moving, on_move) = (
            settings.update_moving_sounds,
            settings.update_on_listener_move,
        );
        let interval = u64::from(settings.sound_update_interval.max(1));
        let Ok(mut m) = self.mixer.lock() else {
            return;
        };
        let cell = m.listener.map(|v| v.floor() as i32);
        let moved = on_move && self.listener_cell.is_some_and(|c| c != cell);
        self.listener_cell = Some(cell);
        if !moving && !moved {
            return;
        }
        let ticks = self.ticks;
        let mut moved_left = MOVE_BATCH;
        let due: Vec<ChannelId> = m
            .channels
            .iter()
            .filter(|(_, c)| c.state == State::Playing && c.hold.is_none() && c.tag.is_some())
            .filter(|(id, c)| {
                if moving && (ticks + **id).is_multiple_of(interval) {
                    return true;
                }
                if moved && moved_left > 0 && ticks >= c.traced + 2 && c.remaining() > 0.4 {
                    moved_left -= 1;
                    return true;
                }
                false
            })
            .map(|(id, _)| *id)
            .collect();
        for id in &due {
            if let Some(c) = m.channels.get_mut(id) {
                c.traced = ticks;
            }
        }
        let due: Vec<(ChannelId, Request, f32)> = due
            .into_iter()
            .filter_map(|id| Some((id, m.channels.get(&id)?)))
            .filter_map(|(id, c)| {
                let (event, source) = c.tag.as_ref()?;
                Some((
                    id,
                    Request {
                        at: c.at.map(f64::from),
                        listener: m.listener,
                        listener_underwater: self.listener_underwater,
                        source: *source,
                        event: event.to_string(),
                    },
                    c.linear?,
                ))
            })
            .collect();
        drop(m);
        for (channel, request, max_distance) in due {
            self.acoustics.evaluate(channel, request, max_distance);
        }
    }
    fn with(&self, channel: ChannelId, f: impl FnOnce(&mut Channel)) {
        if let Ok(mut m) = self.mixer.lock()
            && let Some(c) = m.channels.get_mut(&channel)
        {
            f(c);
        }
    }
}
impl Backend for MixerBackend {
    fn create(&mut self, _streaming: bool) -> Option<ChannelId> {
        self.next += 1;
        let id = self.next;
        self.mixer.lock().ok()?.channels.insert(id, Channel::new());
        Some(id)
    }
    fn destroy(&mut self, channel: ChannelId) {
        if let Ok(mut m) = self.mixer.lock() {
            m.channels.remove(&channel);
        }
    }
    fn set_pitch(&mut self, channel: ChannelId, pitch: f32) {
        self.with(channel, |c| c.pitch = pitch);
    }
    fn set_volume(&mut self, channel: ChannelId, volume: f32) {
        self.with(channel, |c| c.gain = volume);
    }
    fn set_attenuation(&mut self, channel: ChannelId, linear: Option<f32>) {
        self.with(channel, |c| c.linear = linear);
    }
    fn set_looping(&mut self, channel: ChannelId, looping: bool) {
        self.with(channel, |c| c.looping = looping);
    }
    fn set_position(&mut self, channel: ChannelId, p: [f64; 3]) {
        let now = Instant::now();
        self.with(channel, |c| {
            c.motion.update(p, now);
            let at = [p[0] as f32, p[1] as f32, p[2] as f32];
            // A sound that moved is heard from where it is until re-evaluated.
            if c.at != at
                && let Some(e) = &mut c.environment
            {
                e.redirect = None;
            }
            c.at = at;
        });
    }
    fn set_relative(&mut self, channel: ChannelId, relative: bool) {
        self.with(channel, |c| c.relative = relative);
    }
    /// Sound physics for a positional sound starting: hold it until the
    /// thread has its environment (`HOLD` at most), as the mod works it out
    /// before `alSourcePlay`.
    fn starting(&mut self, channel: ChannelId, event: &str, source: usize) {
        if !self.acoustics.enabled() || !self.live {
            return;
        }
        let underwater = self.listener_underwater;
        let Ok(mut m) = self.mixer.lock() else {
            return;
        };
        let listener = m.listener;
        let Some(c) = m.channels.get_mut(&channel) else {
            return;
        };
        let Some(max_distance) = c.linear.filter(|_| !c.relative) else {
            return;
        };
        c.tag = Some((event.into(), source));
        c.traced = self.ticks;
        c.hold = Some(Instant::now() + crate::acoustics::HOLD);
        let request = Request {
            at: c.at.map(f64::from),
            listener,
            listener_underwater: underwater,
            source,
            event: event.to_owned(),
        };
        if !self.acoustics.evaluate(channel, request, max_distance) {
            c.hold = None;
        }
    }
    /// `getCompleteBuffer` (cached, else decoded on the decoder thread) or
    /// `getStream` (decoded on a thread of its own), then attach and
    /// `alSourcePlay`. A sound that cannot load leaves its source initial,
    /// as a failed buffer future does in vanilla.
    fn start(&mut self, channel: ChannelId, sound: &Sound, looping: bool) {
        if !self.live {
            return;
        }
        let (namespace, path) = sound.path();
        let file = self.root.join("assets").join(namespace).join(&path);
        if sound.stream {
            // A streamed sound loops by restarting its stream.
            let mixer = self.mixer.clone();
            std::thread::spawn(move || {
                let samples = decode(&file).map(|(s, r)| (Arc::new(s), r));
                attach(&mixer, channel, samples, looping);
            });
            return;
        }
        let key = format!("{namespace}:{path}");
        let cached = self.cache.lock().ok().and_then(|mut c| c.get(&key));
        if let Some(samples) = cached {
            attach(&self.mixer, channel, samples, false);
            return;
        }
        let decoder = self.decoder.get_or_insert_with(|| {
            let (send, receive) = std::sync::mpsc::channel::<Decode>();
            let (mixer, cache) = (self.mixer.clone(), self.cache.clone());
            std::thread::Builder::new()
                .name("sound decoder".into())
                .spawn(move || {
                    for job in receive {
                        // Queued twice before the first decode finished.
                        let cached = cache.lock().ok().and_then(|mut c| c.get(&job.key));
                        let samples = cached.unwrap_or_else(|| {
                            let samples = decode(&job.file).map(|(s, r)| (Arc::new(s), r));
                            if let Ok(mut c) = cache.lock() {
                                c.insert(job.key, samples.clone());
                            }
                            samples
                        });
                        attach(&mixer, job.channel, samples, false);
                    }
                })
                .expect("sound decoder thread");
            send
        });
        let _ = decoder.send(Decode { key, file, channel });
    }
    /// `alSourceStop`: any state becomes stopped, an initial source too (a
    /// sound still decoding then never starts).
    fn stop(&mut self, channel: ChannelId) {
        self.with(channel, |c| c.state = State::Stopped);
    }
    fn pause(&mut self, channel: ChannelId) {
        self.with(channel, |c| {
            if c.state == State::Playing {
                c.state = State::Paused;
            }
        });
    }
    fn unpause(&mut self, channel: ChannelId) {
        self.with(channel, |c| {
            if c.state == State::Paused {
                c.state = State::Playing;
            }
        });
    }
    fn stopped(&mut self, channel: ChannelId) -> bool {
        self.mixer
            .lock()
            .map(|m| {
                m.channels
                    .get(&channel)
                    .is_none_or(|c| c.state == State::Stopped)
            })
            .unwrap_or(true)
    }
}
/// OpenAL Soft's stereo gains `[left, right]` for a mono source in a
/// direction given in listener space (right, up, forward; any length).
/// Vanilla's context (`Library.createAttributes`: HRTF off) on a stereo
/// output renders pairwise: front azimuths are stretched by 1.5
/// (`ScaleAzimuthFront`, so 60 degrees off-centre pans fully) and the
/// direction is decoded by the first-order stereo decoder, 0.5 W, 0.5 Y and
/// 0.0957 X in this scaling. A centred source gets 0.596 on each side, one
/// behind 0.404, one above or below 0.5, one at the source's own position
/// is panned to the front. Matches OpenAL Soft 1.25.2 (LWJGL 3.4.3, the
/// pinned client's) rendered through `ALC_SOFT_loopback` within 1e-5.
fn stereo_gains(right: f32, up: f32, forward: f32) -> [f32; 2] {
    let length = (right * right + up * up + forward * forward).sqrt();
    if length <= f32::EPSILON {
        return stereo_gains(0.0, 0.0, 1.0);
    }
    let elevation = (up / length).clamp(-1.0, 1.0).asin();
    let mut azimuth = right.atan2(forward);
    if azimuth.abs() < std::f32::consts::FRAC_PI_2 {
        azimuth = (azimuth.abs() * 1.5)
            .min(std::f32::consts::FRAC_PI_2)
            .copysign(azimuth);
    }
    let side = azimuth.sin() * elevation.cos();
    let front = azimuth.cos() * elevation.cos() * 0.095_660_8;
    [0.5 - 0.5 * side + front, 0.5 + 0.5 * side + front]
}
/// A channel's filters and sends for one buffer: the direct path's EFX
/// low-pass, then each send's (gain, high-frequency gain), with distance and
/// air absorption applied.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Wet {
    direct_gain: f32,
    direct_hf: f32,
    sends: [(f32, f32); reverb::SENDS],
}
/// The gain OpenAL applies for distance (`AL_LINEAR_DISTANCE` with reference
/// distance 0 and rolloff 1: the distance clamps to the max distance).
fn distance_gain(linear: Option<f32>, distance: f32) -> f32 {
    match linear {
        None => 1.0,
        Some(max) if max > 0.0 => (1.0 - distance.min(max) / max).clamp(0.0, 1.0),
        Some(_) => 0.0,
    }
}
// Only the output stream mixes; a build without one (no `audio` feature) never does.
#[cfg_attr(not(feature = "audio"), allow(dead_code))]
fn mix(mixer: &mut Mixer, data: &mut [f32], channels: usize, rate: u32) {
    data.fill(0.0);
    let listener = mixer.listener;
    // The camera's forward and up vectors (`Camera.forwardVector`/`upVector`)
    // and OpenAL's right = forward x up.
    let (yaw, pitch) = (
        f64::from(mixer.listener_yaw.to_radians()),
        f64::from(mixer.listener_pitch.to_radians()),
    );
    let forward = [
        -yaw.sin() * pitch.cos(),
        -pitch.sin(),
        yaw.cos() * pitch.cos(),
    ];
    let up = [
        -yaw.sin() * pitch.sin(),
        pitch.cos(),
        yaw.cos() * pitch.sin(),
    ];
    let right = [
        forward[1] * up[2] - forward[2] * up[1],
        forward[2] * up[0] - forward[0] * up[2],
        forward[0] * up[1] - forward[1] * up[0],
    ];
    let dot = |a: [f64; 3], b: [f64; 3]| (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) as f32;
    // Sound physics: built at this rate while on, dropped when off.
    match (&mixer.physics_settings, &mut mixer.physics) {
        (None, physics) => *physics = None,
        (Some(s), Some(p)) if p.rate == rate => {
            if p.settings != *s {
                for (bus, params) in p.buses.iter_mut().zip(reverb::presets(s)) {
                    bus.set_params(params);
                }
                p.settings = s.clone();
            }
        }
        (Some(s), physics) => *physics = Some(Box::new(MixPhysics::new(s, rate))),
    }
    let mut physics = mixer.physics.take();
    if let Some(p) = &mut physics {
        let frames = data.len() / channels.max(1);
        for send in &mut p.sends {
            send.clear();
            send.resize(frames, 0.0);
        }
    }
    let now = Instant::now();
    let spatial = mixer.spatial;
    // Gains ramp over the buffer while anything of Horizon's can change them
    // between buffers; vanilla's mix jumps, as OpenAL's gains do.
    let ramp = physics.is_some() || spatial != Spatial::default();
    let listener_velocity = mixer.listener_motion.velocity(now);
    // How each source is mixed in this buffer, and its step.
    let mut voices: Vec<(ChannelId, Voice, f64)> = Vec::new();
    for (id, c) in &mixer.channels {
        let Some((_, source_rate)) = &c.samples else {
            continue;
        };
        if c.state != State::Playing || c.hold.is_some_and(|until| now < until) {
            continue;
        }
        let environment = physics.as_ref().and(c.environment.as_ref());
        let at = environment
            .and_then(|e| e.redirect)
            .unwrap_or(c.at.map(f64::from));
        let d = if c.relative {
            [f64::from(c.at[0]), f64::from(c.at[1]), f64::from(c.at[2])]
        } else {
            [
                at[0] - listener[0],
                at[1] - listener[1],
                at[2] - listener[2],
            ]
        };
        let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32;
        // The mod's attenuation factor stretches the linear range.
        let linear = match (&physics, c.linear) {
            (Some(p), Some(max)) => Some(max / p.settings.attenuation_factor),
            (_, linear) => linear,
        };
        let gain = c.gain.clamp(0.0, 1.0) * distance_gain(linear, distance);
        let wet = match (environment, &physics) {
            (Some(e), Some(p)) if !c.relative => {
                let air = reverb::AIR_ABSORPTION_HF.powf(p.settings.air_absorption * distance);
                Some(Wet {
                    direct_gain: e.direct_gain,
                    direct_hf: e.direct_cutoff * air,
                    sends: std::array::from_fn(|k| {
                        let room = reverb::room_attenuation(
                            distance,
                            linear,
                            p.buses[k].params().room_rolloff,
                        );
                        (
                            c.gain.clamp(0.0, 1.0) * room * e.send_gain[k],
                            e.send_cutoff[k] * air,
                        )
                    }),
                })
            }
            _ => None,
        };
        // Relative sources are already in OpenAL's listener space (-Z ahead).
        let local = if c.relative {
            [d[0] as f32, d[1] as f32, -d[2] as f32]
        } else {
            [dot(d, right), dot(d, up), dot(d, forward)]
        };
        // Binaural for positional sources: their ears replace the panning.
        let ears = (spatial.hrtf && !c.relative && channels >= 2)
            .then(|| spatial::ears(local[0], local[1], local[2], rate));
        let pan = if ears.is_some() {
            [gain * spatial::BINAURAL_GAIN; 2]
        } else {
            stereo_gains(local[0], local[1], local[2]).map(|pan| gain * pan)
        };
        let mut pitch = c.pitch.max(0.0);
        if let Some(factor) = spatial.doppler.filter(|_| !c.relative) {
            let real = c.at.map(f64::from);
            let to_source = [
                (real[0] - listener[0]) as f32,
                (real[1] - listener[1]) as f32,
                (real[2] - listener[2]) as f32,
            ];
            pitch *= spatial::doppler(to_source, c.motion.velocity(now), listener_velocity, factor);
        }
        let step = f64::from(*source_rate) / f64::from(rate) * f64::from(pitch);
        let voice = Voice {
            pan,
            mono: gain,
            wet,
            ears,
        };
        voices.push((*id, voice, step));
    }
    let frames = (data.len() / channels.max(1)).max(1) as f32;
    for (id, voice, step) in voices {
        let Some(c) = mixer.channels.get_mut(&id) else {
            continue;
        };
        let Some((samples, _)) = c.samples.clone() else {
            continue;
        };
        let from = if ramp {
            c.last
                .filter(|l| l.ears.is_some() == voice.ears.is_some())
                .unwrap_or(voice)
        } else {
            voice
        };
        c.last = ramp.then_some(voice);
        let still = from == voice;
        let (from_ears, to_ears) = (from.ears, voice.ears);
        if to_ears.is_some() && c.binaural.is_none() {
            c.binaural = Some(Box::default());
        }
        for (f, frame) in data.chunks_mut(channels).enumerate() {
            let t = (f + 1) as f32 / frames;
            let v = if still { voice } else { from.toward(&voice, t) };
            let (left, right, gain, wet) = (v.pan[0], v.pan[1], v.mono, v.wet);
            let mut i = c.position as usize;
            if i + 1 >= samples.len() {
                if c.looping && samples.len() > 1 {
                    c.position = 0.0;
                    i = 0;
                } else {
                    c.state = State::Stopped;
                    break;
                }
            }
            let t = (c.position - i as f64) as f32;
            let mut s = samples[i] * (1.0 - t) + samples[i + 1] * t;
            if let (Some(w), Some(p)) = (&wet, &mut physics) {
                let parts = c.split.split(s, p.crossover);
                for (send, &(g, hf)) in p.sends.iter_mut().zip(&w.sends) {
                    if g > 0.0 {
                        send[f] += reverb::lowpass(parts, g, hf);
                    }
                }
                s = reverb::lowpass(parts, w.direct_gain, w.direct_hf);
            }
            if let (Some(to), Some(b)) = (&to_ears, &mut c.binaural) {
                let [l, r] = b.process(s, from_ears.as_ref().unwrap_or(to), to, t, rate);
                frame[0] += l * left;
                frame[1] += r * right;
            } else if channels >= 2 {
                frame[0] += s * left;
                frame[1] += s * right;
            } else {
                // A mono output has no panning: the source's own gain.
                frame[0] += s * gain;
            }
            c.position += step;
        }
    }
    if let Some(p) = &mut physics {
        let MixPhysics { buses, sends, .. } = &mut **p;
        for (bus, send) in buses.iter_mut().zip(sends.iter()) {
            bus.process(send, data, channels);
        }
    }
    mixer.physics = physics;
    // Limit the whole buffer together, preserving waveform and stereo balance
    // instead of flattening each overloaded sample; release over 200 ms
    // (OpenAL Soft's output limiter, `ALC_OUTPUT_LIMITER_SOFT`, releases
    // over 0.2 s).
    let peak = data.iter().fold(1.0f32, |peak, s| peak.max(s.abs()));
    mixer.limiter_peak = mixer.limiter_peak.max(peak);
    let release = (-1.0 / (0.2 * rate as f32)).exp();
    for frame in data.chunks_mut(channels) {
        let gain = mixer.limiter_peak.recip();
        for s in frame {
            *s *= gain;
        }
        mixer.limiter_peak = (mixer.limiter_peak * release).max(peak);
    }
}
/// The mixer's cost, for `examples/sound_physics_bench.rs`: `voices` looping
/// positional channels mixed to 48 kHz stereo in 10 ms buffers for
/// `seconds`, each with a reverb environment when `physics` is on, binaural
/// with `hrtf`.
#[doc(hidden)]
pub fn mix_benchmark(
    voices: usize,
    physics: bool,
    hrtf: bool,
    seconds: f32,
) -> std::time::Duration {
    let mut mixer = Mixer::default();
    let mut random = LegacyRandom::new(1);
    let noise: Arc<Vec<f32>> = Arc::new((0..48_000).map(|_| random.next_float() - 0.5).collect());
    for id in 0..voices as ChannelId {
        let angle = id as f32;
        mixer.channels.insert(
            id,
            Channel {
                samples: Some((noise.clone(), 44_100)),
                position: 0.0,
                state: State::Playing,
                gain: 1.0,
                pitch: 1.0,
                linear: Some(16.0),
                looping: true,
                at: [angle.cos() * 6.0, 0.0, angle.sin() * 6.0],
                relative: false,
                environment: physics.then_some(Environment {
                    direct_gain: 0.8,
                    direct_cutoff: 0.3,
                    send_gain: [0.3, 0.3, 0.2, 0.1],
                    send_cutoff: [0.8; 4],
                    redirect: None,
                }),
                ..Channel::new()
            },
        );
    }
    mixer.physics_settings = physics.then(|| PhysicsSettings {
        enabled: true,
        ..PhysicsSettings::default()
    });
    mixer.spatial.hrtf = hrtf;
    let mut out = vec![0.0f32; 480 * 2];
    let started = Instant::now();
    for _ in 0..(seconds * 100.0) as usize {
        mix(&mut mixer, &mut out, 2, 48_000);
    }
    started.elapsed()
}
/// Why the game should not open an output, if it should not:
/// `HORIZON_NO_AUDIO`, or a virtual screen (`vscreen`), where agents and
/// harnesses run the game and must stay off the user's speakers.
/// `HORIZON_AUDIO=1` opens the output anyway (testing into a null sink).
#[cfg(any(feature = "audio", test))]
fn output_disabled(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<String> {
    if var("HORIZON_NO_AUDIO").is_some() {
        return Some("audio off: HORIZON_NO_AUDIO is set".into());
    }
    if var("HORIZON_AUDIO").is_some_and(|v| v == "1") {
        return None;
    }
    let display = var("WAYLAND_DISPLAY")?;
    Path::new(&display)
        .components()
        .any(|c| c.as_os_str() == "vscreen")
        .then(|| "audio off on a virtual screen (vscreen); HORIZON_AUDIO=1 opens it".into())
}
/// An output stream on `device` in its default configuration, whatever its
/// sample format: the mixer works in f32 and converts.
#[cfg(feature = "audio")]
fn open_device(
    device: &cpal::Device,
    mixer: Arc<Mutex<Mixer>>,
) -> Result<(cpal::Stream, cpal::SupportedStreamConfig)> {
    use cpal::SampleFormat as F;
    let config = device.default_output_config()?;
    let stream = match config.sample_format() {
        F::F32 => build_stream::<f32>(device, &config, mixer),
        F::F64 => build_stream::<f64>(device, &config, mixer),
        F::I16 => build_stream::<i16>(device, &config, mixer),
        F::I32 => build_stream::<i32>(device, &config, mixer),
        F::I8 => build_stream::<i8>(device, &config, mixer),
        F::U16 => build_stream::<u16>(device, &config, mixer),
        F::U32 => build_stream::<u32>(device, &config, mixer),
        F::U8 => build_stream::<u8>(device, &config, mixer),
        other => Err(format!("unsupported sample format {other}").into()),
    }?;
    stream.play()?;
    Ok((stream, config))
}
#[cfg(feature = "audio")]
fn build_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    mixer: Arc<Mutex<Mixer>>,
) -> Result<cpal::Stream> {
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let mut scratch = Vec::new();
    let stream = device.build_output_stream(
        &config.config(),
        move |data: &mut [T], _| {
            scratch.resize(data.len(), 0.0f32);
            if let Ok(mut m) = mixer.lock() {
                mix(&mut m, &mut scratch, channels, rate);
            } else {
                scratch.fill(0.0);
            }
            for (out, s) in data.iter_mut().zip(&scratch) {
                *out = T::from_sample(*s);
            }
        },
        |e| log::warn!("Horizon: audio stream error: {e}"),
        None,
    )?;
    Ok(stream)
}
impl Audio {
    pub fn new(root: &Path) -> Self {
        Self::with_output(root, true)
    }
    /// No output device: the dedicated server.
    pub fn silent(root: &Path) -> Self {
        Self::with_output(root, false)
    }
    fn with_output(root: &Path, output: bool) -> Self {
        let registry = SoundRegistry::load(root);
        if output && registry.is_empty() {
            // The game then plays nothing on a working output (#1035, #1076).
            log::warn!(
                "Horizon: no sounds in {}: they were not copied from your Minecraft \
                 install; prepare the game again in the launcher (or run \
                 horizon-prepare sounds)",
                root.join("assets").display()
            );
        }
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        let opened = if output {
            Self::open(mixer.clone())
        } else {
            Err("no output on a dedicated server".into())
        };
        let (stream, live) = match opened {
            Ok(v) => (Some(v), true),
            Err(e) => {
                if output {
                    log::warn!("Horizon: no sound: {e}");
                }
                (None, false)
            }
        };
        let mut engine = SoundEngine::new(registry);
        engine.loaded = live;
        Self {
            _stream: stream,
            engine,
            backend: MixerBackend {
                mixer,
                root: root.to_owned(),
                cache: Arc::default(),
                decoder: None,
                next: 0,
                live,
                acoustics: Acoustics::default(),
                listener_underwater: false,
                ticks: 0,
                listener_cell: None,
                whooshes: HashMap::new(),
                whoosh_samples: None,
            },
            record_heard: false,
            listener: [0.0; 3],
            listener_yaw: 0.0,
            master: 1.0,
            random: LegacyRandom::new(crate::sound_engine::unique_seed()),
            source_override: None,
            capture: None,
            heard: Default::default(),
            stopped: Default::default(),
        }
    }
    #[cfg(not(feature = "audio"))]
    fn open(_: Arc<Mutex<Mixer>>) -> Result<Stream> {
        Err("this build has no sound output (built without the `audio` feature)".into())
    }
    #[cfg(feature = "audio")]
    fn open(mixer: Arc<Mutex<Mixer>>) -> Result<Stream> {
        if let Some(reason) = output_disabled(|name| std::env::var_os(name)) {
            return Err(reason.into());
        }
        let host = cpal::default_host();
        let mut errors = Vec::new();
        // The default device first, then any other output a host lists:
        // a default that cannot open (busy, or a stale ALSA default) should
        // not leave the game silent when another device works.
        let default = host.default_output_device();
        let others = host.output_devices().into_iter().flatten();
        let mut tried = Vec::new();
        for device in default.into_iter().chain(others) {
            let name = device.name().unwrap_or_else(|_| "unnamed device".into());
            if tried.contains(&name) {
                continue;
            }
            tried.push(name.clone());
            match open_device(&device, mixer.clone()) {
                Ok((stream, config)) => {
                    log::info!(
                        "Horizon: audio output {name} ({} Hz, {} channels, {})",
                        config.sample_rate().0,
                        config.channels(),
                        config.sample_format()
                    );
                    return Ok(stream);
                }
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        if errors.is_empty() {
            return Err(format!("no audio output device on the {} host", host.id().name()).into());
        }
        Err(format!("no audio output device opened ({})", errors.join("; ")).into())
    }
    fn bound(&mut self) -> Bound<'_, MixerBackend> {
        Bound {
            engine: &mut self.engine,
            backend: &mut self.backend,
        }
    }
    /// `SoundManager.updateSource`: the camera position and rotation
    /// (`Listener.setTransform` takes its forward and up vectors).
    pub fn set_listener(&mut self, position: [f64; 3], yaw: f32, pitch: f32) {
        self.listener = position;
        self.listener_yaw = yaw;
        if let Ok(mut m) = self.backend.mixer.lock() {
            m.listener = position;
            m.listener_yaw = yaw;
            m.listener_pitch = pitch;
            m.listener_motion.update(position, Instant::now());
        }
    }
    /// Sync the volume options, refreshing the categories that changed
    /// (the option sliders' `refreshCategoryVolume`).
    fn sync_volumes(&mut self) {
        let mut volumes = [self.master; 11];
        for (i, v) in volumes.iter_mut().enumerate().skip(1) {
            *v = crate::settings::source_volume(i);
        }
        for source in 0..11 {
            if self.engine.source_volumes[source] != volumes[source] {
                self.engine.source_volumes[source] = volumes[source];
                self.engine
                    .refresh_category_volume(source, &mut self.backend);
            }
        }
        self.engine.listening = self.record_heard;
    }
    /// `SoundManager.tick(paused)`, once per client tick.
    pub fn tick(&mut self, paused: bool, ctx: &TickContext) {
        self.sync_volumes();
        self.engine.tick(paused, ctx, &mut self.backend);
        self.backend.ticks += 1;
        if !paused {
            self.backend.reevaluate();
        }
    }
    /// Sound physics' settings (the extra's), applied from the next sound.
    pub fn set_sound_physics(&mut self, settings: &PhysicsSettings) {
        if self.backend.acoustics.settings() == settings {
            return;
        }
        if let Ok(mut m) = self.backend.mixer.lock() {
            m.spatial = Spatial {
                hrtf: settings.hrtf,
                doppler: settings.doppler.then_some(settings.doppler_factor),
                whoosh: settings.arrow_whoosh,
            };
        }
        self.backend.acoustics.set_settings(settings);
        if settings.enabled && self.backend.live {
            self.backend
                .acoustics
                .start(MixerSink(self.backend.mixer.clone()));
        }
        if let Ok(mut m) = self.backend.mixer.lock() {
            m.physics_settings = settings.enabled.then(|| settings.clone());
            if !settings.enabled {
                for c in m.channels.values_mut() {
                    c.environment = None;
                    c.hold = None;
                }
            }
        }
    }
    /// Once a tick: the arrows in flight (a key of their own, position,
    /// speed in blocks a second), each whooshing on a channel of its own
    /// while `arrow_whoosh` is on, on the players' volume.
    pub fn set_whooshes(&mut self, flying: &[(u32, [f64; 3], f32)]) {
        const EVENT: &str = "horizon:entity.arrow.whoosh";
        let on = self.backend.acoustics.settings().arrow_whoosh && self.backend.live;
        let volume = self.engine.source_volumes[crate::sound_engine::MASTER]
            * self.engine.source_volumes[crate::sound_engine::PLAYERS];
        let gone: Vec<u32> = self
            .backend
            .whooshes
            .keys()
            .filter(|k| !on || !flying.iter().any(|f| f.0 == **k))
            .copied()
            .collect();
        for key in gone {
            if let Some(channel) = self.backend.whooshes.remove(&key) {
                self.backend.destroy(channel);
            }
        }
        if !on {
            return;
        }
        let samples = self
            .backend
            .whoosh_samples
            .get_or_insert_with(|| Arc::new(crate::spatial::whoosh_samples()))
            .clone();
        for &(key, at, speed) in flying {
            let (gain, pitch) = crate::spatial::whoosh(speed);
            let channel = match self.backend.whooshes.get(&key) {
                Some(&c) => c,
                None => {
                    if gain <= 0.0 {
                        continue;
                    }
                    let Some(c) = self.backend.create(false) else {
                        continue;
                    };
                    let start = f64::from(key % 97) / 97.0 * samples.len() as f64;
                    self.backend.with(c, |ch| {
                        ch.samples = Some((samples.clone(), crate::spatial::WHOOSH_RATE));
                        ch.position = start;
                        ch.looping = true;
                        ch.linear = Some(16.0);
                        ch.state = State::Playing;
                        ch.tag = Some((EVENT.into(), crate::sound_engine::PLAYERS));
                    });
                    self.backend.whooshes.insert(key, c);
                    self.backend.set_position(c, at);
                    let request = Request {
                        at,
                        listener: self.listener,
                        listener_underwater: self.backend.listener_underwater,
                        source: crate::sound_engine::PLAYERS,
                        event: EVENT.into(),
                    };
                    self.backend.acoustics.evaluate(c, request, 16.0);
                    c
                }
            };
            self.backend.set_position(channel, at);
            self.backend.set_volume(channel, gain * volume);
            self.backend.set_pitch(channel, pitch);
        }
    }
    pub fn sound_physics_enabled(&self) -> bool {
        self.backend.acoustics.enabled() && self.backend.live
    }
    pub fn sound_physics_settings(&self) -> &PhysicsSettings {
        self.backend.acoustics.settings()
    }
    /// Whether sound physics needs the `count` block states (`set_sound_states`).
    pub fn needs_sound_states(&self, count: usize) -> bool {
        self.sound_physics_enabled() && self.backend.acoustics.needs_states(count)
    }
    pub fn set_sound_states(&mut self, states: Vec<crate::acoustics::BlockState>) {
        self.backend.acoustics.set_states(states);
    }
    /// Once a tick with sound physics on: whether the camera is underwater,
    /// and the chunk sections around the listener (`chunk` gives a loaded
    /// chunk's).
    pub fn sync_sound_physics<'c>(
        &mut self,
        underwater: bool,
        chunk: impl FnMut(i32, i32) -> Option<&'c [Arc<crate::chunk::Section>]>,
    ) {
        self.backend.listener_underwater = underwater;
        if self.sound_physics_enabled() {
            self.backend.acoustics.sync(self.listener, chunk);
        }
    }
    /// `SoundManager.pauseAllExcept(MUSIC, UI)`.
    pub fn pause(&mut self) {
        self.engine.pause_all_except(
            &[crate::sound_engine::MUSIC, crate::sound_engine::UI],
            &mut self.backend,
        );
    }
    /// `SoundManager.resume`.
    pub fn resume(&mut self) {
        self.engine.resume(&mut self.backend);
    }
    /// `SoundManager.stop()`: every sound.
    pub fn stop_all(&mut self) {
        self.engine.stop_all(&mut self.backend);
    }
    /// `SoundManager.reload` (resource packs changed): stop every sound, then
    /// read sound events and sound files from the new resource root.
    pub fn reload(&mut self, root: &Path) {
        self.stop_all();
        self.engine.registry = SoundRegistry::load(root);
        self.backend.root = root.to_owned();
        let mut cache = self.backend.cache.lock().unwrap();
        cache.entries.clear();
        cache.bytes = 0;
    }
    /// Play a sound event at a world position, as `ClientLevel.playLocalSound`
    /// (a `SimpleSoundInstance` seeded from the level random), or relative to
    /// the listener without attenuation (`None`).
    pub fn play(&mut self, event: &str, at: Option<[f64; 3]>, volume: f32, pitch: f32) {
        let seed = self.random.next_long();
        self.play_seeded(event, at, volume, pitch, seed);
    }
    /// `play` for this client only: a sound a packet's handler plays
    /// (`playLocalSound`), which the guests play from that packet themselves.
    pub fn play_unshared(&mut self, event: &str, at: Option<[f64; 3]>, volume: f32, pitch: f32) {
        let capture = self.capture.take();
        self.play(event, at, volume, pitch);
        self.capture = capture;
    }
    /// `LivingEntity.handleDamageEvent`'s pitch, `(r - r) * 0.2 + 1`, drawn on the
    /// client (the entity's own random there, never the server's world random).
    pub fn hurt_pitch(&mut self) -> f32 {
        (self.random.next_float() - self.random.next_float()) * 0.2 + 1.0
    }
    /// `AbstractWidget.playButtonClickSound`: an unseeded, relative UI sound.
    pub fn play_button_click(&mut self) {
        self.play_instance(
            SoundInstance::for_ui(
                "minecraft:ui.button.click",
                1.0,
                0.25,
                SoundEngine::unseeded(),
            ),
            false,
        );
    }
    /// `play` with the seed a sound packet carries.
    pub fn play_seeded(
        &mut self,
        event: &str,
        at: Option<[f64; 3]>,
        volume: f32,
        pitch: f32,
        seed: i64,
    ) {
        if let (Some(capture), Some(at)) = (self.capture.as_mut(), at) {
            capture.push((event.to_owned(), at, volume, pitch));
        }
        if at.is_some() {
            *self.heard.entry(event.to_owned()).or_default() += 1;
        }
        self.sync_volumes();
        let event = crate::sound_events::normalize(event);
        let source = self
            .source_override
            .unwrap_or_else(|| crate::settings::sound_source(&event));
        let random = LegacyRandom::new(seed);
        let instance = match at {
            Some(at) => SoundInstance::at(&event, source, volume, pitch, random, at),
            None => SoundInstance::new(
                &event,
                source,
                volume,
                pitch,
                random,
                false,
                0,
                Attenuation::None,
                [0.0; 3],
                true,
            ),
        };
        self.play_instance(instance, false);
    }
    /// Play a sound on a given `SoundSource` index (`/playsound`).
    pub fn play_on(
        &mut self,
        source: usize,
        event: &str,
        at: Option<[f64; 3]>,
        volume: f32,
        pitch: f32,
    ) {
        self.source_override = Some(source);
        self.play(event, at, volume, pitch);
        self.source_override = None;
    }
    /// `SimpleSoundInstance.forJukeboxSong` at a jukebox; the handle stays
    /// until `release`.
    pub fn play_jukebox_song(&mut self, event: &str, pos: [i32; 3]) -> InstanceId {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        let event = crate::sound_events::normalize(event);
        let instance = SoundInstance::for_jukebox_song(&event, at, SoundEngine::unseeded());
        self.play_instance(instance, true).0
    }
    /// Stop and drop a held instance.
    pub fn stop_and_release(&mut self, id: InstanceId) {
        self.stop_instance(id);
        self.release(id);
    }
    /// Every sound event id (for command completion).
    /// The engine's table sizes and the mixer's channels (`status`).
    pub fn census(&self) -> serde_json::Value {
        let mut census = self.engine.census();
        census["channels"] = self
            .backend
            .mixer
            .lock()
            .map_or(0, |m| m.channels.len())
            .into();
        if let Ok(cache) = self.backend.cache.lock() {
            census["decoded"] = cache.entries.len().into();
            census["decoded_mib"] = (cache.bytes >> 20).into();
        }
        if self.backend.acoustics.enabled() {
            census["soundPhysics"] = self.backend.acoustics.stats().json();
        }
        // The playing voices as the mixer last mixed them, while a Horizon
        // sound extra is on (`last` is kept only then).
        if let Ok(m) = self.backend.mixer.lock() {
            let voices: Vec<serde_json::Value> = m
                .channels
                .values()
                .filter(|c| c.state == State::Playing && c.last.is_some())
                .map(|c| {
                    let v = c.last.expect("filtered");
                    serde_json::json!({
                        "event": c.tag.as_ref().map(|(e, _)| e.to_string()),
                        "at": c.at,
                        "volume": c.gain,
                        "gain": v.mono,
                        "direct": v.wet.map(|w| [w.direct_gain, w.direct_hf]),
                        "sends": v.wet.map(|w| w.sends.map(|s| s.0)),
                        "held": c.hold.is_some(),
                        "speed": c.motion.velocity(Instant::now()).iter().map(|x| x * x).sum::<f32>().sqrt(),
                    })
                })
                .collect();
            if !voices.is_empty() {
                census["voices"] = voices.into();
            }
        }
        census
    }
    pub fn event_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.engine.registry.names().cloned().collect();
        v.sort();
        v
    }
    /// `ClientboundStopSoundPacket`: stop the playing sounds of an event
    /// and/or a source.
    pub fn stop_sounds(&mut self, event: Option<&str>, source: Option<usize>) {
        let event = event.map(crate::sound_events::normalize);
        let selection = format!(
            "{} {}",
            source
                .and_then(|s| crate::settings::SOUND_SOURCES.get(s))
                .map_or("*", |s| s),
            event.as_deref().unwrap_or("*")
        );
        *self.stopped.entry(selection).or_default() += 1;
        self.engine
            .stop_matching(event.as_deref(), source, &mut self.backend);
    }
    /// The sounds played since the last call, for the subtitle overlay.
    pub fn take_heard(&mut self) -> Vec<Heard> {
        std::mem::take(&mut self.engine.notifications)
            .into_iter()
            .filter_map(|n| {
                Some(Heard {
                    subtitle: n.subtitle?,
                    at: Some(n.position),
                    range: n.range,
                    looping: n.looping,
                })
            })
            .collect()
    }
}
impl Sounds for Audio {
    fn engine(&mut self) -> &mut SoundEngine {
        &mut self.engine
    }
    fn play_instance(&mut self, instance: SoundInstance, retain: bool) -> (InstanceId, PlayResult) {
        self.sync_volumes();
        self.bound().play_instance(instance, retain)
    }
    fn play_id(&mut self, id: InstanceId) -> PlayResult {
        self.sync_volumes();
        self.bound().play_id(id)
    }
    fn stop_instance(&mut self, id: InstanceId) {
        self.bound().stop_instance(id);
    }
    fn update_category_volume(&mut self, source: usize, gain: f32) {
        self.bound().update_category_volume(source, gain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_click_uses_vanilla_ui_parameters_without_advancing_world_random() {
        let mut audio = Audio::silent(Path::new("/nonexistent/horizon-test-assets"));
        audio.engine.registry.add_file(
            "minecraft",
            br#"{"ui.button.click":{"sounds":["gui/button"]}}"#,
            &|_, _| true,
        );
        // Exercise the mixer without opening an output device. The missing
        // sample is harmless: this test checks the source the engine creates.
        audio.engine.loaded = true;
        audio.backend.live = true;
        let mut random = audio.random.clone();
        audio.play_button_click();
        let instance = audio.engine.instance(1).expect("click reached the engine");
        assert_eq!(instance.identifier, "minecraft:ui.button.click");
        assert_eq!(instance.source, crate::sound_engine::UI);
        assert_eq!(instance.volume, 0.25);
        assert_eq!(instance.pitch, 1.0);
        assert_eq!(instance.attenuation, Attenuation::None);
        assert_eq!(instance.position, [0.0; 3]);
        assert!(instance.relative);
        assert!(!instance.looping);
        assert_eq!(audio.random.next_long(), random.next_long());
        audio.pause();
        assert_eq!(
            audio.backend.mixer.lock().unwrap().channels[&1].state,
            State::Initial
        );
        audio.stop_all();
    }

    fn channel(samples: Vec<f32>, linear: Option<f32>, at: [f32; 3], looping: bool) -> Channel {
        Channel {
            samples: Some((Arc::new(samples), 48_000)),
            position: 0.0,
            state: State::Playing,
            gain: 1.0,
            pitch: 1.0,
            linear,
            looping,
            at,
            relative: false,
            ..Channel::new()
        }
    }

    /// Mix `mixer` for `frames` stereo frames at 48 kHz.
    fn render(mixer: &mut Mixer, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        mix(mixer, &mut out, 2, 48_000);
        out
    }
    fn noise(n: usize) -> Vec<f32> {
        let mut r = LegacyRandom::new(5);
        (0..n).map(|_| r.next_float() * 0.4 - 0.2).collect()
    }
    fn physics_mixer(environment: Option<Environment>, on: bool) -> Mixer {
        let mut m = Mixer::default();
        let mut c = channel(noise(48_000), Some(16.0), [0.0, 0.0, 4.0], false);
        c.environment = environment;
        m.channels.insert(1, c);
        m.physics_settings = on.then(|| PhysicsSettings {
            enabled: true,
            air_absorption: 0.0,
            ..PhysicsSettings::default()
        });
        m
    }

    #[test]
    fn sound_physics_off_or_dry_leaves_the_mix_as_vanilla() {
        let vanilla = render(&mut physics_mixer(None, false), 4096);
        // An environment left from before it was turned off is ignored.
        let off = render(
            &mut physics_mixer(
                Some(Environment {
                    direct_gain: 0.1,
                    ..Environment::DRY
                }),
                false,
            ),
            4096,
        );
        assert_eq!(vanilla, off);
        let dry = render(&mut physics_mixer(Some(Environment::DRY), true), 4096);
        for (a, b) in vanilla.iter().zip(&dry) {
            assert!((a - b).abs() < 1e-5, "{a} {b}");
        }
    }

    #[test]
    fn occlusion_darkens_and_sends_ring_on() {
        let energy = |s: &[f32]| s.iter().map(|x| x * x).sum::<f32>();
        // High frequencies: the sample-to-sample differences.
        let bright = |s: &[f32]| {
            energy(
                &s.chunks(2)
                    .map(|f| f[0])
                    .collect::<Vec<_>>()
                    .windows(2)
                    .map(|w| w[1] - w[0])
                    .collect::<Vec<_>>(),
            )
        };
        let dry = render(&mut physics_mixer(Some(Environment::DRY), true), 8192);
        let muffled = Environment {
            direct_gain: 0.9,
            direct_cutoff: 0.05,
            ..Environment::DRY
        };
        let behind_wall = render(&mut physics_mixer(Some(muffled), true), 8192);
        assert!(bright(&behind_wall) < 0.2 * bright(&dry));
        let cave = Environment {
            send_gain: [0.0, 0.0, 0.0, 0.8],
            ..Environment::DRY
        };
        let mut m = physics_mixer(Some(cave), true);
        m.channels.get_mut(&1).unwrap().samples = Some((Arc::new(noise(4800)), 48_000));
        render(&mut m, 4800);
        // The source has ended; the 4-second bus still rings.
        let tail = render(&mut m, 9600);
        assert!(energy(&tail) > 1e-3, "{}", energy(&tail));
        let mut m = physics_mixer(Some(Environment::DRY), true);
        m.channels.get_mut(&1).unwrap().samples = Some((Arc::new(noise(4800)), 48_000));
        render(&mut m, 4800);
        assert_eq!(energy(&render(&mut m, 9600)), 0.0);
    }

    #[test]
    fn binaural_and_doppler_are_extras_of_their_own() {
        let side = |m: &mut Mixer| {
            let out = render(m, 4096);
            let energy = |k: usize| out.chunks(2).map(|f| f[k] * f[k]).sum::<f32>();
            (energy(0), energy(1))
        };
        // A source on the right (yaw 0 faces +z; right is -x).
        let mut m = Mixer::default();
        m.channels.insert(
            1,
            channel(noise(48_000), Some(16.0), [-4.0, 0.0, 0.0], false),
        );
        let vanilla = side(&mut m);
        let mut m = Mixer::default();
        m.channels.insert(
            1,
            channel(noise(48_000), Some(16.0), [-4.0, 0.0, 0.0], false),
        );
        m.spatial.hrtf = true;
        let (l, r) = side(&mut m);
        assert!(r > 4.0 * l, "{l} {r}");
        assert_ne!((l, r), vanilla);
        // Off again: vanilla's mix.
        let mut m = Mixer::default();
        m.channels.insert(
            1,
            channel(noise(48_000), Some(16.0), [-4.0, 0.0, 0.0], false),
        );
        m.spatial = Spatial::default();
        assert_eq!(side(&mut m), vanilla);

        // Doppler: a source coming at the listener plays faster.
        let advance = |doppler: Option<f32>| {
            let mut m = Mixer::default();
            let mut c = channel(noise(48_000), Some(64.0), [0.0, 0.0, 30.0], true);
            let t0 = Instant::now();
            c.motion
                .update([0.0, 0.0, 32.0], t0 - std::time::Duration::from_millis(100));
            c.motion
                .update([0.0, 0.0, 30.0], t0 - std::time::Duration::from_millis(50));
            m.channels.insert(1, c);
            m.spatial.doppler = doppler;
            render(&mut m, 4800);
            m.channels[&1].position
        };
        let still = advance(None);
        let coming = advance(Some(1.0));
        assert!(coming > still * 1.05, "{still} {coming}");
    }

    #[test]
    fn moving_on_traces_long_sounds_again() {
        let mut b = MixerBackend {
            mixer: Arc::new(Mutex::new(Mixer::default())),
            root: PathBuf::new(),
            cache: Arc::default(),
            decoder: None,
            next: 0,
            live: true,
            acoustics: Acoustics::default(),
            listener_underwater: false,
            ticks: 10,
            listener_cell: None,
            whooshes: HashMap::new(),
            whoosh_samples: None,
        };
        let (send, receive) = std::sync::mpsc::channel();
        struct Count(Mutex<std::sync::mpsc::Sender<ChannelId>>);
        impl Sink for Count {
            fn environment(&self, channel: ChannelId, _: Environment) {
                let _ = self.0.lock().unwrap().send(channel);
            }
        }
        b.acoustics.set_settings(&PhysicsSettings {
            enabled: true,
            ..PhysicsSettings::default()
        });
        b.acoustics.start(Count(Mutex::new(send)));
        b.acoustics.set_states(vec![crate::acoustics::BlockState {
            block: "minecraft:air".into(),
            break_sound: "".into(),
            step_sound: "".into(),
            fluid: crate::sound_physics::FluidKind::None,
            collides: false,
            sturdy: 0,
            closes: 0,
        }]);
        {
            let mut m = b.mixer.lock().unwrap();
            for (id, looping) in [(1, true), (2, false)] {
                let mut c = channel(noise(4_800), Some(16.0), [0.0, 64.0, 4.0], looping);
                c.tag = Some(("minecraft:block.note_block.harp".into(), 4));
                m.channels.insert(id, c);
            }
            m.listener = [0.5, 64.5, 0.5];
        }
        b.reevaluate();
        assert!(receive.try_recv().is_err(), "nothing on the first tick");
        b.ticks += 3;
        b.mixer.lock().unwrap().listener = [1.5, 64.5, 0.5];
        b.reevaluate();
        // The loop is traced again; the 0.1 s sound is about to end.
        let traced = receive
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert_eq!(traced, 1);
        assert!(
            receive
                .recv_timeout(std::time::Duration::from_millis(300))
                .is_err()
        );
    }

    #[test]
    fn held_sounds_wait_for_their_environment_and_redirects_pan() {
        let mut m = physics_mixer(None, true);
        m.channels.get_mut(&1).unwrap().hold =
            Some(Instant::now() + std::time::Duration::from_secs(60));
        assert!(render(&mut m, 512).iter().all(|x| *x == 0.0));
        assert_eq!(m.channels[&1].position, 0.0);
        MixerSink(Arc::new(Mutex::new(Mixer::default()))).environment(1, Environment::DRY);
        let shared = Arc::new(Mutex::new(m));
        MixerSink(shared.clone()).environment(1, Environment::DRY);
        let mut m = Arc::try_unwrap(shared).ok().unwrap().into_inner().unwrap();
        assert!(render(&mut m, 512).iter().any(|x| *x != 0.0));
        // Heard from the right instead of straight ahead (yaw 0 faces +z).
        let right = Environment {
            redirect: Some([-4.0, 0.0, 0.0]),
            ..Environment::DRY
        };
        let out = render(&mut physics_mixer(Some(right), true), 4096);
        let side = |k: usize| out.chunks(2).map(|f| f[k] * f[k]).sum::<f32>();
        assert!(side(1) > 4.0 * side(0), "{} {}", side(0), side(1));
    }

    #[test]
    fn decoded_sounds_stay_within_the_budget_least_recently_used_out() {
        let mut cache = SoundCache::default();
        let quarter = CACHE_BYTES / 4 / std::mem::size_of::<f32>();
        let sound = || Some((Arc::new(vec![0.0f32; quarter]), 44100));
        for key in ["a", "b", "c", "d"] {
            cache.insert(key.into(), sound());
        }
        assert!(cache.get("a").is_some());
        cache.insert("e".into(), sound());
        assert!(cache.bytes <= CACHE_BYTES);
        assert!(
            cache.get("b").is_none(),
            "the least recently used goes first"
        );
        assert!(cache.get("a").is_some() && cache.get("e").is_some());
        // A failed load is remembered without counting.
        cache.insert("missing".into(), None);
        assert_eq!(cache.get("missing"), Some(None));
    }
    #[test]
    fn output_stays_off_on_a_virtual_screen_unless_asked() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| std::ffi::OsString::from(*v))
            }
        };
        assert!(output_disabled(env(&[("WAYLAND_DISPLAY", "wayland-1")])).is_none());
        assert!(output_disabled(env(&[])).is_none());
        const VSCREEN: &str = "/run/user/1000/vscreen/main/wayland-1";
        assert!(output_disabled(env(&[("WAYLAND_DISPLAY", VSCREEN)])).is_some());
        assert!(
            output_disabled(env(&[("WAYLAND_DISPLAY", VSCREEN), ("HORIZON_AUDIO", "1")])).is_none()
        );
        assert!(
            output_disabled(env(&[("HORIZON_NO_AUDIO", "1"), ("HORIZON_AUDIO", "1")])).is_some()
        );
    }
    #[test]
    fn linear_attenuation_reaches_zero_at_the_max_distance() {
        assert_eq!(distance_gain(None, 100.0), 1.0);
        assert_eq!(distance_gain(Some(16.0), 0.0), 1.0);
        assert_eq!(distance_gain(Some(16.0), 8.0), 0.5);
        assert_eq!(distance_gain(Some(16.0), 16.0), 0.0);
        assert_eq!(distance_gain(Some(16.0), 40.0), 0.0);
    }

    #[test]
    fn overlapping_voices_are_limited_without_flattening_or_changing_stereo_balance() {
        let mut m = Mixer::default();
        let wave = vec![0.25, 0.5, 0.75, 1.0, -1.0, -0.75, -0.5, -0.25, 0.0];
        for id in 1..=4 {
            m.channels
                .insert(id, channel(wave.clone(), None, [-1.0, 0.0, 0.0], false));
        }
        let mut data = [0.0; 16];
        mix(&mut m, &mut data, 2, 48_000);
        // West of a listener facing south: fully on the right.
        for (frame, sample) in data.as_chunks::<2>().0.iter().zip(wave) {
            assert!(frame[0].abs() < 1e-6, "{frame:?}");
            assert!((frame[1] - sample).abs() < 1e-6, "{frame:?}");
        }
        assert!(data.iter().all(|s| s.abs() <= 1.0));
    }

    #[test]
    fn limiter_leaves_quiet_audio_alone_and_recovers_across_callbacks() {
        let mut m = Mixer::default();
        m.channels
            .insert(1, channel(vec![0.25; 2], None, [0.0; 3], true));
        let quiet = 0.25 * stereo_gains(0.0, 0.0, 1.0)[0];
        let mut data = [0.0; 16];
        mix(&mut m, &mut data, 2, 48_000);
        assert_eq!(data, [quiet; 16]);

        for id in 2..=5 {
            m.channels
                .insert(id, channel(vec![1.0; 2], None, [0.0; 3], true));
        }
        mix(&mut m, &mut data, 2, 48_000);
        assert!(data.iter().all(|s| s.abs() <= 1.0));
        m.channels.retain(|&id, _| id == 1);
        let mut recovery = vec![0.0; 2 * 48_000];
        mix(&mut m, &mut recovery, 2, 48_000);
        assert!(recovery[0] > 0.0 && recovery[0] < 0.1);
        assert!(recovery.windows(2).all(|w| w[1] >= w[0]));
        assert_eq!(*recovery.last().unwrap(), quiet);
    }

    #[test]
    fn sources_play_attenuated_then_stop_or_loop() {
        let mut m = Mixer::default();
        m.channels
            .insert(1, channel(vec![1.0; 4], Some(16.0), [8.0, 0.0, 0.0], false));
        m.channels
            .insert(2, channel(vec![1.0; 4], None, [0.0; 3], true));
        let mut data = [0.0f32; 16];
        mix(&mut m, &mut data, 2, 48_000);
        assert_eq!(m.channels[&1].state, State::Stopped);
        assert_eq!(m.channels[&2].state, State::Playing);
        // Mono sum of the two at the first frame: the far one is half as loud.
        let first = data[0] + data[1];
        assert!(first > 1.4 && first < 3.1, "{first}");
    }

    #[test]
    fn stereo_panning_matches_openal_soft() {
        // OpenAL Soft 1.25.2 (LWJGL 3.4.3's macOS build), stereo loopback
        // device with vanilla's attributes, 1 kHz sine 4 blocks away.
        let cases = [
            ((0.0, 0.0, 1.0), [0.5957, 0.5957]),
            ((-1.0, 0.0, 0.0), [1.0, 0.0]),
            ((1.0, 0.0, 0.0), [0.0, 1.0]),
            ((0.0, 0.0, -1.0), [0.4043, 0.4043]),
            ((0.0, 1.0, 0.0), [0.5, 0.5]),
            ((0.0, -1.0, 0.0), [0.5, 0.5]),
            ((-1.0, 0.0, 1.0), [0.9985, 0.0747]),
            ((-1.0, 0.0, -1.0), [0.7859, 0.0788]),
            ((0.0, 0.0, 0.0), [0.5957, 0.5957]),
        ];
        for ((right, up, forward), expected) in cases {
            let gains = stereo_gains(right, up, forward);
            for (g, e) in gains.into_iter().zip(expected) {
                assert!((g - e).abs() < 1e-4, "{right} {up} {forward}: {gains:?}");
            }
        }
    }

    #[test]
    fn panning_follows_the_camera_rotation() {
        let gains = |yaw: f32, pitch: f32, at: [f32; 3]| {
            let mut m = Mixer {
                listener_yaw: yaw,
                listener_pitch: pitch,
                ..Mixer::default()
            };
            m.channels.insert(1, channel(vec![1.0; 4], None, at, true));
            let mut data = [0.0; 2];
            mix(&mut m, &mut data, 2, 48_000);
            data
        };
        let close =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
        // Facing south (yaw 0), east is on the left; facing north, on the right.
        assert!(close(gains(0.0, 0.0, [1.0, 0.0, 0.0]), [1.0, 0.0]));
        assert!(close(gains(180.0, 0.0, [1.0, 0.0, 0.0]), [0.0, 1.0]));
        // Looking straight down, a source below is ahead and one south is above.
        assert!(close(gains(0.0, 90.0, [0.0, -1.0, 0.0]), [0.5957, 0.5957]));
        assert!(close(gains(0.0, 90.0, [0.0, 0.0, 1.0]), [0.5, 0.5]));
    }

    #[test]
    fn a_mono_output_plays_sources_unpanned() {
        let mut m = Mixer::default();
        m.channels
            .insert(1, channel(vec![0.5; 4], None, [0.0, 0.0, -3.0], true));
        let mut data = [0.0; 2];
        mix(&mut m, &mut data, 1, 48_000);
        assert_eq!(data, [0.5; 2]);
    }
}
