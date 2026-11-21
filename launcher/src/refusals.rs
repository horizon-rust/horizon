//! Plain words for what stops a game from starting or installing.
use crate::i18n::{tr, trf};
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// The game's exit code when it can't mark its folders as in use (`holder::acquire` in
/// horizon-mod-package).
const LEASES_REFUSED: i32 = 78;

/// Whether `command` runs Horizon's own game with nothing in front of it. Only then is its exit
/// code the game's: a Java instance, a wrapper or a Unix pre-launch script can end with 78 too.
pub(crate) fn runs_horizon_directly(command: &Command) -> bool {
    direct(
        crate::run_leases::is_horizon(command),
        command.get_program(),
    )
}

fn direct(horizon: bool, program: &OsStr) -> bool {
    // The installed client, or on Windows its staged copy: both keep the client's file name.
    horizon && Path::new(program).file_name() == Some(OsStr::new(crate::backend::CLIENT))
}

/// How a run that ended with `status` reads. A directly started game's lease refusal says
/// what to do.
pub fn exit_error(direct: bool, status: std::process::ExitStatus) -> String {
    exit_text(direct, status.code(), &status.to_string())
}

fn exit_text(direct: bool, code: Option<i32>, status: &str) -> String {
    // The game refuses before its log starts, so the launcher's own log is the one to send.
    if direct && code == Some(LEASES_REFUSED) {
        return tr("Horizon stopped before starting because its folders weren't marked as in use. Press Play again. If this repeats, open the Log tab, press Share log… and post the link on Discord.").into();
    }
    format!("Exited with {status}")
}

fn shown(path: &Path) -> String {
    crate::backend::plain(path.to_owned()).display().to_string()
}

/// Whether the data folder comes from the environment: Settings can't move it then
/// (`location_page::environment_blocked`).
fn data_from_environment() -> bool {
    ["HORIZON_LAUNCHER_HOME", "HORIZON_HOME"]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
}

/// A refused file system: at `folder` when the error knows it, against the data folder `data`.
pub(crate) fn unsupported_filesystem(folder: Option<&Path>, data: &Path) -> String {
    unsupported_text(folder, data, data_from_environment())
}

fn unsupported_text(folder: Option<&Path>, data: &Path, environment: bool) -> String {
    match folder {
        Some(folder) if folder == data && environment => trf(
            "Horizon can't use {folder}: its file system isn't supported. Set HORIZON_LAUNCHER_HOME to a folder on a local drive.",
            &[("folder", &shown(folder))],
        ),
        Some(folder) if folder == data => trf(
            "Horizon can't use {folder}: its file system isn't supported. Move the launcher's data to a local drive in Settings > About > Data folder.",
            &[("folder", &shown(folder))],
        ),
        Some(folder) => trf(
            "Horizon can't use {folder}: its file system isn't supported. It must be on a local drive.",
            &[("folder", &shown(folder))],
        ),
        None => tr("A folder Horizon uses is on a file system it doesn't support. Keep the launcher's data and game folders on a local drive.").into(),
    }
}

/// A refused entry at `path`: a link, or not the kind of entry expected there.
pub(crate) fn unexpected_entry(path: &Path, data: &Path) -> String {
    let link = std::fs::symlink_metadata(path).is_ok_and(|meta| crate::native_cache::linked(&meta));
    unexpected_text(path, data, link, data_from_environment())
}

fn unexpected_text(path: &Path, data: &Path, link: bool, environment: bool) -> String {
    match (path == data && link, environment) {
        (true, false) => trf(
            "{folder} is a link. Choose the real folder in Settings > About > Data folder.",
            &[("folder", &shown(path))],
        ),
        (true, true) => trf(
            "{folder} is a link. Set HORIZON_LAUNCHER_HOME to the real folder.",
            &[("folder", &shown(path))],
        ),
        (false, _) => trf(
            "Refusing native cache cleanup at an unexpected file or link: {path}",
            &[("path", &path.display().to_string())],
        ),
    }
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
    /// Nightlies since nightly-2026-11-18 ship only the Apple Silicon game.
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
            tr("This version has no Intel Mac build. A version you already have keeps working.")
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
            tr("This version has no Intel Mac build. A version you already have keeps working.")
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
    fn only_a_directly_started_horizon_game_owns_its_exit_code() {
        let client = OsStr::new(crate::backend::CLIENT);
        let installed = Path::new("versions")
            .join("nightly")
            .join(crate::backend::CLIENT);
        assert!(direct(true, client));
        assert!(direct(true, installed.as_os_str()));
        // A Java instance, a wrapper (gamemoderun) and the Unix pre-launch script (sh).
        assert!(!direct(false, client));
        assert!(!direct(true, OsStr::new("gamemoderun")));
        assert!(!direct(true, OsStr::new("/bin/sh")));

        let mut command = Command::new(&installed);
        assert!(!runs_horizon_directly(&command));
        crate::run_leases::mark(&mut command, Path::new("versions/nightly"));
        assert!(runs_horizon_directly(&command));
        let mut wrapped = Command::new("gamemoderun");
        crate::run_leases::mark(&mut wrapped, Path::new("versions/nightly"));
        assert!(!runs_horizon_directly(&wrapped));
    }

    #[test]
    fn only_the_games_code_78_reads_as_a_lease_refusal() {
        let refused = exit_text(true, Some(78), "exit code: 78");
        assert_eq!(
            refused,
            "Horizon stopped before starting because its folders weren't marked as in use. Press Play again. If this repeats, open the Log tab, press Share log… and post the link on Discord."
        );
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
        let game = Path::new("/mnt/nfs/survival");
        assert_eq!(
            unsupported_text(Some(data), data, false),
            "Horizon can't use /home/player/.local/share/horizon-launcher: its file system isn't supported. Move the launcher's data to a local drive in Settings > About > Data folder."
        );
        assert_eq!(
            unsupported_text(Some(data), data, true),
            "Horizon can't use /home/player/.local/share/horizon-launcher: its file system isn't supported. Set HORIZON_LAUNCHER_HOME to a folder on a local drive."
        );
        // An instance on NFS, with the data folder local: the instance is named, not the data.
        assert_eq!(
            unsupported_text(Some(game), data, false),
            "Horizon can't use /mnt/nfs/survival: its file system isn't supported. It must be on a local drive."
        );
        assert_eq!(
            unsupported_text(None, data, false),
            "A folder Horizon uses is on a file system it doesn't support. Keep the launcher's data and game folders on a local drive."
        );

        assert_eq!(
            unexpected_text(data, data, true, false),
            "/home/player/.local/share/horizon-launcher is a link. Choose the real folder in Settings > About > Data folder."
        );
        assert_eq!(
            unexpected_text(data, data, true, true),
            "/home/player/.local/share/horizon-launcher is a link. Set HORIZON_LAUNCHER_HOME to the real folder."
        );
        let generic = "Refusing native cache cleanup at an unexpected file or link: /home/player/.local/share/horizon-launcher";
        assert_eq!(unexpected_text(data, data, false, false), generic);
        let inner = data.join("native");
        assert!(unexpected_text(&inner, data, true, false).starts_with("Refusing native cache"));

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
    fn a_file_at_the_data_path_is_not_called_a_link() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("horizon-launcher");
        std::fs::write(&data, b"").unwrap();
        assert!(
            unexpected_entry(&data, &data).starts_with("Refusing native cache cleanup"),
            "{}",
            unexpected_entry(&data, &data)
        );
    }

    #[test]
    fn a_refused_file_system_keeps_the_first_folder_named() {
        use crate::native_cache::Error;
        let (game, data) = (Path::new("/mnt/nfs/survival"), Path::new("/data"));
        assert!(matches!(
            Error::UnsupportedFilesystem(None).on(game).on(data),
            Error::UnsupportedFilesystem(Some(folder)) if folder == game
        ));
        assert!(matches!(Error::Busy.on(game), Error::Busy));
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
            "This version has no Intel Mac build. A version you already have keeps working.";
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
