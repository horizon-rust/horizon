//! Touch controls: a floating move stick on the left, a look area on the right and a few buttons,
//! turned into the virtual pad's `PadState` so the game handles them like a controller
//! (docs/android.md). Pure state and geometry in window pixels; the game feeds it the touch events
//! and draws `controls()`.
use crate::gamepad::{Button, PadState};

/// How a touch changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Started,
    Moved,
    Ended,
}

/// A rectangle in window pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub fn contains(&self, at: [f32; 2]) -> bool {
        at[0] >= self.x && at[0] < self.x + self.w && at[1] >= self.y && at[1] < self.y + self.h
    }
}

/// What a button does when held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// A digital pad button.
    Button(Button),
    /// A trigger (attack and use are the pad's triggers): `0` left, `1` right.
    Trigger(usize),
}

/// One on-screen button.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Control {
    pub press: Press,
    /// One or two characters on the button.
    pub label: &'static str,
    pub rect: Rect,
    /// Held down right now.
    pub held: bool,
}

#[derive(Clone, Copy, Debug)]
struct Track {
    id: u64,
    origin: [f32; 2],
    at: [f32; 2],
}
impl Track {
    /// The drag as a stick: right and up positive, length at most 1.
    fn stick(&self, radius: f32) -> [f32; 2] {
        let dx = (self.at[0] - self.origin[0]) / radius;
        let dy = (self.origin[1] - self.at[1]) / radius;
        let length = dx.hypot(dy);
        if length > 1.0 {
            [dx / length, dy / length]
        } else {
            [dx, dy]
        }
    }
}

/// The touch controls' state.
#[derive(Debug, Default)]
pub struct TouchPad {
    screen: [f32; 2],
    moving: Option<Track>,
    looking: Option<Track>,
    /// Pointers holding a button, by index in `layout`.
    holding: Vec<(u64, usize)>,
}

/// Window pixels per touch unit: the controls keep their physical size on any screen. 360 units
/// are the short side of a typical phone.
fn unit(screen: [f32; 2]) -> f32 {
    (screen[0].min(screen[1]) / 360.0).max(1.0)
}

const BUTTONS: [(Press, &str); 9] = [
    (Press::Trigger(1), "Hit"),
    (Press::Trigger(0), "Use"),
    (Press::Button(Button::South), "Jmp"),
    (Press::Button(Button::RightStick), "Snk"),
    (Press::Button(Button::LeftStick), "Spr"),
    (Press::Button(Button::North), "Inv"),
    (Press::Button(Button::Start), "||"),
    (Press::Button(Button::LeftBumper), "<"),
    (Press::Button(Button::RightBumper), ">"),
];

/// Where each button of `BUTTONS` sits on a screen of this size.
fn layout(screen: [f32; 2]) -> [Rect; 9] {
    let u = unit(screen);
    let (w, h) = (screen[0], screen[1]);
    let m = 16.0 * u;
    let big = 72.0 * u;
    let mid = 56.0 * u;
    let small = 44.0 * u;
    let rect = |x: f32, y: f32, s: f32| Rect { x, y, w: s, h: s };
    [
        // Hit and Use at the bottom right, Jump above them.
        rect(w - m - big, h - m - big, big),
        rect(w - m - big - m - mid, h - m - mid, mid),
        rect(w - m - mid, h - m - big - m - mid, mid),
        // Sneak and sprint at the bottom of the left half, out of the stick's way.
        rect(m, h - m - small, small),
        rect(m + small + m, h - m - small, small),
        // Inventory and pause along the top.
        rect(w - m - small, m, small),
        rect(w / 2.0 - small / 2.0, m, small),
        // Hotbar steps either side of the bottom centre.
        rect(w / 2.0 - m / 2.0 - small - small, h - m - small, small),
        rect(w / 2.0 + m / 2.0 + small, h - m - small, small),
    ]
}

impl TouchPad {
    /// The window's size in pixels. Touches in flight keep going.
    pub fn set_screen(&mut self, width: f32, height: f32) {
        self.screen = [width, height];
    }

    /// Nothing is touching the controls.
    pub fn is_idle(&self) -> bool {
        self.moving.is_none() && self.looking.is_none() && self.holding.is_empty()
    }

    /// Let go of everything (a menu opened, the window lost the surface).
    pub fn release_all(&mut self) {
        self.moving = None;
        self.looking = None;
        self.holding.clear();
    }

    /// A touch started, moved or ended at `at` (window pixels).
    pub fn event(&mut self, id: u64, phase: Phase, at: [f32; 2]) {
        match phase {
            Phase::Started => self.start(id, at),
            Phase::Moved => {
                for track in [&mut self.moving, &mut self.looking].into_iter().flatten() {
                    if track.id == id {
                        track.at = at;
                    }
                }
            }
            Phase::Ended => {
                if self.moving.is_some_and(|t| t.id == id) {
                    self.moving = None;
                }
                if self.looking.is_some_and(|t| t.id == id) {
                    self.looking = None;
                }
                self.holding.retain(|(pointer, _)| *pointer != id);
            }
        }
    }

    fn start(&mut self, id: u64, at: [f32; 2]) {
        if let Some(index) = layout(self.screen).iter().position(|r| r.contains(at)) {
            self.holding.push((id, index));
        } else if at[0] < self.screen[0] * 0.45 {
            if self.moving.is_none() {
                self.moving = Some(Track { id, origin: at, at });
            }
        } else if self.looking.is_none() {
            self.looking = Some(Track { id, origin: at, at });
        }
    }

    /// The touches as a pad: the move stick on the left stick, the look drag on the right stick.
    pub fn state(&self) -> PadState {
        let u = unit(self.screen);
        let mut pad = PadState::default();
        if let Some(track) = &self.moving {
            pad.left = track.stick(self.stick_radius());
        }
        if let Some(track) = &self.looking {
            pad.right = track.stick(72.0 * u);
        }
        for (_, index) in &self.holding {
            match BUTTONS[*index].0 {
                Press::Button(button) => pad.set(button, true),
                Press::Trigger(side) => pad.triggers[side] = 1.0,
            }
        }
        pad
    }

    /// The move stick's full travel in window pixels.
    pub fn stick_radius(&self) -> f32 {
        56.0 * unit(self.screen)
    }

    /// Where the move stick's finger started and is now, if it is down (for drawing).
    pub fn move_stick(&self) -> Option<([f32; 2], [f32; 2])> {
        self.moving.map(|t| (t.origin, t.at))
    }

    /// The buttons to draw.
    pub fn controls(&self) -> Vec<Control> {
        let rects = layout(self.screen);
        BUTTONS
            .iter()
            .enumerate()
            .map(|(index, (press, label))| Control {
                press: *press,
                label,
                rect: rects[index],
                held: self.holding.iter().any(|(_, i)| *i == index),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad() -> TouchPad {
        let mut t = TouchPad::default();
        t.set_screen(1080.0, 2400.0);
        t
    }

    #[test]
    fn a_drag_on_the_left_is_the_move_stick() {
        let mut t = pad();
        t.event(1, Phase::Started, [200.0, 1000.0]);
        t.event(1, Phase::Moved, [200.0, 800.0]);
        let s = t.state();
        assert!(s.left[1] > 0.9, "dragging up moves forward: {:?}", s.left);
        assert!(s.left[0].abs() < 0.01);
        t.event(1, Phase::Ended, [200.0, 800.0]);
        assert_eq!(t.state(), PadState::default());
        assert!(t.is_idle());
    }

    #[test]
    fn the_stick_never_exceeds_one() {
        let mut t = pad();
        t.event(1, Phase::Started, [200.0, 1000.0]);
        t.event(1, Phase::Moved, [900.0, 100.0]);
        let s = t.state().left;
        assert!(s[0].hypot(s[1]) <= 1.0001);
    }

    #[test]
    fn a_drag_on_the_right_looks() {
        let mut t = pad();
        t.event(7, Phase::Started, [800.0, 1000.0]);
        t.event(7, Phase::Moved, [860.0, 1000.0]);
        let s = t.state();
        assert!(s.right[0] > 0.0 && s.left == [0.0, 0.0]);
    }

    #[test]
    fn a_button_holds_while_touched() {
        let mut t = pad();
        let attack = t.controls()[0].rect;
        let at = [attack.x + 5.0, attack.y + 5.0];
        t.event(3, Phase::Started, at);
        assert_eq!(t.state().triggers[1], 1.0);
        assert!(t.controls()[0].held);
        t.event(3, Phase::Ended, at);
        assert_eq!(t.state().triggers[1], 0.0);
    }

    #[test]
    fn two_fingers_move_and_look_together() {
        let mut t = pad();
        t.event(1, Phase::Started, [200.0, 1000.0]);
        t.event(2, Phase::Started, [800.0, 1000.0]);
        t.event(1, Phase::Moved, [200.0, 900.0]);
        t.event(2, Phase::Moved, [800.0, 950.0]);
        let s = t.state();
        assert!(s.left[1] > 0.0 && s.right[1] > 0.0 && s.right[0] == 0.0);
    }

    #[test]
    fn buttons_stay_on_screen_and_apart() {
        for screen in [[1080.0, 2400.0], [2400.0, 1080.0], [640.0, 360.0]] {
            let rects = layout(screen);
            for (i, a) in rects.iter().enumerate() {
                assert!(a.x >= 0.0 && a.y >= 0.0, "{screen:?} {i}");
                assert!(
                    a.x + a.w <= screen[0] && a.y + a.h <= screen[1],
                    "{screen:?} {i}"
                );
            }
        }
    }
}
