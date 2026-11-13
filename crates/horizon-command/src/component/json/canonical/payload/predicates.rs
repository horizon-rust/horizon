//! Persistent predicate payload encoding only. No runtime matcher is changed.
use super::*;
use records::{Record, clone_value, double};
mod components;

pub(super) fn encode(
    name: &str,
    v: &Value,
    ctx: &MetadataContext<'_>,
) -> Option<Result<Value, Error>> {
    Some(match name {
        "minecraft:can_break" | "minecraft:can_place_on" => {
            if let Some(a) = v.as_array() {
                let mut a = a
                    .iter()
                    .map(|v| block(v, ctx))
                    .collect::<Result<Vec<_>, _>>();
                a.as_mut()
                    .map(|a| {
                        if a.len() == 1 {
                            a.remove(0)
                        } else {
                            Value::Array(std::mem::take(a))
                        }
                    })
                    .map_err(|e| *e)
            } else {
                block(v, ctx)
            }
        }
        "minecraft:lock" => item(v, ctx),
        _ => return None,
    })
}
fn block(v: &Value, ctx: &MetadataContext<'_>) -> Result<Value, Error> {
    let mut r = Record::new(v);
    r.optional("blocks", clone_value, None)?;
    r.optional(
        "state",
        |v| {
            let m = v
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| {
                    let value = if v.is_string() {
                        v.clone()
                    } else {
                        let mut r = Record::new(v);
                        r.optional("min", clone_value, None)?;
                        r.optional("max", clone_value, None)?;
                        r.finish()
                    };
                    Ok((k.clone(), value))
                })
                .collect::<Result<_, Error>>()?;
            Ok(Value::Object(m))
        },
        None,
    )?;
    r.optional("nbt", super::super::data::flattened_compound, None)?;
    matchers(&mut r, ctx)?;
    Ok(r.finish())
}
fn item(v: &Value, ctx: &MetadataContext<'_>) -> Result<Value, Error> {
    let mut r = Record::new(v);
    r.optional("items", clone_value, None)?;
    r.optional("count", ints, Some(empty()))?;
    matchers(&mut r, ctx)?;
    Ok(r.finish())
}
fn matchers(r: &mut Record<'_>, ctx: &MetadataContext<'_>) -> R {
    r.optional(
        "components",
        |v| {
            let mut out = Map::new();
            for (name, value) in v.as_object().unwrap() {
                let name = crate::args::parse_identifier(name).unwrap();
                let value = super::encode(&name, value, ctx)?;
                out.insert(name, value);
            }
            Ok(Value::Object(out))
        },
        Some(empty()),
    )?;
    r.optional(
        "predicates",
        |v| {
            let mut out = Map::new();
            for (name, value) in v.as_object().unwrap() {
                let name = crate::args::parse_identifier(name).unwrap();
                let value = components::encode(&name, value, ctx)?;
                out.insert(name, value);
            }
            Ok(Value::Object(out))
        },
        Some(empty()),
    )
}
fn empty() -> Value {
    Value::Object(Map::new())
}
fn ints(v: &Value) -> Result<Value, Error> {
    bounds(v, integer)
}
fn doubles(v: &Value) -> Result<Value, Error> {
    bounds(v, double)
}
fn bounds(v: &Value, f: impl Fn(&Value) -> Result<Value, Error>) -> Result<Value, Error> {
    if v.is_number() {
        return f(v);
    }
    let m = v.as_object().unwrap();
    let mut out = Map::new();
    for k in ["min", "max"] {
        if let Some(v) = present(m, k) {
            out.insert(k.into(), f(v)?);
        }
    }
    if let (Some(a), Some(b)) = (out.get("min"), out.get("max")) {
        // Bounds.asPoint uses the decoded numeric value's equality.
        if a == b {
            return Ok(a.clone());
        }
    }
    Ok(Value::Object(out))
}
fn collection(v: &Value, f: impl Fn(&Value) -> Result<Value, Error>) -> Result<Value, Error> {
    let mut r = Record::new(v);
    r.optional("contains", |v| array(v, &f), None)?;
    r.optional(
        "count",
        |v| {
            array(v, |v| {
                let mut r = Record::new(v);
                r.required("test", &f)?;
                r.required("count", ints)?;
                Ok(r.finish())
            })
        },
        None,
    )?;
    r.optional("size", ints, None)?;
    Ok(r.finish())
}
