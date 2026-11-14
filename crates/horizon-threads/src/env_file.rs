//! Horizon's switches from a file: `horizon-env.txt`, beside the executable or in the folder the
//! game starts in, one `HORIZON_NAME=value` a line. A launcher that starts the game without its
//! caller's environment (a VR runtime relaunching it) loses every `HORIZON_*` switch set in a
//! script; the file gives them back. A variable already set in the environment wins.

/// The `HORIZON_*` assignments of `text`: blank lines and `#` comments (a whole line or a line's end) skipped, as are names that
/// aren't Horizon's (the file sets the game's switches, nothing else of the process).
fn parse(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .map(str::trim)
        .map(|line| line.split('#').next().unwrap_or(""))
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.trim(), value.trim()))
        .filter(|(name, _)| {
            name.len() > "HORIZON_".len()
                && name.starts_with("HORIZON_")
                && name
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        })
        .collect()
}

/// Sets the file's switches, before anything reads the environment or starts a thread. Returns
/// what it set, for the log once that opens.
pub fn apply() -> Vec<String> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("horizon-env.txt")));
    let here = std::path::PathBuf::from("horizon-env.txt");
    let mut set = Vec::new();
    for path in beside.into_iter().chain([here]) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (name, value) in parse(&text) {
            if std::env::var_os(name).is_some() {
                continue;
            }
            // SAFETY: called first in `main`, before any other thread exists.
            unsafe { std::env::set_var(name, value) };
            set.push(format!("{name}={value}"));
        }
    }
    set
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_horizon_switches_are_taken_from_the_file() {
        let text = "# VR\nHORIZON_RT_SKIP = 64\n\nPATH=/tmp\nHORIZON_=1\nHORIZON_xr=1\n  HORIZON_XR_VRS=0  \nnot a line\nHORIZON_RT_SHARP=35 # wide";
        assert_eq!(
            super::parse(text),
            [
                ("HORIZON_RT_SKIP", "64"),
                ("HORIZON_XR_VRS", "0"),
                ("HORIZON_RT_SHARP", "35"),
            ]
        );
    }
}
