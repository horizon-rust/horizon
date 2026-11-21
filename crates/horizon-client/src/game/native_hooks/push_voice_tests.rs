#![cfg(test)]
//! `EffectsClientHost::push_voice`: each refusal of the mixer reaches a mod under its own name.
use super::push_voice::voice_error;
use horizon_client_render::audio::VoiceRefusal;
use mod_api::PushVoiceError;

#[test]
fn each_mixer_refusal_keeps_its_own_name() {
    let pairs = [
        (VoiceRefusal::Empty, PushVoiceError::Empty),
        (VoiceRefusal::BadRate, PushVoiceError::BadRate),
        (VoiceRefusal::NotFinite, PushVoiceError::NotFinite),
        (VoiceRefusal::Backlog, PushVoiceError::Backlog),
        (VoiceRefusal::TooManySources, PushVoiceError::TooManySources),
        (VoiceRefusal::NoOutput, PushVoiceError::Unavailable),
    ];
    for (refusal, named) in pairs {
        assert_eq!(voice_error(refusal), named);
    }
}
