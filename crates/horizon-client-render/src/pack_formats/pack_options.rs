//! Polytone's pack options (wiki "Polytone Configs") and the `polytone_condition`s that read
//! them, on pack overlays and on `sounds.json` events.

use crate::packs::Source;
use crate::zip::Zip;
use horizon_pack_options::Configs;
use horizon_polytone::pack::add_entry;
use std::fs;

/// A pack's Polytone config entries (`assets/<namespace>/polytone/config_entries/<name>.json`,
/// in the pack's base, not its overlays) at their default values.
pub fn read(source: &Source) -> Configs {
    let mut configs = Configs::default();
    let id_of = |name: &str| -> Option<String> {
        let rest = name.strip_prefix("assets/")?;
        let (namespace, rest) = rest.split_once('/')?;
        let entry = rest
            .strip_prefix("polytone/config_entries/")?
            .strip_suffix(".json")?;
        Some(format!("{namespace}:{entry}"))
    };
    match source {
        Source::Dir(d) => {
            let Ok(namespaces) = fs::read_dir(d.join("assets")) else {
                return configs;
            };
            for namespace in namespaces.flatten() {
                let folder = namespace.path().join("polytone/config_entries");
                for file in fs::read_dir(&folder).into_iter().flatten().flatten() {
                    let name = format!(
                        "assets/{}/polytone/config_entries/{}",
                        namespace.file_name().to_string_lossy(),
                        file.file_name().to_string_lossy()
                    );
                    if let (Some(id), Ok(bytes)) = (id_of(&name), fs::read(file.path()))
                        && let Ok(json) = serde_json::from_slice(&bytes)
                    {
                        add_entry(&mut configs, id, &json);
                    }
                }
            }
        }
        Source::Zip(z) => {
            let Ok(zip) = Zip::open(z) else {
                return configs;
            };
            let names: Vec<String> = zip
                .names()
                .filter(|n| id_of(n).is_some())
                .map(str::to_owned)
                .collect();
            for name in names {
                if let (Some(id), Ok(bytes)) = (id_of(&name), zip.read(&name))
                    && let Ok(json) = serde_json::from_slice(&bytes)
                {
                    add_entry(&mut configs, id, &json);
                }
            }
        }
    }
    configs
}

/// Polytone conditions in packs, end to end (moved from the engine's `packs` tests, which no
/// longer reach the formats).
#[cfg(test)]
mod pack_tests {
    use crate::packs::{Source, build_overlay, pack};
    use crate::text::Text;
    use std::fs;

    #[test]
    fn a_sound_entry_whose_polytone_condition_fails_is_left_out() {
        crate::pack_formats::install();
        let base =
            std::env::temp_dir().join(format!("horizon-packs-sound-cond-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (vanilla, dir, out) = (base.join("vanilla"), base.join("pack"), base.join("out"));
        fs::create_dir_all(vanilla.join("assets/minecraft")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/sounds.json"),
            r#"{"block.anvil.land":{"sounds":["vanilla"]}}"#,
        )
        .unwrap();
        let configs = dir.join("assets/mypack/polytone/config_entries");
        fs::create_dir_all(&configs).unwrap();
        fs::create_dir_all(dir.join("assets/minecraft")).unwrap();
        fs::write(configs.join("quiet.json"), r#"{"default_value": "OFF"}"#).unwrap();
        fs::write(
            dir.join("pack.mcmeta"),
            r#"{"pack":{"description":"p","pack_format":97}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("assets/minecraft/sounds.json"),
            r#"{"block.anvil.land":{"replace":true,"polytone_condition":"config('mypack:quiet') == 'ON'","sounds":["soft"]},
                "block.anvil.use":{"sounds":["kept"]}}"#,
        )
        .unwrap();
        let pack = pack("file/p".into(), Text::literal("p"), Source::Dir(dir)).unwrap();
        build_overlay(&vanilla, &[pack], &out).unwrap();
        let merged: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("assets/minecraft/sounds.json")).unwrap())
                .unwrap();
        // The condition is false: the pack's entry doesn't replace vanilla's.
        assert_eq!(
            merged["block.anvil.land"]["sounds"],
            serde_json::json!(["vanilla"])
        );
        assert_eq!(
            merged["block.anvil.use"]["sounds"],
            serde_json::json!(["kept"])
        );
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn polytone_conditions_turn_overlays_on_from_the_packs_configs() {
        crate::pack_formats::install();
        let dir =
            std::env::temp_dir().join(format!("horizon-packs-polytone-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let configs = dir.join("assets/mypack/polytone/config_entries");
        fs::create_dir_all(&configs).unwrap();
        fs::write(configs.join("fog.json"), r#"{"default_value": "ON"}"#).unwrap();
        fs::write(
            configs.join("birds.json"),
            r#"{"default_value": 0, "max": 300}"#,
        )
        .unwrap();
        fs::write(
            dir.join("pack.mcmeta"),
            r#"{"pack":{"description":"p","pack_format":97},"overlays":{"entries":[
                {"directory":"opt_fog","formats":[0,0],"polytone_condition":"config('mypack:fog') == 'ON'"},
                {"directory":"opt_birds","formats":[0,0],"polytone_condition":"config('mypack:birds') > 0"},
                {"directory":"opt_plain","formats":[0,0]}]}}"#,
        )
        .unwrap();
        let pack = pack(
            "file/p".into(),
            Text::literal("p"),
            Source::Dir(dir.clone()),
        )
        .unwrap();
        assert_eq!(pack.overlays, ["opt_fog"]);
        let _ = fs::remove_dir_all(dir);
    }
}
