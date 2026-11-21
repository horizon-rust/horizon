//! A client mod's audio in the game's mixer (`Host::push_voice`): decoded voice from a network
//! thread, placed in the world or heard centred.

use std::sync::{Arc, RwLock};

/// Why a mod's audio was not queued. Nothing was queued in any case; each cause has its own name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PushVoiceError {
    /// There were no samples.
    Empty,
    /// The sample rate was 0.
    BadRate,
    /// A sample or a coordinate of the position was NaN or infinite.
    NotFinite,
    /// The source already has two seconds waiting, or this one chunk is longer than that:
    /// a burst is refused rather than delayed, so a stalled sender cannot pile up latency.
    Backlog,
    /// Another source would make more than 64 play at once, across mods.
    TooManySources,
    /// There is no output device to play on: a dedicated server, no sound, or no game backend.
    Unavailable,
}

impl std::fmt::Display for PushVoiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "there are no samples",
            Self::BadRate => "the sample rate is 0",
            Self::NotFinite => "a sample or the position is not a finite number",
            Self::Backlog => "the source has more than two seconds waiting",
            Self::TooManySources => "more than 64 audio sources would play at once",
            Self::Unavailable => "there is no audio output",
        })
    }
}

impl std::error::Error for PushVoiceError {}

impl crate::NoBackend for PushVoiceError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// The game's mixer, as a native mod's own threads reach it. A mod gets it through
/// [`voice_sink`], not directly.
pub trait VoiceOutput: Send + Sync + 'static {
    /// As `Host::push_voice`, from any thread.
    fn push(
        &self,
        mod_id: &str,
        source: u128,
        samples: &[f32],
        rate: u32,
        at: Option<[f64; 3]>,
    ) -> Result<(), PushVoiceError>;
}

static OUTPUT: RwLock<Option<Arc<dyn VoiceOutput>>> = RwLock::new(None);

/// Install the client's mixer, replacing the one before it; a window's game does this once its
/// audio exists.
// hidden: engine plumbing, re-exported through __private for the game.
#[doc(hidden)]
pub fn install_voice_output(output: Arc<dyn VoiceOutput>) {
    if let Ok(mut slot) = OUTPUT.write() {
        *slot = Some(output);
    }
}

/// A mod's way to play decoded audio from any thread, such as the network thread that receives
/// voice, without handing the samples to the game's thread once per tick. Cheap to clone; it
/// keeps working across output device changes and reports [`PushVoiceError::Unavailable`] while
/// the game has no client audio (a dedicated server, a wasm mod, a test).
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: a native mod's own handle; it never crosses the wasm boundary.
#[derive(Clone, Debug)]
pub struct VoiceSink {
    mod_id: Arc<str>,
}

/// A [`VoiceSink`] for `mod_id`: the id a mod's sources are kept apart by, so two mods may use
/// the same source numbers. It is `Send + Sync`: keep it in the thread that decodes audio.
pub fn voice_sink(mod_id: &str) -> VoiceSink {
    VoiceSink {
        mod_id: mod_id.into(),
    }
}

impl VoiceSink {
    /// Queues mono `samples` (-1 to 1) sampled at `rate` Hz behind what `source` already has
    /// waiting, and plays them with the game's sounds, so the output device and the volume
    /// sliders apply. A speaker keeps one `source` (a player's UUID as a `u128`) and its chunks
    /// play in order; different sources play at the same time. `at` is a position in the
    /// client's level, heard with distance (silent at 48 blocks) and direction from the
    /// camera; `None` plays centred. Names its refusal as [`PushVoiceError`] does.
    pub fn push(
        &self,
        source: u128,
        samples: &[f32],
        rate: u32,
        at: Option<[f64; 3]>,
    ) -> Result<(), PushVoiceError> {
        let output = OUTPUT
            .read()
            .ok()
            .and_then(|slot| slot.clone())
            .ok_or(PushVoiceError::Unavailable)?;
        output.push(&self.mod_id, source, samples, rate, at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Record(Mutex<Vec<(String, u128, usize, u32, Option<[f64; 3]>)>>);
    impl VoiceOutput for Record {
        fn push(
            &self,
            mod_id: &str,
            source: u128,
            samples: &[f32],
            rate: u32,
            at: Option<[f64; 3]>,
        ) -> Result<(), PushVoiceError> {
            if rate == 0 {
                return Err(PushVoiceError::BadRate);
            }
            self.0
                .lock()
                .unwrap()
                .push((mod_id.to_owned(), source, samples.len(), rate, at));
            Ok(())
        }
    }

    #[test]
    fn a_sink_pushes_from_another_thread_into_the_installed_output() {
        // Not installed yet: named, not a panic.
        let sink = voice_sink("voice");
        assert_eq!(
            sink.push(1, &[0.5], 48_000, None),
            Err(PushVoiceError::Unavailable)
        );
        let record = Arc::new(Record::default());
        install_voice_output(record.clone());
        let sent = std::thread::spawn(move || {
            let first = sink.push(7, &[0.1; 960], 48_000, Some([1.0, 64.0, -2.5]));
            let second = sink.push(7, &[0.1; 4], 0, None);
            (first, second)
        })
        .join()
        .unwrap();
        assert_eq!(sent, (Ok(()), Err(PushVoiceError::BadRate)));
        assert_eq!(
            *record.0.lock().unwrap(),
            [("voice".to_owned(), 7, 960, 48_000, Some([1.0, 64.0, -2.5]))]
        );
    }
}
