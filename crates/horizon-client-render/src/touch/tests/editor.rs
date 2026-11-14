//! The layout editor (adapted from Starling's touch.rs editor tests).
use super::*;

/// One quick tap in the editor.
fn tap(t: &mut TouchControls, id: u64, p: [f32; 2]) -> Option<EditorExit> {
    t.edit_touch(id, p, true, true)
}

#[test]
fn dragging_moves_a_control_and_cancel_restores_it() {
    let mut t = TouchControls::default();
    t.open_editor();
    assert_eq!(t.edit_touch(1, [0.13, 0.76], true, false), None);
    assert_eq!(t.editor.as_ref().unwrap().selected, Some(Control::Move));
    t.edit_touch(1, [0.18, 0.70], false, true);
    assert!((t.settings.placement(Control::Move).center[0] - 0.18).abs() < 0.001);
    // Editing never reaches the game.
    assert_eq!(t.poll_state(), PadState::default());
    assert!(!t.owns_pointer(1));
    t.cancel_editor();
    assert!(t.editor.is_none());
    assert_eq!(t.settings.placement(Control::Move).center, [0.13, 0.76]);
}

#[test]
fn cancel_keeps_options_set_before_the_editor_opened() {
    let mut t = TouchControls::default();
    t.settings.mode = Mode::Tap;
    t.settings.left_handed = true;
    let original = t.settings.placement(Control::Move).center;
    t.open_editor();
    t.edit_touch(1, original, true, false);
    t.edit_touch(1, [0.7, 0.7], false, true);
    assert_ne!(t.settings.placement(Control::Move).center, original);
    assert_eq!(tap(&mut t, 2, [0.18, 0.07]), Some(EditorExit::Cancelled));
    assert!(t.editor.is_none());
    assert_eq!(t.settings.placement(Control::Move).center, original);
    assert_eq!(t.settings.mode, Mode::Tap);
    assert!(t.settings.left_handed);
}

#[test]
fn done_keeps_the_changes_for_the_game_to_save() {
    let mut t = TouchControls::default();
    t.open_editor();
    t.edit_touch(1, [0.13, 0.76], true, false);
    t.edit_touch(1, [0.2, 0.7], false, true);
    let moved = t.settings.clone();
    assert_eq!(tap(&mut t, 2, [0.05, 0.07]), Some(EditorExit::Done));
    assert!(t.editor.is_none());
    assert_eq!(t.settings, moved);
    assert_eq!(
        tap(&mut t, 3, [0.05, 0.07]),
        None,
        "closed editors ignore touches"
    );
}

#[test]
fn the_size_slider_resizes_the_selected_control() {
    let mut t = TouchControls::default();
    t.settings.mode = Mode::Tap;
    t.open_editor();
    tap(&mut t, 3, [0.13, 0.76]);
    tap(&mut t, 4, [0.6, 0.82]); // Size
    tap(&mut t, 5, [0.65, 0.73]); // the slider, near its right end
    assert!(t.settings.placement(Control::Move).size > 0.4);
    assert_eq!(t.settings.placement(Control::Jump).size, 0.16);
    t.cancel_editor();
    assert_eq!(t.settings.mode, Mode::Tap);
    assert_eq!(t.settings.placement(Control::Move).size, 0.32);
}

#[test]
fn opacity_can_apply_to_every_control_of_the_scheme_and_a_miss_deselects() {
    let mut t = TouchControls::default();
    t.open_editor();
    tap(&mut t, 1, [0.13, 0.76]);
    tap(&mut t, 2, [0.4, 0.82]); // Opacity
    tap(&mut t, 3, [0.5, 0.64]); // Apply to all
    tap(&mut t, 4, [0.5, 0.73]); // the slider, at its middle
    for p in t.settings.layouts[0] {
        assert!((p.opacity - 0.5).abs() < 0.001);
    }
    assert_eq!(t.settings.layouts[1][0].opacity, 0.65);
    tap(&mut t, 5, [0.7, 0.4]);
    assert!(t.editor.as_ref().unwrap().selected.is_none());
    t.cancel_editor();
    assert_eq!(t.settings.layouts[0][0].opacity, 0.65);
}

#[test]
fn remove_hides_a_control_and_the_palette_brings_it_back() {
    let mut t = TouchControls::default();
    t.open_editor();
    let jump = t.placement(Control::Jump).center;
    tap(&mut t, 1, jump);
    tap(&mut t, 2, [0.5, 0.9]); // Remove button
    assert!(!t.visible(Control::Jump));
    assert!(t.editor.as_ref().unwrap().selected.is_none());
    tap(&mut t, 3, [0.1, 0.16]); // Add buttons...
    let jump = Control::Jump as usize;
    let [x, y, w, h] = crate::touch::editor::palette_rect(jump);
    tap(&mut t, 4, [x + w / 2.0, y + h / 2.0]);
    assert!(t.visible(Control::Jump));
    assert_eq!(t.settings.button_visibility[jump], layout::SHOWN);
    // With the palette open, the controls underneath don't take touches.
    let before = t.settings.clone();
    t.edit_touch(5, [0.13, 0.76], true, false);
    t.edit_touch(5, [0.3, 0.3], false, true);
    assert_eq!(t.settings, before);
}

#[test]
fn every_palette_entry_fits_on_screen_and_none_overlap() {
    let rects: Vec<_> = (0..layout::CONTROLS)
        .map(crate::touch::editor::palette_rect)
        .collect();
    for (i, a) in rects.iter().enumerate() {
        assert!(
            a[1] + a[3] <= 0.83,
            "entry {i} stays above the panel's foot"
        );
        for b in &rects[i + 1..] {
            let apart = a[0] + a[2] <= b[0]
                || b[0] + b[2] <= a[0]
                || a[1] + a[3] <= b[1]
                || b[1] + b[3] <= a[1];
            assert!(apart);
        }
    }
}

#[test]
fn reset_restores_the_factory_layout_of_this_scheme_only() {
    let mut t = TouchControls::default();
    t.settings.mode = Mode::Tap;
    t.settings.layouts[0][Control::Jump as usize].center = [0.5, 0.5];
    t.settings.layouts[1][Control::Jump as usize].center = [0.5, 0.5];
    t.settings.button_visibility[Control::Chat as usize] = layout::HIDDEN;
    t.settings
        .custom_buttons
        .push(CustomButton::new("key.keyboard.z".into(), 0));
    t.settings.custom_buttons[0].layouts[1].center = [0.3, 0.3];
    t.open_editor();
    tap(&mut t, 1, [0.3, 0.07]); // Reset
    let factory = Settings::default().layouts[1][Control::Jump as usize];
    assert_eq!(t.settings.layouts[1][Control::Jump as usize], factory);
    assert_eq!(
        t.settings.layouts[0][Control::Jump as usize].center,
        [0.5, 0.5]
    );
    assert!(t.visible(Control::Chat));
    assert_ne!(t.settings.custom_buttons[0].layouts[1].center, [0.3, 0.3]);
    assert_eq!(t.settings.custom_buttons[0].key, "key.keyboard.z");
}

#[test]
fn big_controls_drag_inside_narrow_portrait_surfaces() {
    let mut t = TouchControls::default();
    t.viewport(400, 1000);
    t.settings.layouts[0][Control::Move as usize].size = 0.45;
    t.open_editor();
    let center = t.placement(Control::Move).center;
    t.edit_touch(77, center, true, false);
    assert_eq!(t.editor.as_ref().unwrap().selected, Some(Control::Move));
    t.edit_touch(77, [0.95, 0.95], false, false);
    let [x, y, w, h] = t.placement(Control::Move).rect(t.aspect());
    assert!(x >= 0.0 && y >= 0.0 && x + w <= 1.0001 && y + h <= 1.0001);
    t.cancel_editor();
}

#[test]
fn the_editor_shows_and_drags_controls_the_game_hides() {
    let mut t = TouchControls::default();
    t.settings.mode = Mode::Tap;
    t.settings.dynamic_joystick = true;
    assert!(!t.visible(Control::Shield));
    t.open_editor();
    // The shield button and the dynamic joystick's home can be placed.
    assert!(t.visible(Control::Shield));
    let home = t.placement(Control::Move).center;
    assert_eq!(t.control_at(home), Some(Control::Move));
    let a = crate::hud::HudAtlas::build(std::path::Path::new("/nonexistent/horizon-touch-editor"))
        .unwrap();
    let mut idle = crate::hud::HudBatch::new([1920, 1080]);
    t.draw(&mut idle, &a);
    tap(&mut t, 1, home);
    let mut picked = crate::hud::HudBatch::new([1920, 1080]);
    t.draw(&mut picked, &a);
    assert!(picked.vertices.len() > idle.vertices.len());
    t.cancel_editor();
    assert!(!t.visible(Control::Shield));
}

#[test]
fn the_shield_can_be_picked_and_dragged_without_one_equipped() {
    let mut t = TouchControls::default();
    t.settings.button_visibility[Control::Shield as usize] = layout::SHOWN;
    assert!(!t.shield_available);
    t.open_editor();
    assert!(t.visible(Control::Shield));
    let center = t.placement(Control::Shield).center;
    t.edit_touch(1, center, true, false);
    assert_eq!(t.editor.as_ref().unwrap().selected, Some(Control::Shield));
    let to = [center[0] - 0.05, center[1] - 0.05];
    t.edit_touch(1, to, false, true);
    let moved = t.settings.placement(Control::Shield).center;
    assert!((moved[0] - to[0]).abs() < 0.001 && (moved[1] - to[1]).abs() < 0.001);
    t.cancel_editor();
    assert!(!t.visible(Control::Shield));
}
