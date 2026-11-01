//! Touch controls for the mobile build. Pure logic with no windowing or
//! platform dependency: the platform layer feeds [`TouchControls::touch_down`],
//! [`touch_move`], [`touch_up`] and [`touch_cancel`], and each client tick reads
//! a [`TouchFrame`] with [`TouchControls::take_frame`].
//!
//! Playing (landscape):
//! - Left half of the screen: a floating joystick that appears where the
//!   finger lands and moves the player.
//! - Right half: dragging turns the camera, a quick tap uses/places and a still
//!   hold attacks/breaks.
//! - Buttons from the editable [`Layout`]: jump, sneak, inventory, chat,
//!   keyboard and so on, each a held, latched (toggle) or one-shot action.
//! - The nine hotbar slots.
//! - A small gear at the top right that opens **edit mode**.
//!
//! Edit mode: drag a button to move it, tap one to select it, and use the bar
//! at the top to add, remove, change a button's action, resize it, make it a
//! toggle, reset the layout or finish. Gameplay input is off meanwhile.
//!
//! Movement is `[strafe_left, forward]`, each in `-1.0..=1.0`, the same order
//! as `horizon_client_input::input::keyboard_direction`.
#![forbid(unsafe_code)]

pub mod layout;
pub use layout::{Action, Actions, Button, KEYS, KeySet, Label, Layout, key_index};

pub type Vec2 = [f32; 2];

/// Longest press, in milliseconds, that still counts as a tap.
pub const TAP_MAX_MS: u64 = 200;
/// A still press held this long, in milliseconds, becomes an attack hold.
pub const HOLD_MS: u64 = 300;
/// Movement in pixels under which a press still counts as still.
pub const SLOP_PX: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, p: Vec2) -> bool {
        p[0] >= self.x && p[0] < self.x + self.w && p[1] >= self.y && p[1] < self.y + self.h
    }
}

/// Everything the game needs from touch for one tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TouchFrame {
    /// `[strafe_left, forward]`, length at most 1.
    pub movement: Vec2,
    /// Camera movement since the last frame, in pixels (x right, y down).
    pub look_delta: Vec2,
    /// Held-type actions that are down now (finger down or latched), plus
    /// `Attack` while a still hold is attacking.
    pub held: Actions,
    /// One-shot actions pressed since the last frame. `Use` here is a tap.
    pub pressed: Actions,
    /// The keys and mouse buttons down now, by [`KEYS`] index: buttons bound with `Action::Key`.
    pub keys_held: KeySet,
    /// Keys pressed since the last frame, even for a tap shorter than a frame (also in `keys_held` while down).
    pub keys_tapped: KeySet,
    /// Hotbar slot (0..9) tapped this frame.
    pub hotbar_slot: Option<u8>,
    /// Edit mode is open: the game should not act on anything else.
    pub editing: bool,
}

/// The buttons of the edit bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Done,
    Add,
    Remove,
    Action,
    Smaller,
    Bigger,
    Key,
    Rename,
    Toggle,
    Reset,
}

impl Tool {
    /// In the order of the edit box's grid, five to a row.
    pub const ALL: [Tool; 10] = [
        Tool::Add,
        Tool::Remove,
        Tool::Action,
        Tool::Key,
        Tool::Rename,
        Tool::Smaller,
        Tool::Bigger,
        Tool::Toggle,
        Tool::Reset,
        Tool::Done,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Tool::Done => "Done",
            Tool::Add => "Add",
            Tool::Remove => "Remove",
            Tool::Action => "Action",
            Tool::Smaller => "-",
            Tool::Bigger => "+",
            Tool::Key => "Key",
            Tool::Rename => "Rename",
            Tool::Toggle => "Toggle",
            Tool::Reset => "Reset",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiKind {
    Button,
    Tool,
    Gear,
    /// The Back button of menu screens (Esc).
    Back,
    /// The Keys button of menu screens: send any key to the game.
    Keys,
    /// A fixed movement stick.
    Stick,
    /// The corner handle that resizes the selected button.
    Handle,
    /// The box the edit tools sit in.
    Panel,
}

/// Something to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct UiItem {
    pub rect: Rect,
    pub label: String,
    /// For a stick: where the finger is (pixels), else None.
    pub knob: Option<Vec2>,
    pub kind: UiKind,
    /// Pressed or latched (a button), selected (edit mode), or usable (a tool).
    pub active: bool,
    /// A toggle button, so it can be marked.
    pub toggle: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
    Joystick {
        origin: Vec2,
        current: Vec2,
    },
    Look {
        start: Vec2,
        last: Vec2,
        start_ms: u64,
        moved: bool,
        holding: bool,
    },
    Button(usize),
    Stick {
        index: usize,
        current: Vec2,
    },
    Drag {
        index: usize,
        dx: f32,
        dy: f32,
    },
    Resize {
        index: usize,
    },
    Hotbar(u8),
    /// Consumed on touch down (the gear, an edit tool, empty space in edit mode).
    Ignore,
}

#[derive(Clone, Copy, Debug)]
struct Pointer {
    id: u64,
    role: Role,
}

pub struct TouchControls {
    width: f32,
    height: f32,
    /// Joystick distance in pixels for full speed.
    pub joystick_radius: f32,
    /// Look sensitivity multiplier.
    pub look_sensitivity: f32,
    layout: Layout,
    editing: bool,
    selected: Option<usize>,
    latched: Actions,
    latched_keys: KeySet,
    key_taps: KeySet,
    picker_request: bool,
    rename_request: bool,
    hotbar: Option<[Rect; 9]>,
    layout_dirty: bool,
    pointers: Vec<Pointer>,
    look_accum: Vec2,
    use_pending: bool,
    pressed_pending: Actions,
    hotbar_pending: Option<u8>,
    menu_request: bool,
    /// How opaque the buttons are drawn, 0.2..=1.0.
    pub opacity: f32,
    /// The buttons are hidden (the joystick and look gestures still work).
    pub hidden: bool,
}

impl TouchControls {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            joystick_radius: 90.0,
            look_sensitivity: 1.0,
            layout: Layout::default(),
            editing: false,
            selected: None,
            latched: Actions::default(),
            latched_keys: KeySet::default(),
            key_taps: KeySet::default(),
            picker_request: false,
            rename_request: false,
            hotbar: None,
            layout_dirty: false,
            pointers: Vec::new(),
            look_accum: [0.0, 0.0],
            use_pending: false,
            pressed_pending: Actions::default(),
            hotbar_pending: None,
            menu_request: false,
            opacity: 0.75,
            hidden: false,
        }
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        self.width = width;
        self.height = height;
    }

    /// Where the game's own hotbar slots are, in pixels, so taps line up with
    /// what is drawn. Without it the touch crate's own row is used.
    pub fn set_hotbar_rects(&mut self, rects: Option<[Rect; 9]>) {
        self.hotbar = rects;
    }

    // ---- layout -----------------------------------------------------------

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
        self.selected = None;
        self.latched = Actions::default();
        self.latched_keys = KeySet::default();
        self.pointers.clear();
    }

    pub fn is_editing(&self) -> bool {
        self.editing
    }

    /// The layout as text when it changed since the last call, to save.
    pub fn take_changed_layout(&mut self) -> Option<String> {
        std::mem::take(&mut self.layout_dirty).then(|| self.layout.serialize())
    }

    pub fn set_editing(&mut self, on: bool) {
        if self.editing != on {
            self.editing = on;
            self.selected = None;
            self.pointers.clear();
            self.look_accum = [0.0, 0.0];
            self.use_pending = false;
            self.pressed_pending = Actions::default();
            self.key_taps = KeySet::default();
            self.hotbar_pending = None;
        }
    }

    fn button_px(&self, b: &Button) -> Rect {
        let s = (b.size * self.height).max(40.0);
        Rect {
            x: b.x * self.width - s / 2.0,
            y: b.y * self.height - s / 2.0,
            w: s,
            h: s,
        }
    }

    pub fn button_rect(&self, index: usize) -> Option<Rect> {
        self.layout.buttons.get(index).map(|b| self.button_px(b))
    }

    /// The menu button, bottom left (the ≡ that opens the touch menu).
    pub fn gear_rect(&self) -> Rect {
        let s = (self.height * 0.075).max(40.0);
        Rect {
            x: s * 0.25,
            y: self.height - s * 1.25,
            w: s * 2.4,
            h: s,
        }
    }

    /// The Back button, beside the menu button (shown on menu screens: it is Esc).
    pub fn back_rect(&self) -> Rect {
        let g = self.gear_rect();
        Rect {
            x: g.x + g.w + 10.0,
            y: g.y,
            w: g.h * 1.9,
            h: g.h,
        }
    }

    /// The Keys button, beside Back (shown on menu screens).
    pub fn keys_rect(&self) -> Rect {
        let b = self.back_rect();
        Rect {
            x: b.x + b.w + 10.0,
            y: b.y,
            w: b.h * 1.9,
            h: b.h,
        }
    }

    /// Camera movement since the last call (pixels times the sensitivity), for the game to apply
    /// at once rather than once a tick.
    pub fn take_look(&mut self) -> Vec2 {
        std::mem::take(&mut self.look_accum)
    }

    /// The corner handle of button `index` (bottom right), for resizing in edit mode.
    pub fn handle_rect(&self, index: usize) -> Option<Rect> {
        let r = self.button_rect(index)?;
        let s = 44.0;
        Some(Rect {
            x: r.x + r.w - s * 0.6,
            y: r.y + r.h - s * 0.6,
            w: s,
            h: s,
        })
    }

    /// The menu button was pressed since the last call.
    pub fn take_menu_request(&mut self) -> bool {
        std::mem::take(&mut self.menu_request)
    }

    /// The edit tools' box: a small card at the top middle, five tools to a row.
    pub fn panel_rect(&self) -> Rect {
        let (cw, ch, gap, head) = self.tool_metrics();
        let cols = 5.0;
        let rows = (Tool::ALL.len() as f32 / cols).ceil();
        let w = cols * cw + (cols + 1.0) * gap;
        let h = head + rows * ch + (rows + 1.0) * gap;
        Rect {
            x: ((self.width - w) / 2.0).round(),
            y: 10.0,
            w,
            h,
        }
    }

    /// Chip width and height, the gap and the header, in pixels.
    fn tool_metrics(&self) -> (f32, f32, f32, f32) {
        let cw = (self.height * 0.065).clamp(56.0, 96.0);
        let ch = (self.height * 0.042).clamp(34.0, 52.0);
        (cw, ch, 8.0, 34.0)
    }

    pub fn tool_rect(&self, tool: Tool) -> Rect {
        let (cw, ch, gap, head) = self.tool_metrics();
        let p = self.panel_rect();
        let i = Tool::ALL.iter().position(|t| *t == tool).unwrap_or(0);
        let (col, row) = ((i % 5) as f32, (i / 5) as f32);
        Rect {
            x: p.x + gap + col * (cw + gap),
            y: p.y + head + gap + row * (ch + gap),
            w: cw,
            h: ch,
        }
    }

    /// Hotbar slot rectangle, `slot` in `0..9`.
    pub fn hotbar_rect(&self, slot: u8) -> Rect {
        if let Some(r) = &self.hotbar {
            return r[usize::from(slot).min(8)];
        }
        let s = (self.height * 0.11).max(36.0).min(self.width / 11.0);
        let total = s * 9.0;
        Rect {
            x: (self.width - total) / 2.0 + f32::from(slot) * s,
            y: self.height - s,
            w: s,
            h: s,
        }
    }

    // ---- editing commands -------------------------------------------------

    fn touch_layout(&mut self) {
        self.layout_dirty = true;
    }

    /// Adds a button for the first action that has none yet, in the middle of
    /// the screen, and selects it. Returns whether one was added.
    pub fn add_button(&mut self) -> bool {
        if self.layout.buttons.len() >= layout::MAX_BUTTONS {
            return false;
        }
        let action = Action::ALL
            .into_iter()
            .find(|a| !self.layout.buttons.iter().any(|b| b.action == *a))
            .unwrap_or(Action::Jump);
        self.layout.buttons.push(Button {
            action,
            x: 0.5,
            y: 0.35,
            size: 0.15,
            toggle: false,
            key: None,
            label: None,
        });
        self.selected = Some(self.layout.buttons.len() - 1);
        self.touch_layout();
        true
    }

    pub fn remove_selected(&mut self) -> bool {
        match self.selected.take() {
            Some(i) if i < self.layout.buttons.len() => {
                self.layout.buttons.remove(i);
                self.latched = Actions::default();
                self.touch_layout();
                true
            }
            _ => false,
        }
    }

    /// Gives the selected button the next action.
    pub fn cycle_action(&mut self) -> bool {
        let Some(b) = self.selected.and_then(|i| self.layout.buttons.get_mut(i)) else {
            return false;
        };
        let at = Action::ALL.iter().position(|a| *a == b.action).unwrap_or(0);
        b.action = Action::ALL[(at + 1) % Action::ALL.len()];
        if !b.action.is_held() {
            b.toggle = false;
        }
        if b.action == Action::Key && b.key.is_none() {
            b.key = key_index("KeyE");
        }
        self.touch_layout();
        true
    }

    pub fn resize_selected(&mut self, delta: f32) -> bool {
        let Some(b) = self.selected.and_then(|i| self.layout.buttons.get_mut(i)) else {
            return false;
        };
        b.size = (b.size + delta).clamp(layout::MIN_SIZE, layout::MAX_SIZE);
        self.touch_layout();
        true
    }

    /// Makes the selected held-type button latch (or stop latching).
    pub fn toggle_selected(&mut self) -> bool {
        let Some(b) = self.selected.and_then(|i| self.layout.buttons.get_mut(i)) else {
            return false;
        };
        if !b.action.is_held() {
            return false;
        }
        b.toggle = !b.toggle;
        self.touch_layout();
        true
    }

    /// The Key tool was pressed with a button selected: the game should offer its keys.
    pub fn take_picker_request(&mut self) -> bool {
        std::mem::take(&mut self.picker_request)
    }

    /// The Rename tool was pressed with a button selected: the game asks for a name.
    pub fn take_rename_request(&mut self) -> bool {
        std::mem::take(&mut self.rename_request)
    }

    /// The selected button's own name, if it has one.
    pub fn selected_label(&self) -> Option<String> {
        let b = self.selected.and_then(|i| self.layout.buttons.get(i))?;
        b.label.as_ref().map(|l| l.as_str().to_owned())
    }

    /// Names the selected button (empty text clears the name).
    pub fn set_selected_label(&mut self, text: &str) -> bool {
        let Some(b) = self.selected.and_then(|i| self.layout.buttons.get_mut(i)) else {
            return false;
        };
        b.label = Label::new(text);
        self.touch_layout();
        true
    }

    /// Whether movement is a stick, and switching it (the W/A/S/D cross otherwise).
    pub fn has_stick(&self) -> bool {
        self.layout.has_stick()
    }

    pub fn set_movement_stick(&mut self, stick: bool) {
        self.layout.set_movement_stick(stick);
        self.selected = None;
        self.latched_keys = KeySet::default();
        self.pointers.clear();
        self.touch_layout();
    }

    /// Binds the selected button to `KEYS[key]` (making it a key button).
    pub fn bind_selected(&mut self, key: u8) -> bool {
        let Some(b) = self.selected.and_then(|i| self.layout.buttons.get_mut(i)) else {
            return false;
        };
        if usize::from(key) >= KEYS.len() {
            return false;
        }
        b.action = Action::Key;
        b.key = Some(key);
        self.touch_layout();
        true
    }

    /// The floating joystick while a finger is on it: where it started and where the finger is.
    pub fn joystick(&self) -> Option<(Vec2, Vec2)> {
        self.pointers.iter().find_map(|p| match p.role {
            Role::Joystick { origin, current } if !self.editing => Some((origin, current)),
            _ => None,
        })
    }

    pub fn reset_layout(&mut self) {
        self.layout = Layout::default();
        self.selected = None;
        self.latched = Actions::default();
        self.touch_layout();
    }

    fn run_tool(&mut self, tool: Tool) {
        match tool {
            Tool::Done => self.set_editing(false),
            Tool::Add => {
                self.add_button();
            }
            Tool::Remove => {
                self.remove_selected();
            }
            Tool::Action => {
                self.cycle_action();
            }
            Tool::Smaller => {
                self.resize_selected(-0.02);
            }
            Tool::Bigger => {
                self.resize_selected(0.02);
            }
            Tool::Key => {
                // The game shows its key picker.
                if self.selected.is_some() {
                    self.picker_request = true;
                }
            }
            Tool::Rename => {
                if self.selected.is_some() {
                    self.rename_request = true;
                }
            }
            Tool::Toggle => {
                self.toggle_selected();
            }
            Tool::Reset => self.reset_layout(),
        }
    }

    // ---- touches ----------------------------------------------------------

    fn button_at(&self, p: Vec2) -> Option<usize> {
        if self.hidden && !self.editing {
            return None;
        }
        // The last one added is drawn on top.
        (0..self.layout.buttons.len())
            .rev()
            .find(|&i| self.button_px(&self.layout.buttons[i]).contains(p))
    }

    pub fn touch_down(&mut self, id: u64, pos: Vec2, ms: u64) {
        // A repeated id means a missed up event: replace it.
        self.pointers.retain(|p| p.id != id);

        if self.gear_rect().contains(pos) && !self.editing {
            self.menu_request = true;
            self.pointers.push(Pointer {
                id,
                role: Role::Ignore,
            });
            return;
        }

        if self.editing {
            let handle = self
                .selected
                .and_then(|i| self.handle_rect(i).filter(|h| h.contains(pos)).map(|_| i));
            let role = if let Some(index) = handle {
                Role::Resize { index }
            } else if let Some(tool) = Tool::ALL
                .into_iter()
                .find(|t| self.tool_rect(*t).contains(pos))
            {
                self.run_tool(tool);
                Role::Ignore
            } else if self.panel_rect().contains(pos) {
                // The box between its tools: nothing under it is touched.
                Role::Ignore
            } else if let Some(i) = self.button_at(pos) {
                self.selected = Some(i);
                let r = self.button_px(&self.layout.buttons[i]);
                Role::Drag {
                    index: i,
                    dx: pos[0] - (r.x + r.w / 2.0),
                    dy: pos[1] - (r.y + r.h / 2.0),
                }
            } else {
                self.selected = None;
                Role::Ignore
            };
            // run_tool may have left edit mode and cleared the pointers.
            self.pointers.push(Pointer { id, role });
            return;
        }

        let role = self.pick_role(pos, ms);
        let role = match role {
            Role::Button(i) if self.layout.buttons[i].action == Action::Stick => Role::Stick {
                index: i,
                current: pos,
            },
            other => other,
        };
        // Only one pointer drives the joystick or the camera at a time.
        let duplicate = self.pointers.iter().any(|p| {
            matches!(
                (p.role, role),
                (Role::Joystick { .. }, Role::Joystick { .. })
                    | (Role::Look { .. }, Role::Look { .. })
            )
        });
        if duplicate {
            return;
        }
        match role {
            Role::Hotbar(slot) => self.hotbar_pending = Some(slot),
            Role::Button(i) => {
                let b = self.layout.buttons[i];
                if !b.action.is_held() {
                    self.pressed_pending.insert(b.action);
                } else if b.toggle {
                    // Latch: flip on each press.
                    match (b.action, b.key) {
                        (Action::Key, Some(k)) => self.latched_keys.flip(k),
                        _ => self.latched = flip(self.latched, b.action),
                    }
                } else {
                    // Held while the finger is down; the press is also remembered, so a tap shorter
                    // than a frame still counts.
                    match (b.action, b.key) {
                        (Action::Key, Some(k)) => self.key_taps.insert(k),
                        (Action::Key, None) => {}
                        _ => self.pressed_pending.insert(b.action),
                    }
                }
            }
            _ => {}
        }
        self.pointers.push(Pointer { id, role });
    }

    fn pick_role(&self, p: Vec2, ms: u64) -> Role {
        if let Some(i) = self.button_at(p) {
            return Role::Button(i);
        }
        for slot in 0..9u8 {
            if self.hotbar_rect(slot).contains(p) {
                return Role::Hotbar(slot);
            }
        }
        if p[0] < self.width / 2.0 {
            Role::Joystick {
                origin: p,
                current: p,
            }
        } else {
            Role::Look {
                start: p,
                last: p,
                start_ms: ms,
                moved: false,
                holding: false,
            }
        }
    }

    pub fn touch_move(&mut self, id: u64, pos: Vec2) {
        let sens = self.look_sensitivity;
        let (w, h) = (self.width, self.height);
        let Some(ptr) = self.pointers.iter_mut().find(|p| p.id == id) else {
            return;
        };
        match &mut ptr.role {
            Role::Joystick { current, .. } => *current = pos,
            Role::Look {
                start,
                last,
                moved,
                holding,
                ..
            } => {
                let dx = pos[0] - start[0];
                let dy = pos[1] - start[1];
                if (dx * dx + dy * dy).sqrt() > SLOP_PX {
                    *moved = true;
                    // Moving cancels a pending or active hold-attack.
                    *holding = false;
                }
                self.look_accum[0] += (pos[0] - last[0]) * sens;
                self.look_accum[1] += (pos[1] - last[1]) * sens;
                *last = pos;
            }
            Role::Stick { current, .. } => *current = pos,
            Role::Resize { index } => {
                if let Some(b) = self.layout.buttons.get_mut(*index) {
                    // The button's centre stays; its corner follows the finger.
                    let (cx, cy) = (b.x * w, b.y * h);
                    let half = (pos[0] - cx).abs().max((pos[1] - cy).abs());
                    b.size = (2.0 * half / h).clamp(layout::MIN_SIZE, layout::MAX_SIZE);
                }
            }
            Role::Drag { index, dx, dy } => {
                if let Some(b) = self.layout.buttons.get_mut(*index) {
                    b.x = ((pos[0] - *dx) / w).clamp(0.0, 1.0);
                    b.y = ((pos[1] - *dy) / h).clamp(0.0, 1.0);
                }
            }
            _ => {}
        }
    }

    pub fn touch_up(&mut self, id: u64, ms: u64) {
        let Some(i) = self.pointers.iter().position(|p| p.id == id) else {
            return;
        };
        let ptr = self.pointers.remove(i);
        match ptr.role {
            Role::Look {
                moved: false,
                start_ms,
                holding: false,
                ..
            } if ms.saturating_sub(start_ms) <= TAP_MAX_MS => {
                self.use_pending = true;
            }
            Role::Drag { .. } | Role::Resize { .. } => self.touch_layout(),
            _ => {}
        }
    }

    /// The system took the touch away (a gesture, a call): drop it without
    /// turning it into a tap.
    pub fn touch_cancel(&mut self, id: u64) {
        if let Some(i) = self.pointers.iter().position(|p| p.id == id) {
            if matches!(
                self.pointers[i].role,
                Role::Drag { .. } | Role::Resize { .. }
            ) {
                self.touch_layout();
            }
            self.pointers.remove(i);
        }
    }

    /// Drops every touch and pending event (a menu opened, the window lost focus).
    pub fn reset(&mut self) {
        self.pointers.clear();
        self.look_accum = [0.0, 0.0];
        self.use_pending = false;
        self.pressed_pending = Actions::default();
        self.key_taps = KeySet::default();
        self.hotbar_pending = None;
    }

    /// Call once per tick with the current time. Clears the one-shot events.
    pub fn take_frame(&mut self, ms: u64) -> TouchFrame {
        let mut frame = TouchFrame {
            editing: self.editing,
            ..TouchFrame::default()
        };
        if self.editing {
            return frame;
        }
        for p in &mut self.pointers {
            match &mut p.role {
                Role::Joystick { origin, current } => {
                    let dx = (current[0] - origin[0]) / self.joystick_radius;
                    let dy = (current[1] - origin[1]) / self.joystick_radius;
                    let len = (dx * dx + dy * dy).sqrt();
                    let k = if len > 1.0 { 1.0 / len } else { 1.0 };
                    // Screen y grows downward; dragging up is forward.
                    frame.movement = [-dx * k, -dy * k];
                }
                Role::Look {
                    start_ms,
                    moved,
                    holding,
                    ..
                } => {
                    if !*moved && !*holding && ms.saturating_sub(*start_ms) >= HOLD_MS {
                        *holding = true;
                    }
                    if *holding {
                        frame.held.insert(Action::Attack);
                    }
                }
                Role::Stick { index, current } => {
                    if let Some(r) = self.layout.buttons.get(*index).map(|b| {
                        let s = (b.size * self.height).max(40.0);
                        (b.x * self.width, b.y * self.height, s / 2.0)
                    }) {
                        let dx = (current[0] - r.0) / r.2;
                        let dy = (current[1] - r.1) / r.2;
                        let len = (dx * dx + dy * dy).sqrt();
                        let k = if len > 1.0 { 1.0 / len } else { 1.0 };
                        // A small dead zone in the middle.
                        if len > 0.12 {
                            frame.movement = [-dx * k, -dy * k];
                        }
                    }
                }
                Role::Button(i) => {
                    if let Some(b) = self.layout.buttons.get(*i)
                        && b.action.is_held()
                        && !b.toggle
                    {
                        match (b.action, b.key) {
                            (Action::Key, Some(k)) => frame.keys_held.insert(k),
                            (Action::Key, None) => {}
                            _ => frame.held.insert(b.action),
                        }
                    }
                }
                _ => {}
            }
        }
        for a in self.latched.iter() {
            frame.held.insert(a);
        }
        for k in self.latched_keys.iter() {
            frame.keys_held.insert(k);
        }
        frame.pressed = std::mem::take(&mut self.pressed_pending);
        frame.keys_tapped = std::mem::take(&mut self.key_taps);
        if std::mem::take(&mut self.use_pending) {
            frame.pressed.insert(Action::Use);
        }
        frame.look_delta = std::mem::take(&mut self.look_accum);
        frame.hotbar_slot = self.hotbar_pending.take();
        frame
    }

    /// What to draw: the gear, the buttons and, in edit mode, the edit bar.
    pub fn ui_items(&self) -> Vec<UiItem> {
        let mut items = Vec::new();
        let down: Actions = {
            let mut a = Actions::default();
            for p in &self.pointers {
                if let Role::Button(i) = p.role
                    && let Some(b) = self.layout.buttons.get(i)
                {
                    a.insert(b.action);
                }
            }
            a
        };
        for (i, b) in self.layout.buttons.iter().enumerate() {
            if self.hidden && !self.editing {
                break;
            }
            let active = if self.editing {
                self.selected == Some(i)
            } else {
                down.has(b.action)
                    || (b.toggle
                        && (self.latched.has(b.action)
                            || b.key.is_some_and(|k| self.latched_keys.has(k))))
            };
            let label = match (b.label.as_ref(), b.action, b.key) {
                (Some(l), _, _) => l.as_str().to_owned(),
                (None, Action::Key, Some(k)) => {
                    KEYS.get(usize::from(k)).map_or("Key", |k| k.1).to_owned()
                }
                (None, a, _) => a.label().to_owned(),
            };
            let knob = (b.action == Action::Stick).then(|| {
                let r = self.button_px(b);
                let centre = [r.x + r.w / 2.0, r.y + r.h / 2.0];
                self.pointers
                    .iter()
                    .find_map(|p| match p.role {
                        Role::Stick { index, current } if index == i => Some(current),
                        _ => None,
                    })
                    .unwrap_or(centre)
            });
            items.push(UiItem {
                rect: self.button_px(b),
                label,
                knob,
                kind: if b.action == Action::Stick && !self.editing {
                    UiKind::Stick
                } else {
                    UiKind::Button
                },
                active,
                toggle: b.toggle,
            });
        }
        items.push(UiItem {
            rect: self.gear_rect(),
            label: "Menu".into(),
            knob: None,
            kind: UiKind::Gear,
            active: false,
            toggle: false,
        });
        items.push(UiItem {
            rect: self.back_rect(),
            label: "Back".into(),
            knob: None,
            kind: UiKind::Back,
            active: false,
            toggle: false,
        });
        items.push(UiItem {
            rect: self.keys_rect(),
            label: "Keys".into(),
            knob: None,
            kind: UiKind::Keys,
            active: false,
            toggle: false,
        });
        if self.editing
            && let Some(i) = self.selected
            && let Some(r) = self.handle_rect(i)
        {
            items.push(UiItem {
                rect: r,
                label: String::new(),
                knob: None,
                kind: UiKind::Handle,
                active: true,
                toggle: false,
            });
        }
        if self.editing {
            items.push(UiItem {
                rect: self.panel_rect(),
                label: "Edit layout".into(),
                knob: None,
                kind: UiKind::Panel,
                active: true,
                toggle: false,
            });
            for t in Tool::ALL {
                let needs_sel = matches!(
                    t,
                    Tool::Remove
                        | Tool::Action
                        | Tool::Smaller
                        | Tool::Bigger
                        | Tool::Key
                        | Tool::Rename
                        | Tool::Toggle
                );
                items.push(UiItem {
                    rect: self.tool_rect(t),
                    label: t.label().into(),
                    knob: None,
                    kind: UiKind::Tool,
                    active: !needs_sel || self.selected.is_some(),
                    toggle: false,
                });
            }
        }
        items
    }
}

fn flip(set: Actions, a: Action) -> Actions {
    let mut out = Actions::default();
    for x in set.iter() {
        if x != a {
            out.insert(x);
        }
    }
    if !set.has(a) {
        out.insert(a);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl() -> TouchControls {
        TouchControls::new(1920.0, 1080.0)
    }
    fn center(c: &TouchControls, a: Action) -> Vec2 {
        let i = c
            .layout()
            .buttons
            .iter()
            .position(|b| b.action == a)
            .unwrap();
        let r = c.button_rect(i).unwrap();
        [r.x + r.w / 2.0, r.y + r.h / 2.0]
    }
    fn tap(c: &mut TouchControls, id: u64, p: Vec2) {
        c.touch_down(id, p, 0);
        c.touch_up(id, 10);
    }
    fn tool(c: &TouchControls, t: Tool) -> Vec2 {
        let r = c.tool_rect(t);
        [r.x + r.w / 2.0, r.y + r.h / 2.0]
    }

    #[test]
    fn joystick_up_is_forward_and_clamped() {
        let mut c = ctl();
        c.touch_down(1, [700.0, 700.0], 0);
        c.touch_move(1, [700.0, 400.0]);
        let f = c.take_frame(10);
        assert_eq!(f.movement[0], 0.0);
        assert!((f.movement[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn joystick_right_is_negative_strafe_left() {
        let mut c = ctl();
        c.touch_down(1, [700.0, 700.0], 0);
        c.touch_move(1, [745.0, 700.0]);
        let f = c.take_frame(10);
        assert!((f.movement[0] + 0.5).abs() < 1e-5);
        c.touch_up(1, 20);
        assert_eq!(c.take_frame(30).movement, [0.0, 0.0]);
    }

    #[test]
    fn quick_still_tap_uses_once() {
        let mut c = ctl();
        c.touch_down(1, [1400.0, 400.0], 1000);
        c.touch_up(1, 1100);
        assert!(c.take_frame(1100).pressed.has(Action::Use));
        assert!(!c.take_frame(1110).pressed.has(Action::Use));
    }

    #[test]
    fn still_hold_attacks_until_release_without_using() {
        let mut c = ctl();
        c.touch_down(1, [1400.0, 400.0], 0);
        assert!(!c.take_frame(100).held.has(Action::Attack));
        assert!(c.take_frame(350).held.has(Action::Attack));
        c.touch_up(1, 600);
        let f = c.take_frame(610);
        assert!(!f.held.has(Action::Attack));
        assert!(!f.pressed.has(Action::Use));
    }

    #[test]
    fn drag_looks_and_never_taps_or_attacks() {
        let mut c = ctl();
        c.touch_down(1, [1400.0, 400.0], 0);
        c.touch_move(1, [1450.0, 380.0]);
        c.touch_move(1, [1500.0, 360.0]);
        let f = c.take_frame(400);
        assert_eq!(f.look_delta, [100.0, -40.0]);
        assert!(!f.held.has(Action::Attack));
        c.touch_up(1, 450);
        assert!(!c.take_frame(460).pressed.has(Action::Use));
    }

    #[test]
    fn a_tap_shorter_than_a_frame_still_counts() {
        let mut c = ctl();
        let j = center(&c, Action::Jump);
        tap(&mut c, 1, j);
        let f = c.take_frame(5);
        assert!(!f.held.has(Action::Jump));
        assert!(f.pressed.has(Action::Jump));
        let i = c
            .layout()
            .buttons
            .iter()
            .position(|b| b.key == key_index("KeyD"))
            .unwrap();
        let r = c.button_rect(i).unwrap();
        tap(&mut c, 2, [r.x + 3.0, r.y + 3.0]);
        let f = c.take_frame(9);
        assert!(f.keys_tapped.has(key_index("KeyD").unwrap()));
        assert!(!f.keys_held.has(key_index("KeyD").unwrap()));
        assert!(!c.take_frame(10).keys_tapped.has(key_index("KeyD").unwrap()));
    }

    #[test]
    fn held_button_follows_finger_and_oneshot_fires_once() {
        let mut c = ctl();
        let j = center(&c, Action::Jump);
        c.touch_down(1, j, 0);
        assert!(c.take_frame(1).held.has(Action::Jump));
        c.touch_up(1, 2);
        assert!(!c.take_frame(3).held.has(Action::Jump));

        let inv = center(&c, Action::Inventory);
        tap(&mut c, 2, inv);
        assert!(c.take_frame(20).pressed.has(Action::Inventory));
        assert!(!c.take_frame(21).pressed.has(Action::Inventory));
    }

    #[test]
    fn hotbar_taps_select_a_slot() {
        let mut c = ctl();
        let h = c.hotbar_rect(4);
        c.touch_down(2, [h.x + 2.0, h.y + 2.0], 10);
        assert_eq!(c.take_frame(11).hotbar_slot, Some(4));
        assert_eq!(c.take_frame(12).hotbar_slot, None);
    }

    #[test]
    fn game_hotbar_rects_override() {
        let mut c = ctl();
        let mut r = [Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        }; 9];
        r[0] = Rect {
            x: 100.0,
            y: 900.0,
            w: 60.0,
            h: 60.0,
        };
        c.set_hotbar_rects(Some(r));
        c.touch_down(1, [120.0, 920.0], 0);
        assert_eq!(c.take_frame(1).hotbar_slot, Some(0));
    }

    #[test]
    fn multitouch_move_and_look_together() {
        let mut c = ctl();
        c.touch_down(1, [700.0, 700.0], 0);
        c.touch_down(2, [1400.0, 400.0], 0);
        c.touch_move(1, [700.0, 610.0]);
        c.touch_move(2, [1410.0, 400.0]);
        let f = c.take_frame(5);
        assert!((f.movement[1] - 1.0).abs() < 1e-5);
        assert_eq!(f.look_delta, [10.0, 0.0]);
    }

    #[test]
    fn cancel_drops_touch_without_tap() {
        let mut c = ctl();
        c.touch_down(1, [1400.0, 400.0], 0);
        c.touch_cancel(1);
        c.touch_up(1, 50);
        assert!(!c.take_frame(60).pressed.has(Action::Use));
    }

    #[test]
    fn toggle_button_latches_sprint() {
        let mut c = ctl();
        let i = c
            .layout()
            .buttons
            .iter()
            .position(|b| b.action == Action::Sprint)
            .unwrap();
        c.selected = Some(i);
        assert!(c.toggle_selected());
        let p = center(&c, Action::Sprint);
        tap(&mut c, 1, p);
        assert!(c.take_frame(20).held.has(Action::Sprint));
        assert!(c.take_frame(30).held.has(Action::Sprint));
        tap(&mut c, 2, p);
        assert!(!c.take_frame(40).held.has(Action::Sprint));
    }

    #[test]
    fn the_menu_button_asks_for_the_menu_and_edit_mode_blocks_gameplay() {
        let mut c = ctl();
        let g = c.gear_rect();
        let gp = [g.x + 2.0, g.y + 2.0];
        tap(&mut c, 1, gp);
        assert!(c.take_menu_request());
        assert!(!c.take_menu_request());
        assert!(!c.is_editing());
        c.set_editing(true);
        c.touch_down(2, [1400.0, 400.0], 0);
        let f = c.take_frame(500);
        assert!(f.editing);
        assert!(f.held.iter().next().is_none());
        let dp = tool(&c, Tool::Done);
        tap(&mut c, 3, dp);
        assert!(!c.is_editing());
    }

    #[test]
    fn drag_moves_a_button_and_marks_layout_changed() {
        let mut c = ctl();
        c.set_editing(true);
        let from = center(&c, Action::Jump);
        c.touch_down(1, from, 0);
        c.touch_move(1, [from[0] - 400.0, from[1] - 200.0]);
        c.touch_up(1, 5);
        let b = *c
            .layout()
            .buttons
            .iter()
            .find(|b| b.action == Action::Jump)
            .unwrap();
        assert!((b.x * 1920.0 - (from[0] - 400.0)).abs() < 1.0);
        assert!((b.y * 1080.0 - (from[1] - 200.0)).abs() < 1.0);
        let saved = c.take_changed_layout().unwrap();
        assert!(saved.contains("jump"));
        assert!(c.take_changed_layout().is_none());
    }

    #[test]
    fn drag_is_clamped_to_the_screen() {
        let mut c = ctl();
        c.set_editing(true);
        let from = center(&c, Action::Jump);
        c.touch_down(1, from, 0);
        c.touch_move(1, [5000.0, -300.0]);
        c.touch_up(1, 5);
        let b = *c
            .layout()
            .buttons
            .iter()
            .find(|b| b.action == Action::Jump)
            .unwrap();
        assert_eq!((b.x, b.y), (1.0, 0.0));
    }

    #[test]
    fn edit_tools_add_configure_remove() {
        let mut c = ctl();
        c.set_editing(true);
        let n = c.layout().buttons.len();
        let p = tool(&c, Tool::Add);
        tap(&mut c, 1, p);
        assert_eq!(c.layout().buttons.len(), n + 1);
        // The new button is selected: change its action and size.
        let p = tool(&c, Tool::Action);
        tap(&mut c, 2, p);
        assert_eq!(c.layout().buttons[n].action, Action::Use);
        let before = c.layout().buttons[n].size;
        let p = tool(&c, Tool::Bigger);
        tap(&mut c, 3, p);
        assert!(c.layout().buttons[n].size > before);
        let p = tool(&c, Tool::Remove);
        tap(&mut c, 4, p);
        assert_eq!(c.layout().buttons.len(), n);
        // Nothing selected now: Remove does nothing.
        tap(&mut c, 5, p);
        assert_eq!(c.layout().buttons.len(), n);
    }

    #[test]
    fn keyboard_action_is_a_oneshot_button() {
        let mut c = ctl();
        c.set_editing(true);
        c.add_button();
        while c.layout().buttons.last().unwrap().action != Action::Keyboard {
            c.cycle_action();
        }
        c.set_editing(false);
        let p = center(&c, Action::Keyboard);
        tap(&mut c, 1, p);
        assert!(c.take_frame(20).pressed.has(Action::Keyboard));
    }

    #[test]
    fn a_walking_key_button_holds_its_key_while_pressed() {
        let mut c = ctl();
        let i = c
            .layout()
            .buttons
            .iter()
            .position(|b| b.key == key_index("KeyW"))
            .unwrap();
        let r = c.button_rect(i).unwrap();
        c.touch_down(1, [r.x + 3.0, r.y + 3.0], 0);
        assert!(c.take_frame(1).keys_held.has(key_index("KeyW").unwrap()));
        c.touch_up(1, 2);
        assert!(!c.take_frame(3).keys_held.has(key_index("KeyW").unwrap()));
    }

    #[test]
    fn the_key_tool_binds_any_key_to_any_button() {
        let mut c = ctl();
        c.set_editing(true);
        // Select the jump button, press Key: the game is asked for its picker.
        let jump = center(&c, Action::Jump);
        c.touch_down(1, jump, 0);
        c.touch_up(1, 1);
        let p = tool(&c, Tool::Key);
        tap(&mut c, 2, p);
        assert!(c.take_picker_request());
        // The pick binds it.
        assert!(c.bind_selected(key_index("F5").unwrap()));
        let b = c
            .layout()
            .buttons
            .iter()
            .find(|b| b.key == key_index("F5"))
            .unwrap();
        assert_eq!(b.action, Action::Key);
        c.set_editing(false);
        let r = c
            .button_rect(
                c.layout()
                    .buttons
                    .iter()
                    .position(|b| b.key == key_index("F5"))
                    .unwrap(),
            )
            .unwrap();
        c.touch_down(3, [r.x + 3.0, r.y + 3.0], 0);
        assert!(c.take_frame(1).keys_held.has(key_index("F5").unwrap()));
        // Without a selection the tool asks for nothing.
        c.set_editing(true);
        tap(&mut c, 4, p);
        assert!(!c.take_picker_request());
    }

    #[test]
    fn a_toggle_key_latches() {
        let mut c = ctl();
        c.set_editing(true);
        c.add_button();
        assert!(c.bind_selected(key_index("ControlLeft").unwrap()));
        assert!(c.toggle_selected());
        c.set_editing(false);
        let i = c.layout().buttons.len() - 1;
        let r = c.button_rect(i).unwrap();
        tap(&mut c, 1, [r.x + 3.0, r.y + 3.0]);
        assert!(
            c.take_frame(20)
                .keys_held
                .has(key_index("ControlLeft").unwrap())
        );
        assert!(
            c.take_frame(30)
                .keys_held
                .has(key_index("ControlLeft").unwrap())
        );
        tap(&mut c, 2, [r.x + 3.0, r.y + 3.0]);
        assert!(
            !c.take_frame(40)
                .keys_held
                .has(key_index("ControlLeft").unwrap())
        );
    }

    #[test]
    fn the_joystick_is_reported_while_the_finger_is_down() {
        let mut c = ctl();
        c.touch_down(1, [700.0, 600.0], 0);
        c.touch_move(1, [740.0, 580.0]);
        assert_eq!(c.joystick(), Some(([700.0, 600.0], [740.0, 580.0])));
        c.touch_up(1, 5);
        assert!(c.joystick().is_none());
    }

    #[test]
    fn dragging_the_corner_handle_resizes_the_selected_button() {
        let mut c = ctl();
        c.set_editing(true);
        let jump = center(&c, Action::Jump);
        c.touch_down(1, jump, 0);
        c.touch_up(1, 1);
        let i = c
            .layout()
            .buttons
            .iter()
            .position(|b| b.action == Action::Jump)
            .unwrap();
        let before = c.layout().buttons[i].size;
        let h = c.handle_rect(i).unwrap();
        let from = [h.x + h.w / 2.0, h.y + h.h / 2.0];
        assert!(c.ui_items().iter().any(|u| u.kind == UiKind::Handle));
        c.touch_down(2, from, 0);
        c.touch_move(2, [from[0] + 120.0, from[1] + 120.0]);
        c.touch_up(2, 5);
        let after = c.layout().buttons[i].size;
        assert!(after > before + 0.05, "{before} -> {after}");
        // The centre stays where it was.
        let b = c.layout().buttons[i];
        assert!((b.x * 1920.0 - jump[0]).abs() < 1.0);
        // Pulling the corner in shrinks it, down to the minimum.
        let h2 = c.handle_rect(i).unwrap();
        c.touch_down(3, [h2.x + h2.w / 2.0, h2.y + h2.h / 2.0], 10);
        c.touch_move(3, [jump[0], jump[1]]);
        c.touch_up(3, 15);
        assert_eq!(c.layout().buttons[i].size, layout::MIN_SIZE);
        assert!(c.take_changed_layout().is_some());
    }

    #[test]
    fn a_fixed_stick_walks_and_shows_its_knob() {
        let mut c = ctl();
        c.set_movement_stick(true);
        assert!(c.has_stick());
        let i = c
            .layout()
            .buttons
            .iter()
            .position(|b| b.action == Action::Stick)
            .unwrap();
        let r = c.button_rect(i).unwrap();
        let centre = [r.x + r.w / 2.0, r.y + r.h / 2.0];
        c.touch_down(1, centre, 0);
        c.touch_move(1, [centre[0], centre[1] - r.h]);
        let f = c.take_frame(5);
        assert!(
            f.movement[1] > 0.99 && f.movement[0].abs() < 1e-4,
            "{:?}",
            f.movement
        );
        let stick = c
            .ui_items()
            .into_iter()
            .find(|u| u.kind == UiKind::Stick)
            .unwrap();
        assert_eq!(stick.knob, Some([centre[0], centre[1] - r.h]));
        c.touch_up(1, 9);
        assert_eq!(c.take_frame(10).movement, [0.0, 0.0]);
        // Inside the dead zone nothing moves.
        c.touch_down(2, [centre[0] + 2.0, centre[1]], 20);
        assert_eq!(c.take_frame(21).movement, [0.0, 0.0]);
        c.set_movement_stick(false);
        assert!(!c.has_stick());
    }

    #[test]
    fn rename_asks_for_a_name_and_the_name_is_drawn() {
        let mut c = ctl();
        c.set_editing(true);
        let p = tool(&c, Tool::Rename);
        tap(&mut c, 1, p);
        assert!(!c.take_rename_request(), "nothing selected");
        let jump = center(&c, Action::Jump);
        c.touch_down(2, jump, 0);
        c.touch_up(2, 1);
        tap(&mut c, 3, p);
        assert!(c.take_rename_request());
        assert!(c.set_selected_label("Hop"));
        assert_eq!(c.selected_label().as_deref(), Some("Hop"));
        assert!(c.ui_items().iter().any(|u| u.label == "Hop"));
        assert!(c.set_selected_label("  "));
        assert_eq!(c.selected_label(), None);
        assert!(c.ui_items().iter().any(|u| u.label == "Jump"));
    }

    #[test]
    fn look_is_drained_on_demand_and_scaled() {
        let mut c = ctl();
        c.look_sensitivity = 2.0;
        c.touch_down(1, [1400.0, 400.0], 0);
        c.touch_move(1, [1410.0, 405.0]);
        assert_eq!(c.take_look(), [20.0, 10.0]);
        assert_eq!(c.take_look(), [0.0, 0.0]);
        assert_eq!(c.take_frame(5).look_delta, [0.0, 0.0]);
    }

    #[test]
    fn the_keys_button_sits_beside_back() {
        let c = ctl();
        let (b, k) = (c.back_rect(), c.keys_rect());
        assert!(k.x >= b.x + b.w && (k.y - b.y).abs() < 0.01);
        assert!(c.ui_items().iter().any(|u| u.kind == UiKind::Keys));
    }

    #[test]
    fn the_edit_tools_sit_in_one_compact_box() {
        for (w, h) in [
            (1920.0, 1080.0),
            (2000.0, 1250.0),
            (2400.0, 1080.0),
            (800.0, 480.0),
        ] {
            let c = TouchControls::new(w, h);
            let p = c.panel_rect();
            // Small: under half the width and about a quarter of the height.
            assert!(
                p.w < w * 0.5 && p.h < h * 0.28,
                "{w}x{h}: box {}x{}",
                p.w,
                p.h
            );
            assert!(p.x >= 0.0 && p.x + p.w <= w);
            let mut rects = Vec::new();
            for t in Tool::ALL {
                let r = c.tool_rect(t);
                assert!(
                    r.x >= p.x && r.x + r.w <= p.x + p.w && r.y >= p.y && r.y + r.h <= p.y + p.h,
                    "{t:?} inside the box"
                );
                rects.push(r);
            }
            // No two tools overlap.
            for (i, a) in rects.iter().enumerate() {
                for b in &rects[i + 1..] {
                    assert!(
                        a.x + a.w <= b.x
                            || b.x + b.w <= a.x
                            || a.y + a.h <= b.y
                            || b.y + b.h <= a.y
                    );
                }
            }
        }
    }

    #[test]
    fn a_press_in_the_box_between_tools_does_not_deselect_or_drag() {
        let mut c = ctl();
        c.set_editing(true);
        let jump = center(&c, Action::Jump);
        c.touch_down(1, jump, 0);
        c.touch_up(1, 1);
        let before = c.layout().clone();
        let p = c.panel_rect();
        // The header strip of the box is not a tool.
        c.touch_down(2, [p.x + p.w / 2.0, p.y + 4.0], 5);
        c.touch_move(2, [p.x + p.w / 2.0 + 50.0, p.y + 80.0]);
        c.touch_up(2, 8);
        assert_eq!(c.layout(), &before);
        assert!(
            c.ui_items().iter().any(|u| u.kind == UiKind::Handle),
            "the selection is kept"
        );
    }

    #[test]
    fn the_back_button_sits_beside_the_menu_button() {
        let c = ctl();
        let (g, b) = (c.gear_rect(), c.back_rect());
        assert!(b.x >= g.x + g.w && (b.y - g.y).abs() < 0.01);
        assert!(c.ui_items().iter().any(|u| u.kind == UiKind::Back));
    }

    #[test]
    fn reset_restores_default() {
        let mut c = ctl();
        c.set_editing(true);
        c.add_button();
        let p = tool(&c, Tool::Reset);
        tap(&mut c, 1, p);
        assert_eq!(c.layout(), &Layout::default());
    }

    #[test]
    fn hidden_buttons_do_not_take_touches() {
        let mut c = ctl();
        c.hidden = true;
        let j = center(&c, Action::Jump);
        c.touch_down(1, j, 0);
        assert!(!c.take_frame(1).held.has(Action::Jump));
        // Hidden buttons are not drawn either, except in edit mode.
        let n = c
            .ui_items()
            .iter()
            .filter(|i| i.kind == UiKind::Button)
            .count();
        assert_eq!(n, 0);
        c.set_editing(true);
        assert!(c.ui_items().iter().any(|i| i.kind == UiKind::Button));
    }

    #[test]
    fn ui_items_include_edit_bar_only_when_editing() {
        let mut c = ctl();
        let n = c.ui_items().len();
        c.set_editing(true);
        assert_eq!(c.ui_items().len(), n + Tool::ALL.len() + 1);
    }
}
