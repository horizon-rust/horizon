use super::with_game;
#[test]
#[ignore = "prepared resources; actual review-specific folder pack admission"]
fn review_codec_errors_and_patch_collisions_reach_loaded_game() {
    with_game(|g| {
        let contract = std::path::PathBuf::from(std::env::var_os("H146_REVIEW_CASES").unwrap());
        let cases: serde_json::Value =
            serde_json::from_slice(&std::fs::read(contract).unwrap()).unwrap();
        let dir = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join("review-world");
        assert!(!dir.exists());
        for c in cases.as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let root = dir.join("datapacks").join(name);
            std::fs::create_dir_all(root.join("data/review/function")).unwrap();
            std::fs::write(root.join("pack.mcmeta"), format!(r#"{{"pack":{{"description":{},"pack_format":121,"min_format":[121,0],"max_format":[121,0]}}}}"#, c["description"].as_str().unwrap())).unwrap();
            std::fs::write(
                root.join(format!("data/review/function/{name}.mcfunction")),
                format!("scoreboard players set {name} review 77\nreturn 11\n"),
            )
            .unwrap();
        }
        g.world_root = dir;
        assert!(g.run_command("scoreboard objectives add review dummy"));
        let mut order = None;
        let mut failures = Vec::new();
        for reload in [false, true] {
            g.reload_datapacks(reload);
            println!("selected order {reload}: {:?}", g.fns.enabled);
            if let Some(ref expected) = order {
                assert_eq!(&g.fns.enabled, expected);
            } else {
                order = Some(g.fns.enabled.clone());
            }
            for c in cases.as_array().unwrap() {
                let name = c["name"].as_str().unwrap();
                let valid = c["valid"].as_bool().unwrap();
                for holder in [name, "#r", "#s"] {
                    assert!(g.run_command(&format!("scoreboard players set {holder} review -99")));
                }
                let ran = g.run_command(&format!("execute store result score #r review store success score #s review run function review:{name}"));
                let score = |holder: &str| g.score.board.score(holder, "review").unwrap().value;
                let physical = score(name);
                let result = score("#r");
                let success = score("#s");
                let available = g.fns.packs.iter().any(|p| p.id == format!("file/{name}"));
                let selected = g.fns.enabled.contains(&format!("file/{name}"));
                let loaded = g
                    .fns
                    .store
                    .functions
                    .contains_key(&format!("review:{name}"));
                println!(
                    "{}",
                    serde_json::json!({"name":name,"reload":reload,"valid":valid,"ran":ran,"physical":physical,"result":result,"success":success,"available":available,"selected":selected,"loaded":loaded,"clock":g.world.game_time})
                );
                if (available, selected, loaded, physical, result, success)
                    != if valid {
                        (true, true, true, 77, 11, 1)
                    } else {
                        (false, false, false, -99, 0, 0)
                    }
                {
                    failures.push((name.to_owned(), reload, physical, result, success));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "review metadata admission: {failures:?}"
        );
    });
}
