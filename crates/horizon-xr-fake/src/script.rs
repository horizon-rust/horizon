//! The fake headset's script (`HORIZON_FAKE_XR_SCRIPT`): where the head is over time and
//! what each controller input reads.

use openxr_sys as sys;

pub const IDENTITY: sys::Posef = sys::Posef {
    orientation: sys::Quaternionf {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    },
    position: sys::Vector3f {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    },
};

/// The head standing at 1.6 m, looking down -z.
const HEAD: HeadKey = HeadKey {
    t: 0.0,
    yaw: 0.0,
    pitch: 0.0,
    position: [0.0, 1.6, 0.0],
};

#[derive(Clone, Copy, Debug, PartialEq)]
struct HeadKey {
    t: f32,
    yaw: f32,
    pitch: f32,
    position: [f32; 3],
}

#[derive(Default, Debug)]
pub struct Script {
    head: Vec<HeadKey>,
    /// (time, path, value), in time order: the last one at or before a time holds.
    inputs: Vec<(f32, String, [f32; 2])>,
}

impl Script {
    pub fn load() -> Self {
        std::env::var("HORIZON_FAKE_XR_SCRIPT")
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn parse(text: &str) -> Self {
        let mut script = Self::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let words: Vec<&str> = line.split_whitespace().collect();
            let num = |i: usize| words.get(i).and_then(|w| w.parse::<f32>().ok());
            let Some(t) = num(0) else {
                continue;
            };
            match words.get(1).copied() {
                Some("head") => {
                    let before = script.head.last().copied().unwrap_or(HEAD);
                    script.head.push(HeadKey {
                        t,
                        yaw: num(2).unwrap_or(before.yaw),
                        pitch: num(3).unwrap_or(before.pitch),
                        position: [
                            num(4).unwrap_or(before.position[0]),
                            num(5).unwrap_or(before.position[1]),
                            num(6).unwrap_or(before.position[2]),
                        ],
                    });
                }
                Some("input") => {
                    if let Some(path) = words.get(2) {
                        script.inputs.push((
                            t,
                            (*path).to_owned(),
                            [num(3).unwrap_or(0.0), num(4).unwrap_or(0.0)],
                        ));
                    }
                }
                _ => {}
            }
        }
        script.head.sort_by(|a, b| a.t.total_cmp(&b.t));
        script.inputs.sort_by(|a, b| a.0.total_cmp(&b.0));
        script
    }

    /// The head's pose (stage space) at `t` seconds.
    pub fn head(&self, t: f32) -> sys::Posef {
        let mut before = HEAD;
        let mut key = HEAD;
        for k in &self.head {
            if k.t <= t {
                before = *k;
                key = *k;
            } else {
                let f = ((t - before.t) / (k.t - before.t).max(1e-6)).clamp(0.0, 1.0);
                let lerp = |a: f32, b: f32| a + (b - a) * f;
                key = HeadKey {
                    t,
                    yaw: lerp(before.yaw, k.yaw),
                    pitch: lerp(before.pitch, k.pitch),
                    position: std::array::from_fn(|i| lerp(before.position[i], k.position[i])),
                };
                break;
            }
        }
        pose(key.yaw, key.pitch, key.position)
    }

    /// A binding path's value at `t`, when the script sets it.
    pub fn input(&self, path: &str, t: f32) -> Option<[f32; 2]> {
        self.inputs
            .iter()
            .rev()
            .find(|(at, p, _)| *at <= t && p == path)
            .map(|(_, _, v)| *v)
    }
}

/// Yaw about +y then pitch about +x (degrees), at `position`.
fn pose(yaw: f32, pitch: f32, position: [f32; 3]) -> sys::Posef {
    let (sy, cy) = (yaw.to_radians() * 0.5).sin_cos();
    let (sp, cp) = (pitch.to_radians() * 0.5).sin_cos();
    // q = q_yaw * q_pitch
    sys::Posef {
        orientation: sys::Quaternionf {
            x: cy * sp,
            y: sy * cp,
            z: -sy * sp,
            w: cy * cp,
        },
        position: sys::Vector3f {
            x: position[0],
            y: position[1],
            z: position[2],
        },
    }
}

fn rotate(q: sys::Quaternionf, v: [f32; 3]) -> [f32; 3] {
    // v' = v + 2w(q x v) + 2 q x (q x v)
    let u = [q.x, q.y, q.z];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let t = cross(u, v).map(|c| c * 2.0);
    let c = cross(u, t);
    std::array::from_fn(|i| v[i] + q.w * t[i] + c[i])
}

/// `pose` moved by `local` along its own axes (an eye from the head).
pub fn offset(pose: sys::Posef, local: [f32; 3]) -> sys::Posef {
    let d = rotate(pose.orientation, local);
    sys::Posef {
        orientation: pose.orientation,
        position: sys::Vector3f {
            x: pose.position.x + d[0],
            y: pose.position.y + d[1],
            z: pose.position.z + d[2],
        },
    }
}

pub fn inverse(pose: sys::Posef) -> sys::Posef {
    let q = pose.orientation;
    let conj = sys::Quaternionf {
        x: -q.x,
        y: -q.y,
        z: -q.z,
        w: q.w,
    };
    let p = rotate(conj, [pose.position.x, pose.position.y, pose.position.z]);
    sys::Posef {
        orientation: conj,
        position: sys::Vector3f {
            x: -p[0],
            y: -p[1],
            z: -p[2],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_moves_linearly_between_steps_and_inputs_hold_until_changed() {
        let s = Script::parse(
            "# turn left\n2 head 90 0\n1 input /a 1\n3 input /a 0\n1 input /s 0.5 -1\n",
        );
        let mid = s.head(1.0);
        // Half way: 45 degrees about +y.
        assert!((mid.orientation.y - (22.5f32).to_radians().sin()).abs() < 1e-4);
        assert_eq!(mid.position.y, 1.6);
        assert_eq!(s.input("/a", 0.5), None);
        assert_eq!(s.input("/a", 2.0), Some([1.0, 0.0]));
        assert_eq!(s.input("/a", 3.5), Some([0.0, 0.0]));
        assert_eq!(s.input("/s", 9.0), Some([0.5, -1.0]));
    }

    #[test]
    fn a_left_turn_faces_minus_x_and_eyes_sit_across_the_head() {
        let p = pose(90.0, 0.0, [0.0, 1.6, 0.0]);
        let f = rotate(p.orientation, [0.0, 0.0, -1.0]);
        assert!((f[0] + 1.0).abs() < 1e-5 && f[2].abs() < 1e-5, "{f:?}");
        let right_eye = offset(p, [0.03, 0.0, 0.0]);
        assert!((right_eye.position.z + 0.03).abs() < 1e-5);
        let back = inverse(p);
        let o = offset(back, [0.0, 1.6, 0.0]);
        let _ = o;
        let up = pose(0.0, 30.0, [0.0; 3]);
        let f = rotate(up.orientation, [0.0, 0.0, -1.0]);
        assert!(f[1] > 0.49, "pitch up looks up: {f:?}");
    }
}
