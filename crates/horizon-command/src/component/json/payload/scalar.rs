use super::*;
use crate::component::json::{
    boolean, id, list, number, profile, require, string, validate as text,
};

pub(super) fn validate(name: &str, v: &Value, ctx: &MetadataContext<'_>) -> Option<R> {
    Some(match name {
        "minecraft:damage" | "minecraft:repair_cost" => int(v, 0, i32::MAX),
        "minecraft:max_damage" => int(v, 1, i32::MAX),
        "minecraft:max_stack_size" => int(v, 1, 99),
        "minecraft:map_id" | "minecraft:dyed_color" => number::checked_int(v).map(|_| ()),
        "minecraft:ominous_bottle_amplifier" => int(v, 0, 4),
        "minecraft:minimum_attack_charge" => range(v, 0.0, 1.0),
        "minecraft:potion_duration_scale" => range(v, 0.0, f32::MAX),
        "minecraft:enchantment_glint_override" => boolean(v),
        "minecraft:unbreakable"
        | "minecraft:intangible_projectile"
        | "minecraft:glider"
        | "minecraft:waxed" => require(v.is_object()),
        "minecraft:custom_name" | "minecraft:item_name" => text(v, ctx),
        "minecraft:lore" => {
            if v.as_array().is_some_and(|a| a.len() <= 256) {
                list(v, false, |v| text(v, ctx))
            } else {
                Err(Error::Invalid)
            }
        }
        "minecraft:profile" => profile::validate(v),
        "minecraft:item_model" | "minecraft:tooltip_style" | "minecraft:note_block_sound" => id(v),
        "minecraft:rarity" => one_of(v, &["common", "uncommon", "rare", "epic"]),
        "minecraft:recipes" => list(v, false, id),
        "minecraft:block_state" => map(v).and_then(|m| m.values().try_for_each(string)),
        "minecraft:enchantments" | "minecraft:stored_enchantments" => {
            // Dynamic holder keys cannot resolve in JsonOps. Empty maps need
            // no registry lookup and retain their successful codec result.
            map(v).and_then(|m| require(m.is_empty()))
        }
        // These exact family CODECs are fixed dynamic holders, with no
        // inline alternative (independently checked against each class).
        "minecraft:block_transformer"
        | "minecraft:provides_pottery_pattern"
        | "minecraft:damage_type"
        | "minecraft:villager/variant"
        | "minecraft:wolf/variant"
        | "minecraft:wolf/sound_variant"
        | "minecraft:pig/variant"
        | "minecraft:pig/sound_variant"
        | "minecraft:cow/variant"
        | "minecraft:cow/sound_variant"
        | "minecraft:chicken/variant"
        | "minecraft:chicken/sound_variant"
        | "minecraft:zombie_nautilus/variant"
        | "minecraft:frog/variant"
        | "minecraft:cat/variant"
        | "minecraft:cat/sound_variant"
        | "minecraft:painting/variant" => Err(Error::Invalid),
        _ => return None,
    })
}
