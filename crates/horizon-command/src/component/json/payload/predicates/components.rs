use super::*;
use crate::component::json::all;
use crate::component::json::{boolean, id, validate as text};

pub(super) fn validate(name: &str, v: &Value, ctx: &MetadataContext<'_>) -> R {
    match name {
        "minecraft:custom_data" => nbt(v),
        "minecraft:villager/variant" => fixed_holder_set(v),
        "minecraft:enchantments" | "minecraft:stored_enchantments" => list(v, false, |v| {
            let m = map(v)?;
            all([
                optional(m, "enchantments", fixed_holder_set),
                optional(m, "levels", ints),
            ])
        }),
        "minecraft:firework_explosion" => firework(v),
        "minecraft:damage" => {
            let m = map(v)?;
            all([optional(m, "durability", ints), optional(m, "damage", ints)])
        }
        "minecraft:container" | "minecraft:bundle_contents" => {
            map(v).and_then(|m| optional(m, "items", |v| collection(v, |v| item(v, ctx))))
        }
        "minecraft:potion_contents" => {
            let m = map(v)?;
            all([
                optional(m, "potions", fixed_holder_set),
                optional(m, "effects", |v| {
                    all(map(v)?.iter().map(|(id, v)| {
                        let key = catalog(&Value::String(id.clone()), "minecraft:mob_effect", ctx);
                        let value = map(v).and_then(|m| {
                            all([
                                optional(m, "amplifier", ints),
                                optional(m, "duration", ints),
                                optional(m, "ambient", boolean),
                                optional(m, "visible", boolean),
                            ])
                        });
                        all([key, value])
                    }))
                }),
            ])
        }
        "minecraft:fireworks" => {
            let m = map(v)?;
            all([
                optional(m, "explosions", |v| collection(v, firework)),
                optional(m, "flight_duration", ints),
            ])
        }
        "minecraft:writable_book_content" => {
            map(v).and_then(|m| optional(m, "pages", |v| collection(v, string)))
        }
        "minecraft:written_book_content" => {
            let m = map(v)?;
            all([
                optional(m, "pages", |v| collection(v, |v| text(v, ctx))),
                optional(m, "author", string),
                optional(m, "title", string),
                optional(m, "generation", ints),
                optional(m, "resolved", boolean),
            ])
        }
        "minecraft:attribute_modifiers" => {
            map(v).and_then(|m| optional(m, "modifiers", |v| collection(v, attribute)))
        }
        "minecraft:trim" => {
            let m = map(v)?;
            all([
                optional(m, "material", fixed_holder_set),
                optional(m, "pattern", fixed_holder_set),
            ])
        }
        "minecraft:jukebox_playable" => map(v).and_then(|m| optional(m, "song", fixed_holder_set)),
        _ => Err(Error::Invalid),
    }
}
fn firework(v: &Value) -> R {
    let m = map(v)?;
    all([
        optional(m, "shape", |v| {
            one_of(v, &["small_ball", "large_ball", "star", "creeper", "burst"])
        }),
        optional(m, "has_twinkle", boolean),
        optional(m, "has_trail", boolean),
    ])
}
fn attribute(v: &Value) -> R {
    let m = map(v)?;
    all([
        optional(m, "attribute", fixed_holder_set),
        optional(m, "id", id),
        optional(m, "amount", doubles),
        optional(m, "operation", |v| {
            one_of(
                v,
                &["add_value", "add_multiplied_base", "add_multiplied_total"],
            )
        }),
        optional(m, "slot", |v| {
            one_of(
                v,
                &[
                    "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor",
                    "body", "saddle",
                ],
            )
        }),
    ])
}
