//! The look of Horizon's graphics settings screen, in one place: sizes,
//! colours and the controls it draws with. The screen (`graphics_ui`) only
//! lays out rows and handles input; a menu theme replaces this module.
//!
//! The pages draw on vanilla's own surfaces (`vanilla`): the list background and its
//! separators, a selection list's outline, and the `widget/*` sprites from the player's resource
//! packs, in vanilla's font. Text colours come from `menu_style` (vanilla's greys and chat
//! colours unless the layer above gives others).
use crate::menu_style::{self, SettingsColours};
use crate::{
    hud::{HudAtlas, HudBatch},
    text::Text,
    ui,
};

/// The settings pages' colours (`menu_style`, given at startup).
fn b() -> &'static SettingsColours {
    &menu_style::get().settings
}
mod vanilla;
thread_local! {
    /// Whether a world is behind the screen, as its backdrop was last drawn ([`veil_over`]): vanilla's
    /// list background is see-through over a world and solid over the menu's.
    static IN_WORLD: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// What a setting costs in frame time, shown with its description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cost {
    None,
    Low,
    Medium,
    High,
    /// Turning it on makes frames faster (culling, caching, batching).
    Saves,
}
impl Cost {
    pub fn label(self) -> &'static str {
        match self {
            Cost::None => "No cost",
            Cost::Low => "Low cost",
            Cost::Medium => "Medium cost",
            Cost::High => "High cost",
            Cost::Saves => "Saves time",
        }
    }
    /// `0xRRGGBB`: the palette's success, warn and danger.
    pub fn rgb(self) -> u32 {
        match self {
            Cost::None => b().ink_soft,
            Cost::Low | Cost::Saves => b().success,
            Cost::Medium => b().warn,
            Cost::High => b().danger,
        }
    }
}

use menu_style::hud_colour as hex;
/// The plate and the strip: the page's dark, see-through, so the world shows behind them.
fn glass() -> u32 {
    hex(b().page, 118)
}
/// Their 1 px edge.
pub fn edge() -> u32 {
    hex(b().ink, 26)
}
/// A hovered or focused row, lit on its group's panel.
pub fn hover() -> u32 {
    hex(b().ink, 18)
}
/// The open page in the page list.
pub fn open() -> u32 {
    hex(b().ink, 26)
}
/// The rule under the plate's heading, and between two rows of a group.
pub fn head_rule() -> u32 {
    hex(b().ink, 31)
}
pub fn row_rule() -> u32 {
    hex(b().ink, 15)
}
pub fn accent() -> u32 {
    hex(b().accent, 255)
}
/// The palette's warning colour: a note about the current choice, not an error.
pub fn warn() -> u32 {
    hex(b().warn, 255)
}
/// Vanilla's white text, and its grey for what can't be used now.
pub const WHITE: u32 = u32::MAX;
pub fn muted() -> u32 {
    hex(b().muted, 255)
}
/// A page that isn't open, a value at rest.
pub fn soft() -> u32 {
    hex(b().ink_soft, 255)
}
/// Vanilla's grey for a tooltip's lines under its first (`ChatFormatting.GRAY`).
pub const GREY: u32 = hex(0xaaaaaa, 255);

/// Sizes in GUI pixels.
/// Shorter screens than this (a 720p window at scale 3) keep a narrower margin on top.
pub const TITLED: f32 = 300.0;
pub const ROW: f32 = 20.0;
/// A group's name over its panel, and a page's over its groups while searching.
pub const GROUP_ROW: f32 = 14.0;
pub const PAGE_ROW: f32 = 16.0;
/// Space under a group's panel, and more before the next page's name.
pub const GROUP_GAP: f32 = 10.0;
pub const PAGE_GAP: f32 = 6.0;
/// The control on the right of a row (a button, a box or a slider), and the
/// left column: the search field, the page list and the buttons.
pub const CONTROL_W: f32 = 112.0;
pub const CONTROL_H: f32 = 16.0;
pub const SEARCH_H: f32 = 16.0;
/// Vanilla's scroller: the column `ui::Scroll::press` takes.
pub const SCROLLER_W: f32 = 6.0;
/// The strip under the plate that describes a setting.
pub const DOCK_H: f32 = 50.0;
pub const BUTTON_H: f32 = 20.0;
/// A row's name and control, from its group panel's edges.
pub const INSET: f32 = 10.0;

pub fn fill(batch: &mut HudBatch, atlas: &HudAtlas, [x, y, w, h]: [f32; 4], color: u32) {
    batch.fill(atlas, x, y, w, h, color);
}
/// A 1 px ring inside `rect`: the keyboard or controller focus, in white as vanilla draws it.
pub fn ring(batch: &mut HudBatch, atlas: &HudAtlas, [x, y, w, h]: [f32; 4], color: u32) {
    batch.fill(atlas, x, y, w, 1.0, color);
    batch.fill(atlas, x, y + h - 1.0, w, 1.0, color);
    batch.fill(atlas, x, y + 1.0, 1.0, h - 2.0, color);
    batch.fill(atlas, x + w - 1.0, y + 1.0, 1.0, h - 2.0, color);
}
/// The screen's plate, and the strip under it: dark glass with a thin edge.
pub fn panel(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4]) {
    fill(batch, atlas, rect, glass());
    ring(batch, atlas, rect, edge());
}
/// The backdrop under the settings pages: vanilla's list backgrounds darken what they cover,
/// so it draws nothing, but remembers whether a world is behind the screen (`in_world`).
pub fn veil_over(_batch: &mut HudBatch, _atlas: &HudAtlas, _rect: [f32; 4], in_world: bool) {
    IN_WORLD.set(in_world);
}
/// The settings pages' plate and strip: vanilla's list background between its separators.
pub fn sheet(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4]) {
    vanilla::sheet(batch, atlas, rect, IN_WORLD.get());
}
/// The rule under the plate's heading: vanilla's header separator.
pub fn heading_rule(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4]) {
    vanilla::separator(batch, atlas, rect, IN_WORLD.get());
}
/// A row's light while it is pointed at or focused: a selection list's grey edge round it, its
/// control lit too.
pub fn lit(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4]) {
    vanilla::outline(batch, atlas, rect);
}
/// The open page in the page list: vanilla's selection-list entry (a grey outline round black;
/// the focus ring makes it white).
pub fn open_page(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4]) {
    vanilla::selected(batch, atlas, rect);
}
/// The width of a value's control (a cycler, a key) from the row's right edge: a vanilla button
/// at least a control wide.
pub fn value_w(atlas: &HudAtlas, value: &str) -> f32 {
    (atlas.font.width(value) + vanilla::LABEL_PAD).max(CONTROL_W)
}
/// Where a value's control sits in the row at `y`, from `x` to `right`: a vanilla button as tall
/// as the row's other controls.
pub fn value_rect(x: f32, right: f32, y: f32) -> [f32; 4] {
    [
        x,
        y + ((ROW - CONTROL_H) / 2.0).floor(),
        right - x,
        CONTROL_H,
    ]
}
/// A sunk well: the search field, the value stepper.
pub fn well(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4]) {
    vanilla::well(batch, atlas, rect);
}
/// Vanilla's scroller: its track and thumb.
pub fn scroller(batch: &mut HudBatch, atlas: &HudAtlas, track: [f32; 4], thumb: [f32; 4]) {
    vanilla::scroller(batch, atlas, track, thumb);
}
/// `text` cut to `width` with an ellipsis, as a long name gives way to its control: the HUD
/// kit's `fit::ellipsize`.
pub use crate::hud_kit::fit::ellipsize as fit;
/// Vanilla's tick box ending at `right`, centred in the row at `y`. `lit`: pointed at or held;
/// the box lights then.
pub fn checkbox(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    right: f32,
    y: f32,
    on: bool,
    (lit, live): (bool, bool),
) {
    vanilla::checkbox(batch, atlas, [right, y], on, (lit, live));
}
/// Vanilla's button with its label centred: Undo, Calibrate..., Done.
pub fn button(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    label: &Text,
    flags: (bool, bool),
) {
    button_with_room(batch, atlas, rect, label, flags, 0.0);
}
/// A button whose label leaves `room` at its left, for a controller's glyph drawn there
/// (`widgets::action_glyph_room`). `(hover, live)`: pointed at or focused; usable.
pub fn button_with_room(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    label: &Text,
    (hover, live): (bool, bool),
    room: f32,
) {
    vanilla::button(batch, atlas, rect, label, (hover, live), room);
}
/// Vanilla's slider: its value on the track, the handle at `position`.
pub fn slider(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    value: &Text,
    position: f32,
    (hover, live): (bool, bool),
) {
    vanilla::slider(batch, atlas, rect, value, position, (hover, live));
}
/// A value that a press steps through: vanilla's cycling button.
pub fn cycler(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    value: &Text,
    (hover, live): (bool, bool),
) {
    vanilla::cycler(batch, atlas, rect, value, (hover, live));
}
/// A widget label of `[left, y, width]` inside the widget's `(top, height)`: centred when it fits,
/// else slid (vanilla `AbstractWidget.extractScrollingStringOverContents`) and clipped to the box.
fn scrolling_label(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    [left, y, width]: [f32; 3],
    (top, height): (f32, f32),
    color: u32,
) {
    ui::scrolling_text(
        batch,
        atlas,
        text,
        left,
        left + width,
        top,
        top + height,
        y,
        color,
    );
}
/// A tab as vanilla's Create World screen draws it (`MenuTabBar.MenuTabButton`), in any look:
/// the tab sprite, and on the open tab the menu background inside it and an underline under the
/// label. `(selected, lit, live)`: open; pointed at or focused; usable.
pub fn vanilla_tab(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    label: &Text,
    flags: (bool, bool, bool),
) {
    vanilla::tab(batch, atlas, rect, label, flags);
}
/// The top of a [`vanilla_tab`]'s label, for marks drawn on its line (a controller's glyphs).
pub fn vanilla_tab_label_y(y: f32, h: f32, open: bool) -> f32 {
    vanilla::tab_label_y(y, h, open)
}
/// A screen's title, centred at the top (split-screen profiles).
pub fn title(batch: &mut HudBatch, atlas: &HudAtlas, text: &Text) {
    let w = batch.width;
    ui::centered_text(batch, atlas, text, 0.0, 12.0, w, u32::MAX);
}
/// The cost's colour in the HUD's encoding: the square beside its name.
pub fn cost_color(cost: Cost) -> u32 {
    hex(cost.rgb(), 255)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atlas() -> HudAtlas {
        HudAtlas::build(&horizon_assets::resource_source::ResourceSource::directory(
            (std::path::Path::new("/nonexistent/horizon-test-assets")).to_owned(),
        ))
        .unwrap()
    }

    fn x_span(batch: &HudBatch, plain: &HudBatch) -> (f32, f32) {
        let label = |v: &&crate::hud::HudVertex| !plain.vertices.iter().any(|p| p.pos == v.pos);
        batch
            .vertices
            .iter()
            .filter(label)
            .map(|v| v.pos[0])
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), x| {
                (lo.min(x), hi.max(x))
            })
    }

    /// A label wider than its widget slides inside the widget's box instead of being cut by
    /// characters; a short one stays centred.
    #[test]
    fn long_labels_stay_in_the_widget_and_short_ones_centred() {
        let atlas = atlas();
        let rect = [10.0, 20.0, 100.0, 20.0];
        let long = Text::literal("Graphics API: Prefer Vulkan (Experimental)");
        assert!(atlas.font.width_text(&long.resolved()) > rect[2]);
        let short = Text::literal("Done");
        let short_w = atlas.font.width_text(&short.resolved());
        let draws: [(&str, fn(&mut HudBatch, &HudAtlas, [f32; 4], &Text)); 5] = [
            ("button", |b, a, r, t| button(b, a, r, t, (false, true))),
            ("slider", |b, a, r, t| {
                slider(b, a, r, t, 0.5, (false, true))
            }),
            ("cycler", |b, a, r, t| cycler(b, a, r, t, (false, true))),
            ("open tab", |b, a, r, t| {
                vanilla_tab(b, a, r, t, (true, false, true))
            }),
            ("tab", |b, a, r, t| {
                vanilla_tab(b, a, r, t, (false, true, true))
            }),
        ];
        for (name, draw) in draws {
            let draw = |batch: &mut HudBatch, text: &Text| draw(batch, &atlas, rect, text);
            let mut plain = HudBatch::scratch([640, 480]);
            draw(&mut plain, &Text::empty());
            let base = plain.vertices.len();
            let mut batch = HudBatch::scratch([640, 480]);
            draw(&mut batch, &long);
            assert!(batch.vertices.len() > base, "{name}: long label drawn");
            // The widget's own fill, track and handle also draw: only the label's vertices count.
            let label = batch
                .vertices
                .iter()
                .filter(|v| !plain.vertices.iter().any(|p| p.pos == v.pos));
            for v in label {
                assert!(
                    (rect[0]..=rect[0] + rect[2]).contains(&v.pos[0])
                        && (rect[1]..=rect[1] + rect[3]).contains(&v.pos[1]),
                    "{name}: {:?} outside the widget",
                    v.pos
                );
            }
            // The label's own glyphs (past the widget's fill) start centred.
            let mut batch = HudBatch::scratch([640, 480]);
            draw(&mut batch, &short);
            let (lo, hi) = x_span(&batch, &plain);
            let middle = (lo + hi) / 2.0;
            let want = rect[0] + rect[2] / 2.0;
            assert!(
                (middle - want).abs() <= short_w / 2.0 + 2.0 && (middle - want).abs() < 3.0,
                "{name}: centre {middle}, want {want}"
            );
        }
    }

    /// A tab's label sits where vanilla's `MenuTabButton` puts it: for a 24 px tab, 8 px down
    /// while open and 10 px while not (`(top + bottom - 9) / 2 + 1`, its top 3 px lower).
    #[test]
    fn a_tab_label_sits_on_vanillas_line() {
        assert_eq!(vanilla_tab_label_y(0.0, 24.0, true), 8.0);
        assert_eq!(vanilla_tab_label_y(0.0, 24.0, false), 10.0);
        assert_eq!(vanilla_tab_label_y(40.0, 24.0, false), 50.0);
    }
}
