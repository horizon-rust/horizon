use super::*;
use records::{Record, clone_value, double, n};

pub(super) fn encode(
    name: &str,
    v: &Value,
    ctx: &MetadataContext<'_>,
) -> Option<Result<Value, Error>> {
    Some(match name {
        "minecraft:custom_model_data" => (|| {
            let mut r = Record::new(v);
            r.optional(
                "floats",
                |v| array(v, float),
                Some(Value::Array(Vec::new())),
            )?;
            for key in ["flags", "strings"] {
                r.optional(key, clone_value, Some(Value::Array(Vec::new())))?;
            }
            r.optional(
                "colors",
                |v| array(v, |v| super::super::data::color(v, false)),
                Some(Value::Array(Vec::new())),
            )?;
            Ok(r.finish())
        })(),
        "minecraft:container_loot" => (|| {
            let mut r = Record::new(v);
            r.required("loot_table", identifier)?;
            r.optional(
                "seed",
                |v| {
                    Ok(Value::Number(
                        super::super::super::number::checked_long(v)?.to_string(),
                    ))
                },
                Some(n("0")),
            )?;
            Ok(r.finish())
        })(),
        "minecraft:tooltip_display" => (|| {
            let mut r = Record::new(v);
            r.optional("hide_tooltip", clone_value, Some(Value::Bool(false)))?;
            r.optional(
                "hidden_components",
                |v| {
                    let mut ids = std::collections::BTreeSet::new();
                    for v in v.as_array().unwrap() {
                        ids.insert(
                            crate::args::parse_identifier(v.as_str().unwrap())
                                .ok_or(Error::Invalid)?,
                        );
                    }
                    Ok(Value::Array(ids.into_iter().map(Value::String).collect()))
                },
                Some(Value::Array(Vec::new())),
            )?;
            Ok(r.finish())
        })(),
        "minecraft:debug_stick_state" => v
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| {
                Ok((
                    crate::args::parse_identifier(k).ok_or(Error::Invalid)?,
                    v.clone(),
                ))
            })
            .collect::<Result<_, Error>>()
            .map(Value::Object),
        "minecraft:attribute_modifiers" => array(v, |v| {
            let mut r = Record::new(v);
            r.required("type", identifier)?;
            r.required("id", identifier)?;
            r.required("amount", double)?;
            r.required("operation", clone_value)?;
            r.optional("slot", clone_value, Some(Value::String("any".into())))?;
            r.optional(
                "display",
                |v| {
                    let m = v.as_object().unwrap();
                    let mut out = Map::from([("type".into(), m["type"].clone())]);
                    if m["type"].as_str() == Some("override") {
                        out.insert("value".into(), component(&m["value"], ctx)?);
                    }
                    Ok(Value::Object(out))
                },
                Some(Value::Object(Map::from([(
                    "type".into(),
                    Value::String("default".into()),
                )]))),
            )?;
            Ok(r.finish())
        }),
        "minecraft:bees" => array(v, |v| {
            let mut r = Record::new(v);
            r.required("entity_data", |v| {
                super::encode("minecraft:entity_data", v, ctx)
            })?;
            r.required("ticks_in_hive", integer)?;
            r.required("min_ticks_in_hive", integer)?;
            Ok(r.finish())
        }),
        "minecraft:sign_text_front" | "minecraft:sign_text_back" => (|| {
            let m = v.as_object().unwrap();
            let messages = array(&m["messages"], |v| component(v, ctx))?;
            let mut out = Map::from([
                ("messages".into(), messages.clone()),
                (
                    "color".into(),
                    present(m, "color")
                        .cloned()
                        .unwrap_or(Value::String("black".into())),
                ),
                (
                    "has_glowing_text".into(),
                    present(m, "has_glowing_text")
                        .cloned()
                        .unwrap_or(Value::Bool(false)),
                ),
            ]);
            if let Some(v) = present(m, "filtered_messages") {
                // The decoder's lenient Optional is absent for partial/error
                // lists. Exceptions and unavailable encoders remain explicit.
                if v.as_array().is_some_and(|a| a.len() == 4)
                    && v.as_array()
                        .unwrap()
                        .iter()
                        .all(|v| validate(v, ctx).is_ok())
                {
                    let filtered = array(v, |v| component(v, ctx))?;
                    if filtered != messages {
                        out.insert("filtered_messages".into(), filtered);
                    }
                }
            }
            Ok(Value::Object(out))
        })(),
        _ => return None,
    })
}
