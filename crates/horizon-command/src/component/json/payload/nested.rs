use super::*;
use crate::component::json::all;
use crate::component::json::{
    boolean, field, id, list, number, optional, require, string, validate as text,
};
use horizon_world::nbt::Tag;

pub(super) fn validate(name: &str, v: &Value, ctx: &MetadataContext<'_>) -> Option<R> {
    Some(match name {
        "minecraft:custom_data" | "minecraft:bucket_entity_data" => compound(v),
        "minecraft:entity_data" => entity(v, "minecraft:entity_type", ctx),
        "minecraft:block_entity_data" => entity(v, "minecraft:block_entity_type", ctx),
        "minecraft:use_remainder" | "minecraft:sulfur_cube_content" => template(v, ctx),
        "minecraft:charged_projectiles" => list_limit(v, 1024, |v| template(v, ctx)),
        "minecraft:bundle_contents" => list(v, false, |v| template(v, ctx)),
        "minecraft:container" => list_limit(v, 256, |v| {
            let m = map(v)?;
            all([
                field(m, "slot", |v| int(v, 0, 255)),
                field(m, "item", |v| template(v, ctx)),
            ])
        }),
        "minecraft:pot_decorations" => (|| {
            let m = map(v)?;
            all(["back", "left", "right", "front"]
                .into_iter()
                .map(|key| optional(m, key, |v| template(v, ctx))))
        })(),
        "minecraft:bees" => list(v, false, |v| {
            let m = map(v)?;
            all([
                field(m, "entity_data", |v| {
                    entity(v, "minecraft:entity_type", ctx)
                }),
                field(m, "ticks_in_hive", |v| number::checked_int(v).map(|_| ())),
                field(m, "min_ticks_in_hive", |v| {
                    number::checked_int(v).map(|_| ())
                }),
            ])
        }),
        "minecraft:damage_resistant" => map(v).and_then(|m| field(m, "types", fixed_holder_set)),
        "minecraft:repairable" => map(v).and_then(|m| field(m, "items", fixed_holder_set)),
        "minecraft:provides_banner_patterns" => fixed_holder_set(v),
        "minecraft:mob_visibility" => (|| {
            let m = map(v)?;
            all([
                field(m, "targeting_entity_types", fixed_holder_set),
                field(m, "visibility", |v| range(v, 0.0, 10.0)),
            ])
        })(),
        "minecraft:tooltip_display" => (|| {
            let m = map(v)?;
            all([
                optional(m, "hide_tooltip", boolean),
                optional(m, "hidden_components", |v| {
                    list(v, false, |v| {
                        catalog(v, "minecraft:data_component_type", ctx)
                    })
                }),
            ])
        })(),
        "minecraft:provides_trim_material" => material(v, ctx),
        "minecraft:trim" => (|| {
            let m = map(v)?;
            all([
                field(m, "material", |v| material(v, ctx)),
                field(m, "pattern", |v| pattern(v, ctx)),
            ])
        })(),
        "minecraft:banner_patterns" => list(v, false, |v| {
            let m = map(v)?;
            all([field(m, "pattern", banner_pattern), field(m, "color", dye)])
        }),
        "minecraft:break_sound" => sound(v),
        "minecraft:instrument" => instrument(v, ctx),
        "minecraft:lodestone_tracker" => (|| {
            let m = map(v)?;
            all([
                optional(m, "tracked", boolean),
                optional(m, "target", |v| {
                    let m = map(v)?;
                    all([
                        field(m, "dimension", id),
                        field(m, "pos", |v| {
                            let a = v.as_array().ok_or(Error::Invalid)?;
                            all([
                                require(a.len() == 3),
                                all(a.iter().map(|v| number::checked_int(v).map(|_| ()))),
                            ])
                        }),
                    ])
                }),
            ])
        })(),
        _ => return None,
    })
}
pub(super) fn compound(v: &Value) -> R {
    if let Some(s) = v.as_str() {
        // CustomData explicitly permits TagParser.FLATTENED_CODEC. This is
        // not the permissive NbtOps Component/command value decoder.
        return require(matches!(crate::snbt::parse_fully(s), Ok(Tag::Compound(_))));
    }
    map(v)?;
    super::super::nbt_payload(v)
}
pub(super) fn entity(v: &Value, registry: &str, ctx: &MetadataContext<'_>) -> R {
    compound(v)?;
    if let Some(s) = v.as_str() {
        let Tag::Compound(m) = crate::snbt::parse_fully(s).map_err(|_| Error::Invalid)? else {
            return Err(Error::Invalid);
        };
        let Some(Tag::String(id)) = m.get(&"id".into()) else {
            return Err(Error::Invalid);
        };
        catalog(&Value::String(id.to_string_lossy()), registry, ctx)
    } else {
        field(map(v)?, "id", |v| catalog(v, registry, ctx))
    }
}
pub(super) fn sound(v: &Value) -> R {
    let m = map(v)?;
    field(m, "sound_id", id)
    // `range` is a lenient optional Codec.FLOAT: malformed values default.
}
fn material(v: &Value, ctx: &MetadataContext<'_>) -> R {
    let m = map(v)?;
    all([
        field(m, "palette_id", id),
        field(m, "description", |v| text(v, ctx)),
    ])
}
fn pattern(v: &Value, ctx: &MetadataContext<'_>) -> R {
    let m = map(v)?;
    all([
        field(m, "asset_id", id),
        field(m, "description", |v| text(v, ctx)),
        optional(m, "decal", boolean),
    ])
}
fn banner_pattern(v: &Value) -> R {
    let m = map(v)?;
    all([
        field(m, "asset_id", id),
        field(m, "translation_key", string),
    ])
}
pub(super) fn dye(v: &Value) -> R {
    enums::validate("minecraft:dye", v).unwrap()
}
fn instrument(v: &Value, ctx: &MetadataContext<'_>) -> R {
    let m = map(v)?;
    all([
        field(m, "sound_event", sound),
        field(m, "use_duration", |v| range(v, 0.0, f32::MAX)),
        field(m, "range", positive),
        optional(m, "durability_damage", |v| int(v, 0, i32::MAX)),
        field(m, "description", |v| text(v, ctx)),
    ])
}
