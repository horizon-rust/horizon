//! The OpenAL context belongs to this thread. Game-thread updates only change
//! channel bookkeeping and wake it; decoding and buffer uploads never hold that lock.
use super::{ChannelId, Mixer, State, decoder::Decoder};
use horizon_openal::{NativeOutput, VoiceSettings, VoiceState};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

enum Command {
    Wake,
    Failed(ChannelId),
    Listener([f64; 3], [f32; 3], [f32; 3]),
    Stream(ChannelId, u32, Receiver<Vec<f32>>),
    Shutdown,
}
#[derive(Clone)]
pub(super) struct Notify(Sender<Command>);
impl Notify {
    pub fn wake(&self) {
        let _ = self.0.send(Command::Wake);
    }
}
pub(super) struct Output {
    notify: Notify,
    thread: Option<JoinHandle<()>>,
    pub selection: String,
}
struct Stream {
    rate: u32,
    receive: Receiver<Vec<f32>>,
    finished: bool,
    has_samples: bool,
}
impl Output {
    pub fn open(name: &str, mixer: Arc<Mutex<Mixer>>) -> Result<Self, String> {
        let selection = name.to_owned();
        let name = name.to_owned();
        let (send, receive) = mpsc::channel();
        let (ready, opened) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("sound output".into())
            .spawn(move || {
                let mut native = match NativeOutput::open(&name) {
                    Ok(native) => native,
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                if let Ok(mut m) = mixer.lock() {
                    m.device_name = native.device_name().to_owned();
                }
                log::info!(
                    "Horizon: audio output OpenAL Soft: {} (native sources)",
                    native.device_name()
                );
                if ready.send(Ok(())).is_err() {
                    return;
                }
                run(&mut native, &mixer, receive);
            })
            .map_err(|e| e.to_string())?;
        if let Err(e) = opened.recv().map_err(|e| e.to_string())? {
            let _ = thread.join();
            return Err(e);
        }
        Ok(Self {
            notify: Notify(send),
            thread: Some(thread),
            selection,
        })
    }
    pub fn notify(&self) -> Notify {
        self.notify.clone()
    }
    pub fn listener(&self, at: [f64; 3], forward: [f32; 3], up: [f32; 3]) {
        let _ = self.notify.0.send(Command::Listener(at, forward, up));
    }
    pub fn stream(&self, id: ChannelId, file: PathBuf, looping: bool) {
        let notify = self.notify.clone();
        let result = thread::Builder::new()
            .name("sound stream".into())
            .spawn(move || {
                let Some(mut decoder) = Decoder::open(&file) else {
                    let _ = notify.0.send(Command::Failed(id));
                    return;
                };
                // At most three decoded blocks wait ahead of the three OpenAL buffers.
                let (send, receive) = mpsc::sync_channel(3);
                if notify
                    .0
                    .send(Command::Stream(id, decoder.rate, receive))
                    .is_err()
                {
                    return;
                }
                let mut read_any = false;
                loop {
                    let samples = match decoder.next_chunk() {
                        Ok(samples) => samples,
                        Err(e) => {
                            log::warn!("Horizon: streaming {}: {e}", file.display());
                            break;
                        }
                    };
                    if samples.is_empty() {
                        if !looping || !read_any {
                            break;
                        }
                        let Some(reopened) = Decoder::open(&file) else {
                            break;
                        };
                        if reopened.rate != decoder.rate {
                            break;
                        }
                        decoder = reopened;
                        read_any = false;
                        continue;
                    }
                    read_any = true;
                    if send.send(samples).is_err() {
                        return;
                    }
                    notify.wake();
                }
                drop(send);
                notify.wake();
            });
        if let Err(e) = result {
            log::warn!("Horizon: could not start sound stream: {e}");
        }
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        let _ = self.notify.0.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn run(native: &mut NativeOutput, mixer: &Mutex<Mixer>, receive: Receiver<Command>) {
    let mut streams = HashMap::<ChannelId, Stream>::new();
    let mut sources = HashSet::<ChannelId>::new();
    let mut last_poll = std::time::Instant::now();
    loop {
        let first = match receive.recv_timeout(Duration::from_millis(10)) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let mut listener = None;
        let mut changed = false;
        for command in first.into_iter().chain(receive.try_iter()) {
            match command {
                Command::Shutdown => return,
                Command::Wake => changed = true,
                Command::Failed(id) => {
                    changed = true;
                    if let Ok(mut m) = mixer.lock()
                        && let Some(c) = m.channels.get_mut(&id)
                    {
                        c.state = State::Stopped;
                    }
                }
                Command::Listener(at, forward, up) => listener = Some((at, forward, up)),
                Command::Stream(id, rate, receive) => {
                    changed = true;
                    streams.insert(
                        id,
                        Stream {
                            rate,
                            receive,
                            finished: false,
                            has_samples: false,
                        },
                    );
                }
            }
        }
        if let Some((at, forward, up)) = listener
            && let Err(e) = native.set_listener(at, forward, up)
        {
            log::warn!("Horizon: OpenAL listener: {e}");
        }
        let poll = last_poll.elapsed() >= Duration::from_millis(10);
        if poll {
            last_poll = std::time::Instant::now();
        }
        if !changed && !poll {
            continue;
        }
        let (active, channels) = match mixer.lock() {
            Ok(mut m) => {
                let active = m
                    .channels
                    .iter()
                    .filter(|(_, c)| c.state != State::Stopped)
                    .map(|(&id, _)| id)
                    .collect::<HashSet<_>>();
                let channels = m
                    .channels
                    .iter_mut()
                    .filter_map(|(&id, c)| {
                        if !c.dirty && !poll && !streams.contains_key(&id) {
                            return None;
                        }
                        c.dirty = false;
                        Some((id, c.clone()))
                    })
                    .collect::<Vec<_>>();
                (active, channels)
            }
            Err(_) => return,
        };
        // No OpenAL operation or decoding runs with the game thread's mutex held.
        sources.retain(|id| {
            let keep = active.contains(id);
            if !keep {
                native.remove_voice(*id);
            }
            keep
        });
        streams.retain(|id, _| active.contains(id));
        for (id, mut channel) in channels {
            if channel.state == State::Stopped {
                continue;
            }
            sources.insert(id);
            let result = (|| -> Result<bool, String> {
                if let Some(stream) = streams.get_mut(&id) {
                    while !stream.finished && native.stream_space(id)? > 0 {
                        match stream.receive.try_recv() {
                            Ok(samples) => {
                                native.queue_stream(id, &samples, stream.rate)?;
                                stream.has_samples = true;
                                if channel.state == State::Initial {
                                    channel.state = State::Playing;
                                    if let Ok(mut m) = mixer.lock()
                                        && let Some(c) = m.channels.get_mut(&id)
                                        && c.state == State::Initial
                                    {
                                        c.state = State::Playing;
                                    }
                                }
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                stream.finished = true;
                                native.finish_stream(id);
                            }
                        }
                    }
                    if stream.finished && !stream.has_samples {
                        return Ok(true);
                    }
                }
                native.sync_voice(
                    id,
                    VoiceSettings {
                        samples: channel
                            .samples
                            .as_ref()
                            .map(|(samples, rate)| (samples, *rate)),
                        state: match channel.state {
                            State::Initial => VoiceState::Initial,
                            State::Playing => VoiceState::Playing,
                            State::Paused => VoiceState::Paused,
                            State::Stopped => VoiceState::Stopped,
                        },
                        gain: channel.gain,
                        pitch: channel.pitch,
                        max_distance: channel.linear,
                        looping: channel.looping,
                        position: channel.at,
                        relative: channel.relative,
                    },
                )
            })();
            let stopped = match result {
                Ok(ended) => ended,
                Err(e) => {
                    log::warn!("Horizon: OpenAL source {id}: {e}");
                    true
                }
            };
            if stopped
                && let Ok(mut m) = mixer.lock()
                && let Some(c) = m.channels.get_mut(&id)
                && c.state == channel.state
            {
                c.state = State::Stopped;
            }
        }
        if !native.connected()
            && let Ok(mut m) = mixer.lock()
        {
            m.device_lost = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "manual music probe: ALSOFT_DRIVERS=null HORIZON_TEST_MUSIC=/path/to/music.ogg"]
    fn music_stream_and_many_sources_keep_game_updates_responsive() {
        let Some(file) = std::env::var_os("HORIZON_TEST_MUSIC") else {
            return;
        };
        use crate::sound_engine::Backend;
        let mut backend = super::super::MixerBackend {
            mixer: Arc::default(),
            native: None,
            root: PathBuf::new(),
            cache: Arc::default(),
            decoder: None,
            next: 0,
            live: true,
        };
        let output = Output::open("", backend.mixer.clone()).unwrap();
        let music = backend.create(true).unwrap();
        let started = std::time::Instant::now();
        output.stream(music, file.into(), false);
        let dispatch = started.elapsed();
        let samples = Arc::new(vec![0.0; 24_000]);
        for _ in 0..64 {
            let id = backend.create(false).unwrap();
            super::super::attach(&backend.mixer, id, Some((samples.clone(), 48_000)), false);
        }
        output.notify().wake();
        let mut maximum = Duration::ZERO;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while backend.mixer.lock().unwrap().channels[&music].state == State::Initial {
            assert!(std::time::Instant::now() < deadline, "music did not begin");
            thread::sleep(Duration::from_millis(1));
        }
        let ready = started.elapsed();
        for _ in 0..200 {
            let before = std::time::Instant::now();
            backend.set_volume(music, 0.5);
            maximum = maximum.max(before.elapsed());
            output.notify().wake();
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            backend.mixer.lock().unwrap().channels[&music]
                .samples
                .is_none()
        );
        eprintln!(
            "music dispatch: {dispatch:?}; first streamed block: {ready:?}; max game update with music + 64 sources: {maximum:?}"
        );
    }

    #[test]
    #[ignore = "requires OpenAL Soft; run with ALSOFT_DRIVERS=null"]
    fn decoded_sound_plays_and_finishes_without_a_game_tick() {
        let mut backend = super::super::MixerBackend {
            mixer: Arc::default(),
            native: None,
            root: PathBuf::new(),
            cache: Arc::default(),
            decoder: None,
            next: 0,
            live: true,
        };
        use crate::sound_engine::Backend;
        let output = Output::open("", backend.mixer.clone()).unwrap();
        let id = backend.create(false).unwrap();
        super::super::attach(
            &backend.mixer,
            id,
            Some((Arc::new(vec![0.0; 2_400]), 48_000)),
            false,
        );
        output.notify().wake();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while backend.mixer.lock().unwrap().channels[&id].state != State::Stopped {
            assert!(
                std::time::Instant::now() < deadline,
                "sound never reached OpenAL without tick()"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    #[ignore = "requires OpenAL Soft; run with ALSOFT_DRIVERS=null"]
    fn streaming_queue_consumes_bounded_blocks_without_static_samples() {
        use crate::sound_engine::Backend;
        let mut backend = super::super::MixerBackend {
            mixer: Arc::default(),
            native: None,
            root: PathBuf::new(),
            cache: Arc::default(),
            decoder: None,
            next: 0,
            live: true,
        };
        let output = Output::open("", backend.mixer.clone()).unwrap();
        let id = backend.create(true).unwrap();
        let (send, receive) = mpsc::sync_channel(3);
        output
            .notify
            .0
            .send(Command::Stream(id, 48_000, receive))
            .unwrap();
        let producer = thread::spawn(move || {
            for _ in 0..12 {
                if send.send(vec![0.0; 480]).is_err() {
                    return;
                }
            }
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let state = backend.mixer.lock().unwrap().channels[&id].state;
            if state == State::Stopped {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "stream never finished"
            );
            thread::sleep(Duration::from_millis(5));
        }
        producer.join().unwrap();
        assert!(
            backend.mixer.lock().unwrap().channels[&id]
                .samples
                .is_none()
        );
    }
}
