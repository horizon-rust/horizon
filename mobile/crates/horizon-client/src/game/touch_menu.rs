//! The touch overlay's menu (≡): routing a touch to the right place (the menu, the on-screen
//! controls while playing, or the pointer in menus), its actions, its saved settings and the
//! drawing (`horizon_client::touch_ui`).
use super::*;
use horizon_client::touch_ui::{self, Hit, MenuState, Slider};

const SETTINGS_FILE: &str = "touch_settings.txt";

/// A key or mouse button a touch button presses.
#[derive(Clone, Copy, Debug)]
pub(super) enum TouchOut {
    /// A key as a keyboard sends it: the game's key binds decide what it does.
    Key(KeyCode),
    /// An action's default key, pressed for the action itself, whatever its binding is (the Jump, Sneak
    /// and Sprint buttons).
    Action(KeyCode),
    Mouse(MouseButton),
}

/// The key or mouse button named in `horizon_touch::KEYS` (winit's `KeyCode` names).
pub(super) fn touch_key_output(name: &str) -> Option<TouchOut> {
    use KeyCode::*;
    let code = match name {
        "MouseLeft" => return Some(TouchOut::Mouse(MouseButton::Left)),
        "MouseRight" => return Some(TouchOut::Mouse(MouseButton::Right)),
        "MouseMiddle" => return Some(TouchOut::Mouse(MouseButton::Middle)),
        "KeyA" => KeyA,
        "KeyB" => KeyB,
        "KeyC" => KeyC,
        "KeyD" => KeyD,
        "KeyE" => KeyE,
        "KeyF" => KeyF,
        "KeyG" => KeyG,
        "KeyH" => KeyH,
        "KeyI" => KeyI,
        "KeyJ" => KeyJ,
        "KeyK" => KeyK,
        "KeyL" => KeyL,
        "KeyM" => KeyM,
        "KeyN" => KeyN,
        "KeyO" => KeyO,
        "KeyP" => KeyP,
        "KeyQ" => KeyQ,
        "KeyR" => KeyR,
        "KeyS" => KeyS,
        "KeyT" => KeyT,
        "KeyU" => KeyU,
        "KeyV" => KeyV,
        "KeyW" => KeyW,
        "KeyX" => KeyX,
        "KeyY" => KeyY,
        "KeyZ" => KeyZ,
        "Digit0" => Digit0,
        "Digit1" => Digit1,
        "Digit2" => Digit2,
        "Digit3" => Digit3,
        "Digit4" => Digit4,
        "Digit5" => Digit5,
        "Digit6" => Digit6,
        "Digit7" => Digit7,
        "Digit8" => Digit8,
        "Digit9" => Digit9,
        "F1" => F1,
        "F2" => F2,
        "F3" => F3,
        "F4" => F4,
        "F5" => F5,
        "F6" => F6,
        "F7" => F7,
        "F8" => F8,
        "F9" => F9,
        "F10" => F10,
        "F11" => F11,
        "F12" => F12,
        "Escape" => Escape,
        "Tab" => Tab,
        "Enter" => Enter,
        "Space" => Space,
        "ShiftLeft" => ShiftLeft,
        "ControlLeft" => ControlLeft,
        "AltLeft" => AltLeft,
        "Backspace" => Backspace,
        "ArrowUp" => ArrowUp,
        "ArrowDown" => ArrowDown,
        "ArrowLeft" => ArrowLeft,
        "ArrowRight" => ArrowRight,
        "Slash" => Slash,
        "Backquote" => Backquote,
        "Minus" => Minus,
        "Equal" => Equal,
        "Comma" => Comma,
        "Period" => Period,
        "Semicolon" => Semicolon,
        "Quote" => Quote,
        "BracketLeft" => BracketLeft,
        "BracketRight" => BracketRight,
        "Backslash" => Backslash,
        _ => return None,
    };
    Some(TouchOut::Key(code))
}

impl Game<'_> {
    /// Where a touch goes: the open menu; else the ≡ button; else the on-screen controls while
    /// playing (and in their edit mode); else the pointer, in every menu and screen.
    pub(super) fn touch_route(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        touch: winit::event::Touch,
    ) {
        use winit::event::TouchPhase;
        self.touch_sync();
        self.touch_seen = true;
        self.last_input = Instant::now();
        let pos = [touch.location.x as f32, touch.location.y as f32];
        if self.touch_rename.is_some() {
            self.touch_rename_event(&touch);
            return;
        }
        if self.touch_picker {
            self.touch_picker_event(&touch);
            return;
        }
        if self.touch_menu.open {
            self.touch_menu_event(event_loop, id, &touch);
            return;
        }
        let g = self.touch.gear_rect();
        if touch.phase == TouchPhase::Started && !self.touch.is_editing() && g.contains(pos) {
            self.touch.reset();
            self.touch_pointer = None;
            self.touch_menu.open = true;
            return;
        }
        let gameplay = self.screen == Screen::Playing && self.captured;
        // Back (Esc) on every menu screen, where the on-screen buttons are not shown.
        let b = self.touch.back_rect();
        if touch.phase == TouchPhase::Started
            && !gameplay
            && !self.touch.is_editing()
            && b.contains(pos)
        {
            self.touch_back();
            return;
        }
        // Keys: any key to the game, on menu screens where the buttons are not shown.
        let k = self.touch.keys_rect();
        if touch.phase == TouchPhase::Started
            && !gameplay
            && !self.touch.is_editing()
            && k.contains(pos)
        {
            self.touch_picker = true;
            self.touch_picker_send = true;
            self.touch_picker_tab = 0;
            return;
        }
        if gameplay || self.touch.is_editing() {
            self.touch_pointer = None;
            self.touch_event(&touch);
            // The Key tool asks for the picker.
            if self.touch.take_picker_request() {
                self.touch_picker = true;
                self.touch_picker_tab = 0;
            }
            if self.touch.take_rename_request() {
                self.touch_rename = Some(self.touch.selected_label().unwrap_or_default());
            }
        } else {
            self.touch.reset();
            self.touch_as_mouse(event_loop, id, touch);
        }
    }

    /// Esc, as a keyboard sends it: closes the open screen (the title screen has its own escape).
    pub(super) fn touch_back(&mut self) {
        if let Some(t) = self.title.as_mut() {
            t.escape();
            self.save_title_options();
        } else {
            let _ = self.key_pressed(KeyCode::Escape);
        }
    }

    /// A touch while naming a button: Done saves, Cancel (or outside) drops, the field brings the
    /// system keyboard up (the text arrives as `Ime` events, see `touch_rename_text`).
    fn touch_rename_event(&mut self, touch: &winit::event::Touch) {
        use touch_ui::RenameHit;
        if touch.phase != winit::event::TouchPhase::Started {
            return;
        }
        let (density, size) = self.touch_ui_scale();
        let (x, y) = (
            touch.location.x as f32 / density,
            touch.location.y as f32 / density,
        );
        let mut batch = HudBatch::new(size);
        let ui = horizon_client::ui_theme::Ui::new(&self.gui.atlas, &mut batch, (0.0, 0.0));
        let frame = touch_ui::layout_rename(&ui);
        drop(ui);
        match touch_ui::rename_hit(&frame, x, y) {
            RenameHit::Done => self.touch_rename_done(true),
            RenameHit::Cancel | RenameHit::Backdrop => self.touch_rename_done(false),
            RenameHit::Field | RenameHit::Panel => {}
        }
    }

    pub(super) fn touch_rename_done(&mut self, save: bool) {
        if let Some(text) = self.touch_rename.take()
            && save
        {
            self.touch.set_selected_label(&text);
        }
    }

    /// Text typed (the system keyboard's `Ime::Commit`, or a key) while a name is being entered.
    /// True when it was taken.
    pub(super) fn touch_rename_text(&mut self, event: &WindowEvent) -> bool {
        let Some(text) = self.touch_rename.as_mut() else {
            return false;
        };
        match event {
            WindowEvent::Ime(winit::event::Ime::Commit(t)) => {
                text.push_str(t);
                text.truncate(text.char_indices().nth(12).map_or(text.len(), |(i, _)| i));
                true
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::Backspace) => {
                            text.pop();
                        }
                        PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter) => {
                            self.touch_rename_done(true)
                        }
                        PhysicalKey::Code(KeyCode::Escape) => self.touch_rename_done(false),
                        _ => {
                            if let Some(t) = event
                                .text
                                .as_deref()
                                .filter(|t| t.chars().all(|c| !c.is_control()))
                            {
                                text.push_str(t);
                                text.truncate(
                                    text.char_indices().nth(12).map_or(text.len(), |(i, _)| i),
                                );
                            }
                        }
                    }
                }
                true
            }
            WindowEvent::Ime(_) => true,
            _ => false,
        }
    }

    fn touch_ui_scale(&self) -> (f32, [u32; 2]) {
        let size = self
            .renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height]);
        let d = (size[0] as f32 / 1040.0)
            .min(size[1] as f32 / 700.0)
            .clamp(1.0, 4.0);
        (d, size)
    }

    fn touch_menu_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        touch: &winit::event::Touch,
    ) {
        use winit::event::TouchPhase;
        let (density, size) = self.touch_ui_scale();
        let (x, y) = (
            touch.location.x as f32 / density,
            touch.location.y as f32 / density,
        );
        match touch.phase {
            TouchPhase::Started => {
                let mut batch = HudBatch::new(size);
                let atlas = &self.gui.atlas;
                let ui = horizon_client::ui_theme::Ui::new(atlas, &mut batch, (0.0, 0.0));
                let frame = touch_ui::layout_menu(&ui, &self.touch_menu);
                let (hit, slider) = touch_ui::hit(&frame, x, y);
                drop(ui);
                match hit {
                    Hit::Backdrop => self.touch_menu.open = false,
                    Hit::Panel => {}
                    Hit::Slider(which) => {
                        if let Some((left, width, value)) = slider {
                            self.touch_slider = Some((left, width, which));
                            self.set_slider(which, value);
                        }
                    }
                    Hit::EditLayout => {
                        self.touch_menu.open = false;
                        self.touch.set_editing(true);
                    }
                    Hit::ResetLayout => self.touch.reset_layout(),
                    Hit::ToggleStick => {
                        let stick = !self.touch.has_stick();
                        self.touch.set_movement_stick(stick);
                        self.touch_menu.stick = stick;
                    }
                    Hit::Launcher => {
                        self.touch_menu.open = false;
                        back_to_launcher();
                    }
                    Hit::ToggleControls => {
                        self.touch_menu.hidden = !self.touch_menu.hidden;
                        self.touch.hidden = self.touch_menu.hidden;
                        self.save_touch_settings();
                    }
                    Hit::ToggleFps => {
                        self.touch_menu.show_fps = !self.touch_menu.show_fps;
                        self.save_touch_settings();
                    }
                    Hit::CopyLog => {
                        self.touch_menu.open = false;
                        copy_game_log();
                    }
                    Hit::PauseMenu => {
                        self.touch_menu.open = false;
                        // Esc: the pause menu in a world (with Save and Quit to Title), back from a screen.
                        let _ = self.key_pressed(KeyCode::Escape);
                    }
                    Hit::Quit => {
                        self.touch_menu.open = false;
                        self.window_event(event_loop, id, WindowEvent::CloseRequested);
                    }
                }
            }
            TouchPhase::Moved => {
                if let Some((left, width, which)) = self.touch_slider {
                    self.set_slider(which, touch_ui::slider_value((left, width), x));
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if self.touch_slider.take().is_some() {
                    self.save_touch_settings();
                }
            }
        }
    }

    /// A touch while the key picker is open: a chip binds the selected button, outside closes.
    fn touch_picker_event(&mut self, touch: &winit::event::Touch) {
        use touch_ui::PickerHit;
        if touch.phase != winit::event::TouchPhase::Started {
            return;
        }
        let (density, size) = self.touch_ui_scale();
        let (x, y) = (
            touch.location.x as f32 / density,
            touch.location.y as f32 / density,
        );
        let mut batch = HudBatch::new(size);
        let ui = horizon_client::ui_theme::Ui::new(&self.gui.atlas, &mut batch, (0.0, 0.0));
        let frame = touch_ui::layout_picker(&ui, self.touch_picker_tab);
        drop(ui);
        match touch_ui::picker_hit(&frame, x, y) {
            PickerHit::Tab(t) => self.touch_picker_tab = t,
            PickerHit::Key(k) => {
                if self.touch_picker_send {
                    self.touch_send_key(k);
                } else {
                    self.touch.bind_selected(k);
                }
                self.touch_picker = false;
                self.touch_picker_send = false;
            }
            PickerHit::Close => {
                self.touch_picker = false;
                self.touch_picker_send = false;
            }
            PickerHit::Panel => {}
        }
    }

    /// One press of `KEYS[index]` to the game: the keybind screen takes it as the new binding; else it
    /// is a key press (F3 and its combinations included).
    fn touch_send_key(&mut self, index: u8) {
        use horizon_client::debug_keys::KeyAction;
        let Some(out) = touch_key_output(horizon_touch::KEYS[usize::from(index)].0) else {
            return;
        };
        let waiting = self.options_waiting_for_key();
        let result = match out {
            TouchOut::Mouse(button) if waiting => self.options_bind_mouse(button),
            TouchOut::Mouse(_) => Ok(()),
            TouchOut::Key(code) | TouchOut::Action(code) if waiting => self.options_key(code, None),
            TouchOut::Key(code) | TouchOut::Action(code) => {
                if self.debug_key(code, KeyAction::Press) {
                    let _ = self.debug_key(code, KeyAction::Release);
                    Ok(())
                } else {
                    let r = self.key_pressed(code);
                    let _ = self.debug_key(code, KeyAction::Release);
                    r
                }
            }
        };
        if let Err(e) = result {
            log::warn!("Horizon: could not send the key: {e}");
        }
    }

    fn set_slider(&mut self, which: Slider, value: f32) {
        match which {
            Slider::ButtonOpacity => self.set_button_opacity(value),
            Slider::LookSensitivity => {
                let s = (0.5 + 2.5 * value).clamp(0.5, 3.0);
                self.touch_menu.sensitivity = s;
                self.touch.look_sensitivity = s;
            }
        }
    }

    fn set_button_opacity(&mut self, value: f32) {
        let opacity = (0.2 + 0.8 * value).clamp(0.2, 1.0);
        self.touch_menu.opacity = opacity;
        self.touch.opacity = opacity;
    }

    pub(super) fn load_touch_settings(&mut self) {
        let Ok(text) = std::fs::read_to_string(SETTINGS_FILE) else {
            return;
        };
        for line in text.lines() {
            let mut it = line.split_whitespace();
            match (it.next(), it.next()) {
                (Some("opacity"), Some(v)) => {
                    if let Ok(v) = v.parse::<f32>() {
                        self.set_button_opacity(((v - 0.2) / 0.8).clamp(0.0, 1.0));
                    }
                }
                (Some("hidden"), Some(v)) => {
                    self.touch_menu.hidden = v == "1";
                    self.touch.hidden = self.touch_menu.hidden;
                }
                (Some("fps"), Some(v)) => self.touch_menu.show_fps = v == "1",
                (Some("sensitivity"), Some(v)) => {
                    if let Ok(v) = v.parse::<f32>() {
                        self.set_slider(Slider::LookSensitivity, ((v - 0.5) / 2.5).clamp(0.0, 1.0));
                    }
                }
                _ => {}
            }
        }
    }

    fn save_touch_settings(&self) {
        let m: &MenuState = &self.touch_menu;
        let text = format!(
            "opacity {:.2}\nhidden {}\nfps {}\nsensitivity {:.2}\n",
            m.opacity,
            u8::from(m.hidden),
            u8::from(m.show_fps),
            m.sensitivity
        );
        let _ = std::fs::write(SETTINGS_FILE, text);
    }

    /// Draws the touch overlay: the controls while playing (and in edit mode), the ≡ button and the
    /// open menu, and a mouse arrow in menus.
    pub(super) fn draw_touch(&mut self, batch: &mut HudBatch) {
        if !self.touch_seen {
            return;
        }
        // The window's size and the saved layout, before the first touch too.
        self.touch_sync();
        // Frames a second, for the ≡ button.
        self.touch_fps.1 += 1;
        if self.touch_fps.0.elapsed() >= Duration::from_secs(1) {
            self.touch_menu.fps = self.touch_fps.1;
            self.touch_fps = (Instant::now(), 0);
        }
        self.touch_menu.stick = self.touch.has_stick();
        // The keybind screen is waiting for a key: offer the keys (a touch screen has none).
        let waiting = self.options_waiting_for_key();
        if waiting && !self.touch_waiting_prev && !self.touch_picker {
            self.touch_picker = true;
            self.touch_picker_send = true;
            self.touch_picker_tab = 0;
        }
        self.touch_waiting_prev = waiting;
        let gameplay = self.screen == Screen::Playing && self.captured;
        let editing = self.touch.is_editing();
        let items = self.touch.ui_items();
        let atlas = &self.gui.atlas;
        let mut ui = horizon_client::ui_theme::Ui::new(atlas, batch, self.cursor);
        let fps = self.touch_menu.show_fps.then_some(self.touch_menu.fps);
        if gameplay || editing {
            let stick = self
                .touch
                .joystick()
                .map(|(o, n)| (o, n, self.touch.joystick_radius));
            touch_ui::draw_controls(
                &mut ui,
                &items,
                self.touch_menu.opacity,
                fps,
                editing,
                stick,
            );
        } else {
            // In menus the ≡ and Back buttons only.
            let gear: Vec<_> = items
                .into_iter()
                .filter(|i| {
                    matches!(
                        i.kind,
                        horizon_touch::UiKind::Gear
                            | horizon_touch::UiKind::Back
                            | horizon_touch::UiKind::Keys
                    )
                })
                .collect();
            touch_ui::draw_controls(&mut ui, &gear, self.touch_menu.opacity, fps, false, None);
        }
        if let Some(text) = &self.touch_rename {
            let frame = touch_ui::layout_rename(&ui);
            touch_ui::draw_rename(
                &mut ui,
                &frame,
                text,
                self.touch_epoch.elapsed().as_millis() / 500 % 2 == 0,
            );
        } else if self.touch_picker {
            let frame = touch_ui::layout_picker(&ui, self.touch_picker_tab);
            let title = if self.touch_picker_send {
                "Press a key"
            } else {
                "Bind a key"
            };
            touch_ui::draw_picker(&mut ui, &frame, self.touch_picker_tab, None, title);
        } else if self.touch_menu.open {
            let frame = touch_ui::layout_menu(&ui, &self.touch_menu);
            touch_ui::draw_menu(&mut ui, &frame);
        } else if !gameplay && !editing {
            let (cx, cy) = ui.cursor;
            touch_ui::draw_cursor(&mut ui, cx, cy);
        }
        ui.finish();
    }
}

/// Back to the launcher screen (sign in, instances, mods): a fresh start of the app that shows it.
fn back_to_launcher() {
    // The next start shows the launcher unless a Play asked for the game.
    let _ = std::fs::remove_file(horizon_client::launch::START_MARKER);
    #[cfg(target_os = "android")]
    if let Err(e) = horizon_client::android::restart() {
        log::error!("Horizon: could not go back to the launcher: {e}");
    }
    #[cfg(not(target_os = "android"))]
    log::info!("Horizon: back to the launcher: start the game again");
}

/// The end of the game's log on the clipboard, for a bug report.
fn copy_game_log() {
    let mut text = String::from("Horizon game log\n");
    if let Some(home) = std::env::var_os("HORIZON_HOME") {
        if let Ok(log) =
            std::fs::read_to_string(std::path::Path::new(&home).join("logs/latest.log"))
        {
            let lines: Vec<&str> = log.lines().collect();
            for line in &lines[lines.len().saturating_sub(150)..] {
                text.push_str(line);
                text.push('\n');
            }
        }
    }
    if text.len() > 40_000 {
        text = text[text.len() - 40_000..].to_string();
    }
    #[cfg(target_os = "android")]
    if let Err(e) = horizon_client::android::copy_text(&text) {
        log::warn!("Horizon: could not copy the log: {e}");
    }
    #[cfg(not(target_os = "android"))]
    {
        use horizon_client::clipboard::Clipboard;
        horizon_client::clipboard::SystemClipboard::default().set(&text);
    }
}
