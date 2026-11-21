//! Metadata-local Component encoding for flatRestrictedCodec's size check.
//! Unknown input keys are discarded by decoding. Input JSON length is never
//! used as the size of a decoded component.
use super::{
    Context, Error, R, content, present, validate,
    value::{Map, Value},
};
mod data;
mod dialog;
mod payload;
#[cfg(test)]
mod tests;

pub(super) fn item(
    m: &Map<String, Value>,
    ctx: &super::context::MetadataContext<'_>,
) -> Result<Value, Error> {
    ctx.item_template(m)?;
    let id = crate::args::parse_identifier(m["id"].as_str().unwrap()).ok_or(Error::Invalid)?;
    let mut out = Map::from([("id".into(), Value::String(id))]);
    if let Some(v) = present(m, "count") {
        let count = super::number::checked_int(v)?;
        if count != 1 {
            out.insert("count".into(), Value::Number(count.to_string()));
        }
    }
    if let Some(v) = present(m, "components") {
        // Validation above decodes every value, including losing entries.
        // DispatchedMapCodec retains decoded PatchKey order; DataComponentPatch
        // then puts by the registered type, replacing earlier set/removal values.
        // Reference2ObjectArrayMap retains the first type position on overwrite.
        let mut decoded = Map::new();
        for (name, v) in v.as_object().unwrap() {
            let (removed, name) = name
                .strip_prefix('!')
                .map_or((false, name.as_str()), |s| (true, s));
            let name = crate::args::parse_identifier(name).ok_or(Error::Invalid)?;
            decoded.insert(name, (removed, v));
        }
        let patch = decoded
            .into_iter()
            .map(|(name, (removed, v))| {
                let encoded = if removed {
                    Value::Object(Map::new())
                } else {
                    payload::encode(&name, v, ctx)?
                };
                Ok((format!("{}{name}", if removed { "!" } else { "" }), encoded))
            })
            .collect::<Result<Map<_, _>, Error>>()?;
        if !patch.is_empty() {
            out.insert("components".into(), Value::Object(patch));
        }
    }
    Ok(Value::Object(out))
}

pub(super) fn component(v: &Value, ctx: &dyn Context) -> Result<Value, Error> {
    validate(v, ctx)?;
    match v {
        Value::String(_) => Ok(v.clone()),
        Value::Array(a) => {
            let first = component(&a[0], ctx)?;
            if a.len() == 1 {
                return Ok(first);
            }
            let mut out = object(first);
            let extra = out
                .entry("extra".into())
                .or_insert_with(|| Value::Array(Vec::new()));
            let Value::Array(extra) = extra else {
                unreachable!()
            };
            for v in &a[1..] {
                extra.push(component(v, ctx)?);
            }
            Ok(Value::Object(out))
        }
        Value::Object(m) => {
            let kind = if let Some(v) = present(m, "type") {
                v.as_str().unwrap()
            } else {
                [
                    "text",
                    "translatable",
                    "keybind",
                    "score",
                    "selector",
                    "nbt",
                    "object",
                ]
                .into_iter()
                .find(|kind| content(kind, m, ctx).is_ok())
                .ok_or(Error::Invalid)?
            };
            let mut out = Map::new();
            match kind {
                "text" => copy(m, &mut out, "text"),
                "keybind" => copy(m, &mut out, "keybind"),
                "translatable" => {
                    copy(m, &mut out, "translate");
                    if present(m, "fallback").is_some_and(Value::is_string) {
                        copy(m, &mut out, "fallback");
                    }
                    if let Some(v) =
                        present(m, "with").filter(|v| !v.as_array().unwrap().is_empty())
                    {
                        let a = v
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| {
                                if v.is_number() {
                                    super::number::java_numeric(v)
                                } else if v.is_boolean() {
                                    Ok(v.clone())
                                } else {
                                    component(v, ctx)
                                }
                            })
                            .collect::<Result<_, _>>()?;
                        out.insert("with".into(), Value::Array(a));
                    }
                }
                "score" => {
                    let score = m["score"].as_object().unwrap();
                    let mut encoded = Map::new();
                    copy(score, &mut encoded, "name");
                    copy(score, &mut encoded, "objective");
                    out.insert("score".into(), Value::Object(encoded));
                }
                "selector" => {
                    copy(m, &mut out, "selector");
                    nested(m, &mut out, "separator", ctx)?;
                }
                // These contents require their decoded data source/profile
                // encoders; retaining the input map would be incorrect.
                "nbt" => {
                    copy(m, &mut out, "nbt");
                    for key in ["interpret", "plain"] {
                        if present(m, key).and_then(Value::as_bool) == Some(true) {
                            out.insert(key.into(), Value::Bool(true));
                        }
                    }
                    if let Some(v) = present(m, "separator") {
                        match component(v, ctx) {
                            Ok(v) => {
                                out.insert("separator".into(), v);
                            }
                            Err(Error::Invalid) => {}
                            Err(e) => return Err(e),
                        }
                    }
                    let source = present(m, "source")
                        .and_then(Value::as_str)
                        .or_else(|| {
                            ["entity", "block", "storage"].into_iter().find(|key| {
                                let mut input = Map::from([("nbt".into(), m["nbt"].clone())]);
                                if let Some(v) = m.get(*key) {
                                    input.insert((*key).into(), v.clone());
                                }
                                content("nbt", &input, ctx).is_ok()
                            })
                        })
                        .ok_or(Error::Invalid)?;
                    if source == "storage" {
                        out.insert(
                            source.into(),
                            Value::String(
                                crate::args::parse_identifier(m[source].as_str().unwrap())
                                    .ok_or(Error::Invalid)?,
                            ),
                        );
                    } else {
                        copy(m, &mut out, source);
                    }
                }
                "object" => {
                    // The object discriminator, like content/source, is omitted
                    // by the legacy matcher encoder.
                    let atlas = present(m, "object").and_then(Value::as_str).map_or_else(
                        || {
                            super::field(m, "sprite", super::id).is_ok()
                                && super::optional(m, "atlas", super::id).is_ok()
                        },
                        |s| s == "atlas",
                    );
                    if atlas {
                        out.insert(
                            "sprite".into(),
                            Value::String(
                                crate::args::parse_identifier(m["sprite"].as_str().unwrap())
                                    .ok_or(Error::Invalid)?,
                            ),
                        );
                        if let Some(v) = present(m, "atlas") {
                            let id = crate::args::parse_identifier(v.as_str().unwrap())
                                .ok_or(Error::Invalid)?;
                            if id != "minecraft:blocks" {
                                out.insert("atlas".into(), Value::String(id));
                            }
                        }
                    } else {
                        out.insert("player".into(), data::profile(&m["player"])?);
                        if present(m, "hat").and_then(Value::as_bool) == Some(false) {
                            out.insert("hat".into(), Value::Bool(false));
                        }
                    }
                    nested(m, &mut out, "fallback", ctx)?;
                }
                _ => return Err(Error::Invalid),
            }
            for key in [
                "bold",
                "italic",
                "underlined",
                "strikethrough",
                "obfuscated",
                "insertion",
            ] {
                copy(m, &mut out, key);
            }
            if let Some(v) = present(m, "font") {
                let id = crate::args::parse_identifier(v.as_str().ok_or(Error::IncompleteCodec)?)
                    .ok_or(Error::Invalid)?;
                out.insert("font".into(), Value::String(id));
            }
            if let Some(v) = present(m, "color") {
                let s = v.as_str().unwrap();
                let s = if let Some(hex) = s.strip_prefix('#') {
                    format!(
                        "#{:06x}",
                        i32::from_str_radix(hex, 16).map_err(|_| Error::Invalid)?
                    )
                } else {
                    s.to_owned()
                };
                out.insert("color".into(), Value::String(s));
            }
            if let Some(v) = present(m, "shadow_color") {
                out.insert("shadow_color".into(), data::color(v, true)?);
            }
            if let Some(v) = present(m, "click_event") {
                out.insert("click_event".into(), click(v, ctx)?);
            }
            if let Some(v) = present(m, "hover_event") {
                let m = v.as_object().unwrap();
                let action = m["action"].as_str().unwrap();
                let mut event = match action {
                    "show_text" => Map::from([("value".into(), component(&m["value"], ctx)?)]),
                    "show_item" => ctx.encode_item_template(m)?.as_object().unwrap().clone(),
                    "show_entity" => {
                        let id = crate::args::parse_identifier(m["id"].as_str().unwrap())
                            .ok_or(Error::Invalid)?;
                        let words = if let Some(s) = m["uuid"].as_str() {
                            super::java_uuid_words(s).ok_or(Error::Invalid)?
                        } else {
                            let a = m["uuid"].as_array().unwrap();
                            [
                                super::number::checked_int(&a[0])?,
                                super::number::checked_int(&a[1])?,
                                super::number::checked_int(&a[2])?,
                                super::number::checked_int(&a[3])?,
                            ]
                        };
                        let mut entity = Map::from([
                            ("id".into(), Value::String(id)),
                            (
                                "uuid".into(),
                                Value::Array(
                                    words
                                        .into_iter()
                                        .map(|n| Value::Number(n.to_string()))
                                        .collect(),
                                ),
                            ),
                        ]);
                        nested(m, &mut entity, "name", ctx)?;
                        entity
                    }
                    _ => return Err(Error::Invalid),
                };
                event.insert("action".into(), Value::String(action.into()));
                out.insert("hover_event".into(), Value::Object(event));
            }
            if let Some(v) = present(m, "extra") {
                out.insert(
                    "extra".into(),
                    Value::Array(
                        v.as_array()
                            .unwrap()
                            .iter()
                            .map(|v| component(v, ctx))
                            .collect::<Result<_, _>>()?,
                    ),
                );
            }
            if kind == "text" && out.len() == 1 {
                Ok(out.shift_remove("text").unwrap())
            } else {
                Ok(Value::Object(out))
            }
        }
        _ => Err(Error::Invalid),
    }
}
fn click(v: &Value, ctx: &dyn Context) -> Result<Value, Error> {
    let m = v.as_object().unwrap();
    let action = m["action"].as_str().unwrap();
    let mut out = Map::from([("action".into(), Value::String(action.into()))]);
    match action {
        "open_url" => copy(m, &mut out, "url"),
        "run_command" | "suggest_command" => copy(m, &mut out, "command"),
        "copy_to_clipboard" => copy(m, &mut out, "value"),
        "change_page" => {
            out.insert(
                "page".into(),
                Value::Number(super::number::checked_int(&m["page"])?.to_string()),
            );
        }
        "custom" => {
            if let Some(v) = present(m, "payload") {
                out.insert("payload".into(), data::nbt(v)?);
            }
            let id =
                crate::args::parse_identifier(m["id"].as_str().unwrap()).ok_or(Error::Invalid)?;
            out.insert("id".into(), Value::String(id));
        }
        "show_dialog" => {
            out.insert("dialog".into(), dialog::encode(&m["dialog"], ctx)?);
        }
        _ => return Err(Error::Invalid),
    }
    Ok(Value::Object(out))
}
fn object(v: Value) -> Map<String, Value> {
    match v {
        Value::String(s) => Map::from([("text".into(), Value::String(s))]),
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}
fn copy(m: &Map<String, Value>, out: &mut Map<String, Value>, key: &str) {
    if let Some(v) = present(m, key) {
        out.insert(key.into(), v.clone());
    }
}
fn nested(m: &Map<String, Value>, out: &mut Map<String, Value>, key: &str, ctx: &dyn Context) -> R {
    if let Some(v) = present(m, key) {
        out.insert(key.into(), component(v, ctx)?);
    }
    Ok(())
}
pub(super) fn encoded_len(v: &Value) -> usize {
    match v {
        Value::Null => 4,
        Value::Bool(v) => {
            if *v {
                4
            } else {
                5
            }
        }
        Value::Number(s) => s.len(),
        Value::String(s) => string_len(s),
        Value::Array(a) => 2 + a.len().saturating_sub(1) + a.iter().map(encoded_len).sum::<usize>(),
        Value::Object(m) => {
            2 + m.len().saturating_sub(1)
                + m.iter()
                    .map(|(k, v)| string_len(k) + 1 + encoded_len(v))
                    .sum::<usize>()
        }
    }
}
fn string_len(s: &str) -> usize {
    2 + s
        .chars()
        .map(|c| match c {
            '"' | '\\' | '\u{8}' | '\u{c}' | '\n' | '\r' | '\t' => 2,
            '\u{0}'..='\u{1f}' | '\u{2028}' | '\u{2029}' => 6,
            c => c.len_utf16(),
        })
        .sum::<usize>()
}
