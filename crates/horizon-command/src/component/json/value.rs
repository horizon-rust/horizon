//! Admission-local JSON tree. Numeric tokens remain decimal lexemes rather
//! than passing through f64 before a JsonOps Number conversion. This leaves
//! serde_json and the command/NBT number parsers elsewhere unchanged.
use super::Error;
use serde_json::value::RawValue;
use std::{borrow::Cow, fmt};

// JsonOps iterates Gson object entries in insertion order. In particular,
// distinct set/removal PatchKeys collapse by component identity in that order.
// This map belongs only to metadata admission; generic JSON/NBT stays unchanged.
pub type Map<K, V> = indexmap::IndexMap<K, V>;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Map<String, Value>),
}

/// A caller with original metadata bytes must use `parse_first`; this
/// conversion exists for already-decoded callers and ordinary test values.
pub trait Input {
    fn node(&self) -> Cow<'_, Value>;
}
impl Input for Value {
    fn node(&self) -> Cow<'_, Value> {
        Cow::Borrowed(self)
    }
}
impl Input for serde_json::Value {
    fn node(&self) -> Cow<'_, Value> {
        Cow::Owned(Value::from(self))
    }
}
impl From<&serde_json::Value> for Value {
    fn from(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(v) => Self::Bool(*v),
            serde_json::Value::Number(v) => Self::Number(v.to_string()),
            serde_json::Value::String(v) => Self::String(v.clone()),
            serde_json::Value::Array(v) => Self::Array(v.iter().map(Self::from).collect()),
            serde_json::Value::Object(v) => {
                Self::Object(v.iter().map(|(k, v)| (k.clone(), Self::from(v))).collect())
            }
        }
    }
}

/// The existing metadata policy: a UTF-8 BOM is allowed and only the first
/// JSON value is consumed. No global JSON-number feature is enabled.
pub fn parse_first(bytes: &[u8]) -> Result<Value, Error> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let raw = serde_json::Deserializer::from_slice(bytes)
        .into_iter::<Box<RawValue>>()
        .next()
        .ok_or(Error::Invalid)?
        .map_err(|_| Error::Invalid)?;
    from_raw(&raw)
}
fn from_raw(raw: &RawValue) -> Result<Value, Error> {
    let s = raw.get();
    Ok(match s.as_bytes().first() {
        Some(b'n') => Value::Null,
        Some(b't') => Value::Bool(true),
        Some(b'f') => Value::Bool(false),
        Some(b'"') => Value::String(serde_json::from_str(s).map_err(|_| Error::Invalid)?),
        Some(b'[') => {
            let a: Vec<Box<RawValue>> = serde_json::from_str(s).map_err(|_| Error::Invalid)?;
            Value::Array(a.iter().map(|v| from_raw(v)).collect::<Result<_, _>>()?)
        }
        Some(b'{') => {
            let m: Map<String, Box<RawValue>> =
                serde_json::from_str(s).map_err(|_| Error::Invalid)?;
            Value::Object(
                m.into_iter()
                    .map(|(k, v)| from_raw(&v).map(|v| (k, v)))
                    .collect::<Result<_, _>>()?,
            )
        }
        Some(_) => Value::Number(s.to_owned()),
        None => return Err(Error::Invalid),
    })
}
impl Value {
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_number(&self) -> Option<&str> {
        if let Self::Number(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        self.as_number()?.parse().ok()
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(v) = self {
            Some(*v)
        } else {
            None
        }
    }
    pub fn as_object(&self) -> Option<&Map<String, Self>> {
        if let Self::Object(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_array(&self) -> Option<&Vec<Self>> {
        if let Self::Array(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
    pub fn is_boolean(&self) -> bool {
        matches!(self, Self::Bool(_))
    }
    pub fn is_number(&self) -> bool {
        matches!(self, Self::Number(_))
    }
    pub fn is_string(&self) -> bool {
        matches!(self, Self::String(_))
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }
    pub fn is_array(&self) -> bool {
        matches!(self, Self::Array(_))
    }
}
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("null"),
            Self::Bool(v) => write!(f, "{v}"),
            Self::Number(v) => f.write_str(v),
            Self::String(v) => f.write_str(&serde_json::to_string(v).map_err(|_| fmt::Error)?),
            Self::Array(a) => {
                f.write_str("[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?
                    }
                    write!(f, "{v}")?
                }
                f.write_str("]")
            }
            Self::Object(m) => {
                f.write_str("{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?
                    }
                    write!(
                        f,
                        "{}:{v}",
                        serde_json::to_string(k).map_err(|_| fmt::Error)?
                    )?
                }
                f.write_str("}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::json::{context::UnavailableContext, validate};

    #[test]
    fn raw_decimal_tokens_follow_the_admitted_jsonops_conversions() {
        for (lexeme, expected) in [
            ("9007199254740993", true),
            ("0.99999999999999999999999999999", false),
            ("1e400", false),
            ("1e2147483647", false),
            ("1e2147483648", false),
            ("1e-2147483649", false),
        ] {
            let bytes = format!(
                r#"{{"text":"body","click_event":{{"action":"change_page","page":{lexeme}}}}}"#
            );
            let value = parse_first(bytes.as_bytes()).unwrap();
            assert_eq!(
                validate(&value, &UnavailableContext).is_ok(),
                expected,
                "{lexeme}"
            );
        }
        let value = parse_first(br#"{"translate":"key","with":[1e400]}"#).unwrap();
        assert_eq!(validate(&value, &UnavailableContext), Ok(()));
    }

    #[test]
    fn raw_tree_keeps_the_existing_first_value_and_bom_policy() {
        let value = parse_first(b"\xef\xbb\xbf{\"n\":9007199254740993} // signature").unwrap();
        assert_eq!(
            value.as_object().unwrap()["n"].as_number(),
            Some("9007199254740993")
        );
        assert!(parse_first(br#"{"x":]"#).is_err());
        assert_eq!(parse_first(b"true false"), Ok(Value::Bool(true)));
    }
}
