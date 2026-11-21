use super::*;

pub(super) fn encode(name: &str, v: &Value, ctx: &MetadataContext<'_>) -> Result<Value, Error> {
    match name {
        "minecraft:custom_data" => super::super::super::data::flattened_compound(v),
        "minecraft:villager/variant" => Ok(v.clone()),
        "minecraft:enchantments" | "minecraft:stored_enchantments" => array(v, |v| {
            let mut r = Record::new(v);
            r.optional("enchantments", clone_value, None)?;
            r.optional("levels", ints, Some(empty()))?;
            Ok(r.finish())
        }),
        "minecraft:firework_explosion" => firework(v),
        "minecraft:damage" => {
            let mut r = Record::new(v);
            r.optional("durability", ints, Some(empty()))?;
            r.optional("damage", ints, Some(empty()))?;
            Ok(r.finish())
        }
        "minecraft:container" | "minecraft:bundle_contents" => {
            let mut r = Record::new(v);
            r.optional("items", |v| collection(v, |v| item(v, ctx)), None)?;
            Ok(r.finish())
        }
        "minecraft:potion_contents" => {
            let mut r = Record::new(v);
            r.optional("potions", clone_value, None)?;
            r.optional(
                "effects",
                |v| {
                    let mut out = Map::new();
                    for (k, v) in v.as_object().unwrap() {
                        let mut r = Record::new(v);
                        r.optional("amplifier", ints, Some(empty()))?;
                        r.optional("duration", ints, Some(empty()))?;
                        r.optional("ambient", clone_value, None)?;
                        r.optional("visible", clone_value, None)?;
                        out.insert(crate::args::parse_identifier(k).unwrap(), r.finish());
                    }
                    Ok(Value::Object(out))
                },
                None,
            )?;
            Ok(r.finish())
        }
        "minecraft:fireworks" => {
            let mut r = Record::new(v);
            r.optional("explosions", |v| collection(v, firework), None)?;
            r.optional("flight_duration", ints, Some(empty()))?;
            Ok(r.finish())
        }
        "minecraft:writable_book_content" => {
            let mut r = Record::new(v);
            r.optional("pages", |v| collection(v, clone_value), None)?;
            Ok(r.finish())
        }
        "minecraft:written_book_content" => {
            let mut r = Record::new(v);
            r.optional("pages", |v| collection(v, |v| component(v, ctx)), None)?;
            r.optional("author", clone_value, None)?;
            r.optional("title", clone_value, None)?;
            r.optional("generation", ints, Some(empty()))?;
            r.optional("resolved", clone_value, None)?;
            Ok(r.finish())
        }
        "minecraft:attribute_modifiers" => {
            let mut r = Record::new(v);
            r.optional("modifiers", |v| collection(v, attribute), None)?;
            Ok(r.finish())
        }
        "minecraft:trim" => {
            let mut r = Record::new(v);
            r.optional("material", clone_value, None)?;
            r.optional("pattern", clone_value, None)?;
            Ok(r.finish())
        }
        "minecraft:jukebox_playable" => {
            let mut r = Record::new(v);
            r.optional("song", clone_value, None)?;
            Ok(r.finish())
        }
        _ => Err(Error::IncompleteCodec),
    }
}
fn firework(v: &Value) -> Result<Value, Error> {
    let mut r = Record::new(v);
    for key in ["shape", "has_twinkle", "has_trail"] {
        r.optional(key, clone_value, None)?;
    }
    Ok(r.finish())
}
fn attribute(v: &Value) -> Result<Value, Error> {
    let mut r = Record::new(v);
    r.optional("attribute", clone_value, None)?;
    r.optional("id", identifier, None)?;
    r.optional("amount", doubles, Some(empty()))?;
    r.optional("operation", clone_value, None)?;
    r.optional("slot", clone_value, None)?;
    Ok(r.finish())
}
