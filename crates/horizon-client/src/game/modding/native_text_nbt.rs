//! Text as item components and block entities store it.
use super::Game;
use super::native_text_grammar::build;
use crate::game::api_values::TryIntoApi;
use horizon_client::command::Registries;
use horizon_client::{api_text, lan::data::component_nbt};
use horizon_core::nbt::{Tag, within_depth};
use mod_api::{Text, TextFormError};

/// The deepest NBT a mod-set text may have. The writer takes 512 containers, but the same item is saved on
/// many paths that already use a good part of them (a player's `Inventory`, a chest, a dropped item or an item
/// frame record, an item inside a shulker box or a bundle, and the writer's wrappers for mixed lists), so
/// the cap is a small number every such path fits: 64 containers.
const NBT_DEPTH: usize = 64;

/// A component's JSON as item components and block entities store it: a plain literal with no style
/// and no children is its string (as `ComponentSerialization` saves it, so two stacks that carry the
/// same plain name compare equal), anything else a compound. A form deeper than the NBT writer takes
/// is refused: each child adds an `extra` list and a compound, so a chain the text depth check let
/// through can still be too deep here.
fn component_to_nbt(component: &serde_json::Value) -> Result<Tag, TextFormError> {
    let tag = match component {
        serde_json::Value::Object(o)
            if o.len() == 1 && o.get("text").is_some_and(|t| t.is_string()) =>
        {
            component_nbt(&o["text"])
        }
        other => component_nbt(other),
    };
    if within_depth(&tag, NBT_DEPTH) {
        Ok(tag)
    } else {
        Err(TextFormError::TooDeep)
    }
}

/// The text as item components store it. The JSON between the text and its NBT has no tag widths
/// or typed arrays, so a custom click's payload is not read back from it: its slot is replaced
/// by the tag the mod gave, exactly.
fn text_nbt(text: &Text) -> Result<Tag, TextFormError> {
    let (component, payloads) = api_text::component_with_payload_slots(text)?;
    let mut tag = component_to_nbt(&component)?;
    api_text::restore_payloads(&mut tag, &payloads);
    // The payloads count toward the depth now.
    if within_depth(&tag, NBT_DEPTH) {
        Ok(tag)
    } else {
        Err(TextFormError::TooDeep)
    }
}

fn read_text_nbt(tag: &Tag, registries: &Registries) -> Result<Option<Text>, TextFormError> {
    let (component, nbt) = api_text::ComponentNbt::project(tag, NBT_DEPTH)?;
    build(&component, registries, true)
        .map(|component| nbt.capture(&component))
        .transpose()
}

/// A borrowed request for the generated host forwarder to convert directly to API text.
pub(super) struct ReadNbt<'a> {
    tag: &'a Tag,
    registries: &'a Registries,
}

impl TryIntoApi for ReadNbt<'_> {
    type Api = Option<Text>;
    type Error = TextFormError;

    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        read_text_nbt(self.tag, self.registries)
    }
}

impl Game<'_> {
    pub(super) fn text_from_nbt<'a>(&'a self, tag: &'a Tag) -> ReadNbt<'a> {
        ReadNbt {
            tag,
            registries: &self.ui.registries,
        }
    }

    /// The text as item components store it; takes the mod's own text (`#[raw_text]`).
    pub(super) fn text_to_nbt(&self, text: &Text) -> Result<Tag, TextFormError> {
        text_nbt(text)
    }
}

#[cfg(test)]
mod tests {
    use super::{Registries, component_nbt, component_to_nbt, read_text_nbt, text_nbt};
    use horizon_client::text::Text;
    use horizon_core::nbt::Tag;
    use mod_api::text::{TextClick, TextHover};
    use mod_api::{Text as ApiText, TextFormError};
    use serde_json::json;

    /// Driven by tools/test_native_text_nbt_parity.py against the pinned vanilla codec.
    #[test]
    #[ignore = "vanilla parity harness"]
    fn vanilla_nbt_oracle() {
        let cases =
            std::fs::read_to_string(std::env::var("HORIZON_TEXT_NBT_CASES").unwrap()).unwrap();
        let expected =
            std::fs::read_to_string(std::env::var("HORIZON_TEXT_NBT_ORACLE").unwrap()).unwrap();
        let cases: Vec<_> = cases.lines().collect();
        let expected: Vec<_> = expected.lines().collect();
        assert_eq!(cases.len(), expected.len());
        for (source, expected) in cases.into_iter().zip(expected) {
            let (tag, used) = horizon_core::nbt::parse_snbt(source).unwrap();
            assert_eq!(used, source.len());
            let text = read_text_nbt(&tag, &Registries::default()).unwrap();
            fn pages(text: &ApiText, out: &mut Vec<i32>) {
                if let Some(TextClick::ChangePage(page)) = &text.style.click {
                    out.push(*page);
                }
                if let Some(TextHover::Text(hover)) = &text.style.hover {
                    pages(hover, out);
                }
                for child in &text.children {
                    pages(child, out);
                }
            }
            let actual = match text {
                None => "reject".to_owned(),
                Some(text) => {
                    let mut found = Vec::new();
                    pages(&text, &mut found);
                    let found = format!("{found:?}").replace(' ', "");
                    format!("ok {found}")
                }
            };
            assert_eq!(actual, expected, "{source}");
        }
    }

    /// A plain unstyled literal is its string, a styled one stays a compound: both forms of the same
    /// plain name are one component.
    #[test]
    fn a_plain_text_is_its_string_and_a_styled_one_a_compound() {
        assert_eq!(
            component_to_nbt(&json!({"text": "Prize"})),
            Ok(Tag::String("Prize".into()))
        );
        assert!(matches!(
            component_to_nbt(&json!({"text": "Prize", "bold": true})),
            Ok(Tag::Compound(_))
        ));
        assert!(matches!(
            component_to_nbt(&json!({"text": "a", "extra": [{"text": "b"}]})),
            Ok(Tag::Compound(_))
        ));
    }

    /// A chain of 257 children passes the 512-node text check but its NBT has 513 nested containers; one of
    /// 255 fits the writer alone and not once the item's own containers are counted.
    #[test]
    fn a_text_whose_nbt_is_too_deep_is_refused() {
        let chain = |n: usize| {
            (0..n).fold(
                json!({"text": "x"}),
                |inner, _| json!({"text": "", "extra": [inner]}),
            )
        };
        assert_eq!(component_to_nbt(&chain(257)), Err(TextFormError::TooDeep));
        assert_eq!(component_to_nbt(&chain(200)), Err(TextFormError::TooDeep));
        assert_eq!(component_to_nbt(&chain(32)), Err(TextFormError::TooDeep));
        assert!(component_to_nbt(&chain(31)).is_ok());
        assert!(component_to_nbt(&chain(1)).is_ok());
    }

    /// The NBT reads back as the same component, as an item's name does.
    #[test]
    fn text_to_nbt_is_the_components_nbt_form() {
        let text = Text::legacy("\u{a7}6Golden \u{a7}lsword");
        let tag = component_nbt(&text.component());
        let back = horizon_client::component::name_text(&tag);
        assert_eq!(back.plain(), "Golden sword");
        let styles: Vec<_> = back.chars().map(|(_, style)| style.clone()).collect();
        assert_eq!(styles[0].color, Some(0xFFAA00), "gold");
        assert!(!styles[0].is_bold());
        assert!(styles[7].is_bold(), "the second word is bold");
    }

    /// A custom click's payload is kept without interpretation: the NBT holds the tags the mod gave,
    /// typed arrays and numeric widths included, not what a JSON round trip makes of them.
    #[test]
    fn a_custom_click_payload_keeps_its_exact_tags() {
        let nested = Tag::compound([
            ("flag", Tag::Byte(1)),
            ("ratio", Tag::Float(1.5f32.to_bits())),
            ("bytes", Tag::ByteArray(vec![1, 2, 3])),
        ]);
        let payloads = [
            Tag::ByteArray(vec![0, 1, 255]),
            Tag::IntArray(vec![-1, 0, 70_000]),
            Tag::LongArray(vec![i64::MIN, 0, i64::MAX]),
            nested,
            Tag::compound([("looks_like_a_slot", Tag::String("\0payload:0".into()))]),
        ];
        let click = |index: usize, payload: &Tag| {
            let mut text = ApiText::literal(format!("click {index}"));
            text.style.click = Some(TextClick::Custom {
                id: format!("test:action_{index}"),
                payload: Some(payload.clone()),
            });
            text
        };
        let mut root = ApiText::literal("");
        root.style.color = Some("gold".into());
        root.style.bold = Some(true);
        root.style.italic = Some(false);
        root.children = payloads
            .iter()
            .enumerate()
            .map(|(index, payload)| click(index, payload))
            .collect();
        // A click inside a hover text sits somewhere else in the component than a child's.
        let mut hovered = ApiText::literal("hover");
        hovered.style.hover = Some(TextHover::Text(Box::new(click(9, &payloads[3]))));
        root.children.push(hovered);

        let tag = text_nbt(&root).unwrap();
        let Some(Tag::List(extra)) = tag.get("extra") else {
            panic!("the children are the extra list");
        };
        let payload_of = |component: &Tag| {
            component
                .get("click_event")
                .and_then(|event| event.get("payload"))
                .cloned()
        };
        for (index, payload) in payloads.iter().enumerate() {
            assert_eq!(payload_of(&extra[index]).as_ref(), Some(payload), "{index}");
            let id = extra[index].get("click_event").and_then(|e| e.get("id"));
            assert_eq!(
                id,
                Some(&Tag::String(format!("test:action_{index}").as_str().into()))
            );
        }
        let hover = extra[payloads.len()]
            .get("hover_event")
            .and_then(|event| event.get("value"))
            .expect("the hover text");
        assert_eq!(payload_of(hover).as_ref(), Some(&payloads[3]));
        assert_eq!(read_text_nbt(&tag, &Registries::default()), Ok(Some(root)));
    }

    /// A payload as deep as the text check lets it pass still has to fit the NBT writer once it
    /// sits in the item's containers.
    #[test]
    fn a_custom_click_payload_counts_toward_the_depth_limit() {
        let deep = |n: usize| (0..n).fold(Tag::Int(0), |inner, _| Tag::compound([("x", inner)]));
        let with = |n: usize| {
            let mut text = ApiText::literal("x");
            text.style.click = Some(TextClick::Custom {
                id: "test:deep".into(),
                payload: Some(deep(n)),
            });
            text_nbt(&text)
        };
        assert!(with(10).is_ok());
        assert!(with(62).is_ok());
        assert_eq!(with(63), Err(TextFormError::TooDeep));

        let too_deep = Tag::compound([
            ("text", Tag::String("x".into())),
            (
                "click_event",
                Tag::compound([
                    ("action", Tag::String("custom".into())),
                    ("id", Tag::String("test:deep".into())),
                    ("payload", deep(63)),
                ]),
            ),
        ]);
        assert_eq!(
            read_text_nbt(&too_deep, &Registries::default()),
            Err(TextFormError::TooDeep)
        );
    }

    /// Only the slots this conversion made are restored: a `click_event` inside the opaque components of a
    /// hover item whose payload is text that looks like a slot (also with the right prefix) stays as written.
    #[test]
    fn text_that_looks_like_a_payload_slot_is_not_replaced() {
        let forged = |payload: &str| {
            Tag::compound([(
                "minecraft:custom_data",
                Tag::compound([(
                    "click_event",
                    Tag::compound([
                        ("action", Tag::String("custom".into())),
                        ("payload", Tag::String(payload.into())),
                    ]),
                )]),
            )])
        };
        for lookalike in [
            "\0payload:0",
            "\0payload:0000000000000000:0",
            "\0payload:ffffffffffffffff:0",
        ] {
            let mut hovered = ApiText::literal("hover");
            hovered.style.hover = Some(TextHover::Item {
                id: "minecraft:stone".into(),
                count: 1,
                components: Some(forged(lookalike)),
            });
            let mut root = ApiText::literal("");
            let mut clicked = ApiText::literal("click");
            clicked.style.click = Some(TextClick::Custom {
                id: "test:real".into(),
                payload: Some(Tag::ByteArray(vec![9])),
            });
            root.children = vec![hovered, clicked];
            let tag = text_nbt(&root).unwrap();
            let Some(Tag::List(extra)) = tag.get("extra") else {
                panic!("children");
            };
            let found = extra[0]
                .get("hover_event")
                .and_then(|event| event.get("components"))
                .and_then(|c| c.get("minecraft:custom_data"))
                .and_then(|d| d.get("click_event"))
                .and_then(|e| e.get("payload"));
            assert_eq!(found, Some(&Tag::String(lookalike.into())), "{lookalike}");
            let real = extra[1].get("click_event").and_then(|e| e.get("payload"));
            assert_eq!(real, Some(&Tag::ByteArray(vec![9])));
            assert_eq!(read_text_nbt(&tag, &Registries::default()), Ok(Some(root)));
        }
    }

    #[test]
    fn reading_nbt_distinguishes_invalid_components_and_unrepresentable_text() {
        let registries = Registries::default();
        assert_eq!(read_text_nbt(&Tag::Int(1), &registries), Ok(None));
        assert_eq!(
            read_text_nbt(&Tag::compound([("text", Tag::Int(1))]), &registries),
            Ok(None)
        );
        let invalid_string = Tag::String(horizon_core::nbt::JavaString(vec![0xd800]));
        assert_eq!(
            read_text_nbt(&invalid_string, &registries),
            Err(TextFormError::InvalidString)
        );
        let payload = Tag::compound([
            ("nan", Tag::Double(f64::NAN.to_bits())),
            ("utf16", invalid_string),
        ]);
        let tag = Tag::compound([
            ("text", Tag::String("click".into())),
            (
                "click_event",
                Tag::compound([
                    ("action", Tag::String("custom".into())),
                    ("id", Tag::String("test:action".into())),
                    ("payload", payload.clone()),
                ]),
            ),
        ]);
        assert_eq!(
            read_text_nbt(&tag, &registries)
                .unwrap()
                .unwrap()
                .style
                .click,
            Some(TextClick::Custom {
                id: "test:action".into(),
                payload: Some(payload),
            })
        );
        let literal = ApiText::literal("§aPrize");
        assert_eq!(
            read_text_nbt(&text_nbt(&literal).unwrap(), &registries),
            Ok(Some(literal))
        );
    }

    #[test]
    #[ignore = "daily"]
    fn the_native_host_round_trips_item_display_metadata() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::{Host, Identifier, ItemStack};

        with_game(|game| {
            let host: &dyn Host = game;
            let key = host
                .item_key(&Identifier::parse("minecraft:stone").unwrap())
                .unwrap();
            let mut stack = ItemStack::of(key);
            let mut name = ApiText::literal("Prize");
            name.style.color = Some("gold".into());
            name.style.bold = Some(true);
            name.style.italic = Some(false);
            let lore = [ApiText::literal("First"), ApiText::literal("Second")];
            assert_eq!(
                host.text_from_nbt(&host.text_to_nbt(&name).unwrap()),
                Ok(Some(name.clone()))
            );
            assert_eq!(host.text_from_nbt(&Tag::Int(1)), Ok(None));
            stack.set_display(host, Some(&name), &lore).unwrap();
            assert_eq!(stack.display_name(host), Ok(Some(name)));
            assert_eq!(stack.lore(host), Ok(lore.to_vec()));
            stack.set_display(host, None, &[]).unwrap();
            assert_eq!(stack.display_name(host), Ok(None));
            assert_eq!(stack.lore(host), Ok(vec![]));
        });
    }
}
