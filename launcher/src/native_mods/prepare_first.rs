//! A native build started from the launcher's window whose base version isn't prepared runs the
//! window's own Prepare for that version first, then builds (#18659). The command line keeps
//! its hint to run `horizon-launcher prepare` ([`crate::backend::prepare_hint`]).

use crate::backend::NotReady;
use crate::i18n::trf;

/// A window build waiting for its base's preparation, kept as it was started: it builds this
/// instance on this base, whatever is selected meanwhile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    /// The instance's id, and the version it played when the build was pressed.
    pub instance: String,
    pub plays: Option<String>,
    /// The resolved base, and the player's choice to back up worlds.
    pub base: String,
    pub back_up: bool,
    /// The version to prepare: the base, or a native base's own base ([`crate::backend::prepared_in`]).
    pub needs: String,
    /// The preparation running now was started by this build (its Stop stops it too).
    pub started: bool,
}

/// What the window is doing when a waiting build looks again.
pub struct Now<'a> {
    /// The waiting build's instance as it is now: `None` once removed, else the version it plays.
    pub plays: Option<Option<String>>,
    pub preparing: bool,
    /// The version an install is writing, if one runs.
    pub installing: Option<&'a str>,
    /// The preparation that just ended: the version it prepared and its error as the window
    /// shows it.
    pub ended: Option<(&'a str, Option<&'a str>)>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    Build,
    /// Start the window's Prepare for [`Waiting::needs`], then look again when it ends.
    Prepare,
    /// A preparation, or an install of the version to prepare, runs: look again when it ends.
    Wait,
    /// Drop the build, saying this.
    Stop(String),
}

/// What a window build does next, from when it is pressed until it builds or stops. `ready` is
/// [`crate::backend::ready`]. Only the preparation of the version it needs can fail it; any other
/// that ends only makes it look again. A base that must be reinstalled builds, so the build says
/// so as it always did.
pub fn next(waiting: &Waiting, now: &Now, ready: impl Fn(&str) -> Result<(), NotReady>) -> Next {
    if now.plays.as_ref() != Some(&waiting.plays) {
        return Next::Stop(trf(
            "The build waiting for {version} to be prepared stopped: its instance was removed or now plays another version.",
            &[("version", &waiting.needs)],
        ));
    }
    if let Some((tag, Some(error))) = now.ended {
        if tag == waiting.needs {
            return Next::Stop(error.to_owned());
        }
    }
    // An install's publish replaces the folder a preparation would write into.
    if now.preparing
        || now
            .installing
            .is_some_and(|t| t == waiting.needs || t == waiting.base)
    {
        return Next::Wait;
    }
    match ready(&waiting.base) {
        Err(NotReady::Unprepared(_)) => Next::Prepare,
        Ok(()) | Err(NotReady::Reinstall(_)) => Next::Build,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn waiting() -> Waiting {
        Waiting {
            instance: "a".into(),
            plays: Some("v1".into()),
            base: "v1".into(),
            back_up: true,
            needs: "v1".into(),
            started: false,
        }
    }

    fn now<'a>() -> Now<'a> {
        Now {
            plays: Some(Some("v1".into())),
            preparing: false,
            installing: None,
            ended: None,
        }
    }

    /// `v1` as the disk would answer, prepared once `prepared` is set.
    fn disk(prepared: &Cell<bool>) -> impl Fn(&str) -> Result<(), NotReady> + '_ {
        |tag| match prepared.get() {
            true => Ok(()),
            false => Err(NotReady::Unprepared(tag.into())),
        }
    }

    #[test]
    fn a_window_build_prepares_an_unprepared_base_first_then_builds() {
        let prepared = Cell::new(false);
        let w = waiting();
        assert_eq!(next(&w, &now(), disk(&prepared)), Next::Prepare);
        // Its Prepare runs: nothing starts twice.
        let running = Now {
            preparing: true,
            ..now()
        };
        assert_eq!(next(&w, &running, disk(&prepared)), Next::Wait);
        // It ended well and left v1 prepared: the build goes on.
        prepared.set(true);
        let ended = Now {
            ended: Some(("v1", None)),
            ..now()
        };
        assert_eq!(next(&w, &ended, disk(&prepared)), Next::Build);
        // Prepared from the start: it builds at once.
        assert_eq!(next(&w, &now(), disk(&prepared)), Next::Build);
        // A base to reinstall is no preparation: the build says so itself.
        let reinstall = |_: &str| Err(NotReady::Reinstall("reinstall v1".into()));
        assert_eq!(next(&w, &now(), reinstall), Next::Build);
    }

    #[test]
    fn a_failed_preparation_of_its_base_stops_the_build_with_its_message() {
        let prepared = Cell::new(false);
        let error = "Minecraft 26.3 was not found. Sign in with the account that owns it.";
        let ended = Now {
            ended: Some(("v1", Some(error))),
            ..now()
        };
        assert_eq!(
            next(&waiting(), &ended, disk(&prepared)),
            Next::Stop(error.into())
        );
    }

    #[test]
    fn another_versions_failed_preparation_does_not_stop_the_build() {
        let prepared = Cell::new(false);
        let w = waiting();
        // Prepare & play for v2 failed while this build waited: it prepares v1 itself.
        let ended = Now {
            ended: Some(("v2", Some("v2 failed"))),
            ..now()
        };
        assert_eq!(next(&w, &ended, disk(&prepared)), Next::Prepare);
        prepared.set(true);
        assert_eq!(next(&w, &ended, disk(&prepared)), Next::Build);
    }

    #[test]
    fn the_build_keeps_its_instance_or_stops() {
        let prepared = Cell::new(true);
        let w = waiting();
        // Another instance selected meanwhile changes nothing: `now` is this build's instance.
        assert_eq!(next(&w, &now(), disk(&prepared)), Next::Build);
        // Its instance now plays another version, or is gone: it stops and says so.
        for plays in [Some(Some("v2".to_string())), Some(None), None] {
            let changed = Now { plays, ..now() };
            assert!(matches!(next(&w, &changed, disk(&prepared)), Next::Stop(_)));
        }
    }

    #[test]
    fn an_install_of_the_version_to_prepare_is_waited_for() {
        let prepared = Cell::new(false);
        let w = waiting();
        let installing = Now {
            installing: Some("v1"),
            ..now()
        };
        assert_eq!(next(&w, &installing, disk(&prepared)), Next::Wait);
        let other = Now {
            installing: Some("v2"),
            ..now()
        };
        assert_eq!(next(&w, &other, disk(&prepared)), Next::Prepare);
    }

    #[test]
    fn the_command_line_keeps_its_words_and_the_window_never_shows_them() {
        let v = crate::backend::game_version("v0");
        assert_eq!(
            crate::backend::prepare_hint("v0"),
            format!(
                "v0 is not prepared yet. Prepare it first: `horizon-launcher prepare --version v0` uses your own Minecraft {v}, or downloads it from Mojang for the signed-in account that owns it."
            )
        );
        let failed = || crate::native_mods::Failed::NotPrepared("v0".into());
        assert_eq!(
            crate::native_mods::cli_error(failed()),
            crate::backend::prepare_hint("v0")
        );
        // The window and its build log read the failure's own words.
        assert!(!failed().to_string().contains("horizon-launcher"));
    }
}
