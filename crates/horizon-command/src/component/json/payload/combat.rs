//! Weapon value codecs, separate from runtime combat evaluation.
use super::*;
use crate::component::json::all;
use crate::component::json::{boolean, field, list, optional};

pub(super) fn validate(name: &str, v: &Value) -> Option<R> {
    if !matches!(
        name,
        "minecraft:blocks_attacks" | "minecraft:piercing_weapon" | "minecraft:kinetic_weapon"
    ) {
        return None;
    }
    Some((|| {
        let m = map(v)?;
        match name {
            "minecraft:blocks_attacks" => all([
                optional(m, "block_delay_seconds", nonnegative),
                optional(m, "disable_cooldown_scale", nonnegative),
                optional(m, "damage_reductions", |v| list(v, false, reduction)),
                optional(m, "item_damage", item_damage),
                optional(m, "bypassed_by", fixed_holder_set),
                optional(m, "block_sound", nested::sound),
                optional(m, "disabled_sound", nested::sound),
            ]),
            "minecraft:piercing_weapon" => {
                all([
                    optional(m, "deals_knockback", boolean),
                    optional(m, "dismounts", boolean),
                ])?;
                sounds(m)
            }
            "minecraft:kinetic_weapon" => {
                let mut checks = vec![
                    optional(m, "contact_cooldown_ticks", |v| int(v, 0, i32::MAX)),
                    optional(m, "delay_ticks", |v| int(v, 0, i32::MAX)),
                    optional(m, "forward_movement", unbounded),
                    optional(m, "damage_multiplier", unbounded),
                    sounds(m),
                ];
                for key in [
                    "dismount_conditions",
                    "knockback_conditions",
                    "damage_conditions",
                ] {
                    checks.push(optional(m, key, condition));
                }
                all(checks)
            }
            _ => unreachable!(),
        }
    })())
}
fn unbounded(v: &Value) -> R {
    float(v).map(|_| ())
}
fn nonnegative(v: &Value) -> R {
    range(v, 0.0, f32::MAX)
}
fn sounds(m: &super::super::value::Map<String, Value>) -> R {
    all([
        optional(m, "sound", nested::sound),
        optional(m, "hit_sound", nested::sound),
    ])
}
fn reduction(v: &Value) -> R {
    let m = map(v)?;
    all([
        optional(m, "horizontal_blocking_angle", positive),
        optional(m, "type", fixed_holder_set),
        field(m, "base", unbounded),
        field(m, "factor", unbounded),
    ])
}
fn item_damage(v: &Value) -> R {
    let m = map(v)?;
    all([
        field(m, "threshold", nonnegative),
        field(m, "base", unbounded),
        field(m, "factor", unbounded),
    ])
}
fn condition(v: &Value) -> R {
    let m = map(v)?;
    all([
        field(m, "max_duration_ticks", |v| int(v, 0, i32::MAX)),
        optional(m, "min_speed", unbounded),
        optional(m, "min_relative_speed", unbounded),
    ])
}
