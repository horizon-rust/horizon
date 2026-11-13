use super::*;
use std::io::Write;

#[test]
fn strict_repository_covers_folders_zips_features_order_and_snapshot_reload() {
    let root = std::env::temp_dir().join(format!(
        "h146-discovery-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let world = root.join("world");
    let resources = root.join("resources");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(
        resources.join("command-registries.json"),
        r#"{"by_registry":{"minecraft:item":{"ids":["minecraft:stone"]}}}"#,
    )
    .unwrap();
    let good = r#"{"pack":{"description":{"text":"valid","hover_event":{"action":"show_item","id":"minecraft:stone"}}}}"#;
    let bad = r#"{"pack":{"description":{"text":"partial","bold":1}}}"#;
    let write = |path: &Path, meta: &str, body: &str| {
        std::fs::create_dir_all(path.join("data/test/function")).unwrap();
        std::fs::write(path.join("pack.mcmeta"), meta).unwrap();
        std::fs::write(path.join("data/test/function/probe.mcfunction"), body).unwrap();
    };
    write(&world.join("datapacks/a"), good, "return 11");
    write(&world.join("datapacks/bad"), bad, "return 99");
    write(
        &resources.join("data/minecraft/datapacks/feature_valid"),
        good,
        "return 33",
    );
    write(
        &resources.join("data/minecraft/datapacks/feature_bad"),
        bad,
        "return 99",
    );
    for (name, meta, body) in [
        ("z.zip", good, "return 22"),
        ("bad.zip", bad, "return 99"),
        ("missing.zip", "{}", "return 99"),
    ] {
        let file = std::fs::File::create(world.join("datapacks").join(name)).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (entry, bytes) in [
            ("pack.mcmeta", meta),
            ("data/test/function/probe.mcfunction", body),
        ] {
            zip.start_file(entry, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    let packs = discover_with_features(&world, &resources).unwrap();
    assert_eq!(
        packs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["file/a", "file/z.zip", "feature_valid"]
    );
    for (order, expected) in [
        (["file/z.zip", "file/a"], "return 11"),
        (["file/a", "file/z.zip"], "return 22"),
    ] {
        std::fs::write(
            world.join("datapacks.json"),
            serde_json::json!({"enabled":order,"disabled":["feature_valid"]}).to_string(),
        )
        .unwrap();
        let ids = enabled_ids(&world, &packs, &[]);
        assert_eq!(ids, order);
        let refs = ids
            .iter()
            .map(|id| packs.iter().find(|p| &p.id == id).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            DataStore::load(&refs).functions["test:probe"]
                .instantiate(None)
                .unwrap(),
            [expected]
        );
    }
    let active = DataStore::load(&[&packs[0]]);
    std::fs::write(world.join("datapacks/a/pack.mcmeta"), bad).unwrap();
    let reloaded = discover_with_features(&world, &resources).unwrap();
    assert_eq!(
        reloaded.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["file/z.zip", "feature_valid"]
    );
    assert_eq!(
        active.functions["test:probe"].instantiate(None).unwrap(),
        ["return 11"]
    );
    let error = discover_with_features(&world, &root.join("unavailable"))
        .err()
        .unwrap();
    assert_eq!(
        error.reason,
        horizon_command::component::json::Error::Unavailable
    );
    assert_eq!(
        active.functions["test:probe"].instantiate(None).unwrap(),
        ["return 11"]
    );
    let screen = screen_packs(&world.join("datapacks"), &resources).unwrap();
    assert_eq!(
        screen.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["file/z.zip", "vanilla", "feature_valid"]
    );
}
