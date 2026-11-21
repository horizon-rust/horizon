//! Sign and writable-book codecs. Written-page size requires the decoded
//! component's canonical encoding, not the input JSON's length.
use super::*;
use crate::component::json::all;
use crate::component::json::{boolean, field, lenient, optional, require, validate as text};

pub(super) fn validate(name: &str, v: &Value, ctx: &MetadataContext<'_>) -> Option<R> {
    Some(match name {
        "minecraft:sign_text_front" | "minecraft:sign_text_back" => (|| {
            let m = map(v)?;
            all([
                field(m, "messages", |v| messages(v, ctx)),
                lenient(m, "filtered_messages", |v| messages(v, ctx)),
                optional(m, "color", nested::dye),
                optional(m, "has_glowing_text", boolean),
            ])
        })(),
        "minecraft:writable_book_content" => map(v).and_then(|m| {
            optional(m, "pages", |v| {
                list_limit(v, 100, |v| filterable(v, |v| bounded_string(v, 1024)))
            })
        }),
        "minecraft:written_book_content" => (|| {
            let m = map(v)?;
            all([
                field(m, "title", |v| filterable(v, |v| bounded_string(v, 32))),
                field(m, "author", super::super::string),
                optional(m, "generation", |v| int(v, 0, 3)),
                optional(m, "resolved", boolean),
                optional(m, "pages", |v| {
                    super::super::list(v, false, |v| filterable(v, |v| restricted_page(v, ctx)))
                }),
            ])
        })(),
        _ => return None,
    })
}
fn restricted_page(v: &Value, ctx: &MetadataContext<'_>) -> R {
    text(v, ctx)?;
    let encoded = super::super::canonical::component(v, ctx)?;
    require(super::super::canonical::encoded_len(&encoded) <= 32767)
}
fn messages(v: &Value, ctx: &MetadataContext<'_>) -> R {
    let a = v.as_array().ok_or(Error::Invalid)?;
    all([require(a.len() == 4), all(a.iter().map(|v| text(v, ctx)))])
}
pub(super) fn bounded_string(v: &Value, max: usize) -> R {
    require(v.as_str().is_some_and(|s| s.encode_utf16().count() <= max))
}
pub(super) fn filterable(v: &Value, check: impl Fn(&Value) -> R) -> R {
    if let Ok(m) = map(v) {
        let raw = field(m, "raw", &check);
        let filtered = optional(m, "filtered", &check);
        if raw == Err(Error::DecodeException) || filtered == Err(Error::DecodeException) {
            return Err(Error::DecodeException);
        }
        if raw.is_ok() && filtered.is_ok() {
            return Ok(());
        }
        match check(v) {
            Ok(()) => Ok(()),
            Err(Error::Invalid) => all([raw, filtered]),
            result => result,
        }
    } else {
        check(v)
    }
}
