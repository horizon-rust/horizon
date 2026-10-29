use horizon_port_pack::{modrinth::Cache, pipeline};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use sha2::Sha512;
use std::{fs, io::Write, path::PathBuf, process::Command};

use crate::modpacks;

fn isolated_test(label: &str, test: &str) -> bool {
    if std::env::var("HORIZON_IMPORT_TEST_CASE").as_deref() == Ok(test) {
        fs::write(
            std::env::var_os("HORIZON_IMPORT_TEST_ENTERED").unwrap(),
            test,
        )
        .unwrap();
        return false;
    }
    let home = std::env::temp_dir().join(format!(
        "horizon-import-home-{label}-{}",
        std::process::id()
    ));
    fs::create_dir(&home).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test, "--nocapture"])
        .env("HORIZON_IMPORT_TEST_CASE", test)
        .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("HORIZON_NO_AUDIO", "1");
    for (key, folder) in [
        ("HORIZON_HOME", "horizon"),
        ("HORIZON_LAUNCHER_HOME", "launcher"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
        ("TMPDIR", "tmp"),
        ("HORIZON_LAUNCHER_TRASH", "trash"),
        ("HORIZON_LAUNCHER_DESKTOP", "desktop"),
    ] {
        let path = home.join(folder);
        fs::create_dir(&path).unwrap();
        command.env(key, path);
    }
    command.env("HORIZON_LAUNCHER_CLIPBOARD", home.join("clipboard.txt"));
    let entered = home.join("entered");
    command.env("HORIZON_IMPORT_TEST_ENTERED", &entered);
    let output = command.output().unwrap();
    let ran_target = fs::read_to_string(entered).ok().as_deref() == Some(test);
    fs::remove_dir_all(&home).unwrap();
    assert!(
        output.status.success() && ran_target,
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

struct Fixture {
    name: String,
    work: PathBuf,
    files: Vec<Value>,
}

impl Fixture {
    fn new(label: &str, negative: bool) -> Self {
        let name = format!("Wrapper {label} {negative}");
        let work = modpacks::ports_dir().join(modpacks::slug(&name));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(work.join("jars")).unwrap();
        let mut files = Vec::new();
        let mut cache = Cache::default();
        for id in ["pixelmon", "create"] {
            let jar = work.join(format!("jars/{id}.jar"));
            let mut zip = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
            let mut entries = vec![
                ("META-INF/neoforge.mods.toml".to_owned(), format!("license = \"MIT\"\n[[mods]]\nmodId = \"{id}\"\nversion = \"1\"\ndisplayName = \"Original fixture\"\n")),
                ("fixture/Entity.class".to_owned(), "Original placeholder without bytecode".to_owned()),
                (format!("data/{id}/recipe/example.json"), json!({"type":"minecraft:crafting_shapeless","ingredients":[{"item":"minecraft:stone"}],"result":{"id":"minecraft:stone"}}).to_string()),
            ];
            if id == "pixelmon" {
                entries[0].1.push_str(
                    "[[dependencies.pixelmon]]\nmodId = \"create\"\ntype = \"optional\"\n",
                );
                let test = json!({"type":"neoforge:mod_loaded","modid":"create"});
                let condition = if negative {
                    json!({"type":"neoforge:not","value":test})
                } else {
                    test
                };
                entries.push(("data/pixelmon/lithostitched/worldgen_modifier/conditional.json".into(), json!({
                    "type":"lithostitched:add_template_pool_elements",
                    "template_pools":"minecraft:village/plains/houses",
                    "elements":[{"weight":1,"element":{"element_type":"minecraft:empty_pool_element"}}],
                    "neoforge:conditions":[condition]
                }).to_string()));
            }
            for (name, contents) in entries {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(contents.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
            let bytes = fs::read(&jar).unwrap();
            let sha1 = format!("{:x}", Sha1::digest(&bytes));
            cache.versions.insert(sha1.clone(), json!({}));
            files.push(json!({"path":format!("mods/{id}.jar"),"hashes":{"sha1":sha1,"sha512":format!("{:x}",Sha512::digest(&bytes))},
                "fileSize":bytes.len(),"downloads":[format!("https://cdn.modrinth.com/data/original-fixture/{id}.jar")],
                "env":{"client":"required","server":"required"}}));
        }
        pipeline::write_json(&work.join("modrinth.json"), &cache).unwrap();
        Self { name, work, files }
    }

    fn archive(&self, dependency: bool) -> PathBuf {
        let path = self.work.join("update.mrpack");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        zip.start_file(
            "modrinth.index.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        let files = if dependency {
            &self.files[..]
        } else {
            &self.files[..1]
        };
        let index = json!({"formatVersion":1,"game":"minecraft","name":self.name,"versionId":"1",
            "dependencies":{"minecraft":"1.21.1","neoforge":"21.1.248"},"files":files});
        zip.write_all(index.to_string().as_bytes()).unwrap();
        zip.finish().unwrap();
        path
    }

    fn setup(&self) {
        let imported = modpacks::import_and_port(&self.archive(true), None, &|_| {}).unwrap();
        assert_eq!(imported.portable, 0);
        assert!(imported.mods.is_empty());
        assert_eq!(
            pipeline::carry(
                &self.work,
                None,
                &["pixelmon".into(), "create".into()],
                &|_| {}
            )
            .unwrap(),
            2
        );
    }

    fn owner(&self) -> PathBuf {
        self.work.join("mods/pixelmon")
    }
    fn addition(&self) -> PathBuf {
        self.owner()
            .join("carry/data/pixelmon/worldgen/template_pool/horizon_added/conditional.json")
    }
    fn green(&self) -> PathBuf {
        let path = self.owner().join("harness/result.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"passed":true,"checks":{"recipes":{"passed":1,"failed":0}}}"#,
        )
        .unwrap();
        path
    }
    fn preserved(&self) -> Vec<Vec<u8>> {
        [
            "carry/horizon-mod.toml",
            "carry/data/pixelmon/recipe/example.json",
        ]
        .map(|path| fs::read(self.owner().join(path)).unwrap())
        .to_vec()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.work);
    }
}

#[test]
fn importing_again_refreshes_deep_mod_conditions_without_automatic_carries() {
    if isolated_test("conditions", "modpacks::import_tests::importing_again_refreshes_deep_mod_conditions_without_automatic_carries") { return; }
    for negative in [false, true] {
        let fixture = Fixture::new("conditions", negative);
        fixture.setup();
        assert_eq!(fixture.addition().is_file(), !negative);
        let preserved = fixture.preserved();
        for present in [false, true, false, true] {
            let green = fixture.green();
            let imported =
                modpacks::import_and_port(&fixture.archive(present), None, &|_| {}).unwrap();
            assert_eq!(imported.portable, 0);
            assert_eq!(imported.mods.len(), if present { 2 } else { 1 });
            assert_eq!(fixture.addition().is_file(), present != negative);
            assert!(!green.exists());
            assert_eq!(fixture.preserved(), preserved);
            let mut command = Command::new("unused-fixture-command");
            modpacks::apply_profile(&mut command, &fixture.work).unwrap();
            assert_eq!(
                command
                    .get_args()
                    .skip(1)
                    .step_by(2)
                    .map(PathBuf::from)
                    .collect::<Vec<_>>(),
                imported.mods
            );
        }
        modpacks::remove(&fixture.work).unwrap();
        assert!(!fixture.work.exists());
    }
}

#[test]
fn importing_again_reports_legacy_inputs_and_recovers_after_explicit_carry() {
    if isolated_test("legacy", "modpacks::import_tests::importing_again_reports_legacy_inputs_and_recovers_after_explicit_carry") { return; }
    let fixture = Fixture::new("legacy", false);
    fixture.setup();
    let preserved = fixture.preserved();
    let addition = fs::read(fixture.addition()).unwrap();
    fs::remove_file(fixture.owner().join("worldgen-source.json")).unwrap();
    let archive = fixture.archive(false);
    for _ in 0..2 {
        let green = fixture.green();
        let error = match modpacks::import_and_port(&archive, None, &|_| {}) {
            Ok(_) => panic!("legacy inputs require explicit recovery"),
            Err(error) => error,
        };
        assert!(
            error.contains("is imported, but porting it failed"),
            "{error}"
        );
        assert!(
            error.contains("successful worldgen inputs unavailable"),
            "{error}"
        );
        assert!(error.contains("explicitly carry pixelmon"), "{error}");
        assert!(!green.exists());
        assert_eq!(fixture.preserved(), preserved);
        assert_eq!(fs::read(fixture.addition()).unwrap(), addition);
        assert!(!fixture.owner().join("worldgen-source.json").exists());
    }
    assert_eq!(
        pipeline::carry(&fixture.work, None, &["pixelmon".into()], &|_| {}).unwrap(),
        1
    );
    assert!(!fixture.addition().exists());
    assert!(fixture.owner().join("worldgen-source.json").is_file());
    let imported = modpacks::import_and_port(&archive, None, &|_| {}).unwrap();
    assert_eq!(imported.portable, 0);
    assert_eq!(imported.mods.len(), 1);
    assert_eq!(fixture.preserved(), preserved);
}
