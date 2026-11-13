//! Native operational-authority contract; not a Java failed-reload claim.
use super::*;

#[test]
#[ignore = "requires prepared resources, isolated homes and cwd"]
fn unavailable_metadata_authority_keeps_the_actual_loaded_store() {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap());
    game.world_root = root.join("context-world");
    let pack = game.world_root.join("datapacks/context");
    let functions = pack.join("data/h146/function");
    std::fs::create_dir_all(&functions).unwrap();
    let meta = pack.join("pack.mcmeta");
    let function = functions.join("keep.mcfunction");
    let first = "scoreboard players set #seen h146_context 11\n";
    std::fs::write(&meta, r#"{"pack":{"description":"initial"}}"#).unwrap();
    std::fs::write(&function, first).unwrap();
    assert!(game.run_command("scoreboard objectives add h146_context dummy"));
    let clock = game.world.game_time;
    game.reload_datapacks(false);
    assert!(game.fns.packs.iter().any(|p| p.id == "file/context"));
    let selected = game.fns.enabled.clone();
    let invoke = |game: &mut Game<'_>, phase: &str, expected| {
        for holder in ["#seen", "#r", "#s"] {
            assert!(game.run_command(&format!("scoreboard players set {holder} h146_context -99")));
        }
        assert!(game.run_command("execute store result score #r h146_context store success score #s h146_context run function h146:keep"));
        let score = |holder| {
            game.score
                .board
                .score(holder, "h146_context")
                .unwrap()
                .value
        };
        let (physical, result, success) = (score("#seen"), score("#r"), score("#s"));
        println!(
            "{}",
            serde_json::json!({"phase":phase,"function":"h146:keep","available":game.fns.packs.iter().map(|p|p.id.as_str()).collect::<Vec<_>>(),"selected":game.fns.enabled,"physical":physical,"result":result,"success":success,"clock":game.world.game_time})
        );
        assert_eq!((physical, result, success), (expected, -99, -99));
    };
    invoke(&mut game, "initial", 11);
    let resources = game.options.resources.clone();
    game.options.resources = root.join("missing-catalog");
    std::fs::write(&meta,r#"{"pack":{"description":{"text":"needs authority","hover_event":{"action":"show_item","id":"minecraft:stone"}}}}"#).unwrap();
    std::fs::write(&function, "scoreboard players set #seen h146_context 99\n").unwrap();
    // An authority error returns before publishing packs, functions or tables.
    game.reload_datapacks(true);
    assert_eq!(game.fns.enabled, selected);
    assert_eq!(
        game.fns.store.functions["h146:keep"]
            .instantiate(None)
            .unwrap(),
        vec![first.trim()]
    );
    invoke(&mut game, "unavailable-keeps-loaded-body", 11);
    // Plain inline dialogs/profiles need no item catalog or a second world.
    std::fs::write(&meta,r#"{"pack":{"description":{"object":"player","player":{},"click_event":{"action":"show_dialog","dialog":{"type":"notice","title":"Inline"}}}}}"#).unwrap();
    std::fs::write(&function, "scoreboard players set #seen h146_context 13\n").unwrap();
    game.reload_datapacks(false);
    invoke(&mut game, "inline-without-catalog", 13);
    assert_eq!(game.world.game_time, clock);
    game.options.resources = resources;
}

#[test]
#[ignore = "requires prepared resources and isolated homes"]
fn unavailable_metadata_cannot_publish_command_pack_selection() {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap());
    game.world_root = root.join("selection-world");
    for name in ["a", "b"] {
        let dir = game.world_root.join("datapacks").join(name);
        std::fs::create_dir_all(dir.join("data/selection/function")).unwrap();
        std::fs::write(
            dir.join("pack.mcmeta"),
            r#"{"pack":{"description":"initial"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("data/selection/function/keep.mcfunction"),
            "return 11\n",
        )
        .unwrap();
    }
    let settings = game.world_root.join("datapacks.json");
    std::fs::write(
        &settings,
        r#"{"enabled":["vanilla","file/a"],"disabled":["file/b"]}"#,
    )
    .unwrap();
    game.reload_datapacks(false);
    let selected = game.selected_packs();
    let bytes = std::fs::read(&settings).unwrap();
    let clock = game.world.game_time;
    game.options.resources = root.join("missing-catalog");
    std::fs::write(game.world_root.join("datapacks/b/pack.mcmeta"), r#"{"pack":{"description":{"text":"authority","hover_event":{"action":"show_item","id":"minecraft:stone"}}}}"#).unwrap();
    for command in ["datapack enable \"file/b\"", "datapack disable vanilla"] {
        assert!(!game.run_command(command), "{command}");
        assert_eq!(game.selected_packs(), selected, "{command}");
        assert_eq!(std::fs::read(&settings).unwrap(), bytes, "{command}");
        assert_eq!(
            game.fns.store.functions["selection:keep"]
                .instantiate(None)
                .unwrap(),
            ["return 11"]
        );
        assert_eq!(game.world.game_time, clock);
    }
}

#[test]
#[ignore = "requires prepared resources and isolated homes"]
fn successful_command_pack_selection_uses_the_existing_order() {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    game.world_root = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
        .join("positive-selection");
    for (name, body) in [("a", "return 11"), ("b", "return 22")] {
        let dir = game.world_root.join("datapacks").join(name);
        std::fs::create_dir_all(dir.join("data/selection/function")).unwrap();
        std::fs::write(
            dir.join("pack.mcmeta"),
            r#"{"pack":{"description":"valid"}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("data/selection/function/pick.mcfunction"), body).unwrap();
    }
    std::fs::write(
        game.world_root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/a"],"disabled":["file/b"]}"#,
    )
    .unwrap();
    game.reload_datapacks(false);
    assert!(game.run_command("scoreboard objectives add selection dummy"));
    assert!(game.run_command("scoreboard players set #r selection -99"));
    assert!(game.run_command("scoreboard players set #s selection -99"));
    let clock = game.world.game_time;
    assert!(game.run_command("execute store result score #r selection store success score #s selection run datapack enable \"file/b\" first"));
    assert_eq!(game.selected_packs(), ["vanilla", "file/b", "file/a"]);
    assert_eq!(
        game.fns.store.functions["selection:pick"]
            .instantiate(None)
            .unwrap(),
        ["return 11"]
    );
    let result = game.score.board.score("#r", "selection").unwrap().value;
    let success = game.score.board.score("#s", "selection").unwrap().value;
    assert_eq!((result, success), (3, 1));
    assert_eq!(game.world.game_time, clock);
    println!(
        "{}",
        serde_json::json!({"phase":"valid-command-selection","selected":game.selected_packs(),"result":result,"success":success,"clock":clock})
    );
}
