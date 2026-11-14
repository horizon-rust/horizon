// Adapted from Starling (https://github.com/starling-client/starling, `android` branch at
// 150a414b09a1, `crates/horizon-client-render/src/touch/editor.rs`), written by Vasysik and the
// Starling contributors. Copyright (c) 2026 Ludovic Marie and the Horizon contributors, under the
// same MIT license as Horizon (LICENSE at the repository root).

//! The touch layout editor, drawn over the game: drag a control to move it,
//! pick it to change its size or opacity (or every control's at once), hide
//! it, or bring hidden ones back from the palette. Reset restores the current
//! scheme's factory layout; Cancel restores everything as it was opened.
//! The palette also creates custom buttons, whose key or action is picked
//! from a list or by pressing it on a keyboard.
use super::custom::UNASSIGNED;
use super::{Control, CustomButton, Settings, TouchControls, draw, layout};
use crate::hud::{HudAtlas, HudBatch, rgba};
use crate::lang;

/// How the editor closed. After [`EditorExit::Done`] the game saves
/// [`TouchControls::settings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorExit {
    Done,
    Cancelled,
}

/// The editor's state while it is open ([`TouchControls::editor`]).
pub struct Editor {
    /// The built-in control being edited.
    pub selected: Option<Control>,
    slider: Option<Slider>,
    /// The slider changes every control of the scheme.
    apply_all: bool,
    /// The settings as the editor opened, for Cancel.
    original: Settings,
    /// The finger dragging a control (with its offset from the centre) or the slider.
    drag: Option<(u64, Drag)>,
    /// The palette of built-in controls to show or hide is open.
    palette: bool,
    /// The custom button being edited.
    pub selected_custom: Option<usize>,
    /// The key picker is open for `selected_custom`.
    pub(super) key_picker: bool,
    pub(super) key_page: usize,
    /// The picker lists keyboard keys rather than game actions.
    pub(super) raw_keys: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slider {
    Size,
    Opacity,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    Control([f32; 2]),
    Slider(Slider),
}

// The editor's buttons, as `[x, y, w, h]` fractions of the surface.
const DONE: [f32; 4] = [0.02, 0.04, 0.105, 0.075];
const CANCEL: [f32; 4] = [0.135, 0.04, 0.105, 0.075];
const RESET: [f32; 4] = [0.25, 0.04, 0.105, 0.075];
const PALETTE: [f32; 4] = [0.02, 0.13, 0.335, 0.075];
const APPLY_ALL: [f32; 4] = [0.32, 0.61, 0.36, 0.06];
const SLIDER: [f32; 4] = [0.32, 0.68, 0.36, 0.07];
const OPACITY: [f32; 4] = [0.32, 0.79, 0.17, 0.07];
const SIZE: [f32; 4] = [0.51, 0.79, 0.17, 0.07];
const REMOVE: [f32; 4] = [0.32, 0.88, 0.36, 0.07];
/// Where the slider's track starts and how wide it is.
const TRACK: [f32; 2] = [0.335, 0.33];
/// The sizes the size slider spans (`Placement::sanitize`'s range).
const SIZES: [f32; 2] = [0.045, 0.45];

/// Palette entry `index`, two columns of built-in controls.
pub(super) fn palette_rect(index: usize) -> [f32; 4] {
    [
        0.22 + (index % 2) as f32 * 0.29,
        0.17 + (index / 2) as f32 * 0.095,
        0.27,
        0.08,
    ]
}

impl TouchControls {
    /// Opens the editor; the touch controls stop acting until it closes.
    pub fn open_editor(&mut self) {
        self.clear();
        self.editor = Some(Editor {
            selected: None,
            slider: None,
            apply_all: false,
            original: self.settings.clone(),
            drag: None,
            palette: false,
            selected_custom: None,
            key_picker: false,
            key_page: 0,
            raw_keys: false,
        });
    }

    /// Closes the editor and puts the settings back as it opened.
    pub fn cancel_editor(&mut self) {
        if let Some(e) = self.editor.take() {
            self.settings = e.original;
        }
        self.clear();
    }

    /// A touch while the editor is open: the editor takes every finger, so
    /// editing can't move, mine or open menus. Returns how it closed, if it did.
    pub fn edit_touch(
        &mut self,
        id: u64,
        p: [f32; 2],
        started: bool,
        ended: bool,
    ) -> Option<EditorExit> {
        let e = self.editor.take()?;
        self.editing = true;
        let exit = self.edit_touch_with(e, id, p, started, ended);
        self.editing = false;
        exit
    }

    /// [`Self::edit_touch`] with the editor taken out; puts it back unless
    /// the touch closed it.
    fn edit_touch_with(
        &mut self,
        mut e: Editor,
        id: u64,
        p: [f32; 2],
        started: bool,
        ended: bool,
    ) -> Option<EditorExit> {
        self.clicked |= started;
        if self.edit_custom(&mut e, id, p, started, ended) {
            self.editor = Some(e);
            return None;
        }
        if started {
            if layout::contains(DONE, p) {
                self.clear();
                return Some(EditorExit::Done);
            }
            if layout::contains(CANCEL, p) {
                self.settings = e.original;
                self.clear();
                return Some(EditorExit::Cancelled);
            }
            self.edit_press(&mut e, id, p);
        }
        if let Some((owner, drag)) = e.drag
            && owner == id
            && let Some(selected) = e.selected
        {
            self.edit_drag(selected, drag, e.apply_all, p);
            if ended {
                e.drag = None;
            }
        }
        self.editor = Some(e);
        None
    }

    /// A finger landing in the editor (Done and Cancel aside).
    fn edit_press(&mut self, e: &mut Editor, id: u64, p: [f32; 2]) {
        let mode = self.settings.mode.index();
        let picked = e.selected.is_some();
        if layout::contains(RESET, p) {
            self.settings.layouts[mode] = Settings::default().layouts[mode];
            self.settings.button_visibility = [layout::FOLLOW_SCHEME; layout::CONTROLS];
            for (i, button) in self.settings.custom_buttons.iter_mut().enumerate() {
                button.layouts[mode] = CustomButton::new(String::new(), i).layouts[mode];
            }
        } else if layout::contains(PALETTE, p) {
            e.palette = !e.palette;
            e.selected = None;
            e.selected_custom = None;
            e.drag = None;
        } else if e.palette {
            if let Some(index) =
                (0..layout::CONTROLS).find(|&i| layout::contains(palette_rect(i), p))
            {
                let shown = self.visible(Control::ALL[index]);
                self.settings.button_visibility[index] =
                    if shown { layout::HIDDEN } else { layout::SHOWN };
            }
        } else if picked && layout::contains(REMOVE, p) {
            if let Some(c) = e.selected.take() {
                self.settings.button_visibility[c as usize] = layout::HIDDEN;
            }
            e.drag = None;
        } else if picked && e.slider.is_some() && layout::contains(APPLY_ALL, p) {
            e.apply_all = !e.apply_all;
        } else if let Some(slider) = e.slider.filter(|_| picked && layout::contains(SLIDER, p)) {
            e.drag = Some((id, Drag::Slider(slider)));
        } else if picked && layout::contains(OPACITY, p) {
            e.slider = Some(Slider::Opacity);
        } else if picked && layout::contains(SIZE, p) {
            e.slider = Some(Slider::Size);
        } else if let Some(c) = Control::ALL.into_iter().rev().find(|&c| {
            self.visible(c) && layout::contains(self.placement(c).rect(self.aspect()), p)
        }) {
            let center = self.placement(c).center;
            e.selected = Some(c);
            e.slider = None;
            e.drag = Some((id, Drag::Control([p[0] - center[0], p[1] - center[1]])));
        } else {
            e.selected = None;
            e.slider = None;
        }
    }

    /// A finger dragging the selected control, or the slider, to `p`.
    fn edit_drag(&mut self, selected: Control, drag: Drag, apply_all: bool, p: [f32; 2]) {
        let aspect = self.aspect();
        let scale = self.settings.button_scale;
        let mirror = self.settings.left_handed;
        let slot = self.settings.slot(selected);
        let row = &mut self.settings.layouts[self.settings.mode.index()];
        match drag {
            Drag::Control(offset) => {
                let place = &mut row[slot];
                let mut center = [p[0] - offset[0], p[1] - offset[1]];
                if mirror {
                    center[0] = 1.0 - center[0];
                }
                // Clamp by the size as drawn: a saved size may be wider than a
                // narrow portrait surface.
                let size = (place.size * scale).min(aspect).min(1.0);
                let half = [size / aspect / 2.0, size / 2.0];
                place.center = [
                    center[0].clamp(half[0], 1.0 - half[0]),
                    center[1].clamp(half[1], 1.0 - half[1]),
                ];
            }
            Drag::Slider(slider) => {
                let value = ((p[0] - TRACK[0]) / TRACK[1]).clamp(0.0, 1.0);
                for (i, place) in row.iter_mut().enumerate() {
                    if i != slot && !apply_all {
                        continue;
                    }
                    match slider {
                        Slider::Size => place.size = SIZES[0] + value * (SIZES[1] - SIZES[0]),
                        Slider::Opacity => place.opacity = value,
                    }
                }
            }
        }
    }

    /// The editor's toolbar, drawn under the controls.
    pub(super) fn draw_editor_toolbar(&self, b: &mut HudBatch, a: &HudAtlas) {
        for (rect, label) in [
            (DONE, "gui.done"),
            (CANCEL, "gui.cancel"),
            (RESET, "horizon.touch.editor.reset"),
            (PALETTE, "horizon.touch.editor.add"),
        ] {
            plate(b, a, rect, &lang::get(label), false);
        }
    }

    /// The editor's panels, drawn over the controls.
    pub(super) fn draw_editor(&self, b: &mut HudBatch, a: &HudAtlas) {
        let Some(e) = &self.editor else {
            return;
        };
        let (w, h) = (b.width, b.height);
        if let Some(selected) = e.selected {
            let p = self.placement(selected);
            let remove = lang::get("horizon.touch.editor.remove");
            plate(b, a, REMOVE, &remove, false);
            let opacity = lang::get("horizon.touch.editor.opacity");
            plate(b, a, OPACITY, &opacity, e.slider == Some(Slider::Opacity));
            let size = lang::get("horizon.touch.editor.size");
            plate(b, a, SIZE, &size, e.slider == Some(Slider::Size));
            if let Some(slider) = e.slider {
                b.fill(
                    a,
                    w * 0.31,
                    h * 0.59,
                    w * 0.38,
                    h * 0.18,
                    rgba(15, 15, 15, 235),
                );
                let state = lang::get(match e.apply_all {
                    true => "options.on",
                    false => "options.off",
                });
                let all = lang::get("horizon.touch.editor.apply_all");
                let label = lang::tr("options.generic_value", &[&all, &state]);
                plate(b, a, APPLY_ALL, &label, e.apply_all);
                let (value, percent) = match slider {
                    Slider::Size => ((p.size - SIZES[0]) / (SIZES[1] - SIZES[0]), p.size),
                    Slider::Opacity => (p.opacity, p.opacity),
                };
                let text = format!("{} {:.0}%", lang::get(selected.name()), percent * 100.0);
                b.text(
                    a,
                    &text,
                    w * 0.325,
                    h * 0.68,
                    rgba(245, 245, 245, 255),
                    true,
                );
                slider_bar(b, a, value);
            }
        }
        if e.palette {
            b.fill(
                a,
                w * 0.2,
                h * 0.14,
                w * 0.6,
                h * 0.72,
                rgba(15, 15, 15, 245),
            );
            for (index, control) in Control::ALL.into_iter().enumerate() {
                let shown = self.visible(control);
                let mark = if shown { "[-]" } else { "[+]" };
                let label = format!("{mark} {}", lang::get(control.name()));
                plate(b, a, palette_rect(index), &label, shown);
            }
        }
        self.draw_custom_editor(b, a, e);
    }

    /// The outline round the selected control.
    pub(super) fn draw_selection(&self, b: &mut HudBatch, a: &HudAtlas, [x, y, w, h]: [f32; 4]) {
        let white = rgba(255, 255, 255, 255);
        for [ex, ey, ew, eh] in [
            [x - 2.0, y - 2.0, w + 4.0, 1.0],
            [x - 2.0, y + h + 1.0, w + 4.0, 1.0],
            [x - 2.0, y - 2.0, 1.0, h + 4.0],
            [x + w + 1.0, y - 2.0, 1.0, h + 4.0],
        ] {
            b.fill(a, ex, ey, ew, eh, white);
        }
    }
}

/// An editor button at `rect` (surface fractions); `on` highlights it.
fn plate(b: &mut HudBatch, a: &HudAtlas, [x, y, w, h]: [f32; 4], label: &str, on: bool) {
    let (bw, bh) = (b.width, b.height);
    let color = rgba(255, 255, 255, 255);
    draw::button(b, a, [x * bw, y * bh, w * bw, h * bh], label, on, color);
}

/// The slider's track and knob at `value` (0..1).
fn slider_bar(b: &mut HudBatch, a: &HudAtlas, value: f32) {
    let (w, h) = (b.width, b.height);
    let [x, width] = TRACK.map(|v| v * w);
    b.fill(a, x, h * 0.725, width, h * 0.015, rgba(160, 160, 160, 255));
    b.fill(
        a,
        x + value * width - 3.0,
        h * 0.707,
        6.0,
        h * 0.05,
        rgba(255, 255, 255, 255),
    );
}

// The custom button panel and the key picker.
const CREATE: [f32; 4] = [0.22, 0.84, 0.56, 0.07];
const ACTION: [f32; 4] = [0.32, 0.54, 0.36, 0.07];
const PICK_ACTIONS: [f32; 4] = [0.15, 0.16, 0.33, 0.06];
const PICK_KEYS: [f32; 4] = [0.52, 0.16, 0.33, 0.06];
const PREVIOUS: [f32; 4] = [0.2, 0.86, 0.18, 0.07];
const BACK: [f32; 4] = [0.41, 0.86, 0.18, 0.07];
const NEXT: [f32; 4] = [0.62, 0.86, 0.18, 0.07];
/// Choices on one page of the key picker.
const PAGE: usize = 8;

/// Key picker row `i`.
fn key_rect(i: usize) -> [f32; 4] {
    [0.15, 0.25 + i as f32 * 0.072, 0.7, 0.06]
}

/// What a custom button can send, as (token, label): the game actions bound
/// to a key by default, or every keyboard key the game knows.
pub(super) fn choices(raw_keys: bool) -> Vec<(String, String)> {
    if raw_keys {
        crate::key_names::KEYBOARD
            .iter()
            .filter(|(_, name)| crate::keybinds::key_code(name).is_some())
            .map(|(_, name)| {
                let label = crate::keybinds::display_name(name).plain();
                ((*name).to_owned(), label)
            })
            .collect()
    } else {
        crate::keybinds::all_mappings()
            .filter(|m| m.1 != UNASSIGNED)
            .map(|(name, _, _)| {
                let label = crate::text::Text::translate(name, &[]).plain();
                (format!("action:{name}"), label)
            })
            .collect()
    }
}

impl TouchControls {
    /// Custom buttons in the editor: create one from the palette, drag it,
    /// size it, remove it, or pick its key or action. True when it took the touch.
    fn edit_custom(
        &mut self,
        e: &mut Editor,
        id: u64,
        p: [f32; 2],
        started: bool,
        ended: bool,
    ) -> bool {
        if started
            && [DONE, CANCEL, RESET, PALETTE]
                .iter()
                .any(|&r| layout::contains(r, p))
        {
            return false; // the toolbar
        }
        if e.key_picker {
            if started {
                self.pick_key(e, p);
            }
            return true;
        }
        if e.palette {
            if started && layout::contains(CREATE, p) && self.settings.custom_buttons.len() < 32 {
                let index = self.settings.custom_buttons.len();
                let button = CustomButton::new(UNASSIGNED.into(), index);
                self.settings.custom_buttons.push(button);
                e.selected = None;
                e.selected_custom = Some(index);
                e.key_picker = true;
                e.raw_keys = false;
                e.key_page = 0;
                e.palette = false;
                e.slider = None;
                e.drag = None;
                return true;
            }
            return false;
        }
        if started && let Some(index) = e.selected_custom {
            if layout::contains(ACTION, p) {
                e.key_picker = true;
                e.key_page = 0;
                e.drag = None;
                return true;
            }
            if layout::contains(REMOVE, p) {
                self.settings.custom_buttons.remove(index);
                e.selected_custom = None;
                e.drag = None;
                return true;
            }
            if layout::contains(OPACITY, p) {
                e.slider = Some(Slider::Opacity);
                return true;
            }
            if layout::contains(SIZE, p) {
                e.slider = Some(Slider::Size);
                return true;
            }
            match e.slider.filter(|_| layout::contains(SLIDER, p)) {
                Some(slider) => e.drag = Some((id, Drag::Slider(slider))),
                None => {
                    e.selected_custom = None;
                    e.drag = None;
                }
            }
        }
        if started
            && e.drag.is_none()
            && let Some(index) = self.custom_at(p)
        {
            let center = self.custom_placement(index).center;
            e.selected = None;
            e.selected_custom = Some(index);
            e.slider = None;
            e.apply_all = false;
            e.drag = Some((id, Drag::Control([p[0] - center[0], p[1] - center[1]])));
        }
        let Some(index) = e.selected_custom else {
            return false;
        };
        if let Some((owner, drag)) = e.drag
            && owner == id
        {
            self.drag_custom(index, drag, p);
            if ended {
                e.drag = None;
            }
        }
        true
    }

    fn drag_custom(&mut self, index: usize, drag: Drag, p: [f32; 2]) {
        let aspect = self.aspect();
        let scale = self.settings.button_scale;
        let mirror = self.settings.left_handed;
        let place = &mut self.settings.custom_buttons[index].layouts[self.settings.mode.index()];
        match drag {
            Drag::Control(offset) => {
                let mut center = [p[0] - offset[0], p[1] - offset[1]];
                if mirror {
                    center[0] = 1.0 - center[0];
                }
                let size = (place.size * scale).min(aspect).min(1.0);
                let half = [size / aspect / 2.0, size / 2.0];
                place.center = [
                    center[0].clamp(half[0], 1.0 - half[0]),
                    center[1].clamp(half[1], 1.0 - half[1]),
                ];
            }
            Drag::Slider(slider) => {
                let value = ((p[0] - TRACK[0]) / TRACK[1]).clamp(0.0, 1.0);
                match slider {
                    Slider::Size => place.size = SIZES[0] + value * (SIZES[1] - SIZES[0]),
                    Slider::Opacity => place.opacity = value,
                }
            }
        }
    }

    /// A tap in the key picker.
    fn pick_key(&mut self, e: &mut Editor, p: [f32; 2]) {
        let choices = choices(e.raw_keys);
        if layout::contains(PICK_ACTIONS, p) || layout::contains(PICK_KEYS, p) {
            e.raw_keys = layout::contains(PICK_KEYS, p);
            e.key_page = 0;
        } else if layout::contains(PREVIOUS, p) {
            e.key_page = e.key_page.saturating_sub(1);
        } else if layout::contains(BACK, p) {
            e.key_picker = false;
        } else if layout::contains(NEXT, p) {
            e.key_page = (e.key_page + 1).min(choices.len().saturating_sub(1) / PAGE);
        } else if let Some((token, _)) = choices
            .into_iter()
            .skip(e.key_page * PAGE)
            .take(PAGE)
            .enumerate()
            .find_map(|(i, choice)| layout::contains(key_rect(i), p).then_some(choice))
            && let Some(index) = e.selected_custom
        {
            self.settings.custom_buttons[index].key = token;
            e.key_picker = false;
        }
    }

    /// A key pressed on a keyboard while the key picker is open becomes the
    /// button's key. False when the picker is closed or the key unknown.
    pub fn capture_custom_key(&mut self, name: &str) -> bool {
        if crate::keybinds::key_code(name).is_none() {
            return false;
        }
        let Some(e) = &mut self.editor else {
            return false;
        };
        let Some(index) = e.selected_custom.filter(|_| e.key_picker) else {
            return false;
        };
        self.settings.custom_buttons[index].key = name.into();
        e.key_picker = false;
        true
    }

    fn draw_custom_editor(&self, b: &mut HudBatch, a: &HudAtlas, e: &Editor) {
        let (w, h) = (b.width, b.height);
        if e.palette {
            let create = lang::get("horizon.touch.editor.create");
            plate(b, a, CREATE, &create, false);
        }
        if let Some(index) = e.selected_custom
            && !e.palette
            && !e.key_picker
        {
            let button = &self.settings.custom_buttons[index];
            let p = button.layouts[self.settings.mode.index()];
            let action = lang::tr("horizon.touch.editor.action", &[&button.label()]);
            plate(b, a, ACTION, &action, false);
            let opacity = lang::get("horizon.touch.editor.opacity");
            plate(b, a, OPACITY, &opacity, e.slider == Some(Slider::Opacity));
            let size = lang::get("horizon.touch.editor.size");
            plate(b, a, SIZE, &size, e.slider == Some(Slider::Size));
            let remove = lang::get("horizon.touch.editor.remove");
            plate(b, a, REMOVE, &remove, false);
            if let Some(slider) = e.slider {
                b.fill(
                    a,
                    w * 0.3,
                    h * 0.66,
                    w * 0.4,
                    h * 0.11,
                    rgba(15, 15, 15, 235),
                );
                let (name, value, percent) = match slider {
                    Slider::Size => (size, (p.size - SIZES[0]) / (SIZES[1] - SIZES[0]), p.size),
                    Slider::Opacity => (opacity, p.opacity, p.opacity),
                };
                let text = format!("{name} {:.0}%", percent * 100.0);
                b.text(
                    a,
                    &text,
                    w * 0.325,
                    h * 0.68,
                    rgba(245, 245, 245, 255),
                    true,
                );
                slider_bar(b, a, value);
            }
        }
        if e.key_picker {
            b.fill(
                a,
                w * 0.12,
                h * 0.14,
                w * 0.76,
                h * 0.81,
                rgba(15, 15, 15, 250),
            );
            let actions = lang::get("horizon.touch.editor.game_action");
            plate(b, a, PICK_ACTIONS, &actions, !e.raw_keys);
            let keys = lang::get("horizon.touch.editor.keyboard_key");
            plate(b, a, PICK_KEYS, &keys, e.raw_keys);
            let choices = choices(e.raw_keys);
            for (i, (_, label)) in choices
                .iter()
                .skip(e.key_page * PAGE)
                .take(PAGE)
                .enumerate()
            {
                plate(b, a, key_rect(i), label, false);
            }
            for (rect, key) in [
                (PREVIOUS, "horizon.touch.editor.previous"),
                (BACK, "gui.back"),
                (NEXT, "horizon.touch.editor.next"),
            ] {
                plate(b, a, rect, &lang::get(key), false);
            }
        }
    }
}
