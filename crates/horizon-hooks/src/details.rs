//! What the game says about the machine and its settings, for a bug report a crate above the
//! engine writes (docs/layers.md, "Platform registries"): the graphics adapter, its backend and
//! driver, the render settings, the resource packs and the native mods, as `(label, value)`
//! lines the engine keeps current. They never hold a world's, a server's or a player's name.
use std::sync::Mutex;

static DETAILS: Mutex<Vec<(&'static str, String)>> = Mutex::new(Vec::new());

/// The lines, in the order the engine first set them.
pub fn all() -> Vec<(&'static str, String)> {
    DETAILS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The engine's side. Only the engine calls it.
pub mod engine {
    use super::DETAILS;

    /// Sets line `key`, replacing its last value.
    pub fn set(key: &'static str, value: String) {
        let mut details = DETAILS.lock().unwrap_or_else(|e| e.into_inner());
        match details.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => details.push((key, value)),
        }
    }
}
