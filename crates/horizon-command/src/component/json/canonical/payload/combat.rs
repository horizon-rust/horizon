use super::*;
use records::{Record, clone_value, n, sound};

pub(super) fn encode(name: &str, v: &Value) -> Option<Result<Value, Error>> {
    if !matches!(
        name,
        "minecraft:blocks_attacks" | "minecraft:piercing_weapon" | "minecraft:kinetic_weapon"
    ) {
        return None;
    }
    Some((|| {
        let mut r = Record::new(v);
        match name {
            "minecraft:blocks_attacks" => {
                r.optional("block_delay_seconds", float, Some(n("0.0")))?;
                r.optional("disable_cooldown_scale", float, Some(n("1.0")))?;
                let reduction_default = Value::Array(vec![Value::Object(Map::from([
                    ("base".into(), n("0.0")),
                    ("factor".into(), n("1.0")),
                ]))]);
                r.optional(
                    "damage_reductions",
                    |v| array(v, reduction),
                    Some(reduction_default),
                )?;
                let damage_default = Value::Object(Map::from([
                    ("threshold".into(), n("1.0")),
                    ("base".into(), n("0.0")),
                    ("factor".into(), n("1.0")),
                ]));
                r.optional("item_damage", item_damage, Some(damage_default))?;
                r.optional("bypassed_by", clone_value, None)?;
                r.optional("block_sound", sound, None)?;
                r.optional("disabled_sound", sound, None)?;
            }
            "minecraft:piercing_weapon" => {
                r.optional("deals_knockback", clone_value, Some(Value::Bool(true)))?;
                r.optional("dismounts", clone_value, Some(Value::Bool(false)))?;
                r.optional("sound", sound, None)?;
                r.optional("hit_sound", sound, None)?;
            }
            "minecraft:kinetic_weapon" => {
                r.optional("contact_cooldown_ticks", integer, Some(n("10")))?;
                r.optional("delay_ticks", integer, Some(n("0")))?;
                for key in [
                    "dismount_conditions",
                    "knockback_conditions",
                    "damage_conditions",
                ] {
                    r.optional(key, condition, None)?;
                }
                r.optional("forward_movement", float, Some(n("0.0")))?;
                r.optional("damage_multiplier", float, Some(n("1.0")))?;
                r.optional("sound", sound, None)?;
                r.optional("hit_sound", sound, None)?;
            }
            _ => return Err(Error::IncompleteCodec),
        }
        Ok(r.finish())
    })())
}
fn reduction(v: &Value) -> Result<Value, Error> {
    let mut r = Record::new(v);
    r.optional("horizontal_blocking_angle", float, Some(n("90.0")))?;
    r.optional("type", clone_value, None)?;
    r.required("base", float)?;
    r.required("factor", float)?;
    Ok(r.finish())
}
fn item_damage(v: &Value) -> Result<Value, Error> {
    let mut r = Record::new(v);
    for key in ["threshold", "base", "factor"] {
        r.required(key, float)?;
    }
    Ok(r.finish())
}
fn condition(v: &Value) -> Result<Value, Error> {
    let mut r = Record::new(v);
    r.required("max_duration_ticks", integer)?;
    r.optional("min_speed", float, Some(n("0.0")))?;
    r.optional("min_relative_speed", float, Some(n("0.0")))?;
    Ok(r.finish())
}
