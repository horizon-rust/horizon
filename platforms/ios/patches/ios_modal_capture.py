#!/usr/bin/env python3
"""iOS-only modal input protection and software mouse capture.

Runs on the CI checkout after ios_input_queue.py; does not modify tracked
upstream Horizon files. Desktop cursor locking is not implemented on UIKit.
"""
from pathlib import Path

def change(path, old, new):
    p = Path(path)
    s = p.read_text(encoding="utf-8")
    assert s.count(old) == 1, (path, old[:80], s.count(old))
    p.write_text(s.replace(old, new, 1), encoding="utf-8")

queue = "crates/horizon-client-input/src/ios_input.rs"
change(queue,
       "static TOUCH_GESTURES: AtomicBool = AtomicBool::new(true);",
       """static TOUCH_GESTURES: AtomicBool = AtomicBool::new(true);
static OVERLAY_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn set_overlay_active(value: bool) {
    OVERLAY_ACTIVE.store(value, Ordering::Release);
}

pub fn overlay_active() -> bool {
    OVERLAY_ACTIVE.load(Ordering::Acquire)
}""")

loading = "crates/horizon-client/src/game/loading_screen.rs"
change(loading,
       """    fn ios_drain_inputs(&mut self, event_loop: &ActiveEventLoop) {
        use horizon_client_input::ios_input::Input;""",
       """    fn ios_drain_inputs(&mut self, event_loop: &ActiveEventLoop) {
        use horizon_client_input::ios_input::{self, Input};
        use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
        static WAS_MODAL: AtomicBool = AtomicBool::new(false);
        let modal = ios_input::overlay_active();
        let was_modal = WAS_MODAL.swap(modal, AtomicOrdering::Relaxed);
        if modal && !was_modal {
            if let App::Ready(game) = self {
                let held: Vec<KeyCode> = game.host.physical_keys.iter().copied().collect();
                for key in held {
                    let _ = game.host.mapped_key_event(key, false, false);
                    game.host.physical_keys.remove(&key);
                }
            }
            let id = match self {
                App::Loading(loading) => loading.window.as_ref().map(|w| w.id()),
                App::Ready(game) => game.host.window.as_ref().map(|w| w.id()),
            };
            if let Some(id) = id {
                for button in [MouseButton::Left, MouseButton::Right] {
                    self.window_event_here(event_loop, id, WindowEvent::MouseInput {
                        device_id: DeviceId::dummy(),
                        state: ElementState::Released,
                        button,
                    });
                }
            }
        }
        if modal {
            // Discard all queued controls, chat and look movement during Nova UI.
            let _ = ios_input::take();
            return;
        }""")

change(loading,
       """if horizon_client_input::ios_input::touch_gestures() {
                self.ios_touch_mouse(event_loop, id, touch);
            }""",
       """if horizon_client_input::ios_input::touch_gestures()
                && !horizon_client_input::ios_input::overlay_active()
            {
                self.ios_touch_mouse(event_loop, id, touch);
            }""")

capture = "crates/horizon-client/src/game/pointer_capture.rs"
change(capture,
       "if self.internal_grab() {",
       'if self.internal_grab() || cfg!(target_os = "ios") {')
change(capture,
       'if capture\n            && ((!self.focused && !self.unclaimed_window && !self.scripted_run())',
       'if capture\n            && ((!self.focused && !self.unclaimed_window && !self.scripted_run() && !cfg!(target_os = "ios"))')

handler = "crates/horizon-client/src/game/window_handler.rs"
change(handler,
       '            && self.captured\n            && self.focused\n',
       '            && self.captured\n            && (self.focused || cfg!(target_os = "ios"))\n')
print("Nova: UIKit modal input gate and native software mouse capture applied")
