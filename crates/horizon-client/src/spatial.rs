//! Horizon's spatial audio extras for the mixer (`audio`), beside sound
//! physics: binaural panning and the Doppler effect. Both are off by
//! default; vanilla's context has neither (`Library.createAttributes` turns
//! HRTF off, and no source ever gets a velocity).
//!
//! The binaural panning is a structural head model (Brown and Duda, "A
//! structural model for binaural sound synthesis", 1998), not measured
//! impulse responses: each ear gets the interaural delay of a sphere, its
//! head shadow (a one-pole, one-zero high shelf) and the pinna's echoes,
//! whose delays carry elevation and front from back. It needs no data
//! files and costs a few dozen operations a sample per voice.
//!
//! The arrow whoosh is noise through a band-pass, synthesised once and
//! looped: vanilla has no sound for an arrow in flight.
//!
//! The Doppler effect is OpenAL's (`AL_DOPPLER_FACTOR` 1, speed of sound
//! 343.3 blocks a second), from velocities measured off the positions the
//! game sets, since the game sets none.
use std::f32::consts::{FRAC_PI_2, PI};
use std::time::{Duration, Instant};

/// OpenAL's `AL_SPEED_OF_SOUND`, in blocks (metres) a second.
pub const SPEED_OF_SOUND: f32 = 343.3;
/// The head's radius, metres.
const HEAD_RADIUS: f32 = 0.0875;
/// The head shadow at its deepest (`alpha_min`) and where (`theta_min`).
const SHADOW_MIN: f32 = 0.1;
const SHADOW_ANGLE: f32 = 150.0 * PI / 180.0;
/// The pinna echoes: (reflection, A, B, D), delays in samples at 44.1 kHz.
const PINNA: [(f32, f32, f32, f32); 5] = [
    (0.5, 1.0, 2.0, 1.0),
    (-1.0, 5.0, 4.0, 0.5),
    (0.5, 5.0, 7.0, 0.5),
    (-0.25, 5.0, 11.0, 0.5),
    (0.25, 5.0, 13.0, 0.5),
];
/// How much of the pinna's echoes is mixed in (the model's 1 colours
/// every sound strongly; this keeps the cues and less of the comb).
const PINNA_MIX: f32 = 0.7;
/// The level the binaural ears play at against vanilla's panning, so a
/// sound ahead is as loud (vanilla gives it 0.596 each side).
pub const BINAURAL_GAIN: f32 = 0.6;
/// Input samples an ear can reach back: the longest delay plus the pinna's.
const HISTORY: usize = 128;

/// One ear's filters for a direction, at one output rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ear {
    /// The interaural delay, samples.
    delay: f32,
    /// The head shadow's high-frequency gain (2 facing the sound, 0.1 in
    /// the deepest shadow).
    shadow: f32,
    /// Each pinna echo's delay past `delay`, samples.
    pinna: [f32; 5],
}
/// The two ears (left, right) for a direction in listener space (right,
/// up, forward; any length) at `rate`.
pub fn ears(right: f32, up: f32, forward: f32, rate: u32) -> [Ear; 2] {
    let length = (right * right + up * up + forward * forward).sqrt();
    let (r, u, f) = if length <= f32::EPSILON {
        (0.0, 0.0, 1.0)
    } else {
        (right / length, up / length, forward / length)
    };
    let fs = rate as f32;
    let azimuth = r.atan2(f);
    let elevation = u.clamp(-1.0, 1.0).asin();
    let pinna = PINNA.map(|(_, a, b, d)| {
        (a * (azimuth / 2.0).cos() * (d * (FRAC_PI_2 - elevation)).sin() + b) * fs / 44_100.0
    });
    [-1.0f32, 1.0].map(|side| {
        // The angle between the source and this ear's axis.
        let incidence = (side * r).clamp(-1.0, 1.0).acos();
        let travel = if incidence < FRAC_PI_2 {
            1.0 - incidence.cos()
        } else {
            1.0 + incidence - FRAC_PI_2
        };
        Ear {
            delay: HEAD_RADIUS / SPEED_OF_SOUND * travel * fs,
            shadow: (1.0 + SHADOW_MIN / 2.0)
                + (1.0 - SHADOW_MIN / 2.0) * (incidence / SHADOW_ANGLE * PI).cos(),
            pinna,
        }
    })
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
/// A voice's binaural state: its recent input, the input through the
/// pinna's echoes (the same for both ears, which only read it at their own
/// delay) and each ear's shadow filter.
#[derive(Clone, Debug)]
pub struct Binaural {
    input: [f32; HISTORY],
    pinna: [f32; HISTORY],
    at: usize,
    /// Each shadow filter's last input and output.
    shadow: [(f32, f32); 2],
}
impl Default for Binaural {
    fn default() -> Self {
        Self {
            input: [0.0; HISTORY],
            pinna: [0.0; HISTORY],
            at: 0,
            shadow: [(0.0, 0.0); 2],
        }
    }
}
/// `history` `delay` samples before `at`, between samples.
#[inline]
fn tap(history: &[f32; HISTORY], at: usize, delay: f32) -> f32 {
    let delay = delay.clamp(0.0, (HISTORY - 2) as f32);
    let whole = delay as usize;
    let t = delay - whole as f32;
    let back = |n: usize| history[(at + HISTORY - n) & (HISTORY - 1)];
    back(whole) * (1.0 - t) + back(whole + 1) * t
}
const _: () = assert!(HISTORY.is_power_of_two());
impl Binaural {
    /// One sample in, the two ears out (unscaled), the ears' filters moved
    /// `t` (0 to 1) of the way from `from` to `to` (a buffer's ramp).
    #[inline]
    pub fn process(
        &mut self,
        x: f32,
        from: &[Ear; 2],
        to: &[Ear; 2],
        t: f32,
        rate: u32,
    ) -> [f32; 2] {
        let at = self.at;
        self.input[at] = x;
        let mut p = x;
        for (k, &(reflection, ..)) in PINNA.iter().enumerate() {
            let delay = lerp(from[0].pinna[k], to[0].pinna[k], t);
            p += PINNA_MIX * reflection * tap(&self.input, at, delay);
        }
        self.pinna[at] = p;
        let k = 2.0 * rate as f32;
        let beta = 2.0 * SPEED_OF_SOUND / HEAD_RADIUS;
        let inv = 1.0 / (k + beta);
        let a1 = (beta - k) * inv;
        let mut out = [0.0; 2];
        for ear in 0..2 {
            let v = tap(&self.pinna, at, lerp(from[ear].delay, to[ear].delay, t));
            // H(s) = (alpha s + beta) / (s + beta), bilinear.
            let alpha = lerp(from[ear].shadow, to[ear].shadow, t);
            let (b0, b1) = ((alpha * k + beta) * inv, (beta - alpha * k) * inv);
            let (x1, y1) = self.shadow[ear];
            let y = b0 * v + b1 * x1 - a1 * y1;
            self.shadow[ear] = (v, y);
            out[ear] = y;
        }
        self.at = (at + 1) & (HISTORY - 1);
        out
    }
}

/// The rate the whoosh is synthesised at.
pub const WHOOSH_RATE: u32 = 48_000;
/// Two seconds of the whoosh, looping without a seam: white noise through
/// a band-pass at 900 Hz with a slow flutter, its last quarter second
/// crossfaded into its start.
pub fn whoosh_samples() -> Vec<f32> {
    let rate = WHOOSH_RATE as f32;
    let (len, fade) = (2 * WHOOSH_RATE as usize, WHOOSH_RATE as usize / 4);
    // RBJ band-pass (constant peak gain), Q 0.8.
    let w = std::f32::consts::TAU * 900.0 / rate;
    let alpha = w.sin() / (2.0 * 0.8);
    let a0 = 1.0 + alpha;
    let (b0, b2, a1, a2) = (
        alpha / a0,
        -alpha / a0,
        -2.0 * w.cos() / a0,
        (1.0 - alpha) / a0,
    );
    let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut seed = 0x9e37_79b9u32;
    let raw: Vec<f32> = (0..len + fade)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let x = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
            let y = b0 * x + b2 * x2 - a1 * y1 - a2 * y2;
            (x2, x1, y2, y1) = (x1, x, y1, y);
            // The fletching's flutter.
            let flutter = 1.0 + 0.25 * (std::f32::consts::TAU * 11.0 * i as f32 / rate).sin();
            y * flutter
        })
        .collect();
    let mut out: Vec<f32> = raw[..len].to_vec();
    for i in 0..fade {
        let t = i as f32 / fade as f32;
        out[i] = out[i] * t + raw[len + i] * (1.0 - t);
    }
    let rms = (out.iter().map(|x| x * x).sum::<f32>() / out.len() as f32).sqrt();
    out.iter().map(|x| x * 0.3 / rms.max(1e-6)).collect()
}
/// The whoosh's gain and pitch for an arrow flying at `speed` blocks a
/// second: silent below 8, full at 48 (a bow drawn fully shoots at about 60).
pub fn whoosh(speed: f32) -> (f32, f32) {
    let gain = ((speed - 8.0) / 40.0).clamp(0.0, 1.0) * 0.6;
    let pitch = 0.7 + (speed / 60.0).min(1.5) * 0.6;
    (gain, pitch)
}

/// A velocity measured from the positions given, for the Doppler effect.
/// A position that jumps (a teleport, a sound placed anew) does not count,
/// and the velocity lapses when the positions stop coming.
#[derive(Clone, Copy, Debug, Default)]
pub struct Motion {
    last: Option<([f64; 3], Instant)>,
    velocity: [f32; 3],
    moved: Option<Instant>,
}
/// Faster than this between two positions is a jump, not motion.
const JUMP: f32 = 100.0;
/// A velocity older than this (no new position) is dropped.
const LAPSE: Duration = Duration::from_millis(150);
impl Motion {
    pub fn update(&mut self, at: [f64; 3], now: Instant) {
        if let Some((last, then)) = self.last {
            let dt = now.saturating_duration_since(then).as_secs_f32();
            if at == last {
                return;
            }
            if dt > 1e-3 {
                let v: [f32; 3] = std::array::from_fn(|a| ((at[a] - last[a]) as f32) / dt);
                let speed = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if speed < JUMP && dt < 0.5 {
                    // Half the new measure: positions come once a tick and jitter.
                    self.velocity = std::array::from_fn(|a| 0.5 * self.velocity[a] + 0.5 * v[a]);
                    self.moved = Some(now);
                } else {
                    self.velocity = [0.0; 3];
                }
            }
        }
        self.last = Some((at, now));
    }
    pub fn velocity(&self, now: Instant) -> [f32; 3] {
        match self.moved {
            Some(t) if now.saturating_duration_since(t) <= LAPSE => self.velocity,
            _ => [0.0; 3],
        }
    }
}
/// OpenAL's Doppler pitch factor: `source - listener` is the vector from the
/// listener to the source, the velocities in blocks a second.
pub fn doppler(
    source_minus_listener: [f32; 3],
    source: [f32; 3],
    listener: [f32; 3],
    factor: f32,
) -> f32 {
    let d = source_minus_listener;
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if length <= 1e-4 || factor <= 0.0 {
        return 1.0;
    }
    // Towards the listener: from the source to the listener.
    let towards = d.map(|x| -x / length);
    let dot = |v: [f32; 3]| v[0] * towards[0] + v[1] * towards[1] + v[2] * towards[2];
    let limit = SPEED_OF_SOUND / factor * 0.9;
    let (vls, vss) = (dot(listener).min(limit), dot(source).min(limit));
    ((SPEED_OF_SOUND - factor * vls) / (SPEED_OF_SOUND - factor * vss)).clamp(0.5, 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each ear's RMS level for noise played from a direction.
    fn levels(right: f32, up: f32, forward: f32, high: bool) -> [f32; 2] {
        let rate = 48_000;
        let e = ears(right, up, forward, rate);
        let mut b = Binaural::default();
        let mut seed = 1u32;
        let mut prev = 0.0;
        let mut sum = [0.0f32; 2];
        for i in 0..rate {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = (seed >> 8) as f32 / (1 << 24) as f32 - 0.5;
            // High-passed noise for the shadow, plain for the level.
            let x = if high { n - prev } else { n };
            prev = n;
            let out = b.process(x, &e, &e, 1.0, rate);
            if i > 1000 {
                sum[0] += out[0] * out[0];
                sum[1] += out[1] * out[1];
            }
        }
        sum.map(|s| (s / (rate - 1001) as f32).sqrt())
    }

    #[test]
    fn the_near_ear_hears_first_and_brighter() {
        let [left, right] = ears(1.0, 0.0, 0.0, 48_000);
        assert!(right.delay < 0.01, "{right:?}");
        // About 0.65 ms for the far ear.
        assert!((left.delay / 48.0 - 0.656).abs() < 0.02, "{left:?}");
        assert!(right.shadow > 1.9 && left.shadow < 0.4);
        let [l, r] = levels(1.0, 0.0, 0.0, true);
        assert!(r > 3.0 * l, "{l} {r}");
        // Ahead: both alike.
        let [l, r] = levels(0.0, 0.0, 1.0, true);
        assert!((l / r - 1.0).abs() < 0.05, "{l} {r}");
        let [front_l, _] = ears(0.0, 0.0, 1.0, 48_000);
        assert!(
            front_l.delay > 0.0
                && (front_l.delay - ears(0.0, 0.0, 1.0, 48_000)[1].delay).abs() < 1e-4
        );
    }

    #[test]
    fn pinna_echoes_tell_front_from_back_and_up_from_down() {
        let front = ears(0.0, 0.0, 1.0, 48_000)[0].pinna;
        let back = ears(0.0, 0.0, -1.0, 48_000)[0].pinna;
        let up = ears(0.0, 1.0, 0.1, 48_000)[0].pinna;
        assert_ne!(front, back);
        assert_ne!(front, up);
        // Behind, the echoes sit at their shortest delays (cos(azimuth / 2) = 0).
        assert!(
            (back[1] - 4.0 * 48_000.0 / 44_100.0).abs() < 1e-3,
            "{back:?}"
        );
        // Ahead, after BINAURAL_GAIN, about vanilla's 0.596 of the input
        // (uniform noise in -0.5..0.5: RMS 0.289).
        let [l, r] = levels(0.0, 0.0, 1.0, false);
        for ear in [l, r] {
            let ratio = ear * BINAURAL_GAIN / 0.2887;
            assert!((0.4..0.9).contains(&ratio), "{ratio}");
        }
    }

    #[test]
    fn doppler_rises_as_things_close_in() {
        // A source 10 blocks ahead coming at 34 blocks a second.
        let towards = doppler([0.0, 0.0, 10.0], [0.0, 0.0, -34.33], [0.0; 3], 1.0);
        assert!(
            (towards - 343.3 / (343.3 - 34.33)).abs() < 1e-4,
            "{towards}"
        );
        let away = doppler([0.0, 0.0, 10.0], [0.0, 0.0, 34.33], [0.0; 3], 1.0);
        assert!(away < 1.0);
        // The listener running at it.
        assert!(doppler([0.0, 0.0, 10.0], [0.0; 3], [0.0, 0.0, 10.0], 1.0) > 1.0);
        // Sideways: no shift.
        assert_eq!(
            doppler([0.0, 0.0, 10.0], [30.0, 0.0, 0.0], [0.0; 3], 1.0),
            1.0
        );
        assert_eq!(
            doppler([0.0, 0.0, 10.0], [0.0, 0.0, -34.0], [0.0; 3], 0.0),
            1.0
        );
    }

    #[test]
    fn the_whoosh_loops_without_a_seam_and_follows_speed() {
        let s = whoosh_samples();
        assert_eq!(s.len(), 2 * WHOOSH_RATE as usize);
        // The jump across the loop point is no larger than the sound's own steps.
        let steps = s
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(
            (s[0] - s[s.len() - 1]).abs() <= steps,
            "{} {}",
            s[0],
            s[s.len() - 1]
        );
        assert_eq!(whoosh(5.0).0, 0.0);
        assert!(whoosh(60.0).0 > whoosh(20.0).0);
        assert!(whoosh(60.0).1 > whoosh(20.0).1);
    }

    #[test]
    fn motion_is_measured_and_lapses() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.update([0.0; 3], t0);
        m.update([0.0, 0.0, 1.0], t0 + Duration::from_millis(50));
        let v = m.velocity(t0 + Duration::from_millis(60));
        assert!((v[2] - 10.0).abs() < 1e-3, "{v:?}");
        m.update([0.0, 0.0, 2.0], t0 + Duration::from_millis(100));
        assert!((m.velocity(t0 + Duration::from_millis(110))[2] - 15.0).abs() < 1e-3);
        // Stopped moving: lapses.
        assert_eq!(m.velocity(t0 + Duration::from_millis(400)), [0.0; 3]);
        // A teleport is no velocity.
        m.update([500.0, 0.0, 2.0], t0 + Duration::from_millis(150));
        assert_eq!(m.velocity(t0 + Duration::from_millis(160)), [0.0; 3]);
    }
}
