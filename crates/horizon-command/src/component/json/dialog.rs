//! Context-free dialog codecs used by Component click events. Registry holder
//! references remain invalid in plain JsonOps; inline dialogs decode normally.
use crate::component::json::all;
mod input;
#[cfg(test)]
mod tests;
use super::value::{Input, Map, Value};
use super::{
    Context, Error, R, boolean, field, id, list, nbt_payload, number, optional, present, require,
    validate as component, variable,
};

fn map(value: &Value) -> Result<&Map<String, Value>, Error> {
    value.as_object().ok_or(Error::Invalid)
}
fn kind(map: &Map<String, Value>) -> Result<String, Error> {
    let name = map
        .get("type")
        .and_then(Value::as_str)
        .ok_or(Error::Invalid)?;
    crate::args::parse_identifier(name).ok_or(Error::Invalid)
}
fn bounded(value: &Value, min: i32, max: i32) -> R {
    require((min..=max).contains(&number::checked_int(value)?))
}
fn width(value: &Value) -> R {
    bounded(value, 1, 1024)
}
fn positive(value: &Value) -> R {
    bounded(value, 1, i32::MAX)
}

pub fn validate(value: &impl Input, context: &dyn Context) -> R {
    let node = value.node();
    validate_node(&node, context)
}
fn validate_node(value: &Value, context: &dyn Context) -> R {
    let map = map(value)?;
    let kind = kind(map)?;
    let mut checks = Vec::new();
    let text = |v: &Value| component(v, context);
    checks.push(field(map, "title", text));
    checks.push(optional(map, "external_title", text));
    checks.push(optional(map, "can_close_with_escape", boolean));
    checks.push(optional(map, "pause", boolean));
    checks.push(optional(map, "after_action", |v| {
        require(matches!(
            v.as_str(),
            Some("close" | "none" | "wait_for_response")
        ))
    }));
    checks.push(require(
        !(present(map, "pause")
            .and_then(Value::as_bool)
            .unwrap_or(true)
            && present(map, "after_action").and_then(Value::as_str) == Some("none")),
    ));
    checks.push(optional(map, "body", |v| {
        if v.is_array() {
            list(v, false, |v| body(v, context))
        } else {
            body(v, context)
        }
    }));
    checks.push(optional(map, "inputs", |v| {
        list(v, false, |v| input::validate(v, context))
    }));
    let button = |v: &Value| button(v, context);
    let specific = (|| match kind.as_str() {
        "minecraft:notice" => optional(map, "action", button),
        "minecraft:confirmation" => all([field(map, "yes", button), field(map, "no", button)]),
        "minecraft:multi_action" => all([
            field(map, "actions", |v| list(v, true, button)),
            optional(map, "exit_action", button),
            optional(map, "columns", positive),
        ]),
        name @ ("minecraft:server_links" | "minecraft:dialog_list") => {
            if name == "minecraft:dialog_list" {
                field(map, "dialogs", |v| {
                    // RegistryCodecs' direct holder-set alternative accepts a
                    // singleton inline holder or a list, not unresolved IDs.
                    if v.is_array() {
                        list(v, false, |v| validate(v, context))
                    } else {
                        validate(v, context)
                    }
                })?;
            }
            all([
                optional(map, "exit_action", button),
                optional(map, "columns", positive),
                optional(map, "button_width", width),
            ])
        }
        _ => Err(Error::Invalid),
    })();
    checks.push(specific);
    all(checks)
}

fn button(value: &Value, context: &dyn Context) -> R {
    let map = map(value)?;
    all([
        field(map, "label", |v| component(v, context)),
        optional(map, "tooltip", |v| component(v, context)),
        optional(map, "width", width),
        optional(map, "action", |v| action(v, context)),
    ])
}
fn action(value: &Value, context: &dyn Context) -> R {
    let map = map(value)?;
    let kind = kind(map)?;
    match kind.as_str() {
        "minecraft:dynamic/run_command" => field(map, "template", variable::template),
        "minecraft:dynamic/custom" => all([
            field(map, "id", id),
            optional(map, "additions", |v| {
                map_value(v)?;
                nbt_payload(v)
            }),
        ]),
        _ => {
            // StaticAction wraps the same allowed-from-server ClickEvent
            // value codec, but dispatches through `type` instead of `action`.
            let Some(action) = kind.strip_prefix("minecraft:") else {
                return Err(Error::Invalid);
            };
            let mut event = map.clone();
            event.insert("action".into(), Value::String(action.into()));
            super::click(&Value::Object(event), context)
        }
    }
}
fn map_value(value: &Value) -> R {
    require(value.is_object())
}

fn plain(value: &Value, context: &dyn Context) -> R {
    let record = || -> R {
        let map = map(value)?;
        all([
            field(map, "contents", |v| component(v, context)),
            optional(map, "width", width),
        ])
    };
    match record() {
        Ok(()) => Ok(()),
        Err(Error::DecodeException) => Err(Error::DecodeException),
        Err(first) => match component(value, context) {
            Ok(()) => Ok(()),
            Err(second) => Err(
                if first == Error::DecodeException || second == Error::DecodeException {
                    Error::DecodeException
                } else if first == Error::Unavailable || second == Error::Unavailable {
                    Error::Unavailable
                } else if first == Error::IncompleteCodec || second == Error::IncompleteCodec {
                    Error::IncompleteCodec
                } else {
                    Error::Invalid
                },
            ),
        },
    }
}
fn body(value: &Value, context: &dyn Context) -> R {
    let map = map(value)?;
    match kind(map)?.as_str() {
        "minecraft:plain_message" => all([
            field(map, "contents", |v| component(v, context)),
            optional(map, "width", width),
        ]),
        "minecraft:item" => {
            all([
                field(map, "item", |v| {
                    if let Some(name) = v.as_str() {
                        // ItemStackTemplate.CODEC's bare Item alternative constructs
                        // the same template with count=1 and an empty patch.
                        let mut item = Map::new();
                        item.insert("id".into(), Value::String(name.into()));
                        context.item_template(&item)
                    } else {
                        context.item_template(self::map(v)?)
                    }
                }),
                optional(map, "description", |v| plain(v, context)),
                optional(map, "show_decorations", boolean),
                optional(map, "show_tooltip", boolean),
                optional(map, "width", |v| bounded(v, 1, 256)),
                optional(map, "height", |v| bounded(v, 1, 256)),
            ])
        }
        _ => Err(Error::Invalid),
    }
}
