//! A player's turn. Several `Game`s can run on one thread, one after another (splitscreen is the
//! first such host); the state a `Game` keeps in thread-locals while it runs is swapped in for
//! its turn and out again after it (spec #14206, step 2).
use horizon_client::widgets;
use std::{cell::RefCell, path::PathBuf};

thread_local! {
    /// The options file of the player whose turn it is (none for P1, whose is the game's own):
    /// every player runs on this thread, one after another.
    static OPTIONS: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// The `options.txt` of the player whose turn it is; `None` for P1.
pub(super) fn options_path() -> Option<PathBuf> {
    OPTIONS.with_borrow(Clone::clone)
}

/// Makes `path` the options file of the turn running now (`None`: P1's own).
pub(super) fn set_options(path: Option<PathBuf>) {
    OPTIONS.set(path);
}

/// What one `Game` brings to its turn and takes away again.
pub(super) struct Turn {
    /// Its buttons and focus targets while another player's game runs.
    widgets: widgets::Saved,
    /// Its own options file, if it has one (none for P1).
    pub(super) options: Option<PathBuf>,
    /// Its seat, from 1 (`horizon_hooks::seat()` is this minus 1 during its turn).
    pub(super) seat: usize,
}

impl Default for Turn {
    fn default() -> Self {
        Self {
            widgets: Default::default(),
            options: None,
            seat: 1,
        }
    }
}

impl super::Game<'_> {
    /// This game's turn is over: its buttons and focus targets, which screens record while
    /// drawing and the control socket and the controller's cursor read, are put away.
    pub(super) fn leave_turn(&mut self) {
        self.turn.widgets = widgets::take();
    }

    /// This game's GUI scale is the one in force.
    pub(super) fn set_turn_gui_scale(&self) {
        horizon_client::hud::GUI_SCALE.store(
            self.settings.gui_scale,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// This game's turn starts: its options file, its seat and its own buttons and focus
    /// targets are the ones in force.
    pub(super) fn run_turn(&mut self) {
        self.set_turn_gui_scale();
        widgets::put(std::mem::take(&mut self.turn.widgets));
        set_options(self.turn.options.clone());
        horizon_hooks::engine::set_seat(self.turn.seat - 1);
    }
}
