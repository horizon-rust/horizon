use super::super::value::Value;
use super::super::{
    Context, Error, R, boolean, field, list, optional, present, require, string,
    validate as component, variable,
};
use super::{bounded, kind, map, positive, width};
use crate::component::json::all;

pub(super) fn validate(value: &Value, context: &dyn Context) -> R {
    let map = map(value)?;
    let kind = kind(map)?;
    let common = all([
        field(map, "key", variable::name),
        field(map, "label", |v| component(v, context)),
    ]);
    let specific = (|| match kind.as_str() {
        "minecraft:boolean" => all([
            optional(map, "initial", boolean),
            optional(map, "on_true", string),
            optional(map, "on_false", string),
        ]),
        "minecraft:text" => {
            let fields = all([
                optional(map, "width", width),
                optional(map, "label_visible", boolean),
                optional(map, "initial", string),
                optional(map, "max_length", positive),
            ]);
            let max = present(map, "max_length")
                .and_then(super::super::number::int)
                .unwrap_or(32);
            let initial = require(
                present(map, "initial")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .encode_utf16()
                    .count() as i64
                    <= i64::from(max),
            );
            let multiline = optional(map, "multiline", |v| {
                let map = self::map(v)?;
                all([
                    optional(map, "max_lines", positive),
                    optional(map, "height", |v| bounded(v, 1, 512)),
                ])
            });
            all([fields, initial, multiline])
        }
        "minecraft:single_option" => {
            // RecordCodecBuilder evaluates the options even when width
            // returns a DataResult error. A nested conversion exception must
            // survive a lenient optional Component around this dialog.
            let fields = all([
                optional(map, "width", width),
                optional(map, "label_visible", boolean),
            ]);
            let options = field(map, "options", |options| {
                let entries = list(options, true, |v| {
                    if v.is_string() {
                        return Ok(());
                    }
                    let map = self::map(v)?;
                    all([
                        field(map, "id", string),
                        optional(map, "display", |v| component(v, context)),
                        optional(map, "initial", boolean),
                    ])
                });
                let initial = options.as_array().ok_or(Error::Invalid).and_then(|a| {
                    require(
                        a.iter()
                            .filter(|v| {
                                v.as_object()
                                    .and_then(|m| present(m, "initial"))
                                    .and_then(Value::as_bool)
                                    == Some(true)
                            })
                            .count()
                            <= 1,
                    )
                });
                all([entries, initial])
            });
            all([fields, options])
        }
        "minecraft:number_range" => {
            all([
                optional(map, "width", width),
                optional(map, "label_format", string),
            ])?;
            let start = float(map.get("start").ok_or(Error::Invalid)?)?;
            let end = float(map.get("end").ok_or(Error::Invalid)?)?;
            all([
                optional(map, "initial", |v| {
                    let n = float(v)?;
                    require(n >= start.min(end) && n <= start.max(end))
                }),
                optional(map, "step", |v| {
                    let n = float(v)?;
                    require(n > 0.0 && n <= f32::MAX)
                }),
            ])
        }
        _ => Err(Error::Invalid),
    })();
    all([common, specific])
}
fn float(value: &Value) -> Result<f32, Error> {
    value
        .as_number()
        .and_then(|n| n.parse().ok())
        .ok_or(Error::Invalid)
}
