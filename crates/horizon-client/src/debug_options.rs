//! `DebugOptionsScreen` (F3 + F6): vanilla's debug entries with a three-state
//! choice each (`DebugScreenEntryStatus`: always, in the overlay, never), the
//! search box and the default/performance profiles.
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
    /// `DebugScreenEntryStatus`' label (`debug.entry.always`,
    /// `debug.entry.overlay`, `options.off`).
    pub fn text(self) -> Text {
        match self {
            Self::Always => Text::translate("debug.entry.always", &[]),
            Self::Overlay => Text::translate("debug.entry.overlay", &[]),
            Self::Off => Text::translate("options.off", &[]),
        }
    }
}
/// `DebugEntryCategory`: the two headings the entries sort under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// `DebugEntryCategory.SCREEN_TEXT`.
    Text,
    /// `DebugEntryCategory.RENDERER`.
    Renderer,
}
impl Category {
    pub fn text(self) -> Text {
        match self {
            Self::Text => Text::translate("debug.options.category.text", &[]),
            Self::Renderer => Text::translate("debug.options.category.renderer", &[]),
        }
    }
}
/// The switch an entry drives in the game, for the ones it can.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    /// `entity_hitboxes`: F3 + B.
    Hitboxes,
    /// `chunk_borders`: F3 + G.
    ChunkBorders,
}
/// One `DebugScreenEntries` entry: its id, its category and, where Horizon
/// draws it, the switch it drives.
pub struct Entry {
    /// `Identifier`'s path (the screen shows the id, not a label).
    pub id: &'static str,
    pub category: Category,
    pub toggle: Option<Toggle>,
}
const fn e(id: &'static str, category: Category) -> Entry {
    Entry {
        id,
        category,
        toggle: None,
    }
}
const fn e_toggle(id: &'static str, category: Category, toggle: Toggle) -> Entry {
    Entry {
        id,
        category,
        toggle: Some(toggle),
    }
}
/// `DebugScreenEntries`' entries, in its registration order.
pub const ENTRIES: &[Entry] = &[
    e("game_version", Category::Text),
    e("fps", Category::Text),
    e("tps", Category::Text),
    e("memory", Category::Text),
    e("detailed_memory", Category::Text),
    e("system_specs", Category::Text),
    e("looking_at_block_state", Category::Text),
    e("looking_at_block_tags", Category::Text),
    e("looking_at_fluid_state", Category::Text),
    e("looking_at_fluid_tags", Category::Text),
    e("looking_at_entity", Category::Text),
    e("looking_at_entity_tags", Category::Text),
    e("chunk_render_stats", Category::Text),
    e("chunk_generation_stats", Category::Text),
    e("entity_render_stats", Category::Text),
    e("particle_render_stats", Category::Text),
    e("chunk_source_stats", Category::Text),
    e("player_position", Category::Text),
    e("player_section_position", Category::Text),
    e("player_speed", Category::Text),
    e("light_levels", Category::Text),
    e("heightmap", Category::Text),
    e("biome", Category::Text),
    e("local_difficulty", Category::Text),
    e("day_count", Category::Text),
    e("entity_spawn_counts", Category::Text),
    e("sound_mood", Category::Text),
    e("sound_cache", Category::Text),
    e("post_effects", Category::Text),
    e_toggle("entity_hitboxes", Category::Renderer, Toggle::Hitboxes),
    e_toggle("chunk_borders", Category::Renderer, Toggle::ChunkBorders),
    e("3d_crosshair", Category::Renderer),
    e("chunk_section_paths", Category::Renderer),
    e("gpu_utilization", Category::Text),
    e("simple_performance_impactors", Category::Text),
    e("chunk_section_octree", Category::Renderer),
    e("visualize_water_levels", Category::Renderer),
    e("visualize_heightmap", Category::Renderer),
    e("visualize_collision_boxes", Category::Renderer),
    e("visualize_entity_supporting_blocks", Category::Renderer),
    e("visualize_block_light_levels", Category::Renderer),
    e("visualize_sky_light_levels", Category::Renderer),
    e("visualize_solid_faces", Category::Renderer),
    e("visualize_chunks_on_server", Category::Renderer),
    e("visualize_sky_light_sections", Category::Renderer),
    e("chunk_section_visibility", Category::Renderer),
];
/// `OptionEntry.BUTTON_WIDTH`.
pub const BUTTON_WIDTH: i32 = 60;
/// `ContainerObjectSelectionList`'s row and spacing.
pub const ROW_HEIGHT: i32 = 20;
pub const ROW_GAP: i32 = 4;
/// The list's width (`OptionList.getRowWidth`).
pub const LIST_WIDTH: i32 = 310;
/// `HeaderAndFooterLayout(61, 33)`.
pub const HEADER_HEIGHT: i32 = 61;
pub const FOOTER_HEIGHT: i32 = 33;
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
}
/// One row as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A category heading (`CategoryEntry`).
    Category(Category),
    /// An entry row: its index and its state.
    Entry(usize, Status),
}
#[derive(Default)]
pub struct DebugOptions {
    /// The state of every [`ENTRIES`] row, by index.
    pub status: Vec<Status>,
    pub search: String,
    pub scroll: i32,
    /// The profile the list currently matches, if one was applied.
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
    /// `updateSearch`: the entries whose id holds `search`, grouped under
    /// their categories, a heading only while a row follows it.
    pub fn rows(&self) -> Vec<Row> {
        let needle = self.search.trim().to_lowercase();
        // `OptionList` sorts the entries by category (`DebugEntryCategory`'s
        // sort key), then by id, so each heading appears once.
        let mut matched: Vec<usize> = (0..ENTRIES.len())
            .filter(|i| needle.is_empty() || ENTRIES[*i].id.contains(needle.as_str()))
            .collect();
        matched.sort_by(|a, b| {
            let key = |i: usize| match ENTRIES[i].category {
                Category::Text => 1,
                Category::Renderer => 2,
            };
            (key(*a), ENTRIES[*a].id).cmp(&(key(*b), ENTRIES[*b].id))
        });
        let mut out = Vec::new();
        let mut category = None;
        for i in matched {
            let entry = &ENTRIES[i];
            if category != Some(entry.category) {
                out.push(Row::Category(entry.category));
                category = Some(entry.category);
            }
            out.push(Row::Entry(i, self.status[i]));
        }
        out
    }
    pub fn content_height(&self) -> i32 {
        (self.rows().len() as i32) * (ROW_HEIGHT + ROW_GAP)
    }
    pub fn list_height(&self, height: i32) -> i32 {
        (height - HEADER_HEIGHT - FOOTER_HEIGHT).max(0)
    }
    /// The scroll a wheel turn leaves, clamped to the content.
    pub fn scrolled(&self, amount: f32, height: i32) -> i32 {
        let max = (self.content_height() - self.list_height(height)).max(0);
        (self.scroll - (amount * 10.0) as i32).clamp(0, max)
    }
    /// The row rects, in screen coordinates.
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
    /// The three state buttons of a row, right-aligned like vanilla's:
    /// never, in the overlay, always, left to right.
    pub fn button_rects([x, y, width, height]: [i32; 4]) -> [(Status, [i32; 4]); 3] {
        let right = x + width;
        let h = height - 2;
        let y = y + 2;
        [
            (Status::Off, [right - 3 * BUTTON_WIDTH, y, BUTTON_WIDTH, h]),
            (
                Status::Overlay,
                [right - 2 * BUTTON_WIDTH, y, BUTTON_WIDTH, h],
            ),
            (Status::Always, [right - BUTTON_WIDTH, y, BUTTON_WIDTH, h]),
        ]
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
    fn every_vanilla_entry_is_listed_under_its_category() {
        assert_eq!(ENTRIES.len(), 46);
        let rows = screen("").rows();
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
        assert!(matches!(rows[0], Row::Category(Category::Text)));
    }

    #[test]
    fn search_filters_ids_and_drops_empty_headings() {
        let rows = screen("biome").rows();
        let entries: Vec<&str> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Entry(i, _) => Some(ENTRIES[*i].id),
                Row::Category(_) => None,
            })
            .collect();
        assert!(entries.contains(&"biome"));
        assert!(screen("zzz").rows().is_empty());
    }

    #[test]
    fn only_the_entries_horizon_draws_have_a_switch() {
        let switches: Vec<&str> = ENTRIES
            .iter()
            .filter(|e| e.toggle.is_some())
            .map(|e| e.id)
            .collect();
        assert_eq!(switches, vec!["entity_hitboxes", "chunk_borders"]);
    }

    #[test]
    fn buttons_are_right_aligned_never_overlay_always() {
        let rects = DebugOptions::button_rects([10, 100, 310, 20]);
        assert_eq!(rects[0].0, Status::Off);
        assert_eq!(rects[2].0, Status::Always);
        assert_eq!(rects[0].1[0], 10 + 310 - 3 * BUTTON_WIDTH);
        assert_eq!(rects[2].1[0] + BUTTON_WIDTH, 10 + 310);
        // No overlap between the three.
        assert_eq!(rects[0].1[0] + BUTTON_WIDTH, rects[1].1[0]);
        assert_eq!(rects[1].1[0] + BUTTON_WIDTH, rects[2].1[0]);
    }

    #[test]
    fn scroll_stops_at_the_content() {
        let mut d = screen("");
        d.scroll = 0;
        let max = (d.content_height() - d.list_height(400)).max(0);
        assert_eq!(d.scrolled(-100.0, 400), max);
        d.scroll = max;
        assert_eq!(d.scrolled(100.0, 400), 0);
    }
}
