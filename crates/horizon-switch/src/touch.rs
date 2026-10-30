//! The handheld touch screen (1280x720) as a mouse pointer for the menus: one finger moves the
//! pointer and presses while down; a second finger dragging scrolls, and tapping it (down and up
//! quickly without moving) is a right click at the first finger's place (split a stack, pick up half).

use std::time::{Duration, Instant};

pub const WIDTH: f32 = 1280.0;
pub const HEIGHT: f32 = 720.0;

/// One finger on the screen, in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// What the game's pointer should do this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pointer {
    /// Position as a fraction of the screen, while a finger is down and on its release frame.
    pub at: Option<[f32; 2]>,
    pub down: bool,
    /// Vertical scroll in screen pixels (a second finger dragging).
    pub scroll: f32,
    /// The second finger was just tapped: a right click at `at`.
    pub right_tap: bool,
}

/// A second finger that stays within this many pixels and lifts within this long is a tap.
const TAP_SLOP: f32 = 12.0;
const TAP_TIME: Duration = Duration::from_millis(300);

#[derive(Default)]
pub struct Tracker {
    started: Option<Instant>,
    primary: Option<u32>,
    second_y: Option<f32>,
    /// Where and when the second finger came down, and whether it has moved past the slop.
    second_down: Option<([f32; 2], Duration, bool)>,
    last: Option<[f32; 2]>,
    was_down: bool,
}

impl Tracker {
    /// `touches` now, with the clock of the real time since the first call.
    pub fn update(&mut self, touches: &[Touch]) -> Pointer {
        let now = self.started.get_or_insert_with(Instant::now).elapsed();
        self.update_at(touches, now)
    }

    /// As `update`, at a given time since the start (for tests).
    pub fn update_at(&mut self, touches: &[Touch], now: Duration) -> Pointer {
        // Keep following the same finger; if it lifted, the oldest remaining one takes over.
        let primary = self
            .primary
            .and_then(|id| touches.iter().find(|t| t.id == id))
            .or_else(|| touches.first());
        self.primary = primary.map(|t| t.id);
        let mut scroll = 0.0;
        let mut right_tap = false;
        match touches.iter().find(|t| Some(t.id) != self.primary) {
            Some(t) => {
                if let Some(prev) = self.second_y {
                    scroll = t.y - prev;
                }
                self.second_y = Some(t.y);
                let (start, since, moved) =
                    self.second_down.get_or_insert(([t.x, t.y], now, false));
                let _ = since;
                if (t.x - start[0]).abs() > TAP_SLOP || (t.y - start[1]).abs() > TAP_SLOP {
                    *moved = true;
                }
            }
            None => {
                if let Some((_, since, moved)) = self.second_down.take() {
                    right_tap =
                        !moved && primary.is_some() && now.saturating_sub(since) <= TAP_TIME;
                }
                self.second_y = None;
            }
        }
        match primary {
            Some(t) => {
                let at = [
                    (t.x / WIDTH).clamp(0.0, 1.0),
                    (t.y / HEIGHT).clamp(0.0, 1.0),
                ];
                self.last = Some(at);
                self.was_down = true;
                Pointer {
                    at: Some(at),
                    down: true,
                    scroll,
                    right_tap,
                }
            }
            // The release frame still reports where the finger was, so the click lands there.
            None if self.was_down => {
                self.was_down = false;
                Pointer {
                    at: self.last,
                    down: false,
                    scroll: 0.0,
                    right_tap: false,
                }
            }
            None => Pointer::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_scroll_release() {
        let mut t = Tracker::default();
        let p = t.update(&[Touch {
            id: 1,
            x: 640.0,
            y: 360.0,
        }]);
        assert_eq!(p.at, Some([0.5, 0.5]));
        assert!(p.down);
        t.update(&[
            Touch {
                id: 1,
                x: 640.0,
                y: 360.0,
            },
            Touch {
                id: 2,
                x: 10.0,
                y: 100.0,
            },
        ]);
        let p = t.update(&[
            Touch {
                id: 1,
                x: 640.0,
                y: 360.0,
            },
            Touch {
                id: 2,
                x: 10.0,
                y: 130.0,
            },
        ]);
        assert_eq!(p.scroll, 30.0);
        t.update(&[]);
        assert_eq!(t.update(&[]), Pointer::default());
    }

    #[test]
    fn second_finger_tap_is_a_right_click_but_a_drag_or_a_long_press_is_not() {
        let one = Touch {
            id: 1,
            x: 100.0,
            y: 100.0,
        };
        let two = |x: f32, y: f32| Touch { id: 2, x: x, y: y };
        let ms = Duration::from_millis;
        let mut t = Tracker::default();
        t.update_at(&[one], ms(0));
        t.update_at(&[one, two(500.0, 500.0)], ms(10));
        let p = t.update_at(&[one], ms(120));
        assert!(p.right_tap && p.down);
        // Dragging scrolls and does not click.
        t.update_at(&[one, two(500.0, 500.0)], ms(200));
        t.update_at(&[one, two(500.0, 560.0)], ms(220));
        assert!(!t.update_at(&[one], ms(240)).right_tap);
        // Held too long.
        t.update_at(&[one, two(500.0, 500.0)], ms(300));
        assert!(!t.update_at(&[one], ms(900)).right_tap);
    }

    #[test]
    fn release_frame_keeps_position() {
        let mut t = Tracker::default();
        t.update(&[Touch {
            id: 7,
            x: 128.0,
            y: 72.0,
        }]);
        let p = t.update(&[]);
        assert!(!p.down);
        assert_eq!(p.at, Some([0.1, 0.1]));
    }
}
