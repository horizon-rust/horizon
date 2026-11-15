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
static IN_WORLD: AtomicBool = AtomicBool::new(false);
static POINTER_STATE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Low bit indicates first-person gameplay, high bits identify a transition
/// out of first-person. Navigation between menus must preserve cursor position.
pub fn publish_pointer_mode(in_world: bool, changed_screen: bool) {
    let previous = POINTER_STATE.load(Ordering::Relaxed);
    let epoch = (previous >> 1).wrapping_add(u64::from(changed_screen));
    POINTER_STATE.store((epoch << 1) | u64::from(in_world), Ordering::Release);
}

pub fn pointer_state() -> u64 {
    POINTER_STATE.load(Ordering::Acquire)
}

pub fn set_playing(value: bool) {
    IN_WORLD.store(value, Ordering::Relaxed);
}

pub fn playing() -> bool {
    IN_WORLD.load(Ordering::Relaxed)
}

pub fn set_overlay_active(value: bool) {
    OVERLAY_ACTIVE.store(value, Ordering::Release);
}

pub fn overlay_active() -> bool {
    OVERLAY_ACTIVE.load(Ordering::Acquire)
}""")

# Avoid sending hundreds of individual move events into the expensive
# game event handler. Preserve relative deltas and button/key ordering.
change(queue,
       "            inputs.push(event);",
       """            match event {
                Input::Motion(dx, dy) => {
                    if let Some(Input::Motion(x, y)) = inputs.last_mut() {
                        *x += dx;
                        *y += dy;
                    } else {
                        inputs.push(Input::Motion(dx, dy));
                    }
                }
                Input::Cursor(x, y) => {
                    if let Some(Input::Cursor(old_x, old_y)) = inputs.last_mut() {
                        *old_x = x;
                        *old_y = y;
                    } else {
                        inputs.push(Input::Cursor(x, y));
                    }
                }
                other => inputs.push(other),
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
        // Only recenter on the transition from grabbed gameplay back
        // into a GUI, e.g. opening inventory or pause. Options opened
        // from the main menu preserves the existing cursor position.
        let in_world = matches!(self, App::Ready(game)
            if game.host.screen == Screen::Playing
                && game.host.title.is_none()
                && !game.host.screen_over_playing());
        static WAS_IN_WORLD: AtomicBool = AtomicBool::new(false);
        let was_in_world = WAS_IN_WORLD.swap(in_world, AtomicOrdering::Relaxed);
        let changed = was_in_world && !in_world;
        ios_input::set_playing(in_world);
        ios_input::publish_pointer_mode(in_world, changed);
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

# winit's desktop device-event handler requires an OS pointer grab,
# foreground focus and a keyboard/mouse owner. None are reliable indicators
# of a finger drag on iOS. Feed touch movement to the existing mouse look
# accumulator, so Horizon applies its normal sensitivity and frame smoothing.
# Keep this scoped to live first-person gameplay, never menus or Nova UI.
change(loading,
       """                Input::Motion(x, y) => self.device_event_here(
                    event_loop, DeviceId::dummy(),
                    DeviceEvent::MouseMotion { delta: (x, y) },
                ),""",
       """                Input::Motion(x, y) => {
                    if let App::Ready(game) = self {
                        let host = &mut game.host;
                        if host.screen == Screen::Playing
                            && host.title.is_none()
                            && !host.screen_over_playing()
                            && !host.scripted_run()
                            && host.pending_save.is_none()
                            && !host.pad_radial_captures()
                        {
                            // UIKit has no desktop mouse grab. Once the
                            // world is playing, use the iOS software capture.
                            if !host.captured {
                                host.capture(true);
                            }
                            if host.captured {
                                host.mouse.add_motion(x, y);
                                host.last_input = Instant::now();
                            }
                        }
                    }
                },""")

# A touchscreen is not a desktop keyboard: focusing chat, a sign or
# another field must not summon the iOS keyboard automatically. Nova's
# explicit Type control remains the only keyboard activation path.
change("crates/horizon-client/src/game/ime.rs",
       "        let want = self.text_field_focused();",
       '        let want = self.text_field_focused() && !cfg!(target_os = "ios");')

capture = "crates/horizon-client/src/game/pointer_capture.rs"
change(capture,
       "if self.internal_grab() {",
       'if self.internal_grab() || cfg!(target_os = "ios") {')
change(capture,
       "        let active = self.focused || self.agent_pointer;",
       '        let active = self.focused || self.agent_pointer || cfg!(target_os = "ios");')
change(capture,
       'if capture\n            && ((!self.focused && !self.unclaimed_window && !self.scripted_run())',
       'if capture\n            && ((!self.focused && !self.unclaimed_window && !self.scripted_run() && !cfg!(target_os = "ios"))')

# iOS does not grab a desktop pointer: its first virtual click is an
# actual attack/use action rather than a focus-acquiring mouse click.
change("crates/horizon-client/src/game.rs",
       """                            if pressed && !self.captured {
                                self.capture(true);
                                return Ok(());
                            }""",
       """                            if pressed && !self.captured {
                                self.capture(true);
                                if !cfg!(target_os = "ios") {
                                    return Ok(());
                                }
                            }""")

handler = "crates/horizon-client/src/game/window_handler.rs"
change(handler,
       '            && self.captured\n            && self.focused\n',
       '            && self.captured\n            && (self.focused || cfg!(target_os = "ios"))\n')
print("Nova: UIKit modal input gate and native software mouse capture applied")
