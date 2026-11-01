//! The mobile build's launcher, in the desktop launcher's Night look (docs/design-system.md):
//! a banded night sky with stars, a top bar with the six pages (Play, Versions, Mods, Servers,
//! Patch notes, Settings), a display title over each page and numbered rows. The desktop
//! launcher is GPUI, which has no Android backend, so this redraws its pages with the same
//! tokens (`horizon_theme::night`) and fonts in the game's own UI toolkit.
//!
//! This file is the screen only: a [`View`] in, a drawn frame and hit targets out. The network
//! work and the window are in `game/no_resources.rs` and `game/mobile_launcher.rs`.
use crate::ui_theme::{Face, Ui, color::hex};
use horizon_bootstrap::registry::{Entry, Sort};
use horizon_theme::night::{self, NIGHT as N};

/// Where the sign-in and download are.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// No Azure app (client ID) is configured, so sign-in can't start.
    NoClientId {
        folder: String,
    },
    SignedOut {
        note: Option<String>,
    },
    /// Waiting for the player to enter the code at the verification page.
    Code {
        code: String,
        uri: String,
        status: String,
    },
    /// Signing in again or checking ownership.
    Checking,
    Ready {
        name: String,
        note: Option<String>,
    },
    Downloading {
        stage: String,
        files: (usize, usize),
        bytes: (u64, u64),
    },
    Preparing {
        line: String,
    },
    /// The game files are ready: start the game.
    Finished {
        name: String,
    },
    /// Something the player can't fix by pressing a button again.
    Blocked {
        title: String,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Play,
    Versions,
    Mods,
    Servers,
    Notes,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Play,
        Tab::Versions,
        Tab::Mods,
        Tab::Servers,
        Tab::Notes,
        Tab::Settings,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Tab::Play => "Play",
            Tab::Versions => "Versions",
            Tab::Mods => "Mods",
            Tab::Servers => "Servers",
            Tab::Notes => "Patch notes",
            Tab::Settings => "Settings",
        }
    }
}

/// What a button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Tab(Tab),
    SignIn,
    /// Save the Azure client ID on the clipboard.
    PasteClientId,
    /// Sign in as Minecraft's own app, the way bots do (the player's explicit choice).
    UseMinecraftId,
    OpenLink,
    CopyCode,
    CancelSignIn,
    Play,
    CancelDownload,
    SignOut,
    /// From a blocked page back to the signed-in one.
    Back,
    Start,
    /// Fetch the patch notes again.
    ReloadNotes,
    /// Put the launcher's log on the clipboard, for a bug report.
    CopyLog,
    /// Choose instance `i`.
    Instance(usize),
    NewInstance,
    DeleteInstance,
    DialogOk,
    DialogCancel,
    /// The Mods page's tab (0 mods, 1 native mods, 2 resource packs, 3 shader packs) and its list.
    ModSort(usize),
    ModsReload,
    /// Resource packs' source: 0 Modrinth, 1 Horizon's registry.
    ModSource(usize),
    ModSearch,
    ModEverything,
    ModMore,
    Install(usize),
    RemoveFile(usize),
}

/// The patch notes, fetched from the project's releases.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Notes {
    #[default]
    NotLoaded,
    Loading,
    Failed(String),
    Loaded(Vec<Note>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub title: String,
    pub body: String,
}

/// Everything a frame needs.
#[derive(Clone, Debug)]
pub struct View {
    pub tab: Tab,
    pub scroll: f32,
    pub phase: Phase,
    pub notes: Notes,
    /// Horizon's prepared game files exist.
    pub prepared: bool,
    /// A Java runtime for the exporters was found.
    pub java: bool,
    /// The app's folder, for the rows that name it.
    pub folder: String,
    pub client_id: bool,
    /// Ticks the stars' twinkle.
    pub time_ms: u64,
    /// The instances, the one chosen, and the dialog open over the page.
    pub instances: Vec<String>,
    pub instance: String,
    pub dialog: Option<Dialog>,
    pub mods: ModsView,
}

/// A text dialog over the page (naming a new instance, a search).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    NewInstance,
    Search,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub kind: DialogKind,
    pub title: String,
    pub text: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModsStatus {
    Loading,
    Failed(String),
    Loaded(Vec<Entry>),
}

/// The Mods page: which tab, what the registry lists, what the instance has.
#[derive(Clone, Debug, PartialEq)]
pub struct ModsView {
    pub sort: usize,
    pub status: ModsStatus,
    pub installed: Vec<String>,
    /// What is installing now (a project's id), and a line about the last result.
    pub busy: Option<String>,
    pub note: Option<String>,
    /// Resource packs: 0 Modrinth, 1 Horizon's registry; the search text; any game version; more to load.
    pub source: usize,
    pub query: String,
    pub everything: bool,
    pub more: bool,
}

impl Default for ModsView {
    fn default() -> Self {
        Self {
            sort: 0,
            status: ModsStatus::Loading,
            installed: Vec::new(),
            busy: None,
            note: None,
            source: 0,
            query: String::new(),
            everything: false,
            more: false,
        }
    }
}

impl View {
    pub fn new() -> Self {
        Self {
            tab: Tab::Play,
            scroll: 0.0,
            phase: Phase::SignedOut { note: None },
            notes: Notes::NotLoaded,
            prepared: false,
            java: false,
            folder: String::new(),
            client_id: true,
            time_ms: 0,
            instances: vec!["Default".into()],
            instance: "Default".into(),
            dialog: None,
            mods: ModsView::default(),
        }
    }
}

impl Default for View {
    fn default() -> Self {
        Self::new()
    }
}

const MINECRAFT: &str = "26.3";
const VERSION: &str = env!("CARGO_PKG_VERSION");

// ---- the recorded frame ------------------------------------------------------------------------

enum Item {
    Fill {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        c: u32,
    },
    Border {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        c: u32,
    },
    Text {
        face: Face,
        size: f32,
        line: f32,
        x: f32,
        y: f32,
        c: u32,
        s: String,
    },
}

struct Target {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    cmd: Cmd,
}

/// A laid-out frame: what to draw, what can be pressed and how tall the page is.
pub struct Frame {
    items: Vec<Item>,
    targets: Vec<Target>,
    /// The page's height below the top bar, in dp.
    pub content_height: f32,
    /// The top bar's height.
    pub header_height: f32,
    /// A dialog is open: it takes every press.
    pub modal: bool,
    pub view_height: f32,
}

impl Frame {
    /// The furthest the page can scroll.
    pub fn max_scroll(&self) -> f32 {
        (self.content_height - (self.view_height - self.header_height)).max(0.0)
    }
}

struct B<'a, 'b> {
    ui: &'a Ui<'b>,
    items: Vec<Item>,
    targets: Vec<Target>,
    /// Added to y: the page's scroll.
    dy: f32,
    /// Top of the visible page (items wholly above it are dropped).
    clip_top: f32,
}

impl B<'_, '_> {
    fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: u32) {
        let y = y + self.dy;
        if y + h >= self.clip_top && y <= self.ui.h {
            self.items.push(Item::Fill { x, y, w, h, r, c });
        }
    }
    fn border(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: u32) {
        let y = y + self.dy;
        if y + h >= self.clip_top && y <= self.ui.h {
            self.items.push(Item::Border { x, y, w, h, r, c });
        }
    }
    fn text(&mut self, face: Face, size: f32, line: f32, x: f32, y: f32, c: u32, s: &str) -> f32 {
        let w = self.ui.measure(face, size, s);
        let y = y + self.dy;
        if y + line >= self.clip_top && y <= self.ui.h {
            self.items.push(Item::Text {
                face,
                size,
                line,
                x,
                y,
                c,
                s: s.to_owned(),
            });
        }
        w
    }
    fn wrap(&self, face: Face, size: f32, text: &str, width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for word in paragraph.split(' ') {
                let candidate = if line.is_empty() {
                    word.to_owned()
                } else {
                    format!("{line} {word}")
                };
                if !line.is_empty() && self.ui.measure(face, size, &candidate) > width {
                    lines.push(std::mem::replace(&mut line, word.to_owned()));
                } else {
                    line = candidate;
                }
            }
            lines.push(line);
        }
        lines
    }
    /// Wrapped text in a column; returns its height.
    fn paragraph(
        &mut self,
        face: Face,
        size: f32,
        line: f32,
        x: f32,
        y: f32,
        width: f32,
        c: u32,
        text: &str,
    ) -> f32 {
        let lines = self.wrap(face, size, text, width);
        for (i, l) in lines.iter().enumerate() {
            self.text(face, size, line, x, y + i as f32 * line, c, l);
        }
        lines.len() as f32 * line
    }
    fn target(&mut self, x: f32, y: f32, w: f32, h: f32, cmd: Cmd) {
        self.targets.push(Target {
            x,
            y: y + self.dy,
            w,
            h,
            cmd,
        });
    }
    /// A plate over the sky: a card with its 1 px stroke.
    fn plate(&mut self, x: f32, y: f32, w: f32, h: f32, raised: bool) {
        let (c, a) = if raised {
            (N.card_hi, N.card_hi_alpha)
        } else {
            (N.card, N.card_alpha)
        };
        self.fill(x, y, w, h, night::RADIUS_CONTROL, hex(c, (a * 255.0) as u8));
        self.border(
            x,
            y,
            w,
            h,
            night::RADIUS_CONTROL,
            hex(N.stroke, (N.stroke_alpha * 255.0) as u8),
        );
    }
    /// A card behind what was drawn since item `at` (the lower sky is too bright for text).
    fn veil(&mut self, at: usize, x: f32, y: f32, w: f32, h: f32) {
        let y = y + self.dy;
        let fill = Item::Fill {
            x,
            y,
            w,
            h,
            r: night::RADIUS_CONTROL,
            c: hex(N.card, (N.card_alpha * 255.0) as u8),
        };
        let edge = Item::Border {
            x,
            y,
            w,
            h,
            r: night::RADIUS_CONTROL,
            c: hex(N.stroke, (N.stroke_alpha * 255.0) as u8),
        };
        let at = at.min(self.items.len());
        self.items.insert(at, edge);
        self.items.insert(at, fill);
    }
    fn rule(&mut self, x: f32, y: f32, w: f32) {
        self.fill(
            x,
            y,
            w,
            1.0,
            0.0,
            hex(N.stroke, (N.rule_alpha * 255.0) as u8),
        );
    }
    /// A button; returns its width. `enabled` false draws it dim with no target.
    fn button(
        &mut self,
        x: f32,
        y: f32,
        label: &str,
        cmd: Cmd,
        primary: bool,
        enabled: bool,
    ) -> f32 {
        let h = if primary { night::PLAY } else { 36.0 };
        let size = if primary { 16.0 } else { 14.0 };
        let w = (self.ui.measure(Face::NightBold, size, label) + if primary { 56.0 } else { 32.0 })
            .round();
        let a = if enabled { 255 } else { 110 };
        let (fill, ink) = if primary {
            (hex(N.accent, a), hex(N.on_accent, a))
        } else {
            (
                hex(N.card_hi, if enabled { 230 } else { 110 }),
                hex(N.ink, a),
            )
        };
        let r = if primary {
            night::PIXEL_CORNER
        } else {
            night::RADIUS_CONTROL
        };
        self.fill(x, y, w, h, r, fill);
        if !primary {
            self.border(x, y, w, h, r, hex(N.stroke, 40));
        }
        let lw = self.ui.measure(Face::NightBold, size, label);
        self.text(
            Face::NightBold,
            size,
            h,
            x + ((w - lw) / 2.0).round(),
            y,
            ink,
            label,
        );
        if enabled {
            self.target(x, y, w, h, cmd);
        }
        w
    }
}

fn ink() -> u32 {
    hex(N.ink, 255)
}
fn ink_soft() -> u32 {
    hex(N.ink_soft, 255)
}
fn muted() -> u32 {
    hex(N.muted, 255)
}

/// 12345 as "12,345".
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn mb(bytes: u64) -> String {
    format!("{} MB", bytes >> 20)
}

/// The desktop launcher's pixel sky: banded, with twinkling stars, drifting clouds, a sun and skylines.
fn sky(b: &mut B, w: f32, h: f32, t: u64) {
    b.fill(0.0, 0.0, w, h, 0.0, hex(N.page, 255));
    for (x, y, rw, rh, c, a) in crate::mobile_sky::paint(&N, w, h, t as f32 / 1000.0) {
        if x < w && y < h {
            b.fill(x, y, rw, rh, 0.0, hex(c, a));
        }
    }
}

// ---- pages ---------------------------------------------------------------------------------------

struct Col {
    x: f32,
    w: f32,
    narrow: bool,
}

fn title(b: &mut B, col: &Col, y: f32, title: &str, subtitle: &str) -> f32 {
    let size = if col.narrow { 40.0 } else { 56.0 };
    b.text(Face::Display, size, size * 1.1, col.x, y, ink(), title);
    let mut y = y + size * 1.1 + 6.0;
    if !subtitle.is_empty() {
        y += b.paragraph(
            Face::Night,
            14.0,
            20.0,
            col.x,
            y,
            col.w,
            ink_soft(),
            subtitle,
        );
    }
    y + 24.0
}

/// A Settings row: a rule, the index, a title and its description, and room for controls on the
/// right. Returns the row's height and the right edge x for controls.
struct Row {
    y: f32,
    h: f32,
}

fn row(
    b: &mut B,
    col: &Col,
    y: f32,
    index: Option<&str>,
    name: &str,
    desc: &str,
    control_w: f32,
) -> Row {
    b.rule(col.x, y, col.w);
    let left = if index.is_some() && !col.narrow {
        56.0
    } else {
        0.0
    };
    if let Some(i) = index.filter(|_| !col.narrow) {
        b.text(Face::Mono, 12.0, 18.0, col.x, y + 22.0, muted(), i);
    }
    let text_w = (col.w - left - control_w - 16.0).max(120.0);
    b.text(Face::Night, 14.0, 20.0, col.x + left, y + 16.0, ink(), name);
    let dh = if desc.is_empty() {
        0.0
    } else {
        b.paragraph(
            Face::Night,
            12.0,
            17.0,
            col.x + left,
            y + 38.0,
            text_w,
            muted(),
            desc,
        )
    };
    Row {
        y,
        h: (38.0 + dh + 18.0).max(66.0),
    }
}

fn account_line(v: &View) -> (String, Option<&'static str>) {
    match &v.phase {
        Phase::Ready { name, .. } | Phase::Finished { name } => (format!("Signed in as {name}. This account owns Minecraft: Java Edition."), Some("Sign out")),
        Phase::Checking => ("Signing in and checking that the account owns Minecraft: Java Edition.".into(), None),
        Phase::Code { .. } => ("Waiting for you to enter the code (see Play).".into(), None),
        Phase::NoClientId { .. } => ("Sign-in needs a Microsoft app (Azure client ID). Copy yours, then press Paste client ID.".into(), Some("Paste client ID")),
        Phase::Downloading { .. } | Phase::Preparing { .. } => ("Signed in. Downloading and preparing the game.".into(), None),
        Phase::Blocked { .. } => ("Signed in.".into(), Some("Sign out")),
        Phase::SignedOut { .. } => ("Not signed in. Sign in with the Microsoft account that owns Minecraft: Java Edition.".into(), Some("Sign in")),
    }
}

/// The button on the right of the account row.
fn account_button(b: &mut B, col: &Col, y: f32, label: Option<&'static str>) {
    let Some(label) = label else {
        return;
    };
    let cmd = match label {
        "Sign out" => Cmd::SignOut,
        "Paste client ID" => Cmd::PasteClientId,
        _ => Cmd::SignIn,
    };
    let w = (b.ui.measure(Face::NightBold, 14.0, label) + 32.0).round();
    b.button(col.x + col.w - w, y + 14.0, label, cmd, false, true);
}

fn play(b: &mut B, col: &Col, v: &View, mut y: f32) -> f32 {
    let ready = v.prepared || matches!(v.phase, Phase::Finished { .. });
    // The state line: a dot and what plays.
    let dot = if ready { N.success } else { N.warn };
    b.fill(col.x, y + 6.0, 6.0, 6.0, 0.0, hex(dot, 255));
    let state = format!(
        "Minecraft: Java Edition {MINECRAFT} · Horizon {VERSION} · {}",
        if ready { "Ready" } else { "Needs preparing" }
    );
    b.text(Face::Night, 12.0, 18.0, col.x + 14.0, y, ink_soft(), &state);
    y += 30.0;
    let size = if col.narrow { 44.0 } else { 72.0 };
    b.text(Face::Display, size, size * 1.05, col.x, y, ink(), "Horizon");
    y += size * 1.05 + 8.0;
    y += b.paragraph(
        Face::Night,
        14.0,
        20.0,
        col.x,
        y,
        col.w.min(560.0),
        ink_soft(),
        "Horizon is a Rust rewrite of Minecraft: Java Edition, checked against the game itself. You need your own copy of Minecraft.",
    );
    y += 20.0;

    // The main action, by where the player is.
    let (label, cmd, enabled): (&str, Cmd, bool) = match &v.phase {
        // Prepared and a verified owner: start the game; not prepared: download it first.
        Phase::Ready { .. } | Phase::Finished { .. } if ready => ("Play", Cmd::Start, true),
        Phase::Ready { .. } => ("Play", Cmd::Play, true),
        Phase::SignedOut { .. } => ("Sign in to play", Cmd::SignIn, true),
        Phase::NoClientId { .. } => ("Paste client ID", Cmd::PasteClientId, true),
        Phase::Code { .. } => ("Signing in…", Cmd::SignIn, false),
        Phase::Checking => ("Checking…", Cmd::SignIn, false),
        Phase::Downloading { .. } => ("Cancel download", Cmd::CancelDownload, true),
        Phase::Preparing { .. } => ("Preparing…", Cmd::Play, false),
        Phase::Finished { .. } => ("Play", Cmd::Start, true),
        Phase::Blocked { .. } => ("Back", Cmd::Back, true),
    };
    // The desktop Home's action row: Client | Server on the left, the version and Play on the right.
    let play_w = (b.ui.measure(Face::NightBold, 16.0, label) + 56.0).round();
    let (seg_y, seg_h) = (y + (night::PLAY - 40.0) / 2.0, 40.0);
    let seg_w = (b.ui.measure(Face::Night, 14.0, "Client")
        + b.ui.measure(Face::Night, 14.0, "Server")
        + 64.0)
        .round();
    if col.narrow {
        b.button(col.x, y, label, cmd, true, enabled);
        y += night::PLAY + 12.0;
    } else {
        // Client | Server: the server build is the dedicated server, which a phone doesn't run.
        b.fill(
            col.x,
            seg_y,
            seg_w,
            seg_h,
            night::RADIUS_CONTROL,
            hex(N.card, 215),
        );
        b.border(
            col.x,
            seg_y,
            seg_w,
            seg_h,
            night::RADIUS_CONTROL,
            hex(N.stroke, 30),
        );
        let client_w = b.ui.measure(Face::Night, 14.0, "Client") + 32.0;
        b.fill(
            col.x + 4.0,
            seg_y + 4.0,
            client_w,
            seg_h - 8.0,
            6.0,
            hex(N.card_hi, 240),
        );
        b.text(
            Face::Night,
            14.0,
            seg_h,
            col.x + 20.0,
            seg_y,
            ink(),
            "Client",
        );
        b.text(
            Face::Night,
            14.0,
            seg_h,
            col.x + 4.0 + client_w + 16.0,
            seg_y,
            muted(),
            "Server",
        );
        // The version, and Play.
        let ver = format!("v{VERSION}");
        let ver_w = (b.ui.measure(Face::Night, 14.0, &ver) + 44.0).round();
        let right = col.x + col.w;
        b.fill(
            right - play_w - 12.0 - ver_w,
            seg_y,
            ver_w,
            seg_h,
            night::RADIUS_CONTROL,
            hex(N.card, 215),
        );
        b.border(
            right - play_w - 12.0 - ver_w,
            seg_y,
            ver_w,
            seg_h,
            night::RADIUS_CONTROL,
            hex(N.stroke, 30),
        );
        b.text(
            Face::Night,
            14.0,
            seg_h,
            right - play_w - 12.0 - ver_w + 16.0,
            seg_y,
            ink(),
            &ver,
        );
        // A pixel chevron.
        let (cx, cy) = (right - play_w - 12.0 - 24.0, seg_y + seg_h / 2.0 - 3.0);
        b.fill(cx, cy, 8.0, 2.0, 0.0, muted());
        b.fill(cx + 2.0, cy + 2.0, 4.0, 2.0, 0.0, muted());
        b.fill(cx + 3.0, cy + 4.0, 2.0, 2.0, 0.0, muted());
        b.button(right - play_w, y, label, cmd, true, enabled);
        y += night::PLAY + 24.0;
    }

    // Instances: a row of chips, the chosen one lit, and New.
    {
        b.text(
            Face::Night,
            12.0,
            18.0,
            col.x,
            y,
            muted(),
            &format!("INSTANCE  ·  saves, options, mods and packs live in the chosen one"),
        );
        y += 24.0;
        let mut x = col.x;
        for (i, name) in v.instances.iter().enumerate() {
            let w = (b.ui.measure(Face::Night, 14.0, name) + 32.0).round();
            if x > col.x && x + w > col.x + col.w {
                x = col.x;
                y += 44.0;
            }
            let on = *name == v.instance;
            b.fill(
                x,
                y,
                w,
                36.0,
                18.0,
                if on {
                    hex(N.accent, 255)
                } else {
                    hex(N.card_hi, 215)
                },
            );
            b.text(
                Face::Night,
                14.0,
                36.0,
                x + 16.0,
                y,
                if on { hex(N.on_accent, 255) } else { ink() },
                name,
            );
            b.target(x, y, w, 36.0, Cmd::Instance(i));
            x += w + 8.0;
        }
        let nw = (b.ui.measure(Face::NightBold, 14.0, "+ New instance") + 32.0).round();
        if x > col.x && x + nw > col.x + col.w {
            x = col.x;
            y += 44.0;
        }
        b.button(x, y, "+ New instance", Cmd::NewInstance, false, true);
        if v.instance != "Default" {
            let dw = (b.ui.measure(Face::NightBold, 14.0, "Delete") + 32.0).round();
            b.button(x + nw + 8.0, y, "Delete", Cmd::DeleteInstance, false, true);
            let _ = dw;
        }
        y += 56.0;
    }

    // Notes and errors.
    let note = match &v.phase {
        Phase::SignedOut { note } | Phase::Ready { note, .. } => note.clone(),
        Phase::Blocked { title, message } => Some(format!("{title}. {message}")),
        _ => None,
    };
    if let Some(note) = note {
        let h = b.paragraph(
            Face::Night,
            13.0,
            19.0,
            col.x,
            y,
            col.w,
            hex(N.warn, 255),
            &note,
        );
        y += h + 8.0;
        b.button(col.x, y, "Copy log", Cmd::CopyLog, false, true);
        y += 52.0;
    }

    // No Microsoft app yet: what to do.
    if let Phase::NoClientId { folder } = &v.phase {
        let steps = "To sign in, Horizon needs an app ID. Two ways:\n1. Your own: at portal.azure.com make an App registration (personal Microsoft accounts, public client flows), copy its Application (client) ID and press Paste client ID. Mojang must approve that app for Minecraft before the last step works.\n2. Minecraft's own app ID, the way the EDU-DONUT bot signs in. Microsoft's page will say it is Minecraft asking, Horizon is not approved by Mojang, and anything that follows is at your own risk. Press the button below only if you accept that.";
        let lines = b.wrap(Face::Night, 13.0, steps, col.w - 40.0);
        let extra = folder.split_once(". ").map(|(_, e)| e.to_owned());
        let h = 36.0 + lines.len() as f32 * 19.0 + if extra.is_some() { 28.0 } else { 0.0 } + 56.0;
        b.plate(col.x, y, col.w, h, false);
        for (i, l) in lines.iter().enumerate() {
            b.text(
                Face::Night,
                13.0,
                19.0,
                col.x + 20.0,
                y + 18.0 + i as f32 * 19.0,
                ink_soft(),
                l,
            );
        }
        let mut by = y + 22.0 + lines.len() as f32 * 19.0;
        if let Some(e) = extra {
            b.text(
                Face::Night,
                13.0,
                19.0,
                col.x + 20.0,
                by,
                hex(N.warn, 255),
                &e,
            );
            by += 28.0;
        }
        b.button(
            col.x + 20.0,
            by + 4.0,
            "Use Minecraft's own app ID",
            Cmd::UseMinecraftId,
            false,
            true,
        );
        y += h + 16.0;
    }

    // The sign-in code.
    if let Phase::Code { code, uri, status } = &v.phase {
        let ph = 204.0;
        b.plate(col.x, y, col.w, ph, false);
        b.text(
            Face::Night,
            12.0,
            18.0,
            col.x + 20.0,
            y + 16.0,
            muted(),
            "SIGN IN WITH MICROSOFT",
        );
        b.paragraph(
            Face::Night,
            14.0,
            20.0,
            col.x + 20.0,
            y + 38.0,
            col.w - 40.0,
            ink_soft(),
            &format!("Open {uri} on this device or any other and enter this code:"),
        );
        b.text(Face::Mono, 36.0, 44.0, col.x + 20.0, y + 70.0, ink(), code);
        let mut bx = col.x + 20.0;
        bx += b.button(bx, y + 124.0, "Open the page", Cmd::OpenLink, false, true) + 10.0;
        bx += b.button(bx, y + 124.0, "Copy code", Cmd::CopyCode, false, true) + 10.0;
        b.button(bx, y + 124.0, "Cancel", Cmd::CancelSignIn, false, true);
        let shown = if status.is_empty() {
            "Waiting for you to enter the code…"
        } else {
            status.as_str()
        };
        b.text(
            Face::Mono,
            11.0,
            18.0,
            col.x + 20.0,
            y + 172.0,
            muted(),
            shown,
        );
        y += ph + 16.0;
    }

    // Progress.
    let progress = match &v.phase {
        Phase::Downloading {
            stage,
            files,
            bytes,
        } => {
            let f = if bytes.1 > 0 {
                bytes.0 as f32 / bytes.1 as f32
            } else if files.1 > 0 {
                files.0 as f32 / files.1 as f32
            } else {
                0.0
            };
            let caption = if files.1 > 0 {
                format!(
                    "{stage}: {} of {} files, {} of {}",
                    files.0,
                    files.1,
                    mb(bytes.0),
                    mb(bytes.1)
                )
            } else if bytes.1 > 0 {
                format!("{stage}: {} of {}", mb(bytes.0), mb(bytes.1))
            } else {
                stage.clone()
            };
            Some((f.clamp(0.0, 1.0), caption))
        }
        Phase::Preparing { line } => Some((0.0, line.clone())),
        _ => None,
    };
    if let Some((f, caption)) = progress {
        b.fill(col.x, y, col.w, 8.0, 4.0, hex(N.ink, 28));
        if f > 0.0 {
            b.fill(col.x, y, (col.w * f).max(8.0), 8.0, 4.0, hex(N.accent, 255));
        }
        b.text(Face::Night, 12.0, 18.0, col.x, y + 14.0, muted(), &caption);
        y += 48.0;
    }

    // Rows: resources and account, as on the desktop Home.
    let rows_at = b.items.len();
    let rows_y = y;
    let res = if ready {
        "Prepared. The game files are ready.".to_string()
    } else {
        "Not prepared. Horizon needs the files of your own Minecraft 26.3: Play downloads them from Mojang and prepares them.".to_string()
    };
    let r = row(b, col, y, None, "Resources", &res, 0.0);
    y += r.h;
    let (acct, _) = account_line(v);
    let r = row(b, col, y, None, "Minecraft account", &acct, 150.0);
    account_button(b, col, r.y, account_line(v).1);
    y += r.h;
    b.rule(col.x, y, col.w);
    b.veil(
        rows_at,
        col.x - 16.0,
        rows_y,
        col.w + 32.0,
        y - rows_y + 8.0,
    );
    y + 24.0
}

fn versions(b: &mut B, col: &Col, v: &View, mut y: f32) -> f32 {
    y = title(b, col, y, "Versions", "What this device has installed.");
    let rows = [
        (
            "Horizon",
            format!("{VERSION}. This build, installed. Updates come as a new app."),
            None,
        ),
        (
            "Minecraft: Java Edition",
            format!(
                "{MINECRAFT}. {}",
                if v.prepared {
                    "Game files prepared."
                } else {
                    "Game files not prepared yet: Play downloads and prepares them."
                }
            ),
            None,
        ),
        (
            "Java runtime",
            format!(
                "Java 25 is needed once, to prepare the game files. {}",
                if v.java {
                    "Found."
                } else {
                    "Not found on this device."
                }
            ),
            None::<&str>,
        ),
    ];
    let at = b.items.len();
    let y0 = y;
    for (name, desc, _) in rows {
        let r = row(b, col, y, None, name, &desc, 0.0);
        y += r.h;
    }
    b.rule(col.x, y, col.w);
    b.veil(at, col.x - 16.0, y0, col.w + 32.0, y - y0 + 8.0);
    y + 24.0
}

fn mods(b: &mut B, col: &Col, v: &View, mut y: f32) -> f32 {
    y = title(
        b,
        col,
        y,
        "Mods",
        &format!(
            "Mods, resource packs and shader packs for the instance {}.",
            v.instance
        ),
    );
    // The tabs.
    let mut x = col.x;
    for (i, sort) in Sort::ALL.iter().enumerate() {
        let name = sort.label();
        let w = (b.ui.measure(Face::Night, 14.0, name) + 32.0).round();
        let on = i == v.mods.sort;
        b.fill(
            x,
            y,
            w,
            36.0,
            18.0,
            if on {
                hex(N.card_hi, 235)
            } else {
                hex(N.card, 150)
            },
        );
        b.text(
            Face::Night,
            14.0,
            36.0,
            x + 16.0,
            y,
            if on { ink() } else { muted() },
            name,
        );
        b.target(x, y, w, 36.0, Cmd::ModSort(i));
        x += w + 8.0;
    }
    y += 52.0;
    if let Some(note) = &v.mods.note {
        let h = b.paragraph(
            Face::Night,
            13.0,
            19.0,
            col.x,
            y,
            col.w,
            hex(N.warn, 255),
            note,
        );
        y += h + 10.0;
    }

    // Resource packs: Modrinth or Horizon's registry, and for Modrinth a search.
    let sort = Sort::ALL[v.mods.sort.min(Sort::ALL.len() - 1)];
    let rp = sort == Sort::ResourcePacks;
    let modrinth = rp && v.mods.source == 0;
    if rp {
        let mut x = col.x;
        for (i, name) in ["Modrinth", "Horizon"].iter().enumerate() {
            let w = (b.ui.measure(Face::Night, 14.0, name) + 28.0).round();
            let on = i == v.mods.source;
            b.fill(
                x,
                y,
                w,
                34.0,
                17.0,
                if on {
                    hex(N.accent, 255)
                } else {
                    hex(N.card_hi, 215)
                },
            );
            b.text(
                Face::Night,
                14.0,
                34.0,
                x + 14.0,
                y,
                if on { hex(N.on_accent, 255) } else { ink() },
                name,
            );
            b.target(x, y, w, 34.0, Cmd::ModSource(i));
            x += w + 8.0;
        }
        y += 46.0;
    }
    // The search box (tap to type): Modrinth searches online, the registry's lists are filtered here.
    {
        b.fill(col.x, y, col.w, 44.0, 10.0, hex(N.card, 235));
        b.border(col.x, y, col.w, 44.0, 10.0, hex(N.stroke, 60));
        let placeholder = format!("Search {}", sort.label().to_lowercase());
        let shown = if v.mods.query.is_empty() {
            placeholder.as_str()
        } else {
            v.mods.query.as_str()
        };
        b.text(
            Face::Night,
            15.0,
            44.0,
            col.x + 16.0,
            y,
            if v.mods.query.is_empty() {
                muted()
            } else {
                ink()
            },
            shown,
        );
        b.target(col.x, y, col.w, 44.0, Cmd::ModSearch);
        y += 54.0;
    }
    if modrinth {
        let label = if v.mods.everything {
            "Showing packs for any version"
        } else {
            "Only packs marked for this game"
        };
        let w = (b.ui.measure(Face::Night, 13.0, label) + 28.0).round();
        b.fill(col.x, y, w, 30.0, 15.0, hex(N.card_hi, 215));
        b.text(Face::Night, 13.0, 30.0, col.x + 14.0, y, ink_soft(), label);
        b.target(col.x, y, w, 30.0, Cmd::ModEverything);
        y += 44.0;
    }
    if sort == Sort::NativeMods {
        let h = b.paragraph(
            Face::Night,
            13.0,
            19.0,
            col.x,
            y,
            col.w,
            ink_soft(),
            "Native mods are Rust code compiled into the game itself, which needs a computer's build tools. The mobile launcher lists them from Horizon's registry but can't install them yet.",
        );
        y += h + 14.0;
    }

    // What this instance has.
    b.text(Face::NightBold, 14.0, 24.0, col.x, y, ink(), "Installed");
    y += 30.0;
    if v.mods.installed.is_empty() {
        b.text(
            Face::Night,
            13.0,
            20.0,
            col.x,
            y,
            muted(),
            "Nothing installed in this instance yet.",
        );
        y += 34.0;
    } else {
        let at = b.items.len();
        let y0 = y;
        for (i, name) in v.mods.installed.iter().enumerate() {
            b.rule(col.x, y, col.w);
            b.text(Face::Night, 14.0, 52.0, col.x + 12.0, y, ink(), name);
            let w = (b.ui.measure(Face::NightBold, 14.0, "Remove") + 32.0).round();
            b.button(
                col.x + col.w - w - 8.0,
                y + 8.0,
                "Remove",
                Cmd::RemoveFile(i),
                false,
                true,
            );
            y += 52.0;
        }
        b.rule(col.x, y, col.w);
        b.veil(at, col.x - 8.0, y0, col.w + 16.0, y - y0 + 4.0);
        y += 24.0;
    }

    // What the registry lists.
    b.text(
        Face::NightBold,
        14.0,
        24.0,
        col.x,
        y,
        ink(),
        if modrinth {
            "Browse Modrinth"
        } else {
            "Browse Horizon's registry"
        },
    );
    y += 30.0;
    match &v.mods.status {
        ModsStatus::Loading => {
            b.text(
                Face::Night,
                13.0,
                20.0,
                col.x,
                y,
                muted(),
                "Loading the list…",
            );
            y += 34.0;
        }
        ModsStatus::Failed(e) => {
            let h = b.paragraph(
                Face::Night,
                13.0,
                19.0,
                col.x,
                y,
                col.w,
                hex(N.warn, 255),
                e,
            );
            y += h + 8.0;
            b.button(col.x, y, "Try again", Cmd::ModsReload, false, true);
            y += 52.0;
        }
        ModsStatus::Loaded(list) if list.is_empty() => {
            b.text(
                Face::Night,
                13.0,
                20.0,
                col.x,
                y,
                muted(),
                "The registry lists nothing of this kind for this game yet.",
            );
            y += 34.0;
        }
        ModsStatus::Loaded(list) => {
            let at = b.items.len();
            let y0 = y;
            for (i, e) in list.iter().enumerate() {
                b.rule(col.x, y, col.w);
                let text_w = (col.w - 150.0).max(120.0);
                b.text(
                    Face::NightBold,
                    14.0,
                    22.0,
                    col.x + 12.0,
                    y + 8.0,
                    ink(),
                    &e.name,
                );
                let mut parts: Vec<String> = Vec::new();
                if !e.version.is_empty() {
                    parts.push(e.version.clone());
                }
                if e.size > 0 {
                    parts.push(format!("{} MB", (e.size >> 20).max(1)));
                }
                if e.downloads > 0 {
                    parts.push(format!("{} downloads", thousands(e.downloads)));
                }
                if !e.authors.is_empty() {
                    parts.push(format!("by {}", e.authors));
                }
                parts.push(e.summary.clone());
                let meta = parts.join(" · ");
                let mut line = meta.clone();
                while b.ui.measure(Face::Night, 12.0, &line) > text_w && line.len() > 4 {
                    line.pop();
                }
                if line.len() < meta.len() {
                    line.push('…');
                }
                b.text(
                    Face::Night,
                    12.0,
                    18.0,
                    col.x + 12.0,
                    y + 30.0,
                    muted(),
                    &line,
                );
                let busy = v.mods.busy.as_deref() == Some(e.id.as_str());
                let (label, enabled) = if !sort.installable_here() {
                    ("On a computer", false)
                } else if busy {
                    ("Installing…", false)
                } else if e.installed {
                    ("Installed", false)
                } else {
                    ("Install", true)
                };
                let w = (b.ui.measure(Face::NightBold, 14.0, label) + 32.0).round();
                b.button(
                    col.x + col.w - w - 8.0,
                    y + 14.0,
                    label,
                    Cmd::Install(i),
                    false,
                    enabled,
                );
                y += 62.0;
            }
            b.rule(col.x, y, col.w);
            b.veil(at, col.x - 8.0, y0, col.w + 16.0, y - y0 + 4.0);
            y += 16.0;
            if v.mods.more {
                b.button(col.x, y, "Load more", Cmd::ModMore, false, true);
                y += 52.0;
            }
            y += 8.0;
        }
    }
    y + 12.0
}

fn servers(b: &mut B, col: &Col, mut y: f32) -> f32 {
    y = title(b, col, y, "Servers", "Join a server or play with friends.");
    b.plate(col.x, y, col.w, 120.0, false);
    b.paragraph(
        Face::Night,
        14.0,
        20.0,
        col.x + 20.0,
        y + 20.0,
        col.w - 40.0,
        ink_soft(),
        "The server list is in the game's own Multiplayer screen once the game files are prepared. The launcher's server page (status, friends and invite codes) is not in the mobile launcher yet.",
    );
    y + 144.0
}

fn notes(b: &mut B, col: &Col, v: &View, mut y: f32) -> f32 {
    y = title(b, col, y, "Patch notes", "What changed in each release.");
    match &v.notes {
        Notes::NotLoaded | Notes::Loading => {
            b.text(
                Face::Night,
                14.0,
                20.0,
                col.x,
                y,
                muted(),
                "Loading the release notes…",
            );
            y += 40.0;
        }
        Notes::Failed(e) => {
            b.paragraph(
                Face::Night,
                14.0,
                20.0,
                col.x,
                y,
                col.w,
                hex(N.warn, 255),
                e,
            );
            y += 28.0;
            b.button(col.x, y + 8.0, "Try again", Cmd::ReloadNotes, false, true);
            y += 64.0;
        }
        Notes::Loaded(list) => {
            for note in list {
                let body_lines: Vec<String> = b
                    .wrap(Face::Night, 13.0, &note.body, col.w - 40.0)
                    .into_iter()
                    .take(14)
                    .collect();
                let h = 20.0 + 26.0 + body_lines.len() as f32 * 19.0 + 20.0;
                b.plate(col.x, y, col.w, h, false);
                b.text(
                    Face::NightBold,
                    16.0,
                    24.0,
                    col.x + 20.0,
                    y + 18.0,
                    ink(),
                    &note.title,
                );
                for (i, l) in body_lines.iter().enumerate() {
                    b.text(
                        Face::Night,
                        13.0,
                        19.0,
                        col.x + 20.0,
                        y + 50.0 + i as f32 * 19.0,
                        ink_soft(),
                        l,
                    );
                }
                y += h + 12.0;
            }
        }
    }
    y + 12.0
}

fn settings(b: &mut B, col: &Col, v: &View, mut y: f32) -> f32 {
    y = title(b, col, y, "Settings", "Accounts and the launcher itself.");
    let (acct, button) = account_line(v);
    let at = b.items.len();
    let y0 = y;
    let r = row(b, col, y, Some("01"), "Microsoft account", &acct, 150.0);
    account_button(b, col, r.y, button);
    y += r.h;
    let app = if v.client_id {
        "A Microsoft app is set. Sign-in shows a code to enter at microsoft.com/link."
    } else {
        "No app ID yet: paste your own Azure client ID, or choose to sign in as Minecraft's own app (see Play)."
    };
    let r = row(b, col, y, Some("02"), "Sign-in app", app, 380.0);
    let label = if v.client_id {
        "Replace client ID"
    } else {
        "Paste client ID"
    };
    let w = (b.ui.measure(Face::NightBold, 14.0, label) + 32.0).round();
    b.button(
        col.x + col.w - w,
        r.y + 14.0,
        label,
        Cmd::PasteClientId,
        false,
        true,
    );
    if !v.client_id {
        let l2 = "Use Minecraft's app ID";
        let w2 = (b.ui.measure(Face::NightBold, 14.0, l2) + 32.0).round();
        b.button(
            col.x + col.w - w - w2 - 8.0,
            r.y + 14.0,
            l2,
            Cmd::UseMinecraftId,
            false,
            true,
        );
    }
    y += r.h;
    let r = row(b, col, y, Some("03"), "Game folder", &v.folder, 0.0);
    y += r.h;
    let r = row(
        b,
        col,
        y,
        Some("04"),
        "Game files",
        if v.prepared {
            "Prepared."
        } else {
            "Not prepared yet."
        },
        0.0,
    );
    y += r.h;
    let r = row(
        b,
        col,
        y,
        Some("05"),
        "Horizon",
        &format!("Mobile launcher, Horizon {VERSION}, Minecraft {MINECRAFT}."),
        160.0,
    );
    y += r.h;
    let r = row(
        b,
        col,
        y,
        Some("06"),
        "Log",
        "Copies the launcher's log to the clipboard. Paste it into a message when something goes wrong.",
        160.0,
    );
    let w = (b.ui.measure(Face::NightBold, 14.0, "Copy log") + 32.0).round();
    b.button(
        col.x + col.w - w,
        r.y + 14.0,
        "Copy log",
        Cmd::CopyLog,
        false,
        true,
    );
    y += r.h;
    b.rule(col.x, y, col.w);
    b.veil(at, col.x - 16.0, y0, col.w + 32.0, y - y0 + 8.0);
    y + 32.0
}

/// Lays out one frame.
pub fn layout(ui: &Ui, v: &View) -> Frame {
    let narrow = ui.w < 760.0;
    let margin = if narrow { 20.0 } else { 64.0 };
    let col = Col {
        x: margin,
        w: (ui.w - margin * 2.0).min(896.0),
        narrow,
    };
    let header_h = if narrow { 100.0 } else { 72.0 };
    let mut b = B {
        ui,
        items: Vec::new(),
        targets: Vec::new(),
        dy: 0.0,
        clip_top: header_h,
    };

    sky(&mut b, ui.w, ui.h, v.time_ms);
    b.dy = -v.scroll;
    let top = header_h + 28.0;
    let end = match v.tab {
        Tab::Play => play(&mut b, &col, v, top),
        Tab::Versions => versions(&mut b, &col, v, top),
        Tab::Mods => mods(&mut b, &col, v, top),
        Tab::Servers => servers(&mut b, &col, top),
        Tab::Notes => notes(&mut b, &col, v, top),
        Tab::Settings => settings(&mut b, &col, v, top),
    };
    let content_height = end - header_h;
    b.dy = 0.0;
    b.clip_top = -1000.0;

    // The top bar, over the page as it scrolls under it.
    b.fill(0.0, 0.0, ui.w, header_h, 0.0, hex(N.page, 235));
    b.fill(0.0, header_h, ui.w, 1.0, 0.0, hex(N.stroke, 30));
    // The mark: two bars, as the desktop launcher's.
    b.fill(margin, 24.0, 4.0, 20.0, 0.0, hex(N.accent_hi, 255));
    b.fill(margin + 8.0, 24.0, 4.0, 20.0, 0.0, hex(N.accent_hi, 255));
    b.fill(margin, 32.0, 12.0, 4.0, 0.0, hex(N.accent_hi, 255));
    b.text(
        Face::NightBold,
        18.0,
        36.0,
        margin + 24.0,
        16.0,
        ink(),
        "Horizon",
    );
    let widths: Vec<f32> = Tab::ALL
        .iter()
        .map(|t| (ui.measure(Face::Night, 14.0, t.label()) + 28.0).round())
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + (Tab::ALL.len() as f32 - 1.0) * 4.0;
    let (mut x, ty) = if narrow {
        (margin, 56.0)
    } else {
        (ui.w - margin - total, 18.0)
    };
    for (t, w) in Tab::ALL.iter().zip(widths) {
        let selected = *t == v.tab;
        if selected {
            b.fill(x, ty, w, 36.0, 18.0, hex(N.card_hi, 235));
        }
        b.text(
            Face::Night,
            14.0,
            36.0,
            x + 14.0,
            ty,
            if selected { ink() } else { muted() },
            t.label(),
        );
        b.target(x, ty, w, 36.0, Cmd::Tab(*t));
        x += w + 4.0;
    }
    // A dialog over everything: it takes every press.
    if let Some(d) = &v.dialog {
        b.targets.clear();
        b.fill(0.0, 0.0, ui.w, ui.h, 0.0, hex(0x000000, 160));
        let dw = (ui.w - 32.0).min(440.0);
        let (dx, dy) = ((ui.w - dw) / 2.0, (ui.h * 0.16).max(80.0));
        let dh = 200.0 + if d.error.is_some() { 26.0 } else { 0.0 };
        b.plate(dx, dy, dw, dh, true);
        b.text(
            Face::NightBold,
            18.0,
            28.0,
            dx + 20.0,
            dy + 16.0,
            ink(),
            &d.title,
        );
        b.fill(
            dx + 20.0,
            dy + 60.0,
            dw - 40.0,
            46.0,
            10.0,
            hex(N.card, 255),
        );
        b.border(
            dx + 20.0,
            dy + 60.0,
            dw - 40.0,
            46.0,
            10.0,
            hex(N.accent_hi, 255),
        );
        let shown = if d.text.is_empty() {
            if d.kind == DialogKind::Search {
                "Type what to look for"
            } else {
                "Tap and type a name"
            }
        } else {
            d.text.as_str()
        };
        b.text(
            Face::Night,
            16.0,
            46.0,
            dx + 34.0,
            dy + 60.0,
            if d.text.is_empty() { muted() } else { ink() },
            shown,
        );
        let mut by = dy + 118.0;
        if let Some(e) = &d.error {
            b.text(Face::Night, 12.0, 20.0, dx + 20.0, by, hex(N.warn, 255), e);
            by += 26.0;
        }
        let bw = (dw - 50.0) / 2.0;
        b.fill(dx + 20.0, by + 8.0, bw, 44.0, 10.0, hex(N.card_hi, 255));
        b.text(
            Face::NightBold,
            15.0,
            44.0,
            dx + 20.0 + ((bw - ui.measure(Face::NightBold, 15.0, "Cancel")) / 2.0).round(),
            by + 8.0,
            ink(),
            "Cancel",
        );
        b.target(dx + 20.0, by + 8.0, bw, 44.0, Cmd::DialogCancel);
        b.fill(dx + 30.0 + bw, by + 8.0, bw, 44.0, 10.0, hex(N.accent, 255));
        b.text(
            Face::NightBold,
            15.0,
            44.0,
            dx + 30.0
                + bw
                + ((bw
                    - ui.measure(
                        Face::NightBold,
                        15.0,
                        if d.kind == DialogKind::Search {
                            "Search"
                        } else {
                            "Create"
                        },
                    ))
                    / 2.0)
                    .round(),
            by + 8.0,
            hex(N.on_accent, 255),
            if d.kind == DialogKind::Search {
                "Search"
            } else {
                "Create"
            },
        );
        b.target(dx + 30.0 + bw, by + 8.0, bw, 44.0, Cmd::DialogOk);
    }
    Frame {
        items: b.items,
        targets: b.targets,
        content_height,
        header_height: header_h,
        modal: v.dialog.is_some(),
        view_height: ui.h,
    }
}

/// Draws a laid-out frame.
pub fn draw(ui: &mut Ui, frame: &Frame) {
    for item in &frame.items {
        match item {
            Item::Fill { x, y, w, h, r, c } => ui.rounded(*x, *y, *w, *h, *r, *c),
            Item::Border { x, y, w, h, r, c } => ui.border(*x, *y, *w, *h, *r, *c),
            Item::Text {
                face,
                size,
                line,
                x,
                y,
                c,
                s,
            } => {
                ui.text_line(*face, *size, *line, *x, *y, *c, s);
            }
        }
    }
}

/// The command under the cursor (`ui.cursor`, in dp). The top bar takes only its tabs and the
/// page only its own buttons, so a scrolled page never answers a press on the bar.
pub fn click(ui: &Ui, frame: &Frame) -> Option<Cmd> {
    let (cx, cy) = ui.cursor;
    let in_bar = cy < frame.header_height && !frame.modal;
    frame
        .targets
        .iter()
        .rev()
        .find(|t| {
            cx >= t.x
                && cx < t.x + t.w
                && cy >= t.y
                && cy < t.y + t.h
                && in_bar == matches!(t.cmd, Cmd::Tab(_))
        })
        .map(|t| t.cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::{HudAtlas, HudBatch};

    fn with_ui<R>(size: [u32; 2], cursor: (f32, f32), f: impl FnOnce(&mut Ui) -> R) -> R {
        let atlas =
            HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap();
        let mut batch = HudBatch::new(size);
        let cursor = (cursor.0 / batch.scale, cursor.1 / batch.scale);
        let mut ui = Ui::new(&atlas, &mut batch, cursor);
        f(&mut ui)
    }

    fn phases() -> Vec<Phase> {
        vec![
            Phase::NoClientId {
                folder: "/data/x".into(),
            },
            Phase::SignedOut {
                note: Some("Sign-in was declined in the browser.".into()),
            },
            Phase::Code {
                code: "AB12CD34".into(),
                uri: "https://www.microsoft.com/link".into(),
                status: String::new(),
            },
            Phase::Checking,
            Phase::Ready {
                name: "Steve".into(),
                note: None,
            },
            Phase::Downloading {
                stage: "Downloading".into(),
                files: (12, 400),
                bytes: (30 << 20, 500 << 20),
            },
            Phase::Preparing {
                line: "Importing the client jar".into(),
            },
            Phase::Finished {
                name: "Steve".into(),
            },
            Phase::Blocked {
                title: "Needs Java".into(),
                message: "Java 25 was not found.".into(),
            },
        ]
    }

    fn view(tab: Tab, phase: Phase) -> View {
        View {
            tab,
            phase,
            notes: Notes::Loaded(vec![Note {
                title: "v1".into(),
                body: "Line one\nLine two".into(),
            }]),
            folder: "/data/user/0/dev.horizon.mobile/files".into(),
            ..View::new()
        }
    }

    #[test]
    fn every_page_in_every_phase_lays_out_and_scrolls_within_bounds() {
        for size in [[1280, 720], [800, 480], [412, 915], [2560, 1600]] {
            for tab in Tab::ALL {
                for phase in phases() {
                    let v = view(tab, phase);
                    with_ui(size, (0.0, 0.0), |ui| {
                        let f = layout(ui, &v);
                        assert!(f.content_height > 0.0);
                        assert!(f.max_scroll() >= 0.0);
                    });
                }
            }
        }
    }

    #[test]
    fn tabs_are_where_they_are_drawn() {
        for size in [[1280, 720], [412, 915]] {
            let v = view(
                Tab::Play,
                Phase::Ready {
                    name: "Steve".into(),
                    note: None,
                },
            );
            let spots: Vec<(Cmd, (f32, f32), f32)> = with_ui(size, (0.0, 0.0), |ui| {
                let f = layout(ui, &v);
                f.targets
                    .iter()
                    .filter(|t| matches!(t.cmd, Cmd::Tab(_)))
                    .map(|t| (t.cmd, (t.x + t.w / 2.0, t.y + t.h / 2.0), ui.density))
                    .collect()
            });
            assert_eq!(spots.len(), Tab::ALL.len());
            for (cmd, c, density) in spots {
                let got = with_ui(size, (c.0 * density, c.1 * density), |ui| {
                    click(ui, &layout(ui, &v))
                });
                assert_eq!(got, Some(cmd), "{size:?}");
            }
        }
    }

    #[test]
    fn play_button_follows_the_phase() {
        let find = |phase: Phase| -> Vec<Cmd> {
            let v = view(Tab::Play, phase);
            with_ui([1280, 720], (0.0, 0.0), |ui| {
                layout(ui, &v)
                    .targets
                    .iter()
                    .map(|t| t.cmd)
                    .filter(|c| !matches!(c, Cmd::Tab(_)))
                    .collect()
            })
        };
        assert!(
            find(Phase::Ready {
                name: "S".into(),
                note: None
            })
            .contains(&Cmd::Play)
        );
        assert!(find(Phase::SignedOut { note: None }).contains(&Cmd::SignIn));
        assert!(find(Phase::Finished { name: "S".into() }).contains(&Cmd::Start));
        assert!(
            find(Phase::Downloading {
                stage: "x".into(),
                files: (0, 1),
                bytes: (0, 1)
            })
            .contains(&Cmd::CancelDownload)
        );
        // Busy phases have no way to start another job.
        assert!(!find(Phase::Checking).contains(&Cmd::Play));
        assert!(
            !find(Phase::NoClientId {
                folder: String::new()
            })
            .contains(&Cmd::SignIn)
        );
    }

    #[test]
    fn instances_mods_and_the_dialog_lay_out_and_take_their_presses() {
        let entry = |n: &str, installed| Entry {
            id: n.to_lowercase(),
            name: n.into(),
            summary:
                "A summary that is quite long and has to be cut to fit the row on a small screen"
                    .into(),
            version: "1.0".into(),
            size: 3 << 20,
            installed,
            authors: "Someone".into(),
            downloads: 1234,
        };
        for size in [[1280, 720], [412, 915]] {
            // Play with several instances: each chip, New and Delete are pressable.
            let mut v = view(
                Tab::Play,
                Phase::Ready {
                    name: "Steve".into(),
                    note: None,
                },
            );
            v.instances = vec!["Default".into(), "Survival".into(), "Creative build".into()];
            v.instance = "Survival".into();
            let cmds: Vec<Cmd> = with_ui(size, (0.0, 0.0), |ui| {
                layout(ui, &v).targets.iter().map(|t| t.cmd).collect()
            });
            for want in [
                Cmd::Instance(0),
                Cmd::Instance(1),
                Cmd::Instance(2),
                Cmd::NewInstance,
                Cmd::DeleteInstance,
            ] {
                assert!(cmds.contains(&want), "{size:?} {want:?}");
            }
            // The Default instance can't be deleted.
            v.instance = "Default".into();
            let cmds: Vec<Cmd> = with_ui(size, (0.0, 0.0), |ui| {
                layout(ui, &v).targets.iter().map(|t| t.cmd).collect()
            });
            assert!(!cmds.contains(&Cmd::DeleteInstance));

            // The Mods page: tabs, installed files, install buttons (not for an installed one).
            let mut m = view(Tab::Mods, Phase::Checking);
            m.mods = ModsView {
                sort: 2,
                status: ModsStatus::Loaded(vec![entry("Alpha", false), entry("Beta", true)]),
                installed: vec!["beta.zip".into()],
                busy: None,
                note: Some("Installed beta.zip.".into()),
                source: 0,
                query: "faith".into(),
                everything: false,
                more: true,
            };
            let cmds: Vec<Cmd> = with_ui(size, (0.0, 0.0), |ui| {
                layout(ui, &m).targets.iter().map(|t| t.cmd).collect()
            });
            for want in [
                Cmd::ModSort(0),
                Cmd::ModSort(1),
                Cmd::ModSort(2),
                Cmd::ModSort(3),
                Cmd::Install(0),
                Cmd::RemoveFile(0),
                Cmd::ModSource(0),
                Cmd::ModSource(1),
                Cmd::ModSearch,
                Cmd::ModEverything,
                Cmd::ModMore,
            ] {
                assert!(cmds.contains(&want), "{size:?} {want:?}");
            }
            assert!(
                !cmds.contains(&Cmd::Install(1)),
                "an installed project has no Install button"
            );

            // Native mods are listed but not installable here.
            let mut n = m.clone();
            n.mods.sort = 1;
            let cmds: Vec<Cmd> = with_ui(size, (0.0, 0.0), |ui| {
                layout(ui, &n).targets.iter().map(|t| t.cmd).collect()
            });
            assert!(
                !cmds.iter().any(|c| matches!(c, Cmd::Install(_))),
                "no Install for native mods"
            );
            assert!(cmds.contains(&Cmd::ModSearch));

            // A dialog takes every press: only its own buttons answer.
            let mut d = view(Tab::Play, Phase::SignedOut { note: None });
            d.dialog = Some(Dialog {
                kind: DialogKind::NewInstance,
                title: "New instance".into(),
                text: "Sky".into(),
                error: Some("Taken.".into()),
            });
            let (cmds, modal): (Vec<Cmd>, bool) = with_ui(size, (0.0, 0.0), |ui| {
                let f = layout(ui, &d);
                (f.targets.iter().map(|t| t.cmd).collect(), f.modal)
            });
            assert!(modal);
            assert_eq!(cmds, [Cmd::DialogCancel, Cmd::DialogOk]);
        }
    }

    #[test]
    fn scrolled_content_does_not_take_header_clicks() {
        let mut v = view(Tab::Settings, Phase::SignedOut { note: None });
        v.scroll = 60.0;
        // A cursor in the header over where a scrolled button would be does nothing.
        let got = with_ui([1280, 720], (200.0, 20.0), |ui| click(ui, &layout(ui, &v)));
        assert!(got.is_none() || matches!(got, Some(Cmd::Tab(_))));
    }
}
