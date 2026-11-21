use super::*;
use crate::component::json::{all, boolean, field, id, list, number, optional, string};

pub(super) fn validate(name: &str, value: &Value, _: &MetadataContext<'_>) -> Option<R> {
    if !matches!(
        name,
        "minecraft:use_effects"
            | "minecraft:weapon"
            | "minecraft:attack_range"
            | "minecraft:use_cooldown"
            | "minecraft:enchantable"
            | "minecraft:villager_food"
            | "minecraft:food"
            | "minecraft:custom_model_data"
            | "minecraft:container_loot"
    ) {
        return None;
    }
    Some((|| -> R {
        let m = map(value)?;
        match name {
            "minecraft:use_effects" => all([
                optional(m, "can_sprint", boolean),
                optional(m, "interact_vibrations", boolean),
                optional(m, "speed_multiplier", |v| range(v, 0.0, 1.0)),
            ]),
            "minecraft:weapon" => all([
                optional(m, "item_damage_per_attack", |v| int(v, 0, i32::MAX)),
                optional(m, "disable_blocking_for_seconds", |v| {
                    range(v, 0.0, f32::MAX)
                }),
            ]),
            "minecraft:attack_range" => {
                for key in [
                    "min_reach",
                    "max_reach",
                    "min_creative_reach",
                    "max_creative_reach",
                    "hitbox_margin",
                ] {
                    optional(m, key, |v| range(v, 0.0, 64.0))?;
                }
                optional(m, "mob_factor", |v| range(v, 0.0, 2.0))
            }
            "minecraft:use_cooldown" => all([
                field(m, "seconds", positive),
                optional(m, "cooldown_group", id),
            ]),
            "minecraft:enchantable" | "minecraft:villager_food" => field(
                m,
                if name == "minecraft:enchantable" {
                    "value"
                } else {
                    "nutrition"
                },
                |v| int(v, 1, i32::MAX),
            ),
            "minecraft:food" => all([
                field(m, "nutrition", |v| int(v, 0, i32::MAX)),
                field(m, "saturation", |v| float(v).map(|_| ())),
                optional(m, "can_always_eat", boolean),
            ]),
            "minecraft:custom_model_data" => all([
                optional(m, "floats", |v| list(v, false, |v| float(v).map(|_| ()))),
                optional(m, "flags", |v| list(v, false, boolean)),
                optional(m, "strings", |v| list(v, false, string)),
                optional(m, "colors", |v| list(v, false, rgb)),
            ]),
            "minecraft:container_loot" => all([
                field(m, "loot_table", id),
                optional(m, "seed", |v| number::checked_long(v).map(|_| ())),
            ]),
            _ => Err(Error::IncompleteCodec),
        }
    })())
}
fn rgb(value: &Value) -> R {
    if value.is_number() {
        return number::checked_int(value).map(|_| ());
    }
    let a = value.as_array().ok_or(Error::Invalid)?;
    super::super::require(a.len() == 3)?;
    a.iter().try_for_each(|v| float(v).map(|_| ()))
}
