use super::*;
use crate::Registries;
use crate::component::json::{context::MetadataContext, value::parse_first};

fn payload(id: &str, raw: &str) -> R {
    let registries = Registries {
        items: vec!["minecraft:stone".into()],
        entity_types: vec!["minecraft:pig".into()],
        persistent_components: vec![id.into()],
        ..Default::default()
    };
    let context = MetadataContext {
        registries: &registries,
    };
    let input = parse_first(
        format!(r#"{{"id":"minecraft:stone","components":{{"{id}":{raw}}}}}"#).as_bytes(),
    )
    .unwrap();
    context.item_template(input.as_object().unwrap())
}

#[test]
fn scalar_families_use_typed_persistent_codecs() {
    for (id, raw, expected) in [
        ("damage", "0", true),
        ("damage", "-1", false),
        ("damage", "2147483648", false),
        ("max_stack_size", "99", true),
        ("max_stack_size", "100", false),
        ("max_stack_size", "99.99999999999999999999999999", true),
        ("enchantment_glint_override", "true", true),
        ("enchantment_glint_override", "1", false),
        ("unbreakable", "{}", true),
        ("unbreakable", "1", false),
        ("rarity", r#""rare""#, true),
        ("rarity", r#""invalid""#, false),
        ("dye", r#""red""#, true),
        ("dye", "3", false),
        ("salmon/size", r#""medium""#, true),
        ("salmon/size", r#""red""#, false),
        ("block_state", r#"{"facing":"north"}"#, true),
        ("block_state", r#"{"facing":true}"#, false),
    ] {
        assert_eq!(
            payload(&format!("minecraft:{id}"), raw),
            if expected {
                Ok(())
            } else {
                Err(Error::Invalid)
            },
            "{id}: {raw}"
        );
    }
}

#[test]
fn nested_text_errors_are_not_admitted_as_partial_values() {
    for (id, raw, expected) in [
        ("custom_name", r#"{"text":"name"}"#, true),
        ("custom_name", r#"{"text":"name","bold":1}"#, false),
        ("lore", r#"["line",{"text":"tail"}]"#, true),
        ("lore", r#"["line",{}]"#, false),
        (
            "custom_model_data",
            r#"{"floats":[1],"flags":[true],"strings":["x"],"colors":[[1,0,0]]}"#,
            true,
        ),
        ("custom_model_data", r#"{"flags":[1]}"#, false),
    ] {
        assert_eq!(
            payload(&format!("minecraft:{id}"), raw),
            if expected {
                Ok(())
            } else {
                Err(Error::Invalid)
            },
            "{id}: {raw}"
        );
    }
}

#[test]
fn record_families_enforce_fields_and_source_specific_ranges() {
    for (id, raw, expected) in [
        ("use_effects", "{}", true),
        ("use_effects", r#"{"speed_multiplier":2}"#, false),
        ("weapon", "{}", true),
        ("weapon", r#"{"item_damage_per_attack":-1}"#, false),
        ("attack_range", "{}", true),
        ("attack_range", r#"{"mob_factor":3}"#, false),
        ("use_cooldown", r#"{"seconds":1}"#, true),
        ("use_cooldown", r#"{"seconds":0}"#, false),
        ("use_cooldown", "{}", false),
        ("food", r#"{"nutrition":0,"saturation":-1}"#, true),
        ("food", r#"{"nutrition":-1,"saturation":1}"#, false),
        ("enchantable", r#"{"value":1}"#, true),
        ("enchantable", r#"{"value":0}"#, false),
    ] {
        assert_eq!(
            payload(&format!("minecraft:{id}"), raw),
            if expected {
                Ok(())
            } else {
                Err(Error::Invalid)
            },
            "{id}: {raw}"
        );
    }
}

#[test]
fn world_registry_presence_does_not_promote_plain_ops_to_registry_ops() {
    assert_eq!(payload("minecraft:enchantments", "{}"), Ok(()));
    assert_eq!(
        payload("minecraft:enchantments", r#"{"minecraft:unbreaking":1}"#),
        Err(Error::Invalid)
    );
    assert_eq!(
        payload("minecraft:wolf/variant", r#""minecraft:pale""#),
        Err(Error::Invalid)
    );
}

#[test]
fn structured_payloads_reject_partial_nested_results() {
    for (id, raw, expected) in [
        ("attack_animation", r#"{"type":"stab","duration":0}"#, true),
        ("interact_animation", r#"{"type":"eat"}"#, false),
        ("attack_animation", r#"{"duration":-1}"#, false),
        ("compostable", r#"{"layers":"custom:layers"}"#, true),
        (
            "compostable",
            r#"{"layers":{"type":"constant","value":1}}"#,
            false,
        ),
        (
            "cooking_fuel",
            r#"{"burn_time":20,"speed_multiplier":1}"#,
            true,
        ),
        ("cooking_fuel", r#"{"burn_time":20}"#, false),
        (
            "brewing_fuel",
            r#"{"uses":-1,"speed_multiplier":"custom:speed"}"#,
            true,
        ),
        (
            "blocks_attacks",
            r#"{"damage_reductions":[{"base":0,"factor":1,"type":[]}]}"#,
            true,
        ),
        (
            "blocks_attacks",
            r#"{"damage_reductions":[{"base":0}]}"#,
            false,
        ),
        (
            "blocks_attacks",
            r#"{"item_damage":{"threshold":-1,"base":0,"factor":1}}"#,
            false,
        ),
        (
            "blocks_attacks",
            r##"{"bypassed_by":"#minecraft:bypasses_shield"}"##,
            false,
        ),
        (
            "kinetic_weapon",
            r#"{"damage_conditions":{"max_duration_ticks":0,"min_speed":-1}}"#,
            true,
        ),
        (
            "kinetic_weapon",
            r#"{"damage_conditions":{"min_speed":1}}"#,
            false,
        ),
        (
            "piercing_weapon",
            r#"{"sound":{"sound_id":"custom:hit","range":false}}"#,
            true,
        ),
        (
            "piercing_weapon",
            r#"{"sound":"minecraft:item.trident.hit"}"#,
            false,
        ),
        (
            "firework_explosion",
            r#"{"shape":"star","colors":[-1,1.9]}"#,
            true,
        ),
        (
            "firework_explosion",
            r#"{"shape":"star","colors":[[1,0,0]]}"#,
            false,
        ),
        (
            "fireworks",
            r#"{"flight_duration":255,"explosions":[{"shape":"burst"}]}"#,
            true,
        ),
        ("fireworks", r#"{"flight_duration":256}"#, false),
        (
            "fireworks",
            r#"{"explosions":[{"shape":"burst"},{"colors":[]}]}"#,
            false,
        ),
        (
            "jukebox_playable",
            r#"{"sound_event":{"sound_id":"custom:sound"},"description":"record","length_in_seconds":1,"comparator_output":15}"#,
            true,
        ),
        (
            "jukebox_playable",
            r#"{"sound_event":{"sound_id":"custom:sound"},"description":{"text":"record","bold":1},"length_in_seconds":1,"comparator_output":15}"#,
            false,
        ),
        (
            "consumable",
            r#"{"animation":"spear","on_consume_effects":[{"type":"clear_all_effects"}]}"#,
            true,
        ),
        (
            "consumable",
            r#"{"on_consume_effects":[{"type":"clear_all_effects"},{"type":"play_sound"}]}"#,
            false,
        ),
        (
            "death_protection",
            r#"{"death_effects":[{"type":"apply_effects","effects":[],"probability":1}]}"#,
            true,
        ),
        (
            "death_protection",
            r#"{"death_effects":[{"type":"teleport_randomly","diameter":0}]}"#,
            false,
        ),
        (
            "equippable",
            r#"{"slot":"saddle","allowed_entities":[]}"#,
            true,
        ),
        (
            "equippable",
            r#"{"slot":"saddle","allowed_entities":["minecraft:pig"]}"#,
            false,
        ),
        ("tool", r#"{"rules":[{"blocks":[],"speed":1}]}"#, true),
        ("tool", r#"{"rules":[{"blocks":[],"speed":0}]}"#, false),
    ] {
        assert_eq!(
            payload(&format!("minecraft:{id}"), raw),
            if expected {
                Ok(())
            } else {
                Err(Error::Invalid)
            },
            "{id}: {raw}"
        );
    }
}

#[test]
fn writable_and_sign_codecs_apply_their_own_filtering_rules() {
    for (id, raw, expected) in [
        (
            "writable_book_content",
            r#"{"pages":["page",{"raw":"original","filtered":"filtered"}]}"#,
            true,
        ),
        (
            "writable_book_content",
            r#"{"pages":[{"raw":"original","filtered":1}]}"#,
            false,
        ),
        (
            "sign_text_front",
            r#"{"messages":["a","b","c","d"],"filtered_messages":[{}]}"#,
            true,
        ),
        ("sign_text_back", r#"{"messages":["a","b","c",{}]}"#, false),
        (
            "sign_text_back",
            r#"{"messages":["a","b","c","d"],"color":"not_a_color"}"#,
            false,
        ),
    ] {
        assert_eq!(
            payload(&format!("minecraft:{id}"), raw),
            if expected {
                Ok(())
            } else {
                Err(Error::Invalid)
            },
            "{id}: {raw}"
        );
    }
    let too_long = serde_json::json!({"pages":["x".repeat(1025)]}).to_string();
    assert_eq!(
        payload("minecraft:writable_book_content", &too_long),
        Err(Error::Invalid)
    );
}

#[test]
fn predicate_payload_codecs_validate_nested_values_without_runtime_matching() {
    for (id, raw, expected) in [
        (
            "can_break",
            r#"[{"blocks":[],"state":{"age":{"min":"6","max":"1"}}}]"#,
            true,
        ),
        ("can_place_on", r#"{"state":{"age":6}}"#, false),
        ("can_place_on", r#"{"nbt":{}}"#, true),
        ("lock", r#"{"count":{"min":1,"max":2}}"#, true),
        ("lock", r#"{"count":{"min":2,"max":1}}"#, false),
        (
            "lock",
            r#"{"predicates":{"minecraft:damage":{"damage":{"min":0}}}}"#,
            true,
        ),
        (
            "lock",
            r#"{"predicates":{"minecraft:damage":{"damage":{"min":"0"}}}}"#,
            false,
        ),
        (
            "lock",
            r#"{"predicates":{"minecraft:container":{"items":{"count":[{"test":{},"count":1}]}}}}"#,
            true,
        ),
        (
            "lock",
            r#"{"predicates":{"minecraft:container":{"items":{"count":[{"test":{}}]}}}}"#,
            false,
        ),
        ("lock", r#"{"predicates":{"minecraft:missing":{}}}"#, false),
        (
            "written_book_content",
            r#"{"title":"title","author":"author"}"#,
            true,
        ),
        (
            "written_book_content",
            r#"{"title":"title","author":"author","pages":[{"raw":"page","filtered":1}]}"#,
            false,
        ),
    ] {
        assert_eq!(
            payload(&format!("minecraft:{id}"), raw),
            if expected {
                Ok(())
            } else {
                Err(Error::Invalid)
            },
            "{id}: {raw}"
        );
    }
}

#[test]
fn predicate_alias_collisions_do_not_skip_the_dispatched_value_codec() {
    let registries = Registries {
        persistent_components: vec!["minecraft:damage".into()],
        ..Default::default()
    };
    let context = MetadataContext {
        registries: &registries,
    };
    // The official dispatched-map codec decodes each registered value before
    // reporting aliases as duplicate keys. This matters inside lenient fields.
    for (raw, expected) in [
        (
            r#"{"components":{"damage":0,"minecraft:damage":1}}"#,
            Err(Error::Invalid),
        ),
        (
            r#"{"components":{"damage":0,"minecraft:damage":1e2147483647}}"#,
            Err(Error::DecodeException),
        ),
        (
            r#"{"predicates":{"damage":{},"minecraft:damage":{}}}"#,
            Err(Error::Invalid),
        ),
        (
            r#"{"predicates":{"damage":{},"minecraft:damage":{"damage":1e2147483647}}}"#,
            Err(Error::DecodeException),
        ),
    ] {
        let value = parse_first(raw.as_bytes()).unwrap();
        assert_eq!(
            validate("minecraft:lock", &value, &context),
            expected,
            "{raw}"
        );
    }
}

#[test]
fn written_literal_pages_count_encoded_utf16_and_escapes() {
    for (page, expected) in [
        ("x".repeat(32765), true),
        ("x".repeat(32766), false),
        ("😀".repeat(16382), true),
        ("😀".repeat(16383), false),
        ("\"".repeat(16382), true),
        ("\"".repeat(16383), false),
    ] {
        let raw = serde_json::json!({"title":"title","author":"author","pages":[page]}).to_string();
        assert_eq!(
            payload("minecraft:written_book_content", &raw),
            if expected {
                Ok(())
            } else {
                Err(Error::Invalid)
            }
        );
    }
    // The page codec measures the decoded component. An ignored key must
    // not count, and an array is folded into the first component's siblings.
    let raw = serde_json::json!({"title":"title","author":"author","pages":[{"text":"page","ignored":"x".repeat(40000)},["first","second"]]}).to_string();
    assert_eq!(payload("minecraft:written_book_content", &raw), Ok(()));
    for (number, expected) in [
        ("0.99999999999999999999999", Ok(())),
        ("9007199254740993", Ok(())),
        ("1e-400", Ok(())),
        ("1e400", Err(Error::DecodeException)),
    ] {
        let raw = format!(
            r#"{{"title":"t","author":"a","pages":[{{"translate":"key","with":[{number}]}}]}}"#
        );
        assert_eq!(payload("minecraft:written_book_content", &raw), expected);
    }
}

#[test]
#[ignore = "requires the retained pinned metadata oracle and actual prepared catalog"]
fn retained_metadata_payloads_match_the_pinned_oracle() {
    let source = std::env::var("H146_METADATA_ORACLE").expect("retained oracle path");
    let registry_path =
        std::env::var("H146_REGISTRIES").expect("existing prepared registry export");
    let output = std::env::var("H146_METADATA_OUTPUT").expect("fresh native output path");
    let export: serde_json::Value =
        serde_json::from_slice(&std::fs::read(registry_path).unwrap()).unwrap();
    let registries = Registries::from_export(&export);
    assert_eq!(registries.persistent_components.len(), 119);
    let context = MetadataContext {
        registries: &registries,
    };
    let rows = parse_first(&std::fs::read(source).unwrap()).unwrap();
    let mut results = Vec::new();
    let mut failures = 0;
    for row in rows.as_array().unwrap() {
        let m = row.as_object().unwrap();
        let name = m["name"].as_str().unwrap();
        let expected = m["getOrThrow_admitted"].as_bool().unwrap();
        let actual = crate::component::json::validate(&m["input"], &context);
        // An unfinished backend never counts as a matching valid negative.
        let qualified = actual != Err(Error::IncompleteCodec) && actual != Err(Error::Unavailable);
        let matched = qualified && actual.is_ok() == expected;
        if !matched {
            failures += 1;
        }
        results.push(serde_json::json!({"name":name,"oracle_admitted":expected,
            "native_result":format!("{actual:?}"),"qualified":qualified,"matched":matched,
            "input":m["input"].to_string()}));
    }
    std::fs::write(output, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    assert_eq!(
        failures, 0,
        "retained literal contracts; 50 empty examples remain unestablished as family schemas"
    );
}
