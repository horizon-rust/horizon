pub use client::voice::{PushVoiceError, VoiceSink, voice_sink};
mod __private {
    pub mod game {
        pub use crate::client::voice::{VoiceOutput, install_voice_output};
    }
}
