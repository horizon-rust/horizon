//! Whether automatic updates may change the explicit base selection.
use super::*;

/// Whether a version installed as an update becomes the selected one: when the player had none,
/// was on the newest installed Nightly (an older one picked on Versions stays), or on the newest
/// installed release (so a player of the release channel moves
/// to the Nightly at the first check, docs/release-flow.md). An older release picked by hand
/// stays, and so does an imported build (`build_import`) or a snapshot (installed by its tag
/// alone); an instance pinned to a version keeps it either way.
pub fn select_after_update(selected: Option<&str>, installed_before: &[String]) -> bool {
    match selected {
        None => true,
        Some(s) if crate::nightly::date_of(s).is_some() => installed_before
            .iter()
            .find(|t| crate::nightly::date_of(t).is_some())
            .is_none_or(|newest| newest == s),
        Some(s)
            if crate::build_import::is_imported(s)
                || crate::developer_install::uses_developer(s)
                || horizon_release_sign::Channel::of(s)
                    == Some(horizon_release_sign::Channel::Snapshot) =>
        {
            false
        }
        Some(s) => installed_before
            .iter()
            .find(|t| is_game_tag(t))
            .is_none_or(|newest| newest == s),
    }
}

/// Whether `tag`, installed only because an instance is pinned to it, becomes the selected one
/// too: when it is now the newest installed (`installed`) and the selection was following the
/// newest (`select_after_update` on `installed_before`). Without this, the selection would stay
/// on the build before it, which then reads as picked on Versions and no update moves it again.
/// An older build for an instance, or a selection a person picked, stays as it is.
pub fn select_after_instance_install(
    selected: Option<&str>,
    installed_before: &[String],
    installed: &[String],
    tag: &str,
) -> bool {
    installed.first().map(String::as_str) == Some(tag)
        && select_after_update(selected, installed_before)
}
