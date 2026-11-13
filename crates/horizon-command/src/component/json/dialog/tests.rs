use super::super::context::UnavailableContext;
use super::*;
use serde_json::json;

#[test]
fn inline_dialogs_obey_full_codecs_without_a_world_registry() {
    // Pinned ResourceMetadata/getOrThrow observations. These include errors
    // with partial Components, which must never be promoted to full success.
    let cases = [
        (json!({"type":"notice","title":"title"}), true),
        (
            json!({"type":"confirmation","title":"title","yes":{"label":"yes"},"no":{"label":"no"}}),
            true,
        ),
        (
            json!({"type":"multi_action","title":"title","actions":[{"label":"ok"}]}),
            true,
        ),
        (json!({"type":"server_links","title":"title"}), true),
        (
            json!({"type":"dialog_list","title":"title","dialogs":[]}),
            true,
        ),
        (
            json!({"type":"dialog_list","title":"title","dialogs":{"type":"notice","title":"inner"}}),
            true,
        ),
        (
            json!({"type":"dialog_list","title":"title","dialogs":["minecraft:custom_options"]}),
            false,
        ),
        (json!({"type":"notice","title":{}}), false),
        (
            json!({"type":"notice","title":"title","pause":true,"after_action":"none"}),
            false,
        ),
        (
            json!({"type":"notice","title":"title","pause":true,"after_action":"wait_for_response"}),
            true,
        ),
        (
            json!({"type":"notice","title":"title","action":{"label":"ok","width":0}}),
            false,
        ),
        (
            json!({"type":"notice","title":"title","body":{"type":"plain_message","contents":{}}}),
            false,
        ),
    ];
    for (value, expected) in cases {
        let rich = json!({"text":"body","click_event":{"action":"show_dialog","dialog":value}});
        assert_eq!(
            component(&rich, &UnavailableContext).is_ok(),
            expected,
            "{value}"
        );
    }
}

#[test]
fn dialog_input_types_keep_utf16_and_typed_bounds() {
    for (input, expected) in [
        (json!({"type":"text","key":"é"}), true),
        (json!({"type":"text","key":"\u{0345}"}), false),
        (json!({"type":"text","key":"\u{10400}"}), false),
        (json!({"type":"text","key":""}), true),
        (json!({"type":"text","initial":"😀","max_length":1}), false),
        (json!({"type":"text","multiline":{"height":513}}), false),
        (json!({"type":"text","multiline":{}}), true),
        (
            json!({"type":"boolean","initial":true,"on_true":"yes","on_false":"no"}),
            true,
        ),
        (json!({"type":"boolean","initial":1}), false),
        (
            json!({"type":"single_option","options":["a",{"id":"b","initial":true}]}),
            true,
        ),
        (
            json!({"type":"single_option","options":[{"id":"a","initial":true},{"id":"b","initial":true}]}),
            false,
        ),
        (json!({"type":"single_option","options":[]}), false),
        (
            json!({"type":"number_range","start":3,"end":1,"initial":2,"step":0.5}),
            true,
        ),
        (
            json!({"type":"number_range","start":1,"end":3,"initial":4}),
            false,
        ),
        (
            json!({"type":"number_range","start":1,"end":3,"step":0}),
            false,
        ),
    ] {
        let mut input = input.as_object().unwrap().clone();
        input.entry("key").or_insert(json!("field"));
        input.insert("label".into(), json!("label"));
        let dialog = json!({"type":"notice","title":"title","inputs":[input]});
        assert_eq!(
            validate(&dialog, &UnavailableContext).is_ok(),
            expected,
            "{dialog}"
        );
    }
}

#[test]
fn static_and_dynamic_dialog_actions_decode_their_payloads() {
    for (action, expected) in [
        (
            json!({"type":"dynamic/run_command","template":"say $(é)"}),
            true,
        ),
        (
            json!({"type":"dynamic/run_command","template":"say $()"}),
            true,
        ),
        (
            json!({"type":"dynamic/run_command","template":"say fixed"}),
            false,
        ),
        (
            json!({"type":"dynamic/run_command","template":"say $(x"}),
            false,
        ),
        (
            json!({"type":"dynamic/run_command","template":"say $(bad-name)"}),
            false,
        ),
        (
            json!({"type":"dynamic/custom","id":"rich:action","additions":{"x":1}}),
            true,
        ),
        (
            json!({"type":"dynamic/custom","id":"rich:action","additions":[]}),
            false,
        ),
        (json!({"type":"open_file","path":"x"}), false),
        (json!({"type":"run_command","command":"say hi"}), true),
    ] {
        let dialog =
            json!({"type":"notice","title":"title","action":{"label":"ok","action":action}});
        assert_eq!(
            validate(&dialog, &UnavailableContext).is_ok(),
            expected,
            "{dialog}"
        );
    }
}
