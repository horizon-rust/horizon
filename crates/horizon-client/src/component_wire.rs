//! Bounded Horizon string patches carried by vanilla custom-data NBT.
use horizon_world::nbt::{self, JavaString, Tag};
use std::collections::BTreeMap;

const ENVELOPE: &str = "horizon:string_components";
const CUSTOM_DATA: &str = "minecraft:custom_data";
const LIMITS: nbt::Limits = nbt::Limits {
    max_bytes: 8192,
    max_nodes: 128,
    max_depth: 8,
};

fn key(key: &str) -> Result<(), String> {
    let Some((namespace, path)) = key.split_once(':') else {
        return Err("Custom component needs a namespace".into());
    };
    if key.len() > 128
        || namespace.is_empty()
        || path.is_empty()
        || matches!(namespace, "minecraft" | "horizon")
        || !namespace
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
        || !path
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_./-".contains(&b))
    {
        return Err("Invalid or reserved custom string component key".into());
    }
    Ok(())
}
fn text(tag: &Tag) -> Result<String, String> {
    let Tag::String(value) = tag else {
        return Err("Custom component value is not a string".into());
    };
    if value.0.len() > 256 {
        return Err("Custom component string exceeds 256 UTF-8 bytes".into());
    }
    let value = value
        .to_string()
        .map_err(|_| "Invalid custom component UTF-16")?;
    if value.len() > 256 {
        return Err("Custom component string exceeds 256 UTF-8 bytes".into());
    }
    Ok(value)
}
fn bounded(envelope: &Tag) -> Result<(), String> {
    nbt::write_network(envelope, LIMITS)
        .map(|_| ())
        .map_err(|e| format!("Custom component envelope exceeds limits: {e}"))
}

/// Only actual stack patches travel. Missing keys continue to inherit prototypes.
pub fn encode(other: &[(String, Tag)]) -> Result<Option<Tag>, String> {
    let mut original = None;
    let mut patches = BTreeMap::new();
    let mut removed = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (name, value) in other {
        if name == CUSTOM_DATA {
            if original.replace(value.clone()).is_some() {
                return Err("Duplicate custom-data component".into());
            }
            continue;
        }
        let (name, removal) = name
            .strip_prefix('!')
            .map_or((name.as_str(), false), |name| (name, true));
        if !name.contains(':') {
            return Err("Custom component needs an explicit namespace".into());
        }
        if name.starts_with("minecraft:") {
            continue;
        }
        key(name)?;
        if !seen.insert(name.to_owned()) {
            return Err("Conflicting or duplicate custom component patches".into());
        }
        if seen.len() > 16 {
            return Err("Custom component envelope exceeds sixteen keys".into());
        }
        if removal {
            if !matches!(value, Tag::Compound(map) if map.is_empty()) {
                return Err("Custom removal must be an empty compound".into());
            }
            removed.push(Tag::String(name.into()));
        } else {
            patches.insert(
                JavaString::from(name),
                Tag::String(text(value)?.as_str().into()),
            );
        }
    }
    if original
        .as_ref()
        .is_some_and(|tag| tag.get(ENVELOPE).is_some())
    {
        return Err("Reserved custom-data envelope key collision".into());
    }
    if seen.is_empty() {
        return Ok(original);
    }
    let envelope = Tag::compound([
        ("version", Tag::Int(1)),
        ("original_present", Tag::Byte(i8::from(original.is_some()))),
        ("patches", Tag::Compound(patches)),
        ("removed", Tag::List(removed)),
    ]);
    bounded(&envelope)?;
    let mut custom = match original {
        None => BTreeMap::new(),
        Some(Tag::Compound(map)) => map,
        Some(_) => return Err("Cannot merge string envelope into non-compound custom data".into()),
    };
    custom.insert(ENVELOPE.into(), envelope);
    Ok(Some(Tag::Compound(custom)))
}

/// Strip a valid envelope, preserving the original custom-data presence and content.
pub fn restore(components: &mut Tag) -> Result<(), String> {
    let Some(custom) = components.get(CUSTOM_DATA) else {
        return Ok(());
    };
    let Some(envelope) = custom.get(ENVELOPE) else {
        return Ok(());
    };
    bounded(envelope)?;
    let Tag::Compound(fields) = envelope else {
        return Err("Malformed custom component envelope".into());
    };
    if fields.len() != 4 || envelope.get("version") != Some(&Tag::Int(1)) {
        return Err("Unsupported custom component envelope version or fields".into());
    }
    let present = match envelope.get("original_present") {
        Some(Tag::Byte(0)) => false,
        Some(Tag::Byte(1)) => true,
        _ => return Err("Malformed original custom-data presence".into()),
    };
    let Some(Tag::Compound(patches)) = envelope.get("patches") else {
        return Err("Malformed custom string patches".into());
    };
    let Some(Tag::List(removed)) = envelope.get("removed") else {
        return Err("Malformed custom string removals".into());
    };
    if patches.len() + removed.len() > 16 || patches.len() + removed.len() == 0 {
        return Err("Invalid custom component key count".into());
    }
    let mut restored = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (name, value) in patches {
        if name.0.len() > 128 {
            return Err("Custom component key exceeds 128 bytes".into());
        }
        let name = name
            .to_string()
            .map_err(|_| "Invalid custom component key UTF-16")?;
        key(&name)?;
        if !seen.insert(name.clone()) {
            return Err("Duplicate custom string patch".into());
        }
        restored.push((name, Tag::String(text(value)?.as_str().into())));
    }
    for value in removed {
        let name = text(value)?;
        key(&name)?;
        if !seen.insert(name.clone()) {
            return Err("Conflicting custom string removal".into());
        }
        restored.push((format!("!{name}"), Tag::Compound(BTreeMap::new())));
    }
    let Tag::Compound(mut original) = custom.clone() else {
        return Err("Malformed custom data".into());
    };
    original.remove(&JavaString::from(ENVELOPE));
    if !present && !original.is_empty() {
        return Err("Unexpected original custom-data content".into());
    }
    let Tag::Compound(components) = components else {
        return Err("Malformed stack components".into());
    };
    for (name, _) in &restored {
        if components.contains_key(&JavaString::from(name.as_str())) {
            return Err("Envelope conflicts with an existing component".into());
        }
    }
    components.remove(&JavaString::from(CUSTOM_DATA));
    if present {
        components.insert(CUSTOM_DATA.into(), Tag::Compound(original));
    }
    for (name, value) in restored {
        components.insert(name.as_str().into(), value);
    }
    Ok(())
}
