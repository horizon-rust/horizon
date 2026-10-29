use horizon_port_pack::{
    import::{Pack, PackFile},
    inventory,
    modrinth::Cache,
    pipeline, report,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "horizon-pixelmon-archive-{label}-{}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn pixelmon_archive_needs_deep_api_without_network_metadata() {
    let work = TestDir::new("mod");
    let jar = work.0.join("opaque.jar");
    let mut zip = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
    for (name, bytes) in [
        (
            "META-INF/neoforge.mods.toml",
            "license = \"All rights reserved\"\n[[mods]]\nmodId = \"pixelmon\"\ndisplayName = \"Original fixture\"\nversion = \"9.4.2\"\n",
        ),
        ("example/Creature.class", "synthetic placeholder"),
        (
            "assets/pixelmon/models/item/example.json",
            r#"{"parent":"minecraft:item/generated"}"#,
        ),
        (
            "data/pixelmon/species/001_example.json",
            r#"{"name":"Example","forms":[]}"#,
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let pack = Pack {
        files: vec![PackFile {
            path: "mods/opaque.jar".into(),
            jar: Some(jar),
            ..PackFile::default()
        }],
        ..Pack::default()
    };
    let records = inventory::build(&pack, &Cache::default(), &work.0);
    let record = &records[0];
    assert_eq!(record.key, "pixelmon");
    assert_eq!(record.class, "content");
    assert_eq!(record.tier, "deep");
    assert_eq!(record.confidence, "known");
    assert_eq!(record.info.as_ref().unwrap().census.classes, 1);
    assert!(!pipeline::portable(record));
    let report = report::build(&records, &work.0);
    assert_eq!(report[0].status, report::Status::NotYet);
    assert!(report[0].reason.contains("entities"));
}

#[test]
fn pack_kind_wins_over_pixelmon_name_without_network_metadata() {
    let work = TestDir::new("pack-kinds");
    let jars = work.0.join("jars");
    fs::create_dir(&jars).unwrap();
    let mut zip = zip::ZipWriter::new(fs::File::create(jars.join("pixelmon.zip")).unwrap());
    zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"{"pack":{"pack_format":48,"description":"Original fixture"}}"#)
        .unwrap();
    zip.finish().unwrap();

    for (kind, class, tier, portable) in [
        ("datapacks", "content", "data", true),
        ("resourcepacks", "other", "data", false),
        ("shaderpacks", "other", "none", false),
    ] {
        let pack = Pack {
            files: vec![PackFile {
                path: format!("{kind}/pixelmon.zip"),
                name: Some("Pixelmon".into()),
                ..PackFile::default()
            }],
            ..Pack::default()
        };
        let records = inventory::build(&pack, &Cache::default(), &work.0);
        let record = &records[0];
        assert_eq!(record.key, "pixelmon");
        assert_eq!(
            (
                record.class.as_str(),
                record.tier.as_str(),
                record.confidence.as_str()
            ),
            (class, tier, "kind"),
            "{kind}"
        );
        assert_eq!(pipeline::portable(record), portable, "{kind}");
    }
}

fn collision_archive(path: &Path, id: &str, is_mod: bool) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let mut entries = vec![(
        "pack.mcmeta".to_owned(),
        r#"{"pack":{"pack_format":48,"description":"Original fixture"}}"#.to_owned(),
    )];
    if is_mod {
        entries.extend([
            (
                "META-INF/neoforge.mods.toml".to_owned(),
                format!("license = \"MIT\"\n[[mods]]\nmodId = \"{id}\"\ndisplayName = \"Original fixture\"\nversion = \"1\"\n"),
            ),
            ("example/Creature.class".to_owned(), "synthetic placeholder".to_owned()),
        ]);
    }
    let recipe = if is_mod { "from_mod" } else { "from_datapack" };
    entries.push((
        format!("data/{id}/recipe/{recipe}.json"),
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"item":"minecraft:stone"}],"result":{"id":"minecraft:stone","count":1}}"#.to_owned(),
    ));
    for (name, text) in entries {
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn pack_entry(path: &str, archive: &Path, hashes: (&str, &str)) -> serde_json::Value {
    let bytes = fs::read(archive).unwrap();
    assert_eq!(horizon_port_pack::import::sha1_hex(&bytes), hashes.0);
    serde_json::json!({
        "path": path,
        "hashes": { "sha1": hashes.0, "sha512": hashes.1 },
        "downloads": [format!("https://cdn.modrinth.com/data/original-fixture/{path}")],
        "fileSize": bytes.len(),
        "env": { "client": "required", "server": "required" }
    })
}

fn conditional_archive(
    path: &Path,
    id: &str,
    condition: Option<&str>,
    dependency: Option<(&str, bool)>,
    deep: bool,
    namespace: Option<&str>,
) {
    let mut files = vec![(
        "pack.mcmeta".to_owned(),
        r#"{"pack":{"pack_format":48,"description":"Original fixture"}}"#.to_owned(),
    )];
    if path.extension().is_some_and(|e| e == "jar") {
        let mut metadata = format!(
            "license = \"MIT\"\n[[mods]]\nmodId = \"{id}\"\ndisplayName = \"Original fixture\"\nversion = \"1\"\n"
        );
        if let Some((other, required)) = dependency {
            let kind = if required { "required" } else { "optional" };
            metadata.push_str(&format!(
                "[[dependencies.{id}]]\nmodId = \"{other}\"\ntype = \"{kind}\"\n"
            ));
        }
        files.push(("META-INF/neoforge.mods.toml".into(), metadata));
    }
    if deep {
        files.push((
            "original/Fixture.class".into(),
            "original placeholder without bytecode".into(),
        ));
    }
    files.push((format!("data/{id}/recipe/example.json"), r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"item":"minecraft:stone"}],"result":{"id":"minecraft:stone","count":1}}"#.into()));
    if let Some(other) = condition {
        let condition = if let Some(other) = other.strip_prefix('!') {
            serde_json::json!({"type": "neoforge:not", "value": {"type": "neoforge:mod_loaded", "modid": other}})
        } else {
            serde_json::json!({"type": "neoforge:mod_loaded", "modid": other})
        };
        files.push((format!("data/{id}/lithostitched/worldgen_modifier/optional.json"), serde_json::json!({
            "type": "lithostitched:add_template_pool_elements", "template_pools": "minecraft:village/plains/houses",
            "elements": [{"weight": 1, "element": {"element_type": "minecraft:empty_pool_element"}}],
                "neoforge:conditions": [condition]
        }).to_string()));
    }
    if let Some(other) = namespace {
        files.push((
            format!("data/{id}/tags/block/claims.json"),
            serde_json::json!({"values": [format!("{other}:marker")]}).to_string(),
        ));
        files.push((
            format!("data/{other}/worldgen/configured_feature/marker.json"),
            r#"{"type":"minecraft:no_op","config":{}}"#.into(),
        ));
    }
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in files {
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
        zip.write_all(bytes.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn conditional_addition(work: &Path, folder: &str, id: &str) -> PathBuf {
    work.join("mods").join(folder).join(format!(
        "carry/data/{id}/worldgen/template_pool/horizon_added/optional.json"
    ))
}

#[test]
fn ordinary_port_refreshes_deep_conditions_without_an_automatic_carry() {
    for negative in [false, true] {
        let work = TestDir::new(&format!("deep-optional-{negative}"));
        let helper = work.0.join("helper.jar");
        let native = work.0.join("native.jar");
        conditional_archive(
            &helper,
            "pixelmon",
            Some(if negative { "!create" } else { "create" }),
            Some(("create", false)),
            true,
            None,
        );
        conditional_archive(&native, "create", None, None, true, None);
        let mut pack = Pack {
            files: vec![
                PackFile {
                    path: "mods/helper.jar".into(),
                    jar: Some(helper),
                    ..PackFile::default()
                },
                PackFile {
                    path: "mods/native.jar".into(),
                    jar: Some(native),
                    ..PackFile::default()
                },
            ],
            ..Pack::default()
        };
        pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
        let records = pipeline::inventory(&work.0, true, &|_| {}).unwrap();
        assert!(records.iter().all(|r| !pipeline::portable(r)));
        pipeline::port(&work.0, None, None, &|_| {}).unwrap();
        assert!(pipeline::carried(&work.0).unwrap().is_empty());
        pipeline::carry(
            &work.0,
            None,
            &["pixelmon".into(), "create".into()],
            &|_| {},
        )
        .unwrap();
        let addition = conditional_addition(&work.0, "pixelmon", "pixelmon");
        assert_eq!(addition.is_file(), !negative);
        let owner = work.0.join("mods/pixelmon");
        let unchanged = ["horizon-mod.toml", "data/pixelmon/recipe/example.json"]
            .map(|name| (name, fs::read(owner.join("carry").join(name)).unwrap()));
        let dependency = pack.files.pop().unwrap();
        for present in [false, true, false, true] {
            pack.files.truncate(1);
            if present {
                pack.files.push(PackFile {
                    path: dependency.path.clone(),
                    jar: dependency.jar.clone(),
                    ..PackFile::default()
                });
            }
            pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
            let evidence = owner.join("harness/result.json");
            fs::create_dir_all(evidence.parent().unwrap()).unwrap();
            pipeline::write_json(
                &evidence,
                &serde_json::json!({"passed": true, "checks": {"recipes": {"passed": 1, "failed": 0}}}),
            )
            .unwrap();
            pipeline::port(&work.0, None, None, &|_| {}).unwrap();
            assert_eq!(addition.is_file(), present != negative);
            assert!(
                !evidence.exists(),
                "selection changes invalidate old checks"
            );
            let (on, off) = pipeline::loadable(&pipeline::carried(&work.0).unwrap());
            assert!(off.is_empty());
            assert_eq!(on.len(), if present { 2 } else { 1 });
            for (name, bytes) in &unchanged {
                assert_eq!(&fs::read(owner.join("carry").join(name)).unwrap(), bytes);
            }
            assert!(
                pipeline::report(&work.0, &|_| {})
                    .unwrap()
                    .iter()
                    .all(|row| { row.status == report::Status::Partial && row.tier == "deep" })
            );
        }
    }
}

#[test]
fn legacy_worldgen_requires_explicit_carry_instead_of_unproven_archive_backfill() {
    for cached in [false, true] {
        for (version, weight) in [("1", 1), ("2", 7), ("1", 7)] {
            let work = TestDir::new(&format!("legacy-inputs-{cached}-{version}-{weight}"));
            let helper = work.0.join("helper.jar");
            let native = work.0.join("pixelmon.jar");
            conditional_archive(&native, "pixelmon", None, None, true, None);
            let source = serde_json::json!({
                "type": "lithostitched:add_template_pool_elements",
                "template_pools": "minecraft:village/plains/houses",
                "elements": [{"weight": 1, "element": {"element_type": "minecraft:empty_pool_element"}}],
                "neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "pixelmon"}]
            });
            let recipe = serde_json::json!({"type": "minecraft:crafting_shapeless", "ingredients": ["minecraft:stone"], "result": {"id": "minecraft:stone", "count": 1}});
            let modifier = "data/review_helper/lithostitched/worldgen_modifier/optional.json";
            let write_helper = |version: &str, weight: u32| {
                let mut source = source.clone();
                source["elements"][0]["weight"] = weight.into();
                let mut recipe = recipe.clone();
                recipe["result"]["count"] = weight.into();
                let mut zip = zip::ZipWriter::new(fs::File::create(&helper).unwrap());
                for (name, text) in [
                    (
                        "META-INF/neoforge.mods.toml",
                        format!(
                            "license = \"MIT\"\n[[mods]]\nmodId = \"review_helper\"\nversion = \"{version}\"\n"
                        ),
                    ),
                    (modifier, source.to_string()),
                    ("data/review_helper/recipe/example.json", recipe.to_string()),
                ] {
                    zip.start_file(name, zip::write::SimpleFileOptions::default())
                        .unwrap();
                    zip.write_all(text.as_bytes()).unwrap();
                }
                zip.finish().unwrap();
            };
            write_helper("1", 1);
            let pack = Pack {
                files: vec![
                    PackFile {
                        path: "mods/pixelmon.jar".into(),
                        jar: Some(native),
                        ..PackFile::default()
                    },
                    PackFile {
                        path: "mods/helper.jar".into(),
                        jar: Some(helper.clone()),
                        ..PackFile::default()
                    },
                ],
                ..Pack::default()
            };
            pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
            pipeline::inventory(&work.0, true, &|_| {}).unwrap();
            pipeline::carry(&work.0, None, &["pixelmon".into()], &|_| {}).unwrap();
            let owner = work.0.join("mods/review_helper");
            let inside = owner.join("carry");
            if cached {
                pipeline::carry(&work.0, None, &["review_helper".into()], &|_| {}).unwrap();
            } else {
                // Original legacy output has no record of the successful archive bytes.
                fs::create_dir_all(&inside).unwrap();
                fs::write(inside.join("horizon-mod.toml"), "id = \"review_helper\"\nname = \"Original legacy fixture\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n").unwrap();
                for (path, value) in [
                    (modifier, source.clone()),
                    ("data/review_helper/recipe/example.json", recipe.clone()),
                ] {
                    fs::create_dir_all(inside.join(path).parent().unwrap()).unwrap();
                    pipeline::write_json(&inside.join(path), &value).unwrap();
                }
                let addition = conditional_addition(&work.0, "review_helper", "review_helper");
                fs::create_dir_all(addition.parent().unwrap()).unwrap();
                pipeline::write_json(&addition, &serde_json::json!({"elements": source["elements"], "horizon:add_to": ["minecraft:village/plains/houses"]})).unwrap();
                pipeline::write_json(
                    &owner.join("carry-report.json"),
                    &serde_json::json!({"worldgen_types": {}}),
                )
                .unwrap();
                pipeline::write_json(&owner.join("source.json"), &"mods/helper.jar").unwrap();
            }
            let paths = [
                "horizon-mod.toml",
                "data/review_helper/recipe/example.json",
                modifier,
                "data/review_helper/worldgen/template_pool/horizon_added/optional.json",
            ];
            let before = paths.map(|path| fs::read(inside.join(path)).unwrap());
            write_helper(version, weight);
            pipeline::inventory(&work.0, true, &|_| {}).unwrap();
            for _ in 0..2 {
                let result = pipeline::carry(&work.0, None, &["pixelmon".into()], &|_| {});
                if cached {
                    result.unwrap();
                } else {
                    let error = result.unwrap_err();
                    assert!(error.contains("explicitly carry"), "{error}");
                    assert!(!owner.join("worldgen-source.json").exists());
                }
                for (path, bytes) in paths.iter().zip(&before) {
                    assert_eq!(&fs::read(inside.join(path)).unwrap(), bytes);
                }
            }
            pipeline::carry(&work.0, None, &["review_helper".into()], &|_| {}).unwrap();
            let addition: serde_json::Value = pipeline::read_json(&conditional_addition(
                &work.0,
                "review_helper",
                "review_helper",
            ))
            .unwrap();
            let recipe: serde_json::Value =
                pipeline::read_json(&inside.join("data/review_helper/recipe/example.json"))
                    .unwrap();
            assert_eq!(addition["elements"][0]["weight"], weight);
            assert_eq!(recipe["result"]["count"], weight);
            assert!(owner.join("worldgen-source.json").is_file());
        }
    }
}

fn corrupt_original_recipe(path: &Path, id: &str) {
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let offset = archive
        .by_name(&format!("data/{id}/recipe/example.json"))
        .unwrap()
        .data_start() as usize;
    drop(archive);
    let mut data = fs::read(path).unwrap();
    data[offset + 1] ^= 1;
    fs::write(path, data).unwrap();
}

#[test]
fn genuine_mod_selection_does_not_depend_on_a_pack_folder_prefix() {
    for id in ["a_review_pack", "create", "pixelmon", "z_review_pack"] {
        let work = TestDir::new(&format!("selection-{id}"));
        let native = work.0.join("opaque.jar");
        let data = work.0.join(format!("{id}.zip"));
        conditional_archive(&native, id, None, None, true, None);
        conditional_archive(&data, id, None, None, false, None);
        let mut pack = Pack {
            files: vec![PackFile {
                path: "mods/opaque.jar".into(),
                jar: Some(native),
                ..PackFile::default()
            }],
            ..Pack::default()
        };
        pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
        pipeline::inventory(&work.0, true, &|_| {}).unwrap();
        pipeline::carry(&work.0, None, &[id.into()], &|_| {}).unwrap();
        pack.files.push(PackFile {
            path: format!("datapacks/{id}.zip"),
            jar: Some(data),
            ..PackFile::default()
        });
        pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
        for _ in 0..3 {
            pipeline::port(&work.0, None, None, &|_| {}).unwrap();
            let owned = pipeline::carried(&work.0).unwrap();
            let (on, off) = pipeline::loadable(&owned);
            assert_eq!(
                on,
                [work.0.join("mods").join(id).join("carry")],
                "{id}: {off:?}"
            );
            assert!(
                work.0
                    .join(format!(
                        "mods/datapacks@{id}/carry/data/{id}/recipe/example.json"
                    ))
                    .is_file()
            );
        }
    }
}

#[test]
fn optional_forward_backward_and_mutual_conditions_use_the_final_selected_mods() {
    for (mode, required) in [
        ("forward", false),
        ("backward", false),
        ("mutual", false),
        ("negative-mutual", false),
        ("required", true),
    ] {
        let work = TestDir::new(mode);
        let alpha = work.0.join("a.jar");
        let zeta = work.0.join("z.jar");
        conditional_archive(
            &alpha,
            "review_alpha",
            (mode != "backward").then_some(if mode == "negative-mutual" {
                "!review_zeta"
            } else {
                "review_zeta"
            }),
            Some(("review_zeta", required)),
            false,
            None,
        );
        conditional_archive(
            &zeta,
            "review_zeta",
            matches!(mode, "backward" | "mutual" | "negative-mutual").then_some(
                if mode == "negative-mutual" {
                    "!review_alpha"
                } else {
                    "review_alpha"
                },
            ),
            matches!(mode, "mutual" | "negative-mutual").then_some(("review_alpha", false)),
            false,
            None,
        );
        let pack = Pack {
            files: vec![
                PackFile {
                    path: "mods/z.jar".into(),
                    jar: Some(zeta),
                    ..PackFile::default()
                },
                PackFile {
                    path: "mods/a.jar".into(),
                    jar: Some(alpha),
                    ..PackFile::default()
                },
            ],
            ..Pack::default()
        };
        pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
        for run in 0..3 {
            pipeline::port(&work.0, None, None, &|_| {}).unwrap();
            let (selected, _) = pipeline::loadable(&pipeline::carried(&work.0).unwrap());
            assert_eq!(selected.len(), 2);
            assert_eq!(
                conditional_addition(&work.0, "review_alpha", "review_alpha").is_file(),
                !matches!(mode, "backward" | "negative-mutual"),
                "{mode}: run {run}"
            );
            assert_eq!(
                conditional_addition(&work.0, "review_zeta", "review_zeta").is_file(),
                matches!(mode, "backward" | "mutual"),
                "{mode}: run {run}"
            );
        }
    }
}

#[test]
fn failed_replacement_conditions_follow_the_surviving_manifest() {
    for id in ["review_native", "pixelmon"] {
        let work = TestDir::new(&format!("failed-replacement-{id}"));
        let native = work.0.join("native.jar");
        let helper = work.0.join("helper.zip");
        conditional_archive(&native, id, None, None, id == "pixelmon", None);
        conditional_archive(&helper, "helper", Some(id), None, false, None);
        let pack = Pack {
            files: vec![
                PackFile {
                    path: "mods/native.jar".into(),
                    jar: Some(native.clone()),
                    ..PackFile::default()
                },
                PackFile {
                    path: "datapacks/helper.zip".into(),
                    jar: Some(helper),
                    ..PackFile::default()
                },
            ],
            ..Pack::default()
        };
        pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
        pipeline::inventory(&work.0, true, &|_| {}).unwrap();
        pipeline::carry(&work.0, None, &[id.into(), "helper".into()], &|_| {}).unwrap();
        let addition = conditional_addition(&work.0, "datapacks@helper", "helper");
        assert!(addition.is_file());
        let good = fs::read(&native).unwrap();
        fs::remove_file(work.0.join(format!("mods/{id}/worldgen-source.json"))).unwrap();
        fs::write(&native, b"original invalid ZIP control").unwrap();
        pipeline::carry(&work.0, None, &[id.into(), "helper".into()], &|_| {}).unwrap();
        assert!(
            work.0
                .join(format!("mods/{id}/carry/horizon-mod.toml"))
                .is_file()
        );
        assert!(
            addition.is_file(),
            "failure before replacement preserves the usable Mod"
        );
        fs::write(&native, good).unwrap();
        corrupt_original_recipe(&native, id);
        for _ in 0..2 {
            if id == "pixelmon" {
                pipeline::carry(&work.0, None, &[id.into(), "helper".into()], &|_| {}).unwrap();
            } else {
                pipeline::port(&work.0, None, None, &|_| {}).unwrap();
            }
            assert!(
                !work
                    .0
                    .join(format!("mods/{id}/carry/horizon-mod.toml"))
                    .exists()
            );
            assert!(
                !addition.exists(),
                "a destroyed previous output is not a loaded Mod"
            );
        }
    }
}

#[test]
fn unchanged_helper_conditions_refresh_when_only_the_mod_is_replaced() {
    let work = TestDir::new("unchanged-helper");
    let native = work.0.join("native.jar");
    let helper = work.0.join("helper.zip");
    conditional_archive(&native, "review_native", None, None, false, None);
    conditional_archive(&helper, "helper", Some("review_native"), None, false, None);
    let pack = Pack {
        files: vec![
            PackFile {
                path: "mods/native.jar".into(),
                jar: Some(native.clone()),
                ..PackFile::default()
            },
            PackFile {
                path: "datapacks/helper.zip".into(),
                jar: Some(helper),
                ..PackFile::default()
            },
        ],
        ..Pack::default()
    };
    pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
    pipeline::port(&work.0, None, None, &|_| {}).unwrap();
    let addition = conditional_addition(&work.0, "datapacks@helper", "helper");
    assert!(addition.is_file());
    let evidence = work.0.join("mods/datapacks@helper/harness/result.json");
    fs::create_dir(evidence.parent().unwrap()).unwrap();
    pipeline::write_json(
        &evidence,
        &serde_json::json!({"passed": true, "checks": {"recipes": {"passed": 1, "failed": 0}}}),
    )
    .unwrap();
    corrupt_original_recipe(&native, "review_native");
    pipeline::carry(&work.0, None, &["review_native".into()], &|_| {}).unwrap();
    assert!(!addition.exists());
    assert!(
        !evidence.exists(),
        "changing a conditional output invalidates its previous checks"
    );
}

#[test]
fn namespace_losers_do_not_satisfy_mod_conditions() {
    let work = TestDir::new("namespace-condition");
    let blocker = work.0.join("blocker.jar");
    let native = work.0.join("zeta.jar");
    let helper = work.0.join("helper.zip");
    conditional_archive(&blocker, "pixelmon", None, None, true, Some("review_zeta"));
    conditional_archive(
        &native,
        "review_zeta",
        Some("review_zeta"),
        None,
        false,
        None,
    );
    conditional_archive(&helper, "helper", Some("review_zeta"), None, false, None);
    let mut pack = Pack {
        files: vec![
            PackFile {
                path: "mods/blocker.jar".into(),
                jar: Some(blocker),
                ..PackFile::default()
            },
            PackFile {
                path: "datapacks/helper.zip".into(),
                jar: Some(helper),
                ..PackFile::default()
            },
        ],
        ..Pack::default()
    };
    pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
    pipeline::inventory(&work.0, true, &|_| {}).unwrap();
    pipeline::carry(&work.0, None, &["pixelmon".into()], &|_| {}).unwrap();
    pack.files.push(PackFile {
        path: "mods/zeta.jar".into(),
        jar: Some(native),
        ..PackFile::default()
    });
    pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
    let addition = conditional_addition(&work.0, "datapacks@helper", "helper");
    for _ in 0..3 {
        pipeline::port(&work.0, None, None, &|_| {}).unwrap();
        let (selected, off) = pipeline::loadable(&pipeline::carried(&work.0).unwrap());
        assert!(
            off.iter()
                .any(|message| message.starts_with("review_zeta not loaded"))
        );
        assert!(!selected.contains(&work.0.join("mods/review_zeta/carry")));
        assert!(
            !addition.exists(),
            "only a selected Mod can satisfy a Mod condition"
        );
    }
    pack.files.remove(0);
    pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
    pipeline::port(&work.0, None, None, &|_| {}).unwrap();
    assert!(
        addition.is_file(),
        "removing the namespace owner makes the successful Mod selectable"
    );
}

#[test]
fn automatic_port_keeps_same_named_datapacks_separate_from_deep_mods() {
    let root = TestDir::new("automatic-port");
    // Hashes of the original stored ZIP fixtures, guarded by their SHA-1 above.
    for (id, data_hashes, mod_hashes) in [
        (
            "pixelmon",
            (
                "979ea057c900bcd8ab2847def272f0767bd0ee33",
                "213a0b466f46d3c9dbb907ae0f6af4d59b72eb82763de0d2152e46584eae7ced47d0358770e48e04d25dd78c349f183da250a3c6acd6c1162e2c4eab87297ea3",
            ),
            (
                "fa5d3dd2de4d081dc516356346507a273081bda1",
                "61bfc388329602fe8141075732109c78ff0abe72445044eedf3260b1c36033d73cec97a47a48ef15cb0d056df2e18b62589243e88cfa6214fb8821e462146871",
            ),
        ),
        (
            "create",
            (
                "b70bd94ba954878ca8fc294d038c8f2baed319fa",
                "94bbddf7f60f7a6349e84f91fb27ffa3e06ca588bb02b0aacf02fb5560270fde0b18d674e79b7a962d5fcfa43927ba20adc1c8cdbdc60a437c1e31924f3b46ee",
            ),
            (
                "97142d906189382e4528fcefa73aa7721916007e",
                "9f72113ebb2e106a2d1ea07db71b5b58c0f64980a1da8f51b1471b3684f3e8ddcd108424f8136f602e94b50617f28d05be4069f726e2c6ebefad5a254833611f",
            ),
        ),
    ] {
        for mod_first in [false, true] {
            let work = root.0.join(format!("{id}-{mod_first}"));
            let jars = work.join("jars");
            fs::create_dir_all(&jars).unwrap();
            let data = jars.join(format!("{id}.zip"));
            let jar = jars.join(format!("opaque-{id}.jar"));
            collision_archive(&data, id, false);
            collision_archive(&jar, id, true);
            let mut files = vec![
                pack_entry(&format!("datapacks/{id}.zip"), &data, data_hashes),
                pack_entry(&format!("mods/opaque-{id}.jar"), &jar, mod_hashes),
            ];
            if mod_first {
                files.reverse();
            }
            let index = serde_json::json!({
                "formatVersion": 1,
                "game": "minecraft",
                "versionId": "original-fixture",
                "name": "Original collision fixture",
                "dependencies": { "minecraft": "1.21.1", "neoforge": "21.1.248" },
                "files": files
            });
            let pack = work.join("fixture.mrpack");
            let mut zip = zip::ZipWriter::new(fs::File::create(&pack).unwrap());
            zip.start_file(
                "modrinth.index.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(index.to_string().as_bytes()).unwrap();
            zip.finish().unwrap();
            // Keep loader lookup offline too, with an original empty data archive.
            let loader = work.join("loader");
            fs::create_dir(&loader).unwrap();
            zip::ZipWriter::new(
                fs::File::create(loader.join("neoforge-21.1.248-universal.jar")).unwrap(),
            )
            .finish()
            .unwrap();

            let output = Command::new(env!("CARGO_BIN_EXE_horizon-port-pack"))
                .arg("port")
                .arg(&pack)
                .arg(&work)
                .arg("--offline")
                .output()
                .unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(
                output.status.success(),
                "{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                stdout.contains("1 mods carried over"),
                "{id}, {mod_first}: {stdout}"
            );
            let records: Vec<inventory::Record> =
                pipeline::read_json(&work.join("inventory.json")).unwrap();
            assert_eq!(records.len(), 2);
            assert!(
                records
                    .iter()
                    .all(|r| r.key == id && r.info.as_ref().unwrap().mod_id == id)
            );
            let data_record = records
                .iter()
                .find(|r| r.path.starts_with("datapacks/"))
                .unwrap();
            let mod_record = records
                .iter()
                .find(|r| r.path.starts_with("mods/"))
                .unwrap();
            assert_eq!(
                (
                    &*data_record.class,
                    &*data_record.tier,
                    &*data_record.confidence
                ),
                ("content", "data", "kind")
            );
            assert_eq!(data_record.info.as_ref().unwrap().census.classes, 0);
            assert_eq!(
                (
                    &*mod_record.class,
                    &*mod_record.tier,
                    &*mod_record.confidence
                ),
                ("content", "deep", "known")
            );
            assert!(!pipeline::portable(mod_record));
            let carried = work
                .join("mods")
                .join(format!("datapacks@{id}"))
                .join("carry");
            assert!(
                carried
                    .join(format!("data/{id}/recipe/from_datapack.json"))
                    .is_file()
            );
            assert!(
                !carried
                    .join(format!("data/{id}/recipe/from_mod.json"))
                    .exists()
            );
            assert!(!work.join("mods").join(id).exists());
            let manifest = horizon_mods::Manifest::parse(
                &fs::read_to_string(carried.join("horizon-mod.toml")).unwrap(),
            )
            .unwrap();
            assert_eq!(manifest.id, id);
            let rows: Vec<report::Row> = pipeline::read_json(&work.join("coverage.json")).unwrap();
            assert_eq!(
                rows.iter().find(|r| r.tier == "deep").unwrap().status,
                report::Status::NotYet
            );
        }
    }
}

#[test]
fn legacy_datapack_updates_select_new_content_without_stealing_a_mods_output() {
    use horizon_port_pack::{carry, jar};
    let work = TestDir::new("legacy-update");
    let data = work.0.join("a_review_pack.zip");
    let write_data = |recipe: &str| {
        let mut zip = zip::ZipWriter::new(fs::File::create(&data).unwrap());
        for (name, value) in [
            (
                "pack.mcmeta".to_owned(),
                r#"{"pack":{"pack_format":48,"description":"Original fixture"}}"#,
            ),
            (
                format!("data/a_review_pack/recipe/{recipe}.json"),
                r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"item":"minecraft:stone"}],"result":{"id":"minecraft:stone"}}"#,
            ),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(value.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    };
    write_data("old_recipe");
    let mut pack = Pack {
        files: vec![PackFile {
            path: "datapacks/a_review_pack.zip".into(),
            jar: Some(data.clone()),
            ..PackFile::default()
        }],
        ..Pack::default()
    };
    pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
    pipeline::inventory(&work.0, true, &|_| {}).unwrap();
    let legacy = work.0.join("mods/a_review_pack");
    let context = carry::Pack {
        loaded: &|_| true,
        mods: &|_| false,
    };
    // The unchanged carry writer reproduces the older unscoped output layout.
    carry::carry(
        &data,
        &jar::read(&data).unwrap(),
        &legacy,
        None,
        &|_| true,
        &context,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(pipeline::carried(&work.0).unwrap(), [legacy.join("carry")]);
    assert_eq!(
        pipeline::report(&work.0, &|_| {}).unwrap()[0].status,
        report::Status::Partial
    );
    write_data("new_recipe");
    pipeline::port(&work.0, None, None, &|_| {}).unwrap();
    let current = work.0.join("mods/datapacks@a_review_pack/carry");
    let paths = pipeline::carried(&work.0).unwrap();
    let (on, off) = pipeline::loadable(&paths);
    assert_eq!(on.as_slice(), std::slice::from_ref(&current));
    assert!(off.is_empty());
    assert!(
        current
            .join("data/a_review_pack/recipe/new_recipe.json")
            .is_file()
    );
    assert!(
        !current
            .join("data/a_review_pack/recipe/old_recipe.json")
            .exists()
    );
    assert!(
        legacy
            .join("carry/data/a_review_pack/recipe/old_recipe.json")
            .is_file()
    );

    // Adding a same-named mod must not turn the old datapack into that mod's output.
    let mod_jar = work.0.join("opaque.jar");
    collision_archive(&mod_jar, "a_review_pack", true);
    pack.files.push(PackFile {
        path: "mods/opaque.jar".into(),
        jar: Some(mod_jar),
        ..PackFile::default()
    });
    pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
    pipeline::inventory(&work.0, true, &|_| {}).unwrap();
    assert_eq!(
        pipeline::carried(&work.0).unwrap().as_slice(),
        std::slice::from_ref(&current)
    );
    pipeline::carry(&work.0, None, &["a_review_pack".into()], &|_| {}).unwrap();
    assert!(
        legacy
            .join("carry/data/a_review_pack/recipe/from_mod.json")
            .is_file()
    );
    let paths = pipeline::carried(&work.0).unwrap();
    assert!(paths.contains(&legacy.join("carry")));
    assert!(paths.contains(&current));
    let (on, _) = pipeline::loadable(&paths);
    assert_eq!(on, [legacy.join("carry")]);
}

#[test]
fn stale_datapack_output_does_not_satisfy_a_deep_mod_load_condition() {
    let work = TestDir::new("stale-condition");
    let legacy = work.0.join("mods/pixelmon");
    fs::create_dir_all(legacy.join("carry")).unwrap();
    fs::write(legacy.join("carry/horizon-mod.toml"), "id = \"pixelmon\"\nname = \"Original fixture\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n").unwrap();
    pipeline::write_json(&legacy.join("source.json"), &"datapacks/pixelmon.zip").unwrap();
    let mod_jar = work.0.join("opaque.jar");
    collision_archive(&mod_jar, "pixelmon", true);
    let data = work.0.join("helper.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&data).unwrap());
    for (path, value) in [
        (
            "pack.mcmeta",
            serde_json::json!({"pack": {"pack_format": 48, "description": "Original fixture"}}),
        ),
        (
            "data/helper/lithostitched/worldgen_modifier/optional.json",
            serde_json::json!({
            "type": "lithostitched:add_template_pool_elements", "template_pools": "minecraft:village/plains/houses",
            "elements": [{"weight": 1, "element": {"element_type": "minecraft:empty_pool_element"}}],
            "neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "pixelmon"}]}),
        ),
    ] {
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(value.to_string().as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let mut pack = Pack {
        files: vec![
            PackFile {
                path: "datapacks/helper.zip".into(),
                jar: Some(data),
                ..PackFile::default()
            },
            PackFile {
                path: "mods/opaque.jar".into(),
                jar: Some(mod_jar),
                ..PackFile::default()
            },
        ],
        ..Pack::default()
    };
    pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
    pipeline::inventory(&work.0, true, &|_| {}).unwrap();
    pipeline::port(&work.0, None, None, &|_| {}).unwrap();
    let addition = work
        .0
        .join("mods/datapacks@helper/carry/data/helper/worldgen/template_pool")
        .join(horizon_port_pack::worldgen::ADDED)
        .join("optional.json");
    assert!(
        !addition.exists(),
        "a superseded datapack is not a loaded Pixelmon mod"
    );
    let same_named_data = work.0.join("pixelmon.zip");
    collision_archive(&same_named_data, "pixelmon", false);
    pack.files.push(PackFile {
        path: "datapacks/pixelmon.zip".into(),
        jar: Some(same_named_data),
        ..PackFile::default()
    });
    pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
    pipeline::port(&work.0, None, None, &|_| {}).unwrap();
    assert!(
        !addition.exists(),
        "a current datapack filename is not a loaded Pixelmon mod"
    );
    let mod_jar = pack.files[1].jar.as_ref().unwrap();
    let original_bytes = fs::read(mod_jar).unwrap();
    fs::write(mod_jar, b"original invalid archive control").unwrap();
    pipeline::carry(
        &work.0,
        None,
        &["helper".into(), "pixelmon".into()],
        &|_| {},
    )
    .unwrap();
    assert!(
        !addition.exists(),
        "a failed mod carry cannot satisfy its load condition"
    );
    fs::write(mod_jar, original_bytes).unwrap();
    pipeline::carry(
        &work.0,
        None,
        &["helper".into(), "pixelmon".into()],
        &|_| {},
    )
    .unwrap();
    assert!(
        addition.is_file(),
        "a successful mod carry precedes its dependent pack"
    );
    pipeline::port(&work.0, None, None, &|_| {}).unwrap();
    assert!(
        addition.is_file(),
        "a genuine carried mod satisfies the condition"
    );
}

#[test]
fn all_unmarked_generations_keep_their_previous_owner_or_remain_excluded() {
    use horizon_port_pack::{carry, jar};
    for previous in ["data", "ambiguous", "mod", "missing"] {
        let work = TestDir::new(&format!("generations-{previous}"));
        let data = work.0.join("pixelmon.zip");
        let mod_jar = work.0.join("opaque.jar");
        collision_archive(&data, "pixelmon", false);
        collision_archive(&mod_jar, "pixelmon", true);
        let data_record = || PackFile {
            path: "datapacks/pixelmon.zip".into(),
            jar: Some(data.clone()),
            ..PackFile::default()
        };
        let mod_record = || PackFile {
            path: "mods/opaque.jar".into(),
            jar: Some(mod_jar.clone()),
            ..PackFile::default()
        };
        let pack = Pack {
            files: match previous {
                "data" => vec![data_record()],
                "mod" => vec![mod_record()],
                _ => vec![data_record(), mod_record()],
            },
            ..Pack::default()
        };
        if previous != "missing" {
            pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
            pipeline::inventory(&work.0, true, &|_| {}).unwrap();
        }
        let legacy = work.0.join("mods/pixelmon");
        let scoped = work.0.join("mods/datapacks@pixelmon");
        let context = carry::Pack {
            loaded: &|_| false,
            mods: &|_| false,
        };
        for dir in [&legacy, &scoped] {
            let source = if previous == "mod" && *dir == legacy {
                &mod_jar
            } else {
                &data
            };
            carry::carry(
                source,
                &jar::read(source).unwrap(),
                dir,
                None,
                &|_| true,
                &context,
                &Default::default(),
            )
            .unwrap();
            assert!(!dir.join("source.json").exists());
        }
        let retained =
            fs::read(scoped.join("carry/data/pixelmon/recipe/from_datapack.json")).unwrap();
        pipeline::write_json(
            &work.0.join("pack.json"),
            &Pack {
                files: vec![mod_record()],
                ..Pack::default()
            },
        )
        .unwrap();
        for _ in 0..3 {
            pipeline::port(&work.0, None, None, &|_| {}).unwrap();
            let paths = pipeline::carried(&work.0).unwrap();
            let rows = pipeline::report(&work.0, &|_| {}).unwrap();
            if previous == "mod" {
                assert_eq!(paths, [legacy.join("carry")]);
                assert_eq!(rows[0].status, report::Status::Partial);
            } else {
                assert!(paths.is_empty(), "{previous}: {paths:?}");
                assert_eq!(rows[0].status, report::Status::NotYet);
            }
            let bare_source: String = pipeline::read_json(&legacy.join("source.json")).unwrap();
            let scoped_source: String = pipeline::read_json(&scoped.join("source.json")).unwrap();
            assert_eq!(
                bare_source,
                match previous {
                    "data" => "datapacks/pixelmon.zip",
                    "mod" => "mods/opaque.jar",
                    _ => "",
                }
            );
            assert_eq!(
                scoped_source,
                if matches!(previous, "data" | "ambiguous") {
                    "datapacks/pixelmon.zip"
                } else {
                    ""
                }
            );
            assert_eq!(
                fs::read(scoped.join("carry/data/pixelmon/recipe/from_datapack.json")).unwrap(),
                retained
            );
        }
        pipeline::carry(&work.0, None, &["pixelmon".into()], &|_| {}).unwrap();
        assert_eq!(pipeline::carried(&work.0).unwrap(), [legacy.join("carry")]);
        assert!(
            legacy
                .join("carry/data/pixelmon/recipe/from_mod.json")
                .is_file()
        );
    }
}

#[test]
fn replacing_carried_content_requires_fresh_harness_evidence() {
    let work = TestDir::new("replace-evidence");
    let data = work.0.join("review_evidence.zip");
    let write_data = |recipe: &str| {
        let mut zip = zip::ZipWriter::new(fs::File::create(&data).unwrap());
        zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"pack":{"pack_format":48,"description":"Original fixture"}}"#)
            .unwrap();
        zip.start_file(
            format!("data/review_evidence/recipe/{recipe}.json"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(br#"{"type":"minecraft:crafting_shapeless","ingredients":[{"item":"minecraft:stone"}],"result":{"id":"minecraft:stone","count":1}}"#).unwrap();
        zip.finish().unwrap();
    };
    let write_pack = |generation: &str| {
        pipeline::write_json(
            &work.0.join("pack.json"),
            &Pack {
                files: vec![PackFile {
                    path: format!("datapacks/{generation}/review_evidence.zip"),
                    jar: Some(data.clone()),
                    ..PackFile::default()
                }],
                ..Pack::default()
            },
        )
        .unwrap();
    };
    write_data("before");
    write_pack("old");
    pipeline::port(&work.0, None, None, &|_| {}).unwrap();
    let output = work.0.join("mods/datapacks@review_evidence");
    let harness = output.join("harness/result.json");
    fs::create_dir(harness.parent().unwrap()).unwrap();
    let green =
        serde_json::json!({"passed": true, "checks": {"recipes": {"passed": 1, "failed": 0}}});
    pipeline::write_json(&harness, &green).unwrap();
    assert_eq!(
        pipeline::report(&work.0, &|_| {}).unwrap()[0].status,
        report::Status::Ported
    );
    write_data("after");
    write_pack("new");
    pipeline::port(&work.0, None, None, &|_| {}).unwrap();
    assert!(
        !harness.exists(),
        "replacing content must invalidate its old comparisons"
    );
    let rows = pipeline::report(&work.0, &|_| {}).unwrap();
    assert_eq!(rows[0].status, report::Status::Partial);
    assert!(rows[0].reason.contains("harness not run"));
    assert!(
        output
            .join("carry/data/review_evidence/recipe/after.json")
            .is_file()
    );
    assert!(
        !output
            .join("carry/data/review_evidence/recipe/before.json")
            .exists()
    );
    pipeline::write_json(&harness, &green).unwrap();
    assert_eq!(
        pipeline::report(&work.0, &|_| {}).unwrap()[0].status,
        report::Status::Ported
    );
    pipeline::carry(&work.0, None, &["review_evidence".into()], &|_| {}).unwrap();
    assert!(
        !harness.exists(),
        "explicit carry must invalidate comparisons too"
    );
}

#[test]
fn manual_pixelmon_content_and_green_recipe_checks_do_not_prove_native_behavior() {
    let work = TestDir::new("manual-report");
    let jar = work.0.join("opaque.jar");
    collision_archive(&jar, "pixelmon", true);
    let pack = Pack {
        files: vec![PackFile {
            path: "mods/opaque.jar".into(),
            jar: Some(jar),
            ..PackFile::default()
        }],
        ..Pack::default()
    };
    pipeline::write_json(&work.0.join("pack.json"), &pack).unwrap();
    pipeline::inventory(&work.0, true, &|_| {}).unwrap();
    assert_eq!(
        pipeline::carry(&work.0, None, &["pixelmon".into()], &|_| {}).unwrap(),
        1
    );
    let harness = work.0.join("mods/pixelmon/harness");
    fs::create_dir(&harness).unwrap();
    pipeline::write_json(
        &harness.join("result.json"),
        &serde_json::json!({"passed": true, "checks": {"recipes": {"passed": 1, "failed": 0}}}),
    )
    .unwrap();
    let rows = pipeline::report(&work.0, &|_| {}).unwrap();
    assert_eq!(rows[0].status, report::Status::Partial);
    assert!(
        rows[0]
            .reason
            .contains("native behavior still needs implementation and verification")
    );
}
