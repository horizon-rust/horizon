//! A minimum launcher version (#18631). `min_launcher` in links.json on main names the oldest
//! launcher that may stay in use, for a release known to be broken. Every launcher already
//! downloads and checks a newer launcher as soon as an update check finds one; one older than
//! `min_launcher` that replaces itself then installs it and restarts without waiting for Update.
//! While a game it started runs it waits, and installs when the window closes
//! (`update_handoff`). A failed update says why, as any other does, and the launcher goes on.
use crate::backend;

/// The version `min_launcher` names: `0.2.80`, or the tag `launcher@0.2.80`. None for anything
/// else, an empty value included.
pub fn version_in(text: &str) -> Option<String> {
    let text = text.trim();
    let version = backend::launcher_version(text).unwrap_or(text);
    version
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| version.to_owned())
}

/// Whether launcher `version` is older than `min`.
pub fn below(version: &str, min: &str) -> bool {
    let tag = |v: &str| format!("{}{v}", backend::LAUNCHER_TAG_PREFIX);
    backend::compare_versions(&tag(version), &tag(min)).is_lt()
}

/// Whether launcher `version` installs its staged update now (`ready`) and restarts: it is below
/// `min`, it replaces itself, and no game it started runs or is starting (`busy`).
pub fn installs_now(
    version: &str,
    min: Option<&str>,
    self_update_allowed: bool,
    ready: bool,
    busy: bool,
) -> bool {
    self_update_allowed && ready && !busy && min.is_some_and(|min| below(version, min))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_minimum_is_a_version_or_a_launcher_tag() {
        assert_eq!(version_in("0.2.80").as_deref(), Some("0.2.80"));
        assert_eq!(version_in(" launcher@0.2.80 ").as_deref(), Some("0.2.80"));
        assert_eq!(version_in(""), None);
        assert_eq!(version_in("latest"), None);
        assert_eq!(version_in("v2026.10.1"), None);
    }

    #[test]
    fn versions_compare_as_releases() {
        assert!(below("0.2.79", "0.2.80"));
        assert!(below("0.2.9", "0.2.10"));
        assert!(below("0.2.80-rc.1", "0.2.80"));
        assert!(!below("0.2.80", "0.2.80"));
        assert!(!below("0.2.81", "0.2.80"));
        assert!(!below("0.3.0", "0.2.80"));
    }

    #[test]
    fn only_a_launcher_below_the_minimum_with_nothing_running_installs_now() {
        let min = Some("0.2.80");
        assert!(installs_now("0.2.79", min, true, true, false));
        // At or above the minimum, or no minimum: Update or closing the window, as always.
        assert!(!installs_now("0.2.80", min, true, true, false));
        assert!(!installs_now("0.2.79", None, true, true, false));
        // A development build or HORIZON_LAUNCHER_NO_SELF_UPDATE never replaces itself.
        assert!(!installs_now("0.2.79", min, false, true, false));
        // Nothing downloaded and checked yet (or that failed): nothing to install.
        assert!(!installs_now("0.2.79", min, true, false, false));
        // A game runs: it goes in when the window closes.
        assert!(!installs_now("0.2.79", min, true, true, true));
    }
}
