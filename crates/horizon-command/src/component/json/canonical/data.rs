use super::*;
use horizon_world::nbt::Tag;

pub(super) fn profile(v: &Value) -> Result<Value, Error> {
    super::super::profile::validate(v)?;
    if v.is_string() {
        return Ok(Value::Object(Map::from([("name".into(), v.clone())])));
    }
    let m = v.as_object().unwrap();
    let mut out = Map::new();
    copy(m, &mut out, "name");
    if let Some(v) = present(m, "id") {
        out.insert(
            "id".into(),
            Value::Array(
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| {
                        Ok(Value::Number(
                            super::super::number::checked_int(v)?.to_string(),
                        ))
                    })
                    .collect::<Result<_, Error>>()?,
            ),
        );
    }
    if let Some(v) = present(m, "properties") {
        let mut properties = Vec::new();
        match v {
            Value::Object(m) => {
                for (name, values) in m {
                    for v in values.as_array().unwrap() {
                        properties.push(Value::Object(Map::from([
                            ("name".into(), Value::String(name.clone())),
                            ("value".into(), v.clone()),
                        ])));
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    let mut p = Map::new();
                    let m = v.as_object().unwrap();
                    for key in ["name", "value", "signature"] {
                        copy(m, &mut p, key);
                    }
                    properties.push(Value::Object(p));
                }
            }
            _ => unreachable!(),
        }
        if !properties.is_empty() {
            out.insert("properties".into(), Value::Array(properties));
        }
    }
    for key in ["texture", "cape", "elytra"] {
        if let Some(v) = present(m, key) {
            out.insert(
                key.into(),
                Value::String(
                    crate::args::parse_identifier(v.as_str().unwrap()).ok_or(Error::Invalid)?,
                ),
            );
        }
    }
    copy(m, &mut out, "model");
    Ok(Value::Object(out))
}
pub(super) fn nbt(v: &Value) -> Result<Value, Error> {
    match v {
        Value::Null => Err(Error::Invalid),
        Value::Bool(v) => Ok(Value::Number(if *v { "1" } else { "0" }.into())),
        Value::Number(_) => super::super::number::java_numeric(v),
        Value::String(_) => Ok(v.clone()),
        Value::Array(a) => Ok(Value::Array(a.iter().map(nbt).collect::<Result<_, _>>()?)),
        Value::Object(m) => Ok(Value::Object(
            m.iter()
                .map(|(k, v)| Ok((k.clone(), nbt(v)?)))
                .collect::<Result<_, Error>>()?,
        )),
    }
}
pub(super) fn color(v: &Value, alpha: bool) -> Result<Value, Error> {
    if v.is_number() {
        return Ok(Value::Number(
            super::super::number::checked_int(v)?.to_string(),
        ));
    }
    let a = v.as_array().ok_or(Error::Invalid)?;
    let channel = |v: &Value| -> Result<u32, Error> {
        let f = v
            .as_number()
            .unwrap()
            .parse::<f32>()
            .map_err(|_| Error::Invalid)?
            * 255.0;
        let n = f as i32;
        let n = if f < n as f32 { n.wrapping_sub(1) } else { n };
        Ok((n & 255) as u32)
    };
    let rgb = (channel(&a[0])? << 16) | (channel(&a[1])? << 8) | channel(&a[2])?;
    let packed = if alpha {
        (channel(&a[3])? << 24) | rgb
    } else {
        0xff00_0000 | rgb
    };
    Ok(Value::Number((packed as i32).to_string()))
}
pub(super) fn compound(v: &Value) -> Result<Value, Error> {
    if let Some(s) = v.as_str() {
        tag(&crate::snbt::parse_fully(s).map_err(|_| Error::Invalid)?)
    } else {
        nbt(v)
    }
}
fn tag(t: &Tag) -> Result<Value, Error> {
    match t {
        Tag::End => Err(Error::Invalid),
        Tag::Byte(n) => Ok(Value::Number(n.to_string())),
        Tag::Short(n) => Ok(Value::Number(n.to_string())),
        Tag::Int(n) => Ok(Value::Number(n.to_string())),
        Tag::Long(n) => Ok(Value::Number(n.to_string())),
        Tag::Float(n) => {
            let n = f32::from_bits(*n);
            if !n.is_finite() {
                return Err(Error::DecodeException);
            }
            Ok(Value::Number(horizon_world::snbt_text::java_float(n)))
        }
        Tag::Double(n) => {
            let n = f64::from_bits(*n);
            if !n.is_finite() {
                return Err(Error::DecodeException);
            }
            Ok(Value::Number(horizon_world::snbt_text::java_double(n)))
        }
        Tag::String(s) => Ok(Value::String(s.to_string_lossy())),
        Tag::List(a) => Ok(Value::Array(a.iter().map(tag).collect::<Result<_, _>>()?)),
        Tag::Compound(m) => Ok(Value::Object(
            m.iter()
                .map(|(k, v)| Ok((k.to_string_lossy(), tag(v)?)))
                .collect::<Result<_, Error>>()?,
        )),
        Tag::ByteArray(a) => Ok(Value::Array(
            a.iter()
                .map(|n| Value::Number((*n as i8).to_string()))
                .collect(),
        )),
        Tag::IntArray(a) => Ok(Value::Array(
            a.iter().map(|n| Value::Number(n.to_string())).collect(),
        )),
        Tag::LongArray(a) => Ok(Value::Array(
            a.iter().map(|n| Value::Number(n.to_string())).collect(),
        )),
    }
}

// NbtPredicate encodes through TagParser.FLATTENED_CODEC, whose output is
// StringTagVisitor text, rather than CompoundTag.CODEC's JSON object.
pub(super) fn flattened_compound(v: &Value) -> Result<Value, Error> {
    let tag = if let Some(s) = v.as_str() {
        crate::snbt::parse_fully(s).map_err(|_| Error::Invalid)?
    } else {
        json_tag(v)?
    };
    Ok(Value::String(horizon_world::snbt_text::to_snbt(&tag)))
}
fn json_tag(v: &Value) -> Result<Tag, Error> {
    Ok(match v {
        Value::Null => return Err(Error::Invalid),
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Array(a) => Tag::List(a.iter().map(json_tag).collect::<Result<_, _>>()?),
        Value::Object(m) => Tag::Compound(
            m.iter()
                .map(|(k, v)| Ok((k.as_str().into(), json_tag(v)?)))
                .collect::<Result<_, Error>>()?,
        ),
        Value::Number(raw) => {
            super::super::number::nbt(v)?;
            if let Some(n) = super::super::number::exact_long(raw) {
                if let Ok(n) = i8::try_from(n) {
                    Tag::Byte(n)
                } else if let Ok(n) = i16::try_from(n) {
                    Tag::Short(n)
                } else if let Ok(n) = i32::try_from(n) {
                    Tag::Int(n)
                } else {
                    Tag::Long(n)
                }
            } else {
                let d = raw.parse::<f64>().map_err(|_| Error::DecodeException)?;
                if f64::from(d as f32) == d {
                    Tag::Float((d as f32).to_bits())
                } else {
                    Tag::Double(d.to_bits())
                }
            }
        }
    })
}
