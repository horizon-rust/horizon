//! The criteria that only ask where the player is are asked again when anything they answer from
//! changes: the biome there (`/fillbiome`), the block, the dimension, the grants, the tree
//! (`/reload` makes another with the data packs' tags); and never skipped when they ask for a
//! structure.
use super::*;
use crate::game::native_guest_turn_tests::with_guests;

fn criterion(location: &str) -> String {
    format!(
        r#"{{"criteria": {{"c": {{"trigger": "minecraft:location", "conditions": {{"player":
        {{"type": "minecraft:entity_properties", "entity": "this",
        "predicate": {{"minecraft:location": {location}}}}}}}}}}}}}"#
    )
}

/// `/fillbiome` over the chunks about the guests, done on the chunks (the command needs the
/// level's own bookkeeping): every cell of the columns is `biome`.
fn fill(game: &mut Game<'_>, biome: &str) {
    let id = game.res.biomes().id(biome);
    let height = game.world.layout().height() / 4;
    for x in -1..=1 {
        for z in -1..=1 {
            if let Some(chunk) = game.world.chunks.get_mut(&(x, z)) {
                let chunk = Arc::make_mut(chunk);
                for qy in 0..height {
                    for qx in 0..4 {
                        for qz in 0..4 {
                            chunk.set_biome_cell(qx, qy, qz, id);
                        }
                    }
                }
            }
        }
    }
}

/// The column the guests stand in, empty, if the fixture has none.
fn column(game: &mut Game<'_>) {
    if !game.world.chunks.contains_key(&(0, 0)) {
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
    }
}

fn tree(dir: &str, extra: &[(&str, &str)]) -> Arc<Tree> {
    let scratch = std::env::current_dir().unwrap().join(dir);
    let advancements = scratch.join("data/testmod/advancement");
    std::fs::create_dir_all(&advancements).unwrap();
    let mut all = vec![
        ("plains", r#"{"biomes": "minecraft:plains"}"#),
        ("desert", r#"{"biomes": "minecraft:desert"}"#),
        ("forest", r##"{"biomes": "#minecraft:is_forest"}"##),
        ("nether", r#"{"dimension": "minecraft:the_nether"}"#),
        ("village", r#"{"structures": "minecraft:village_plains"}"#),
        (
            "block",
            r#"{"biomes": "minecraft:swamp", "block": {"blocks": "minecraft:stone"}}"#,
        ),
    ];
    all.extend_from_slice(extra);
    for (name, location) in all {
        std::fs::write(
            advancements.join(format!("{name}.json")),
            criterion(location),
        )
        .unwrap();
    }
    let tree = Arc::new(Tree::load(&scratch.join("data")));
    let _ = std::fs::remove_dir_all(&scratch);
    tree
}

#[test]
fn only_biome_and_dimension_criteria_are_remembered() {
    let ask = |location: &str| {
        let v: Value = serde_json::from_str(&criterion(location)).unwrap();
        asks_only_where(&v["criteria"]["c"]["conditions"])
    };
    assert!(ask(r#"{"biomes": "minecraft:plains"}"#));
    assert!(ask(
        r##"{"biomes": "#minecraft:is_forest", "dimension": "minecraft:overworld"}"##
    ));
    assert!(ask(r#"{"dimension": "minecraft:the_nether"}"#));
    assert!(!ask(r#"{"structures": "minecraft:village_plains"}"#));
    assert!(!ask(
        r#"{"biomes": "minecraft:plains", "structures": "minecraft:village_plains"}"#
    ));
    assert!(!ask(r#"{"block": {"blocks": "minecraft:stone"}}"#));
    assert!(!ask(r#"{"light": {"light": 5}}"#));
    assert!(!ask("{}"));
}

/// One player on one block for 20 ticks at a time, with the biome there changed by `/fillbiome`,
/// a grant revoked, the tree replaced: the grants of each tick, with and without the memo.
fn scenario(memo: bool, ignored: u8, blind_to_revokes: bool) -> (Vec<String>, u32) {
    NO_MEMO.set(!memo);
    IGNORED.set(ignored);
    SKIPS.set(0);
    let mut log = Vec::new();
    with_guests(|game| {
        game.set_allow_commands(true);
        column(game);
        game.adv.tree = tree("memo-data-a", &[]);
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_all(|guest| guest.advancements_loaded = true);
        let eval = |game: &mut Game<'_>, what: &str, log: &mut Vec<String>| {
            game.ticks = (game.ticks / 20 + 1) * 20;
            game.as_guest(12, |game| game.tick_advancements()).unwrap();
            let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
            let granted: Vec<String> = guest
                .advancements
                .criteria
                .iter()
                .filter(|(_, m)| !m.is_empty())
                .map(|(id, _)| id.clone())
                .collect();
            log.push(format!("{what}: {granted:?}"));
        };
        fill(game, "minecraft:badlands");
        eval(game, "badlands", &mut log);
        eval(game, "badlands again", &mut log);
        eval(game, "badlands a third time", &mut log);
        fill(game, "minecraft:plains");
        eval(game, "plains", &mut log);
        eval(game, "plains again", &mut log);
        // A revoked grant is asked for again where the player still stands.
        game.lan.as_mut().unwrap().hub.guests.update_all(|guest| {
            // The memo a revoke forgets; a control puts it back, as a memo that did not know of
            // revokes would have it.
            let kept = guest.advancements.memo.clone();
            guest.advancements.revoke("testmod:plains", "c");
            if blind_to_revokes {
                guest.advancements.memo = kept;
            }
        });
        eval(game, "plains, revoked", &mut log);
        fill(game, "minecraft:desert");
        eval(game, "desert", &mut log);
        fill(game, "minecraft:forest");
        eval(game, "forest (a tag)", &mut log);
        // Another tree, as `/reload` makes one: a criterion that is new is asked.
        game.adv.tree = tree(
            "memo-data-b",
            &[("swamp", r#"{"biomes": "minecraft:swamp"}"#)],
        );
        eval(game, "the new tree, still forest", &mut log);
        fill(game, "minecraft:swamp");
        eval(game, "swamp", &mut log);
        eval(game, "swamp again", &mut log);
    });
    NO_MEMO.set(false);
    IGNORED.set(0);
    (log, SKIPS.get())
}

#[test]
#[ignore = "daily"]
fn the_location_memo_changes_no_grant() {
    let (old, none) = scenario(false, 0, false);
    let (new, skipped) = scenario(true, 0, false);
    assert_eq!(none, 0);
    assert_eq!(old, new, "the grants of every tick");
    eprintln!("{new:#?}\nskipped evaluations: {skipped}");
    // The ticks where the answer was found again from the memo: the repeats.
    assert!(skipped >= 3, "{skipped}");
    // And the grants happen where the place changed, not later.
    assert!(new[3].contains("testmod:plains"), "{:?}", new[3]);
    assert!(!new[2].contains("testmod:plains"));
    assert!(
        new[5].contains("testmod:plains"),
        "revoked, granted again: {:?}",
        new[5]
    );
    assert!(new[6].contains("testmod:desert"));
    assert!(
        new[7].contains("testmod:forest"),
        "a biome tag: {:?}",
        new[7]
    );
    assert!(new[9].contains("testmod:swamp"), "{:?}", new[9]);
    assert!(!new[8].contains("testmod:swamp"));
    // Controls: a memo that leaves out the biome, or one that does not know of revokes, misses the
    // changes it is for.
    let (blind, _) = scenario(true, 4, false);
    assert_ne!(old, blind, "a memo that ignores the biome is seen");
    let (blind, _) = scenario(true, 0, true);
    assert_ne!(old, blind, "a memo that does not know of revokes is seen");
}

/// `/reload` with data packs: a pack's advancement is asked at the place the player already
/// stands (the tree is another one), and so is one whose biome tag the pack changed (the same
/// advancement, the tag another). Run in a process of its own, like the other tests that reload.
#[test]
#[ignore = "daily"]
fn a_reload_asks_the_criteria_again() {
    use crate::game::lan_actions::GuestCommand;
    use crate::game::native_entity_refs::tests::run_fresh_cases;
    if run_fresh_cases(
        "game::advancements::location_memo::tests::a_reload_asks_the_criteria_again",
        "HORIZON_LOCATION_RELOAD_CHILD",
        &["reload", "blind"],
    ) {
        return;
    }
    // The control: a memo that does not look at the tree misses the reload.
    let blind = std::env::var("HORIZON_LOCATION_RELOAD_CHILD").as_deref() == Ok("blind");
    IGNORED.set(u8::from(blind));
    let scratch = std::env::current_dir().unwrap().join("reload-data");
    with_guests(|game| {
        game.set_allow_commands(true);
        column(game);
        game.world_root = scratch.join("world");
        let pack = game.world_root.join("datapacks/test");
        let data = pack.join("data/testmod");
        std::fs::create_dir_all(data.join("advancement")).unwrap();
        std::fs::create_dir_all(data.join("tags/worldgen/biome")).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"test","pack_format":121}}"#,
        )
        .unwrap();
        std::fs::write(
            game.world_root.join("datapacks.json"),
            r#"{"enabled":["file/test"],"disabled":[]}"#,
        )
        .unwrap();
        for (name, tag) in [("cold", "#testmod:cold"), ("warm", "#testmod:warm")] {
            std::fs::write(
                data.join(format!("advancement/{name}.json")),
                criterion(&format!(r#"{{"biomes": "{tag}"}}"#)),
            )
            .unwrap();
        }
        let tag = |name: &str, biome: &str| {
            std::fs::write(
                data.join(format!("tags/worldgen/biome/{name}.json")),
                format!(r#"{{"values": ["minecraft:{biome}"]}}"#),
            )
            .unwrap();
        };
        let reload = |game: &mut Game<'_>| {
            assert!(game.run_guest_command(GuestCommand {
                conn: 12,
                command: "reload".into(),
                permission: 4,
                integrated: true,
                quiet: false,
            }));
        };
        let eval = |game: &mut Game<'_>| -> Vec<String> {
            game.ticks = (game.ticks / 20 + 1) * 20;
            game.as_guest(12, |game| game.tick_advancements()).unwrap();
            let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
            guest
                .advancements
                .criteria
                .iter()
                .filter(|(id, m)| id.starts_with("testmod:") && !m.is_empty())
                .map(|(id, _)| id.clone())
                .collect()
        };
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_all(|guest| guest.advancements_loaded = true);
        game.lan.as_mut().unwrap().hub.mods_loaded = true;
        fill(game, "minecraft:plains");
        tag("cold", "plains");
        tag("warm", "desert");
        // Before the reload the pack's advancements are not in the tree.
        assert!(eval(game).is_empty());
        SKIPS.set(0);
        assert!(eval(game).is_empty());
        reload(game);
        // The tree is another one, with the pack's: `cold` holds plains.
        if blind {
            // A memo that does not look at the tree asks the old tree's criteria still: the new
            // `cold` is not asked at the place the memo was found at.
            assert!(eval(game).is_empty(), "the memo without the tree is blind");
            return;
        }
        assert_eq!(eval(game), ["testmod:cold"], "a pack's advancement and tag");
        assert!(eval(game) == ["testmod:cold"]);
        let before = SKIPS.get();
        assert!(eval(game) == ["testmod:cold"]);
        assert!(SKIPS.get() > before, "the memo works after the reload too");
        // The tag alone changes: `warm` now holds plains, and nothing else did.
        tag("warm", "plains");
        reload(game);
        assert_eq!(
            eval(game),
            ["testmod:cold", "testmod:warm"],
            "the same advancement, another tag"
        );
    });
    let _ = std::fs::remove_dir_all(&scratch);
}
