//! The F3 + F6 screen (`DebugOptionsScreen`): vanilla's debug entries with a
//! three-state choice each, a search box and the default/performance profiles.
use super::{Game, Screen};
use horizon_client::{
    debug_options::{DebugOptions, ENTRIES, LIST_WIDTH, Profile, Row, Status, Toggle},
    hud::HudBatch,
    lang,
    text::Text,
    ui,
};
use winit::keyboard::KeyCode;

/// The Done button's width (`.width(60)`).
const DONE_WIDTH: i32 = 60;
/// The profile buttons' width (`.width(120)`).
const PROFILE_WIDTH: i32 = 120;
impl Game<'_> {
    pub(super) fn open_debug_options(&mut self) {
        let status = ENTRIES
            .iter()
            .map(|e| match e.toggle {
                Some(toggle) if !self.debug_switch_on(toggle) => Status::Off,
                _ => Status::Always,
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
    fn debug_switch_on(&self, toggle: Toggle) -> bool {
        match toggle {
            Toggle::Hitboxes => self.show_hitboxes,
            Toggle::ChunkBorders => self.show_chunk_borders,
        }
    }
    fn set_debug_switch(&mut self, toggle: Toggle, on: bool) {
        match toggle {
            Toggle::Hitboxes => self.show_hitboxes = on,
            Toggle::ChunkBorders => self.show_chunk_borders = on,
        }
    }
    /// `DebugScreenProfile`: the default profile leaves every entry on, the
    /// performance profile keeps the screen text and drops the renderers.
    fn apply_debug_profile(&mut self, profile: Profile) {
        for (i, entry) in ENTRIES.iter().enumerate() {
            let status = match (profile, entry.toggle.is_some()) {
                (_, false) => Status::Always,
                (Profile::Performance, true) => Status::Off,
                (Profile::Default, true) => Status::Always,
            };
            if let Some(options) = self.debug_options.as_mut() {
                options.status[i] = status;
            }
            if let Some(toggle) = entry.toggle {
                self.set_debug_switch(toggle, status != Status::Off);
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
        let cursor = (self.cursor.0 as i32, self.cursor.1 as i32);
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
        for (i, line) in wrap(&warning, LIST_WIDTH as f32, |s| {
            HudBatch::text_width(atlas, s)
        })
        .into_iter()
        .enumerate()
        {
            centered(
                batch,
                &Text::literal(line),
                36 + i as i32 * 10,
                0xFF_FF_55_55,
            );
        }
        // The list; rows outside its box are left out.
        let (top, bottom) = (61, h - 33);
        for (row, rect) in rows {
            let [x, y, _, height] = rect;
            if y + height < top || y > bottom {
                continue;
            }
            match row {
                Row::Category(category) => {
                    centered(batch, &category.text(), y + 6, 0xFF_FF_FF_55);
                }
                Row::Entry(i, status) => {
                    let entry = &ENTRIES[i];
                    // `OptionEntry`: the id, italic when the entry cannot be
                    // switched (vanilla's `isAllowed`).
                    let name = if entry.toggle.is_some() {
                        Text::literal(entry.id)
                    } else {
                        Text::literal(entry.id).italic()
                    };
                    batch.text_styled(atlas, &name, (x + 2) as f32, (y + 6) as f32, u32::MAX, true);
                    for (choice, [bx, by, bw, bh]) in DebugOptions::button_rects(rect) {
                        let hover = entry.toggle.is_some()
                            && cursor.0 >= bx
                            && cursor.0 < bx + bw
                            && cursor.1 >= by
                            && cursor.1 < by + bh;
                        ui::button(
                            batch,
                            atlas,
                            bx as f32,
                            by as f32,
                            bw as f32,
                            bh as f32,
                            &choice.text(),
                            entry.toggle.is_some(),
                            hover,
                        );
                        // The chosen one is drawn as the pressed button.
                        if choice == status {
                            batch.fill(
                                atlas,
                                bx as f32,
                                by as f32,
                                bw as f32,
                                bh as f32,
                                0x40_00_00_00,
                            );
                        }
                    }
                }
            }
        }
        // The footer: the profiles, then Done.
        let mut x = w / 2 - (PROFILE_WIDTH * 2 + 4 + 8 + DONE_WIDTH) / 2;
        for p in [Profile::Default, Profile::Performance] {
            // `profileButton.active = !isUsingProfile(profile)`.
            let active = p != profile;
            let hover = cursor.0 >= x
                && cursor.0 < x + PROFILE_WIDTH
                && cursor.1 >= h - 27
                && cursor.1 < h - 7;
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
        let hover =
            cursor.0 >= x && cursor.0 < x + DONE_WIDTH && cursor.1 >= h - 27 && cursor.1 < h - 7;
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
        // The search box, on the title row.
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
    /// A press in the screen: a state button picks its status, Done closes.
    pub(super) fn debug_options_click(&mut self) {
        let (w, h) = self.gui_size();
        let (w, h) = (w as i32, h as i32);
        let cursor = (self.cursor.0 as i32, self.cursor.1 as i32);
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
        let search_x = w / 2 + LIST_WIDTH / 2 - 100;
        let Some(options) = self.debug_options.as_mut() else {
            return;
        };
        if cursor.1 >= 17 && cursor.1 < 35 && cursor.0 >= search_x && cursor.0 < search_x + 100 {
            options.typing = true;
            return;
        }
        options.typing = false;
        for (row, rect) in options.row_rects(w) {
            if rect[1] + rect[3] < 61 || rect[1] > h - 33 {
                continue;
            }
            let Row::Entry(i, _) = row else {
                continue;
            };
            let Some(toggle) = ENTRIES[i].toggle else {
                continue;
            };
            for (choice, [bx, by, bw, bh]) in DebugOptions::button_rects(rect) {
                if cursor.0 >= bx && cursor.0 < bx + bw && cursor.1 >= by && cursor.1 < by + bh {
                    options.status[i] = choice;
                    self.set_debug_switch(toggle, choice != Status::Off);
                    return;
                }
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
/// Greedy word wrap to `width` (`StringSplitter`): a word wider than the box
/// stands alone on its line.
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
