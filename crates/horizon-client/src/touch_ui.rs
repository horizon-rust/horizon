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
//! - The auxiliary keys, in a world too: `...` at the top left opens a row
//!   (F3, F5, Chat, Keys) and a grid of the desktop keys a phone cannot
//!   type (F1-F12, Tab, Esc, Shift, Ctrl, drop, the advancements...), the
//!   F3 combos and a held F3. Keys opens Horizon's on-screen keyboard
//!   ([`crate::osk`]) for typing by touch.
use crate::hud::{HudAtlas, HudBatch, rgba};
use crate::lang;
use crate::osk::{Keyboard, Output};
use crate::pad_glyph::Glyphs;
use crate::ui_theme::Ui;
use crate::widgets::{self, Kind};
use std::collections::{BTreeMap, BTreeSet};
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
/// mouse and its held buttons, the auxiliary keys. Ids are the platform's
/// finger ids.
#[derive(Default)]
pub struct TouchUi {
    /// The auxiliary keys are open (`...`).
    pub expanded: bool,
    /// The on-screen keyboard, while open.
    pub keyboard: Option<Keyboard>,
    /// F3 stays held without a finger on it (HoldF3), for F3 combos with
    /// keys of the world.
    pub f3_latched: bool,
    /// The key each finger holds.
    held_keys: BTreeMap<u64, KeyCode>,
    /// Shift and Ctrl, toggled on and off by a tap.
    latched_modifiers: BTreeSet<KeyCode>,
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
    /// Press this key while the finger stays down (Back is
    /// [`KeyCode::Escape`]).
    Key(KeyCode),
    /// Open or close the auxiliary keys.
    More,
    /// Open or close the on-screen keyboard.
    Keyboard,
    /// Toggle the held F3.
    LatchF3,
    /// Pick the block under the crosshair (the middle click).
    Pick,
    /// An F3 combo: F3 held while this key is tapped.
    DebugCombo(KeyCode),
    /// Toggle the virtual mouse.
    VirtualMouse,
    /// Hold LMB (`false`) or RMB (`true`) while the finger stays down.
    MouseButton(bool),
}

/// The auxiliary keys' grid (labels as in [`label`]), four to a row, in order; after them comes
/// HoldF3. Pick sends the middle click, the last four are F3 combos.
const KEYS: [(&str, KeyCode); 28] = [
    ("F1", KeyCode::F1),
    ("F2", KeyCode::F2),
    ("F3", KeyCode::F3),
    ("F4", KeyCode::F4),
    ("F5", KeyCode::F5),
    ("F6", KeyCode::F6),
    ("F7", KeyCode::F7),
    ("F8", KeyCode::F8),
    ("F9", KeyCode::F9),
    ("F10", KeyCode::F10),
    ("F11", KeyCode::F11),
    ("F12", KeyCode::F12),
    ("horizon.touch.key.tab", KeyCode::Tab),
    ("horizon.touch.swap", KeyCode::KeyF),
    ("horizon.touch.key.drop", KeyCode::KeyQ),
    ("horizon.touch.key.advancements", KeyCode::KeyL),
    ("horizon.touch.chat", KeyCode::KeyT),
    ("/", KeyCode::Slash),
    ("horizon.touch.key.enter", KeyCode::Enter),
    ("horizon.touch.key.escape", KeyCode::Escape),
    ("horizon.touch.key.shift", KeyCode::ShiftLeft),
    ("horizon.touch.key.control", KeyCode::ControlLeft),
    ("horizon.touch.key.inventory", KeyCode::KeyE),
    ("horizon.touch.key.pick", KeyCode::ContextMenu),
    ("F3+B", KeyCode::KeyB),
    ("F3+G", KeyCode::KeyG),
    ("F3+A", KeyCode::KeyA),
    ("F3+T", KeyCode::KeyT),
];
/// Where [`KEYS`] at `index` goes: Pick and the combos are their own actions.
const PICK: usize = 23;
const COMBOS: usize = 24;
/// The grid's top left and a cell's step, in GUI pixels.
const GRID: [f32; 2] = [3.0, 27.0];
const CELL: [f32; 2] = [50.0, 24.0];

/// The top row from the left: `...` always, the others while open, as
/// `(label, x, width, action)`, labels as in [`label`].
const TOP_ROW: [(&str, f32, f32, Action); 5] = [
    ("...", 3.0, 33.0, Action::More),
    ("F3", 39.0, 33.0, Action::Key(KeyCode::F3)),
    ("F5", 75.0, 33.0, Action::Key(KeyCode::F5)),
    (
        "horizon.touch.chat",
        111.0,
        33.0,
        Action::Key(KeyCode::KeyT),
    ),
    ("horizon.touch.keys", 147.0, 39.0, Action::Keyboard),
];

/// The grid cell at `index` (`KEYS`, then HoldF3), `[x, y, w, h]`.
fn grid_rect(index: usize) -> [f32; 4] {
    let (column, row) = ((index % 4) as f32, (index / 4) as f32);
    [
        GRID[0] + column * CELL[0],
        GRID[1] + row * CELL[1],
        48.0,
        ROW_H,
    ]
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
    /// A finger holds `key`; whether it is the first to hold it (only then
    /// does the game get a press).
    pub fn key_down(&mut self, id: u64, key: KeyCode) -> bool {
        let first = !self.key_held(key);
        self.held_keys.insert(id, key);
        first
    }

    /// A finger lifts: the key to release, when it held one that nothing
    /// else (another finger, the held F3) still holds.
    pub fn key_up(&mut self, id: u64) -> Option<KeyCode> {
        let key = self.held_keys.remove(&id)?;
        (!self.key_held(key)).then_some(key)
    }

    /// The finger holds a key, so its release is the layer's.
    pub fn owns_key(&self, id: u64) -> bool {
        self.held_keys.contains_key(&id)
    }

    /// Tap Shift or Ctrl: whether it is now down.
    pub fn toggle_modifier(&mut self, key: KeyCode) -> bool {
        if self.latched_modifiers.remove(&key) {
            false
        } else {
            self.latched_modifiers.insert(key);
            true
        }
    }

    pub fn modifier_latched(&self, key: KeyCode) -> bool {
        self.latched_modifiers.contains(&key)
    }

    pub fn clear_modifiers(&mut self) {
        self.latched_modifiers.clear();
    }

    /// A finger or the held F3 holds `key`.
    pub fn key_held(&self, key: KeyCode) -> bool {
        (key == KeyCode::F3 && self.f3_latched) || self.held_keys.values().any(|k| *k == key)
    }

    /// Let go of every held key and the held F3: the keys to release, each
    /// once.
    pub fn release_keys(&mut self) -> Vec<KeyCode> {
        let mut keys: Vec<_> = std::mem::take(&mut self.held_keys).into_values().collect();
        if std::mem::take(&mut self.f3_latched) {
            keys.push(KeyCode::F3);
        }
        keys.sort();
        keys.dedup();
        keys
    }

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
    /// buttons show only there; the auxiliary keys show in a world too).
    pub fn action(&self, w: f32, h: f32, p: [f32; 2], menu: bool) -> Option<Action> {
        if menu {
            if self.virtual_mouse
                && let Some((_, right)) = mouse_button_rects(w, h)
                    .into_iter()
                    .find(|(rect, _)| inside(*rect, p))
            {
                return Some(Action::MouseButton(right));
            }
            if let Some((_, action)) = menu_row_rects(w)
                .into_iter()
                .find(|(rect, _)| inside(*rect, p))
            {
                return Some(action);
            }
        }
        let shown = if self.expanded { 5 } else { 1 };
        if let Some(&(_, _, _, action)) = TOP_ROW[..shown]
            .iter()
            .find(|(_, x, width, _)| inside([*x, 3.0, *width, ROW_H], p))
        {
            return Some(action);
        }
        if !self.expanded {
            return None;
        }
        let index = (0..=KEYS.len()).find(|i| inside(grid_rect(*i), p))?;
        Some(match KEYS.get(index) {
            None => Action::LatchF3,
            Some(_) if index == PICK => Action::Pick,
            Some((_, key)) if index >= COMBOS => Action::DebugCombo(*key),
            Some((_, key)) => Action::Key(*key),
        })
    }

    /// Draw the touch layer's buttons, in GUI pixels; `menu` as in
    /// [`Self::action`].
    pub fn draw(&self, b: &mut HudBatch, a: &HudAtlas, menu: bool) {
        let shown = if self.expanded { 5 } else { 1 };
        for (text, x, width, action) in &TOP_ROW[..shown] {
            let held = *action == Action::Key(KeyCode::F3) && self.key_held(KeyCode::F3);
            let label = if held { "F3*".to_owned() } else { label(text) };
            button(b, a, [*x, 3.0, *width, ROW_H], &label, held);
        }
        if menu {
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
        if !self.expanded {
            return;
        }
        for (i, (text, key)) in KEYS.iter().enumerate() {
            let held = self.key_held(*key) || self.modifier_latched(*key);
            button(b, a, grid_rect(i), &label(text), held && i < PICK);
        }
        let key = if self.f3_latched {
            "horizon.touch.f3_on"
        } else {
            "horizon.touch.hold_f3"
        };
        let text = lang::get(key);
        button(b, a, grid_rect(KEYS.len()), &text, self.f3_latched);
    }

    /// A touch at `p` (GUI pixels) on the open on-screen keyboard, as
    /// [`Self::draw_keyboard`] last drew it: what the key under it types, if
    /// any.
    pub fn keyboard_touch(
        &mut self,
        b: &mut HudBatch,
        a: &HudAtlas,
        p: [f32; 2],
    ) -> Option<Output> {
        let ui = Ui::new(a, b, (0.0, 0.0));
        let at = p.map(|v| v * ui.batch.scale / ui.density);
        self.keyboard.as_mut()?.touch(at)
    }

    /// Draw the open on-screen keyboard across the bottom of the screen.
    pub fn draw_keyboard(&self, b: &mut HudBatch, a: &HudAtlas) {
        if let Some(keyboard) = &self.keyboard {
            let mut ui = Ui::new(a, b, (0.0, 0.0));
            keyboard.draw(&mut ui, false, Glyphs::default());
            ui.finish();
        }
    }
}

/// A button label: a `horizon.touch.*` translation key in the player's
/// language, or a key name ("F1", "/", "F3+B") that reads the same in every
/// language.
fn label(text: &str) -> String {
    // Split, not one prefix literal: the en_us key check reads every dotted string as a key.
    if text.split('.').take(2).eq(["horizon", "touch"]) {
        lang::get(text)
    } else {
        text.to_owned()
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
