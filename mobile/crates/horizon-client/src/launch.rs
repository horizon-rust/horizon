//! How the mobile app starts: the launcher screen first, the game when Play asked for it.
//! The two are separate event loops, and Android allows one per process, so going from one to
//! the other restarts the app, leaving a note in the app's folder.

/// A file in the app folder: its presence at start means "start the game" (written by the
/// launcher's Play, with the instance's name inside; removed when the game starts).
pub const START_MARKER: &str = "start-game";

/// The instance named in the marker, taking the marker away; None when the launcher should show.
pub fn take_start_request(folder: &std::path::Path) -> Option<String> {
    let path = folder.join(START_MARKER);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(text.trim().to_owned())
}

/// Asks for the game to start on the next launch, in `instance` (a folder name; "" for the default).
pub fn request_start(folder: &std::path::Path, instance: &str) -> std::io::Result<()> {
    std::fs::write(folder.join(START_MARKER), instance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_request_is_taken_once() {
        let dir = std::env::temp_dir().join(format!("hl-start-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(take_start_request(&dir), None);
        request_start(&dir, "Survival").unwrap();
        assert_eq!(take_start_request(&dir).as_deref(), Some("Survival"));
        assert_eq!(take_start_request(&dir), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
