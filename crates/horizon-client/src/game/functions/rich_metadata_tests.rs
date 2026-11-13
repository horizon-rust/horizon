use std::sync::Arc;
use horizon_client::chunk::Chunk;
use horizon_core::{geometry::Vec3,position::ChunkPos};
use super::*;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::motion::PlayerMotion;
fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.game_dir = Some(
        std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join(format!("h146-rich-{:?}", std::thread::current().id())),
    );
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
    game.creative = true;
    let time = game.world.game_time;
    test(&mut game);
    assert_eq!(game.world.game_time, time);
}

#[test]
#[ignore = "requires prepared resources; H146 current rich metadata baseline"]
fn rich_description_admission_reaches_loaded_game_functions() {
    with_game(|game| {
        // The 53 descriptions and vanilla's verdict on each.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/fixtures/rich_description_admission.json");
        let cases: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let dir = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join("rich-world");
        assert!(!dir.exists());
        for (i, case) in cases["cases"].as_array().unwrap().iter().enumerate() {
            let name = case["name"].as_str().unwrap();
            let root = dir.join("datapacks").join(name);
            std::fs::create_dir_all(root.join("data/rich/function")).unwrap();
            std::fs::write(root.join("pack.mcmeta"),serde_json::json!({"pack":{"description":case["description"],"pack_format":121,"min_format":[121,0],"max_format":[121,0]}}).to_string()).unwrap();
            std::fs::write(root.join(format!("data/rich/function/{name}.mcfunction")),format!("scoreboard players set {name} rich {}\n",100+i)).unwrap();
        }
        game.world_root=dir;
        assert!(game.run_command("scoreboard objectives add rich dummy"));
        let mut mismatches=Vec::new();
        for reload in [false,true] {
            game.reload_datapacks(reload);
            for (i, case) in cases["cases"].as_array().unwrap().iter().enumerate() {
                let name=case["name"].as_str().unwrap();let valid=case["observed_valid"].as_bool().unwrap();
                for holder in [name,"#r","#s"] {assert!(game.run_command(&format!("scoreboard players set {holder} rich -99")));}
                let ran=game.run_command(&format!("execute store result score #r rich store success score #s rich run function rich:{name}"));
                let physical=game.score.board.score(name,"rich").unwrap().value;
                let selected=game.fns.enabled.contains(&format!("file/{name}"));
                let available=game.fns.packs.iter().any(|p|p.id==format!("file/{name}"));
                let registered=game.fns.store.functions.contains_key(&format!("rich:{name}"));
                let result=game.score.board.score("#r","rich").unwrap().value;let success=game.score.board.score("#s","rich").unwrap().value;
                println!("{}",serde_json::json!({"phase":reload,"name":name,"expected_valid":valid,"selected":selected,"available":available,"registered":registered,"ran":ran,"result":result,"success":success,"physical":physical}));
                if valid {assert_eq!(physical,100+i as i32,"positive actual loaded function prerequisite {name}");}
                if selected!=valid || available!=valid || registered!=valid || physical!=if valid {100+i as i32}else{-99} {mismatches.push((reload,name.to_owned(),physical));}
            }
        }
        assert!(mismatches.is_empty(),"Java-rejected metadata exposed real functions and mutated sentinels: {mismatches:?}");
    });
}

#[path = "rich_metadata_tests/review_tests.rs"]
mod review_tests;
