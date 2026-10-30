//! libnx `HidNpadButton`/`HidNpadStyleTag` bits to a positional pad, and what a controller
//! change means (Joy-Con attached, detached, handheld, gone) so no button sticks.

/// `HidNpadButton` (switch/services/hid.h).
pub mod bits {
    pub const A: u64 = 1 << 0;
    pub const B: u64 = 1 << 1;
    pub const X: u64 = 1 << 2;
    pub const Y: u64 = 1 << 3;
    pub const STICK_L: u64 = 1 << 4;
    pub const STICK_R: u64 = 1 << 5;
    pub const L: u64 = 1 << 6;
    pub const R: u64 = 1 << 7;
    pub const ZL: u64 = 1 << 8;
    pub const ZR: u64 = 1 << 9;
    pub const PLUS: u64 = 1 << 10;
    pub const MINUS: u64 = 1 << 11;
    pub const LEFT: u64 = 1 << 12;
    pub const UP: u64 = 1 << 13;
    pub const RIGHT: u64 = 1 << 14;
    pub const DOWN: u64 = 1 << 15;
    pub const LEFT_SL: u64 = 1 << 24;
    pub const LEFT_SR: u64 = 1 << 25;
    pub const RIGHT_SL: u64 = 1 << 26;
    pub const RIGHT_SR: u64 = 1 << 27;
}

/// `HidNpadStyleTag` bits.
pub mod style {
    pub const FULL_KEY: u32 = 1 << 0;
    pub const HANDHELD: u32 = 1 << 1;
    pub const JOY_DUAL: u32 = 1 << 2;
    pub const JOY_LEFT: u32 = 1 << 3;
    pub const JOY_RIGHT: u32 = 1 << 4;
    /// Pro Controller, handheld, dual and single Joy-Con.
    pub const STANDARD: u32 = 0x1F;
}

/// A button by position, in the order of `horizon_client::gamepad::Button` (South is Nintendo B).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    South,
    East,
    West,
    North,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

/// One libnx `PadState` read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Raw {
    pub buttons: u64,
    /// Left then right stick, each axis -32767..=32767, +y up.
    pub sticks: [[i32; 2]; 2],
    pub style_set: u32,
}

/// The pad as the game sees it: held keys, sticks and triggers (ZL/ZR are digital on a Switch).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub held: [bool; 16],
    pub left: [f32; 2],
    pub right: [f32; 2],
    pub triggers: [f32; 2],
    /// Any controller is attached.
    pub connected: bool,
    /// Held as a single sideways Joy-Con: one stick, SL/SR are the bumpers.
    pub sideways: bool,
}

impl Frame {
    pub fn is_held(&self, key: Key) -> bool {
        self.held[key as usize]
    }
}

/// Turns reads into frames and notices controller changes.
#[derive(Default)]
pub struct Tracker {
    style_set: u32,
    /// Frames still reported as released after a change: the new controller's first read can carry
    /// the old one's buttons, and a Joy-Con pulled off the rail must not keep its grip.
    settle: u8,
}

const SETTLE: u8 = 2;

impl Tracker {
    /// `Some(style_set)` when the attached controllers changed on the last `frame` call.
    pub fn changed(&self) -> Option<u32> {
        (self.settle == SETTLE).then_some(self.style_set)
    }

    pub fn frame(&mut self, raw: Raw, deadzone: f32) -> Frame {
        if raw.style_set != self.style_set {
            self.style_set = raw.style_set;
            self.settle = SETTLE;
        } else if self.settle > 0 {
            self.settle -= 1;
        }
        let connected = raw.style_set & style::STANDARD != 0;
        if !connected || self.settle > 0 {
            return Frame {
                connected,
                ..Frame::default()
            };
        }
        map(raw, deadzone)
    }
}

fn map(raw: Raw, deadzone: f32) -> Frame {
    let single = raw.style_set & (style::JOY_LEFT | style::JOY_RIGHT) != 0
        && raw.style_set & (style::FULL_KEY | style::HANDHELD | style::JOY_DUAL) == 0;
    let b = raw.buttons;
    let on = |m: u64| b & m != 0;
    let axis = |v: i32| (v as f32 / 32767.0).clamp(-1.0, 1.0);
    let stick = |s: [i32; 2]| radial([axis(s[0]), axis(s[1])], deadzone);
    let mut held = [false; 16];
    for (key, v) in [
        (Key::South, on(bits::B)),
        (Key::East, on(bits::A)),
        (Key::West, on(bits::Y)),
        (Key::North, on(bits::X)),
        (Key::LeftBumper, on(bits::L)),
        (Key::RightBumper, on(bits::R)),
        (Key::Select, on(bits::MINUS)),
        (Key::Start, on(bits::PLUS)),
        (Key::LeftStick, on(bits::STICK_L)),
        (Key::RightStick, on(bits::STICK_R)),
        (Key::DPadUp, on(bits::UP)),
        (Key::DPadDown, on(bits::DOWN)),
        (Key::DPadLeft, on(bits::LEFT)),
        (Key::DPadRight, on(bits::RIGHT)),
    ] {
        held[key as usize] = v;
    }
    let triggers = [on(bits::ZL) as u8 as f32, on(bits::ZR) as u8 as f32];
    if !single {
        return Frame {
            held,
            left: stick(raw.sticks[0]),
            right: stick(raw.sticks[1]),
            triggers,
            connected: true,
            sideways: false,
        };
    }
    // Held sideways: the one stick drives movement, SL/SR are the bumpers, and the buttons and
    // d-pad are rotated so "up" is away from the player. Joy-Con (L) is turned counter-clockwise,
    // Joy-Con (R) clockwise. (Untested on hardware: check the direction on a real Joy-Con.)
    let l = raw.style_set & style::JOY_LEFT != 0;
    let (sl, sr) = if l {
        (bits::LEFT_SL, bits::LEFT_SR)
    } else {
        (bits::RIGHT_SL, bits::RIGHT_SR)
    };
    let [x, y] = stick(if l { raw.sticks[0] } else { raw.sticks[1] });
    let left = if l { [-y, x] } else { [y, -x] };
    // (North, East, South, West) and (Up, Right, Down, Left) each move one step around.
    let rotate = |held: &mut [bool; 16], ring: [Key; 4]| {
        let v = ring.map(|k| held[k as usize]);
        for (i, k) in ring.iter().enumerate() {
            held[*k as usize] = if l { v[(i + 1) % 4] } else { v[(i + 3) % 4] };
        }
    };
    rotate(&mut held, [Key::North, Key::East, Key::South, Key::West]);
    rotate(
        &mut held,
        [Key::DPadUp, Key::DPadRight, Key::DPadDown, Key::DPadLeft],
    );
    held[Key::LeftBumper as usize] = on(sl);
    held[Key::RightBumper as usize] = on(sr);
    held[Key::LeftStick as usize] = on(bits::STICK_L) || on(bits::STICK_R);
    held[Key::RightStick as usize] = false;
    Frame {
        held,
        left,
        right: [0.0, 0.0],
        triggers,
        connected: true,
        sideways: true,
    }
}

/// Zero inside `deadzone` (a released stick rests near 0.08), then rescaled so the edge of the dead
/// zone is 0 and full deflection is 1. The game applies its own dead zones on top; this one only
/// removes the hardware's drift.
fn radial(v: [f32; 2], deadzone: f32) -> [f32; 2] {
    let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if len <= deadzone {
        return [0.0, 0.0];
    }
    let k = ((len - deadzone) / (1.0 - deadzone)).min(1.0) / len;
    [v[0] * k, v[1] * k]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(buttons: u64, style_set: u32) -> Raw {
        Raw {
            buttons,
            sticks: [[0; 2]; 2],
            style_set,
        }
    }

    fn settled(style_set: u32) -> Tracker {
        let mut t = Tracker::default();
        for _ in 0..3 {
            t.frame(raw(0, style_set), 0.1);
        }
        t
    }

    #[test]
    fn nintendo_b_is_south() {
        let mut t = settled(style::FULL_KEY);
        let f = t.frame(raw(bits::B | bits::ZR, style::FULL_KEY), 0.1);
        assert!(f.is_held(Key::South) && !f.is_held(Key::East));
        assert_eq!(f.triggers, [0.0, 1.0]);
    }

    #[test]
    fn style_change_releases_everything_briefly() {
        let mut t = settled(style::HANDHELD);
        let f = t.frame(raw(bits::A, style::FULL_KEY), 0.1);
        assert_eq!(t.changed(), Some(style::FULL_KEY));
        assert!(!f.is_held(Key::East) && f.connected);
        t.frame(raw(bits::A, style::FULL_KEY), 0.1);
        assert!(
            t.frame(raw(bits::A, style::FULL_KEY), 0.1)
                .is_held(Key::East)
        );
    }

    #[test]
    fn disconnect_is_not_connected_and_zero() {
        let mut t = settled(style::FULL_KEY);
        let f = t.frame(raw(bits::A, 0), 0.1);
        assert!(!f.connected && f == Frame::default());
    }

    #[test]
    fn deadzone_removes_rest_drift_and_reaches_one() {
        assert_eq!(radial([0.081, 0.0], 0.1), [0.0, 0.0]);
        assert_eq!(radial([1.0, 0.0], 0.1), [1.0, 0.0]);
    }

    #[test]
    fn single_joycon_left_uses_sl_sr_and_rotates_stick() {
        let mut t = settled(style::JOY_LEFT);
        let mut r = raw(bits::LEFT_SL | bits::UP, style::JOY_LEFT);
        r.sticks[0] = [0, 32767];
        let f = t.frame(r, 0.1);
        assert!(f.sideways && f.is_held(Key::LeftBumper));
        assert_eq!(f.left, [-1.0, 0.0]);
        // The d-pad's Up (the Joy-Con's rail side after turning) moved one step round.
        assert!(!f.is_held(Key::DPadUp) && f.is_held(Key::DPadLeft));
    }
}
