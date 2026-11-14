//! The pause menu's Report Bugs icon when a crate registered a page at
//! `horizon_hooks::pages::Place::ReportBugs`: that page as a form over the world, drawn and
//! edited as the title screen draws a `Place::Title` page (`multiplayer::Form`). Without one the
//! icon opens vanilla's link (game/screens.rs).
use super::{Game, Screen};
use horizon_client::{
    hud::HudBatch,
    multiplayer::{self, Form, Outcome},
    text::Text,
    ui::{self, Caret, EditBox},
};
use std::time::Instant;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

/// The open form, and what its fields keep between frames.
pub(super) struct ReportForm {
    form: Option<Form>,
    caret: Caret,
    /// When the focused field got the focus, for the caret's blink.
    focused_at: Instant,
    /// The button Tab moved to; None while a field has the focus.
    button: Option<u8>,
}

impl Default for ReportForm {
    fn default() -> Self {
        Self {
            form: None,
            caret: Caret::default(),
            focused_at: Instant::now(),
            button: None,
        }
    }
}

impl Game<'_> {
    /// Opens the registered report page as a form; false when no crate registered one.
    pub(super) fn open_report_form(&mut self) -> bool {
        let Some(page) = multiplayer::report_bugs_page() else {
            return false;
        };
        self.report = ReportForm {
            form: Some(Form::open(page)),
            ..ReportForm::default()
        };
        self.screen = Screen::ReportForm;
        true
    }

    /// Back to the pause menu; the page keeps what it was told.
    pub(super) fn close_report_form(&mut self) {
        self.report = ReportForm::default();
        self.screen = Screen::Paused;
    }

    pub(super) fn draw_report_form(&mut self, batch: &mut HudBatch) {
        let ReportForm {
            form,
            caret,
            focused_at,
            button,
        } = &mut self.report;
        let Some(form) = form.as_ref() else {
            return;
        };
        horizon_client::widgets::begin_screen(|| form.narration());
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        form.draw(atlas, batch, caret, *focused_at);
        for (x, y, bw, label, active, id) in form.buttons(w, h) {
            let hover = active && (ui::hovered(self.cursor, x, y, bw, 20.0) || *button == Some(id));
            let label = Text::literal(label);
            match multiplayer::button_action(id) {
                Some(action) => {
                    ui::action_button(batch, atlas, action, x, y, bw, 20.0, &label, active, hover)
                }
                None => ui::button(batch, atlas, x, y, bw, 20.0, &label, active, hover),
            }
        }
    }

    pub(super) fn report_form_click(&mut self) {
        let (w, h) = self.gui_size();
        let cursor = self.cursor;
        let shift = self.has_shift_down();
        let ReportForm {
            form,
            caret,
            focused_at,
            button,
        } = &mut self.report;
        let Some(form) = form.as_mut() else {
            return;
        };
        let hit = form
            .buttons(w, h)
            .into_iter()
            .find(|b| b.4 && ui::hovered(cursor, b.0, b.1, b.2, 20.0))
            .map(|b| b.5);
        if hit.is_some() {
            self.audio.play_button_click();
        }
        let outcome = form.click(hit, cursor.0, cursor.1);
        if hit.is_none()
            && let Some(rect) = form.focused_box()
            && ui::hovered(cursor, rect[0], rect[1], rect[2], rect[3])
            && let Some(field) = form.focused_text()
        {
            // EditBox.onClick: the caret goes under the pointer.
            caret.press(&self.gui.atlas, field, rect, cursor.0, shift);
            *focused_at = Instant::now();
            *button = None;
        }
        if let Outcome::Back = outcome {
            self.close_report_form();
        }
    }

    /// Escape leaves, Tab moves through the fields and buttons, Enter presses or submits, the
    /// rest edits the focused field.
    pub(super) fn report_form_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let PhysicalKey::Code(key) = event.physical_key else {
            return;
        };
        // Select all, copy, paste and cut in the focused field.
        if let Some(shortcut) = self.edit_shortcut(event) {
            let ReportForm {
                form,
                caret,
                button,
                ..
            } = &mut self.report;
            if button.is_none()
                && let Some((id, value, max)) = form.as_mut().and_then(Form::focused_text)
            {
                let board = &mut self.clipboard;
                caret.edit(id, value, max, |edit| edit.shortcut(shortcut, board));
            }
            return;
        }
        let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
        let (w, h) = self.gui_size();
        let Some(form) = self.report.form.as_mut() else {
            return;
        };
        let outcome = match key {
            KeyCode::Escape => Outcome::Back,
            KeyCode::Tab => {
                let ids: Vec<u8> = form
                    .buttons(w, h)
                    .iter()
                    .filter(|b| b.4)
                    .map(|b| b.5)
                    .collect();
                form.tab_dir(shift, &ids, &mut self.report.button);
                self.report.focused_at = Instant::now();
                Outcome::None
            }
            KeyCode::Enter | KeyCode::NumpadEnter => match self.report.button {
                Some(id) => {
                    self.audio.play_button_click();
                    form.click(Some(id), 0.0, 0.0)
                }
                None => form.confirm(),
            },
            _ => {
                if !self.report_form_edit(|edit| edit.key(key, ctrl, shift))
                    && self.types_text()
                    && let Some(text) = event.text.as_deref()
                {
                    self.report_form_type(text);
                }
                Outcome::None
            }
        };
        if let Outcome::Back = outcome {
            self.close_report_form();
        }
    }

    /// Edits the focused field through its `EditBox`; false when no field has the focus.
    fn report_form_edit(&mut self, f: impl FnOnce(&mut EditBox) -> bool) -> bool {
        let ReportForm {
            form,
            caret,
            button,
            ..
        } = &mut self.report;
        if button.is_some() {
            return false;
        }
        match form.as_mut().and_then(Form::focused_text) {
            Some((id, value, max)) => caret.edit(id, value, max, f),
            None => false,
        }
    }

    /// Whether a field has the focus: typing goes to it.
    pub(super) fn report_form_typing(&self) -> bool {
        self.report.form.is_some() && self.report.button.is_none()
    }

    pub(super) fn report_form_type(&mut self, text: &str) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.report_form_edit(|edit| {
            edit.insert_text(&text);
            true
        });
    }

    pub(super) fn report_form_backspace(&mut self) {
        self.report_form_edit(|edit| edit.key(KeyCode::Backspace, false, false));
    }
}
