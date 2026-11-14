// Adapted from Starling (https://github.com/starling-client/starling, `android` branch at
// 150a414b09a1, `crates/horizon-client-render/src/touch/editor.rs`), written by Vasysik and the
// Starling contributors. Copyright (c) 2026 Ludovic Marie and the Horizon contributors, under the
// same MIT license as Horizon (LICENSE at the repository root).

//! The touch layout editor, drawn over the game: drag a control to move it,
//! pick it to change its size or opacity (or every control's at once), hide
//! it, or bring hidden ones back from the palette. Reset restores the current
//! scheme's factory layout; Cancel restores everything as it was opened.
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
