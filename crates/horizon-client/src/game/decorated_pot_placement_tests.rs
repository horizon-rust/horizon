//! #16482: a normal block-item use must keep a raw decorated-pot entity.
//! Loaded as a child test module of `block_items`; no production fixture hooks.
use super::*;
use crate::game::native_machines::tests::server_game;
use horizon_client::mod_api::NativeUseTarget;
use horizon_core::{geometry::Direction, position::BlockPos};
use std::sync::Arc;

const POS: [i32; 3] = [4, 200, 4];
const DECORATIONS: &str = r#"{back:{id:"minecraft:angler_pottery_sherd"},left:{id:"minecraft:archer_pottery_sherd"},right:{id:"minecraft:skull_pottery_sherd"},front:{id:"minecraft:prize_pottery_sherd",count:3,components:{"minecraft:custom_data":{regression:16482},"minecraft:custom_name":"Front"}}}"#;
const CONTENTS: &str =
    r#"{id:"minecraft:diamond",count:3,components:{"minecraft:custom_name":"Inside"}}"#;

fn parse(input: &str) -> Tag {
    let (tag, used) = horizon_world::nbt::parse_snbt(input).expect("valid test NBT");
    assert_eq!(used, input.len());
    tag
}

/// Unlike calling apply_placed_item directly, this traverses use_item/use_hand,
/// including the placement caller's decision whether the new entity is modeled.
fn place(game: &mut Game<'_>, stack: ItemStack) {
    let used = game
        .native_use_as_player(
            stack,
            [4.5, 201.5, 6.5],
            180.0,
            45.0,
            NativeUseTarget::Block {
                pos: [4, 199, 4],
                face: Direction::Up,
                point: [4.5, 200.0, 4.5],
            },
            false,
        )
        .expect("an authoritative server with no player acting");
    assert!(used.consumed, "the ordinary block-item use succeeded");
    assert!(
        used.held.is_none(),
        "survival placement consumed the one pot"
    );
    assert_eq!(
        &*game.res.facts().get(game.world.block(4, 200, 4)).name,
        "minecraft:decorated_pot"
    );
}

fn views(game: &Game<'_>, decorated: bool) {
    let saved = game.block_entity_nbt(POS).expect("actual pot entity NBT");
    for (key, value) in [("x", 4), ("y", 200), ("z", 4)] {
        assert_eq!(saved.get(key), Some(&Tag::Int(value)));
    }
    assert_eq!(
        saved.get("id"),
        Some(&Tag::String("minecraft:decorated_pot".into()))
    );
    let sherds = Some(if decorated {
        parse(DECORATIONS)
    } else {
        parse(
            r#"{back:{id:"minecraft:brick"},left:{id:"minecraft:brick"},right:{id:"minecraft:brick"},front:{id:"minecraft:brick"}}"#,
        )
    });
    let contents = decorated.then(|| parse(CONTENTS));
    assert_eq!(saved.get("sherds"), sherds.as_ref());
    assert_eq!(saved.get("item"), contents.as_ref());
    for include_data in [false, true] {
        let picked = game
            .picked_stack(POS, include_data)
            .expect("actual Pick Block");
        assert_eq!(
            (&*picked.item, picked.count),
            ("minecraft:decorated_pot", 1)
        );
        let patch = Tag::Compound(components(&picked));
        let actual = patch.get("minecraft:pot_decorations");
        if decorated {
            assert_eq!(
                actual,
                sherds.as_ref(),
                "all four sides, counts and components survive Pick Block"
            );
        } else {
            assert!(
                actual.is_none_or(|tag| tag == &parse("{}")),
                "a new default pot must not inherit old decorations"
            );
        }
        if include_data && decorated {
            assert_eq!(
                patch.get("minecraft:container"),
                Some(&parse(&format!("[{{slot:0,item:{CONTENTS}}}]")))
            );
        } else {
            assert!(
                patch
                    .get("minecraft:container")
                    .is_none_or(|tag| tag == &parse("[]")),
                "ordinary Pick Block copies decorations, not stored contents"
            );
        }
    }
}

/// Use the same chunk snapshot and codecs as saving, then actually install the
/// decoded chunk before reading or picking again. No expected value is obtained
/// from block_entity_item::place/pick or the block-entity canonicalizer.
fn reload(game: &mut Game<'_>, decorated: bool) {
    let resident = game.world.chunks.get(&(0, 0)).expect("resident chunk");
    let snapshot = game.with_entities([resident]).remove(0);
    let encoded = save::encode(&snapshot, game.pack.assets.registry, game.res.biomes());
    let Tag::List(entries) = encoded.tag.get("block_entities").expect("saved entities") else {
        panic!("a saved entity list")
    };
    let pots: Vec<_> = entries
        .iter()
        .filter(|tag| {
            tag.get("x") == Some(&Tag::Int(4))
                && tag.get("y") == Some(&Tag::Int(200))
                && tag.get("z") == Some(&Tag::Int(4))
        })
        .collect();
    assert_eq!(
        pots.len(),
        1,
        "exactly one saved pot, no stale raw duplicate"
    );
    let sherds = Some(if decorated {
        parse(DECORATIONS)
    } else {
        parse(
            r#"{back:{id:"minecraft:brick"},left:{id:"minecraft:brick"},right:{id:"minecraft:brick"},front:{id:"minecraft:brick"}}"#,
        )
    });
    let contents = decorated.then(|| parse(CONTENTS));
    assert_eq!(pots[0].get("sherds"), sherds.as_ref());
    assert_eq!(pots[0].get("item"), contents.as_ref());
    let decoded = save::decode(&encoded, game.pack.assets.registry, game.res.biomes())
        .expect("saved chunk decodes");
    game.world.chunks.insert((0, 0), Arc::new(decoded));
    views(game, decorated);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and scratch homes/cwd"]
fn ordinary_pot_placement_survives_pick_reload_and_default_replacement() {
    let mut game = server_game();
    game.build_registries();
    game.set_allow_commands(true);
    game.block_ticks
        .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
    let clock = game.world.game_time;
    let source = parse(&format!(
        "{{id:\"minecraft:decorated_pot\",count:1,components:{{\"minecraft:pot_decorations\":{DECORATIONS},\"minecraft:container\":[{{slot:0,item:{CONTENTS}}}]}}}}"
    ));
    let stack = save::item_from_nbt(&source).expect("real component-bearing item codec");
    place(&mut game, stack);
    views(&game, true);
    let update = horizon_client::lan::chunks::pot_tag(&game.world.chunks[&(0, 0)], (4, 200, 4))
        .expect("the server's block entity update tag");
    assert_eq!(update.get("sherds"), Some(&parse(DECORATIONS)));
    assert_eq!(update.get("item"), Some(&parse(CONTENTS)));
    reload(&mut game, true);

    // Destroying an intact pot copies its decorations into the pot drop and
    // spills the stored item separately (DecoratedPotBlock.getDrops).
    assert!(game.run_command("setblock 4 200 4 air destroy"));
    game.flush_spill();
    let pot = game
        .dropped
        .iter()
        .find(|item| &*item.stack.item == "minecraft:decorated_pot")
        .expect("the intact pot dropped");
    let patch = Tag::Compound(components(&pot.stack));
    assert_eq!(
        patch.get("minecraft:pot_decorations"),
        Some(&parse(DECORATIONS))
    );
    assert!(patch.get("minecraft:container").is_none());
    let contents = save::item_from_nbt(&parse(CONTENTS)).unwrap();
    assert!(game.dropped.iter().any(|item| item.stack == contents));

    // A new default pot at the very same position must not resurrect the old
    // raw entity's templates or contents, including after another chunk save.
    place(&mut game, ItemStack::new("minecraft:decorated_pot", 1));
    views(&game, false);
    reload(&mut game, false);
    assert_eq!(
        game.world.game_time, clock,
        "no ticks needed to retain item data"
    );
}
