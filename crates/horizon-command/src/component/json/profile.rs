//! ResolvableProfile's plain JSON codec, including its two property-map forms.
use super::value::{Input, Value};
use super::{Error, R, all, field, id, list, number, optional, require, string};

pub fn validate(value: &impl Input) -> R {
    let node = value.node();
    validate_node(&node)
}
fn validate_node(value: &Value) -> R {
    if value.is_string() {
        return player_name(value);
    }
    let map = value.as_object().ok_or(Error::Invalid)?;
    let mut checks = vec![
        optional(map, "name", player_name),
        // Partial profiles use UUIDUtil.CODEC, not hover's lenient UUID string.
        optional(map, "id", |v| {
            let a = v.as_array().ok_or(Error::Invalid)?;
            all([
                require(a.len() == 4),
                all(a.iter().map(|v| number::checked_int(v).map(|_| ()))),
            ])
        }),
        optional(map, "properties", properties),
    ];
    for key in ["texture", "cape", "elytra"] {
        checks.push(optional(map, key, id));
    }
    checks.push(optional(map, "model", |v| {
        require(matches!(v.as_str(), Some("slim" | "wide")))
    }));
    all(checks)
}

fn player_name(v: &Value) -> R {
    let name = v.as_str().ok_or(Error::Invalid)?;
    // StringUtil.isValidPlayerName: all UTF-16 units are greater than 32
    // and less than 127; empty names are accepted by the codec.
    require(name.len() <= 16 && name.bytes().all(|c| c > 32 && c < 127))
}

fn bounded_string(v: &Value, max: usize) -> R {
    let value = v.as_str().ok_or(Error::Invalid)?;
    require(value.encode_utf16().count() <= max)
}

fn properties(value: &Value) -> R {
    if let Some(map) = value.as_object() {
        // This alternative bounds the number of keys, not total properties,
        // and uses unbounded Codec.STRING for the keys and list entries.
        require(map.len() <= 16)?;
        return map.values().try_for_each(|v| list(v, false, string));
    }
    let entries = value.as_array().ok_or(Error::Invalid)?;
    require(entries.len() <= 16)?;
    entries.iter().try_for_each(|entry| {
        let map = entry.as_object().ok_or(Error::Invalid)?;
        field(map, "name", |v| bounded_string(v, 64))?;
        field(map, "value", |v| bounded_string(v, 32767))?;
        optional(map, "signature", |v| bounded_string(v, 1024))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn admitted_profile_forms_and_independent_uuid_codec() {
        for value in [
            json!({}),
            json!("Player"),
            json!({"name":""}),
            json!({"id":[1,2,3,4]}),
            json!({"texture":"minecraft:entity/player/wide/steve"}),
            json!({"properties":{"textures":["encoded"]}}),
            json!({"properties":[{"name":"textures","value":"encoded"}]}),
        ] {
            assert_eq!(validate(&value), Ok(()), "{value}");
        }
        for value in [
            json!({"name":"é"}),
            json!({"id":"1-1-1-1-1"}),
            json!({"id":"00000000000000000000000000000001"}),
            json!({"properties":{"textures":[{"value":"encoded"}]}}),
        ] {
            assert_eq!(validate(&value), Err(Error::Invalid), "{value}");
        }
    }

    #[test]
    fn property_alternatives_keep_their_distinct_limits() {
        let mut keys = serde_json::Map::new();
        for i in 0..16 {
            keys.insert(i.to_string(), json!(["value"]));
        }
        assert_eq!(validate(&json!({"properties":keys})), Ok(()));
        keys.insert("extra".into(), json!(["value"]));
        assert_eq!(validate(&json!({"properties":keys})), Err(Error::Invalid));
        assert_eq!(
            validate(&json!({"properties":{"textures":vec!["v";17]}})),
            Ok(())
        );
        assert_eq!(
            validate(&json!({"properties":[{"name":"x".repeat(65),"value":"v"}]})),
            Err(Error::Invalid)
        );
        assert_eq!(validate(&json!({"name":"a.b!","model":"slim"})), Ok(()));
        assert_eq!(validate(&json!({"name":"a b"})), Err(Error::Invalid));
    }
}
