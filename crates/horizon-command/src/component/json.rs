//! Component codec admission in plain JsonOps (pack metadata), independently
//! of the NbtOps command decoder and of permissive text rendering.
mod canonical;
pub mod context;
pub mod dialog;
mod item;
mod number;
mod payload;
pub mod profile;
#[cfg(test)]
mod tests;
pub mod value;
mod variable;
use value::{Input, Map, Value};

/// The existing registry and nested codec authority supplied by the caller.
/// These checks must report unavailable authority separately from invalid data.
/// No default implementation admits a payload without its actual codec.
pub trait Context {
    fn item_template(&self, value: &Map<String, Value>) -> Result<(), Error>;
    /// The same decoded value's JsonOps encoding, needed only by restricted
    /// written pages. A validation-only context cannot invent this encoding.
    fn encode_item_template(&self, _: &Map<String, Value>) -> Result<Value, Error> {
        Err(Error::Unavailable)
    }
    fn entity_type(&self, id: &str) -> Result<(), Error>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unavailable,
    /// A pinned registered payload codec is still under implementation.
    /// Caller integration requires zero reach here for all 119 families.
    IncompleteCodec,
    /// JsonOps conversion throws rather than returning a DataResult error
    /// (numeric conversion or a null child in a converted stream/map).
    /// Lenient optional fields cannot discard this.
    DecodeException,
}
type R = Result<(), Error>;
// Record/list codecs evaluate all child decoders. A DataResult error in an
// earlier field cannot suppress a numeric exception in a later child.
fn all(results: impl IntoIterator<Item = R>) -> R {
    fn rank(value: R) -> u8 {
        match value {
            Ok(()) => 0,
            Err(Error::Invalid) => 1,
            Err(Error::Unavailable) => 2,
            Err(Error::IncompleteCodec) => 3,
            Err(Error::DecodeException) => 4,
        }
    }
    results
        .into_iter()
        .max_by_key(|value| rank(*value))
        .unwrap_or(Ok(()))
}
fn require(value: bool) -> R {
    if value { Ok(()) } else { Err(Error::Invalid) }
}
fn present<'a>(map: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    map.get(name).filter(|v| !v.is_null())
}
fn optional(map: &Map<String, Value>, name: &str, check: impl FnOnce(&Value) -> R) -> R {
    match present(map, name) {
        Some(v) => check(v),
        None => Ok(()),
    }
}
fn string(v: &Value) -> R {
    require(v.is_string())
}
fn boolean(v: &Value) -> R {
    require(v.is_boolean())
}
fn field(map: &Map<String, Value>, name: &str, check: impl FnOnce(&Value) -> R) -> R {
    check(map.get(name).ok_or(Error::Invalid)?)
}
fn list(v: &Value, nonempty: bool, check: impl Fn(&Value) -> R) -> R {
    let values = v.as_array().ok_or(Error::Invalid)?;
    require(!nonempty || !values.is_empty())?;
    all(values.iter().map(check))
}
fn id(v: &Value) -> R {
    require(
        v.as_str()
            .is_some_and(|s| crate::args::parse_identifier(s).is_some()),
    )
}
fn selector(v: &Value) -> R {
    let s = v.as_str().ok_or(Error::Invalid)?;
    let reg = crate::Registries {
        permission: 4,
        ..Default::default()
    };
    let mut reader = crate::Reader::new(s);
    require(crate::selector::parse(&mut reader, &reg, true).is_ok() && !reader.can_read())
}
fn block_pos(v: &Value) -> R {
    let s = v.as_str().ok_or(Error::Invalid)?;
    let mut reader = crate::Reader::new(s);
    require(
        crate::ArgType::BlockPos
            .parse(&mut reader, &crate::Registries::default())
            .is_ok()
            && !reader.can_read(),
    )
}
/// Full success only. In particular, a codec error carrying a partial component
/// is rejected just as ResourceMetadata.getSection's getOrThrow rejects it.
pub fn validate(value: &impl Input, context: &dyn Context) -> R {
    let node = value.node();
    validate_node(&node, context)
}
fn validate_node(value: &Value, context: &dyn Context) -> R {
    match value {
        Value::String(_) => Ok(()),
        Value::Array(_) => list(value, true, |v| validate(v, context)),
        Value::Object(map) => {
            let kinds = [
                "text",
                "translatable",
                "keybind",
                "score",
                "selector",
                "nbt",
                "object",
            ];
            let contents = (|| {
                if let Some(kind) = present(map, "type") {
                    let kind = kind.as_str().ok_or(Error::Invalid)?;
                    require(kinds.contains(&kind))?;
                    content(kind, map, context)?;
                } else {
                    let mut unavailable = None;
                    let mut matched = false;
                    for kind in kinds {
                        match content(kind, map, context) {
                            Ok(()) => {
                                matched = true;
                                break;
                            }
                            Err(e @ (Error::Unavailable | Error::IncompleteCodec)) => {
                                unavailable = Some(e)
                            }
                            Err(Error::Invalid) => {}
                            Err(Error::DecodeException) => return Err(Error::DecodeException),
                        }
                    }
                    if !matched {
                        return Err(unavailable.unwrap_or(Error::Invalid));
                    }
                }
                Ok(())
            })();
            all([
                contents,
                optional(map, "extra", |v| list(v, true, |v| validate(v, context))),
                style(map, context),
            ])
        }
        _ => Err(Error::Invalid),
    }
}
fn content(kind: &str, map: &Map<String, Value>, context: &dyn Context) -> R {
    match kind {
        "text" => field(map, "text", string),
        "translatable" => all([
            field(map, "translate", string),
            lenient(map, "fallback", string),
            optional(map, "with", |v| {
                list(v, false, |v| {
                    // ARG_CODEC tries ExtraCodecs.JAVA before the Component
                    // alternative, including conversion of ignored map keys.
                    java_conversion(v)?;
                    if v.is_string() || v.is_number() || v.is_boolean() {
                        Ok(())
                    } else {
                        validate(v, context)
                    }
                })
            }),
        ]),
        "keybind" => field(map, "keybind", string),
        "score" => {
            let score = map
                .get("score")
                .and_then(Value::as_object)
                .ok_or(Error::Invalid)?;
            // ScoreContents' name is Either(compilable selector, literal string).
            field(score, "name", string)?;
            field(score, "objective", string)
        }
        "selector" => all([
            field(map, "selector", selector),
            optional(map, "separator", |v| validate(v, context)),
        ]),
        "nbt" => {
            let path = field(map, "nbt", |v| {
                require(
                    v.as_str()
                        .is_some_and(|s| horizon_client_core::nbt_path::NbtPath::parse(s).is_ok()),
                )
            });
            let separator = lenient(map, "separator", |v| validate(v, context));
            let flags = require(
                !(present(map, "interpret").and_then(Value::as_bool) == Some(true)
                    && present(map, "plain").and_then(Value::as_bool) == Some(true)),
            );
            // interpret/plain/separator use lenientOptionalFieldOf: malformed
            // optional values become the default, unlike style fields below.
            let source = if let Some(source) = present(map, "source") {
                match source.as_str() {
                    Some("entity") => field(map, "entity", selector),
                    Some("block") => field(map, "block", block_pos),
                    Some("storage") => field(map, "storage", id),
                    _ => Err(Error::Invalid),
                }
            } else {
                let mut matched = false;
                for (name, check) in [
                    ("entity", selector as fn(&Value) -> R),
                    ("block", block_pos),
                    ("storage", id),
                ] {
                    if map.get(name).is_some_and(|v| check(v).is_ok()) {
                        matched = true;
                        break;
                    }
                }
                require(matched)
            };
            all([path, separator, flags, source])
        }
        "object" => {
            let atlas = || -> R {
                field(map, "sprite", id)?;
                optional(map, "atlas", id)
            };
            let player = || -> R {
                field(map, "player", profile::validate)?;
                optional(map, "hat", boolean)
            };
            let info = (|| {
                match present(map, "object") {
                    Some(v) => match v.as_str() {
                        Some("atlas") => atlas()?,
                        Some("player") => player()?,
                        _ => return Err(Error::Invalid),
                    },
                    None => atlas().or_else(|_| player())?,
                };
                Ok(())
            })();
            all([info, optional(map, "fallback", |v| validate(v, context))])
        }
        _ => Err(Error::Invalid),
    }
}
fn style(map: &Map<String, Value>, context: &dyn Context) -> R {
    let mut checks = vec![optional(map, "color", |v| {
        let s = v.as_str().ok_or(Error::Invalid)?;
        if let Some(hex) = s.strip_prefix('#') {
            let digits = hex.strip_prefix(['+', '-']).unwrap_or(hex);
            require(
                !digits.is_empty()
                    && digits.bytes().all(|c| c.is_ascii_hexdigit())
                    && i32::from_str_radix(hex, 16).is_ok_and(|n| (0..=0xff_ffff).contains(&n)),
            )
        } else {
            require(horizon_client_core::text::named_color(s).is_some())
        }
    })];
    for name in [
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
    ] {
        checks.push(optional(map, name, boolean));
    }
    checks.push(optional(map, "shadow_color", |v| {
        if v.is_number() {
            number::checked_int(v).map(|_| ())
        } else {
            require(
                v.as_array()
                    .is_some_and(|a| a.len() == 4 && a.iter().all(Value::is_number)),
            )
        }
    }));
    checks.push(optional(map, "insertion", string));
    checks.push(optional(map, "font", id));
    checks.push(optional(map, "click_event", |v| click(v, context)));
    checks.push(optional(map, "hover_event", |v| hover(v, context)));
    all(checks)
}
fn click(value: &Value, context: &dyn Context) -> R {
    let map = value.as_object().ok_or(Error::Invalid)?;
    match map
        .get("action")
        .and_then(Value::as_str)
        .ok_or(Error::Invalid)?
    {
        "open_url" => field(map, "url", |v| {
            require(
                v.as_str()
                    .is_some_and(horizon_client_core::component::untrusted_http_url),
            )
        }),
        "run_command" | "suggest_command" => field(map, "command", string),
        "change_page" => field(map, "page", |v| require(number::checked_int(v)? > 0)),
        "copy_to_clipboard" => field(map, "value", string),
        "custom" => all([field(map, "id", id), optional(map, "payload", nbt_payload)]),
        "show_dialog" => field(map, "dialog", |v| dialog::validate(v, context)),
        // ClickEvent.Action.CODEC refuses OPEN_FILE in untrusted metadata.
        _ => Err(Error::Invalid),
    }
}
fn hover(value: &Value, context: &dyn Context) -> R {
    let map = value.as_object().ok_or(Error::Invalid)?;
    match map
        .get("action")
        .and_then(Value::as_str)
        .ok_or(Error::Invalid)?
    {
        "show_text" => field(map, "value", |v| validate(v, context)),
        "show_item" => context.item_template(map),
        "show_entity" => all([
            field(map, "id", |v| {
                id(v)?;
                context.entity_type(
                    &crate::args::parse_identifier(v.as_str().unwrap()).ok_or(Error::Invalid)?,
                )
            }),
            field(map, "uuid", uuid),
            optional(map, "name", |v| validate(v, context)),
        ]),
        _ => Err(Error::Invalid),
    }
}
fn uuid(value: &Value) -> R {
    if let Some(s) = value.as_str() {
        return java_uuid_words(s).map(|_| ()).ok_or(Error::Invalid);
    }
    let a = value.as_array().ok_or(Error::Invalid)?;
    all([
        require(a.len() == 4),
        all(a.iter().map(|v| number::checked_int(v).map(|_| ()))),
    ])
}

// ExtraCodecs.NBT converts a present JSON value with NbtOps. End tags cannot
// be inserted into compounds or lists, so nested JSON null is not valid NBT.
fn nbt_payload(value: &Value) -> R {
    if value.is_null() {
        Err(Error::Invalid)
    } else {
        java_conversion(value)
    }
}
// The JAVA and NBT codecs both use JsonOps.convertTo. getMapValues/getStream
// replace a JsonNull child by Java null; converting that child throws. A root
// JsonNull is different: convertTo recognizes it and returns the target empty.
fn java_conversion(value: &Value) -> R {
    let child = |value: &Value| {
        if value.is_null() {
            Err(Error::DecodeException)
        } else {
            java_conversion(value)
        }
    };
    match value {
        Value::Array(a) => all(a.iter().map(child)),
        Value::Object(m) => all(m.values().map(child)),
        Value::Number(_) => number::nbt(value),
        _ => Ok(()),
    }
}

fn lenient(map: &Map<String, Value>, name: &str, check: impl FnOnce(&Value) -> R) -> R {
    match optional(map, name, check) {
        Ok(()) | Err(Error::Invalid) => Ok(()),
        Err(error) => Err(error),
    }
}

// UUIDUtil's string alternative uses UUID.fromString. Its five hexadecimal
// words may be shorter than the canonical widths; each is a signed long.
fn java_uuid_words(s: &str) -> Option<[i32; 4]> {
    if s.encode_utf16().count() > 36 {
        return None;
    }
    let words: Vec<_> = s.split('-').collect();
    if words.len() != 5 {
        return None;
    }
    let mut parsed = Vec::with_capacity(5);
    for word in words {
        let word = word.strip_prefix('+').unwrap_or(word);
        if word.is_empty() {
            return None;
        }
        let mut value = 0_i64;
        for c in word.chars() {
            let digit = c
                .to_digit(16)
                .or_else(|| {
                    horizon_core::text::parse_java_integer(&c.to_string()).map(|n| n as u32)
                })
                .or_else(|| match c as u32 {
                    n @ 0xff21..=0xff26 => Some(n - 0xff21 + 10),
                    n @ 0xff41..=0xff46 => Some(n - 0xff41 + 10),
                    _ => None,
                });
            let digit = digit?;
            let next = value
                .checked_mul(16)
                .and_then(|n| n.checked_add(i64::from(digit)))?;
            value = next;
        }
        parsed.push(value as u64);
    }
    let high =
        ((parsed[0] & 0xffff_ffff) << 32) | ((parsed[1] & 0xffff) << 16) | (parsed[2] & 0xffff);
    let low = ((parsed[3] & 0xffff) << 48) | (parsed[4] & 0xffff_ffff_ffff);
    Some([
        (high >> 32) as i32,
        high as i32,
        (low >> 32) as i32,
        low as i32,
    ])
}

#[cfg(test)]
mod review_tests;
