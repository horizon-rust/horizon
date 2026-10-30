//! HD haptics: on USB the DualSense is also a four-channel 48 kHz audio device, whose channels 3
//! and 4 drive the left and right voice-coil actuators (1 and 2 are its speaker and headset).
//! Vibrations are synthesized as waveforms on those two channels; the first two stay silent.
//! Over Bluetooth there is no audio device, and the motors' emulation is used instead.
use horizon_openal::OutputStream;
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const RATE: u32 = 48_000;
/// The frequencies the heavy and light parts of a vibration play at.
pub const HEAVY_HZ: f32 = 80.0;
pub const LIGHT_HZ: f32 = 200.0;
/// Fades between steps, so a level change doesn't click.
const RAMP: f32 = 0.004;

/// The pad's audio output, playing the queued samples (left, right actuator).
pub struct Haptics {
    queue: Arc<Mutex<VecDeque<[f32; 2]>>>,
    _stream: OutputStream,
}

impl Haptics {
    /// The DualSense's audio device, if one is there with its four channels (USB).
    pub fn open() -> Option<Haptics> {
        let devices = horizon_openal::output_devices().ok()?;
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let source = queue.clone();
        let stream = devices
            .into_iter()
            .filter(|name| is_dualsense_name(name))
            .find_map(|name| {
                let source = source.clone();
                match OutputStream::open_quad(&name, move |data, rate| {
                    if rate != RATE {
                        data.fill(0.0);
                        return;
                    }
                    if let Ok(mut queue) = source.try_lock() {
                        for frame in data.chunks_exact_mut(4) {
                            let [left, right] = queue.pop_front().unwrap_or_default();
                            frame.copy_from_slice(&[0.0, 0.0, left, right]);
                        }
                    } else {
                        data.fill(0.0);
                    }
                }) {
                    Ok(stream) if stream.sample_rate() == RATE => Some(stream),
                    Ok(_) => None,
                    Err(error) => {
                        log::info!("Horizon: DualSense haptics unavailable on {name}: {error}");
                        None
                    }
                }
            })?;
        log::info!("DualSense HD haptics through its audio device");
        Some(Haptics {
            queue,
            _stream: stream,
        })
    }

    /// Play a vibration (heavy and light levels, 0..1, each for a while), replacing the one
    /// playing.
    pub fn play(&self, steps: &[(f32, f32, Duration)]) {
        let samples = waveform(steps);
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        queue.clear();
        queue.extend(samples.into_iter().map(|v| [v, v]));
    }

    /// Stop at once.
    pub fn stop(&self) {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// "DualSense Wireless Controller" on macOS; on Windows the endpoint is often just "Speakers
/// (Wireless Controller)" (DS5Windows' and dse-haptics' setup notes), a name the DualShock 4's
/// two-channel headset output shares, which the four channels rule out; ALSA names the card
/// "Controller".
fn is_dualsense_name(name: &str) -> bool {
    name.contains("DualSense")
        || name.contains("Wireless Controller")
        || name.contains("CARD=Controller")
}

/// The samples of a vibration: the heavy and light tones at each step's levels, with short
/// ramps between steps and to silence at the end.
pub fn waveform(steps: &[(f32, f32, Duration)]) -> Vec<f32> {
    let ramp = (RAMP * RATE as f32) as usize;
    let mut out = Vec::new();
    let mut level = [0.0f32; 2];
    let mut t = 0usize;
    for &(heavy, light, length) in steps.iter().chain(&[(0.0, 0.0, Duration::ZERO)]) {
        let target = [heavy.clamp(0.0, 1.0), light.clamp(0.0, 1.0)];
        let from = level;
        let n = (length.as_secs_f32() * RATE as f32) as usize;
        for i in 0..n.max(ramp) {
            let k = (i as f32 / ramp as f32).min(1.0);
            level = [0, 1].map(|j| from[j] + (target[j] - from[j]) * k);
            let time = t as f32 / RATE as f32;
            let v =
                level[0] * (TAU * HEAVY_HZ * time).sin() + level[1] * (TAU * LIGHT_HZ * time).sin();
            out.push(v.clamp(-1.0, 1.0));
            t += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_names() {
        assert!(is_dualsense_name("DualSense Wireless Controller"));
        assert!(is_dualsense_name("Speakers (Wireless Controller)"));
        assert!(is_dualsense_name(
            "Speakers (2- DualSense Wireless Controller)"
        ));
        assert!(is_dualsense_name("plughw:CARD=Controller,DEV=0"));
        assert!(!is_dualsense_name("MacBook Pro Speakers"));
        assert!(!is_dualsense_name("hw:CARD=PCH,DEV=0"));
    }

    #[test]
    fn waveform_follows_the_steps() {
        let w = waveform(&[(1.0, 0.0, Duration::from_millis(100))]);
        // The step, then the ramp down.
        assert_eq!(w.len(), 4800 + 192);
        // Starts silent, reaches full strength, ends silent.
        assert_eq!(w[0], 0.0);
        let peak = w[1000..4800].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.99, "{peak}");
        assert!(w.last().unwrap().abs() < 0.01);
        assert!(w.iter().all(|v| v.abs() <= 1.0));
        assert!(waveform(&[]).iter().all(|v| *v == 0.0));
    }
}
