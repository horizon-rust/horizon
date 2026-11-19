//! A native build started from the launcher's window whose base version isn't prepared runs the
//! window's own Prepare for that version first, then builds (#18659). The command line keeps
//! its hint to run `horizon-launcher prepare` ([`crate::backend::prepare_hint`]).

use crate::backend::NotReady;

/// What a window build does before it builds.
#[derive(Debug, PartialEq, Eq)]
pub enum First {
    Build,
    /// Prepare this version (the base whose files the build reads), then start the build again.
    Prepare(String),
    /// A preparation is under way: start the build again once it ends.
    Wait,
}

/// `ready` is [`crate::backend::ready`] for the build's base; `preparing`, whether a preparation
/// is under way. A base that must be reinstalled builds, so the build says so as it always did.
pub fn first(ready: Result<(), NotReady>, preparing: bool) -> First {
    match ready {
        Err(NotReady::Unprepared(_)) if preparing => First::Wait,
        Err(NotReady::Unprepared(tag)) => First::Prepare(tag),
        Ok(()) | Err(NotReady::Reinstall(_)) => First::Build,
    }
}

/// A build waiting on a preparation that ended with `error` (Prepare's message as the window
/// shows it): starts again, or stops with that message.
pub fn after_prepare(error: Option<&str>) -> Result<(), String> {
    error.map_or(Ok(()), |error| Err(error.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_build_prepares_an_unprepared_base_first_then_builds() {
        let unprepared = || Err(NotReady::Unprepared("v1".into()));
        // Not prepared: Prepare runs for the base, then the build starts again and builds.
        assert_eq!(first(unprepared(), false), First::Prepare("v1".into()));
        assert_eq!(after_prepare(None), Ok(()));
        assert_eq!(first(Ok(()), false), First::Build);
        // Prepared: it builds at once, even while another version is being prepared.
        assert_eq!(first(Ok(()), true), First::Build);
        // A preparation already under way is waited for, not started twice.
        assert_eq!(first(unprepared(), true), First::Wait);
        // A base to reinstall is no preparation: the build says so itself.
        let reinstall = Err(NotReady::Reinstall("reinstall v1".into()));
        assert_eq!(first(reinstall, false), First::Build);
    }

    #[test]
    fn a_failed_preparation_stops_the_build_with_its_own_message() {
        let error = "Minecraft 1.21.11 was not found. Sign in with the account that owns it.";
        assert_eq!(after_prepare(Some(error)), Err(error.to_owned()));
    }
}
