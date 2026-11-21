//! Predicate codecs embedded in persistent values. This module only decodes
//! metadata; command matching and runtime predicate evaluation are unchanged.
use crate::component::json::all;
mod components;
use super::*;
use crate::component::json::value::Map;
use crate::component::json::{field, list, optional, present, require, string};

pub(super) fn validate(name: &str, v: &Value, ctx: &MetadataContext<'_>) -> Option<R> {
    Some(match name {
        "minecraft:can_break" | "minecraft:can_place_on" => {
            if v.is_array() {
                list(v, false, |v| block(v, ctx))
            } else {
                block(v, ctx)
            }
        }
        "minecraft:lock" => item(v, ctx),
        _ => return None,
    })
}
fn block(v: &Value, ctx: &MetadataContext<'_>) -> R {
    let m = map(v)?;
    let fields = all([
        optional(m, "blocks", fixed_holder_set),
        optional(m, "state", |v| {
            map(v)?.values().try_for_each(|v| {
                if v.is_string() {
                    string(v)
                } else {
                    let m = map(v)?;
                    all([optional(m, "min", string), optional(m, "max", string)])
                }
            })
        }),
        optional(m, "nbt", nbt),
    ]);
    all([fields, matchers(m, ctx)])
}
fn item(v: &Value, ctx: &MetadataContext<'_>) -> R {
    let m = map(v)?;
    let fields = all([
        optional(m, "items", fixed_holder_set),
        optional(m, "count", ints),
    ]);
    all([fields, matchers(m, ctx)])
}
fn matchers(m: &Map<String, Value>, ctx: &MetadataContext<'_>) -> R {
    all([
        optional(m, "components", |v| {
            let mut seen = std::collections::BTreeSet::new();
            all(map(v)?.iter().map(|(name, v)| {
                let name = crate::args::parse_identifier(name).ok_or(Error::Invalid)?;
                super::super::context::registered(&ctx.registries.persistent_components, &name)?;
                // DispatchedMapCodec parses the value before duplicate-key
                // rejection; an ordinary error cannot hide a thrown codec.
                all([
                    require(seen.insert(name.clone())),
                    super::validate(&name, v, ctx),
                ])
            }))
        }),
        optional(m, "predicates", |v| {
            let mut seen = std::collections::BTreeSet::new();
            all(map(v)?.iter().map(|(name, v)| {
                let name = crate::args::parse_identifier(name).ok_or(Error::Invalid)?;
                all([
                    require(seen.insert(name.clone())),
                    components::validate(&name, v, ctx),
                ])
            }))
        }),
    ])
}
fn nbt(v: &Value) -> R {
    // TagParser.LENIENT_CODEC and CustomData.COMPOUND_TAG_CODEC have the
    // same two alternatives, in opposite order: SNBT string or compound.
    nested::compound(v)
}
fn ints(v: &Value) -> R {
    if v.is_number() {
        return super::int(v, i32::MIN, i32::MAX);
    }
    let m = map(v)?;
    all([
        optional(m, "min", |v| super::int(v, i32::MIN, i32::MAX)),
        optional(m, "max", |v| super::int(v, i32::MIN, i32::MAX)),
    ])?;
    if let (Some(a), Some(b)) = (present(m, "min"), present(m, "max")) {
        require(super::super::number::checked_int(a)? <= super::super::number::checked_int(b)?)?;
    }
    Ok(())
}
fn doubles(v: &Value) -> R {
    let number = |v: &Value| {
        v.as_number()
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or(Error::Invalid)
    };
    if v.is_number() {
        return number(v).map(|_| ());
    }
    let m = map(v)?;
    all([
        optional(m, "min", |v| number(v).map(|_| ())),
        optional(m, "max", |v| number(v).map(|_| ())),
    ])?;
    if let (Some(a), Some(b)) = (present(m, "min"), present(m, "max")) {
        require(number(a)? <= number(b)?)?;
    }
    Ok(())
}
fn collection(v: &Value, check: impl Fn(&Value) -> R) -> R {
    let m = map(v)?;
    all([
        optional(m, "contains", |v| list(v, false, &check)),
        optional(m, "count", |v| {
            list(v, false, |v| {
                let m = map(v)?;
                all([field(m, "test", &check), field(m, "count", ints)])
            })
        }),
        optional(m, "size", ints),
    ])
}
