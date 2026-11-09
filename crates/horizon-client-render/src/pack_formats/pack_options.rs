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
