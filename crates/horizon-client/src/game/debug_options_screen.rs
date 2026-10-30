//! The F3 + F6 screen (`DebugOptionsScreen`): a scrollable list of the
//! debug overlay's entries with their three states, a search box and the
//! default/performance profiles.
use super::{Game, Screen};
use horizon_client::{
    debug_options::{DebugOptions, ENTRIES, LIST_WIDTH, Profile, Row, Status},
    hud::HudBatch,
    lang,
    text::Text,
    ui,
};
use winit::keyboard::KeyCode;

/// The Done button's width (`Button.builder(...).width(60)`).
const DONE_WIDTH: i32 = 60;
/// The profile buttons' width (`.width(120)`).
const PROFILE_WIDTH: i32 = 120;
impl Game<'_> {
    pub(super) fn open_debug_options(&mut self) {
        let status = ENTRIES
            .iter()
            .map(|e| {
                if self.debug_status(e.id) == Status::Always {
                    Status::Always
                } else {
                    Status::Off
                }
            })
            .collect();
        self.debug_options = Some(DebugOptions::new(status));
        self.screen = Screen::DebugOptions;
        self.keys.clear();
        self.left = false;
        self.right = false;
        self.mining = None;
        self.capture(false);
    }
    pub(super) fn close_debug_options(&mut self) {
        self.debug_options = None;
        self.screen = Screen::Playing;
        self.capture(true);
    }
    /// Whether a toggle is on now.
    fn debug_status(&self, id: &str) -> Status {
        let on = match id {
            "overlay" => self.debug,
            "fps_chart" => self.debug_charts.fps,
            "network_chart" => self.debug_charts.network,
            "hitboxes" => self.show_hitboxes,
            "chunk_borders" => self.show_chunk_borders,
            "improved_transparency" => self.settings.improved_transparency,
            _ => false,
        };
        if on { Status::Always } else { Status::Off }
    }
    /// Set a toggle, as the F3 keys do.
    fn set_debug_status(&mut self, id: &str, status: Status) {
        let on = status != Status::Off;
        match id {
            "overlay" => self.debug = on,
            "fps_chart" => {
                if self.debug_charts.fps != on {
                    self.debug_charts.toggle_fps(&mut self.debug);
                }
            }
            "network_chart" => {
                if self.debug_charts.network != on {
                    self.debug_charts.toggle_network(&mut self.debug);
                }
            }
            "hitboxes" => self.show_hitboxes = on,
            "chunk_borders" => self.show_chunk_borders = on,
            "improved_transparency" => {
                self.settings.improved_transparency = on;
                self.settings.save(&Self::options_path());
                self.apply_settings();
            }
            _ => {}
        }
    }
    fn apply_debug_profile(&mut self, profile: Profile) {
        for (i, entry) in ENTRIES.iter().enumerate() {
            self.set_debug_status(entry.id, profile.status(entry.id));
            if let Some(options) = self.debug_options.as_mut() {
                options.status[i] = profile.status(entry.id);
            }
        }
        if let Some(options) = self.debug_options.as_mut() {
            options.profile = profile;
        }
    }
    pub(super) fn draw_debug_options(&mut self, batch: &mut HudBatch) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let Some(options) = self.debug_options.as_mut() else {
            return;
        };
        options.scroll = options.scroll.clamp(
            0,
            (options.content_height() - options.list_height(h)).max(0),
        );
        let rows = options.row_rects(w);
        let search = options.search.clone();
        let typing = options.typing;
        let profile = options.profile;
        let atlas = &self.gui.atlas;
        // Screen.renderBackground: the menu's dimming over the world.
        batch.fill(atlas, 0.0, 0.0, w as f32, h as f32, 0xC0_10_10_10);
        let centered = |batch: &mut HudBatch, text: &Text, y: i32, color: u32| {
            let width = HudBatch::text_width(atlas, &text.plain());
            batch.text_styled(atlas, text, (w as f32 - width) / 2.0, y as f32, color, true);
        };
        centered(
            batch,
            &Text::translate("debug.options.title", &[]),
            15,
            u32::MAX,
        );
        // `MultiLineTextWidget(SUBTITLE, font).setMaxWidth(rowWidth)`: the
        // warning wraps to the list's width, centred line by line.
        let warning = lang::get_or(
            "debug.options.warning",
            "These options are for testing purposes only.",
        );
        let lines = wrap(&warning, LIST_WIDTH as f32, |s| {
            HudBatch::text_width(atlas, s)
        });
        for (i, line) in lines.into_iter().enumerate() {
            centered(
                batch,
                &Text::literal(line),
                36 + i as i32 * 10,
                0xFF_FF_55_55,
            );
        }
        // The list; rows outside its box are left out.
        let top = 61;
        let bottom = h - 33;
        for (row, [x, y, width, height]) in rows {
            if y + height < top || y > bottom {
                continue;
            }
            match row {
                Row::Category(key) => {
                    centered(batch, &Text::translate(key, &[]), y + 6, 0xFF_FF_FF_55);
                }
                Row::Entry(i, status) => {
                    let label = lang::get_or(ENTRIES[i].label_key, ENTRIES[i].label);
                    batch.text(
                        atlas,
                        &label,
                        (x + 2) as f32,
                        (y + 6) as f32,
                        u32::MAX,
                        true,
                    );
                    // The state button at the row's right (`CycleButton`).
                    let sw = 90.0;
                    let sx = (x + width) as f32 - sw;
                    let hover = self.cursor.0 as i32 >= sx as i32
                        && (self.cursor.0 as i32) < (sx as i32 + sw as i32)
                        && self.cursor.1 as i32 >= y
                        && (self.cursor.1 as i32) < y + height;
                    ui::button(
                        batch,
                        atlas,
                        sx,
                        y as f32,
                        sw,
                        height as f32,
                        &status.text(),
                        true,
                        hover,
                    );
                }
            }
        }
        // The footer: the profiles, then Done.
        let mut x = w / 2 - (PROFILE_WIDTH * 2 + 4 + 8 + DONE_WIDTH) / 2;
        for p in [Profile::Default, Profile::Performance] {
            // `profileButton.active = !isUsingProfile(profile)`: the current
            // profile's button is the disabled one.
            let active = p != profile;
            let hover = self.cursor.0 as i32 >= x
                && (self.cursor.0 as i32) < x + PROFILE_WIDTH
                && self.cursor.1 as i32 >= h - 27
                && (self.cursor.1 as i32) < h - 7;
            ui::button(
                batch,
                atlas,
                x as f32,
                (h - 27) as f32,
                PROFILE_WIDTH as f32,
                20.0,
                &p.text(),
                active,
                hover,
            );
            x += PROFILE_WIDTH + 4;
        }
        x += 8;
        let hover = self.cursor.0 as i32 >= x
            && (self.cursor.0 as i32) < x + DONE_WIDTH
            && self.cursor.1 as i32 >= h - 27
            && (self.cursor.1 as i32) < h - 7;
        ui::button(
            batch,
            atlas,
            x as f32,
            (h - 27) as f32,
            DONE_WIDTH as f32,
            20.0,
            &Text::translate("gui.done", &[]),
            true,
            hover,
        );
        // The search box, above the list.
        let search_text = if search.is_empty() {
            lang::get_or("debug.options.search", "Search...")
        } else {
            search.clone()
        };
        let search_x = w / 2 + LIST_WIDTH / 2 - 100;
        batch.fill(atlas, search_x as f32, 17.0, 100.0, 18.0, 0xFF_00_00_00);
        batch.text(
            atlas,
            &search_text,
            (search_x + 4) as f32,
            22.0,
            if search.is_empty() {
                0xFF_80_80_80
            } else {
                u32::MAX
            },
            false,
        );
        if typing {
            let caret = HudBatch::text_width(atlas, &search) as i32;
            batch.fill(
                atlas,
                (search_x + 4 + caret) as f32,
                21.0,
                1.0,
                11.0,
                0xFF_FF_FF_FF,
            );
        }
    }
    /// A press in the screen: a state button cycles, Done closes.
    pub(super) fn debug_options_click(&mut self) {
        let (w, h) = self.gui_size();
        let (w, h) = (w as i32, h as i32);
        let cursor = (self.cursor.0 as i32, self.cursor.1 as i32);
        let Some(options) = self.debug_options.as_mut() else {
            return;
        };
        // The footer buttons first.
        let mut x = w / 2 - (PROFILE_WIDTH * 2 + 4 + 8 + DONE_WIDTH) / 2;
        for p in [Profile::Default, Profile::Performance] {
            if cursor.0 >= x
                && cursor.0 < x + PROFILE_WIDTH
                && cursor.1 >= h - 27
                && cursor.1 < h - 7
            {
                self.apply_debug_profile(p);
                return;
            }
            x += PROFILE_WIDTH + 4;
        }
        x += 8;
        if cursor.0 >= x && cursor.0 < x + DONE_WIDTH && cursor.1 >= h - 27 && cursor.1 < h - 7 {
            self.close_debug_options();
            return;
        }
        // The search box.
        let search_x = w / 2 + LIST_WIDTH / 2 - 100;
        if cursor.1 >= 17 && cursor.1 < 35 && cursor.0 >= search_x && cursor.0 < search_x + 100 {
            options.typing = true;
            return;
        }
        options.typing = false;
        let rows = options.row_rects(w);
        for (row, [rx, ry, rw, rh]) in rows {
            if let Row::Entry(i, status) = row
                && cursor.0 >= rx
                && cursor.0 < rx + rw
                && cursor.1 >= ry
                && cursor.1 < ry + rh
            {
                let next = status.next();
                options.status[i] = next;
                let id = ENTRIES[i].id;
                self.set_debug_status(id, next);
                return;
            }
        }
    }
    /// A key in the screen: Escape closes, the search box takes text.
    pub(super) fn debug_options_key(&mut self, key: KeyCode, text: Option<&str>) -> bool {
        if key == KeyCode::Escape {
            self.close_debug_options();
            return true;
        }
        let Some(options) = self.debug_options.as_mut() else {
            return false;
        };
        if !options.typing {
            return false;
        }
        match key {
            KeyCode::Backspace => {
                options.search.pop();
            }
            KeyCode::Enter => options.typing = false,
            _ => {
                if let Some(text) = text {
                    options.search.push_str(text);
                }
            }
        }
        options.scroll = 0;
        true
    }
    pub(super) fn debug_options_scroll(&mut self, amount: f32) {
        let (_, h) = self.gui_size();
        let h = h as i32;
        if let Some(options) = self.debug_options.as_mut() {
            options.scroll = options.scrolled(amount, h);
        }
    }
}

/// Greedy word wrap to `width` (`StringSplitter`): a word wider than the
/// box stands alone on its line.
fn wrap(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if line.is_empty() || measure(&candidate) <= width {
            line = candidate;
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_owned();
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
