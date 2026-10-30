//! VR controllers in menus (Horizon's own; docs/vr.md): while a menu, a screen or the title
//! is open, the pointing hand's laser moves the pointer to where it meets the HUD panel, and
//! its trigger is the left mouse button, so every screen works as with a mouse (hover
//! highlights, clicks, drags, sliders). The right stick scrolls and A clicks too, as for any
//! controller in pointer mode (`pad_menu`); the left stick click puts the panel in front of
//! the head again. The renderer finds the hit and draws the laser (`gfx/xr.rs`).
use super::{Game, Screen};
use std::time::Instant;
use winit::event_loop::ActiveEventLoop;

/// Trigger travel that presses the pointer's button, and that releases it again.
const PRESS: f32 = 0.6;
const RELEASE: f32 = 0.4;

/// The laser's state between polls.
#[derive(Default)]
pub(super) struct XrMenu {
    /// The trigger is pulled (past `PRESS`, not yet back under `RELEASE`).
    pulled: bool,
    /// The pointer's button is down because of it.
    pressed: bool,
    /// The recentre button was down last poll.
    recentre: bool,
    /// Where the laser last put the pointer (window pixels).
    at: Option<[f32; 2]>,
}

impl Game<'_> {
    /// Once per poll, after the pad: the laser drives the pointer while a menu is open.
    pub(super) fn xr_pointer_frame(&mut self, event_loop: &ActiveEventLoop) {
        let menu = self.pad_in_menu() && self.settings.controller.enabled;
        let Some(pointer) = self.renderer.as_mut().and_then(|r| r.xr_pointer(menu)) else {
            return;
        };
        if !menu || !pointer.tracked {
            if self.pad.xr.pressed && menu {
                self.pad_pointer_button(event_loop, false, false);
            }
            self.pad.xr = XrMenu::default();
            self.pad.menu.laser = false;
            return;
        }
        // An inventory throws a stack with that button; the panel opened in front there.
        if pointer.recentre
            && !self.pad.xr.recentre
            && !matches!(self.screen, Screen::Inventory | Screen::Creative)
            && let Some(r) = self.renderer.as_mut()
        {
            r.xr_recentre();
        }
        self.pad.xr.recentre = pointer.recentre;
        self.pad.menu.laser = true;
        if let Some(at) = pointer.at
            && self.pad.xr.at != Some(at)
            && let Some(batch) = self.gui_scale()
        {
            self.pad.xr.at = Some(at);
            self.pad_pointer_to(event_loop, (at[0] / batch.scale, at[1] / batch.scale));
            self.pad.in_use = true;
            self.last_input = Instant::now();
        }
        let pulled = if self.pad.xr.pulled {
            pointer.trigger > RELEASE
        } else {
            pointer.trigger > PRESS
        };
        if pulled && !self.pad.xr.pulled && pointer.at.is_some() {
            self.pad.xr.pressed = true;
            self.pad_pointer_button(event_loop, false, true);
            self.last_input = Instant::now();
        } else if !pulled && self.pad.xr.pressed {
            self.pad.xr.pressed = false;
            self.pad_pointer_button(event_loop, false, false);
        }
        self.pad.xr.pulled = pulled;
    }
}
