use super::*;

/// Field defaults below follow each family's RecordCodecBuilder, not an item
/// sample. Values are already decoded by the paired persistent codec.
pub(super) fn encode(
    name: &str,
    v: &Value,
    ctx: &MetadataContext<'_>,
) -> Option<Result<Value, Error>> {
    if !matches!(
        name,
        "minecraft:use_effects"
            | "minecraft:weapon"
            | "minecraft:attack_range"
            | "minecraft:use_cooldown"
            | "minecraft:enchantable"
            | "minecraft:villager_food"
            | "minecraft:food"
            | "minecraft:attack_animation"
            | "minecraft:interact_animation"
            | "minecraft:cooking_fuel"
            | "minecraft:brewing_fuel"
            | "minecraft:compostable"
            | "minecraft:break_sound"
            | "minecraft:instrument"
            | "minecraft:jukebox_playable"
            | "minecraft:provides_trim_material"
            | "minecraft:trim"
            | "minecraft:banner_patterns"
            | "minecraft:damage_resistant"
            | "minecraft:repairable"
            | "minecraft:mob_visibility"
            | "minecraft:provides_banner_patterns"
            | "minecraft:pot_decorations"
            | "minecraft:lodestone_tracker"
            | "minecraft:firework_explosion"
            | "minecraft:fireworks"
            | "minecraft:map_decorations"
    ) {
        return None;
    }
    Some((|| {
        if name == "minecraft:provides_banner_patterns" {
            return Ok(Value::Array(Vec::new()));
        }
        if name == "minecraft:banner_patterns" {
            return array(v, |v| {
                let mut r = Record::new(v);
                r.required("pattern", |v| {
                    let mut p = Record::new(v);
                    p.required("asset_id", identifier)?;
                    p.required("translation_key", clone_value)?;
                    Ok(p.finish())
                })?;
                r.required("color", clone_value)?;
                Ok(r.finish())
            });
        }
        if name == "minecraft:map_decorations" {
            let mut out = Map::new();
            for (k, v) in v.as_object().unwrap() {
                let mut r = Record::new(v);
                r.required("type", identifier)?;
                r.required("x", double)?;
                r.required("z", double)?;
                r.required("rotation", float)?;
                out.insert(k.clone(), r.finish());
            }
            return Ok(Value::Object(out));
        }
        let mut r = Record::new(v);
        match name {
            "minecraft:use_effects" => {
                r.optional("can_sprint", clone_value, Some(Value::Bool(false)))?;
                r.optional("interact_vibrations", clone_value, Some(Value::Bool(true)))?;
                r.optional("speed_multiplier", float, Some(n("0.2")))?;
            }
            "minecraft:weapon" => {
                r.optional("item_damage_per_attack", integer, Some(n("1")))?;
                r.optional("disable_blocking_for_seconds", float, Some(n("0.0")))?;
            }
            "minecraft:attack_range" => {
                for (key, default) in [
                    ("min_reach", "0.0"),
                    ("max_reach", "3.0"),
                    ("min_creative_reach", "0.0"),
                    ("max_creative_reach", "5.0"),
                    ("hitbox_margin", "0.3"),
                    ("mob_factor", "1.0"),
                ] {
                    r.optional(key, float, Some(n(default)))?;
                }
            }
            "minecraft:use_cooldown" => {
                r.required("seconds", float)?;
                r.optional("cooldown_group", identifier, None)?;
            }
            "minecraft:enchantable" => r.required("value", integer)?,
            "minecraft:villager_food" => r.required("nutrition", integer)?,
            "minecraft:food" => {
                r.required("nutrition", integer)?;
                r.required("saturation", float)?;
                r.optional("can_always_eat", clone_value, Some(Value::Bool(false)))?;
            }
            "minecraft:attack_animation" | "minecraft:interact_animation" => {
                r.optional("type", clone_value, Some(Value::String("whack".into())))?;
                r.optional("duration", integer, Some(n("6")))?;
            }
            "minecraft:compostable" => r.required("layers", resolvable_int)?,
            "minecraft:cooking_fuel" | "minecraft:brewing_fuel" => {
                r.required(
                    if name == "minecraft:cooking_fuel" {
                        "burn_time"
                    } else {
                        "uses"
                    },
                    resolvable_int,
                )?;
                r.required("speed_multiplier", resolvable_float)?;
            }
            "minecraft:break_sound" => return sound(v),
            "minecraft:instrument" => {
                r.required("sound_event", sound)?;
                r.required("use_duration", float)?;
                r.required("range", float)?;
                r.optional("durability_damage", integer, Some(n("0")))?;
                r.required("description", |v| component(v, ctx))?;
            }
            "minecraft:jukebox_playable" => {
                r.required("sound_event", sound)?;
                r.required("description", |v| component(v, ctx))?;
                r.required("length_in_seconds", float)?;
                r.required("comparator_output", integer)?;
            }
            "minecraft:provides_trim_material" => return material(v, ctx),
            "minecraft:trim" => {
                r.required("material", |v| material(v, ctx))?;
                r.required("pattern", |v| {
                    let mut p = Record::new(v);
                    p.required("asset_id", identifier)?;
                    p.required("description", |v| component(v, ctx))?;
                    p.optional("decal", clone_value, Some(Value::Bool(false)))?;
                    Ok(p.finish())
                })?;
            }
            "minecraft:damage_resistant" => r.required("types", clone_value)?,
            "minecraft:repairable" => r.required("items", clone_value)?,
            "minecraft:mob_visibility" => {
                r.required("targeting_entity_types", clone_value)?;
                r.required("visibility", float)?;
            }
            "minecraft:pot_decorations" => {
                for key in ["back", "left", "right", "front"] {
                    r.optional(key, |v| template(v, ctx), None)?;
                }
            }
            "minecraft:lodestone_tracker" => {
                r.optional("tracked", clone_value, Some(Value::Bool(true)))?;
                r.optional(
                    "target",
                    |v| {
                        let mut t = Record::new(v);
                        t.required("dimension", identifier)?;
                        t.required("pos", |v| array(v, integer))?;
                        Ok(t.finish())
                    },
                    None,
                )?;
            }
            "minecraft:firework_explosion" => return explosion(v),
            "minecraft:fireworks" => {
                r.optional("flight_duration", integer, Some(n("0")))?;
                r.optional(
                    "explosions",
                    |v| array(v, explosion),
                    Some(Value::Array(Vec::new())),
                )?;
            }
            _ => return Err(Error::IncompleteCodec),
        }
        Ok(r.finish())
    })())
}
pub(super) struct Record<'a> {
    input: &'a Map<String, Value>,
    output: Map<String, Value>,
}
impl<'a> Record<'a> {
    pub(super) fn new(v: &'a Value) -> Self {
        Self {
            input: v.as_object().unwrap(),
            output: Map::new(),
        }
    }
    pub(super) fn required(
        &mut self,
        key: &str,
        encode: impl FnOnce(&Value) -> Result<Value, Error>,
    ) -> R {
        self.output.insert(key.into(), encode(&self.input[key])?);
        Ok(())
    }
    pub(super) fn optional(
        &mut self,
        key: &str,
        encode: impl FnOnce(&Value) -> Result<Value, Error>,
        default: Option<Value>,
    ) -> R {
        if let Some(v) = present(self.input, key) {
            let v = encode(v)?;
            if default.as_ref() != Some(&v) {
                self.output.insert(key.into(), v);
            }
        }
        Ok(())
    }
    pub(super) fn finish(self) -> Value {
        Value::Object(self.output)
    }
}
pub(super) fn clone_value(v: &Value) -> Result<Value, Error> {
    Ok(v.clone())
}
pub(super) fn n(s: &str) -> Value {
    Value::Number(s.into())
}
pub(super) fn double(v: &Value) -> Result<Value, Error> {
    let n = v
        .as_number()
        .unwrap()
        .parse::<f64>()
        .map_err(|_| Error::DecodeException)?;
    if !n.is_finite() {
        return Err(Error::DecodeException);
    }
    Ok(Value::Number(horizon_world::snbt_text::java_double(n)))
}
fn resolvable_int(v: &Value) -> Result<Value, Error> {
    if v.is_string() {
        identifier(v)
    } else {
        integer(v)
    }
}
fn resolvable_float(v: &Value) -> Result<Value, Error> {
    if v.is_string() {
        identifier(v)
    } else {
        float(v)
    }
}
pub(super) fn sound(v: &Value) -> Result<Value, Error> {
    let mut r = Record::new(v);
    r.required("sound_id", identifier)?;
    // Invalid optional range is the decoder's empty Optional, not raw data.
    if let Some(v) = present(r.input, "range").filter(|v| v.is_number()) {
        r.output.insert("range".into(), float(v)?);
    }
    Ok(r.finish())
}
fn material(v: &Value, ctx: &MetadataContext<'_>) -> Result<Value, Error> {
    let mut r = Record::new(v);
    r.required("palette_id", identifier)?;
    r.required("description", |v| component(v, ctx))?;
    Ok(r.finish())
}
fn explosion(v: &Value) -> Result<Value, Error> {
    let mut r = Record::new(v);
    r.required("shape", clone_value)?;
    for key in ["colors", "fade_colors"] {
        r.optional(key, |v| array(v, integer), Some(Value::Array(Vec::new())))?;
    }
    for key in ["has_trail", "has_twinkle"] {
        r.optional(key, clone_value, Some(Value::Bool(false)))?;
    }
    Ok(r.finish())
}
