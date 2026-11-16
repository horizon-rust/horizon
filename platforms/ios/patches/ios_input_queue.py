#!/usr/bin/env python3
"""Add an iOS-only input queue to Horizon's input layer, bridged from UIKit.

CI modifies the checkout only. All original Horizon files remain upstream-owned
in Git history. The game consumes virtual keys via its existing key mapping,
and mouse press/motion via its original winit event handlers.
"""
from pathlib import Path

def change(path, old, new):
    file = Path(path)
    source = file.read_text()
    assert source.count(old) == 1, f"{path}: upstream changed near {old[:80]!r}"
    file.write_text(source.replace(old, new, 1))

queue_path = Path("crates/horizon-client-input/src/ios_input.rs")
assert not queue_path.exists(), "Upstream now owns ios_input; reassess integration"
queue_path.write_text("//! Nova iOS input bridge. The gameplay runtime calls these safe functions;\n//! only the iOS host's C ABI needs raw pointers. Nothing runs on desktop.\nuse std::sync::{\n    Mutex, OnceLock,\n    atomic::{AtomicBool, Ordering},\n};\n\n#[derive(Debug)]\npub enum Input {\n    Key(u32, bool),\n    Mouse(u32, bool),\n    Cursor(f64, f64),\n    Motion(f64, f64),\n    Text(String),\n    KeyboardClosed,\n    Close,\n}\n\nstatic INPUTS: OnceLock<Mutex<Vec<Input>>> = OnceLock::new();\nstatic TOUCH_GESTURES: AtomicBool = AtomicBool::new(true);\n\nfn input_queue() -> &'static Mutex<Vec<Input>> {\n    INPUTS.get_or_init(|| Mutex::new(Vec::new()))\n}\n\npub fn push(event: Input) {\n    if let Ok(mut inputs) = input_queue().lock() {\n        // Ignore excessive events instead of exhausting the main thread.\n        if inputs.len() < 2048 {\n            inputs.push(event);\n        }\n    }\n}\n\npub fn take() -> Vec<Input> {\n    input_queue()\n        .lock()\n        .map(|mut inputs| std::mem::take(&mut *inputs))\n        .unwrap_or_default()\n}\n\npub fn set_touch_gestures(value: bool) {\n    TOUCH_GESTURES.store(value, Ordering::Relaxed);\n}\n\npub fn touch_gestures() -> bool {\n    TOUCH_GESTURES.load(Ordering::Relaxed)\n}\n")

change("crates/horizon-client-input/src/lib.rs", "pub mod input;\n",
       'pub mod input;\n#[cfg(target_os = "ios")]\npub mod ios_input;\n')

keymap = """    #[cfg(target_os = "ios")]
    fn ios_key_code(code: u32) -> Option<KeyCode> {
        Some(match code {
KEY_MAP
            _ => return None,
        })
    }

    #[cfg(target_os = "ios")]
    fn ios_drain_inputs(&mut self, event_loop: &ActiveEventLoop) {
        use horizon_client_input::ios_input::Input;
        let id = match self {
            App::Loading(loading) => loading.window.as_ref().map(|w| w.id()),
            App::Ready(game) => game.host.window.as_ref().map(|w| w.id()),
        };
        let Some(id) = id else { return };
        for input in horizon_client_input::ios_input::take() {
            match input {
                Input::Key(code, down) => {
                    let Some(key) = Self::ios_key_code(code) else { continue };
                    // Winit KeyEvent has private platform fields: use Horizon's
                    // supported mapped-key input path instead of fabricating one.
                    match self {
                        App::Ready(game) => {
                            game.host.ios_virtual_key(event_loop, key, down);
                        }
                        App::Loading(loading) => {
                            if down {
                                loading.menu_key_focus(event_loop, key);
                            }
                        }
                    }
                }
                Input::Mouse(button, down) => {
                    self.window_event_here(
                        event_loop, id,
                        WindowEvent::MouseInput {
                            device_id: DeviceId::dummy(),
                            state: if down { ElementState::Pressed } else { ElementState::Released },
                            button: if button == 1 { MouseButton::Right } else { MouseButton::Left },
                        },
                    );
                }
                Input::Cursor(x, y) => self.window_event_here(
                    event_loop, id,
                    WindowEvent::CursorMoved {
                        device_id: DeviceId::dummy(),
                        position: winit::dpi::PhysicalPosition::new(x, y),
                    },
                ),
                Input::Motion(x, y) => self.device_event_here(
                    event_loop, DeviceId::dummy(),
                    DeviceEvent::MouseMotion { delta: (x, y) },
                ),
                Input::Text(text) => self.window_event_here(
                    event_loop, id,
                    WindowEvent::Ime(winit::event::Ime::Commit(text)),
                ),
                Input::KeyboardClosed => {
                    match self {
                        App::Loading(loading) => {
                            if let Some(window) = &loading.window {
                                window.set_ime_allowed(false);
                            }
                        }
                        App::Ready(game) => {
                            if let Some(window) = &game.host.window {
                                window.set_ime_allowed(false);
                            }
                        }
                    }
                }
                Input::Close => self.window_event_here(
                    event_loop, id, WindowEvent::CloseRequested,
                ),
            }
        }
    }

"""
from itertools import chain
keys = list("ABCDEFGHIJKLMNOPQRSTUVWXYZ")
digits = list("0123456789")
rows = []
for letter in keys:
    rows.append(f"            {ord(letter)} => KeyCode::Key{letter},")
for digit in digits:
    rows.append(f"            {ord(digit)} => KeyCode::Digit{digit},")
special = {
    1001: "Space", 1002: "ShiftLeft", 1003: "ControlLeft",
    1004: "Escape", 1005: "Enter", 1006: "Tab",
    1007: "Backspace", 1008: "ArrowUp", 1009: "ArrowDown",
    1010: "ArrowLeft", 1011: "ArrowRight", 1012: "F1",
    1013: "F2", 1014: "F3", 1015: "F5", 1016: "F11",
    1017: "Slash",
}
for number, key in special.items():
    rows.append(f"            {number} => KeyCode::{key},")
keymap = keymap.replace("KEY_MAP", "\n".join(rows))
# Keep the original implementation, only widen its visibility for the iOS
# application handler (which lives in a sibling module).
change("crates/horizon-client/src/game/interface/startup_menu.rs",
       "    fn menu_key_focus(&mut self, event_loop: &dyn EventLoopCtl, key: KeyCode) -> bool {",
       "    pub(super) fn menu_key_focus(&mut self, event_loop: &dyn EventLoopCtl, key: KeyCode) -> bool {")

change("crates/horizon-client/src/game/runtime/loading_screen.rs",
       "impl App {\n", "impl App {\n" + keymap)

change("crates/horizon-client/src/game/runtime/loading_screen.rs",
       """    fn about_to_wait_here(&mut self, event_loop: &ActiveEventLoop) {
""",
       """    fn about_to_wait_here(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_os = "ios")]
        self.ios_drain_inputs(event_loop);
""")
change("crates/horizon-client/src/game/runtime/loading_screen.rs",
       """        if let WindowEvent::Touch(touch) = &event {
            self.ios_touch_mouse(event_loop, id, touch);
            return;
        }
""",
       """        if let WindowEvent::Touch(touch) = &event {
            if horizon_client_input::ios_input::touch_gestures() {
                self.ios_touch_mouse(event_loop, id, touch);
            }
            return;
        }
""")

# The mapped-key handler deliberately handles gameplay bindings, not raw
# keyboard callbacks for chat, title screens or debug overlays. Give iOS
# physical keys the same high-level screen handling without attempting to
# construct winit::KeyEvent (which has private platform-specific fields).
ios_keys_path = Path("crates/horizon-client/src/game/ios_keyboard.rs")
assert not ios_keys_path.exists(), "Upstream now has an iOS keyboard adapter"
ios_keys_path.write_text(r'''//! Nova's iOS-only physical-key adapter.
//! UIKit controls and the software keyboard share this path. The native winit
//! KeyEvent type is intentionally never fabricated: its platform fields are private.
use super::*;
use horizon_client::debug_keys::KeyAction;

impl Game<'_> {
    pub(super) fn ios_virtual_key(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        key: KeyCode,
        pressed: bool,
    ) {
        if let Err(error) = self.ios_virtual_key_inner(event_loop, key, pressed) {
            log::warn!("Nova iOS input: {key:?} ({pressed}): {error}");
        }
    }

    fn ios_virtual_key_inner(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        key: KeyCode,
        pressed: bool,
    ) -> Result<()> {
        if pressed {
            self.physical_keys.insert(key);
            self.input_keyboard_used();
            self.last_input = Instant::now();
        } else {
            self.physical_keys.remove(&key);
        }

        // winit normally emits both KeyboardInput and ModifiersChanged.
        // Nova's touch keys have no OS modifier callback, so update Horizon's
        // modifier word as well as its physical-key set. Sneak and sprint
        // check this word every frame and otherwise cancel a held Shift/Ctrl.
        // Reuse the upstream helper to preserve other held modifier keys.
        if matches!(
            key,
            KeyCode::ShiftLeft | KeyCode::ShiftRight
                | KeyCode::ControlLeft | KeyCode::ControlRight
                | KeyCode::AltLeft | KeyCode::AltRight
                | KeyCode::SuperLeft | KeyCode::SuperRight
        ) {
            horizon_client_input::modifiers::set_key(
                &mut self.modifiers, &mut self.physical_keys, key, pressed,
            );
            self.update_modifiers(self.modifiers);
        }

        // Desktop keyboard events reach debug handling before screens and
        // bindings; F3 toggles on release, so both edges must be forwarded.
        let action = if pressed { KeyAction::Press } else { KeyAction::Release };
        if self.debug_key(key, action) {
            return Ok(());
        }
        if !pressed {
            if self.screen == Screen::Chat && self.title.is_none() {
                self.keys.remove(&key);
            }
            return self.mapped_key_event(key, false, false);
        }

        // Vanilla uses raw keys for the active screen before translating a
        // key into movement or another mapped gameplay action.
        if self.key_focus_key(event_loop, key) {
            return Ok(());
        }
        if self.dialog_open() {
            return self.dialog_key(key, None, None);
        }
        if self.screen == Screen::Chat && self.title.is_none() {
            self.keys.insert(key);
            self.chat_key_code(key, None);
            return Ok(());
        }
        if self.screen == Screen::Options {
            return self.options_key(key, None);
        }
        if self.title_input_active() {
            let (w, h) = self.gui_size();
            match key {
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    let (action, sound) = self
                        .title
                        .as_mut()
                        .map_or((Action::None, false), |t| t.confirm_key_size((w, h)));
                    self.save_title_options();
                    if sound {
                        self.audio.play_button_click();
                    }
                    self.title_action(event_loop, action)?;
                }
                KeyCode::Backspace | KeyCode::Delete
                | KeyCode::ArrowLeft | KeyCode::ArrowRight
                | KeyCode::Home | KeyCode::End
                    if self.title.as_mut().is_some_and(|t| t.has_caret_field()) =>
                {
                    let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
                    if let Some(t) = self.title.as_mut() {
                        t.field_key(key, ctrl, shift);
                    }
                }
                KeyCode::ArrowLeft | KeyCode::ArrowRight
                | KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    let shift = self.has_shift_down();
                    if let Some(t) = self.title.as_mut() {
                        t.arrow_key(key, (w, h), shift);
                    }
                }
                KeyCode::Backspace => self.title_edit(None, true),
                KeyCode::Tab => {
                    let shift = self.has_shift_down();
                    if let Some(t) = self.title.as_mut() {
                        t.tab_dir(shift, (w, h));
                    }
                }
                KeyCode::Escape => {
                    if let Some(t) = self.title.as_mut() {
                        t.escape();
                        self.reopen_data_packs(event_loop)?;
                    }
                    self.save_title_options();
                }
                KeyCode::F5 => {
                    if let Some(t) = self.title.as_mut() {
                        t.refresh_servers();
                    }
                }
                // The IME commit path delivers printable text separately.
                _ => {}
            }
            return Ok(());
        }

        // Text fields hosted inside screens also consume their own raw keys.
        if self.station_typing() && key != KeyCode::Escape {
            if !self.station_key(key) {
                self.station_text(None, key == KeyCode::Backspace);
            }
            return Ok(());
        }
        if self.screen == Screen::Creative
            && self.creative_screen.tab().search
            && !self.mod_browser_typing()
            && key != KeyCode::Escape
        {
            if !self.search_tab_hotbar_key(key) {
                let before = self.creative_screen.search.value.clone();
                let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
                if self.creative_screen.search.key(key, ctrl, shift)
                    && self.creative_screen.search.value != before
                {
                    self.creative_screen.scroll = 0.0;
                }
            }
            return Ok(());
        }
        if self.screen == Screen::Paused && self.pause_menu && key != KeyCode::Escape {
            self.pause_key(key)?;
            return Ok(());
        }

        // Movement, jump, inventory and other gameplay mappings remain the
        // original Horizon implementation. Don't alter camera or touch input.
        self.mapped_key_event(key, true, false)
    }
}
''')
game_root = "crates/horizon-client/src/game.rs"
root_text = Path(game_root).read_text()
ios_module = '#[cfg(target_os = "ios")]\nmod ios_keyboard;\n'
if "mod key_codes;\n" in root_text:
    change(game_root, "mod key_codes;\n", "mod key_codes;\n" + ios_module)
else:
    # Newer Horizon generates the module declarations at build time.
    generated = 'include!(concat!(env!("OUT_DIR"), "/game_modules.rs"));\n'
    change(game_root, generated, generated + ios_module)

print("Nova: injected iOS-only input queue and key/mouse dispatch")
