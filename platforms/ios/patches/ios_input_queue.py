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
queue_path.write_text("//! Nova iOS input bridge. The gameplay runtime calls these safe functions;\n//! only the iOS host's C ABI needs raw pointers. Nothing runs on desktop.\nuse std::sync::{\n    Mutex, OnceLock,\n    atomic::{AtomicBool, Ordering},\n};\n\n#[derive(Debug)]\npub enum Input {\n    Key(u32, bool),\n    Mouse(u32, bool),\n    Cursor(f64, f64),\n    Motion(f64, f64),\n    Text(String),\n    Close,\n}\n\nstatic INPUTS: OnceLock<Mutex<Vec<Input>>> = OnceLock::new();\nstatic TOUCH_GESTURES: AtomicBool = AtomicBool::new(true);\n\nfn input_queue() -> &'static Mutex<Vec<Input>> {\n    INPUTS.get_or_init(|| Mutex::new(Vec::new()))\n}\n\npub fn push(event: Input) {\n    if let Ok(mut inputs) = input_queue().lock() {\n        // Ignore excessive events instead of exhausting the main thread.\n        if inputs.len() < 2048 {\n            inputs.push(event);\n        }\n    }\n}\n\npub fn take() -> Vec<Input> {\n    input_queue()\n        .lock()\n        .map(|mut inputs| std::mem::take(&mut *inputs))\n        .unwrap_or_default()\n}\n\npub fn set_touch_gestures(value: bool) {\n    TOUCH_GESTURES.store(value, Ordering::Relaxed);\n}\n\npub fn touch_gestures() -> bool {\n    TOUCH_GESTURES.load(Ordering::Relaxed)\n}\n")

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
                    match self {
                        App::Loading(loading) => {
                            if down {
                                loading.menu_key_focus(event_loop, key);
                            }
                        }
                        App::Ready(game) => {
                            if down {
                                game.host.physical_keys.insert(key);
                            } else {
                                game.host.physical_keys.remove(&key);
                            }
                            if let Err(error) = game.host.mapped_key_event(key, down, false) {
                                log::warn!("Nova iOS virtual key: {error}");
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
change("crates/horizon-client/src/game/loading_screen.rs",
       "impl App {\n", "impl App {\n" + keymap)

change("crates/horizon-client/src/game/loading_screen.rs",
       """    fn about_to_wait_here(&mut self, event_loop: &ActiveEventLoop) {
""",
       """    fn about_to_wait_here(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_os = "ios")]
        self.ios_drain_inputs(event_loop);
""")
change("crates/horizon-client/src/game/loading_screen.rs",
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
print("Nova: injected iOS-only input queue and key/mouse dispatch")
