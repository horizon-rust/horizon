//! A mob's custom name, saved whole when it is styled.
use super::*;

/// `Entity.saveWithoutId`'s `CustomName`: a name from the save keeps its own
/// shape unless it changed; a styled name (a name tag's, an egg's) is saved
/// whole.
pub(super) fn put_mob_custom_name(tag: &mut Tag, m: &Mob) {
    match (&m.custom_name_text, &m.custom_name) {
        (Some(text), _) => {
            if !tag.get("CustomName").is_some_and(|t| name_text(t) == *text) {
                put(
                    tag,
                    "CustomName",
                    crate::scoreboard::json_to_tag(&text.component()),
                );
            }
        }
        (None, Some(name)) if custom_name(tag) != m.custom_name => {
            put(tag, "CustomName", Tag::String(name.as_str().into()));
        }
        _ => {}
    }
}

/// `Bucketable.saveDefaultDataToBucketTag`: `copyFrom(CUSTOM_NAME)`, whole.
pub(super) fn bucket_custom_name(m: &Mob) -> Option<Tag> {
    match (&m.custom_name_text, &m.custom_name) {
        (Some(text), _) => Some(crate::scoreboard::json_to_tag(&text.component())),
        (None, name) => name.as_ref().map(|name| Tag::String(name.as_str().into())),
    }
}

impl Mob {
    /// `Entity.getCustomName`: the whole text, style included, else the plain name.
    pub fn display_custom_name(&self) -> Option<crate::text::Text> {
        let plain = || self.custom_name.as_deref().map(crate::text::Text::literal);
        self.custom_name_text.clone().or_else(plain)
    }
}
