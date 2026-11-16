//! Portable mode (#17377): an empty file named `portable` beside the launcher keeps everything the
//! launcher and the game write in a `data` folder next to it, so the whole install moves on a USB
//! drive or lives on another disk. Like a launcher unpacked from the ZIP, a portable one never
//! writes to the registry. Microsoft sign-ins stay in the data folder rather than the system's
//! keychain, sealed without this computer's identity so they open on any computer; secrets that
//! never go to a file of the launcher's (the Tailscale token) last for one run only.
//!
//! Turned on with [`apply`], first thing in the process: it names the data folder through the
//! environment overrides every part of the launcher, its helper processes and the game already
//! read (`HORIZON_LAUNCHER_HOME`, `HORIZON_HOME`), and marks the process with [`ENV`] so the helpers
//! it starts are portable too.

use std::path::{Path, PathBuf};

/// The file beside the launcher's executable that turns portable mode on. Its content is ignored.
pub const MARKER: &str = "portable";

/// The folder beside the launcher's executable that holds a portable install's data.
pub const DATA: &str = "data";

/// Set to `1` in a portable launcher's process, and so in every process it starts.
pub const ENV: &str = "HORIZON_LAUNCHER_PORTABLE";

/// Where a launcher at `exe` keeps its data when it is portable: `None` without the marker.
pub fn folder_for(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    dir.join(MARKER).is_file().then(|| dir.join(DATA))
}

/// The environment a portable install at `data` sets: the launcher's and the game's folders inside
/// it, and [`ENV`]. A folder the player already set with its variable stays theirs.
pub fn environment(data: &Path, set: impl Fn(&str) -> bool) -> Vec<(&'static str, PathBuf)> {
    let mut vars = vec![(ENV, PathBuf::from("1"))];
    for (name, folder) in [
        ("HORIZON_LAUNCHER_HOME", "launcher"),
        ("HORIZON_HOME", "game"),
    ] {
        if !set(name) {
            vars.push((name, data.join(folder)));
        }
    }
    vars
}

/// Turns portable mode on when the marker sits beside the running launcher. Call it first in
/// `main`, before any other thread starts and before any launcher path is read.
pub fn apply() {
    if is_on() {
        return;
    }
    // An AppImage runs a copy inside its read-only mount: the marker sits beside the AppImage.
    let exe = std::env::var_os("APPIMAGE")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok());
    let Some(data) = exe.and_then(|exe| folder_for(&exe)) else {
        return;
    };
    let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    for (name, value) in environment(&data, set) {
        std::env::set_var(name, value);
    }
}

/// Whether this process is a portable launcher's (or one it started).
pub fn is_on() -> bool {
    std::env::var_os(ENV).is_some_and(|v| v == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_beside_the_launcher_turns_portable_mode_on() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("horizon-launcher");
        assert_eq!(folder_for(&exe), None);
        // A folder of that name is not the marker.
        std::fs::create_dir(dir.path().join(MARKER)).unwrap();
        assert_eq!(folder_for(&exe), None);
        std::fs::remove_dir(dir.path().join(MARKER)).unwrap();
        std::fs::write(dir.path().join(MARKER), "").unwrap();
        assert_eq!(folder_for(&exe), Some(dir.path().join("data")));
    }

    #[test]
    fn portable_folders_sit_in_the_data_folder_unless_the_player_set_their_own() {
        let data = Path::new("/usb/horizon/data");
        let all = environment(data, |_| false);
        assert_eq!(
            all,
            vec![
                ("HORIZON_LAUNCHER_PORTABLE", PathBuf::from("1")),
                ("HORIZON_LAUNCHER_HOME", data.join("launcher")),
                ("HORIZON_HOME", data.join("game")),
            ]
        );
        let theirs = environment(data, |name| name == "HORIZON_HOME");
        assert_eq!(
            theirs,
            vec![
                ("HORIZON_LAUNCHER_PORTABLE", PathBuf::from("1")),
                ("HORIZON_LAUNCHER_HOME", data.join("launcher")),
            ]
        );
    }
}
