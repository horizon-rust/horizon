//! The pause menu's Report Bugs screen (docs/telemetry.md, "Bug reports from the game"). The
//! player picks a kind and an area, writes a sentence and what they expected, reads the whole
//! issue as it will be posted, and Send posts it to the telemetry server's `POST /v1/report`,
//! which opens a public GitHub issue and answers with its link. It is not data collection:
//! nothing is kept on the machine or sent in the background, and clicking Send is the consent
//! for that one report.
//!
//! A Horizon crate: it plugs into the game through the platform's registries ([`PAGE`] at
//! `Place::ReportBugs`, [`HOOKS`], [`LANG`]), which the official executable (`horizon-official`)
//! registers (docs/layers.md, "How a Horizon crate plugs in"). Without it the pause menu's icon
//! opens vanilla's link.
//!
//! The technical part is what the game already collects: the build, the machine, the graphics
//! adapter and the render settings. A performance report also needs a measurement, which the game
//! does not take yet, so the screen does not send one.
#![forbid(unsafe_code)]

pub mod body;
mod page;
pub mod send;
pub mod technical;

use horizon_platform::hooks::{Hooks, Started};
use horizon_platform::system::{Machine, scrub::Scrubber};
use std::path::PathBuf;
use std::sync::OnceLock;

pub use page::{PAGE, Screen};

/// The screen's text, for `horizon_platform::lang::register`.
pub static LANG: &horizon_platform::lang::Files =
    &[("en_us", include_str!("../assets/lang/en_us.json"))];

/// The hooks, for `horizon_platform::hooks::register`: they tell the screen which build this is.
pub static HOOKS: Reporter = Reporter;

pub struct Reporter;

/// The build and the player's game folder, once the game has started.
static START: OnceLock<(String, PathBuf)> = OnceLock::new();
/// The machine, read once in the background after the start (some probes run programs).
static MACHINE: OnceLock<Machine> = OnceLock::new();

impl Hooks for Reporter {
    fn started(&self, game: &Started<'_>) {
        if START
            .set((game.version.to_owned(), game.game_dir.to_owned()))
            .is_err()
        {
            return;
        }
        let dir = game.game_dir.to_owned();
        let spawned = std::thread::Builder::new()
            .name("Report details".into())
            .spawn(move || {
                let _ = MACHINE.set(Machine::detect_in(Some(dir.as_path())));
            });
        if let Err(e) = spawned {
            log::info!("Horizon: the machine's details are read when a report needs them: {e}");
        }
    }
}

/// The technical part of a report now: the build, the machine (read here if the background read
/// has not finished) and the engine's details, scrubbed.
fn technical_now() -> body::Technical {
    let version = START
        .get()
        .map_or("unknown", |(version, _)| version.as_str());
    let machine = MACHINE.get().cloned().unwrap_or_else(Machine::detect);
    technical::gather(
        version,
        &machine,
        &horizon_platform::details::all(),
        &Scrubber::new(),
    )
}
