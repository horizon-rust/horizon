use super::*;
use crate::component::json::all;
use crate::component::json::{boolean, field, id, list, optional, string};

pub(super) fn validate(name: &str, v: &Value, ctx: &MetadataContext<'_>) -> Option<R> {
    Some(match name {
        "minecraft:potion_contents" => {
            if v.is_string() {
                catalog(v, "minecraft:potion", ctx)
            } else {
                (|| {
                    let m = map(v)?;
                    all([
                        optional(m, "potion", |v| catalog(v, "minecraft:potion", ctx)),
                        optional(m, "custom_color", |v| int(v, i32::MIN, i32::MAX)),
                        optional(m, "custom_effects", |v| list(v, false, |v| effect(v, ctx))),
                        optional(m, "custom_name", string),
                    ])
                })()
            }
        }
        "minecraft:suspicious_stew_effects" => list(v, false, |v| {
            let m = map(v)?;
            let id = field(m, "id", |v| catalog(v, "minecraft:mob_effect", ctx));
            // duration is lenientOptionalFieldOf(INT). Conversion exceptions
            // still escape the codec; malformed non-numbers use the default.
            let duration = m
                .get("duration")
                .filter(|v| v.is_number())
                .map_or(Ok(()), |v| int(v, i32::MIN, i32::MAX));
            all([id, duration])
        }),
        "minecraft:death_protection" => map(v)
            .and_then(|m| optional(m, "death_effects", |v| list(v, false, |v| consume(v, ctx)))),
        "minecraft:consumable" => (|| {
            let m = map(v)?;
            all([
                optional(m, "consume_seconds", |v| range(v, 0.0, f32::MAX)),
                optional(m, "animation", |v| {
                    one_of(
                        v,
                        &[
                            "none",
                            "eat",
                            "drink",
                            "block",
                            "bow",
                            "trident",
                            "crossbow",
                            "spyglass",
                            "toot_horn",
                            "brush",
                            "bundle",
                            "spear",
                        ],
                    )
                }),
                optional(m, "sound", nested::sound),
                optional(m, "has_consume_particles", boolean),
                optional(m, "on_consume_effects", |v| {
                    list(v, false, |v| consume(v, ctx))
                }),
            ])
        })(),
        "minecraft:equippable" => (|| {
            let m = map(v)?;
            all([
                field(m, "slot", |v| {
                    one_of(
                        v,
                        &[
                            "mainhand", "offhand", "feet", "legs", "chest", "head", "body",
                            "saddle",
                        ],
                    )
                }),
                optional(m, "equip_sound", nested::sound),
                optional(m, "asset_id", id),
                optional(m, "camera_overlay", id),
                optional(m, "allowed_entities", fixed_holder_set),
            ])?;
            for key in [
                "dispensable",
                "swappable",
                "damage_on_hurt",
                "equip_on_interact",
                "can_be_sheared",
            ] {
                optional(m, key, boolean)?;
            }
            optional(m, "shearing_sound", nested::sound)
        })(),
        "minecraft:tool" => (|| {
            let m = map(v)?;
            all([
                field(m, "rules", |v| {
                    list(v, false, |v| {
                        let m = map(v)?;
                        all([
                            field(m, "blocks", fixed_holder_set),
                            optional(m, "speed", positive),
                            optional(m, "correct_for_drops", boolean),
                        ])
                    })
                }),
                optional(m, "default_mining_speed", |v| float(v).map(|_| ())),
                optional(m, "damage_per_block", |v| int(v, 0, i32::MAX)),
                optional(m, "can_destroy_blocks_in_creative", boolean),
            ])
        })(),
        _ => return None,
    })
}
fn effect(v: &Value, ctx: &MetadataContext<'_>) -> R {
    let m = map(v)?;
    all([
        field(m, "id", |v| catalog(v, "minecraft:mob_effect", ctx)),
        details(v),
    ])
}
fn details(v: &Value) -> R {
    let m = map(v)?;
    let mut checks = vec![
        optional(m, "amplifier", |v| int(v, 0, 255)),
        optional(m, "duration", |v| int(v, i32::MIN, i32::MAX)),
        optional(m, "hidden_effect", details),
    ];
    for key in ["ambient", "show_particles", "show_icon"] {
        checks.push(optional(m, key, boolean));
    }
    all(checks)
}

fn consume(v: &Value, ctx: &MetadataContext<'_>) -> R {
    let m = map(v)?;
    let kind = crate::args::parse_identifier(
        m.get("type")
            .and_then(Value::as_str)
            .ok_or(Error::Invalid)?,
    )
    .ok_or(Error::Invalid)?;
    match kind.as_str() {
        "minecraft:apply_effects" => all([
            field(m, "effects", |v| list(v, false, |v| effect(v, ctx))),
            optional(m, "probability", |v| range(v, 0.0, 1.0)),
        ]),
        "minecraft:remove_effects" => field(m, "effects", fixed_holder_set),
        "minecraft:clear_all_effects" => Ok(()),
        "minecraft:teleport_randomly" => all([
            optional(m, "diameter", positive),
            optional(m, "directional_particles", boolean),
        ]),
        "minecraft:play_sound" => field(m, "sound", nested::sound),
        _ => Err(Error::Invalid),
    }
}
