use super::*;
use records::{Record, clone_value, n, sound};

pub(super) fn encode(
    name: &str,
    v: &Value,
    _: &MetadataContext<'_>,
) -> Option<Result<Value, Error>> {
    if !matches!(
        name,
        "minecraft:potion_contents"
            | "minecraft:suspicious_stew_effects"
            | "minecraft:consumable"
            | "minecraft:death_protection"
            | "minecraft:equippable"
            | "minecraft:tool"
    ) {
        return None;
    }
    Some((|| {
        if name == "minecraft:potion_contents" && v.is_string() {
            return Ok(Value::Object(Map::from([(
                "potion".into(),
                identifier(v)?,
            )])));
        }
        if name == "minecraft:suspicious_stew_effects" {
            return array(v, |v| {
                let mut r = Record::new(v);
                r.required("id", identifier)?;
                if let Some(value) =
                    present(v.as_object().unwrap(), "duration").filter(|v| v.is_number())
                    && super::super::super::number::checked_int(value)? != 160
                {
                    r.required("duration", integer)?;
                }
                Ok(r.finish())
            });
        }
        let mut r = Record::new(v);
        match name {
            "minecraft:potion_contents" => {
                r.optional("potion", identifier, None)?;
                r.optional("custom_color", integer, None)?;
                r.optional(
                    "custom_effects",
                    |v| array(v, effect),
                    Some(Value::Array(Vec::new())),
                )?;
                r.optional("custom_name", clone_value, None)?;
            }
            "minecraft:consumable" => {
                r.optional("consume_seconds", float, Some(n("1.6")))?;
                r.optional("animation", clone_value, Some(Value::String("eat".into())))?;
                // A provided inline Holder.Direct is distinct from the default
                // registered Holder.Reference, even with the same sound id.
                r.optional("sound", sound, None)?;
                r.optional(
                    "has_consume_particles",
                    clone_value,
                    Some(Value::Bool(true)),
                )?;
                r.optional(
                    "on_consume_effects",
                    |v| array(v, consume),
                    Some(Value::Array(Vec::new())),
                )?;
            }
            "minecraft:death_protection" => r.optional(
                "death_effects",
                |v| array(v, consume),
                Some(Value::Array(Vec::new())),
            )?,
            "minecraft:equippable" => {
                r.required("slot", clone_value)?;
                r.optional("equip_sound", sound, None)?;
                r.optional("asset_id", identifier, None)?;
                r.optional("camera_overlay", identifier, None)?;
                r.optional("allowed_entities", clone_value, None)?;
                for key in ["dispensable", "swappable", "damage_on_hurt"] {
                    r.optional(key, clone_value, Some(Value::Bool(true)))?;
                }
                for key in ["equip_on_interact", "can_be_sheared"] {
                    r.optional(key, clone_value, Some(Value::Bool(false)))?;
                }
                r.optional("shearing_sound", sound, None)?;
            }
            "minecraft:tool" => {
                r.required("rules", |v| {
                    array(v, |v| {
                        let mut rule = Record::new(v);
                        rule.required("blocks", clone_value)?;
                        rule.optional("speed", float, None)?;
                        rule.optional("correct_for_drops", clone_value, None)?;
                        Ok(rule.finish())
                    })
                })?;
                r.optional("default_mining_speed", float, Some(n("1.0")))?;
                r.optional("damage_per_block", integer, Some(n("1")))?;
                r.optional(
                    "can_destroy_blocks_in_creative",
                    clone_value,
                    Some(Value::Bool(true)),
                )?;
            }
            _ => return Err(Error::IncompleteCodec),
        }
        Ok(r.finish())
    })())
}
fn effect(v: &Value) -> Result<Value, Error> {
    let Value::Object(mut out) = details(v)? else {
        unreachable!()
    };
    out.insert("id".into(), identifier(&v.as_object().unwrap()["id"])?);
    Ok(Value::Object(out))
}
fn details(v: &Value) -> Result<Value, Error> {
    let m = v.as_object().unwrap();
    let mut r = Record::new(v);
    r.optional("amplifier", integer, Some(n("0")))?;
    r.optional("duration", integer, Some(n("0")))?;
    r.optional("ambient", clone_value, Some(Value::Bool(false)))?;
    r.optional("show_particles", clone_value, Some(Value::Bool(true)))?;
    r.optional("hidden_effect", details, None)?;
    let show_particles = present(m, "show_particles")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let show_icon = present(m, "show_icon")
        .and_then(Value::as_bool)
        .unwrap_or(show_particles);
    let Value::Object(mut out) = r.finish() else {
        unreachable!()
    };
    // Details' encoder always returns Optional.of(showIcon).
    out.insert("show_icon".into(), Value::Bool(show_icon));
    Ok(Value::Object(out))
}
fn consume(v: &Value) -> Result<Value, Error> {
    let m = v.as_object().unwrap();
    let kind = crate::args::parse_identifier(m["type"].as_str().unwrap()).ok_or(Error::Invalid)?;
    let mut r = Record::new(v);
    match kind.as_str() {
        "minecraft:apply_effects" => {
            r.required("effects", |v| array(v, effect))?;
            r.optional("probability", float, Some(n("1.0")))?;
        }
        "minecraft:remove_effects" => r.required("effects", clone_value)?,
        "minecraft:clear_all_effects" => {}
        "minecraft:teleport_randomly" => {
            r.optional("diameter", float, Some(n("16.0")))?;
            r.optional(
                "directional_particles",
                clone_value,
                Some(Value::Bool(true)),
            )?;
        }
        "minecraft:play_sound" => r.required("sound", sound)?,
        _ => return Err(Error::Invalid),
    }
    let Value::Object(mut out) = r.finish() else {
        unreachable!()
    };
    out.insert("type".into(), Value::String(kind));
    Ok(Value::Object(out))
}
