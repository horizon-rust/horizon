//! The touch overlay's look, in the style of Android game launchers' control menus: dark rounded
//! panels with rows, toggles and sliders, a blue accent, and a ≡ button that opens them. Only
//! drawing and hit-testing here; what the buttons do is in `game/touch_menu.rs`.
use crate::ui_theme::{Face, Ui, color::hex};
use horizon_touch::{UiItem, UiKind};

const PANEL: u32 = 0x2b2b2e;
const ROW: u32 = 0x3a3a3d;
const ACCENT: u32 = 0x4f7dff;
const INK: u32 = 0xffffff;

/// What the menu shows.
#[derive(Clone, Debug)]
pub struct MenuState {
    pub open: bool,
    pub show_fps: bool,
    pub fps: u32,
    /// The on-screen buttons' opacity (0.2..=1.0) and whether they are hidden.
    pub opacity: f32,
    pub hidden: bool,
    /// Movement is a stick, not the W/A/S/D buttons.
    pub stick: bool,
    /// Camera speed, 0.5..=3.0 times.
    pub sensitivity: f32,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            open: false,
            show_fps: true,
            fps: 0,
            opacity: 0.75,
            hidden: false,
            stick: false,
            sensitivity: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slider {
    ButtonOpacity,
    LookSensitivity,
}

/// What a row does when pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    EditLayout,
    ResetLayout,
    ToggleControls,
    ToggleStick,
    Launcher,
    CopyLog,
    PauseMenu,
    Quit,
    ToggleFps,
    Slider(Slider),
    /// A press on a panel that does nothing (keeps it from closing the menu).
    Panel,
    /// A press outside the panels.
    Backdrop,
}

enum Kind {
    Button(&'static str),
    Toggle(&'static str, bool),
    Slider(&'static str, f32, String),
}

struct Row {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    kind: Kind,
    hit: Hit,
}

struct Panel {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    title: &'static str,
    rows: Vec<Row>,
}

/// A laid-out menu.
pub struct MenuFrame {
    panels: Vec<Panel>,
    w: f32,
    h: f32,
}

const ROW_H: f32 = 52.0;
const GAP: f32 = 8.0;
const HEAD: f32 = 56.0;

fn rows_for(state: &MenuState, which: u8) -> Vec<(Kind, Hit)> {
    match which {
        0 => vec![
            (Kind::Button("Edit control layout"), Hit::EditLayout),
            (Kind::Button("Reset control layout"), Hit::ResetLayout),
            (
                Kind::Toggle("Show on-screen buttons", !state.hidden),
                Hit::ToggleControls,
            ),
            (
                Kind::Toggle("Movement stick (instead of W A S D)", state.stick),
                Hit::ToggleStick,
            ),
            (
                Kind::Slider(
                    "Button opacity",
                    ((state.opacity - 0.2) / 0.8).clamp(0.0, 1.0),
                    format!("{}%", (state.opacity * 100.0).round()),
                ),
                Hit::Slider(Slider::ButtonOpacity),
            ),
            (
                Kind::Slider(
                    "Camera sensitivity",
                    ((state.sensitivity - 0.5) / 2.5).clamp(0.0, 1.0),
                    format!("{:.1}x", state.sensitivity),
                ),
                Hit::Slider(Slider::LookSensitivity),
            ),
        ],
        _ => vec![
            (Kind::Button("Pause menu (Esc)"), Hit::PauseMenu),
            (Kind::Button("Back to the Horizon launcher"), Hit::Launcher),
            (Kind::Button("Copy log"), Hit::CopyLog),
            (Kind::Toggle("FPS display", state.show_fps), Hit::ToggleFps),
            (Kind::Button("Quit game"), Hit::Quit),
        ],
    }
}

fn row_height(kind: &Kind) -> f32 {
    match kind {
        Kind::Slider(..) => ROW_H + 18.0,
        _ => ROW_H,
    }
}

fn panel(x: f32, y: f32, w: f32, title: &'static str, rows: Vec<(Kind, Hit)>) -> Panel {
    let mut cursor = y + HEAD;
    let mut out = Vec::new();
    for (kind, hit) in rows {
        let h = row_height(&kind);
        out.push(Row {
            x: x + 12.0,
            y: cursor,
            w: w - 24.0,
            h,
            kind,
            hit,
        });
        cursor += h + GAP;
    }
    Panel {
        x,
        y,
        w,
        h: cursor - y + 4.0,
        title,
        rows: out,
    }
}

/// Lays the open menu out for a window `ui.w` by `ui.h` dp: two panels side by side, or one
/// stack on a narrow screen.
pub fn layout_menu(ui: &Ui, state: &MenuState) -> MenuFrame {
    let mut panels = Vec::new();
    if ui.w >= 760.0 {
        let pw = (ui.w * 0.44).min(440.0);
        let top = 16.0;
        panels.push(panel(16.0, top, pw, "Controls", rows_for(state, 0)));
        panels.push(panel(
            ui.w - 16.0 - pw,
            top,
            pw,
            "Game Menu",
            rows_for(state, 1),
        ));
    } else {
        let pw = (ui.w - 32.0).min(440.0);
        let mut rows = rows_for(state, 0);
        rows.extend(rows_for(state, 1));
        panels.push(panel((ui.w - pw) / 2.0, 12.0, pw, "Menu", rows));
    }
    MenuFrame {
        panels,
        w: ui.w,
        h: ui.h,
    }
}

/// The row under `(x, y)` dp, or the panel, or the backdrop.
pub fn hit(frame: &MenuFrame, x: f32, y: f32) -> (Hit, Option<(f32, f32, f32)>) {
    for p in &frame.panels {
        for r in &p.rows {
            if x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h {
                // A slider also reports its track: left, width and the value there.
                let slider = matches!(r.kind, Kind::Slider(..)).then_some((
                    r.x + 16.0,
                    r.w - 32.0,
                    ((x - (r.x + 16.0)) / (r.w - 32.0)).clamp(0.0, 1.0),
                ));
                return (r.hit, slider);
            }
        }
        if x >= p.x && x < p.x + p.w && y >= p.y && y < p.y + p.h {
            return (Hit::Panel, None);
        }
    }
    (Hit::Backdrop, None)
}

/// A slider's value for a finger at `x` dp, given the track reported by [`hit`] when it was pressed.
pub fn slider_value(track: (f32, f32), x: f32) -> f32 {
    ((x - track.0) / track.1).clamp(0.0, 1.0)
}

fn text_centered(
    ui: &mut Ui,
    face: Face,
    size: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
    s: &str,
) {
    let tw = ui.measure(face, size, s);
    ui.text_line(face, size, h, x + ((w - tw) / 2.0).round(), y, color, s);
}

pub fn draw_menu(ui: &mut Ui, frame: &MenuFrame) {
    ui.rounded(0.0, 0.0, frame.w, frame.h, 0.0, hex(0x000000, 150));
    for p in &frame.panels {
        ui.rounded(p.x, p.y, p.w, p.h, 18.0, hex(PANEL, 245));
        text_centered(
            ui,
            Face::Medium,
            17.0,
            p.x,
            p.y + 4.0,
            p.w,
            HEAD - 8.0,
            hex(INK, 255),
            p.title,
        );
        for r in &p.rows {
            ui.rounded(r.x, r.y, r.w, r.h, 12.0, hex(ROW, 255));
            match &r.kind {
                Kind::Button(label) => {
                    ui.text_line(
                        Face::Medium,
                        15.0,
                        r.h,
                        r.x + 16.0,
                        r.y,
                        hex(INK, 255),
                        label,
                    );
                }
                Kind::Toggle(label, on) => {
                    ui.text_line(
                        Face::Medium,
                        15.0,
                        r.h,
                        r.x + 16.0,
                        r.y,
                        hex(INK, 255),
                        label,
                    );
                    let (tw, th) = (52.0, 30.0);
                    let (tx, ty) = (r.x + r.w - tw - 14.0, r.y + (r.h - th) / 2.0);
                    ui.rounded(
                        tx,
                        ty,
                        tw,
                        th,
                        th / 2.0,
                        hex(if *on { ACCENT } else { 0x55555a }, 255),
                    );
                    let knob = th - 8.0;
                    let kx = if *on { tx + tw - knob - 4.0 } else { tx + 4.0 };
                    ui.rounded(kx, ty + 4.0, knob, knob, knob / 2.0, hex(INK, 255));
                }
                Kind::Slider(label, value, shown) => {
                    ui.text_line(
                        Face::Medium,
                        14.0,
                        28.0,
                        r.x + 16.0,
                        r.y + 4.0,
                        hex(INK, 255),
                        label,
                    );
                    let sw = ui.measure(Face::Medium, 14.0, shown);
                    ui.text_line(
                        Face::Medium,
                        14.0,
                        28.0,
                        r.x + r.w - 16.0 - sw,
                        r.y + 4.0,
                        hex(INK, 255),
                        shown,
                    );
                    let (tx, ty, tw) = (r.x + 16.0, r.y + r.h - 24.0, r.w - 32.0);
                    ui.rounded(tx, ty, tw, 10.0, 5.0, hex(0x5a5a60, 255));
                    ui.rounded(tx, ty, (tw * value).max(10.0), 10.0, 5.0, hex(ACCENT, 255));
                    ui.rounded(
                        tx + tw * value - 3.0,
                        ty - 5.0,
                        6.0,
                        20.0,
                        3.0,
                        hex(INK, 255),
                    );
                }
            }
        }
    }
}

/// The on-screen buttons, the edit bar and the menu button, from the touch crate's items (pixels).
pub fn draw_controls(
    ui: &mut Ui,
    items: &[UiItem],
    opacity: f32,
    fps: Option<u32>,
    editing: bool,
    joystick: Option<([f32; 2], [f32; 2], f32)>,
) {
    let d = ui.density;
    // The floating joystick: a ring where the finger landed and a knob that follows it.
    if let Some((origin, now, radius)) = joystick {
        let (ox, oy, r) = (origin[0] / d, origin[1] / d, radius / d);
        ui.rounded(ox - r, oy - r, r * 2.0, r * 2.0, r, hex(0xffffff, 40));
        ui.border(ox - r, oy - r, r * 2.0, r * 2.0, r, hex(0xffffff, 110));
        let (dx, dy) = ((now[0] - origin[0]) / d, (now[1] - origin[1]) / d);
        let len = (dx * dx + dy * dy).sqrt().max(0.001);
        let k = if len > r { r / len } else { 1.0 };
        let (kx, ky) = (ox + dx * k, oy + dy * k);
        let kr = r * 0.38;
        ui.rounded(kx - kr, ky - kr, kr * 2.0, kr * 2.0, kr, hex(0xffffff, 150));
    }
    let alpha = |a: f32| (a * 255.0).round() as u8;
    for item in items {
        let (x, y, w, h) = (
            item.rect.x / d,
            item.rect.y / d,
            item.rect.w / d,
            item.rect.h / d,
        );
        match item.kind {
            UiKind::Stick => {
                // A fixed stick: a ring, and a knob where the finger is.
                let (cx, cy, r) = (x + w / 2.0, y + h / 2.0, w / 2.0);
                ui.rounded(x, y, w, h, r, hex(0xffffff, alpha(0.10 + 0.2 * opacity)));
                ui.border(x, y, w, h, r, hex(0xffffff, alpha(0.3 + 0.4 * opacity)));
                let knob = item.knob.map_or((cx, cy), |k| {
                    let (kx, ky) = (k[0] / d - cx, k[1] / d - cy);
                    let len = (kx * kx + ky * ky).sqrt().max(0.001);
                    let m = if len > r { r / len } else { 1.0 };
                    (cx + kx * m, cy + ky * m)
                });
                let kr = r * 0.36;
                ui.rounded(
                    knob.0 - kr,
                    knob.1 - kr,
                    kr * 2.0,
                    kr * 2.0,
                    kr,
                    hex(0xffffff, alpha(0.35 + 0.45 * opacity)),
                );
            }
            UiKind::Handle => {
                // The corner handle: a blue dot with a white ring.
                ui.rounded(
                    x + 8.0,
                    y + 8.0,
                    w - 16.0,
                    h - 16.0,
                    (w - 16.0) / 2.0,
                    hex(ACCENT, 255),
                );
                ui.border(
                    x + 8.0,
                    y + 8.0,
                    w - 16.0,
                    h - 16.0,
                    (w - 16.0) / 2.0,
                    hex(INK, 255),
                );
            }
            UiKind::Keys => {
                ui.rounded(x, y, w, h, h / 2.0, hex(PANEL, 215));
                text_centered(ui, Face::Medium, 14.0, x, y, w, h, hex(INK, 255), "Keys");
            }
            UiKind::Back => {
                ui.rounded(x, y, w, h, h / 2.0, hex(PANEL, 215));
                // A chevron pointing left, then the word.
                let (ax, ay) = (x + 18.0, y + h / 2.0);
                for i in 0..9 {
                    let dy = i as f32 - 4.0;
                    ui.rounded(
                        ax + dy.abs() * 1.2,
                        ay + dy * 1.2,
                        2.2,
                        1.4,
                        0.0,
                        hex(INK, 255),
                    );
                }
                ui.text_line(Face::Medium, 14.0, h, x + 40.0, y, hex(INK, 255), "Back");
            }
            UiKind::Button => {
                let selected = editing && item.active;
                let (fill, a) = if item.active {
                    (ACCENT, 0.55 + 0.45 * opacity)
                } else {
                    (0xffffff, 0.14 + 0.26 * opacity)
                };
                ui.rounded(x, y, w, h, 14.0, hex(fill, alpha(a)));
                ui.border(
                    x,
                    y,
                    w,
                    h,
                    14.0,
                    hex(
                        if selected { ACCENT } else { 0xffffff },
                        alpha(if selected { 1.0 } else { 0.25 + 0.4 * opacity }),
                    ),
                );
                let label = if item.toggle {
                    format!("{} •", item.label)
                } else {
                    item.label.clone()
                };
                text_centered(
                    ui,
                    Face::Medium,
                    14.0,
                    x,
                    y,
                    w,
                    h,
                    hex(INK, alpha(0.55 + 0.45 * opacity)),
                    &label,
                );
            }
            UiKind::Panel => {
                ui.rounded(x, y, w, h, 16.0, hex(PANEL, 245));
                ui.text_line(
                    Face::Medium,
                    13.0,
                    34.0,
                    x + 14.0,
                    y,
                    hex(INK, 255),
                    &item.label,
                );
                let hint = "drag to move · corner to resize";
                let hw = ui.measure(Face::Medium, 11.0, hint);
                ui.text_line(
                    Face::Medium,
                    11.0,
                    34.0,
                    x + w - hw - 14.0,
                    y,
                    hex(0xb4b4bc, 255),
                    hint,
                );
            }
            UiKind::Tool => {
                let enabled = item.active;
                let primary = item.label == "Done";
                let fill = if primary { ACCENT } else { ROW };
                ui.rounded(x, y, w, h, 10.0, hex(fill, if enabled { 255 } else { 120 }));
                text_centered(
                    ui,
                    Face::Medium,
                    12.5,
                    x,
                    y,
                    w,
                    h,
                    hex(INK, if enabled { 255 } else { 110 }),
                    &item.label,
                );
            }
            UiKind::Gear => {
                ui.rounded(x, y, w, h, h / 2.0, hex(PANEL, 215));
                // The three bars of a menu button, drawn (the font has no ≡).
                for i in 0..3 {
                    ui.rounded(
                        x + 16.0,
                        y + h / 2.0 - 7.0 + i as f32 * 6.0,
                        16.0,
                        2.0,
                        1.0,
                        hex(INK, 255),
                    );
                }
                let label = match fps {
                    Some(n) => format!("FPS: {n}"),
                    None => "Menu".to_string(),
                };
                ui.text_line(Face::Medium, 14.0, h, x + 42.0, y, hex(INK, 255), &label);
            }
        }
    }
}

/// A mouse arrow at `(x, y)` dp: white with a dark outline, 12 dp tall.
pub fn draw_cursor(ui: &mut Ui, x: f32, y: f32) {
    const ROWS: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 5, 5, 3, 3];
    for (i, w) in ROWS.iter().enumerate() {
        let (ry, rw) = (y + i as f32, *w as f32);
        ui.rounded(x - 1.0, ry, rw + 2.0, 1.0, 0.0, hex(0x000000, 255));
    }
    ui.rounded(x - 1.0, y - 1.0, 3.0, 1.0, 0.0, hex(0x000000, 255));
    ui.rounded(x, y + 12.0, 4.0, 1.0, 0.0, hex(0x000000, 255));
    for (i, w) in ROWS.iter().enumerate() {
        ui.rounded(x, y + i as f32, *w as f32, 1.0, 0.0, hex(INK, 255));
    }
}

/// Which group of the picker a key is in.
fn group_of(name: &str) -> usize {
    if name.starts_with("Key") {
        0
    } else if name.starts_with("Digit") {
        1
    } else if name.len() <= 3
        && name.starts_with('F')
        && name[1..].chars().all(|c| c.is_ascii_digit())
    {
        2
    } else if name.starts_with("Mouse") {
        5
    } else if matches!(
        name,
        "Escape"
            | "Tab"
            | "Enter"
            | "Space"
            | "ShiftLeft"
            | "ControlLeft"
            | "AltLeft"
            | "Backspace"
            | "ArrowUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
    ) {
        3
    } else {
        4
    }
}

/// The picker's tabs.
pub const PICKER_TABS: [&str; 6] = ["Letters", "Numbers", "F keys", "Keys", "Symbols", "Mouse"];

/// The key picker: a compact sheet with a tab for each kind of key and the keys of the open tab as
/// small chips.
pub struct PickerFrame {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    tabs: Vec<(f32, f32, f32, f32, usize)>,
    chips: Vec<(f32, f32, f32, f32, u8)>,
    screen: (f32, f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerHit {
    Key(u8),
    Tab(usize),
    /// A press on the panel between chips.
    Panel,
    /// A press outside it: close.
    Close,
}

pub fn layout_picker(ui: &Ui, tab: usize) -> PickerFrame {
    let w = (ui.w - 24.0).min(500.0);
    let (cw, ch, gap) = (56.0, 40.0, 6.0);
    let cols = (((w - 24.0 + gap) / (cw + gap)).floor() as usize).max(3);
    let keys: Vec<u8> = (0..horizon_touch::KEYS.len() as u8)
        .filter(|i| group_of(horizon_touch::KEYS[usize::from(*i)].0) == tab)
        .collect();
    let rows = keys.len().div_ceil(cols).max(1);
    // Tabs under the title, in rows of what fits.
    let mut tabs = Vec::new();
    let (mut tx, mut ty) = (0.0f32, 0.0f32);
    let tab_h = 32.0;
    let mut tab_rows = 1.0;
    let widths: Vec<f32> = PICKER_TABS
        .iter()
        .map(|t| (ui.measure(Face::Medium, 13.0, t) + 24.0).round())
        .collect();
    for (i, tw) in widths.iter().enumerate() {
        if tx > 0.0 && tx + tw > w - 24.0 {
            tx = 0.0;
            ty += tab_h + 6.0;
            tab_rows += 1.0;
        }
        tabs.push((tx, ty, *tw, tab_h, i));
        tx += tw + 6.0;
    }
    let tabs_h = tab_rows * (tab_h + 6.0);
    let head = 46.0;
    let h = head + tabs_h + rows as f32 * (ch + gap) + 12.0;
    let x = ((ui.w - w) / 2.0).round();
    let y = ((ui.h - h) / 2.0).max(8.0).round();
    let tabs = tabs
        .into_iter()
        .map(|(a, b, c, d, i)| (x + 12.0 + a, y + head + b, c, d, i))
        .collect();
    let chips = keys
        .iter()
        .enumerate()
        .map(|(n, k)| {
            let (c, r) = (n % cols, n / cols);
            (
                x + 12.0 + c as f32 * (cw + gap),
                y + head + tabs_h + r as f32 * (ch + gap),
                cw,
                ch,
                *k,
            )
        })
        .collect();
    PickerFrame {
        x,
        y,
        w,
        h,
        tabs,
        chips,
        screen: (ui.w, ui.h),
    }
}

pub fn picker_hit(frame: &PickerFrame, x: f32, y: f32) -> PickerHit {
    let inside = |r: &(f32, f32, f32, f32)| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3;
    if let Some(t) = frame.tabs.iter().find(|t| inside(&(t.0, t.1, t.2, t.3))) {
        return PickerHit::Tab(t.4);
    }
    if let Some(c) = frame.chips.iter().find(|c| inside(&(c.0, c.1, c.2, c.3))) {
        return PickerHit::Key(c.4);
    }
    if inside(&(frame.x, frame.y, frame.w, frame.h)) {
        PickerHit::Panel
    } else {
        PickerHit::Close
    }
}

pub fn draw_picker(ui: &mut Ui, frame: &PickerFrame, tab: usize, current: Option<u8>, title: &str) {
    ui.rounded(
        0.0,
        0.0,
        frame.screen.0,
        frame.screen.1,
        0.0,
        hex(0x000000, 140),
    );
    ui.rounded(frame.x, frame.y, frame.w, frame.h, 16.0, hex(PANEL, 250));
    text_centered(
        ui,
        Face::Medium,
        16.0,
        frame.x,
        frame.y + 2.0,
        frame.w,
        40.0,
        hex(INK, 255),
        title,
    );
    for (x, y, w, h, i) in &frame.tabs {
        let on = *i == tab;
        ui.rounded(
            *x,
            *y,
            *w,
            *h,
            h / 2.0,
            hex(if on { ACCENT } else { ROW }, 255),
        );
        text_centered(
            ui,
            Face::Medium,
            13.0,
            *x,
            *y,
            *w,
            *h,
            hex(INK, 255),
            PICKER_TABS[*i],
        );
    }
    for (x, y, w, h, i) in &frame.chips {
        let on = current == Some(*i);
        ui.rounded(
            *x,
            *y,
            *w,
            *h,
            10.0,
            hex(if on { ACCENT } else { ROW }, 255),
        );
        text_centered(
            ui,
            Face::Medium,
            14.0,
            *x,
            *y,
            *w,
            *h,
            hex(INK, 255),
            horizon_touch::KEYS[usize::from(*i)].1,
        );
    }
}

/// The rename dialog: a text field and Cancel / Done.
pub struct RenameFrame {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    field: (f32, f32, f32, f32),
    cancel: (f32, f32, f32, f32),
    done: (f32, f32, f32, f32),
    screen: (f32, f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameHit {
    Done,
    Cancel,
    /// The field: the system keyboard comes up.
    Field,
    Panel,
    Backdrop,
}

pub fn layout_rename(ui: &Ui) -> RenameFrame {
    let w = (ui.w - 32.0).min(420.0);
    let h = 176.0;
    let x = ((ui.w - w) / 2.0).round();
    let y = (ui.h * 0.18).round().min(ui.h - h - 8.0).max(8.0);
    let field = (x + 16.0, y + 56.0, w - 32.0, 46.0);
    let bw = (w - 32.0 - 10.0) / 2.0;
    let cancel = (x + 16.0, y + h - 54.0, bw, 40.0);
    let done = (x + 16.0 + bw + 10.0, y + h - 54.0, bw, 40.0);
    RenameFrame {
        x,
        y,
        w,
        h,
        field,
        cancel,
        done,
        screen: (ui.w, ui.h),
    }
}

pub fn rename_hit(f: &RenameFrame, x: f32, y: f32) -> RenameHit {
    let inside = |r: &(f32, f32, f32, f32)| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3;
    if inside(&f.done) {
        RenameHit::Done
    } else if inside(&f.cancel) {
        RenameHit::Cancel
    } else if inside(&f.field) {
        RenameHit::Field
    } else if inside(&(f.x, f.y, f.w, f.h)) {
        RenameHit::Panel
    } else {
        RenameHit::Backdrop
    }
}

pub fn draw_rename(ui: &mut Ui, f: &RenameFrame, text: &str, blink: bool) {
    ui.rounded(0.0, 0.0, f.screen.0, f.screen.1, 0.0, hex(0x000000, 140));
    ui.rounded(f.x, f.y, f.w, f.h, 16.0, hex(PANEL, 250));
    text_centered(
        ui,
        Face::Medium,
        16.0,
        f.x,
        f.y + 6.0,
        f.w,
        40.0,
        hex(INK, 255),
        "Name this button",
    );
    ui.rounded(
        f.field.0,
        f.field.1,
        f.field.2,
        f.field.3,
        10.0,
        hex(ROW, 255),
    );
    ui.border(
        f.field.0,
        f.field.1,
        f.field.2,
        f.field.3,
        10.0,
        hex(ACCENT, 255),
    );
    let tw = ui.measure(Face::Medium, 16.0, text);
    ui.text_line(
        Face::Medium,
        16.0,
        f.field.3,
        f.field.0 + 14.0,
        f.field.1,
        hex(INK, 255),
        text,
    );
    if blink {
        ui.rounded(
            f.field.0 + 16.0 + tw,
            f.field.1 + 11.0,
            2.0,
            f.field.3 - 22.0,
            1.0,
            hex(INK, 255),
        );
    }
    ui.rounded(
        f.cancel.0,
        f.cancel.1,
        f.cancel.2,
        f.cancel.3,
        12.0,
        hex(ROW, 255),
    );
    text_centered(
        ui,
        Face::Medium,
        15.0,
        f.cancel.0,
        f.cancel.1,
        f.cancel.2,
        f.cancel.3,
        hex(INK, 255),
        "Cancel",
    );
    ui.rounded(
        f.done.0,
        f.done.1,
        f.done.2,
        f.done.3,
        12.0,
        hex(ACCENT, 255),
    );
    text_centered(
        ui,
        Face::Medium,
        15.0,
        f.done.0,
        f.done.1,
        f.done.2,
        f.done.3,
        hex(INK, 255),
        "Done",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::{HudAtlas, HudBatch};

    fn with_ui<R>(size: [u32; 2], f: impl FnOnce(&mut Ui) -> R) -> R {
        let atlas =
            HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap();
        let mut batch = HudBatch::new(size);
        let mut ui = Ui::new(&atlas, &mut batch, (0.0, 0.0));
        f(&mut ui)
    }

    #[test]
    fn every_row_is_hit_where_it_is_drawn_and_panels_fit() {
        for size in [[2000, 1250], [1280, 720], [412, 915]] {
            with_ui(size, |ui| {
                let frame = layout_menu(ui, &MenuState::default());
                for p in &frame.panels {
                    assert!(
                        p.x >= 0.0 && p.x + p.w <= ui.w + 0.5,
                        "{size:?} panel width"
                    );
                    for r in &p.rows {
                        let (got, slider) = hit(&frame, r.x + r.w / 2.0, r.y + r.h / 2.0);
                        assert_eq!(got, r.hit);
                        assert_eq!(slider.is_some(), matches!(r.hit, Hit::Slider(_)));
                    }
                }
                assert_eq!(hit(&frame, ui.w / 2.0, ui.h - 1.0).0, Hit::Backdrop);
            });
        }
    }

    #[test]
    fn slider_value_is_clamped_to_the_track() {
        assert_eq!(slider_value((100.0, 200.0), 50.0), 0.0);
        assert_eq!(slider_value((100.0, 200.0), 200.0), 0.5);
        assert_eq!(slider_value((100.0, 200.0), 900.0), 1.0);
    }

    #[test]
    fn the_menu_draws() {
        with_ui([1280, 720], |ui| {
            let frame = layout_menu(
                ui,
                &MenuState {
                    open: true,
                    ..MenuState::default()
                },
            );
            draw_menu(ui, &frame);
            draw_cursor(ui, 10.0, 10.0);
        });
    }

    #[test]
    fn every_key_chip_is_hit_where_it_is_drawn_and_fits_in_every_tab() {
        for size in [[2000, 1250], [1280, 720], [412, 915]] {
            with_ui(size, |ui| {
                let mut total = 0;
                for tab in 0..PICKER_TABS.len() {
                    let frame = layout_picker(ui, tab);
                    total += frame.chips.len();
                    for (x, y, w, h, i) in &frame.chips {
                        assert!(
                            *x >= 0.0 && x + w <= ui.w + 0.5,
                            "{size:?} tab {tab} chip {i}"
                        );
                        assert_eq!(
                            picker_hit(&frame, x + w / 2.0, y + h / 2.0),
                            PickerHit::Key(*i)
                        );
                    }
                    for (x, y, w, h, t) in &frame.tabs {
                        assert!(x + w <= frame.x + frame.w + 0.5);
                        assert_eq!(
                            picker_hit(&frame, x + w / 2.0, y + h / 2.0),
                            PickerHit::Tab(*t)
                        );
                    }
                    assert_eq!(picker_hit(&frame, 1.0, 1.0), PickerHit::Close);
                    assert!(
                        frame.y >= 0.0 && frame.y + frame.h <= ui.h + 0.5,
                        "{size:?} tab {tab} fits the screen"
                    );
                }
                // Every key is in exactly one tab.
                assert_eq!(total, horizon_touch::KEYS.len());
            });
        }
    }

    #[test]
    fn the_rename_dialog_is_hit_where_it_is_drawn() {
        with_ui([1280, 720], |ui| {
            let f = layout_rename(ui);
            let c = |r: (f32, f32, f32, f32)| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
            assert_eq!(rename_hit(&f, c(f.done).0, c(f.done).1), RenameHit::Done);
            assert_eq!(
                rename_hit(&f, c(f.cancel).0, c(f.cancel).1),
                RenameHit::Cancel
            );
            assert_eq!(rename_hit(&f, c(f.field).0, c(f.field).1), RenameHit::Field);
            assert_eq!(rename_hit(&f, 1.0, 1.0), RenameHit::Backdrop);
            draw_rename(ui, &f, "Fire", true);
        });
    }

    #[test]
    fn the_picker_and_joystick_draw() {
        with_ui([1280, 720], |ui| {
            let frame = layout_picker(ui, 0);
            draw_picker(ui, &frame, 0, Some(3), "Bind a key");
            draw_controls(
                ui,
                &[],
                0.75,
                Some(60),
                false,
                Some(([300.0, 300.0], [340.0, 280.0], 90.0)),
            );
        });
    }
}
