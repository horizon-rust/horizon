//! `DebugOptionsScreen` (F3 + F6): the debug overlay's toggles as a list, with
//! `DebugScreenEntryStatus`' three states per entry, a search box and the
//! default/performance profiles.
use crate::text::Text;

/// `DebugScreenEntryStatus`: where an entry draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// `ALWAYS_ON`: in the overlay and in F3.
    #[default]
    Always,
    /// `IN_OVERLAY`: only while the overlay is open.
    Overlay,
    /// `NEVER`: not drawn.
    Off,
}
impl Status {
    /// `DebugScreenEntryStatus.next`: always on, in the overlay, off.
    pub fn next(self) -> Self {
        match self {
            Self::Always => Self::Overlay,
            Self::Overlay => Self::Off,
            Self::Off => Self::Always,
        }
    }
    /// `DebugScreenEntryStatus`' label (`debug.entry.always`, `debug.entry.overlay`,
    /// `options.off`).
    pub fn text(self) -> Text {
        match self {
            Self::Always => Text::translate("debug.entry.always", &[]),
            Self::Overlay => Text::translate("debug.entry.overlay", &[]),
            Self::Off => Text::translate("options.off", &[]),
        }
    }
}
/// A toggle the screen lists: a debug renderer or overlay section.
pub struct Entry {
    /// The switch it drives, as the row reports it back.
    pub id: &'static str,
    /// Vanilla's label key where one exists, else the English label.
    pub label_key: &'static str,
    pub label: &'static str,
    /// The section it belongs to (`debug.options.category.*`).
    pub category: &'static str,
}
/// The row height and spacing of the list (`ContainerObjectSelectionList`).
pub const ROW_HEIGHT: i32 = 20;
pub const ROW_GAP: i32 = 4;
/// The list's width, as the vanilla screen's `rowWidth`.
pub const LIST_WIDTH: i32 = 310;
/// `HeaderAndFooterLayout` of the vanilla screen.
pub const HEADER_HEIGHT: i32 = 61;
pub const FOOTER_HEIGHT: i32 = 33;
/// Vanilla's rows, in the order the screen lists them.
pub const ENTRIES: &[Entry] = &[
    Entry {
        id: "overlay",
        label_key: "horizon.debug.screen",
        label: "Debug screen",
        category: "debug.options.category.text",
    },
    Entry {
        id: "fps_chart",
        label_key: "horizon.debug.fpsChart",
        label: "FPS chart",
        category: "debug.options.category.text",
    },
    Entry {
        id: "network_chart",
        label_key: "horizon.debug.networkChart",
        label: "Network chart",
        category: "debug.options.category.text",
    },
    Entry {
        id: "hitboxes",
        label_key: "horizon.debug.hitboxes",
        label: "Hitboxes",
        category: "debug.options.category.renderer",
    },
    Entry {
        id: "chunk_borders",
        label_key: "horizon.debug.chunkBorders",
        label: "Chunk borders",
        category: "debug.options.category.renderer",
    },
    Entry {
        id: "improved_transparency",
        label_key: "options.improvedTransparency",
        label: "Improved Transparency",
        category: "debug.options.category.renderer",
    },
];
/// The profiles the footer offers (`DebugScreenProfile`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    #[default]
    Default,
    Performance,
}
impl Profile {
    pub fn text(self) -> Text {
        match self {
            Self::Default => Text::translate("debug.options.profile.default", &[]),
            Self::Performance => Text::translate("debug.options.profile.performance", &[]),
        }
    }
    /// What the profile sets `id` to: the default shows everything, the
    /// performance profile keeps the text and leaves the renderers off.
    pub fn status(self, id: &str) -> Status {
        match (self, id) {
            (Self::Performance, "overlay") => Status::Always,
            (Self::Performance, _) => Status::Off,
            (Self::Default, _) => Status::Always,
        }
    }
}
/// One row as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A category heading (`CategoryEntry`).
    Category(&'static str),
    /// An entry row: which entry and its state.
    Entry(usize, Status),
}
#[derive(Default)]
pub struct DebugOptions {
    /// The state of every [`ENTRIES`] row, by index.
    pub status: Vec<Status>,
    pub search: String,
    pub scroll: i32,
    pub profile: Profile,
    /// The search box has the keyboard.
    pub typing: bool,
}
impl DebugOptions {
    pub fn new(status: Vec<Status>) -> Self {
        Self {
            status,
            ..Self::default()
        }
    }
    /// `updateSearch`: the entries whose label holds `search` (case-insensitive),
    /// grouped under their categories, a heading only while a row follows it.
    pub fn rows(&self) -> Vec<Row> {
        let needle = self.search.trim().to_lowercase();
        let mut out = Vec::new();
        let mut category = None;
        for (i, entry) in ENTRIES.iter().enumerate() {
            let label = crate::lang::get_or(entry.label_key, entry.label);
            if !needle.is_empty() && !label.to_lowercase().contains(&needle) {
                continue;
            }
            if category != Some(entry.category) {
                out.push(Row::Category(entry.category));
                category = Some(entry.category);
            }
            out.push(Row::Entry(i, self.status[i]));
        }
        out
    }
    /// The content height of the rows, scrolled by at most what fits.
    pub fn content_height(&self) -> i32 {
        (self.rows().len() as i32) * (ROW_HEIGHT + ROW_GAP)
    }
    /// The list's visible height for a screen `height`.
    pub fn list_height(&self, height: i32) -> i32 {
        (height - HEADER_HEIGHT - FOOTER_HEIGHT).max(0)
    }
    /// The scroll the wheel answers with, clamped to the content.
    pub fn scrolled(&self, amount: f32, height: i32) -> i32 {
        let max = (self.content_height() - self.list_height(height)).max(0);
        (self.scroll - (amount * 10.0) as i32).clamp(0, max)
    }
    /// The row rects for a screen, in the list's own coordinates.
    pub fn row_rects(&self, width: i32) -> Vec<(Row, [i32; 4])> {
        let left = (width - LIST_WIDTH) / 2;
        self.rows()
            .into_iter()
            .enumerate()
            .map(|(i, row)| {
                let y = HEADER_HEIGHT + i as i32 * (ROW_HEIGHT + ROW_GAP) - self.scroll;
                (row, [left, y, LIST_WIDTH, ROW_HEIGHT])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(search: &str) -> DebugOptions {
        let mut d = DebugOptions::new(vec![Status::Always; ENTRIES.len()]);
        d.search = search.to_owned();
        d
    }

    #[test]
    fn status_cycles_always_overlay_off() {
        assert_eq!(Status::Always.next(), Status::Overlay);
        assert_eq!(Status::Overlay.next(), Status::Off);
        assert_eq!(Status::Off.next(), Status::Always);
    }

    #[test]
    fn categories_head_each_group_once() {
        let rows = screen("").rows();
        // Every entry, each category before its first row.
        assert_eq!(
            rows.iter()
                .filter(|r| matches!(r, Row::Category(_)))
                .count(),
            2
        );
        assert_eq!(
            rows.iter().filter(|r| matches!(r, Row::Entry(..))).count(),
            ENTRIES.len()
        );
        assert!(matches!(
            rows[0],
            Row::Category("debug.options.category.text")
        ));
    }

    #[test]
    fn search_filters_and_regroups() {
        let rows = screen("chart").rows();
        let entries: Vec<usize> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Entry(i, _) => Some(*i),
                Row::Category(_) => None,
            })
            .collect();
        assert_eq!(entries.len(), 2, "FPS and network charts");
        assert!(rows.iter().any(|r| matches!(r, Row::Category(_))));
        // A search that matches nothing leaves no heading behind.
        assert!(screen("zzz").rows().is_empty());
    }

    #[test]
    fn scroll_stops_at_the_content() {
        let mut d = screen("");
        d.scroll = 0;
        let big = d.scrolled(-100.0, 400);
        let max = (d.content_height() - d.list_height(400)).max(0);
        assert_eq!(big, max);
        d.scroll = max;
        assert_eq!(d.scrolled(100.0, 400), 0);
    }

    #[test]
    fn the_performance_profile_keeps_the_text_only() {
        assert_eq!(Profile::Performance.status("overlay"), Status::Always);
        assert_eq!(Profile::Performance.status("hitboxes"), Status::Off);
        assert_eq!(Profile::Default.status("hitboxes"), Status::Always);
    }
}
