//! Plain words for what stops a game from starting or installing.
use crate::i18n::{tr, trf};
use std::path::Path;

/// The game's exit code when it can't mark its folders as in use (`holder::acquire` in
/// horizon-mod-package).
const LEASES_REFUSED: i32 = 78;

/// How a run that ended with `status` reads. The game's lease refusal says what to do.
pub fn exit_error(game: bool, status: std::process::ExitStatus) -> String {
    exit_text(game, status.code(), &status.to_string())
}

fn exit_text(game: bool, code: Option<i32>, status: &str) -> String {
    if game && code == Some(LEASES_REFUSED) {
        return tr("Horizon stopped before starting because its folders weren't marked as in use. Press Play again. If this repeats, report it on Discord with latest.log from the logs folder.").into();
    }
    format!("Exited with {status}")
}

fn shown(path: &Path) -> String {
    crate::backend::plain(path.to_owned()).display().to_string()
}

/// The lease registry lives in the launcher's data folder `data`: its file system decides.
pub(crate) fn unsupported_filesystem(data: &Path) -> String {
    trf(
        "Horizon can't use {folder}: its file system isn't supported. Move the launcher's data to a local drive in Settings > About > Data folder.",
        &[("folder", &shown(data))],
    )
}

/// What a refused link at `path` reads when it is the data folder itself.
pub(crate) fn linked_data(path: &Path, data: &Path) -> Option<String> {
    (path == data).then(|| {
        trf(
            "{folder} is a link. Choose the real folder in Settings > About > Data folder.",
            &[("folder", &shown(path))],
        )
    })
}

/// A Play refused because a folder couldn't be opened or written.
pub(crate) fn unwritable(error: &str, game: Option<&Path>, data: &Path) -> String {
    match game {
        Some(game) => trf(
            "{error}. Make sure your account can write to {game} and {data}, then try again.",
            &[
                ("error", error),
                ("game", &shown(game)),
                ("data", &shown(data)),
            ],
        ),
        None => trf(
            "{error}. Make sure your account can write to {data}, then try again.",
            &[("error", error), ("data", &shown(data))],
        ),
    }
}

/// Where a release without a build for this computer leaves the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Host {
    /// Nightlies since nightly-2026-11-13 ship only the Apple Silicon game.
    IntelMac,
    /// The x86_64 launcher under Rosetta, which looks for the Intel game that no longer ships.
    TranslatedMac,
    Other,
}

fn host_for(os: &str, arch: &str, translated: bool) -> Host {
    match (os, arch) {
        ("macos", "x86_64") if translated => Host::TranslatedMac,
        ("macos", "x86_64") => Host::IntelMac,
        _ => Host::Other,
    }
}

fn host() -> Host {
    static HOST: std::sync::OnceLock<Host> = std::sync::OnceLock::new();
    *HOST.get_or_init(|| host_for(std::env::consts::OS, std::env::consts::ARCH, translated()))
}

#[cfg(target_os = "macos")]
fn translated() -> bool {
    let mut value: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: the name is NUL-terminated, and value and size describe one writable c_int.
    let result = unsafe {
        libc::sysctlbyname(
            c"sysctl.proc_translated".as_ptr(),
            (&mut value as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    // Macs without Rosetta don't know the name: not translated.
    result == 0 && value == 1
}

#[cfg(not(target_os = "macos"))]
fn translated() -> bool {
    false
}

fn launcher_download() -> String {
    crate::backend::download_page(&crate::backend::Links::default().website)
}

/// The version list's line for a release with no build here, and the page a click opens.
pub fn no_build_card() -> (String, Option<String>) {
    card_for(host(), launcher_download())
}

fn card_for(host: Host, download: String) -> (String, Option<String>) {
    match host {
        Host::Other => (tr("No build for this platform").into(), None),
        Host::IntelMac => (
            tr("This Nightly has no Intel Mac build yet. A version you already have keeps working.")
                .into(),
            None,
        ),
        Host::TranslatedMac => (
            tr("You're running the Intel launcher on an Apple Silicon Mac. Download the Apple Silicon launcher.")
                .into(),
            Some(download),
        ),
    }
}

/// Why `tag` (or, with `None`, the newest release) can't be installed here.
pub fn no_build(tag: Option<&str>) -> String {
    no_build_for(host(), tag, &launcher_download())
}

fn no_build_for(host: Host, tag: Option<&str>, download: &str) -> String {
    match (host, tag) {
        (Host::Other, Some(tag)) => trf("{tag} has no build for this platform.", &[("tag", tag)]),
        (Host::Other, None) => tr("No release has a build for this platform yet.").into(),
        (Host::IntelMac, _) => {
            tr("This Nightly has no Intel Mac build yet. A version you already have keeps working.")
                .into()
        }
        (Host::TranslatedMac, _) => trf(
            "You're running the Intel launcher on an Apple Silicon Mac. Download the Apple Silicon launcher: {url}",
            &[("url", download)],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_games_code_78_reads_as_a_lease_refusal() {
        let refused = exit_text(true, Some(78), "exit code: 78");
        assert!(
            refused.starts_with("Horizon stopped before starting"),
            "{refused}"
        );
        assert!(refused.contains("Press Play again"), "{refused}");
        assert_eq!(
            exit_text(false, Some(78), "exit code: 78"),
            "Exited with exit code: 78"
        );
        assert_eq!(
            exit_text(true, Some(1), "exit code: 1"),
            "Exited with exit code: 1"
        );
        assert_eq!(exit_text(true, None, "signal: 9"), "Exited with signal: 9");
    }

    #[test]
    fn storage_refusals_name_the_folder() {
        let data = Path::new("/home/player/.local/share/horizon-launcher");
        assert_eq!(
            unsupported_filesystem(data),
            "Horizon can't use /home/player/.local/share/horizon-launcher: its file system isn't supported. Move the launcher's data to a local drive in Settings > About > Data folder."
        );
        assert_eq!(
            linked_data(data, data).as_deref(),
            Some("/home/player/.local/share/horizon-launcher is a link. Choose the real folder in Settings > About > Data folder.")
        );
        assert_eq!(linked_data(&data.join("native/old"), data), None);
        let game = Path::new("/home/player/games/survival");
        assert_eq!(
            unwritable("Permission denied", Some(game), data),
            "Permission denied. Make sure your account can write to /home/player/games/survival and /home/player/.local/share/horizon-launcher, then try again."
        );
        assert_eq!(
            unwritable("Permission denied", None, data),
            "Permission denied. Make sure your account can write to /home/player/.local/share/horizon-launcher, then try again."
        );
    }

    #[test]
    fn a_mac_without_its_build_says_which_launcher_or_version_to_use() {
        assert_eq!(host_for("macos", "x86_64", false), Host::IntelMac);
        assert_eq!(host_for("macos", "x86_64", true), Host::TranslatedMac);
        assert_eq!(host_for("macos", "aarch64", false), Host::Other);
        assert_eq!(host_for("linux", "x86_64", false), Host::Other);
        // The flag only exists for macOS processes.
        assert_eq!(host_for("windows", "x86_64", true), Host::Other);

        let url = "https://playhorizon.dev/docs/install";
        assert_eq!(
            no_build_for(Host::Other, Some("nightly-2026-11-20"), url),
            "nightly-2026-11-20 has no build for this platform."
        );
        assert_eq!(
            no_build_for(Host::Other, None, url),
            "No release has a build for this platform yet."
        );
        let intel =
            "This Nightly has no Intel Mac build yet. A version you already have keeps working.";
        assert_eq!(
            no_build_for(Host::IntelMac, Some("nightly-2026-11-20"), url),
            intel
        );
        assert_eq!(no_build_for(Host::IntelMac, None, url), intel);
        assert_eq!(
            no_build_for(Host::TranslatedMac, None, url),
            "You're running the Intel launcher on an Apple Silicon Mac. Download the Apple Silicon launcher: https://playhorizon.dev/docs/install"
        );

        assert_eq!(
            card_for(Host::Other, url.into()),
            ("No build for this platform".into(), None)
        );
        assert_eq!(card_for(Host::IntelMac, url.into()), (intel.into(), None));
        assert_eq!(
            card_for(Host::TranslatedMac, url.into()).1.as_deref(),
            Some(url)
        );
    }
}
