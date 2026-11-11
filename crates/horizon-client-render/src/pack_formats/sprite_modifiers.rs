//! Polytone's screen sprite modifiers (`polytone/overlay_modifiers`, wiki "Screen Sprite
//! Modifiers"), read for the engine's [`crate::hud::sprite_modifiers::SpriteModifiers`].
use crate::hud::sprite_modifiers::{SpriteChange, SpriteModifiers};
use horizon_polytone::sprites::{SpriteModifier, sprite_id};
use std::collections::HashMap;
use std::path::Path;

/// A modifier's change as the engine draws it.
fn change(c: &horizon_polytone::sprites::SpriteChange) -> SpriteChange {
    SpriteChange {
        offset: c.offset,
        uv: c.uv,
        color: c.color,
        texture: c.texture.clone(),
    }
}

/// The sprite id of an atlas key, when it is a GUI sprite.
fn sprite_of(key: &str) -> Option<String> {
    if let Some(path) = key.strip_prefix("gui/sprites/") {
        return Some(sprite_id(path));
    }
    let (namespace, rest) = key.split_once(':')?;
    Some(format!(
        "{namespace}:{}",
        rest.strip_prefix("gui/sprites/")?
    ))
}

/// The modifiers of the packs merged at `root` for the GUI sprites among `keys` (the atlas's):
/// for each sprite, the last file that targets it.
pub fn read<'a>(root: &Path, keys: impl Iterator<Item = &'a String>) -> SpriteModifiers {
    let mut modifiers = Vec::new();
    let namespaces = std::fs::read_dir(root.join("assets"))
        .into_iter()
        .flatten()
        .flatten();
    for namespace in namespaces {
        let mut stack = vec![namespace.path().join("polytone/overlay_modifiers")];
        while let Some(dir) = stack.pop() {
            let mut entries: Vec<_> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .collect();
            entries.sort_by_key(|e| e.path());
            for entry in entries {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let parsed = horizon_world::json_file::read::<serde_json::Value>(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|json| SpriteModifier::parse(&json));
                match parsed {
                    Ok(m) => modifiers.push(m),
                    Err(e) => log::warn!("{}: {e}", path.display()),
                }
            }
        }
    }
    from_modifiers(modifiers, keys)
}

/// `modifiers` (in load order) for the GUI sprites among `keys`: for each sprite, the last that
/// targets it.
pub fn from_modifiers<'a>(
    modifiers: Vec<SpriteModifier>,
    keys: impl Iterator<Item = &'a String>,
) -> SpriteModifiers {
    let mut by_sprite = HashMap::new();
    if !modifiers.is_empty() {
        for key in keys {
            let Some(id) = sprite_of(key) else {
                continue;
            };
            if let Some(m) = modifiers
                .iter()
                .rev()
                .find(|m| m.targets.matches(&id, |_| false))
            {
                let overlays = m.overlays.iter().map(change).collect();
                by_sprite.insert(key.clone(), (change(&m.change), overlays));
            }
        }
    }
    SpriteModifiers::new(by_sprite)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_keys_turn_into_sprite_ids() {
        assert_eq!(
            sprite_of("gui/sprites/hud/hotbar").as_deref(),
            Some("minecraft:hud/hotbar")
        );
        assert_eq!(
            sprite_of("mypack:gui/sprites/hud/bar").as_deref(),
            Some("mypack:hud/bar")
        );
        assert_eq!(sprite_of("block/stone"), None);
    }

    #[test]
    fn each_sprite_takes_the_last_modifier_aiming_at_it() {
        let modifiers: Vec<SpriteModifier> = [
            serde_json::json!({"texture": "minecraft:hud/hotbar", "x_inc": -20, "width_inc": 40}),
            serde_json::json!({"texture": "minecraft:hud/hotbar", "color": -65536}),
            serde_json::json!({"texture": "minecraft:hud/hotbar_selection", "overlays": [{"texture": "minecraft:hud/heart/full", "y_inc": -30}]}),
        ]
        .iter()
        .map(|j| SpriteModifier::parse(j).unwrap())
        .collect();
        let keys = [
            "gui/sprites/hud/hotbar",
            "gui/sprites/hud/hotbar_selection",
            "block/stone",
        ]
        .map(str::to_owned);
        let mods = from_modifiers(modifiers, keys.iter());
        let expected: HashMap<String, (SpriteChange, Vec<SpriteChange>)> = [
            (
                "gui/sprites/hud/hotbar".to_owned(),
                (
                    SpriteChange {
                        color: Some(0xFFFF_0000),
                        ..SpriteChange::default()
                    },
                    vec![],
                ),
            ),
            (
                "gui/sprites/hud/hotbar_selection".to_owned(),
                (
                    SpriteChange::default(),
                    vec![SpriteChange {
                        offset: [0, -30, 0, 0],
                        texture: Some("minecraft:hud/heart/full".into()),
                        ..SpriteChange::default()
                    }],
                ),
            ),
        ]
        .into();
        assert_eq!(mods, SpriteModifiers::new(expected));
    }
}
