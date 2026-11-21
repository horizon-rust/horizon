use super::{
    Context, Error, canonical,
    context::{MetadataContext, UnavailableContext},
    value::parse_first,
};
use crate::Registries;

#[test]
fn review_single_option_decodes_children_despite_width_error() {
    for (page, width, expected) in [
        ("1e2147483647", 0, Err(Error::DecodeException)),
        ("1", 0, Ok(())),
        ("1e2147483647", 200, Err(Error::DecodeException)),
        ("1", 200, Ok(())),
    ] {
        let raw = r#"{"nbt":"x","storage":"test:x","separator":{"text":"s","click_event":{"action":"show_dialog","dialog":{"type":"notice","title":"t","inputs":[{"type":"single_option","key":"k","label":"l","width":WIDTH,"options":[{"id":"a","display":{"text":"o","click_event":{"action":"change_page","page":PAGE}}}]}]}}}}"#.replace("PAGE", page).replace("WIDTH", &width.to_string());
        assert_eq!(
            super::validate(&parse_first(raw.as_bytes()).unwrap(), &UnavailableContext),
            expected,
            "{page}/{width}"
        );
    }
}

fn registry() -> Registries {
    Registries {
        items: vec!["minecraft:stone".into()],
        persistent_components: ["custom_name", "written_book_content"]
            .map(|s| format!("minecraft:{s}"))
            .to_vec(),
        ..Default::default()
    }
}

#[test]
fn review_patch_collapses_by_identity_in_decoded_input_order() {
    let registries = registry();
    let ctx = MetadataContext {
        registries: &registries,
    };
    for (patch, expected) in [
        (
            r#""custom_name":"n","!custom_name":{}"#,
            r#"{"!minecraft:custom_name":{}}"#,
        ),
        (
            r#""!custom_name":{},"minecraft:custom_name":"n""#,
            r#"{"minecraft:custom_name":"n"}"#,
        ),
        (
            r#""minecraft:custom_name":"n","!minecraft:custom_name":{}"#,
            r#"{"!minecraft:custom_name":{}}"#,
        ),
        (
            r#""!minecraft:custom_name":{},"custom_name":"n""#,
            r#"{"minecraft:custom_name":"n"}"#,
        ),
    ] {
        let raw = format!(r#"{{"id":"stone","components":{{{patch}}}}}"#);
        let v = parse_first(raw.as_bytes()).unwrap();
        let encoded = canonical::item(v.as_object().unwrap(), &ctx).unwrap();
        assert_eq!(
            &encoded.as_object().unwrap()["components"],
            &parse_first(expected.as_bytes()).unwrap(),
            "{patch}"
        );
    }
    // Same decoded PatchKey aliases remain an error; a losing value still decodes.
    for patch in [
        r#""custom_name":"a","minecraft:custom_name":"b""#,
        r#""custom_name":{},"!custom_name":{}"#,
    ] {
        let raw = format!(r#"{{"id":"stone","components":{{{patch}}}}}"#);
        assert_eq!(
            ctx.item_template(parse_first(raw.as_bytes()).unwrap().as_object().unwrap()),
            Err(Error::Invalid)
        );
    }
}

#[test]
fn review_collapsed_patch_controls_restricted_page_size() {
    let registries = registry();
    let ctx = MetadataContext {
        registries: &registries,
    };
    // Independently encoded Java set-winner page is 32760 UTF-16 code units;
    // retaining the removal too would incorrectly exceed the 32767 bound.
    for patch in [
        r#""custom_name":"n","!custom_name":{}"#,
        r#""!custom_name":{},"minecraft:custom_name":"n""#,
    ] {
        let text = "x".repeat(32646);
        let raw = format!(
            r#"{{"id":"stone","components":{{"written_book_content":{{"title":"t","author":"a","pages":[{{"text":"{text}","hover_event":{{"action":"show_item","id":"stone","components":{{{patch}}}}}}}]}}}}}}"#
        );
        assert_eq!(
            ctx.item_template(parse_first(raw.as_bytes()).unwrap().as_object().unwrap()),
            Ok(()),
            "{patch}"
        );
    }
}
