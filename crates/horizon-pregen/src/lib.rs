//! The chunk pregeneration extra (`docs/extras.md`): `/pregen` in the player's own single-player
//! and hosted LAN worlds.
//!
//! A Horizon crate (docs/layers.md): the engine keeps the command and its generator, and asks the
//! hooks whether to offer it ([`HOOKS`], `pregen_command`) when a world opens and after a setting
//! changes. Its setting, `pregen`, keeps its line in `horizon-extras.txt` ([`SETTING`]) and its
//! row in the Extras page's World group ([`SECTION`]). `horizon-official` registers all three.
use horizon_platform::hooks::{Hooks, seat};
use horizon_platform::pages::{Act, Label, Page, Place, Row};
use horizon_platform::settings::Store;
use std::sync::Mutex;

/// The setting's key in `horizon-extras.txt`, as the engine's extras wrote it.
const KEY: &str = "pregen";
const LABEL: &str = "horizon.extras.pregen";
const ABOUT: &str = "horizon.extras.pregen.tooltip";

/// The switch, one per splitscreen seat (`horizon_platform::hooks::seat`), as each player's
/// extras file had its own before. Only the window that owns the world changes its commands.
pub static HOOKS: Pregen = Pregen {
    seats: Mutex::new(Vec::new()),
};

pub struct Pregen {
    seats: Mutex<Vec<bool>>,
}

impl Pregen {
    /// The `pregen` setting of the player whose turn it is.
    pub fn set_enabled(&self, on: bool) {
        self.with(|s| *s = on);
    }
    /// Whether the player whose turn it is has it on.
    pub fn enabled(&self) -> bool {
        self.with(|s| *s)
    }
    fn with<T>(&self, f: impl FnOnce(&mut bool) -> T) -> T {
        let mut seats = self.seats.lock().unwrap_or_else(|e| e.into_inner());
        let i = seat();
        if seats.len() <= i {
            seats.resize(i + 1, false);
        }
        f(&mut seats[i])
    }
}

impl Hooks for Pregen {
    fn pregen_command(&self) -> bool {
        self.enabled()
    }
}

/// `pregen` in `horizon-extras.txt`, the player's own file in splitscreen: the game loads and
/// saves it on the player's turn, so it is that seat's.
pub static SETTING: Setting = Setting;

pub struct Setting;

impl Store for Setting {
    fn file(&self) -> &'static str {
        "horizon-extras.txt"
    }
    fn owns(&self, key: &str) -> bool {
        key == KEY
    }
    fn defaults(&self) {
        HOOKS.set_enabled(false);
    }
    fn load(&self, _key: &str, value: &str) {
        if let Ok(on) = value.parse() {
            HOOKS.set_enabled(on);
        }
    }
    fn save(&self, line: &mut dyn FnMut(&str, &str)) {
        line(KEY, if HOOKS.enabled() { "true" } else { "false" });
    }
    fn is_default(&self) -> bool {
        !HOOKS.enabled()
    }
}

/// Its row on the Extras page, in the World group it always had.
pub static SECTION: Section = Section;

pub struct Section;

impl Page for Section {
    fn id(&self) -> &'static str {
        "horizon-pregen.extras"
    }
    fn title(&self) -> &'static str {
        "horizon.extras.header.world"
    }
    fn place(&self) -> Place {
        Place::Extras
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Toggle {
            label: LABEL,
            on: HOOKS.enabled(),
        });
    }
    fn act(&self, row: usize, act: Act) {
        if row == 0 && matches!(act, Act::Press | Act::PressBack) {
            HOOKS.set_enabled(!HOOKS.enabled());
        }
    }
    fn tooltip(&self, _row: usize) -> Option<Label> {
        Some(Label::Key(ABOUT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_hooks::engine::set_seat;

    /// The player's line loads into its seat's setting, the hook and the row show it, a press
    /// flips it, and the line saved says what it is now; the other seats keep theirs.
    #[test]
    fn the_setting_loads_shows_flips_and_saves_per_seat() {
        set_seat(1);
        assert!(SETTING.is_default() && SETTING.owns("pregen") && !SETTING.owns("pregenKey"));
        assert!(!HOOKS.pregen_command());
        SETTING.load("pregen", "true");
        assert!(HOOKS.pregen_command());
        let mut rows = Vec::new();
        SECTION.rows(&mut rows);
        let shown = Row::Toggle {
            label: "horizon.extras.pregen",
            on: true,
        };
        assert_eq!(rows, [shown]);
        SECTION.act(0, Act::Press);
        assert!(!HOOKS.pregen_command());
        let mut saved = Vec::new();
        SETTING.save(&mut |key, value| saved.push(format!("{key}:{value}")));
        assert_eq!(saved, ["pregen:false"]);
        SETTING.load("pregen", "true");
        set_seat(0);
        assert!(!HOOKS.enabled(), "P1 keeps its own");
        set_seat(1);
        assert!(HOOKS.enabled() && !SETTING.is_default());
        // A value that isn't a switch changes nothing.
        SETTING.load("pregen", "yes");
        assert!(HOOKS.enabled());
        // Seat 1 switches to a profile whose file has no `pregen`: the load starts from the
        // default, and seat 0 keeps its own.
        set_seat(0);
        SETTING.load("pregen", "true");
        set_seat(1);
        SETTING.defaults();
        assert!(!HOOKS.pregen_command() && SETTING.is_default());
        set_seat(0);
        assert!(HOOKS.enabled(), "P1's stays");
        SETTING.defaults();
    }
}
