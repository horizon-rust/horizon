use super::{context::UnavailableContext, *};
use serde_json::json;

#[test]
fn json_component_admission_matches_pinned_metadata_controls() {
    // Independent observations from actual 26.3 ResourceMetadata.getSection:
    // full success, not resultOrPartial and not rendered text.
    let cases = [
        (json!("rich"), true),
        (json!(17), false),
        (json!(true), false),
        (json!(null), false),
        (json!({}), false),
        (json!([]), false),
        (json!({"text":"rich"}), true),
        (json!({"text":17}), false),
        (json!({"text":null}), false),
        (json!(["rich",{"text":"tail","bold":true}]), true),
        (json!(["rich", {}]), false),
        (json!({"type":"text","text":"rich"}), true),
        (json!({"type":"missing","text":"rich"}), false),
        (json!({"text":"rich","extra":[]}), false),
        (json!({"text":"rich","extra":[{}]}), false),
        (json!({"text":"rich","extra":null}), true),
        (
            json!({"translate":"key","fallback":"%s","with":["rich",2,true,{"text":"nested"}]}),
            true,
        ),
        (json!({"translate":"key","with":[{}]}), false),
        (json!({"text":false,"translate":"key"}), true),
        (json!({"type":"text","translate":"key"}), false),
        (json!({"keybind":"key.jump"}), true),
        (json!({"keybind":false}), false),
        (json!({"score":{"name":"Player","objective":"probe"}}), true),
        (json!({"score":{"name":"Player"}}), false),
        (json!({"selector":"@a","separator":{"text":","}}), true),
        (json!({"selector":"@not_a_selector"}), false),
        (json!({"nbt":"x","storage":"minecraft:test"}), true),
        (json!({"nbt":"x","entity":"@s"}), true),
        (json!({"nbt":"x","block":"~ ~ ~"}), true),
        (json!({"nbt":"x"}), false),
        (json!({"nbt":"[","storage":"minecraft:test"}), false),
        (json!({"nbt":"x","storage":"rich:x","interpret":1}), true),
        (
            json!({"nbt":"x","source":"missing","entity":"@s","storage":"rich:test"}),
            false,
        ),
        (
            json!({"nbt":"x","source":null,"entity":"@s","storage":"rich:test"}),
            true,
        ),
        (json!({"object":"atlas","sprite":"minecraft:test"}), true),
        (json!({"object":"atlas","sprite":"BAD:"}), false),
        (json!({"text":"rich","bold":1}), false),
        (json!({"text":"rich","color":"not_a_color"}), false),
        (
            json!({"text":"rich","font":{"type":"reference","id":"minecraft:default"}}),
            false,
        ),
        (json!({"text":"rich","shadow_color":[2,0,0,1]}), true),
        (json!({"text":"rich","shadow_color":null}), true),
        (
            json!({"text":"rich","hover_event":{"action":"show_text","value":{}}}),
            false,
        ),
        (
            json!({"text":"rich","click_event":{"action":"open_file","path":"x"}}),
            false,
        ),
        (
            json!({"text":"rich","click_event":{"action":"custom","id":"rich:test","payload":{"x":null}}}),
            false,
        ),
        (
            json!({"text":"rich","click_event":{"action":"custom","id":"rich:test","payload":[{},1,"text"]}}),
            true,
        ),
    ];
    for (value, expected) in cases {
        assert_eq!(
            validate(&value, &UnavailableContext).is_ok(),
            expected,
            "{value}"
        );
    }
}

#[test]
fn missing_registry_is_not_a_negative_component_decision() {
    let value = json!({"text":"body","hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":[1,2,3,4]}});
    assert_eq!(
        validate(&value, &UnavailableContext),
        Err(Error::Unavailable)
    );
}

#[test]
fn item_templates_borrow_the_catalog_and_require_component_codecs() {
    struct MissingPayload;
    impl context::PayloadCodecs for MissingPayload {
        fn component(&self, _: &str, _: &Value, _: &dyn Context) -> R {
            Err(Error::Unavailable)
        }
    }
    let registries = crate::Registries {
        items: vec!["minecraft:air".into(), "minecraft:stone".into()],
        persistent_components: vec!["minecraft:custom_name".into()],
        ..Default::default()
    };
    let context = context::BorrowedContext {
        registries: &registries,
        payloads: &MissingPayload,
    };
    for (item, expected) in [
        (json!({"id":"minecraft:stone"}), Ok(())),
        (json!({"id":"minecraft:stone","count":1.9}), Ok(())),
        (
            json!({"id":"minecraft:stone","count":4294967297_u64}),
            Ok(()),
        ),
        (json!({"id":"minecraft:air"}), Err(Error::Invalid)),
        (json!({"id":"rich:no_such_item"}), Err(Error::Invalid)),
        (
            json!({"id":"minecraft:stone","count":100}),
            Err(Error::Invalid),
        ),
        (
            json!({"id":"minecraft:stone","components":{"!minecraft:custom_name":{}}}),
            Ok(()),
        ),
        (
            json!({"id":"minecraft:stone","components":{"!minecraft:custom_name":{"ignored":1}}}),
            Ok(()),
        ),
        (
            json!({"id":"minecraft:stone","components":{"!minecraft:custom_name":1}}),
            Err(Error::Invalid),
        ),
        (
            json!({"id":"minecraft:stone","components":{"!!minecraft:custom_name":{}}}),
            Err(Error::Invalid),
        ),
        (
            json!({"id":"minecraft:stone","components":{"minecraft:custom_name":"name"}}),
            Err(Error::Unavailable),
        ),
    ] {
        assert_eq!(
            context.item_template(Value::from(&item).as_object().unwrap()),
            expected,
            "{item}"
        );
    }
    let value = json!({"text":"body","click_event":{"action":"show_dialog","dialog":{
        "type":"notice","title":"title","body":{"type":"item","item":"minecraft:stone"}
    }}});
    assert_eq!(validate(&value, &context), Ok(()));
}

#[test]
fn json_number_and_uuid_alternatives_keep_their_java_boundaries() {
    for (value, valid) in [
        (json!(2147483648_u64), false),
        (json!(4294967297_u64), true),
        (json!(1.5), true),
        (json!(-0.5), false),
        (json!(1e20), true),
    ] {
        let value = json!({"text":"body","click_event":{"action":"change_page","page":value}});
        assert_eq!(
            validate(&value, &UnavailableContext).is_ok(),
            valid,
            "{value}"
        );
    }
    for (value, valid) in [
        ("1-1-1-1-1", true),
        ("000000001-1-1-1-1", true),
        ("ffffffffffffffff-1-1-1-1", false),
        ("7fffffffffffffff-1-1-1-1", true),
        ("+1-1-1-1-1", true),
        ("1-1-1-1-1-1", false),
        ("00000001000000010000000100000001", false),
    ] {
        assert_eq!(java_uuid_words(value).is_some(), valid, "{value}");
    }
}
#[test]
fn raw_number_exceptions_escape_lenient_component_fields() {
    let context = super::context::UnavailableContext;
    for (raw, expected) in [
        (
            r#"{"nbt":"x","storage":"test:x","separator":{"text":"x","bold":1}}"#,
            Ok(()),
        ),
        (
            r#"{"nbt":"x","storage":"test:x","separator":{"text":"x","click_event":{"action":"change_page","page":1e2147483647}}}"#,
            Err(super::Error::DecodeException),
        ),
        (
            r#"{"text":"x","click_event":{"action":"custom","id":"test:x","payload":{"n":1e400}}}"#,
            Ok(()),
        ),
        (
            r#"{"text":"x","click_event":{"action":"custom","id":"test:x","payload":{"n":1e2147483647}}}"#,
            Err(super::Error::DecodeException),
        ),
    ] {
        let value = super::value::parse_first(raw.as_bytes()).unwrap();
        assert_eq!(super::validate(&value, &context), expected, "{raw}");
    }
}

#[test]
fn numeric_exceptions_are_not_hidden_by_earlier_record_or_list_errors() {
    // Six newly admitted eager-boundary cases, without replaying old forms.
    let registries = crate::Registries {
        items: vec!["minecraft:stone".into()],
        ..Default::default()
    };
    let context = super::context::MetadataContext {
        registries: &registries,
    };
    for inner in [
        r#"{"text":"x","extra":[],"click_event":{"action":"change_page","page":1e2147483647}}"#,
        r#"{"text":"x","bold":1,"click_event":{"action":"change_page","page":1e2147483647}}"#,
        r#"{"nbt":"[","storage":"x","separator":{"text":"n","click_event":{"action":"change_page","page":1e2147483647}}}"#,
        r#"[null,{"text":"n","click_event":{"action":"change_page","page":1e2147483647}}]"#,
        r#"{"text":"x","click_event":{"action":"show_dialog","dialog":{"type":"notice","title":null,"inputs":[{"type":"text","key":"k","label":"l","max_length":1e2147483647}]}}}"#,
        r#"{"text":"x","hover_event":{"action":"show_item","id":"not registered","count":1e2147483647}}"#,
    ] {
        let raw = format!(r#"{{"nbt":"foo","storage":"test:root","separator":{inner}}}"#);
        let value = super::value::parse_first(raw.as_bytes()).unwrap();
        assert_eq!(
            validate(&value, &context),
            Err(Error::DecodeException),
            "{inner}"
        );
    }
}

#[test]
fn translation_arguments_try_java_conversion_before_component_fallback() {
    let context = super::context::UnavailableContext;
    for (raw, expected) in [
        (
            r#"{"translate":"test","with":[1e2147483647]}"#,
            Err(Error::DecodeException),
        ),
        (
            r#"{"translate":"test","with":[{"text":"ok","ignored":1e2147483647}]}"#,
            Err(Error::DecodeException),
        ),
        (
            r#"{"translate":"test","with":[{"text":"ok","ignored":1e400}]}"#,
            Ok(()),
        ),
        (
            r#"{"translate":"test","with":[{"text":"ok","ignored":null}]}"#,
            Err(Error::DecodeException),
        ),
        (r#"{"translate":"test","with":[null]}"#, Err(Error::Invalid)),
    ] {
        let v = super::value::parse_first(raw.as_bytes()).unwrap();
        assert_eq!(validate(&v, &context), expected, "{raw}");
    }
}
