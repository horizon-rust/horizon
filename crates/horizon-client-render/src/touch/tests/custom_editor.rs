//! Custom buttons in the layout editor.
use super::*;
use crate::touch::custom::UNASSIGNED;
use crate::touch::editor::{choices, palette_rect};

const PALETTE: [f32; 2] = [0.15, 0.165];
const CREATE: [f32; 2] = [0.5, 0.875];

fn tap(t: &mut TouchControls, id: u64, p: [f32; 2]) -> Option<EditorExit> {
    t.edit_touch(id, p, true, true)
}

fn picking(t: &TouchControls) -> bool {
    t.editor.as_ref().unwrap().key_picker
}

/// The centre of key picker row `i`.
fn row(i: usize) -> [f32; 2] {
    [0.5, 0.28 + i as f32 * 0.072]
}

/// An open editor with one new button, its key picker open.
fn created() -> TouchControls {
    let mut t = TouchControls::default();
    t.viewport(1920, 1080);
    t.open_editor();
    tap(&mut t, 1, PALETTE);
    tap(&mut t, 2, CREATE);
    t
}

#[test]
fn create_opens_the_picker_and_cancel_takes_the_button_away() {
    let mut t = created();
    assert!(t.editor.is_some());
    assert_eq!(t.settings.custom_buttons.len(), 1);
    assert_eq!(t.settings.custom_buttons[0].key, UNASSIGNED);
    assert!(picking(&t));
    assert_eq!(t.editor.as_ref().unwrap().selected_custom, Some(0));
    t.cancel_editor();
    assert!(t.settings.custom_buttons.is_empty());
}

#[test]
fn the_picker_assigns_actions_or_keys_and_pages_through_them() {
    let mut t = created();
    tap(&mut t, 3, row(1));
    assert!(!picking(&t));
    assert_eq!(t.settings.custom_buttons[0].key, choices(false)[1].0);
    // Pick again, from the keyboard keys, on the second page.
    let action = [0.5, 0.575];
    tap(&mut t, 4, action);
    assert!(picking(&t));
    tap(&mut t, 5, [0.6, 0.19]); // Keyboard key
    tap(&mut t, 6, [0.7, 0.9]); // Next
    tap(&mut t, 7, [0.7, 0.9]);
    tap(&mut t, 8, [0.3, 0.9]); // Previous
    assert_eq!(t.editor.as_ref().unwrap().key_page, 1);
    tap(&mut t, 9, row(0));
    assert_eq!(t.settings.custom_buttons[0].key, choices(true)[8].0);
    // Back closes the picker and keeps the key.
    tap(&mut t, 10, action);
    tap(&mut t, 11, [0.5, 0.9]);
    assert!(!picking(&t));
    assert_eq!(t.settings.custom_buttons[0].key, choices(true)[8].0);
}

#[test]
fn a_keyboard_key_can_be_captured_while_picking() {
    let mut t = TouchControls::default();
    assert!(!t.capture_custom_key("key.keyboard.z"), "no editor");
    let mut t2 = created();
    assert!(!t2.capture_custom_key("key.keyboard.no-such-key"));
    assert!(t2.capture_custom_key("key.keyboard.z"));
    assert_eq!(t2.settings.custom_buttons[0].key, "key.keyboard.z");
    assert!(!picking(&t2));
    assert!(!t2.capture_custom_key("key.keyboard.x"), "picker closed");
    t.open_editor();
    assert!(!t.capture_custom_key("key.keyboard.z"), "nothing selected");
}

#[test]
fn custom_buttons_drag_resize_and_remove() {
    let mut t = created();
    t.capture_custom_key("key.keyboard.z");
    tap(&mut t, 3, [0.9, 0.3]); // elsewhere: deselect
    assert_eq!(t.editor.as_ref().unwrap().selected_custom, None);
    let start = t.custom_placement(0).center;
    t.edit_touch(4, start, true, false);
    assert_eq!(t.editor.as_ref().unwrap().selected_custom, Some(0));
    t.edit_touch(4, [0.5, 0.3], false, true);
    let moved = t.settings.custom_buttons[0].layouts[0].center;
    assert!((moved[0] - 0.5).abs() < 1e-4 && (moved[1] - 0.3).abs() < 1e-4);
    // Other schemes keep their own place.
    assert_eq!(t.settings.custom_buttons[0].layouts[1].center, start);
    tap(&mut t, 5, [0.6, 0.82]); // Size
    tap(&mut t, 6, [0.675, 0.7]); // the slider's right end
    assert!((t.settings.custom_buttons[0].layouts[0].size - 0.45).abs() < 1e-5);
    tap(&mut t, 7, [0.5, 0.9]); // Remove button
    assert!(t.settings.custom_buttons.is_empty());
    assert_eq!(tap(&mut t, 8, [0.05, 0.07]), Some(EditorExit::Done));
}

#[test]
fn a_button_beside_the_toolbar_can_be_picked_again() {
    let mut t = created();
    t.capture_custom_key("key.keyboard.z");
    t.settings.custom_buttons[0].layouts[0].center = [0.8, 0.06];
    tap(&mut t, 3, [0.9, 0.3]); // elsewhere: deselect
    assert_eq!(t.editor.as_ref().unwrap().selected_custom, None);
    tap(&mut t, 4, [0.8, 0.06]);
    assert_eq!(t.editor.as_ref().unwrap().selected_custom, Some(0));
}

#[test]
fn the_palette_still_reaches_its_last_entries_and_holds_at_most_32() {
    let mut t = TouchControls::default();
    t.settings.offhand_button = false;
    t.open_editor();
    tap(&mut t, 1, PALETTE);
    let [x, y, w, h] = palette_rect(Control::Offhand as usize);
    tap(&mut t, 2, [x + w / 2.0, y + h / 2.0]);
    assert!(t.visible(Control::Offhand));
    assert!(t.settings.custom_buttons.is_empty());
    t.settings.custom_buttons = (0..32)
        .map(|i| CustomButton::new("key.keyboard.z".into(), i))
        .collect();
    tap(&mut t, 3, CREATE);
    assert_eq!(t.settings.custom_buttons.len(), 32);
}

#[test]
fn every_choice_is_a_valid_saved_token() {
    let actions = choices(false);
    let keys = choices(true);
    assert!(
        actions
            .iter()
            .any(|(token, _)| token == "action:key.attack")
    );
    assert!(keys.iter().any(|(token, _)| token == "key.keyboard.z"));
    assert!(keys.len() > 16);
    for (token, label) in actions.iter().chain(&keys) {
        assert!(CustomButton::valid_token(token), "{token}");
        assert!(!label.is_empty(), "{token}");
    }
}

#[test]
fn the_selected_button_and_its_panels_are_drawn() {
    let a = crate::hud::HudAtlas::build(std::path::Path::new("/nonexistent/horizon-touch-keys"))
        .unwrap();
    let count = |t: &TouchControls| {
        let mut b = crate::hud::HudBatch::new([1920, 1080]);
        t.draw(&mut b, &a);
        b.vertices.len()
    };
    let mut t = created();
    let picker = count(&t);
    t.capture_custom_key("key.keyboard.z");
    let panel = count(&t);
    tap(&mut t, 3, [0.9, 0.3]);
    let idle = count(&t);
    assert!(picker > panel && panel > idle);
}
