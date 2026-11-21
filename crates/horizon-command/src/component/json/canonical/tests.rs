use super::*;
use crate::{
    Registries,
    component::json::{context::MetadataContext, value::parse_first},
};

#[test]
fn nested_item_encoding_discards_unknown_fields_and_omits_typed_defaults() {
    let registries = Registries {
        items: vec!["minecraft:stone".into()],
        persistent_components: [
            "damage",
            "weapon",
            "use_effects",
            "attack_range",
            "consumable",
            "custom_name",
            "written_book_content",
        ]
        .map(|s| format!("minecraft:{s}"))
        .to_vec(),
        ..Default::default()
    };
    let ctx = MetadataContext {
        registries: &registries,
    };
    let input = parse_first(br#"{"text":"page","hover_event":{"action":"show_item","id":"stone","count":1,"ignored":true,"components":{"damage":1.9,"weapon":{"item_damage_per_attack":1,"disable_blocking_for_seconds":0,"unknown":true},"use_effects":{"can_sprint":false,"interact_vibrations":true,"speed_multiplier":0.2},"attack_range":{"min_reach":0,"max_reach":3,"min_creative_reach":0,"max_creative_reach":5,"hitbox_margin":0.3,"mob_factor":1}}}}"#).unwrap();
    let expected = parse_first(br#"{"text":"page","hover_event":{"action":"show_item","id":"minecraft:stone","components":{"minecraft:damage":1,"minecraft:weapon":{},"minecraft:use_effects":{},"minecraft:attack_range":{}}}}"#).unwrap();
    assert_eq!(component(&input, &ctx), Ok(expected));

    let input = parse_first(br#"{"id":"stone","components":{"consumable":{"consume_seconds":1.6,"animation":"eat","has_consume_particles":true,"on_consume_effects":[{"type":"teleport_randomly","diameter":16,"directional_particles":true}]}}}"#).unwrap();
    let expected = parse_first(br#"{"id":"minecraft:stone","components":{"minecraft:consumable":{"on_consume_effects":[{"type":"minecraft:teleport_randomly"}]}}}"#).unwrap();
    assert_eq!(item(input.as_object().unwrap(), &ctx), Ok(expected));
}

#[test]
fn restricted_page_size_uses_nested_item_codecs_and_real_component_content() {
    let registries = Registries {
        items: vec!["minecraft:stone".into()],
        persistent_components: ["weapon", "custom_name", "written_book_content"]
            .map(|s| format!("minecraft:{s}"))
            .to_vec(),
        ..Default::default()
    };
    let ctx = MetadataContext {
        registries: &registries,
    };
    for (patch, expected) in [
        (
            serde_json::json!({"weapon":{"ignored":"x".repeat(40000)}}),
            Ok(()),
        ),
        (
            serde_json::json!({"custom_name":{"text":"x".repeat(40000)}}),
            Err(Error::Invalid),
        ),
    ] {
        let page = serde_json::json!({"text":"page","hover_event":{"action":"show_item","id":"stone","components":patch}});
        let item = serde_json::json!({"id":"stone","components":{"written_book_content":{"title":"t","author":"a","pages":[page]}}});
        let input = parse_first(item.to_string().as_bytes()).unwrap();
        assert_eq!(ctx.item_template(input.as_object().unwrap()), expected);
    }
}

#[test]
fn canonical_entity_hover_keeps_java_uuid_bits_and_decoded_name() {
    let registries = Registries {
        entity_types: vec!["minecraft:pig".into()],
        ..Default::default()
    };
    let ctx = MetadataContext {
        registries: &registries,
    };
    let input = parse_first(br#"{"text":"p","hover_event":{"action":"show_entity","id":"pig","uuid":"1-2-3-4-5","name":{"text":"n","unknown":true}}}"#).unwrap();
    let expected = parse_first(br#"{"text":"p","hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":[1,131075,262144,5],"name":"n"}}"#).unwrap();
    assert_eq!(component(&input, &ctx), Ok(expected));
}

#[test]
fn canonical_dialogs_encode_nested_templates_and_discard_unknown_size() {
    let registries = Registries {
        items: vec!["minecraft:stone".into()],
        persistent_components: vec!["minecraft:weapon".into()],
        ..Default::default()
    };
    let ctx = MetadataContext {
        registries: &registries,
    };
    let input = serde_json::json!({"text":"p","click_event":{"action":"show_dialog","dialog":{
        "type":"notice","title":{"text":"Title"},"pause":true,"ignored":"x".repeat(40000),
        "body":[{"type":"item","item":{"id":"stone","components":{"weapon":{"ignored":"x".repeat(40000)}}},"description":"Stone","width":16}],
        "inputs":[{"type":"boolean","key":"ok","label":"Okay","initial":false}],
        "action":{"label":{"translate":"gui.ok"},"width":150}
    }}});
    let expected = parse_first(br#"{"text":"p","click_event":{"action":"show_dialog","dialog":{"type":"minecraft:notice","title":"Title","body":{"type":"minecraft:item","item":{"id":"minecraft:stone","components":{"minecraft:weapon":{}}},"description":{"contents":"Stone"}},"inputs":[{"type":"minecraft:boolean","key":"ok","label":"Okay"}]}}}"#).unwrap();
    assert_eq!(
        component(&parse_first(input.to_string().as_bytes()).unwrap(), &ctx),
        Ok(expected)
    );
    // Inline controls do not need an unrelated loaded world or item registry.
    let plain = parse_first(br#"{"text":"p","click_event":{"action":"show_dialog","dialog":{"type":"notice","title":"t","inputs":[{"type":"single_option","key":"k","label":"l","options":["one"]}]}}}"#).unwrap();
    assert!(component(&plain, &crate::component::json::context::UnavailableContext).is_ok());
}

#[test]
fn canonical_predicate_payloads_use_typed_points_and_flattened_nbt() {
    let registries = Registries {
        items: vec!["minecraft:stone".into()],
        persistent_components: ["can_break", "lock", "damage"]
            .map(|s| format!("minecraft:{s}"))
            .to_vec(),
        ..Default::default()
    };
    let ctx = MetadataContext {
        registries: &registries,
    };
    let input = parse_first(br#"{"id":"stone","components":{"can_break":[{"blocks":[],"nbt":{"n":1,"s":"x"},"state":{"age":{"min":"1","unknown":true}},"components":{},"predicates":{"damage":{"damage":{},"durability":{"min":2.9,"max":2.1}}}}],"lock":{"count":{},"predicates":{"container":{"items":{"count":[{"test":{"count":{"min":3,"max":3}},"count":2}]}}}}}}"#).unwrap();
    let expected = parse_first(br#"{"id":"minecraft:stone","components":{"minecraft:can_break":{"blocks":[],"nbt":"{n:1b,s:\"x\"}","state":{"age":{"min":"1"}},"predicates":{"minecraft:damage":{"durability":2}}},"minecraft:lock":{"predicates":{"minecraft:container":{"items":{"count":[{"test":{"count":3},"count":2}]}}}}}}"#).unwrap();
    assert_eq!(item(input.as_object().unwrap(), &ctx), Ok(expected));
}
