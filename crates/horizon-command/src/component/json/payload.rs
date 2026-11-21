//! Source-specific persistent DataComponents payload codecs. No generic NBT
//! decoder or successful default substitutes for a registered value codec.
mod combat;
mod consumption;
mod effects;
mod enums;
mod nested;
mod predicates;
mod records;
mod scalar;
#[cfg(test)]
mod tests;
mod written;
use super::{Context, Error, R, context::MetadataContext, value::Value};

pub(super) fn validate(id: &str, value: &Value, context: &MetadataContext<'_>) -> R {
    if let Some(result) = scalar::validate(id, value, context) {
        return result;
    }
    if let Some(result) = records::validate(id, value, context) {
        return result;
    }
    if let Some(result) = combat::validate(id, value) {
        return result;
    }
    if let Some(result) = effects::validate(id, value, context) {
        return result;
    }
    if let Some(result) = consumption::validate(id, value, context) {
        return result;
    }
    if let Some(result) = written::validate(id, value, context) {
        return result;
    }
    if let Some(result) = predicates::validate(id, value, context) {
        return result;
    }
    if let Some(result) = enums::validate(id, value) {
        return result;
    }
    if let Some(result) = nested::validate(id, value, context) {
        return result;
    }
    // This explicit implementation gap cannot be treated as a valid negative.
    // Admission wiring is prohibited until the pinned coverage check has no
    // IncompleteCodec result for any of its 119 persistent families.
    Err(Error::IncompleteCodec)
}
fn int(value: &Value, min: i32, max: i32) -> R {
    super::require((min..=max).contains(&super::number::checked_int(value)?))
}
fn float(value: &Value) -> Result<f32, Error> {
    value
        .as_number()
        .and_then(|s| s.parse().ok())
        .ok_or(Error::Invalid)
}
fn range(value: &Value, min: f32, max: f32) -> R {
    let n = float(value)?;
    super::require(n >= min && n <= max)
}
fn positive(value: &Value) -> R {
    let n = float(value)?;
    super::require(n > 0.0 && n <= f32::MAX)
}
fn map(value: &Value) -> Result<&super::value::Map<String, Value>, Error> {
    value.as_object().ok_or(Error::Invalid)
}
fn one_of(value: &Value, names: &[&str]) -> R {
    super::require(value.as_str().is_some_and(|s| names.contains(&s)))
}
fn template(value: &Value, ctx: &dyn Context) -> R {
    if let Some(id) = value.as_str() {
        let mut m = super::value::Map::new();
        m.insert("id".into(), Value::String(id.into()));
        ctx.item_template(&m)
    } else {
        ctx.item_template(map(value)?)
    }
}
fn list_limit(value: &Value, max: usize, check: impl Fn(&Value) -> R) -> R {
    let a = value.as_array().ok_or(Error::Invalid)?;
    super::all([
        super::require(a.len() <= max),
        super::all(a.iter().map(check)),
    ])
}
fn fixed_holder_set(value: &Value) -> R {
    // A fixed dynamic holder has no decoding route without RegistryOps.
    // Its list alternative can still fully decode an empty direct set.
    super::require(value.as_array().is_some_and(Vec::is_empty))
}
fn catalog(value: &Value, registry: &str, ctx: &MetadataContext<'_>) -> R {
    let id = crate::args::parse_identifier(value.as_str().ok_or(Error::Invalid)?)
        .ok_or(Error::Invalid)?;
    super::context::registered(ctx.registries.ids(registry), &id)
}
