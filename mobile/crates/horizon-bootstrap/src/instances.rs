//! Instances: separate game folders (saves, options, mods, resource packs), as the desktop launcher
//! has. Each is a folder under `instances/`; the game runs with it as its working directory.

use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT: &str = "Default";
const SELECTED: &str = "selected.txt";

/// A folder name for `name`: trimmed, letters, digits, spaces, `-` and `_`, 1 to 24 characters.
pub fn sanitize(name: &str) -> Option<String> {
    let name: String = name
        .trim()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(24)
        .collect();
    let name = name.trim().to_owned();
    (!name.is_empty()).then_some(name)
}

pub fn dir(root: &Path, name: &str) -> PathBuf {
    root.join(name)
}

/// The instances under `root`, `Default` first (made if there is none), the rest by name.
pub fn list(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| sanitize(n).as_deref() == Some(n.as_str()))
        .collect();
    if !names.iter().any(|n| n == DEFAULT) {
        let _ = fs::create_dir_all(root.join(DEFAULT));
        names.push(DEFAULT.to_owned());
    }
    names.sort_by(|a, b| (a != DEFAULT, a.to_lowercase()).cmp(&(b != DEFAULT, b.to_lowercase())));
    names
}

pub fn create(root: &Path, name: &str) -> Result<String, String> {
    let name = sanitize(name).ok_or("Give the instance a name (letters, digits, spaces).")?;
    let path = root.join(&name);
    if path.exists() {
        return Err(format!("There is already an instance called {name}."));
    }
    fs::create_dir_all(&path).map_err(|e| format!("Could not make the instance: {e}"))?;
    Ok(name)
}

/// Deletes an instance and everything in it (its worlds too). `Default` stays.
pub fn delete(root: &Path, name: &str) -> Result<(), String> {
    if name == DEFAULT {
        return Err("The Default instance can't be deleted.".into());
    }
    if sanitize(name).as_deref() != Some(name) {
        return Err("Not an instance.".into());
    }
    fs::remove_dir_all(root.join(name)).map_err(|e| format!("Could not delete {name}: {e}"))
}

/// The instance last chosen (Default when none, or when it is gone).
pub fn selected(root: &Path) -> String {
    let chosen = fs::read_to_string(root.join(SELECTED))
        .ok()
        .map(|t| t.trim().to_owned());
    match chosen {
        Some(n) if sanitize(&n).as_deref() == Some(n.as_str()) && root.join(&n).is_dir() => n,
        _ => DEFAULT.to_owned(),
    }
}

pub fn select(root: &Path, name: &str) {
    let _ = fs::create_dir_all(root);
    let _ = fs::write(root.join(SELECTED), name);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("hb-inst-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(sanitize("  My World! ").as_deref(), Some("My World"));
        assert_eq!(sanitize("../evil").as_deref(), Some("evil"));
        assert_eq!(sanitize("!!!"), None);
        assert_eq!(sanitize(&"a".repeat(40)).unwrap().len(), 24);
    }

    #[test]
    fn create_list_select_delete() {
        let root = temp("a");
        assert_eq!(list(&root), [DEFAULT]);
        assert_eq!(create(&root, "Survival").unwrap(), "Survival");
        assert!(create(&root, "Survival").is_err());
        assert_eq!(create(&root, "a Mods").unwrap(), "a Mods");
        assert_eq!(list(&root), [DEFAULT, "a Mods", "Survival"]);
        assert_eq!(selected(&root), DEFAULT);
        select(&root, "Survival");
        assert_eq!(selected(&root), "Survival");
        delete(&root, "Survival").unwrap();
        assert_eq!(
            selected(&root),
            DEFAULT,
            "a deleted instance is no longer selected"
        );
        assert!(delete(&root, DEFAULT).is_err());
        assert!(delete(&root, "../x").is_err());
        let _ = fs::remove_dir_all(root);
    }
}
