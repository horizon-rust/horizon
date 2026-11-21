//! `EffectsClientHost::push_voice` and the sink for a mod's own threads: a client mod's decoded
//! audio into the window's mixer.
use super::*;
use horizon_client_render::audio::{Audio, VoiceHandle, VoiceRefusal};
use mod_api::PushVoiceError;

impl Game<'_> {
    pub(in crate::game) fn push_voice(
        &self,
        mod_id: &str,
        source: u128,
        samples: &[f32],
        rate: u32,
        at: Option<[f64; 3]>,
    ) -> Result<(), PushVoiceError> {
        // A server thread's `Game` has no output of its own: its audio is silent.
        if !self.native_window() {
            return Err(PushVoiceError::Unavailable);
        }
        self.audio
            .push_voice(mod_id, source, samples, rate, at)
            .map_err(voice_error)
    }
}

/// The mod API's name for a refusal of the mixer.
pub(super) fn voice_error(refusal: VoiceRefusal) -> PushVoiceError {
    match refusal {
        VoiceRefusal::Empty => PushVoiceError::Empty,
        VoiceRefusal::BadRate => PushVoiceError::BadRate,
        VoiceRefusal::NotFinite => PushVoiceError::NotFinite,
        VoiceRefusal::Backlog => PushVoiceError::Backlog,
        VoiceRefusal::TooManySources => PushVoiceError::TooManySources,
        VoiceRefusal::NoOutput => PushVoiceError::Unavailable,
    }
}

/// The window's mixer as the mod API's [`mod_api::__private::game::VoiceOutput`]: what
/// `voice_sink` reaches from a mod's network thread.
struct WindowVoice(VoiceHandle);

impl mod_api::__private::game::VoiceOutput for WindowVoice {
    fn push(
        &self,
        mod_id: &str,
        source: u128,
        samples: &[f32],
        rate: u32,
        at: Option<[f64; 3]>,
    ) -> Result<(), PushVoiceError> {
        self.0
            .push(mod_id, source, samples, rate, at)
            .map_err(voice_error)
    }
}

/// Hands mods' `voice_sink` the window's mixer; a later window's replaces it.
pub(in crate::game) fn install_voice_output(audio: &Audio) {
    mod_api::__private::game::install_voice_output(std::sync::Arc::new(WindowVoice(
        audio.voice_handle(),
    )));
}
