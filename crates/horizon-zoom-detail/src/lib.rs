//! The zoom LOD detail extra (`lodZoomDetail` in `horizon-extras.txt`): far terrain a zoom
//! magnifies is planned finer, so a spyglass does not show the coarse cells it was given for an
//! unzoomed screen (`lod::ZoomAim` in the engine).
//!
//! A Horizon crate (docs/layers.md): the engine asks the hooks whether to aim far terrain at a
//! zoom ([`HOOKS`], `far_terrain_zoom_detail`) and still does nothing unless far terrain is on and
//! the view is zoomed. Its setting keeps its line ([`SETTING`]); it has no row, as before: off
//! until a measurement shows the extra tiles land while the zoom is still held.
//! `horizon-official` registers both.
use horizon_platform::hooks::{Hooks, seat};
use horizon_platform::settings::Store;
use std::sync::Mutex;

/// The setting's key in `horizon-extras.txt`, as the engine's extras wrote it.
const KEY: &str = "lodZoomDetail";

/// The switch, one per splitscreen seat (`horizon_platform::hooks::seat`), as each player's
/// extras file had its own before.
pub static HOOKS: ZoomDetail = ZoomDetail {
    seats: Mutex::new(Vec::new()),
};

pub struct ZoomDetail {
    seats: Mutex<Vec<bool>>,
}

impl ZoomDetail {
    /// The `lodZoomDetail` setting of the player whose turn it is.
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

impl Hooks for ZoomDetail {
    fn far_terrain_zoom_detail(&self) -> bool {
        self.enabled()
    }
}

/// `lodZoomDetail` in `horizon-extras.txt`, the player's own file in splitscreen: the game loads
/// and saves it on the player's turn, so it is that seat's.
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

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_hooks::engine::set_seat;

    /// The player's line loads into its seat's setting, the hook answers it, and the line saved
    /// says what it is; the other seats keep theirs.
    #[test]
    fn the_setting_loads_answers_and_saves_per_seat() {
        set_seat(1);
        assert!(SETTING.is_default() && SETTING.owns("lodZoomDetail") && !SETTING.owns("zoom"));
        assert!(!HOOKS.far_terrain_zoom_detail());
        SETTING.load("lodZoomDetail", "true");
        assert!(HOOKS.far_terrain_zoom_detail() && !SETTING.is_default());
        let mut saved = Vec::new();
        SETTING.save(&mut |key, value| saved.push(format!("{key}:{value}")));
        assert_eq!(saved, ["lodZoomDetail:true"]);
        set_seat(0);
        assert!(!HOOKS.enabled(), "P1 keeps its own");
        set_seat(1);
        // A value that isn't a switch changes nothing.
        SETTING.load("lodZoomDetail", "yes");
        assert!(HOOKS.enabled());
        SETTING.load("lodZoomDetail", "false");
        assert!(SETTING.is_default());
        // Seat 1 switches to a profile whose file has no `lodZoomDetail`: the load starts from
        // the default, and seat 0 keeps its own.
        SETTING.load("lodZoomDetail", "true");
        set_seat(0);
        SETTING.load("lodZoomDetail", "true");
        set_seat(1);
        SETTING.defaults();
        assert!(!HOOKS.far_terrain_zoom_detail() && SETTING.is_default());
        set_seat(0);
        assert!(HOOKS.enabled(), "P1's stays");
        SETTING.defaults();
    }
}
