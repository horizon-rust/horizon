//! Touch screens (Android; docs/android.md). In the world the touches drive the on-screen controls
//! (`horizon_client::touch`), which the pad merges with a real controller. In menus a touch is the
//! mouse: it moves the pointer and clicks the left button, so every screen works as with a mouse.
use super::Game;
use horizon_client::{
    hud::{HudBatch, rgba},
    touch::Phase,
};
use std::time::Instant;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalPosition,
    event::{ElementState, MouseButton, Touch, TouchPhase, WindowEvent},
    event_loop::ActiveEventLoop,
    window::WindowId,
};

impl Game<'_> {
    /// A touch. `Some` is the event to go on with in place of it (a menu's click), `None` means it
    /// was handled.
    pub(super) fn touch_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        touch: Touch,
    ) -> Option<WindowEvent> {
        self.last_input = Instant::now();
        let (width, height) = self
            .renderer
            .as_ref()
            .map(|r| (r.config.width as f32, r.config.height as f32))?;
        self.pad.touch.set_screen(width, height);
        let at = [touch.location.x as f32, touch.location.y as f32];
        if !self.pad_in_menu() {
            let phase = match touch.phase {
                TouchPhase::Started => Phase::Started,
                TouchPhase::Moved => Phase::Moved,
                TouchPhase::Ended | TouchPhase::Cancelled => Phase::Ended,
            };
            self.pad.touch.event(touch.id, phase, at);
            return None;
        }
        // A screen is open: the fingers that held controls let go, and the touch is the mouse.
        self.pad.touch.release_all();
        let device_id = touch.device_id;
        let position = PhysicalPosition::new(touch.location.x, touch.location.y);
        let moved = WindowEvent::CursorMoved {
            device_id,
            position,
        };
        let click = |state| WindowEvent::MouseInput {
            device_id,
            state,
            button: MouseButton::Left,
        };
        match touch.phase {
            TouchPhase::Started => {
                self.window_event(event_loop, id, moved);
                Some(click(ElementState::Pressed))
            }
            TouchPhase::Moved => Some(moved),
            TouchPhase::Ended => {
                self.window_event(event_loop, id, moved);
                Some(click(ElementState::Released))
            }
            TouchPhase::Cancelled => Some(click(ElementState::Released)),
        }
    }

    /// The on-screen controls, over the world.
    pub(super) fn draw_touch_controls(&mut self, batch: &mut HudBatch) {
        if !cfg!(target_os = "android") || self.pad_in_menu() {
            return;
        }
        let scale = batch.scale;
        self.pad
            .touch
            .set_screen(batch.width * scale, batch.height * scale);
        let atlas = &self.gui.atlas;
        if let Some((origin, at)) = self.pad.touch.move_stick() {
            let radius = self.pad.touch.stick_radius();
            let (dx, dy) = (at[0] - origin[0], at[1] - origin[1]);
            let length = dx.hypot(dy).max(0.001);
            let pull = length.min(radius) / length;
            let (ring, knob) = (radius / scale, radius * 0.4 / scale);
            let (ox, oy) = (origin[0] / scale, origin[1] / scale);
            batch.fill(
                atlas,
                ox - ring,
                oy - ring,
                ring * 2.0,
                ring * 2.0,
                rgba(255, 255, 255, 36),
            );
            let (kx, ky) = (ox + dx * pull / scale, oy + dy * pull / scale);
            batch.fill(
                atlas,
                kx - knob,
                ky - knob,
                knob * 2.0,
                knob * 2.0,
                rgba(255, 255, 255, 110),
            );
        }
        for control in self.pad.touch.controls() {
            let r = control.rect;
            let (x, y, w, h) = (r.x / scale, r.y / scale, r.w / scale, r.h / scale);
            let alpha = if control.held { 140 } else { 70 };
            batch.fill(atlas, x, y, w, h, rgba(32, 32, 32, alpha));
            let text = HudBatch::text_width(atlas, control.label);
            batch.text(
                atlas,
                control.label,
                x + (w - text) / 2.0,
                y + (h - 8.0) / 2.0,
                rgba(255, 255, 255, 230),
                true,
            );
        }
    }
}
