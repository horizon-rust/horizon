//! ItemStackTemplate's map codec and DataComponentPatch's dispatched map.
//! Each set value must pass its actual component codec supplied by the caller.
use super::value::{Map, Value};
use super::{Error, R, all, number, optional, require};
use crate::Registries;
use std::collections::BTreeSet;

pub(super) fn validate(
    map: &Map<String, Value>,
    registries: &Registries,
    component: impl Fn(&str, &Value) -> R,
) -> R {
    let item = (|| {
        let raw = map
            .get("id")
            .and_then(Value::as_str)
            .ok_or(Error::Invalid)?;
        let id = crate::args::parse_identifier(raw).ok_or(Error::Invalid)?;
        require(id != "minecraft:air")?;
        let items = registries.ids("minecraft:item");
        if items.is_empty() {
            return Err(Error::Unavailable);
        }
        require(items.contains(&id))
    })();
    all([
        item,
        optional(map, "count", |v| {
            require((1..=99).contains(&number::checked_int(v)?))
        }),
        optional(map, "components", |v| {
            let patch = v.as_object().ok_or(Error::Invalid)?;
            if !patch.is_empty() && registries.persistent_components.is_empty() {
                return Err(Error::Unavailable);
            }
            let mut decoded = BTreeSet::new();
            all(patch.iter().map(|(key, value)| {
                let removed = key.starts_with('!');
                let raw = if removed { &key[1..] } else { key.as_str() };
                let id = crate::args::parse_identifier(raw).ok_or(Error::Invalid)?;
                require(registries.persistent_components.contains(&id))?;
                // Aliases of the same decoded PatchKey collide; set and remove
                // are distinct PatchKeys and both may be present in the input.
                let unique = require(decoded.insert((id.clone(), removed)));
                let value = if removed {
                    // Removed.CODEC is a MapCodec.unitCodec: extra map entries
                    // are ignored, but scalar/list removal payloads are invalid.
                    require(value.is_object())
                } else {
                    component(&id, value)
                };
                all([unique, value])
            }))
        }),
    ])
}
