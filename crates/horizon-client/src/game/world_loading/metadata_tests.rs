use super::*;

#[test]
#[ignore = "requires prepared resources and isolated homes"]
fn pre_world_metadata_errors_propagate_without_empty_worldgen_fallback() {
    let game = crate::game::native_machines::tests::server_game();
    let root = PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join("pre-world");
    let resources = root.join("missing-catalog");
    let pack = root.join("datapacks/p");
    std::fs::create_dir_all(pack.join("data/test/worldgen/feature")).unwrap();
    let meta = pack.join("pack.mcmeta");
    let body = r#"{"type":"minecraft:no_op","config":{}}"#;
    std::fs::write(pack.join("data/test/worldgen/feature/probe.json"), body).unwrap();
    let biomes = game.res.biomes();
    for (label, description, admitted) in [
        (
            "inline",
            r#"{"text":"ok","click_event":{"action":"show_dialog","dialog":{"type":"notice","title":"Inline"}}}"#,
            true,
        ),
        ("partial-invalid", r#"{"text":"bad","bold":1}"#, false),
    ] {
        std::fs::write(
            &meta,
            format!(r#"{{"pack":{{"description":{description}}}}}"#),
        )
        .unwrap();
        let (overrides, _) = worldgen_overrides_of(&root, &resources, biomes, &[]).unwrap();
        assert_eq!(
            overrides
                .entries("features")
                .any(|(id, _)| id == "test:probe"),
            admitted,
            "{label}"
        );
        let created =
            creation_worldgen_overrides(Some(&root), &resources, &["file/p".into()], biomes)
                .unwrap();
        assert_eq!(
            created
                .entries("features")
                .any(|(id, _)| id == "test:probe"),
            admitted,
            "{label}"
        );
        assert!(Arc::ptr_eq(
            &packed_biomes(&root, &resources, biomes, &[]).unwrap(),
            biomes
        ));
        assert_eq!(
            level_shapes(&root, biomes, &resources).unwrap(),
            horizon_client::chunk::LevelShapes::VANILLA
        );
    }
    std::fs::write(&meta, r#"{"pack":{"description":"first"}}"#).unwrap();
    let second = root.join("datapacks/q");
    std::fs::create_dir_all(second.join("data/test/worldgen/feature")).unwrap();
    std::fs::write(
        second.join("pack.mcmeta"),
        r#"{"pack":{"description":"second"}}"#,
    )
    .unwrap();
    let other = r#"{"type":"minecraft:no_op","config":{},"witness":2}"#;
    std::fs::write(second.join("data/test/worldgen/feature/probe.json"), other).unwrap();
    for (order, expected) in [(["file/q", "file/p"], body), (["file/p", "file/q"], other)] {
        std::fs::write(
            root.join("datapacks.json"),
            serde_json::json!({"enabled":order,"disabled":[]}).to_string(),
        )
        .unwrap();
        let (overrides, _) = worldgen_overrides_of(&root, &resources, biomes, &[]).unwrap();
        assert_eq!(
            overrides
                .entries("features")
                .find(|(id, _)| *id == "test:probe")
                .unwrap()
                .1,
            expected
        );
    }
    std::fs::write(&meta, r#"{"pack":{"description":{"text":"needs authority","hover_event":{"action":"show_item","id":"minecraft:stone"}}}}"#).unwrap();
    assert!(matches!(
        worldgen_overrides_of(&root, &resources, biomes, &[]),
        Err(horizon_client::datapack::metadata::AdmissionError {
            reason: horizon_command::component::json::Error::Unavailable,
            ..
        })
    ));
    assert!(
        creation_worldgen_overrides(Some(&root), &resources, &["file/p".into()], biomes).is_err()
    );
    assert!(packed_biomes(&root, &resources, biomes, &[]).is_err());
    assert!(level_shapes(&root, biomes, &resources).is_err());
    let mut options = game.options.clone();
    options.resources = resources;
    let error = make_streaming(
        &game.res,
        &game.pack,
        &options,
        &game.settings,
        &root,
        Dim::Overworld,
        horizon_client::chunk::LevelShapes::VANILLA.get("minecraft:overworld"),
        0,
        &[],
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("metadata codec unavailable"));
}
