//! Source-specific animation, fuel, firework and jukebox codecs.
use super::*;
use crate::component::json::all;
use crate::component::json::{boolean, field, id, list, optional, validate as text};

pub(super) fn validate(name: &str, v: &Value, ctx: &MetadataContext<'_>) -> Option<R> {
    Some(match name {
        "minecraft:attack_animation" | "minecraft:interact_animation" => swing(v),
        "minecraft:compostable" => map(v).and_then(|m| field(m, "layers", resolvable_int)),
        "minecraft:cooking_fuel" | "minecraft:brewing_fuel" => (|| {
            let m = map(v)?;
            all([
                field(
                    m,
                    if name == "minecraft:cooking_fuel" {
                        "burn_time"
                    } else {
                        "uses"
                    },
                    resolvable_int,
                ),
                field(m, "speed_multiplier", resolvable_float),
            ])
        })(),
        "minecraft:firework_explosion" => explosion(v),
        "minecraft:fireworks" => (|| {
            let m = map(v)?;
            all([
                optional(m, "flight_duration", |v| int(v, 0, 255)),
                optional(m, "explosions", |v| list_limit(v, 256, explosion)),
            ])
        })(),
        "minecraft:jukebox_playable" => (|| {
            let m = map(v)?;
            all([
                field(m, "sound_event", nested::sound),
                field(m, "description", |v| text(v, ctx)),
                field(m, "length_in_seconds", positive),
                field(m, "comparator_output", |v| int(v, 0, 15)),
            ])
        })(),
        "minecraft:map_decorations" => map(v).and_then(|m| {
            m.values().try_for_each(|v| {
                let m = map(v)?;
                all([
                    field(m, "type", |v| {
                        catalog(v, "minecraft:map_decoration_type", ctx)
                    }),
                    field(m, "x", double),
                    field(m, "z", double),
                    field(m, "rotation", |v| float(v).map(|_| ())),
                ])
            })
        }),
        "minecraft:attribute_modifiers" => list(v, false, |v| {
            let m = map(v)?;
            all([
                field(m, "type", |v| catalog(v, "minecraft:attribute", ctx)),
                field(m, "id", id),
                field(m, "amount", double),
                field(m, "operation", |v| {
                    one_of(
                        v,
                        &["add_value", "add_multiplied_base", "add_multiplied_total"],
                    )
                }),
                optional(m, "slot", |v| {
                    one_of(
                        v,
                        &[
                            "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head",
                            "armor", "body", "saddle",
                        ],
                    )
                }),
                optional(m, "display", |v| {
                    let m = map(v)?;
                    match m.get("type").and_then(Value::as_str) {
                        Some("default" | "hidden") => Ok(()),
                        Some("override") => field(m, "value", |v| text(v, ctx)),
                        _ => Err(Error::Invalid),
                    }
                }),
            ])
        }),
        "minecraft:debug_stick_state" => map(v).and_then(|m| {
            m.iter().try_for_each(|(block, v)| {
                let block = crate::args::parse_identifier(block).ok_or(Error::Invalid)?;
                // Property definitions come from the existing loaded registry,
                // never from a default item example or a second schema.
                if ctx.registries.blocks.is_empty() {
                    return Err(Error::Unavailable);
                }
                let properties = ctx.registries.blocks.get(&block).ok_or(Error::Invalid)?;
                super::super::require(
                    v.as_str()
                        .is_some_and(|name| properties.iter().any(|(p, _)| p == name)),
                )
            })
        }),
        _ => return None,
    })
}
fn double(v: &Value) -> R {
    super::super::require(v.as_number().is_some_and(|s| s.parse::<f64>().is_ok()))
}
fn resolvable_int(v: &Value) -> R {
    // ResolvableInt.CODEC is Constant | ResourceKey, not a dispatched
    // runtime number-provider map and not a registry holder lookup.
    if v.is_string() {
        id(v)
    } else {
        int(v, i32::MIN, i32::MAX)
    }
}
fn resolvable_float(v: &Value) -> R {
    if v.is_string() {
        id(v)
    } else {
        float(v).map(|_| ())
    }
}
fn swing(v: &Value) -> R {
    let m = map(v)?;
    all([
        optional(m, "type", |v| one_of(v, &["none", "whack", "stab"])),
        optional(m, "duration", |v| int(v, 0, i32::MAX)),
    ])
}
fn explosion(v: &Value) -> R {
    let m = map(v)?;
    let mut checks = vec![
        field(m, "shape", |v| {
            one_of(v, &["small_ball", "large_ball", "star", "creeper", "burst"])
        }),
        optional(m, "has_trail", boolean),
        optional(m, "has_twinkle", boolean),
    ];
    for key in ["colors", "fade_colors"] {
        checks.push(optional(m, key, |v| {
            list(v, false, |v| int(v, i32::MIN, i32::MAX))
        }));
    }
    all(checks)
}
