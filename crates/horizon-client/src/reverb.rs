//! The mixer's side of sound physics (`sound_physics`): the EFX parts Sound
//! Physics Remastered drives in OpenAL Soft, rebuilt for Horizon's mixer. A
//! channel's direct path and its four sends go through an EFX-style low-pass
//! (`AL_LOWPASS_GAIN`, and `AL_LOWPASS_GAINHF` above 5 kHz) into four shared
//! reverb buses, one per auxiliary effect slot, with the mod's EAX reverb
//! presets (decay 0.15, 0.55, 1.68 and 4.14 seconds).
//!
//! The buses are a feedback delay network, not OpenAL Soft's EAX reverb: the
//! decay times, high-frequency decay, reflections and late delays follow the
//! presets, the exact colour of OpenAL Soft's reverb does not.
use crate::sound_physics::Settings;

/// The reference frequency of EFX's low-pass `GAINHF` (`LOWPASSFREQREF`).
pub const HF_REFERENCE: f32 = 5000.0;
/// OpenAL Soft's air absorption: the high-frequency gain kept per metre
/// (`AIRABSORBGAINHF`) at an air absorption factor of 1.
pub const AIR_ABSORPTION_HF: f32 = 0.994_26;
pub const SENDS: usize = 4;

/// The crossover all of a channel's filters share: they differ only in
/// their gains, so one split feeds the direct path and every send. A
/// second-order Butterworth low-pass at `HF_REFERENCE` and its complement, so
/// a gain on the part above tends to that gain at high frequencies as EFX's
/// shelf does.
#[derive(Clone, Copy, Debug, Default)]
pub struct Split {
    z1: f32,
    z2: f32,
}
/// The crossover's biquad coefficients for one output rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crossover {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}
impl Split {
    /// The crossover at `HF_REFERENCE` for `rate` (RBJ low-pass, Q 1/sqrt 2).
    pub fn coefficient(rate: u32) -> Crossover {
        let w = std::f32::consts::TAU * HF_REFERENCE.min(rate as f32 * 0.45) / rate as f32;
        let alpha = w.sin() / std::f32::consts::SQRT_2;
        let cos = w.cos();
        let a0 = 1.0 + alpha;
        Crossover {
            b0: (1.0 - cos) / 2.0 / a0,
            b1: (1.0 - cos) / a0,
            b2: (1.0 - cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
        }
    }
    /// `x` as (below, above) the crossover; they add back to `x`.
    #[inline]
    pub fn split(&mut self, x: f32, c: Crossover) -> (f32, f32) {
        let low = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * low + self.z2;
        self.z2 = c.b2 * x - c.a2 * low;
        (low, x - low)
    }
}
/// An EFX low-pass on a split sample: `gain` overall, `gain_hf` more above
/// the reference frequency.
#[inline]
pub fn lowpass((low, high): (f32, f32), gain: f32, gain_hf: f32) -> f32 {
    gain * (low + gain_hf * high)
}

/// The EAX reverb parameters of one effect slot (`ReverbParams`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub decay_time: f32,
    pub density: f32,
    pub diffusion: f32,
    pub gain: f32,
    pub gain_hf: f32,
    pub decay_hf_ratio: f32,
    pub reflections_gain: f32,
    pub reflections_delay: f32,
    pub late_gain: f32,
    pub late_delay: f32,
    pub room_rolloff: f32,
}
/// The mod's four presets under `settings` (`ReverbParams.getReverb0..3`).
pub fn presets(settings: &Settings) -> [Params; SENDS] {
    let global = 0.7 * settings.reverb_gain * 0.85;
    let bright = settings.reverb_brightness;
    let attenuation = settings.attenuation_factor;
    let preset = |decay_time,
                  density,
                  gain,
                  gain_hf,
                  hf,
                  reflections_gain,
                  reflections_delay,
                  late_delay,
                  rolloff: f32| Params {
        decay_time,
        density,
        diffusion: 1.0,
        gain: gain * global,
        gain_hf,
        decay_hf_ratio: hf * bright,
        reflections_gain,
        reflections_delay,
        late_gain: 1.26,
        late_delay,
        room_rolloff: rolloff * attenuation,
    };
    [
        preset(0.15, 0.0, 0.2, 0.99, 0.6, 2.5, 0.001, 0.011, 0.16),
        preset(0.55, 0.0, 0.3, 0.99, 0.7, 0.2, 0.015, 0.011, 0.15),
        preset(1.68, 0.1, 0.5, 0.99, 0.7, 0.0, 0.021, 0.021, 0.13),
        preset(4.142, 0.5, 0.4, 0.89, 0.7, 0.0, 0.025, 0.021, 0.11),
    ]
}
/// OpenAL's room rolloff for the linear distance model (reference distance
/// 0): the wet path fades with `room_rolloff` instead of the source's rolloff.
pub fn room_attenuation(distance: f32, max_distance: Option<f32>, room_rolloff: f32) -> f32 {
    match max_distance {
        Some(max) if max > 0.0 => (1.0 - room_rolloff * distance.min(max) / max).clamp(0.0, 1.0),
        Some(_) => 0.0,
        None => 1.0,
    }
}

/// A circular delay line.
#[derive(Clone, Debug)]
struct Delay {
    buffer: Vec<f32>,
    at: usize,
}
impl Delay {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; length.max(1)],
            at: 0,
        }
    }
    /// The sample written `delay` samples ago (1..=length).
    #[inline]
    fn tap(&self, delay: usize) -> f32 {
        let delay = delay.min(self.buffer.len());
        let i = if self.at >= delay {
            self.at - delay
        } else {
            self.at + self.buffer.len() - delay
        };
        self.buffer[i]
    }
    /// The oldest sample, the one the next `push` replaces.
    #[inline]
    fn oldest(&self) -> f32 {
        self.buffer[self.at]
    }
    #[inline]
    fn push(&mut self, x: f32) {
        self.buffer[self.at] = x;
        self.at += 1;
        if self.at == self.buffer.len() {
            self.at = 0;
        }
    }
    fn clear(&mut self) {
        self.buffer.fill(0.0);
    }
}
/// The late reverb's delay lengths at 44.1 kHz (mutually prime, 23 to 69 ms).
const LINES: [usize; 8] = [1031, 1327, 1523, 1871, 2083, 2423, 2729, 3061];
/// Early reflection taps, as fractions past the reflections delay.
const EARLY: [(f32, f32); 4] = [(0.0, 1.0), (0.0043, -0.8), (0.0091, 0.6), (0.0137, -0.45)];

/// One reverb bus: mono in, stereo out.
#[derive(Clone, Debug)]
pub struct Bus {
    params: Params,
    rate: u32,
    input_split: Split,
    /// Predelay holding the input for the reflections and the late reverb.
    pre: Delay,
    diffusers: [Delay; 2],
    lines: Vec<Delay>,
    lengths: Vec<usize>,
    damping: Vec<Split>,
    /// Per line: (feedback gain, high-frequency gain relative to it).
    feedback: Vec<(f32, f32)>,
    coefficient: Crossover,
    /// Samples since the input was last non-silent.
    quiet: usize,
}
impl Bus {
    pub fn new(params: Params, rate: u32) -> Self {
        let scale = rate as f32 / 44_100.0 * (0.4 + 0.6 * params.density.clamp(0.0, 1.0));
        let lengths: Vec<usize> = LINES
            .iter()
            .map(|&l| ((l as f32 * scale) as usize).max(16))
            .collect();
        let pre_len = ((0.3 + 0.1 + 0.02) * rate as f32) as usize;
        let mut bus = Self {
            params,
            rate,
            input_split: Split::default(),
            pre: Delay::new(pre_len),
            diffusers: [
                Delay::new((0.0047 * rate as f32) as usize),
                Delay::new((0.0031 * rate as f32) as usize),
            ],
            lines: lengths.iter().map(|&l| Delay::new(l)).collect(),
            lengths,
            damping: vec![Split::default(); LINES.len()],
            feedback: Vec::new(),
            coefficient: Split::coefficient(rate),
            quiet: usize::MAX,
        };
        bus.set_params(params);
        bus
    }
    /// New parameters (the line lengths stay: density only shapes a new bus).
    pub fn set_params(&mut self, params: Params) {
        self.params = params;
        let rate = self.rate as f32;
        let decay = params.decay_time.max(0.1);
        let decay_hf = decay * params.decay_hf_ratio.clamp(0.1, 1.0);
        self.feedback = self
            .lengths
            .iter()
            .map(|&l| {
                let g = 10f32.powf(-3.0 * l as f32 / (rate * decay));
                let g_hf = 10f32.powf(-3.0 * l as f32 / (rate * decay_hf));
                (g, g_hf / g)
            })
            .collect();
    }
    pub fn params(&self) -> Params {
        self.params
    }
    /// Silent: nothing came in for longer than the tail lasts.
    pub fn idle(&self) -> bool {
        self.quiet as f32 > (self.params.decay_time + 0.5) * self.rate as f32
    }
    pub fn clear(&mut self) {
        self.pre.clear();
        self.diffusers.iter_mut().for_each(Delay::clear);
        self.lines.iter_mut().for_each(Delay::clear);
        self.damping.fill(Split::default());
        self.input_split = Split::default();
        self.quiet = usize::MAX;
    }
    /// Run `input` (mono, one sample a frame) and add the reverb to `out`
    /// (`channels` interleaved; mono outputs get both sides).
    pub fn process(&mut self, input: &[f32], out: &mut [f32], channels: usize) {
        if input.iter().any(|x| *x != 0.0) {
            self.quiet = 0;
        } else if self.idle() {
            return;
        }
        let p = self.params;
        let rate = self.rate as f32;
        let reflections = (p.reflections_delay * rate) as usize + 1;
        let late = reflections + (p.late_delay * rate) as usize;
        let early_taps = EARLY.map(|(t, g)| (reflections + (t * rate) as usize, g));
        let diffusion = 0.6 * p.diffusion.clamp(0.0, 1.0);
        let n = self.lines.len();
        let norm = (2.0 / n as f32).sqrt();
        let mut taps = [0.0f32; 8];
        for (x, frame) in input.iter().zip(out.chunks_mut(channels.max(1))) {
            let x = lowpass(
                self.input_split.split(*x, self.coefficient),
                p.gain,
                p.gain_hf,
            );
            self.pre.push(x);
            let mut early = [0.0f32; 2];
            for (i, &(delay, g)) in early_taps.iter().enumerate() {
                early[i & 1] += self.pre.tap(delay) * g;
            }
            // Two Schroeder all-passes smear the late input.
            let mut d = self.pre.tap(late);
            for ap in &mut self.diffusers {
                let delayed = ap.tap(ap.buffer.len());
                let v = d + diffusion * delayed;
                ap.push(v);
                d = delayed - diffusion * v;
            }
            for (tap, line) in taps.iter_mut().zip(&self.lines) {
                *tap = line.oldest();
            }
            // Householder feedback: every line feeds every other.
            let sum: f32 = taps[..n].iter().sum::<f32>() * (2.0 / n as f32);
            let mut wet = [0.0f32; 2];
            for i in 0..n {
                let (g, hf) = self.feedback[i];
                let mixed = taps[i] - sum;
                let damped = lowpass(self.damping[i].split(mixed, self.coefficient), g, hf);
                self.lines[i].push(damped + d);
                wet[i & 1] += taps[i];
            }
            let left = early[0] * p.reflections_gain * 0.25 + wet[0] * p.late_gain * norm * 0.5;
            let right = early[1] * p.reflections_gain * 0.25 + wet[1] * p.late_gain * norm * 0.5;
            if channels >= 2 {
                frame[0] += left;
                frame[1] += right;
            } else {
                frame[0] += 0.5 * (left + right);
            }
        }
        self.quiet = self.quiet.saturating_add(input.len());
        if input.iter().rev().take(1).any(|x| *x != 0.0) {
            self.quiet = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy(s: &[f32]) -> f32 {
        s.iter().map(|x| x * x).sum()
    }
    /// The time the energy of an impulse response takes to fall 60 dB
    /// (Schroeder integration, from the -5 to the -25 dB points).
    fn t60(params: Params, rate: u32) -> f32 {
        let mut bus = Bus::new(params, rate);
        let len = (rate as f32 * (params.decay_time * 3.0 + 0.5)) as usize;
        let mut input = vec![0.0; len];
        input[0] = 1.0;
        let mut out = vec![0.0; len * 2];
        bus.process(&input, &mut out, 2);
        let mono: Vec<f32> = out.chunks(2).map(|f| f[0] + f[1]).collect();
        let mut remaining: Vec<f32> = mono
            .iter()
            .rev()
            .scan(0.0, |acc, x| {
                *acc += x * x;
                Some(*acc)
            })
            .collect();
        remaining.reverse();
        let total = remaining[0];
        let at = |db: f32| {
            remaining
                .iter()
                .position(|&e| 10.0 * (e / total).log10() < db)
                .unwrap()
        };
        let (a, b) = (at(-5.0), at(-25.0));
        (b - a) as f32 / rate as f32 * 3.0
    }

    #[test]
    fn lowpass_keeps_lows_and_scales_highs() {
        let rate = 48_000;
        let c = Split::coefficient(rate);
        let run = |freq: f32, gain_hf: f32| {
            let mut split = Split::default();
            let out: Vec<f32> = (0..rate)
                .map(|i| {
                    let x = (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin();
                    lowpass(split.split(x, c), 1.0, gain_hf)
                })
                .collect();
            (energy(&out[rate as usize / 2..]) / (rate as f32 / 4.0)).sqrt()
        };
        assert!((run(100.0, 0.1) - 1.0).abs() < 0.03);
        assert!(run(15_000.0, 0.1) < 0.25);
        // gain_hf 1 is transparent at every frequency.
        assert!((run(15_000.0, 1.0) - 1.0).abs() < 1e-3);
        let mut split = Split::default();
        let (lo, hi) = split.split(0.7, c);
        assert!((lo + hi - 0.7).abs() < 1e-7);
    }

    #[test]
    fn presets_decay_as_the_mod_sets_them() {
        let presets = presets(&Settings::default());
        assert_eq!(presets.map(|p| p.decay_time), [0.15, 0.55, 1.68, 4.142]);
        for p in presets {
            let measured = t60(p, 48_000);
            assert!(
                (measured / p.decay_time - 1.0).abs() < 0.35,
                "decay {} measured {measured}",
                p.decay_time
            );
        }
        let louder = crate::reverb::presets(&Settings {
            reverb_gain: 2.0,
            ..Settings::default()
        });
        assert!((louder[0].gain / presets[0].gain - 2.0).abs() < 1e-6);
    }

    #[test]
    fn idle_buses_cost_nothing_and_room_rolloff() {
        let p = presets(&Settings::default())[0];
        let mut bus = Bus::new(p, 48_000);
        assert!(bus.idle());
        let mut out = vec![0.0; 512];
        bus.process(&[0.0; 256], &mut out, 2);
        assert!(out.iter().all(|x| *x == 0.0));
        let mut input = [0.0; 256];
        input[0] = 1.0;
        bus.process(&input, &mut out, 2);
        assert!(!bus.idle());
        for _ in 0..2_000 {
            bus.process(&[0.0; 256], &mut out, 2);
        }
        assert!(bus.idle());
        assert_eq!(room_attenuation(8.0, Some(16.0), 0.16), 1.0 - 0.08);
        assert_eq!(room_attenuation(8.0, None, 0.16), 1.0);
    }
}
