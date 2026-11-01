//! The look of the graphics settings screen, after Sodium 0.7's video settings
//! (`net.caffeinemc.mods.sodium.client.gui`): its sizes (`Layout`), colours
//! (`Colors`) and flat widgets. `graphics_ui` lays the screen out and handles
//! its input.
use crate::hud::{HudAtlas, HudBatch};

/// `Layout`, in GUI pixels.
pub const BUTTON_SHORT: f32 = 20.0;
pub const BUTTON_LONG: f32 = 65.0;
pub const INNER_MARGIN: f32 = 5.0;
pub const OPTION_GROUP_MARGIN: f32 = 3.0;
pub const OPTION_PAGE_MARGIN: f32 = 6.0;
pub const OPTION_LEFT_INSET: f32 = OPTION_GROUP_MARGIN;
pub const SCROLLBAR_WIDTH: f32 = 7.0;
pub const TEXT_LEFT_PADDING: f32 = 8.0;
pub const TEXT_LINE_SPACING: f32 = 2.0;
pub const PAGE_LIST_WIDTH: f32 = 125.0;
pub const OPTION_WIDTH: f32 = 210.0;
pub const OPTION_LIST_SCROLLBAR_OFFSET: f32 = 5.0;
pub const OPTION_TEXT_SIDE_PADDING: f32 = 6.0;
pub const OPTION_LABEL_END_PADDING: f32 = 20.0;
pub const TICKBOX_CONTROL_WIDTH: f32 = 30.0;
pub const CYCLING_CONTROL_WIDTH: f32 = 70.0;
pub const SLIDER_WIDTH: f32 = 90.0;
pub const SLIDER_HEIGHT: f32 = 10.0;
pub const CONTROL_ICON_SIZE: f32 = 10.0;
pub const MIN_TOOLTIP_WIDTH: f32 = 100.0;
pub const MAX_TOOLTIP_WIDTH: f32 = 200.0;
pub const TOOLTIP_OUTER_MARGIN: f32 = 3.0;
pub const CONTENT_BORDER_MIN_WIDTH: f32 = 100.0;
pub const CONTENT_BORDER_HEIGHT: f32 = 12.0;
pub const CONTENT_MIN_HEIGHT: f32 = 300.0;
pub const PAGE_ENTRY_SELECTION_BAR_WIDTH: f32 = 3.0;
pub const SECTION_FOCUS_LEAD_ROWS: f32 = 3.0;
/// `Layout.entryHeight`: two lines of text.
pub const ENTRY_HEIGHT: f32 = 18.0;
/// `Layout.pageHeaderHeight`: three lines of text.
pub const PAGE_HEADER_HEIGHT: f32 = 27.0;
/// `REGULAR_TEXT_BASELINE_OFFSET`: text sits this far above a row's centre.
pub const BASELINE: f32 = -4.0;

/// An ARGB colour as Sodium writes it, in `HudBatch`'s order.
const fn argb(c: u32) -> u32 {
    c & 0xFF00_FF00 | (c >> 16) & 0xFF | (c & 0xFF) << 16
}
/// `Colors`.
pub const THEME: u32 = argb(0xFF94_E4D3);
pub const THEME_LIGHTER: u32 = argb(0xFFCC_FDEE);
pub const THEME_DARKER: u32 = argb(0xFF7A_9E9E);
pub const FOREGROUND: u32 = argb(0xFFFF_FFFF);
pub const FOREGROUND_DISABLED: u32 = argb(0xFFAA_AAAA);
pub const BACKGROUND_LIGHT: u32 = argb(0x4000_0000);
pub const BACKGROUND_MEDIUM: u32 = argb(0x6000_0000);
pub const BACKGROUND_HOVER: u32 = argb(0xE000_0000);
pub const BACKGROUND_OVERLAY: u32 = argb(0xEA00_0000);
pub const BACKGROUND_DEFAULT: u32 = argb(0x9000_0000);
pub const BACKGROUND_DARKER: u32 = argb(0xB000_0000);
pub const BACKGROUND_HIGHLIGHT: u32 = argb(0x08FF_FFFF);
pub const BUTTON_BORDER: u32 = argb(0x8000_FFEE);
/// `ScrollbarWidget.COLOR` and `HIGHLIGHT_COLOR`.
pub const SCROLLBAR: u32 = argb(0x9632_3232);
pub const SCROLLBAR_THUMB: u32 = argb(0x9664_6464);

/// `AbstractWidget.drawRect`: corners, not a size.
pub fn rect(batch: &mut HudBatch, atlas: &HudAtlas, [x1, y1, x2, y2]: [f32; 4], color: u32) {
    batch.fill(atlas, x1, y1, x2 - x1, y2 - y1, color);
}
/// `GuiGraphics.renderOutline`, one pixel inside the corners.
pub fn border(batch: &mut HudBatch, atlas: &HudAtlas, [x1, y1, x2, y2]: [f32; 4], color: u32) {
    rect(batch, atlas, [x1, y1, x2, y1 + 1.0], color);
    rect(batch, atlas, [x1, y2 - 1.0, x2, y2], color);
    rect(batch, atlas, [x1, y1 + 1.0, x1 + 1.0, y2 - 1.0], color);
    rect(batch, atlas, [x2 - 1.0, y1 + 1.0, x2, y2 - 1.0], color);
}
/// `GuiGraphics.text`, with its shadow; `§` codes style it.
pub fn text(batch: &mut HudBatch, atlas: &HudAtlas, s: &str, x: f32, y: f32, color: u32) -> f32 {
    batch.text(atlas, s, x.floor(), y.floor(), color, true)
}
pub fn width(atlas: &HudAtlas, s: &str) -> f32 {
    atlas.font.width(s)
}
/// `AbstractWidget.truncateTextToFit`: cut with "..." to fit `max`.
pub fn truncate(atlas: &HudAtlas, s: &str, max: f32) -> String {
    if width(atlas, s) <= max {
        return s.to_owned();
    }
    let room = (max - width(atlas, "...")).max(0.0);
    let mut out = String::new();
    for c in s.chars() {
        out.push(c);
        if width(atlas, &out) > room {
            out.pop();
            break;
        }
    }
    out.trim_end().to_owned() + "..."
}

/// `FlatButtonWidget` with its background and no frame (Done, Undo).
pub fn flat_button(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    r: [f32; 4],
    label: &str,
    hovered: bool,
    focused: bool,
) {
    rect(
        batch,
        atlas,
        r,
        if hovered {
            BACKGROUND_HOVER
        } else {
            BACKGROUND_DEFAULT
        },
    );
    let x = ((r[0] + r[2]) / 2.0).floor() - (width(atlas, label) / 2.0).floor();
    let y = ((r[1] + r[3]) / 2.0).floor() - 4.0;
    text(batch, atlas, label, x, y, FOREGROUND);
    if focused {
        border(batch, atlas, r, BUTTON_BORDER);
    }
}

/// `CenteredFlatWidget`: an entry of the page list. A mod header has a
/// subtitle and can't be selected.
pub fn page_entry(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    r: [f32; 4],
    label: &str,
    subtitle: Option<&str>,
    hovered: bool,
    selected: bool,
) {
    let selectable = subtitle.is_none();
    let background = if hovered {
        BACKGROUND_HIGHLIGHT
    } else if selected {
        BACKGROUND_DEFAULT
    } else {
        BACKGROUND_LIGHT
    };
    let color = if selected || !selectable {
        THEME_LIGHTER
    } else if hovered {
        THEME
    } else {
        THEME_DARKER
    };
    if selectable {
        rect(batch, atlas, r, background);
    }
    if selected {
        let bar = [r[2] - PAGE_ENTRY_SELECTION_BAR_WIDTH, r[1], r[2], r[3]];
        rect(batch, atlas, bar, THEME_LIGHTER);
    }
    let x = r[0] + TEXT_LEFT_PADDING;
    let room = r[2] - r[0] - 14.0 - TEXT_LEFT_PADDING;
    let height = r[3] - r[1];
    match subtitle {
        None => {
            let y = (r[1] + (height - 9.0) * 0.5).ceil();
            text(batch, atlas, &truncate(atlas, label, room), x, y, color);
        }
        Some(sub) => {
            let center = r[1] + height * 0.5;
            let y = (center - (9.0 + TEXT_LINE_SPACING * 0.5)).ceil();
            text(batch, atlas, &truncate(atlas, label, room), x, y, color);
            let y = (center + TEXT_LINE_SPACING * 0.5).ceil();
            text(batch, atlas, &truncate(atlas, sub, room), x, y, color);
        }
    }
}

/// `OptionListWidget.HeaderWidget`: the mod's name, a page's or a group's.
pub fn list_header(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    r: [f32; 4],
    title: &str,
    color: u32,
    background: u32,
) {
    rect(batch, atlas, r, background);
    let title = truncate(atlas, title, r[2] - r[0] - OPTION_TEXT_SIDE_PADDING * 2.0);
    let y = ((r[1] + r[3]) / 2.0).floor() + BASELINE;
    text(batch, atlas, &title, r[0] + OPTION_PAGE_MARGIN, y, color);
}

/// How an option row shows its value (`Control.createElement`).
pub enum Control<'a> {
    /// `TickBoxControl`.
    TickBox(bool),
    /// `SliderControl`: the value's text and the thumb's place, 0 to 1.
    Slider(&'a str, f32),
    /// `CyclingControl`: the value's text.
    Cycling(&'a str),
}
impl Control<'_> {
    /// `Control.getMaxWidth`: the room the name leaves it.
    fn content_width(&self, atlas: &HudAtlas, open: bool) -> f32 {
        match self {
            Control::TickBox(_) => TICKBOX_CONTROL_WIDTH,
            Control::Cycling(_) => CYCLING_CONTROL_WIDTH,
            Control::Slider(value, _) if open => SLIDER_WIDTH + width(atlas, value),
            Control::Slider(value, _) => width(atlas, value),
        }
    }
}

/// `ControlElement` and its control: the option's name on the left, its
/// value on the right. A disabled option is grey and struck through; a
/// slider shows its track while hovered or focused.
pub fn option_row(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    r: [f32; 4],
    name: &str,
    control: &Control,
    enabled: bool,
    hovered: bool,
    focused: bool,
) {
    let center = ((r[1] + r[3]) / 2.0).floor();
    rect(
        batch,
        atlas,
        r,
        if hovered {
            BACKGROUND_HOVER
        } else {
            BACKGROUND_LIGHT
        },
    );
    let open = enabled && (hovered || focused);
    let room = r[2] - r[0] - control.content_width(atlas, open) - OPTION_LABEL_END_PADDING;
    let name = truncate(atlas, name, room);
    let label = if enabled {
        format!("§f{name}")
    } else {
        format!("§7§m{name}")
    };
    text(
        batch,
        atlas,
        &label,
        r[0] + OPTION_TEXT_SIDE_PADDING,
        center + BASELINE,
        FOREGROUND,
    );
    if focused {
        border(batch, atlas, r, FOREGROUND);
    }
    let right = r[2] - OPTION_TEXT_SIDE_PADDING;
    match *control {
        Control::TickBox(ticked) => {
            let x = right - CONTROL_ICON_SIZE;
            let y = center - CONTROL_ICON_SIZE / 2.0;
            let b = [x, y, x + CONTROL_ICON_SIZE, y + CONTROL_ICON_SIZE];
            let color = match (enabled, ticked) {
                (false, _) => FOREGROUND_DISABLED,
                (true, true) => THEME,
                (true, false) => FOREGROUND,
            };
            if ticked {
                rect(
                    batch,
                    atlas,
                    [b[0] + 2.0, b[1] + 2.0, b[2] - 2.0, b[3] - 2.0],
                    color,
                );
            }
            if enabled {
                border(batch, atlas, b, color);
            } else {
                // Only the corners, three pixels each way.
                for (cx, cy, dx, dy) in [
                    (b[0], b[1], 1.0, 1.0),
                    (b[2], b[1], -1.0, 1.0),
                    (b[0], b[3], 1.0, -1.0),
                    (b[2], b[3], -1.0, -1.0),
                ] {
                    let along = [cx, cy, cx + 3.0 * dx, cy + dy];
                    let down = [cx, cy, cx + dx, cy + 3.0 * dy];
                    for [x1, y1, x2, y2] in [along, down] {
                        let r = [x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)];
                        rect(batch, atlas, r, color);
                    }
                }
            }
        }
        Control::Cycling(value) => {
            let value = if enabled {
                value.to_owned()
            } else {
                format!("§7§o{value}")
            };
            let x = right - width(atlas, &value);
            text(batch, atlas, &value, x, center + BASELINE, FOREGROUND);
        }
        Control::Slider(value, position) => {
            let value = if enabled {
                value.to_owned()
            } else {
                format!("§7§o{value}")
            };
            let label_width = width(atlas, &value);
            let sx = right - SLIDER_WIDTH;
            let sy = center - SLIDER_HEIGHT / 2.0;
            let label_y = sy + SLIDER_HEIGHT / 2.0 + BASELINE;
            if open {
                let thumb = (sx + position.clamp(0.0, 1.0) * SLIDER_WIDTH - 2.0).floor();
                let track = (sy + SLIDER_HEIGHT / 2.0 - 0.5).floor();
                rect(
                    batch,
                    atlas,
                    [sx, track, sx + SLIDER_WIDTH, track + 1.0],
                    THEME_LIGHTER,
                );
                rect(
                    batch,
                    atlas,
                    [thumb, sy, thumb + 4.0, sy + SLIDER_HEIGHT],
                    FOREGROUND,
                );
                let x = sx - label_width - OPTION_TEXT_SIDE_PADDING;
                text(batch, atlas, &value, x, label_y, FOREGROUND);
            } else {
                let x = sx + SLIDER_WIDTH - label_width;
                text(batch, atlas, &value, x, label_y, FOREGROUND);
            }
        }
    }
}

/// Where a slider's track lies in its row, for clicks and drags.
pub fn slider_track(r: [f32; 4]) -> [f32; 4] {
    let center = ((r[1] + r[3]) / 2.0).floor();
    let x = r[2] - OPTION_TEXT_SIDE_PADDING - SLIDER_WIDTH;
    let y = center - SLIDER_HEIGHT / 2.0;
    [x, y, x + SLIDER_WIDTH, y + SLIDER_HEIGHT]
}

/// `ScrollbarWidget`, vertical: the track and its thumb.
pub fn scrollbar(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    r: [f32; 4],
    visible: f32,
    total: f32,
    amount: f32,
) {
    if total <= visible {
        return;
    }
    rect(batch, atlas, r, SCROLLBAR);
    let length = r[3] - r[1];
    let start = r[1] + (amount / total * length).round();
    let thumb = (visible / total * length).round();
    rect(
        batch,
        atlas,
        [r[0], start, r[2], start + thumb],
        SCROLLBAR_THUMB,
    );
}

/// `ScrollableTooltip.render`: the box beside the hovered option, with the
/// arrow pointing at it, or over the list where there is no room (`overlay`).
pub fn tooltip(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    r: [f32; 4],
    arrow_y: Option<f32>,
    lines: &[(String, u32)],
) {
    if let Some(y) = arrow_y {
        let x = r[0];
        let corners = [[x, y], [x - 5.0, y + 4.5], [x - 5.0, y + 4.5], [x, y + 9.0]];
        batch.fill_quad(atlas, corners, BACKGROUND_DEFAULT);
    }
    let start = batch.vertices.len();
    let background = if arrow_y.is_some() {
        BACKGROUND_LIGHT
    } else {
        BACKGROUND_OVERLAY
    };
    rect(batch, atlas, r, background);
    let pad = INNER_MARGIN - 1.0;
    for (i, (line, color)) in lines.iter().enumerate() {
        let y = r[1] + pad + i as f32 * (9.0 + TEXT_LINE_SPACING);
        text(batch, atlas, line, r[0] + pad, y, *color);
    }
    batch.clip_since(start, r[0], r[1], r[2], r[3]);
}
