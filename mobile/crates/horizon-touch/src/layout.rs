//! The editable button layout: which actions have an on-screen button, where
//! each sits and how big it is. Saved as one line per button:
//! `action x y size toggle` (x, y the centre as a fraction of the screen, size
//! a fraction of the screen height).

/// What a button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Jump,
    Sneak,
    Sprint,
    Attack,
    Use,
    Inventory,
    Chat,
    /// Opens the on-screen keyboard.
    Keyboard,
    Drop,
    SwapHands,
    Perspective,
    Pause,
    /// Presses a key or mouse button chosen from [`KEYS`].
    Key,
    /// A fixed movement stick: drag inside it to walk.
    Stick,
}

impl Action {
    pub const ALL: [Action; 14] = [
        Action::Jump,
        Action::Sneak,
        Action::Sprint,
        Action::Attack,
        Action::Use,
        Action::Inventory,
        Action::Chat,
        Action::Keyboard,
        Action::Drop,
        Action::SwapHands,
        Action::Perspective,
        Action::Pause,
        Action::Key,
        Action::Stick,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Action::Jump => "jump",
            Action::Sneak => "sneak",
            Action::Sprint => "sprint",
            Action::Attack => "attack",
            Action::Use => "use",
            Action::Inventory => "inventory",
            Action::Chat => "chat",
            Action::Keyboard => "keyboard",
            Action::Drop => "drop",
            Action::SwapHands => "swap",
            Action::Perspective => "view",
            Action::Pause => "pause",
            Action::Key => "key",
            Action::Stick => "stick",
        }
    }

    /// Short text drawn on the button.
    pub fn label(self) -> &'static str {
        match self {
            Action::Jump => "Jump",
            Action::Sneak => "Sneak",
            Action::Sprint => "Sprint",
            Action::Attack => "Hit",
            Action::Use => "Use",
            Action::Inventory => "Inv",
            Action::Chat => "Chat",
            Action::Keyboard => "Keys",
            Action::Drop => "Drop",
            Action::SwapHands => "Swap",
            Action::Perspective => "View",
            Action::Pause => "Menu",
            Action::Key => "Key",
            Action::Stick => "Stick",
        }
    }

    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.name() == name)
    }

    /// Held actions stay on while the finger is down (or latched); the rest
    /// fire once per press.
    pub fn is_held(self) -> bool {
        matches!(
            self,
            Action::Jump
                | Action::Sneak
                | Action::Sprint
                | Action::Attack
                | Action::Use
                | Action::Key
        )
    }

    fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// A set of actions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Actions(u16);

impl Actions {
    pub fn has(self, a: Action) -> bool {
        self.0 & a.bit() != 0
    }
    pub fn insert(&mut self, a: Action) {
        self.0 |= a.bit();
    }
    pub fn iter(self) -> impl Iterator<Item = Action> {
        Action::ALL.into_iter().filter(move |a| self.has(*a))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Button {
    pub action: Action,
    /// Centre, as a fraction of the screen width and height.
    pub x: f32,
    pub y: f32,
    /// Side length as a fraction of the screen height.
    pub size: f32,
    /// A held action latches on each press instead of following the finger.
    pub toggle: bool,
    /// For [`Action::Key`]: the key or mouse button, an index into [`KEYS`].
    pub key: Option<u8>,
    /// The player's own name for it, drawn instead of the action's.
    pub label: Option<Label>,
}

pub const MIN_SIZE: f32 = 0.08;
pub const MAX_SIZE: f32 = 0.48;
pub const MAX_BUTTONS: usize = 24;

/// The keys and mouse buttons a button can be bound to: `(name, label on the button)`. The names are
/// winit's `KeyCode` variants; the three mouse buttons are `MouseLeft`, `MouseRight` and `MouseMiddle`.
pub const KEYS: &[(&str, &str)] = &[
    ("KeyW", "W"),
    ("KeyA", "A"),
    ("KeyS", "S"),
    ("KeyD", "D"),
    ("KeyQ", "Q"),
    ("KeyE", "E"),
    ("KeyR", "R"),
    ("KeyT", "T"),
    ("KeyY", "Y"),
    ("KeyU", "U"),
    ("KeyI", "I"),
    ("KeyO", "O"),
    ("KeyP", "P"),
    ("KeyF", "F"),
    ("KeyG", "G"),
    ("KeyH", "H"),
    ("KeyJ", "J"),
    ("KeyK", "K"),
    ("KeyL", "L"),
    ("KeyZ", "Z"),
    ("KeyX", "X"),
    ("KeyC", "C"),
    ("KeyV", "V"),
    ("KeyB", "B"),
    ("KeyN", "N"),
    ("KeyM", "M"),
    ("Digit1", "1"),
    ("Digit2", "2"),
    ("Digit3", "3"),
    ("Digit4", "4"),
    ("Digit5", "5"),
    ("Digit6", "6"),
    ("Digit7", "7"),
    ("Digit8", "8"),
    ("Digit9", "9"),
    ("Digit0", "0"),
    ("F1", "F1"),
    ("F2", "F2"),
    ("F3", "F3"),
    ("F4", "F4"),
    ("F5", "F5"),
    ("F6", "F6"),
    ("F7", "F7"),
    ("F8", "F8"),
    ("F9", "F9"),
    ("F10", "F10"),
    ("F11", "F11"),
    ("F12", "F12"),
    ("Escape", "Esc"),
    ("Tab", "Tab"),
    ("Enter", "Enter"),
    ("Space", "Space"),
    ("ShiftLeft", "Shift"),
    ("ControlLeft", "Ctrl"),
    ("AltLeft", "Alt"),
    ("Backspace", "Bksp"),
    ("ArrowUp", "Up"),
    ("ArrowDown", "Down"),
    ("ArrowLeft", "Left"),
    ("ArrowRight", "Right"),
    ("Slash", "/"),
    ("Backquote", "`"),
    ("Minus", "-"),
    ("Equal", "="),
    ("Comma", ","),
    ("Period", "."),
    ("Semicolon", ";"),
    ("Quote", "'"),
    ("BracketLeft", "["),
    ("BracketRight", "]"),
    ("Backslash", "\\"),
    ("MouseLeft", "LMB"),
    ("MouseRight", "RMB"),
    ("MouseMiddle", "MMB"),
];

/// A button's own name: up to 12 bytes of text.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Label {
    len: u8,
    buf: [u8; 12],
}

impl Label {
    /// `text` trimmed, cut to 12 bytes at a character boundary; None when it is empty.
    pub fn new(text: &str) -> Option<Label> {
        let text = text.trim();
        let mut end = text.len().min(12);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let text = &text[..end];
        if text.is_empty() {
            return None;
        }
        let mut buf = [0u8; 12];
        buf[..text.len()].copy_from_slice(text.as_bytes());
        Some(Label {
            len: text.len() as u8,
            buf,
        })
    }
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("")
    }
}

impl std::fmt::Debug for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

/// A set of [`KEYS`] by index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeySet(u128);

impl KeySet {
    pub fn has(self, key: u8) -> bool {
        key < 128 && self.0 & (1u128 << key) != 0
    }
    pub fn insert(&mut self, key: u8) {
        if key < 128 {
            self.0 |= 1u128 << key;
        }
    }
    pub fn flip(&mut self, key: u8) {
        if key < 128 {
            self.0 ^= 1u128 << key;
        }
    }
    pub fn iter(self) -> impl Iterator<Item = u8> {
        (0..KEYS.len() as u8).filter(move |k| self.has(*k))
    }
}

pub fn key_index(name: &str) -> Option<u8> {
    KEYS.iter().position(|(n, _)| *n == name).map(|i| i as u8)
}

/// The first line of a saved layout.
const HEADER: &str = "# horizon-touch-layout 2";

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub buttons: Vec<Button>,
}

impl Default for Layout {
    fn default() -> Self {
        let act = |action, x, y| Button {
            action,
            x,
            y,
            size: 0.13,
            toggle: false,
            key: None,
            label: None,
        };
        let key = |name: &str, x, y| Button {
            action: Action::Key,
            x,
            y,
            size: 0.13,
            toggle: false,
            key: key_index(name),
            label: None,
        };
        Layout {
            buttons: vec![
                // A cross for walking, left.
                key("KeyW", 0.13, 0.58),
                key("KeyA", 0.055, 0.74),
                key("KeyS", 0.13, 0.90),
                key("KeyD", 0.205, 0.74),
                // Jump and sneak, right.
                act(Action::Jump, 0.90, 0.74),
                act(Action::Sneak, 0.90, 0.54),
                act(Action::Sprint, 0.78, 0.82),
                // The game's own menus, top left.
                act(Action::Pause, 0.045, 0.13),
                act(Action::Inventory, 0.12, 0.13),
                act(Action::Chat, 0.195, 0.13),
            ],
        }
    }
}

impl Layout {
    /// Whether movement is a stick (rather than the W/A/S/D cross).
    pub fn has_stick(&self) -> bool {
        self.buttons.iter().any(|b| b.action == Action::Stick)
    }

    /// Movement as a fixed stick (true) or as the W/A/S/D cross (false), keeping every other button.
    pub fn set_movement_stick(&mut self, stick: bool) {
        let walking = |b: &Button| {
            b.action == Action::Key
                && ["KeyW", "KeyA", "KeyS", "KeyD"]
                    .iter()
                    .any(|n| b.key == key_index(n))
        };
        self.buttons
            .retain(|b| b.action != Action::Stick && !walking(b));
        let d = Layout::default();
        if stick {
            self.buttons.insert(
                0,
                Button {
                    action: Action::Stick,
                    x: 0.14,
                    y: 0.72,
                    size: 0.30,
                    toggle: false,
                    key: None,
                    label: None,
                },
            );
        } else {
            let cross: Vec<Button> = d.buttons.iter().filter(|b| walking(b)).copied().collect();
            for (i, b) in cross.into_iter().enumerate() {
                self.buttons.insert(i, b);
            }
        }
    }

    pub fn serialize(&self) -> String {
        let mut out = String::from(HEADER);
        out.push('\n');
        for b in &self.buttons {
            out.push_str(&format!(
                "{} {:.4} {:.4} {:.4} {} {} {}\n",
                b.action.name(),
                b.x,
                b.y,
                b.size,
                u8::from(b.toggle),
                b.key
                    .and_then(|k| KEYS.get(usize::from(k)))
                    .map_or("-", |k| k.0),
                b.label.as_ref().map_or("", Label::as_str)
            ));
        }
        out
    }

    /// Reads a saved layout. Bad lines are skipped; an empty or unreadable
    /// file gives `None` so the caller keeps the default.
    pub fn parse(text: &str) -> Option<Layout> {
        // A layout from before the header (and the walking buttons) is dropped for the new default.
        if !text.trim().is_empty() && text.lines().next().map(str::trim) != Some(HEADER) {
            return None;
        }
        let mut buttons = Vec::new();
        for line in text.lines() {
            // action x y size toggle key label...: the label is the rest of the line.
            let mut it = line.trim_end().splitn(7, ' ');
            let (Some(a), Some(x), Some(y), Some(sz)) =
                (it.next(), it.next(), it.next(), it.next())
            else {
                continue;
            };
            let (Some(action), Ok(x), Ok(y), Ok(size)) = (
                Action::from_name(a),
                x.parse::<f32>(),
                y.parse::<f32>(),
                sz.parse::<f32>(),
            ) else {
                continue;
            };
            if !(x.is_finite() && y.is_finite() && size.is_finite()) {
                continue;
            }
            let toggle = it.next() == Some("1");
            let key = it.next().and_then(key_index);
            let label = it.next().and_then(Label::new);
            buttons.push(Button {
                action,
                x: x.clamp(0.0, 1.0),
                y: y.clamp(0.0, 1.0),
                size: size.clamp(MIN_SIZE, MAX_SIZE),
                toggle,
                key: key.filter(|_| action == Action::Key),
                label,
            });
            if buttons.len() == MAX_BUTTONS {
                break;
            }
        }
        // A deliberately empty layout is written as an empty file, so only
        // a file with no recognisable lines *and* some text is rejected.
        if buttons.is_empty() && !text.trim().is_empty() {
            None
        } else {
            Some(Layout { buttons })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_keys_and_toggles() {
        let mut l = Layout::default();
        l.buttons[0].toggle = true;
        let n = l.buttons.len();
        l.buttons.push(Button {
            action: Action::Keyboard,
            x: 0.5,
            y: 0.25,
            size: 0.2,
            toggle: false,
            key: None,
            label: Label::new("My chat"),
        });
        l.buttons.push(Button {
            action: Action::Key,
            x: 0.3,
            y: 0.4,
            size: 0.1,
            toggle: true,
            key: key_index("F5"),
            label: None,
        });
        let back = Layout::parse(&l.serialize()).unwrap();
        assert_eq!(back.buttons.len(), l.buttons.len());
        assert!(back.buttons[0].toggle);
        assert_eq!(back.buttons[0].key, key_index("KeyW"));
        assert_eq!(back.buttons[n].action, Action::Keyboard);
        assert_eq!(
            back.buttons[n].label.as_ref().map(Label::as_str),
            Some("My chat")
        );
        assert!((back.buttons[n].size - 0.2).abs() < 1e-3);
        assert_eq!(back.buttons[n + 1].key, key_index("F5"));
        assert!(back.buttons[n + 1].toggle);
    }

    #[test]
    fn the_default_has_walking_keys_jump_and_pause() {
        let l = Layout::default();
        let keys: Vec<_> = l.buttons.iter().filter_map(|b| b.key).collect();
        for k in ["KeyW", "KeyA", "KeyS", "KeyD"] {
            assert!(keys.contains(&key_index(k).unwrap()), "{k}");
        }
        assert!(l.buttons.iter().any(|b| b.action == Action::Jump));
        assert!(l.buttons.iter().any(|b| b.action == Action::Pause));
    }

    #[test]
    fn labels_are_trimmed_and_cut_at_a_character_boundary() {
        assert!(Label::new("   ").is_none());
        assert_eq!(Label::new("  Fire ").unwrap().as_str(), "Fire");
        assert_eq!(
            Label::new("abcdefghijklmnop").unwrap().as_str(),
            "abcdefghijkl"
        );
        // 'é' is two bytes: the cut never splits it.
        assert_eq!(Label::new("aaaaaaaaaaaéé").unwrap().as_str(), "aaaaaaaaaaa");
    }

    #[test]
    fn the_movement_style_swaps_the_cross_for_a_stick_and_back() {
        let mut l = Layout::default();
        assert!(!l.has_stick());
        let others = l.buttons.iter().filter(|b| b.key.is_none()).count();
        l.set_movement_stick(true);
        assert!(l.has_stick());
        assert!(l.buttons.iter().all(|b| b.key != key_index("KeyW")));
        assert_eq!(
            l.buttons
                .iter()
                .filter(|b| b.key.is_none() && b.action != Action::Stick)
                .count(),
            others
        );
        l.set_movement_stick(false);
        assert!(!l.has_stick());
        assert!(l.buttons.iter().any(|b| b.key == key_index("KeyW")));
        assert!(l.buttons.iter().any(|b| b.key == key_index("KeyD")));
    }

    #[test]
    fn an_old_layout_without_the_header_is_dropped() {
        assert!(Layout::parse("jump 0.9 0.7 0.15 0\nsneak 0.9 0.5 0.15 0\n").is_none());
    }

    #[test]
    fn parse_skips_bad_lines_and_clamps() {
        let l = Layout::parse(&format!(
            "{HEADER}\njunk\njump 2.0 -1 9 0\nnope 0 0 0.1 0\nsneak nan 0 0.1\n"
        ))
        .unwrap();
        assert_eq!(l.buttons.len(), 1);
        assert_eq!(l.buttons[0].x, 1.0);
        assert_eq!(l.buttons[0].y, 0.0);
        assert_eq!(l.buttons[0].size, MAX_SIZE);
    }

    #[test]
    fn garbage_is_rejected_empty_is_allowed() {
        assert!(Layout::parse("garbage only").is_none());
        assert_eq!(Layout::parse("").unwrap().buttons.len(), 0);
    }

    #[test]
    fn key_names_are_unique_and_the_set_works() {
        let mut names: Vec<_> = KEYS.iter().map(|k| k.0).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), KEYS.len());
        assert!(KEYS.len() <= 128);
        let mut set = KeySet::default();
        set.insert(3);
        set.flip(70 % KEYS.len() as u8);
        assert!(set.has(3));
        set.flip(3);
        assert!(!set.has(3));
    }
}
