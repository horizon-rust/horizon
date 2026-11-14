//! Aiming with the eyes in VR (Horizon's own; docs/vr.md): the block the player looks at is
//! the one they target. A streamed headset gives the gaze coarsely (the Vision Pro through
//! KRVR: steps of about 12 degrees), so the gaze is smoothed a little, a target is kept while
//! the gaze stays near it, and a new one is taken only once the gaze has rested on it for a
//! moment: the outline never flickers between blocks.

/// How fast the smoothed gaze follows the eyes (seconds to cover about 63% of a jump).
const SMOOTHING: f32 = 0.04;
/// The target is kept while the gaze is within this angle of it (degrees): a bit more than
/// half the Vision Pro's step, so the next step lets it go.
const HOLD: f64 = 7.0;
/// How long the gaze rests on a new block before it becomes the target (seconds).
const DWELL: f32 = 0.12;

/// A block a ray meets: its position and where the ray enters it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub pos: [i32; 3],
    pub point: [f64; 3],
}

/// What the player aims with this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Aim {
    /// No usable gaze: the head aims (the player's yaw and pitch).
    #[default]
    Head,
    /// The eyes aim along this unit direction (world space, from the player's eye).
    Eyes([f64; 3]),
    /// The eyes are on no block (the sky, too far): nothing is targeted.
    Nothing,
}

/// The eyes' aim between frames.
#[derive(Clone, Debug, Default)]
pub struct GazeAim {
    /// The gaze, smoothed (world space, unit).
    smoothed: Option<[f64; 3]>,
    /// The block targeted now; `None` while the gaze rests on nothing.
    target: Option<Target>,
    /// What the gaze moved onto (a block or nothing) and for how long.
    pending: Option<(Option<[i32; 3]>, f32)>,
}

fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = v.iter().map(|c| c * c).sum::<f64>().sqrt();
    (length > 1e-9).then(|| v.map(|c| c / length))
}

fn angle(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dot: f64 = (0..3).map(|i| a[i] * b[i]).sum();
    dot.clamp(-1.0, 1.0).acos().to_degrees()
}

impl GazeAim {
    /// Once a frame, `seconds` after the last: the eye's position and the gaze (world space,
    /// unit; `None` without a valid gaze), and `cast`, the block a ray from the eye along a
    /// unit direction meets within reach.
    pub fn update(
        &mut self,
        seconds: f32,
        eye: [f64; 3],
        gaze: Option<[f64; 3]>,
        cast: impl Fn([f64; 3]) -> Option<Target>,
    ) -> Aim {
        let Some(gaze) = gaze.and_then(normalize) else {
            *self = Self::default();
            return Aim::Head;
        };
        let fresh = self.smoothed.is_none();
        let smoothed = match self.smoothed {
            Some(s) => {
                let a = f64::from(1.0 - (-seconds.max(0.0) / SMOOTHING).exp());
                normalize(std::array::from_fn(|i| s[i] + (gaze[i] - s[i]) * a)).unwrap_or(gaze)
            }
            None => gaze,
        };
        self.smoothed = Some(smoothed);
        let towards = |t: Target| normalize(std::array::from_fn(|i| t.point[i] - eye[i]));
        // The target, while it is still there: a ray towards it still meets it.
        let held = self.target.and_then(|t| {
            let dir = towards(t)?;
            cast(dir).is_some_and(|hit| hit.pos == t.pos).then_some(dir)
        });
        if let Some(dir) = held
            && angle(dir, smoothed) < HOLD
        {
            self.pending = None;
            return Aim::Eyes(dir);
        }
        let candidate = cast(smoothed);
        let key = candidate.map(|c| c.pos);
        let rested = match &mut self.pending {
            Some((pending, time)) if *pending == key => {
                *time += seconds;
                *time >= DWELL
            }
            _ => {
                self.pending = Some((key, 0.0));
                false
            }
        };
        // The gaze's first frame takes what it is on, and a target that is gone (broken,
        // hidden) gives way at once; otherwise the gaze must rest on the new block (or on
        // nothing) first. Resting elsewhere on the target's block moves the point it is held
        // from (moving it at once would drag it along behind a jump and hold too long).
        let gone = self.target.is_some() && held.is_none();
        if fresh || gone || rested {
            self.target = candidate;
            self.pending = None;
        }
        match self.target.and_then(towards) {
            Some(dir) => Aim::Eyes(dir),
            None => Aim::Nothing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wall of blocks across z = -4 (x from -10 to 10, y from 0 to 3), seen from the eye at
    /// (0.5, 1.6, 0.5): the ray's block and where it enters it.
    fn wall(dir: [f64; 3]) -> Option<Target> {
        let eye = [0.5, 1.6, 0.5];
        if dir[2] >= -1e-6 {
            return None;
        }
        let t = (-4.0 + 1.0 - eye[2]) / dir[2];
        let point: [f64; 3] = std::array::from_fn(|i| eye[i] + dir[i] * t);
        let pos = [point[0].floor() as i32, point[1].floor() as i32, -4];
        ((-10..=10).contains(&pos[0]) && (0..=3).contains(&pos[1]) && t <= 5.0)
            .then_some(Target { pos, point })
    }

    /// The direction yaw degrees to the right of -z, and pitch up.
    fn look(yaw: f64, pitch: f64) -> [f64; 3] {
        let (y, p) = (yaw.to_radians(), pitch.to_radians());
        [y.sin() * p.cos(), p.sin(), -y.cos() * p.cos()]
    }

    fn block(aim: Aim) -> Option<[i32; 3]> {
        match aim {
            Aim::Eyes(dir) => wall(dir).map(|t| t.pos),
            _ => None,
        }
    }

    const EYE: [f64; 3] = [0.5, 1.6, 0.5];
    const FRAME: f32 = 1.0 / 90.0;

    #[test]
    fn without_a_gaze_the_head_aims() {
        let mut aim = GazeAim::default();
        assert_eq!(aim.update(FRAME, EYE, None, wall), Aim::Head);
        aim.update(FRAME, EYE, Some(look(0.0, 0.0)), wall);
        // The gaze lost again: back to the head, and nothing is remembered.
        assert_eq!(aim.update(FRAME, EYE, None, wall), Aim::Head);
        assert!(aim.target.is_none() && aim.smoothed.is_none());
    }

    #[test]
    fn the_first_block_looked_at_is_targeted_at_once_and_the_sky_is_nothing() {
        let mut aim = GazeAim::default();
        let first = aim.update(FRAME, EYE, Some(look(0.0, 0.0)), wall);
        assert_eq!(block(first), Some([0, 1, -4]));
        let mut sky = GazeAim::default();
        assert_eq!(
            sky.update(FRAME, EYE, Some(look(0.0, 60.0)), wall),
            Aim::Nothing
        );
    }

    #[test]
    fn a_gaze_that_jumps_in_12_degree_steps_moves_the_target_once_per_step_without_flicker() {
        let mut aim = GazeAim::default();
        // The eyes sweep right in the Vision Pro's steps: 0, 12, 24 degrees, half a second each.
        let mut seen = Vec::new();
        for step in 0..3 {
            for _ in 0..45 {
                let a = aim.update(FRAME, EYE, Some(look(12.0 * f64::from(step), 0.0)), wall);
                let b = block(a);
                if seen.last() != Some(&b) {
                    seen.push(b);
                }
            }
        }
        // One target per step: the blocks the smoothed gaze crosses on the way never show.
        assert_eq!(
            seen,
            vec![Some([0, 1, -4]), Some([1, 1, -4]), Some([2, 1, -4])],
            "{seen:?}"
        );
    }

    #[test]
    fn small_wobbles_and_a_single_stray_sample_keep_the_target() {
        let mut aim = GazeAim::default();
        for _ in 0..30 {
            aim.update(FRAME, EYE, Some(look(0.0, 0.0)), wall);
        }
        for i in 0..90 {
            // A wobble of 4 degrees, and one frame 12 degrees off.
            let yaw = if i == 40 {
                12.0
            } else {
                4.0 * (f64::from(i) * 0.3).sin()
            };
            let a = aim.update(FRAME, EYE, Some(look(yaw, 0.0)), wall);
            assert_eq!(block(a), Some([0, 1, -4]), "frame {i}");
        }
    }

    #[test]
    fn looking_off_the_wall_lets_the_target_go_only_after_resting_there() {
        let mut aim = GazeAim::default();
        for _ in 0..30 {
            aim.update(FRAME, EYE, Some(look(0.0, 0.0)), wall);
        }
        let mut frames = 0;
        while aim.update(FRAME, EYE, Some(look(0.0, 60.0)), wall) != Aim::Nothing {
            frames += 1;
            assert!(frames < 90, "never let go");
        }
        let seconds = frames as f32 * FRAME;
        assert!(seconds >= DWELL, "let go after {seconds} s");
    }

    #[test]
    fn a_target_that_goes_away_gives_way_at_once() {
        let mut aim = GazeAim::default();
        for _ in 0..30 {
            aim.update(FRAME, EYE, Some(look(0.0, 0.0)), wall);
        }
        // The block looked at is broken: the ray goes on to nothing behind it.
        let broken = |dir: [f64; 3]| wall(dir).filter(|t| t.pos != [0, 1, -4]);
        assert_eq!(
            aim.update(FRAME, EYE, Some(look(0.0, 0.0)), broken),
            Aim::Nothing
        );
    }
}
