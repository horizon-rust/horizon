//! Horizon's inventory extras (#332, `docs/extras.md`): Mouse Tweaks-style controls ([`tweaks`])
//! and sorting ([`sort`]), both off by default. They act on container screens through the
//! engine's container hooks (`horizon_platform::containers`), with ordinary slot clicks only, so
//! with both off every screen clicks exactly as vanilla's.
//!
//! Their settings keep their lines in `horizon-extras.txt` ([`SETTINGS`]) and their rows in the
//! Extras page's Inventory group ([`SECTION`]). `horizon-official` registers all three.
use horizon_platform::{
    containers::{Container, SlotId},
    hooks::{Hooks, seat},
    hud::Canvas,
    pages::{Act, Label, Page, Place, Row},
    settings::Store,
};
use std::sync::Mutex;

mod plan;
pub mod sort;
pub mod tweaks;

/// The settings, as `horizon-extras.txt` names them.
#[derive(Clone, Debug, PartialEq)]
struct Options {
    /// `mouseTweaks`: the drags and the wheel.
    mouse_tweaks: bool,
    /// `mouseTweaksWheel`: the wheel's part of them.
    wheel: bool,
    /// `sort`: the sort buttons.
    sort: bool,
    /// `sortKey`: the key that sorts the part under the cursor, `key.keyboard.unknown` for none.
    sort_key: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mouse_tweaks: false,
            wheel: true,
            sort: false,
            sort_key: "key.keyboard.unknown".into(),
        }
    }
}

#[derive(Default)]
struct Seat {
    options: Options,
    tweaks: tweaks::Tweaks,
}

/// The extras on the container hooks. Splitscreen players share the hooks, so each seat
/// (`horizon_platform::hooks::seat`) has its own settings and drag, as each player's game had.
pub static HOOKS: InventoryTweaks = InventoryTweaks {
    seats: Mutex::new(Vec::new()),
};

pub struct InventoryTweaks {
    seats: Mutex<Vec<Seat>>,
}

impl InventoryTweaks {
    fn with<T>(&self, f: impl FnOnce(&mut Seat) -> T) -> T {
        let mut seats = self.seats.lock().unwrap_or_else(|e| e.into_inner());
        let i = seat();
        if seats.len() <= i {
            seats.resize_with(i + 1, Seat::default);
        }
        f(&mut seats[i])
    }
    fn options(&self) -> Options {
        self.with(|s| s.options.clone())
    }
}

impl Hooks for InventoryTweaks {
    fn container_press(&self, c: &mut dyn Container, slot: Option<SlotId>, right: bool) -> bool {
        self.with(|s| {
            let on = s.options.mouse_tweaks;
            (!right && s.options.sort && sort::press(c)) || s.tweaks.press(on, c, slot, right)
        })
    }
    fn container_drag(&self, c: &mut dyn Container, slot: Option<SlotId>) -> bool {
        self.with(|s| s.tweaks.drag(s.options.mouse_tweaks, c, slot))
    }
    fn container_release(&self, _c: &mut dyn Container) {
        self.with(|s| s.tweaks.release());
    }
    fn container_wheel(&self, c: &mut dyn Container, slot: Option<SlotId>, steps: f32) -> bool {
        self.with(|s| {
            let on = s.options.mouse_tweaks && s.options.wheel;
            s.tweaks.wheel(on, c, slot, steps)
        })
    }
    fn container_key(&self, c: &mut dyn Container, slot: Option<SlotId>, key: &str) -> bool {
        let options = self.options();
        options.sort && options.sort_key == key && sort::key(c, slot)
    }
    fn container_draw(&self, c: &dyn Container, canvas: &mut dyn Canvas) {
        if self.options().sort {
            sort::draw(c, canvas);
        }
    }
}

const MOUSE_TWEAKS: &str = "mouseTweaks";
const WHEEL: &str = "mouseTweaksWheel";
const SORT: &str = "sort";
const SORT_KEY: &str = "sortKey";

/// Their lines in `horizon-extras.txt`, the player's own file in splitscreen: the game loads and
/// saves them on the player's turn, so they are that seat's.
pub static SETTINGS: Settings = Settings;

pub struct Settings;

impl Store for Settings {
    fn file(&self) -> &'static str {
        "horizon-extras.txt"
    }
    fn owns(&self, key: &str) -> bool {
        matches!(key, MOUSE_TWEAKS | WHEEL | SORT | SORT_KEY)
    }
    fn load(&self, key: &str, value: &str) {
        HOOKS.with(|s| {
            let o = &mut s.options;
            let flag = value.parse::<bool>();
            match key {
                MOUSE_TWEAKS => o.mouse_tweaks = flag.unwrap_or(o.mouse_tweaks),
                WHEEL => o.wheel = flag.unwrap_or(o.wheel),
                SORT => o.sort = flag.unwrap_or(o.sort),
                SORT_KEY if value.starts_with("key.") => o.sort_key = value.to_owned(),
                _ => {}
            }
        });
    }
    fn save(&self, line: &mut dyn FnMut(&str, &str)) {
        let o = HOOKS.options();
        let text = |on: bool| if on { "true" } else { "false" };
        line(MOUSE_TWEAKS, text(o.mouse_tweaks));
        line(WHEEL, text(o.wheel));
        line(SORT, text(o.sort));
        line(SORT_KEY, &o.sort_key);
    }
    fn is_default(&self) -> bool {
        HOOKS.options() == Options::default()
    }
}

/// Their rows in the Extras page's Inventory group, where they always were.
pub static SECTION: Section = Section;

pub struct Section;

/// The rows, in order: (label, tooltip).
const ROWS: [(&str, &str); 4] = [
    (
        "horizon.extras.mouse_tweaks",
        "horizon.extras.mouse_tweaks.tooltip",
    ),
    (
        "horizon.extras.mouse_tweaks_wheel",
        "horizon.extras.mouse_tweaks_wheel.tooltip",
    ),
    ("horizon.extras.sort", "horizon.extras.sort.tooltip"),
    ("horizon.extras.sort_key", "horizon.extras.sort_key.tooltip"),
];

impl Page for Section {
    fn id(&self) -> &'static str {
        "horizon-inventory-tweaks.extras"
    }
    fn title(&self) -> &'static str {
        "horizon.extras.header.inventory"
    }
    fn place(&self) -> Place {
        Place::Extras
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        let o = HOOKS.options();
        let [tweaks, wheel, sort, key] = ROWS.map(|(label, _)| label);
        rows.push(Row::Toggle {
            label: tweaks,
            on: o.mouse_tweaks,
        });
        rows.push(Row::Toggle {
            label: wheel,
            on: o.wheel,
        });
        rows.push(Row::Toggle {
            label: sort,
            on: o.sort,
        });
        rows.push(Row::Key {
            label: key,
            key: o.sort_key,
        });
    }
    fn act(&self, row: usize, act: Act) {
        HOOKS.with(|s| {
            let o = &mut s.options;
            match (row, act) {
                (0, Act::Press | Act::PressBack) => o.mouse_tweaks = !o.mouse_tweaks,
                (1, Act::Press | Act::PressBack) => o.wheel = !o.wheel,
                (2, Act::Press | Act::PressBack) => o.sort = !o.sort,
                (3, Act::Bind(key)) => o.sort_key = key,
                _ => {}
            }
        });
    }
    fn tooltip(&self, row: usize) -> Option<Label> {
        ROWS.get(row).map(|&(_, about)| Label::Key(about))
    }
    /// The wheel and the key do nothing while their switch is off: greyed out.
    fn live(&self, row: usize) -> bool {
        let o = HOOKS.options();
        match row {
            1 => o.mouse_tweaks,
            3 => o.sort,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests;
