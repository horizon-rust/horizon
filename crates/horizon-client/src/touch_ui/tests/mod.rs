// Adapted from Starling (https://github.com/starling-client/starling, `android` branch at
// 150a414b09a1, `crates/horizon-client/src/touch_ui.rs`), written by Vasysik and the Starling
// contributors. Copyright (c) 2026 Ludovic Marie and the Horizon contributors, under the same MIT
// license as Horizon (LICENSE at the repository root).

//! Tests of the menu touch layer.
use super::*;

const W: f32 = 427.0;
const H: f32 = 267.0;
const SIZE: [f32; 2] = [400.0, 240.0];

fn relative(ui: &mut TouchUi, id: u64, p: [f32; 2], phase: TouchPhase, clicks: bool) -> [f32; 2] {
    ui.relative_touch(id, p, phase, [100.0, 80.0], SIZE, 1.0, clicks)
}

/// Make the virtual mouse's finger look held for `ms`.
fn age(ui: &mut TouchUi, ms: u64) {
    ui.relative.as_mut().unwrap().since = Instant::now() - Duration::from_millis(ms);
}

#[test]
fn scrolling_never_clicks() {
    let mut u = TouchUi::default();
    assert_eq!(
        u.gesture(1, [20.0, 200.0], TouchPhase::Started),
        Gesture::None
    );
    assert_eq!(
        u.gesture(1, [20.0, 140.0], TouchPhase::Moved),
        Gesture::Scroll(-2.0)
    );
    assert_eq!(
        u.gesture(1, [20.0, 170.0], TouchPhase::Moved),
        Gesture::Scroll(1.0)
    );
    assert_eq!(
        u.gesture(1, [20.0, 170.0], TouchPhase::Ended),
        Gesture::None
    );
}

#[test]
fn a_small_wobble_still_taps_and_does_not_scroll() {
    let mut u = TouchUi::default();
    u.gesture(1, [20.0, 20.0], TouchPhase::Started);
    assert_eq!(u.gesture(1, [25.0, 28.0], TouchPhase::Moved), Gesture::None);
    assert_eq!(u.gesture(1, [26.0, 29.0], TouchPhase::Ended), Gesture::Tap);
}

#[test]
fn tap_release_cancel_and_second_finger() {
    let mut u = TouchUi::default();
    u.gesture(1, [20.0, 20.0], TouchPhase::Started);
    // A second finger neither taps nor takes over the gesture.
    u.gesture(2, [80.0, 20.0], TouchPhase::Started);
    assert_eq!(u.gesture(2, [20.0, 20.0], TouchPhase::Ended), Gesture::None);
    assert_eq!(u.gesture(1, [21.0, 21.0], TouchPhase::Ended), Gesture::Tap);
    // A cancelled finger never taps.
    u.gesture(1, [20.0, 20.0], TouchPhase::Started);
    u.gesture(1, [20.0, 20.0], TouchPhase::Cancelled);
    assert_eq!(u.gesture(1, [20.0, 20.0], TouchPhase::Ended), Gesture::None);
}

#[test]
fn a_finger_on_a_slider_drags_it_until_it_lifts() {
    widgets::target_as(
        [10.0, 10.0, 100.0, 20.0],
        Kind::Slider {
            value: 0.5,
            track: [10.0, 100.0],
        },
    );
    widgets::target([10.0, 40.0, 100.0, 20.0]);
    widgets::end_frame(widgets::generation() + 1);
    let mut u = TouchUi::default();
    // A button is no slider: the scroll-or-tap gesture takes it.
    assert!(!u.direct_pointer(1, [50.0, 50.0], TouchPhase::Started));
    assert!(u.direct_pointer(2, [50.0, 20.0], TouchPhase::Started));
    // Off the slider now, and the other finger: still only finger 2.
    assert!(u.direct_pointer(2, [300.0, 200.0], TouchPhase::Moved));
    assert!(!u.direct_pointer(1, [50.0, 20.0], TouchPhase::Moved));
    // A second finger landing on the slider takes nothing from finger 2.
    assert!(!u.direct_pointer(3, [50.0, 20.0], TouchPhase::Started));
    assert!(u.direct_pointer(2, [310.0, 200.0], TouchPhase::Moved));
    assert!(!u.direct_pointer(3, [60.0, 20.0], TouchPhase::Ended));
    assert!(u.direct_pointer(2, [300.0, 200.0], TouchPhase::Ended));
    assert!(!u.direct_pointer(2, [300.0, 200.0], TouchPhase::Moved));
    // Once it lifts, the next finger on the slider has it.
    assert!(u.direct_pointer(4, [50.0, 20.0], TouchPhase::Started));
}

#[test]
fn virtual_mouse_moves_relatively_and_dragging_never_clicks() {
    let mut ui = TouchUi::default();
    let cursor = [100.0, 80.0];
    let start = ui.relative_touch(
        1,
        [300.0, 200.0],
        TouchPhase::Started,
        cursor,
        SIZE,
        2.0,
        true,
    );
    // Touching down leaves the pointer where it was.
    assert_eq!(start, cursor);
    let p = ui.relative_touch(
        1,
        [310.0, 195.0],
        TouchPhase::Moved,
        cursor,
        SIZE,
        2.0,
        true,
    );
    assert_eq!(p, [120.0, 70.0]);
    ui.relative_touch(1, [310.0, 195.0], TouchPhase::Ended, p, SIZE, 2.0, true);
    assert_eq!(ui.relative_click(true), None);
}

#[test]
fn the_pointer_stays_on_the_screen() {
    let mut ui = TouchUi::default();
    ui.relative_touch(
        1,
        [0.0, 0.0],
        TouchPhase::Started,
        [5.0, 5.0],
        SIZE,
        1.0,
        true,
    );
    let p = ui.relative_touch(
        1,
        [-50.0, 900.0],
        TouchPhase::Moved,
        [5.0, 5.0],
        SIZE,
        1.0,
        true,
    );
    assert_eq!(p, [0.0, 239.0]);
}

#[test]
fn only_the_first_finger_moves_the_pointer() {
    let mut ui = TouchUi::default();
    relative(&mut ui, 1, [10.0, 10.0], TouchPhase::Started, true);
    relative(&mut ui, 2, [50.0, 50.0], TouchPhase::Started, true);
    assert_eq!(
        relative(&mut ui, 2, [90.0, 90.0], TouchPhase::Moved, true),
        [100.0, 80.0]
    );
    assert_eq!(
        relative(&mut ui, 1, [20.0, 10.0], TouchPhase::Moved, true),
        [110.0, 80.0]
    );
}

#[test]
fn virtual_mouse_tap_hold_cancel_and_click_preference() {
    let mut ui = TouchUi::default();
    let p = [100.0, 80.0];
    // A tap left-clicks once.
    relative(&mut ui, 1, p, TouchPhase::Started, true);
    relative(&mut ui, 1, p, TouchPhase::Ended, true);
    assert_eq!(ui.relative_click(true), Some(false));
    assert_eq!(ui.relative_click(true), None);
    // A hold right-clicks once, before the finger lifts, and not again on release.
    relative(&mut ui, 2, p, TouchPhase::Started, true);
    assert_eq!(ui.relative_click(true), None);
    age(&mut ui, 1000);
    assert_eq!(ui.relative_click(true), Some(true));
    assert_eq!(ui.relative_click(true), None);
    relative(&mut ui, 2, p, TouchPhase::Ended, true);
    assert_eq!(ui.relative_click(true), None);
    // Without clicks nothing is owed.
    relative(&mut ui, 3, p, TouchPhase::Started, false);
    relative(&mut ui, 3, p, TouchPhase::Ended, false);
    assert_eq!(ui.relative_click(false), None);
    // A cancelled finger owes nothing.
    relative(&mut ui, 4, p, TouchPhase::Started, true);
    ui.cancel_relative();
    relative(&mut ui, 4, p, TouchPhase::Ended, true);
    assert_eq!(ui.relative_click(true), None);
    relative(&mut ui, 5, p, TouchPhase::Started, true);
    relative(&mut ui, 5, p, TouchPhase::Cancelled, true);
    assert_eq!(ui.relative_click(true), None);
}

#[test]
fn explicit_mouse_drag_does_not_generate_an_extra_tap_or_hold_click() {
    let mut ui = TouchUi::default();
    relative(&mut ui, 1, [10.0, 10.0], TouchPhase::Started, true);
    ui.press_mouse(2, false);
    age(&mut ui, 700);
    assert_eq!(ui.relative_click(false), None);
    assert_eq!(ui.release_mouse(2), Some((false, true)));
    assert_eq!(ui.relative_click(true), None);
    relative(&mut ui, 1, [10.0, 10.0], TouchPhase::Ended, true);
    assert_eq!(ui.relative_click(true), None);
}

#[test]
fn mouse_toggle_and_back_show_only_on_screens() {
    let mut ui = TouchUi::default();
    assert_eq!(
        ui.action(W, H, [350.0, 10.0], true),
        Some(Action::VirtualMouse)
    );
    assert_eq!(
        ui.action(W, H, [400.0, 10.0], true),
        Some(Action::Key(KeyCode::Escape))
    );
    assert_eq!(ui.action(W, H, [350.0, 10.0], false), None);
    assert_eq!(ui.action(W, H, [400.0, 10.0], false), None);
    assert_eq!(ui.action(W, H, [200.0, 10.0], true), None);
    // LMB and RMB only with the virtual mouse on.
    assert_eq!(ui.action(W, H, [399.0, 226.0], true), None);
    ui.virtual_mouse = true;
    assert_eq!(
        ui.action(W, H, [399.0, 226.0], true),
        Some(Action::MouseButton(false))
    );
    assert_eq!(
        ui.action(W, H, [399.0, 250.0], true),
        Some(Action::MouseButton(true))
    );
    assert_eq!(ui.action(W, H, [213.0, 250.0], true), None);
    assert_eq!(ui.action(W, H, [399.0, 226.0], false), None);
}

#[test]
fn menu_mouse_buttons_share_ownership_and_release_outside_their_bounds() {
    let mut ui = TouchUi::default();
    assert!(ui.press_mouse(1, false));
    assert!(!ui.press_mouse(2, false));
    assert!(ui.press_mouse(3, true));
    assert!(ui.mouse_held() && ui.owns_mouse(2) && !ui.owns_mouse(4));
    assert_eq!(ui.release_mouse(1), Some((false, false)));
    assert_eq!(ui.release_mouse(2), Some((false, true)));
    assert_eq!(ui.release_mouse(2), None);
    assert_eq!(ui.release_all_mouse(), vec![true]);
    assert_eq!(ui.release_mouse(3), None);
    assert!(!ui.mouse_held());
}

#[test]
fn the_touch_layer_draws_only_on_screens() {
    let atlas = HudAtlas::build(std::path::Path::new("/nonexistent/horizon-touch-ui")).unwrap();
    let count = |ui: &TouchUi, menu: bool| {
        let mut b = HudBatch::new([1280, 720]);
        ui.draw(&mut b, &atlas, menu);
        b.vertices.len()
    };
    let mut ui = TouchUi::default();
    assert_eq!(count(&ui, false), 0);
    let row = count(&ui, true);
    assert!(row > 0);
    ui.virtual_mouse = true;
    assert!(count(&ui, true) > row);
    let mut b = HudBatch::new([1280, 720]);
    draw_pointer(&mut b, &atlas, (10.0, 10.0));
    assert!(!b.vertices.is_empty());
}

/// The press/release table of a physical left click: `(pressed, owned, touched)`.
#[test]
fn aux_mouse_presses_belong_to_the_layer_only_over_its_buttons() {
    // In a world, attack and use are not the layer's.
    assert_eq!(aux_mouse_phase(true, false, false), None);
    assert_eq!(
        aux_mouse_phase(true, false, true),
        Some(TouchPhase::Started)
    );
    // A release without a press the layer holds stays with the game.
    assert_eq!(aux_mouse_phase(false, false, true), None);
}

#[test]
fn a_held_mouse_button_releases_after_a_screen_change() {
    // The pointer held RMB, then the screen changed: its release is still
    // the layer's (nothing is under it now), then frees it.
    let mut ui = TouchUi::default();
    assert_eq!(aux_mouse_phase(false, ui.owns_mouse(7), false), None);
    ui.press_mouse(7, true);
    assert_eq!(
        aux_mouse_phase(false, ui.owns_mouse(7), false),
        Some(TouchPhase::Ended)
    );
    assert_eq!(ui.release_mouse(7), Some((true, true)));
    assert_eq!(aux_mouse_phase(false, ui.owns_mouse(7), false), None);
}
