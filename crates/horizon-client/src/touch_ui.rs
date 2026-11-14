// Adapted from Starling (https://github.com/starling-client/starling, `android` branch at
// 150a414b09a1, `crates/horizon-client/src/touch_ui.rs`), written by Vasysik and the Starling
// contributors. Copyright (c) 2026 Ludovic Marie and the Horizon contributors, under the same MIT
// license as Horizon (LICENSE at the repository root).

//! Touch on menus and screens (Android first), apart from the gameplay
//! fingers of [`horizon_client_render::touch::TouchControls`]. Nothing here runs unless a
//! touch surface is in use.
//!
//! - [`TouchUi::gesture`]: on a list a drag scrolls and a tap clicks on
//!   release, so scrolling a list never activates a row;
//!   [`TouchUi::direct_pointer`] keeps a finger that lands on a slider
//!   dragging it like a mouse.
//! - The virtual mouse ([`TouchUi::relative_touch`]): the finger moves the
//!   pointer by how far it slides, like a laptop touchpad, so touching down
//!   never teleports an item held in the inventory. A still tap left-clicks,
//!   a still hold right-clicks once; LMB and RMB buttons in the lower right
//!   corner hold a button down for dragging.
//! - The menu row at the top right: the virtual mouse toggle and Back.
use crate::hud::{HudAtlas, HudBatch, rgba};
use crate::lang;
use crate::widgets::{self, Kind};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use winit::{event::TouchPhase, keyboard::KeyCode};

/// A menu finger that moves farther than this (surface pixels) scrolls
/// instead of tapping.
const TAP_SLOP: f32 = 12.0;
/// Surface pixels of drag per scrolled line.
const SCROLL_STEP: f32 = 30.0;
/// A virtual-mouse finger that moves farther than this (GUI pixels) is a
/// drag and never clicks.
const POINTER_SLOP: f32 = 3.0;
/// A still virtual-mouse finger held this long right-clicks.
const RIGHT_CLICK_DELAY: Duration = Duration::from_millis(500);
/// The height of the menu row's buttons, in GUI pixels.
const ROW_H: f32 = 20.0;

/// Menu touch state: the scroll-or-tap gesture, a slider drag, the virtual
/// mouse and its held buttons. Ids are the platform's finger ids.
#[derive(Default)]
pub struct TouchUi {
    /// Menu fingers drive a relative pointer instead of tapping (toggled by
    /// the menu row's Mouse button).
    pub virtual_mouse: bool,
    /// Fingers holding LMB (`false`) or RMB (`true`).
    mouse_buttons: BTreeMap<u64, bool>,
    relative: Option<RelativeTouch>,
    /// A click the virtual mouse owes the game: `Some(right)`.
    relative_click: Option<bool>,
    gesture: Option<GestureTouch>,
    /// The finger dragging a slider.
    pointer_drag: Option<u64>,
}

/// The finger of the scroll-or-tap gesture.
struct GestureTouch {
    id: u64,
    start: [f32; 2],
    previous: [f32; 2],
    dragged: bool,
}

/// The finger moving the virtual mouse.
struct RelativeTouch {
    id: u64,
    origin: [f32; 2],
    last: [f32; 2],
    since: Instant,
    /// Moved past [`POINTER_SLOP`], or never clicks: no click on release.
    dragged: bool,
    /// The hold already right-clicked.
    right_sent: bool,
}

/// What a menu finger did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    None,
    /// Lifted without dragging: click where it was.
    Tap,
    /// Dragged: scroll by this many lines (positive is down the screen).
    Scroll(f32),
}

/// A button of the touch layer under a finger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Press this key (Back is [`KeyCode::Escape`]).
    Key(KeyCode),
    /// Toggle the virtual mouse.
    VirtualMouse,
    /// Hold LMB (`false`) or RMB (`true`) while the finger stays down.
    MouseButton(bool),
}

/// The LMB and RMB buttons, `[x, y, w, h]` in GUI pixels, lower right.
fn mouse_button_rects(w: f32, h: f32) -> [([f32; 4], bool); 2] {
    [
        ([w - 53.0, h - 51.0, 50.0, ROW_H], false),
        ([w - 53.0, h - 27.0, 50.0, ROW_H], true),
    ]
}

/// The menu row's Mouse toggle and Back button, at the top right.
fn menu_row_rects(w: f32) -> [([f32; 4], Action); 2] {
    [
        ([w - 108.0, 3.0, 50.0, ROW_H], Action::VirtualMouse),
        ([w - 53.0, 3.0, 50.0, ROW_H], Action::Key(KeyCode::Escape)),
    ]
}

fn inside([x, y, w, h]: [f32; 4], p: [f32; 2]) -> bool {
    (x..x + w).contains(&p[0]) && (y..y + h).contains(&p[1])
}

/// The outlined arrow of a pointer that has no system cursor (the virtual
/// mouse; a controller in menus can share it), its tip at `cursor`.
pub fn draw_pointer(batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32)) {
    let (x, y) = (cursor.0.floor(), cursor.1.floor());
    const ROWS: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 4, 2, 1];
    for (i, &w) in ROWS.iter().enumerate() {
        let black = rgba(0, 0, 0, 255);
        batch.fill(atlas, x - 1.0, y + i as f32, f32::from(w) + 2.0, 1.0, black);
    }
    batch.fill(atlas, x - 1.0, y + 10.0, 2.0, 1.0, rgba(0, 0, 0, 255));
    for (i, &w) in ROWS.iter().enumerate().skip(1).take(8) {
        let white = rgba(255, 255, 255, 255);
        batch.fill(atlas, x, y + i as f32, f32::from(w) - 1.0, 1.0, white);
    }
}

impl TouchUi {
    /// A finger presses LMB or RMB; whether it is the first to hold that
    /// button (only then does the game get a press).
    pub fn press_mouse(&mut self, id: u64, right: bool) -> bool {
        if let Some(touch) = &mut self.relative {
            touch.dragged = true;
        }
        let first = !self.mouse_buttons.values().any(|v| *v == right);
        self.mouse_buttons.insert(id, right);
        first
    }

    /// A finger lifts from LMB or RMB: `Some((right, last))`, `last` when no
    /// other finger still holds that button (only then does the game get a
    /// release). `None` when the finger held no mouse button.
    pub fn release_mouse(&mut self, id: u64) -> Option<(bool, bool)> {
        let right = self.mouse_buttons.remove(&id)?;
        Some((right, !self.mouse_buttons.values().any(|v| *v == right)))
    }

    pub fn mouse_held(&self) -> bool {
        !self.mouse_buttons.is_empty()
    }

    /// The finger holds one of the mouse buttons, so its release ends it
    /// even when the screen or layout changed underneath.
    pub fn owns_mouse(&self, id: u64) -> bool {
        self.mouse_buttons.contains_key(&id)
    }

    /// Let go of every held mouse button: the buttons to release, each once.
    pub fn release_all_mouse(&mut self) -> Vec<bool> {
        let mut buttons: Vec<_> = std::mem::take(&mut self.mouse_buttons)
            .into_values()
            .collect();
        buttons.sort();
        buttons.dedup();
        buttons
    }

    /// Forget every finger in progress (the platform cancelled the touches,
    /// or a screen closed): no tap, scroll or click follows.
    pub fn cancel(&mut self) {
        self.gesture = None;
        self.pointer_drag = None;
        self.cancel_relative();
    }

    /// Forget the virtual mouse's finger and any click it owes.
    pub fn cancel_relative(&mut self) {
        self.relative = None;
        self.relative_click = None;
    }

    /// The virtual mouse: moves `cursor` (GUI pixels, kept inside `size`) by
    /// how far the finger slid times `sensitivity`, and returns where it is
    /// now. Only the first finger down moves it. With `clicks`, lifting a
    /// finger that stayed still owes a click ([`Self::relative_click`]): left
    /// for a tap, right for a hold.
    #[allow(clippy::too_many_arguments)]
    pub fn relative_touch(
        &mut self,
        id: u64,
        point: [f32; 2],
        phase: TouchPhase,
        cursor: [f32; 2],
        size: [f32; 2],
        sensitivity: f32,
        clicks: bool,
    ) -> [f32; 2] {
        if phase == TouchPhase::Cancelled {
            self.cancel();
            return cursor;
        }
        if phase == TouchPhase::Started && self.relative.is_none() {
            self.relative = Some(RelativeTouch {
                id,
                origin: point,
                last: point,
                since: Instant::now(),
                dragged: !clicks,
                right_sent: false,
            });
        }
        let Some(touch) = self.relative.as_mut().filter(|t| t.id == id) else {
            return cursor;
        };
        touch.dragged |=
            (point[0] - touch.origin[0]).hypot(point[1] - touch.origin[1]) > POINTER_SLOP;
        let position = [0, 1].map(|i| {
            (cursor[i] + (point[i] - touch.last[i]) * sensitivity)
                .clamp(0.0, (size[i] - 1.0).max(0.0))
        });
        touch.last = point;
        if phase == TouchPhase::Ended {
            if clicks && !touch.dragged && !touch.right_sent {
                self.relative_click = Some(touch.since.elapsed() >= RIGHT_CLICK_DELAY);
            }
            self.relative = None;
        }
        position
    }

    /// The click the virtual mouse owes, polled each tick: `Some(false)` for
    /// LMB, `Some(true)` for RMB. A still hold right-clicks once as soon as
    /// it is long enough, before the finger lifts. Without `clicks` (a
    /// mouse button is held) it owes nothing.
    pub fn relative_click(&mut self, clicks: bool) -> Option<bool> {
        if !clicks {
            self.relative_click = None;
            return None;
        }
        if let Some(right) = self.relative_click.take() {
            return Some(right);
        }
        let touch = self.relative.as_mut()?;
        if !touch.dragged && !touch.right_sent && touch.since.elapsed() >= RIGHT_CLICK_DELAY {
            touch.right_sent = true;
            return Some(true);
        }
        None
    }

    /// Whether this finger drives the pointer directly, like a mouse with its
    /// button held: one that lands on a slider of the last drawn frame keeps
    /// it until it lifts, and another finger landing on a slider meanwhile
    /// takes nothing. `p` is in GUI pixels.
    pub fn direct_pointer(&mut self, id: u64, p: [f32; 2], phase: TouchPhase) -> bool {
        if phase == TouchPhase::Started
            && self.pointer_drag.is_none()
            && widgets::last_targets()
                .iter()
                .any(|t| matches!(t.kind, Kind::Slider { .. }) && inside(t.rect, p))
        {
            self.pointer_drag = Some(id);
        }
        if self.pointer_drag != Some(id) {
            return false;
        }
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.pointer_drag = None;
        }
        true
    }

    /// Scroll or tap: a finger that drags past [`TAP_SLOP`] scrolls as it
    /// moves and never clicks; one that lifts without dragging taps. Only
    /// the first finger down counts. `p` is in surface pixels.
    pub fn gesture(&mut self, id: u64, p: [f32; 2], phase: TouchPhase) -> Gesture {
        if phase == TouchPhase::Cancelled {
            self.cancel();
            return Gesture::None;
        }
        if phase == TouchPhase::Started {
            if self.gesture.is_none() {
                self.gesture = Some(GestureTouch {
                    id,
                    start: p,
                    previous: p,
                    dragged: false,
                });
            }
            return Gesture::None;
        }
        let Some(touch) = self.gesture.as_mut().filter(|t| t.id == id) else {
            return Gesture::None;
        };
        touch.dragged |= (p[0] - touch.start[0]).hypot(p[1] - touch.start[1]) > TAP_SLOP;
        let dy = p[1] - touch.previous[1];
        touch.previous = p;
        if phase == TouchPhase::Ended {
            let tap = !touch.dragged;
            self.cancel();
            return if tap { Gesture::Tap } else { Gesture::None };
        }
        if touch.dragged {
            Gesture::Scroll(dy / SCROLL_STEP)
        } else {
            Gesture::None
        }
    }

    /// The touch layer's button under `p` (GUI pixels) on a `w` x `h` GUI,
    /// if any. `menu`: a screen is open (the menu row and the mouse
    /// buttons show only there).
    pub fn action(&self, w: f32, h: f32, p: [f32; 2], menu: bool) -> Option<Action> {
        if !menu {
            return None;
        }
        if self.virtual_mouse
            && let Some((_, right)) = mouse_button_rects(w, h)
                .into_iter()
                .find(|(rect, _)| inside(*rect, p))
        {
            return Some(Action::MouseButton(right));
        }
        menu_row_rects(w)
            .into_iter()
            .find(|(rect, _)| inside(*rect, p))
            .map(|(_, action)| action)
    }

    /// Draw the touch layer's buttons on a screen (`menu`), in GUI pixels.
    pub fn draw(&self, b: &mut HudBatch, a: &HudAtlas, menu: bool) {
        if !menu {
            return;
        }
        for (rect, action) in menu_row_rects(b.width) {
            let key = match action {
                Action::VirtualMouse if self.virtual_mouse => "horizon.touch.mouse_on",
                Action::VirtualMouse => "horizon.touch.mouse",
                _ => "gui.back",
            };
            button(b, a, rect, &lang::get(key), false);
        }
        if self.virtual_mouse {
            for (rect, right) in mouse_button_rects(b.width, b.height) {
                let held = self.mouse_buttons.values().any(|v| *v == right);
                let key = if right {
                    "horizon.touch.rmb"
                } else {
                    "horizon.touch.lmb"
                };
                button(b, a, rect, &lang::get(key), held);
            }
        }
    }
}

/// A touch-layer button, drawn like the on-screen controls' buttons.
fn button(b: &mut HudBatch, a: &HudAtlas, rect: [f32; 4], label: &str, held: bool) {
    horizon_client_render::touch::button(b, a, rect, label, held, rgba(245, 245, 245, 255));
}

/// Whether a physical left click (a mouse, a TV remote's pointer) takes part
/// in the touch layer's buttons as a finger would, instead of going to the
/// screen: a press only over one of its buttons (`touched`; anywhere else it
/// belongs to the world or the screen), a release only of a press it already
/// holds (`owned`), even when the layout no longer has that button under
/// the pointer, so a held press survives a screen change. `Started` or
/// `Ended` for the finger to feed; `None` leaves the click to the game.
pub fn aux_mouse_phase(pressed: bool, owned: bool, touched: bool) -> Option<TouchPhase> {
    if pressed {
        touched.then_some(TouchPhase::Started)
    } else {
        owned.then_some(TouchPhase::Ended)
    }
}

#[cfg(test)]
mod tests;
