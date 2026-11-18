//! The offline player name the launcher gives the game (`HORIZON_PLAYER_NAME`).
//! A signed-in Microsoft account always plays under its own name; this is the name for
//! account-free play, kept in `settings.json` and editable in Settings.

/// A name Minecraft accepts: 3 to 16 letters, digits or underscores.
pub fn valid(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// What's wrong with the Settings field's text, or `None` when it can be saved. Empty saves
/// "no name", and the game keeps the one it made for itself.
pub fn refuse(text: &str) -> Option<String> {
    let name = text.trim();
    if name.is_empty() || valid(name) {
        return None;
    }
    Some(
        crate::i18n::tr("3 to 16 letters, digits or underscores; no spaces or other characters.")
            .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_minecraft_takes() {
        assert!(valid("Notch"));
        assert!(valid("steve"));
        assert!(valid("abc"));
        assert!(valid(&"x".repeat(16)));
        assert!(valid("A_9"));
        assert!(!valid(""), "empty is 'no name', not a valid one");
        assert!(!valid("ab"), "two letters");
        assert!(!valid(&"x".repeat(17)), "seventeen letters");
        assert!(!valid("Two Words"), "a space");
        assert!(!valid("Stevé"), "a non-ASCII letter");
        assert!(!valid("a-b"), "a dash");
    }

    #[test]
    fn the_field_says_what_is_wrong() {
        assert_eq!(refuse(""), None, "empty means the game's own name");
        assert_eq!(refuse("Notch"), None);
        assert_eq!(
            refuse("  Notch  "),
            None,
            "spaces around it are the field's"
        );
        assert!(refuse("x").is_some(), "too short");
        assert!(refuse("Two Words").is_some(), "a space");
    }
}
