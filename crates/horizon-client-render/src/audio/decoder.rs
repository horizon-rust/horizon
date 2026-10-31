//! Bounded Ogg decoding shared by static buffers and streamed sounds.
use std::{fs::File, path::Path};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{Decoder as Codec, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader},
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

pub(super) const CHUNK_SAMPLES: usize = 16_384;

pub(super) struct Decoder {
    format: Box<dyn FormatReader>,
    codec: Box<dyn Codec>,
    track: u32,
    pub rate: u32,
}
impl Decoder {
    pub fn open(path: &Path) -> Option<Self> {
        let stream = MediaSourceStream::new(Box::new(File::open(path).ok()?), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("ogg");
        let format = symphonia::default::get_probe()
            .format(
                &hint,
                stream,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .ok()?
            .format;
        let track = format.default_track()?;
        let rate = track.codec_params.sample_rate?;
        let codec = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .ok()?;
        Some(Self {
            track: track.id,
            rate,
            codec,
            format,
        })
    }
    pub fn next_chunk(&mut self) -> Result<Vec<f32>, String> {
        let mut mono = Vec::with_capacity(CHUNK_SAMPLES);
        while mono.len() < CHUNK_SAMPLES {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.to_string()),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let buffer = match self.codec.decode(&packet) {
                Ok(buffer) => buffer,
                Err(Error::DecodeError(_)) => continue,
                Err(e) => return Err(e.to_string()),
            };
            let spec = *buffer.spec();
            if spec.rate != self.rate {
                return Err("sound sample rate changed".into());
            }
            let channels = spec.channels.count();
            if channels == 0 {
                return Err("sound has no channels".into());
            }
            let mut samples = SampleBuffer::<f32>::new(buffer.capacity() as u64, spec);
            samples.copy_interleaved_ref(buffer);
            mono.extend(
                samples
                    .samples()
                    .chunks_exact(channels)
                    .map(|frame| frame.iter().sum::<f32>() / channels as f32),
            );
        }
        Ok(mono)
    }
}
