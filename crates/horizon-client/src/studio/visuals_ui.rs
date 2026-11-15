//! Visuals & Replay Inspector Sidebar UI
//!
//! Right-docked inspector panel with a draggable resize border handle.
//! Provides a solid dark background to prevent world/sky bleed during replay inspection.

use horizon_client_render::{
    graphics_style as style,
    hud::{HudAtlas, HudBatch},
    text::Text,
    ui,
};

/// In-game replay visuals sidebar UI controller and renderer.
#[derive(Clone, Debug)]
pub struct VisualsUi {
    pub open: bool,
    pub width: f32,
    pub resizing_width: bool,
    pub scroll_y: f32,
}

impl Default for VisualsUi {
    fn default() -> Self {
        Self {
            open: false,
            width: 195.0,
            resizing_width: false,
            scroll_y: 0.0,
        }
    }
}

impl VisualsUi {
    pub const HEAD_H: f32 = 22.0;
    pub const TOP_OFFSET: f32 = 24.0;
    pub const MIN_WIDTH: f32 = 160.0;
    pub const DEFAULT_WIDTH: f32 = 195.0;

    pub fn new() -> Self {
        Self::default()
    }

    /// Resize sidebar width via dragging, clamped to bounds.
    pub fn handle_resize_drag(&mut self, cursor_x: f32, screen_width: f32) {
        if self.resizing_width {
            let max_w = (screen_width * 0.6).clamp(240.0, 480.0);
            let new_width = (screen_width - cursor_x).clamp(Self::MIN_WIDTH, max_w);
            self.width = new_width.round();
        }
    }

    /// Complete dragging when mouse button is released.
    pub fn handle_mouse_up(&mut self) {
        self.resizing_width = false;
    }

    /// Sidebar panel geometry [x, y, width, height].
    pub fn panel_rect(&self, screen_w: f32, screen_h: f32, bottom_offset: f32) -> [f32; 4] {
        let top_offset = Self::TOP_OFFSET;
        let panel_h = (screen_h - top_offset - bottom_offset).max(80.0);
        let panel_x = screen_w - self.width;
        let panel_y = top_offset;
        [panel_x, panel_y, self.width, panel_h]
    }

    /// Whether cursor is hovering over the sidebar or its resize handle.
    pub fn is_hovered(
        &self,
        cursor: (f32, f32),
        screen_w: f32,
        screen_h: f32,
        bottom_offset: f32,
    ) -> bool {
        if !self.open {
            return false;
        }
        let [x, y, _w, h] = self.panel_rect(screen_w, screen_h, bottom_offset);
        cursor.0 >= x - 4.0 && cursor.0 <= screen_w && cursor.1 >= y && cursor.1 <= y + h
    }

    /// Render solid dark sidebar with header, border, resize handle, and close button.
    pub fn render(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        bottom_offset: f32,
    ) {
        if !self.open {
            return;
        }

        let [panel_x, panel_y, panel_w, panel_h] =
            self.panel_rect(batch.width, batch.height, bottom_offset);

        // 1. Solid background panel (opaque so sky/world never bleeds through)
        style::fill(
            batch,
            atlas,
            [panel_x, panel_y, panel_w, panel_h],
            0xFF14_161D,
        );
        style::fill(batch, atlas, [panel_x, panel_y, 1.0, panel_h], 0xFF2D_323F);

        // 2. Left resize border handle
        let handle_hovered = cursor.0 >= panel_x - 3.0
            && cursor.0 <= panel_x + 3.0
            && cursor.1 >= panel_y
            && cursor.1 <= panel_y + panel_h;
        if self.resizing_width {
            style::fill(
                batch,
                atlas,
                [panel_x - 1.0, panel_y, 2.0, panel_h],
                style::accent(),
            );
        } else if handle_hovered {
            style::fill(
                batch,
                atlas,
                [panel_x - 1.0, panel_y, 2.0, panel_h],
                0xAAFF_FFFF,
            );
        }

        // 3. Header: solid studio header dark
        let head_h = Self::HEAD_H;
        style::fill(
            batch,
            atlas,
            [panel_x, panel_y, panel_w, head_h],
            0xFF18_1B24,
        );
        style::fill(
            batch,
            atlas,
            [panel_x, panel_y + head_h - 1.0, panel_w, 1.0],
            0xFF2D_323F,
        );

        batch.text_styled(
            atlas,
            &Text::translate("horizon.studio.visuals.title", &[]),
            panel_x + 8.0,
            panel_y + 5.0,
            style::WHITE,
            true,
        );

        // 4. Close button [✕]
        let close_rect = [panel_x + panel_w - 18.0, panel_y + 3.0, 15.0, 15.0];
        let close_hover = ui::hovered(
            cursor,
            close_rect[0],
            close_rect[1],
            close_rect[2],
            close_rect[3],
        );
        style::button(
            batch,
            atlas,
            close_rect,
            &Text::literal("✕"),
            (close_hover, true, false),
        );
    }

    /// Handle click on sidebar: close button or left resize handle. Returns true if handled.
    pub fn handle_mouse_click(
        &mut self,
        cursor: (f32, f32),
        screen_w: f32,
        screen_h: f32,
        bottom_offset: f32,
    ) -> bool {
        if !self.open {
            return false;
        }

        let [panel_x, panel_y, panel_w, panel_h] =
            self.panel_rect(screen_w, screen_h, bottom_offset);

        // Click on left resize handle
        if cursor.0 >= panel_x - 4.0
            && cursor.0 <= panel_x + 4.0
            && cursor.1 >= panel_y
            && cursor.1 <= panel_y + panel_h
        {
            self.resizing_width = true;
            return true;
        }

        if cursor.0 < panel_x
            || cursor.0 > screen_w
            || cursor.1 < panel_y
            || cursor.1 > panel_y + panel_h
        {
            return false;
        }

        // Click on close button
        let close_rect = [panel_x + panel_w - 18.0, panel_y + 3.0, 15.0, 15.0];
        if ui::hovered(
            cursor,
            close_rect[0],
            close_rect[1],
            close_rect[2],
            close_rect[3],
        ) {
            self.open = false;
            return true;
        }

        true
    }

    /// Handle mouse scroll inside the sidebar.
    pub fn handle_scroll(&mut self, delta_y: f32, screen_h: f32, bottom_offset: f32) -> bool {
        if !self.open {
            return false;
        }
        let panel_h = (screen_h - Self::TOP_OFFSET - bottom_offset).max(80.0);
        let content_h = (panel_h - Self::HEAD_H).max(1.0);
        let max_scroll = (content_h - 100.0).max(0.0);
        self.scroll_y = (self.scroll_y - delta_y * 16.0).clamp(0.0, max_scroll);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visuals_sidebar_resizing_clamps_within_bounds() {
        let mut ui = VisualsUi::new();
        assert!(!ui.resizing_width);
        ui.handle_resize_drag(500.0, 960.0);
        assert_eq!(ui.width, 195.0);

        ui.resizing_width = true;
        ui.handle_resize_drag(700.0, 960.0);
        assert_eq!(ui.width, 260.0);

        ui.handle_resize_drag(900.0, 960.0);
        assert_eq!(ui.width, VisualsUi::MIN_WIDTH);

        ui.handle_resize_drag(100.0, 960.0);
        assert_eq!(ui.width, 480.0);

        ui.handle_mouse_up();
        assert!(!ui.resizing_width);
    }

    #[test]
    fn visuals_sidebar_mouse_click_handles_close_and_resize() {
        let mut ui = VisualsUi::new();
        let (screen_w, screen_h) = (800.0, 600.0);
        let bottom_offset = 64.0;

        // Closed sidebar ignores clicks
        assert!(!ui.handle_mouse_click((700.0, 100.0), screen_w, screen_h, bottom_offset));

        ui.open = true;
        let [panel_x, panel_y, panel_w, _panel_h] =
            ui.panel_rect(screen_w, screen_h, bottom_offset);

        // Click outside left
        assert!(!ui.handle_mouse_click((panel_x - 10.0, 100.0), screen_w, screen_h, bottom_offset));

        // Click on left resize handle
        assert!(ui.handle_mouse_click((panel_x - 1.0, 100.0), screen_w, screen_h, bottom_offset));
        assert!(ui.resizing_width);
        ui.handle_mouse_up();

        // Click on close button
        let close_x = panel_x + panel_w - 10.0;
        let close_y = panel_y + 10.0;
        assert!(ui.handle_mouse_click((close_x, close_y), screen_w, screen_h, bottom_offset));
        assert!(!ui.open);
    }

    #[test]
    fn visuals_sidebar_panel_geometry_and_hover() {
        let mut ui = VisualsUi::new();
        let (screen_w, screen_h) = (1000.0, 800.0);
        let bottom_offset = 70.0;

        assert!(!ui.is_hovered((900.0, 100.0), screen_w, screen_h, bottom_offset));

        ui.open = true;
        let rect = ui.panel_rect(screen_w, screen_h, bottom_offset);
        assert_eq!(rect[0], 1000.0 - 195.0);
        assert_eq!(rect[1], VisualsUi::TOP_OFFSET);
        assert_eq!(rect[2], 195.0);
        assert_eq!(rect[3], 800.0 - VisualsUi::TOP_OFFSET - 70.0);

        assert!(ui.is_hovered((900.0, 100.0), screen_w, screen_h, bottom_offset));
        assert!(ui.is_hovered((rect[0] - 2.0, 100.0), screen_w, screen_h, bottom_offset));
        assert!(!ui.is_hovered((rect[0] - 10.0, 100.0), screen_w, screen_h, bottom_offset));
    }
}
