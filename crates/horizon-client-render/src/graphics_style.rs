//! The look of Horizon's graphics settings screen, in one place: sizes,
//! colours and the controls it draws with. The screen (`graphics_ui`) only
//! lays out rows and handles input; a menu theme replaces this module.
//!
//! It is the design system's Blocks look (`horizon_theme::BLOCKS_DARK`,
//! `docs/design-system.md`): square panels with an outline, a light bevel
//! along the top and left and a darker lip under them, sunk wells for fields,
//! boxes and tracks, and the violet face on the one main button, in vanilla's
//! font.
use crate::{
    hud::{HudAtlas, HudBatch},
    text::Text,
    ui,
};
use horizon_theme::BLOCKS_DARK as B;

/// What a setting costs in frame time, shown with its description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cost {
    None,
    Low,
    Medium,
    High,
    /// Turning it on makes frames faster (culling).
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
            Cost::None => B.ink_soft,
            Cost::Low | Cost::Saves => B.success,
            Cost::Medium => B.warn,
            Cost::High => B.danger,
        }
    }
}

/// `0xRRGGBB` and an alpha in the HUD's colour encoding.
const fn hex(c: u32, a: u8) -> u32 {
    (a as u32) << 24 | (c & 0xff) << 16 | (c >> 8 & 0xff) << 8 | (c >> 16 & 0xff)
}
pub const PANEL: u32 = hex(B.panel, 255);
/// A group's panel, raised off the plate.
const RAISED: u32 = hex(B.raised, 255);
/// A hovered or focused row, lit on its group's panel.
pub const HOVER: u32 = hex(B.ink, 16);
/// The open page in the page list.
pub const OPEN: u32 = hex(B.ink, 31);
const BUTTON: u32 = hex(B.button, 255);
const BUTTON_BEVEL: u32 = hex(B.button_bevel, 255);
const WELL: u32 = hex(B.well, 255);
pub const BEVEL: u32 = hex(B.bevel, 255);
const LIP: u32 = hex(B.lip, 255);
const OUTLINE: u32 = hex(B.outline, 255);
const FACE: u32 = hex(B.face, 255);
const FACE_BEVEL: u32 = hex(B.face_bevel, 255);
const FACE_LIP: u32 = hex(B.face_lip, 255);
pub const ACCENT: u32 = hex(B.accent, 255);
/// Vanilla's white text, and its grey for what can't be used now.
pub const WHITE: u32 = u32::MAX;
pub const MUTED: u32 = hex(B.muted, 255);
/// A page that isn't open, a value at rest.
pub const SOFT: u32 = hex(B.ink_soft, 255);
/// Vanilla's grey for a tooltip's lines under its first (`ChatFormatting.GRAY`).
pub const GREY: u32 = hex(0xaaaaaa, 255);
/// A dim veil over the backdrop, under the panels.
pub const VEIL: u32 = hex(B.page, 140);

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
/// A block: the outline, the fill, a light top and left edge, a dark lip `lip` high.
fn block(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    [x, y, w, h]: [f32; 4],
    (face, bevel, under): (u32, u32, u32),
    lip: f32,
) {
    fill(batch, atlas, [x, y, w, h], OUTLINE);
    fill(batch, atlas, [x + 1.0, y + 1.0, w - 2.0, h - 2.0], face);
    fill(batch, atlas, [x + 1.0, y + 1.0, w - 2.0, 1.0], bevel);
    fill(batch, atlas, [x + 1.0, y + 1.0, 1.0, h - 2.0 - lip], bevel);
    fill(
        batch,
        atlas,
        [x + 1.0, y + h - 1.0 - lip, w - 2.0, lip],
        under,
    );
}
/// A button's face, bevel and lip; a control that can't be used now.
const FACE_OF_BUTTON: (u32, u32, u32) = (BUTTON, BUTTON_BEVEL, LIP);
const DIM: (u32, u32, u32) = (PANEL, PANEL, LIP);
/// The screen's plate, and the strip under it.
pub fn panel(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4]) {
    block(batch, atlas, rect, (PANEL, BEVEL, LIP), 2.0);
}
/// A group's panel, raised off the plate, its rows on it.
pub fn group(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4]) {
    block(batch, atlas, rect, (RAISED, BEVEL, LIP), 1.0);
}
/// A sunk well: fields, tick boxes, slider tracks, the scroller's track.
pub fn well(batch: &mut HudBatch, atlas: &HudAtlas, [x, y, w, h]: [f32; 4]) {
    fill(batch, atlas, [x, y, w, h], OUTLINE);
    fill(batch, atlas, [x + 1.0, y + 1.0, w - 2.0, h - 2.0], WELL);
    fill(batch, atlas, [x + 1.0, y + 1.0, w - 2.0, 1.0], LIP);
}
/// The scroller's thumb, a button on its track.
pub fn thumb(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4]) {
    block(batch, atlas, rect, FACE_OF_BUTTON, 1.0);
}
/// `text` cut to `width` with an ellipsis, as a long name gives way to its control.
pub fn fit(atlas: &HudAtlas, text: &str, width: f32) -> String {
    if atlas.font.width(text) <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    for c in text.chars() {
        if atlas.font.width(&format!("{out}{c}...")) > width {
            break;
        }
        out.push(c);
    }
    format!("{}...", out.trim_end())
}
/// A 16 px tick box ending at `right`, centred in the row at `y`.
pub fn checkbox(batch: &mut HudBatch, atlas: &HudAtlas, right: f32, y: f32, on: bool, live: bool) {
    let rect = [
        right - CONTROL_H,
        y + (ROW - CONTROL_H) / 2.0,
        CONTROL_H,
        CONTROL_H,
    ];
    well(batch, atlas, rect);
    if on {
        tick(
            batch,
            atlas,
            rect[0] + 3.0,
            rect[1] + 4.0,
            if live { ACCENT } else { MUTED },
        );
    }
}
/// A pixel tick, 10 by 8, over its own shadow.
fn tick(batch: &mut HudBatch, atlas: &HudAtlas, x: f32, y: f32, color: u32) {
    const TICK: [&str; 8] = [
        "         #",
        "        ##",
        "       ## ",
        "#     ##  ",
        "##   ##   ",
        " ## ##    ",
        "  ###     ",
        "   #      ",
    ];
    for (j, row) in TICK.iter().enumerate() {
        for (i, c) in row.chars().enumerate() {
            if c == '#' {
                let (px, py) = (x + i as f32, y + j as f32);
                fill(batch, atlas, [px + 1.0, py + 1.0, 1.0, 1.0], OUTLINE);
                fill(batch, atlas, [px, py, 1.0, 1.0], color);
            }
        }
    }
}
/// A beveled button with its label centred: a row's value, Undo, and Done
/// on the violet face (`primary`).
pub fn button(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    label: &Text,
    (hover, live, primary): (bool, bool, bool),
) {
    let [x, y, w, h] = rect;
    let (colors, lip) = match (live, primary) {
        (false, _) => (DIM, 1.0),
        (true, true) => ((FACE, FACE_BEVEL, FACE_LIP), 2.0),
        (true, false) => (FACE_OF_BUTTON, 1.0),
    };
    block(batch, atlas, rect, colors, lip);
    // Vanilla's highlighted button: a white edge.
    if hover && live {
        ring(batch, atlas, rect, WHITE);
    }
    let color = if live { WHITE } else { MUTED };
    let ty = y + ((h - 8.0) / 2.0).floor() - if primary { 1.0 } else { 0.0 };
    ui::centered_text(batch, atlas, label, x + 2.0, ty, w - 4.0, color);
}
/// Vanilla's slider: a well, the 8 px handle at `position`, the value written over both.
pub fn slider(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    value: &Text,
    position: f32,
    (hover, live): (bool, bool),
) {
    let [x, y, w, h] = rect;
    well(batch, atlas, rect);
    let at = x + 1.0 + (position.clamp(0.0, 1.0) * (w - 10.0)).round();
    let handle = [at, y, 8.0, h];
    block(
        batch,
        atlas,
        handle,
        if live { FACE_OF_BUTTON } else { DIM },
        1.0,
    );
    if hover && live {
        ring(batch, atlas, handle, WHITE);
    }
    let color = if live { WHITE } else { MUTED };
    let ty = y + ((h - 8.0) / 2.0).floor();
    ui::centered_text(batch, atlas, value, x + 2.0, ty, w - 4.0, color);
}
/// A value that a press steps through: in a well, between the arrows that say so.
pub fn cycler(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    value: &Text,
    (hover, live): (bool, bool),
) {
    let [x, y, w, h] = rect;
    well(batch, atlas, rect);
    if hover && live {
        ring(batch, atlas, rect, WHITE);
    }
    let ty = y + ((h - 8.0) / 2.0).floor();
    let arrows = if hover && live { WHITE } else { MUTED };
    batch.text_styled(atlas, &Text::literal("<"), x + 3.0, ty, arrows, true);
    batch.text_styled(atlas, &Text::literal(">"), x + w - 8.0, ty, arrows, true);
    let color = match (live, hover) {
        (false, _) => MUTED,
        (true, true) => WHITE,
        (true, false) => SOFT,
    };
    batch.text_styled(atlas, value, x + 9.0, ty, color, true);
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
