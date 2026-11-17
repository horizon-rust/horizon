//! Canonical JSON conversion and normalization for Minecraft 26.3 item data
//! components (`DataComponents`), so NBT-backed stack components and resource
//! pack `items/*.json` component cases compare with `ComponentContents` parity.
use crate::{component, identifier::normalize_id, scoreboard::canonical_component};
use horizon_world::nbt::Tag;
use serde_json::{Map, Value};

/// Converts an NBT tag to raw JSON, keeping numeric tags numeric (`Tag::Byte`
/// stays a number, unlike `scoreboard::tag_to_json`).
pub fn raw_tag_json(tag: &Tag) -> Value {
    match tag {
        Tag::End => Value::Null,
        Tag::Byte(v) => Value::from(*v),
        Tag::Short(v) => Value::from(*v),
        Tag::Int(v) => Value::from(*v),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(bits) => serde_json::Number::from_f64(f64::from(f32::from_bits(*bits)))
            .map_or(Value::Null, Value::Number),
        Tag::Double(bits) => {
            serde_json::Number::from_f64(f64::from_bits(*bits)).map_or(Value::Null, Value::Number)
        }
        Tag::ByteArray(a) => Value::Array(a.iter().map(|b| Value::from(*b as i8)).collect()),
        Tag::String(s) => Value::String(s.to_string_lossy()),
        Tag::List(l) => Value::Array(l.iter().map(raw_tag_json).collect()),
        Tag::Compound(c) => Value::Object(
            c.iter()
                .map(|(k, v)| (k.to_string_lossy(), raw_tag_json(v)))
                .collect(),
        ),
        Tag::IntArray(a) => Value::Array(a.iter().map(|v| Value::from(*v)).collect()),
        Tag::LongArray(a) => Value::Array(a.iter().map(|v| Value::from(*v)).collect()),
    }
}

/// `NbtOps.getBooleanValue`: every numeric tag is true unless its double value
/// is zero (`NaN` and fractions are true; `-0.0` is false).
pub fn nbt_bool(tag: &Tag) -> Option<bool> {
    Some(match tag {
        Tag::Byte(v) => *v != 0,
        Tag::Short(v) => *v != 0,
        Tag::Int(v) => *v != 0,
        Tag::Long(v) => *v != 0,
        Tag::Float(bits) => f32::from_bits(*bits) != 0.0,
        Tag::Double(bits) => f64::from_bits(*bits) != 0.0,
        _ => return None,
    })
}

/// Converts a stack NBT component tag into the canonical JSON form its
/// `DataComponents` codec produces.
pub fn nbt_component_to_json(component_id: &str, tag: &Tag) -> Value {
    let id = normalize_id(component_id).unwrap_or_else(|| component_id.to_owned());
    match id.as_str() {
        "minecraft:enchantment_glint_override" => nbt_bool(tag)
            .map(Value::Bool)
            .unwrap_or_else(|| raw_tag_json(tag)),
        "minecraft:custom_model_data" => custom_model_data_from_nbt(tag),
        _ => {
            let mut value = raw_tag_json(tag);
            apply_nbt_bool_fields(&id, tag, &mut value);
            canonicalize_component_json(&id, value)
        }
    }
}

/// Normalizes a JSON component value to the canonical form its `DataComponents`
/// codec produces under `JsonOps`.
pub fn canonicalize_component_json(component_id: &str, value: Value) -> Value {
    let id = normalize_id(component_id).unwrap_or_else(|| component_id.to_owned());
    match id.as_str() {
        "minecraft:custom_name" | "minecraft:item_name" => canonical_text_component(&value),
        "minecraft:lore" => match value {
            Value::Array(lines) => {
                Value::Array(lines.iter().map(canonical_text_component).collect())
            }
            other => other,
        },
        "minecraft:enchantment_glint_override" => json_bool(&value).map_or(value, Value::Bool),
        "minecraft:custom_model_data" => canonical_custom_model_data(value),
        "minecraft:written_book_content" => canonical_written_book(value),
        "minecraft:writable_book_content" => canonical_writable_book(value),
        "minecraft:sign_text_front" | "minecraft:sign_text_back" => canonical_sign_text(value),
        "minecraft:attribute_modifiers" => canonical_attribute_modifiers(value),
        "minecraft:entity_data" | "minecraft:block_entity_data" => canonical_typed_entity(value),
        "minecraft:lodestone_tracker" => canonical_lodestone_tracker(value),
        "minecraft:tooltip_display" => canonical_tooltip_display(value),
        "minecraft:equippable" => canonical_equippable(value),
        "minecraft:consumable" => canonical_consumable(value),
        "minecraft:food" => canonical_food(value),
        "minecraft:use_effects" => canonical_use_effects(value),
        "minecraft:firework_explosion" => canonical_firework_explosion(value),
        "minecraft:fireworks" => canonical_fireworks(value),
        "minecraft:potion_contents" => canonical_potion_contents(value),
        "minecraft:enchantments" | "minecraft:stored_enchantments" => canonical_id_map(value),
        "minecraft:trim" => canonical_trim(value),
        "minecraft:item_model"
        | "minecraft:tooltip_style"
        | "minecraft:note_block_sound"
        | "minecraft:damage_type"
        | "minecraft:block_transformer"
        | "minecraft:break_sound"
        | "minecraft:provides_pottery_pattern" => normalize_string_id(value),
        "minecraft:instrument" | "minecraft:provides_trim_material" => {
            canonical_holder_with_description(value)
        }
        "minecraft:painting/variant" => canonical_painting_variant(value),
        "minecraft:jukebox_playable" => canonical_jukebox_playable(value),
        _ => value,
    }
}

fn json_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().map(|f| f != 0.0),
        _ => None,
    }
}

fn normalize_string_id(v: Value) -> Value {
    match v {
        Value::String(ref s) => normalize_id(s).map_or(v, Value::String),
        other => other,
    }
}

fn normalize_id_field(map: &mut Map<String, Value>, key: &str) {
    if let Some(Value::String(s)) = map.get(key)
        && let Some(id) = normalize_id(s)
    {
        map.insert(key.to_owned(), Value::String(id));
    }
}

fn normalize_bool_field(map: &mut Map<String, Value>, key: &str, default: Option<bool>) {
    let Some(v) = map.get(key) else {
        return;
    };
    if let Some(b) = json_bool(v) {
        if default == Some(b) {
            map.remove(key);
        } else {
            map.insert(key.to_owned(), Value::Bool(b));
        }
    }
}

fn set_always_present_bool(map: &mut Map<String, Value>, key: &str, default: bool) {
    let b = map.get(key).and_then(json_bool).unwrap_or(default);
    map.insert(key.to_owned(), Value::Bool(b));
}

fn remove_empty_array(map: &mut Map<String, Value>, key: &str) {
    if map
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        map.remove(key);
    }
}

fn remove_default_i64(map: &mut Map<String, Value>, key: &str, default: i64) {
    if map.get(key).and_then(Value::as_i64) == Some(default) {
        map.remove(key);
    }
}

/// Normalizes a `ComponentSerialization.CODEC` JSON tree for equality with
/// vanilla `Component.equals` (style booleans, colors, shadow_color, font,
/// inline objects, click/hover events, and collapsed/list forms).
pub fn canonical_text_component(v: &Value) -> Value {
    let c = canonical_component(v);
    normalize_text_fields(c)
}

fn normalize_text_fields(v: Value) -> Value {
    let Value::Object(mut map) = v else {
        return v;
    };
    if let Some(shadow) = map.get("shadow_color")
        && let Some(packed) = component::shadow_color(shadow)
    {
        map.insert("shadow_color".into(), Value::from(packed as i32));
    }
    normalize_id_field(&mut map, "font");
    if map.contains_key("player") {
        normalize_bool_field(&mut map, "hat", Some(true));
    }
    if map.contains_key("nbt") {
        normalize_bool_field(&mut map, "interpret", Some(false));
        normalize_bool_field(&mut map, "plain", Some(false));
    }
    if let Some(Value::Object(hover)) = map.get_mut("hover_event") {
        match hover.get("action").and_then(Value::as_str) {
            Some("show_text") => {
                if let Some(val) = hover.remove("value") {
                    hover.insert("value".into(), normalize_text_fields(val));
                }
            }
            Some("show_item") | Some("show_entity") => {
                normalize_id_field(hover, "id");
            }
            _ => {}
        }
    }
    if let Some(Value::Object(click)) = map.get_mut("click_event")
        && click.get("action").and_then(Value::as_str) == Some("custom")
    {
        normalize_id_field(click, "id");
    }
    for key in ["extra", "with"] {
        if let Some(Value::Array(items)) = map.remove(key) {
            let items: Vec<Value> = items.into_iter().map(normalize_text_fields).collect();
            map.insert(key.into(), Value::Array(items));
        }
    }
    if let Some(sep) = map.remove("separator") {
        map.insert("separator".into(), normalize_text_fields(sep));
    }
    Value::Object(map)
}

fn canonical_filterable(v: Value, map_inner: fn(&Value) -> Value) -> Value {
    match v {
        Value::Object(mut map) if map.contains_key("raw") => {
            if let Some(raw) = map.remove("raw") {
                map.insert("raw".into(), map_inner(&raw));
            }
            match map.remove("filtered") {
                Some(Value::Null) | None => {}
                Some(filtered) => {
                    map.insert("filtered".into(), map_inner(&filtered));
                }
            }
            Value::Object(map)
        }
        other => Value::Object(Map::from_iter([("raw".into(), map_inner(&other))])),
    }
}

fn custom_model_data_from_nbt(tag: &Tag) -> Value {
    let mut value = raw_tag_json(tag);
    let flags: Option<Vec<Value>> = match tag.get("flags") {
        Some(Tag::List(flags)) => flags.iter().map(|f| nbt_bool(f).map(Value::Bool)).collect(),
        Some(Tag::ByteArray(flags)) => flags
            .iter()
            .map(|f| nbt_bool(&Tag::Byte(*f as i8)).map(Value::Bool))
            .collect(),
        Some(Tag::IntArray(flags)) => flags
            .iter()
            .map(|f| nbt_bool(&Tag::Int(*f)).map(Value::Bool))
            .collect(),
        Some(Tag::LongArray(flags)) => flags
            .iter()
            .map(|f| nbt_bool(&Tag::Long(*f)).map(Value::Bool))
            .collect(),
        _ => None,
    };
    if let (Some(flags), Some(obj)) = (flags, value.as_object_mut()) {
        obj.insert("flags".into(), Value::Array(flags));
    }
    canonical_custom_model_data(value)
}

fn canonical_custom_model_data(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    if let Some(Value::Array(flags)) = map.get_mut("flags") {
        for flag in flags {
            if let Some(b) = json_bool(flag) {
                *flag = Value::Bool(b);
            }
        }
    }
    for key in ["floats", "flags", "strings", "colors"] {
        remove_empty_array(&mut map, key);
    }
    Value::Object(map)
}

fn canonical_written_book(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    if let Some(title) = map.remove("title") {
        map.insert("title".into(), canonical_filterable(title, Value::clone));
    }
    if let Some(Value::Array(pages)) = map.remove("pages") {
        let pages: Vec<Value> = pages
            .into_iter()
            .map(|p| canonical_filterable(p, canonical_text_component))
            .collect();
        if !pages.is_empty() {
            map.insert("pages".into(), Value::Array(pages));
        }
    }
    remove_default_i64(&mut map, "generation", 0);
    normalize_bool_field(&mut map, "resolved", Some(false));
    Value::Object(map)
}

fn canonical_writable_book(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    if let Some(Value::Array(pages)) = map.remove("pages") {
        let pages: Vec<Value> = pages
            .into_iter()
            .map(|p| canonical_filterable(p, Value::clone))
            .collect();
        if !pages.is_empty() {
            map.insert("pages".into(), Value::Array(pages));
        }
    }
    Value::Object(map)
}

fn canonical_sign_text(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    if let Some(Value::Array(msgs)) = map.remove("messages") {
        map.insert(
            "messages".into(),
            Value::Array(msgs.iter().map(canonical_text_component).collect()),
        );
    }
    if let Some(Value::Array(filtered)) = map.remove("filtered_messages") {
        let filtered: Vec<Value> = filtered.iter().map(canonical_text_component).collect();
        if map.get("messages").and_then(Value::as_array) != Some(&filtered) {
            map.insert("filtered_messages".into(), Value::Array(filtered));
        }
    }
    if !map.contains_key("color") {
        map.insert("color".into(), Value::String("black".into()));
    }
    set_always_present_bool(&mut map, "has_glowing_text", false);
    Value::Object(map)
}

fn canonical_attribute_modifiers(value: Value) -> Value {
    let Value::Array(entries) = value else {
        return value;
    };
    let entries = entries
        .into_iter()
        .map(|entry| {
            let Value::Object(mut map) = entry else {
                return entry;
            };
            normalize_id_field(&mut map, "type");
            normalize_id_field(&mut map, "id");
            if map.get("slot").and_then(Value::as_str) == Some("any") {
                map.remove("slot");
            }
            if let Some(Value::Object(display)) = map.get_mut("display") {
                match display.get("type").and_then(Value::as_str) {
                    Some("default") => {
                        map.remove("display");
                    }
                    Some("override") => {
                        if let Some(val) = display.remove("value") {
                            display.insert("value".into(), canonical_text_component(&val));
                        }
                    }
                    _ => {}
                }
            }
            Value::Object(map)
        })
        .collect();
    Value::Array(entries)
}

fn canonical_typed_entity(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_id_field(&mut map, "id");
    Value::Object(map)
}

fn canonical_lodestone_tracker(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_bool_field(&mut map, "tracked", Some(true));
    if let Some(Value::Object(target)) = map.get_mut("target") {
        normalize_id_field(target, "dimension");
    }
    Value::Object(map)
}

fn canonical_tooltip_display(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_bool_field(&mut map, "hide_tooltip", Some(false));
    if let Some(Value::Array(hidden)) = map.remove("hidden_components") {
        let hidden: Vec<Value> = hidden.into_iter().map(normalize_string_id).collect();
        if !hidden.is_empty() {
            map.insert("hidden_components".into(), Value::Array(hidden));
        }
    }
    Value::Object(map)
}

fn canonical_equippable(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    for key in [
        "equip_sound",
        "asset_id",
        "camera_overlay",
        "shearing_sound",
    ] {
        normalize_id_field(&mut map, key);
    }
    if let Some(Value::String(allowed)) = map.get("allowed_entities")
        && !allowed.starts_with('#')
    {
        normalize_id_field(&mut map, "allowed_entities");
    } else if let Some(Value::Array(allowed)) = map.get_mut("allowed_entities") {
        for item in allowed {
            if let Value::String(s) = item
                && let Some(id) = normalize_id(s)
            {
                *s = id;
            }
        }
    }
    for key in ["dispensable", "swappable", "damage_on_hurt"] {
        normalize_bool_field(&mut map, key, Some(true));
    }
    for key in ["equip_on_interact", "can_be_sheared"] {
        normalize_bool_field(&mut map, key, Some(false));
    }
    Value::Object(map)
}

fn canonical_consumable(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_bool_field(&mut map, "has_consume_particles", Some(true));
    normalize_id_field(&mut map, "sound");
    remove_empty_array(&mut map, "on_consume_effects");
    Value::Object(map)
}

fn canonical_food(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_bool_field(&mut map, "can_always_eat", Some(false));
    Value::Object(map)
}

fn canonical_use_effects(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_bool_field(&mut map, "can_sprint", Some(false));
    normalize_bool_field(&mut map, "interact_vibrations", Some(true));
    Value::Object(map)
}

fn canonical_firework_explosion(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    remove_empty_array(&mut map, "colors");
    remove_empty_array(&mut map, "fade_colors");
    normalize_bool_field(&mut map, "has_trail", Some(false));
    normalize_bool_field(&mut map, "has_twinkle", Some(false));
    Value::Object(map)
}

fn canonical_fireworks(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    remove_default_i64(&mut map, "flight_duration", 0);
    if let Some(Value::Array(explosions)) = map.remove("explosions") {
        let explosions: Vec<Value> = explosions
            .into_iter()
            .map(canonical_firework_explosion)
            .collect();
        if !explosions.is_empty() {
            map.insert("explosions".into(), Value::Array(explosions));
        }
    }
    Value::Object(map)
}

fn canonical_mob_effect_instance(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_id_field(&mut map, "id");
    canonical_mob_effect_details(&mut map);
    Value::Object(map)
}

fn canonical_mob_effect_details(map: &mut Map<String, Value>) {
    remove_default_i64(map, "amplifier", 0);
    remove_default_i64(map, "duration", 0);
    normalize_bool_field(map, "ambient", Some(false));
    normalize_bool_field(map, "show_particles", Some(true));
    let show_particles = map
        .get("show_particles")
        .and_then(json_bool)
        .unwrap_or(true);
    set_always_present_bool(map, "show_icon", show_particles);
    if let Some(Value::Object(hidden)) = map.get_mut("hidden_effect") {
        canonical_mob_effect_details(hidden);
    }
}

fn canonical_potion_contents(value: Value) -> Value {
    match value {
        Value::String(s) => {
            let id = normalize_id(&s).unwrap_or(s);
            Value::Object(Map::from_iter([("potion".into(), Value::String(id))]))
        }
        Value::Object(mut map) => {
            normalize_id_field(&mut map, "potion");
            if let Some(Value::Array(effects)) = map.remove("custom_effects") {
                let effects: Vec<Value> = effects
                    .into_iter()
                    .map(canonical_mob_effect_instance)
                    .collect();
                if !effects.is_empty() {
                    map.insert("custom_effects".into(), Value::Array(effects));
                }
            }
            Value::Object(map)
        }
        other => other,
    }
}

fn canonical_id_map(value: Value) -> Value {
    let Value::Object(map) = value else {
        return value;
    };
    Value::Object(
        map.into_iter()
            .map(|(k, v)| (normalize_id(&k).unwrap_or(k), v))
            .collect(),
    )
}

fn canonical_trim(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    for key in ["material", "pattern"] {
        if let Some(v) = map.remove(key) {
            map.insert(key.into(), canonical_holder_with_description(v));
        }
    }
    Value::Object(map)
}

fn canonical_holder_with_description(value: Value) -> Value {
    match value {
        Value::String(s) => Value::String(normalize_id(&s).unwrap_or(s)),
        Value::Object(mut map) => {
            if let Some(desc) = map.remove("description") {
                map.insert("description".into(), canonical_text_component(&desc));
            }
            normalize_id_field(&mut map, "sound_event");
            normalize_id_field(&mut map, "asset_name");
            normalize_id_field(&mut map, "asset_id");
            normalize_id_field(&mut map, "ingredient");
            Value::Object(map)
        }
        other => other,
    }
}

fn canonical_painting_variant(value: Value) -> Value {
    match value {
        Value::String(s) => Value::String(normalize_id(&s).unwrap_or(s)),
        Value::Object(mut map) => {
            normalize_id_field(&mut map, "asset_id");
            for key in ["title", "author"] {
                if let Some(v) = map.remove(key) {
                    map.insert(key.into(), canonical_text_component(&v));
                }
            }
            Value::Object(map)
        }
        other => other,
    }
}

fn canonical_jukebox_playable(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    if let Some(song) = map.remove("song") {
        map.insert("song".into(), canonical_holder_with_description(song));
    }
    Value::Object(map)
}

/// Preserves `NbtOps.getBooleanValue` for non-finite float/double tags (such
/// as `NaN`, which `raw_tag_json` maps to `Value::Null`) on known boolean keys.
fn apply_nbt_bool_fields(component_id: &str, tag: &Tag, value: &mut Value) {
    match component_id {
        "minecraft:written_book_content" => {
            patch_bool_key(tag, value, "resolved");
        }
        "minecraft:sign_text_front" | "minecraft:sign_text_back" => {
            patch_bool_key(tag, value, "has_glowing_text");
        }
        "minecraft:lodestone_tracker" => patch_bool_key(tag, value, "tracked"),
        "minecraft:tooltip_display" => patch_bool_key(tag, value, "hide_tooltip"),
        "minecraft:equippable" => {
            for key in [
                "dispensable",
                "swappable",
                "damage_on_hurt",
                "equip_on_interact",
                "can_be_sheared",
            ] {
                patch_bool_key(tag, value, key);
            }
        }
        "minecraft:consumable" => patch_bool_key(tag, value, "has_consume_particles"),
        "minecraft:food" => patch_bool_key(tag, value, "can_always_eat"),
        "minecraft:use_effects" => {
            patch_bool_key(tag, value, "can_sprint");
            patch_bool_key(tag, value, "interact_vibrations");
        }
        "minecraft:firework_explosion" => {
            patch_bool_key(tag, value, "has_trail");
            patch_bool_key(tag, value, "has_twinkle");
        }
        "minecraft:fireworks" => {
            if let (Some(Tag::List(tags)), Some(vals)) = (
                tag.get("explosions"),
                value.get_mut("explosions").and_then(|v| v.as_array_mut()),
            ) {
                for (t, v) in tags.iter().zip(vals.iter_mut()) {
                    patch_bool_key(t, v, "has_trail");
                    patch_bool_key(t, v, "has_twinkle");
                }
            }
        }
        _ => {}
    }
}

fn patch_bool_key(tag: &Tag, value: &mut Value, key: &str) {
    if let Some(child) = tag.get(key)
        && let Some(b) = nbt_bool(child)
        && let Some(obj) = value.as_object_mut()
    {
        obj.insert(key.to_owned(), Value::Bool(b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::nbt::parse_snbt;
    use serde_json::json;

    #[test]
    fn custom_name_nbt_booleans_and_shorthands_match_pack_json() {
        let (nbt, _) = parse_snbt(
            r#"{text:"",extra:[{text:"Styled Item",strikethrough:0b,obfuscated:0b,bold:0b,italic:1b,underlined:0b,color:"gold"}]}"#,
        )
        .unwrap();
        let pack_when = json!({
            "text": "",
            "extra": [{
                "text": "Styled Item",
                "strikethrough": false,
                "obfuscated": false,
                "bold": false,
                "italic": true,
                "underlined": false,
                "color": "gold"
            }]
        });
        assert_eq!(
            nbt_component_to_json("minecraft:custom_name", &nbt),
            canonicalize_component_json("minecraft:custom_name", pack_when)
        );
        // `SelectItemModel` keys on `TextColor.hashCode()`, which includes `name`,
        // so hex `#ffaa00` normalizes to `#FFAA00` without collapsing to `"gold"`.
        assert_eq!(
            canonicalize_component_json(
                "minecraft:custom_name",
                json!({"text": "Hex", "color": "#ffaa00"})
            ),
            canonicalize_component_json(
                "minecraft:custom_name",
                json!({"text": "Hex", "color": "#FFAA00"})
            )
        );
        assert_ne!(
            canonicalize_component_json(
                "minecraft:custom_name",
                json!({"text": "Hex", "color": "#ffaa00"})
            ),
            canonicalize_component_json(
                "minecraft:custom_name",
                json!({"text": "Hex", "color": "gold"})
            )
        );
        assert_eq!(
            canonicalize_component_json("minecraft:custom_name", json!({"text": "Plain Title"})),
            canonicalize_component_json("minecraft:custom_name", json!("Plain Title"))
        );
        assert_eq!(
            canonicalize_component_json(
                "minecraft:custom_name",
                json!(["", {"text": "First", "color": "green", "italic": 1}])
            ),
            canonicalize_component_json(
                "minecraft:custom_name",
                json!({"text": "", "extra": [{"text": "First", "color": "green", "italic": true}]})
            )
        );
    }

    #[test]
    fn written_book_normalizes_filterable_resolved_and_defaults() {
        let nbt = Tag::compound([
            ("title", Tag::String("Ledger".into())),
            ("author", Tag::String("Alice".into())),
            ("generation", Tag::Int(1)),
            (
                "pages",
                Tag::List(vec![Tag::String("Page 1\nLine 2".into())]),
            ),
            ("resolved", Tag::Byte(1)),
        ]);
        let pack_when = json!({
            "generation": 1,
            "pages": [{"raw": "Page 1\nLine 2"}],
            "resolved": true,
            "title": {"raw": "Ledger"},
            "author": "Alice"
        });
        assert_eq!(
            nbt_component_to_json("minecraft:written_book_content", &nbt),
            canonicalize_component_json("minecraft:written_book_content", pack_when)
        );
    }

    #[test]
    fn entity_data_normalizes_id_while_custom_data_keeps_numeric_bytes() {
        let entity_nbt = Tag::compound([
            ("id", Tag::String("minecraft:frog".into())),
            ("variant", Tag::String("cold".into())),
        ]);
        let entity_when = json!({"id": "frog", "variant": "cold"});
        assert_eq!(
            nbt_component_to_json("entity_data", &entity_nbt),
            canonicalize_component_json("entity_data", entity_when)
        );

        let custom = Tag::compound([("flag", Tag::Byte(1)), ("count", Tag::Byte(0))]);
        assert_eq!(
            nbt_component_to_json("minecraft:custom_data", &custom),
            json!({"flag": 1, "count": 0})
        );
    }
}
