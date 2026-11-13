//! Encoders paired with persistent payload codecs. No original map is used
//! as a substitute for a decoded record with discarded/defaulted fields.
use super::*;
use crate::component::json::context::MetadataContext;
mod combat;
mod consumption;
mod misc;
mod predicates;
mod records;

pub(super) fn encode(name: &str, v: &Value, ctx: &MetadataContext<'_>) -> Result<Value, Error> {
    super::super::payload::validate(name, v, ctx)?;
    if let Some(encoded) = records::encode(name, v, ctx) {
        return encoded;
    }
    if let Some(encoded) = consumption::encode(name, v, ctx) {
        return encoded;
    }
    if let Some(encoded) = combat::encode(name, v) {
        return encoded;
    }
    if let Some(encoded) = misc::encode(name, v, ctx) {
        return encoded;
    }
    if let Some(encoded) = predicates::encode(name, v, ctx) {
        return encoded;
    }
    match name {
        "minecraft:damage"
        | "minecraft:repair_cost"
        | "minecraft:max_damage"
        | "minecraft:max_stack_size"
        | "minecraft:map_id"
        | "minecraft:dyed_color"
        | "minecraft:ominous_bottle_amplifier" => integer(v),
        "minecraft:potion_duration_scale" | "minecraft:minimum_attack_charge" => float(v),
        "minecraft:enchantment_glint_override" => Ok(v.clone()),
        "minecraft:unbreakable"
        | "minecraft:intangible_projectile"
        | "minecraft:glider"
        | "minecraft:waxed"
        | "minecraft:enchantments"
        | "minecraft:stored_enchantments" => Ok(Value::Object(Map::new())),
        "minecraft:custom_name" | "minecraft:item_name" => component(v, ctx),
        "minecraft:lore" => array(v, |v| component(v, ctx)),
        "minecraft:item_model" | "minecraft:tooltip_style" | "minecraft:note_block_sound" => {
            identifier(v)
        }
        "minecraft:recipes" => array(v, identifier),
        "minecraft:block_state" => Ok(v.clone()),
        "minecraft:profile" => super::data::profile(v),
        "minecraft:custom_data" | "minecraft:bucket_entity_data" => super::data::compound(v),
        "minecraft:entity_data" | "minecraft:block_entity_data" => {
            let Value::Object(mut m) = super::data::compound(v)? else {
                return Err(Error::Invalid);
            };
            let id = identifier(&m["id"])?;
            m.insert("id".into(), id);
            Ok(Value::Object(m))
        }
        "minecraft:rarity"
        | "minecraft:dye"
        | "minecraft:base_color"
        | "minecraft:wolf/collar"
        | "minecraft:cat/collar"
        | "minecraft:sheep/color"
        | "minecraft:shulker/color"
        | "minecraft:cushion/color"
        | "minecraft:tropical_fish/base_color"
        | "minecraft:tropical_fish/pattern_color"
        | "minecraft:fox/variant"
        | "minecraft:salmon/size"
        | "minecraft:parrot/variant"
        | "minecraft:tropical_fish/pattern"
        | "minecraft:mooshroom/variant"
        | "minecraft:rabbit/variant"
        | "minecraft:horse/variant"
        | "minecraft:llama/variant"
        | "minecraft:axolotl/variant" => Ok(v.clone()),
        "minecraft:charged_projectiles" | "minecraft:bundle_contents" => {
            array(v, |v| template(v, ctx))
        }
        "minecraft:use_remainder" | "minecraft:sulfur_cube_content" => template(v, ctx),
        "minecraft:container" => array(v, |v| {
            let m = v.as_object().unwrap();
            Ok(Value::Object(Map::from([
                ("slot".into(), integer(&m["slot"])?),
                ("item".into(), template(&m["item"], ctx)?),
            ])))
        }),
        "minecraft:written_book_content" => written(v, ctx),
        "minecraft:writable_book_content" => {
            let m = v.as_object().unwrap();
            let mut out = Map::new();
            if let Some(v) = present(m, "pages").filter(|v| !v.as_array().unwrap().is_empty()) {
                out.insert(
                    "pages".into(),
                    array(v, |v| filterable(v, |v| Ok(v.clone())))?,
                );
            }
            Ok(Value::Object(out))
        }
        _ => Err(Error::IncompleteCodec),
    }
}
pub(super) fn integer(v: &Value) -> Result<Value, Error> {
    Ok(Value::Number(
        super::super::number::checked_int(v)?.to_string(),
    ))
}
pub(super) fn float(v: &Value) -> Result<Value, Error> {
    let n = v
        .as_number()
        .unwrap()
        .parse::<f32>()
        .map_err(|_| Error::DecodeException)?;
    if !n.is_finite() {
        return Err(Error::DecodeException);
    }
    Ok(Value::Number(horizon_world::snbt_text::java_float(n)))
}
pub(super) fn identifier(v: &Value) -> Result<Value, Error> {
    Ok(Value::String(
        crate::args::parse_identifier(v.as_str().unwrap()).ok_or(Error::Invalid)?,
    ))
}
fn array(v: &Value, encode: impl Fn(&Value) -> Result<Value, Error>) -> Result<Value, Error> {
    Ok(Value::Array(
        v.as_array()
            .unwrap()
            .iter()
            .map(encode)
            .collect::<Result<_, _>>()?,
    ))
}
fn template(v: &Value, ctx: &MetadataContext<'_>) -> Result<Value, Error> {
    if v.is_string() {
        item(&Map::from([("id".into(), v.clone())]), ctx)
    } else {
        item(v.as_object().unwrap(), ctx)
    }
}
fn filterable(v: &Value, encode: impl Fn(&Value) -> Result<Value, Error>) -> Result<Value, Error> {
    let mut out = Map::new();
    if let Some(m) = v.as_object().filter(|m| present(m, "raw").is_some()) {
        let raw = encode(&m["raw"]);
        let filtered = present(m, "filtered").map(&encode).transpose();
        match (raw, filtered) {
            (Err(Error::DecodeException), _) | (_, Err(Error::DecodeException)) => {
                return Err(Error::DecodeException);
            }
            (Ok(raw), Ok(filtered)) => {
                out.insert("raw".into(), raw);
                if let Some(v) = filtered {
                    out.insert("filtered".into(), v);
                }
            }
            (Err(Error::Invalid), _) | (_, Err(Error::Invalid)) => {
                out.insert("raw".into(), encode(v)?);
            }
            (Err(e), _) | (_, Err(e)) => return Err(e),
        }
    } else {
        out.insert("raw".into(), encode(v)?);
    }
    Ok(Value::Object(out))
}
fn written(v: &Value, ctx: &MetadataContext<'_>) -> Result<Value, Error> {
    let m = v.as_object().unwrap();
    let mut out = Map::from([
        ("title".into(), filterable(&m["title"], |v| Ok(v.clone()))?),
        ("author".into(), m["author"].clone()),
    ]);
    if let Some(v) = present(m, "generation") {
        let n = super::super::number::checked_int(v)?;
        if n != 0 {
            out.insert("generation".into(), Value::Number(n.to_string()));
        }
    }
    if present(m, "resolved").and_then(Value::as_bool) == Some(true) {
        out.insert("resolved".into(), Value::Bool(true));
    }
    if let Some(v) = present(m, "pages").filter(|v| !v.as_array().unwrap().is_empty()) {
        out.insert(
            "pages".into(),
            array(v, |v| filterable(v, |v| component(v, ctx)))?,
        );
    }
    Ok(Value::Object(out))
}
