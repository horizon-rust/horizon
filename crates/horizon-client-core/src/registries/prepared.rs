//! The existing per-resource-root command export authority, shared by command
//! construction and metadata callers that run before a live world exists.
use super::Registries;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type Exports = Vec<(PathBuf, Option<Arc<Registries>>)>;
static COMMAND_EXPORTS: Mutex<Exports> = Mutex::new(Vec::new());

/// Read a root once when retained by a client. Dedicated one-shot callers
/// pass `keep=false`, preserving the existing no-retention contract.
/// No metadata validity result is cached and roots never share a catalog.
pub fn command_export(dir: &Path, keep: bool) -> Option<Arc<Registries>> {
    let mut cache = COMMAND_EXPORTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, export)) = cache.iter().find(|(cached, _)| cached == dir) {
        return export.clone();
    }
    let export = std::fs::read(dir.join("command-registries.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .map(|export: serde_json::Value| {
            let mut registries = Registries::from_export(&export);
            // These legal domains already belong to the prepared export.
            // Live command construction continues to use its BlockRegistry;
            // fill_from_exported does not overwrite that loaded block map.
            if let Some(blocks) = export.get("blocks") {
                registries.blocks = serde_json::from_value(blocks.clone()).unwrap_or_default();
            }
            Arc::new(registries)
        });
    if keep {
        cache.push((dir.to_owned(), export.clone()));
    }
    export
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roots_keep_their_own_catalog_and_one_shot_reads_do_not_publish_a_cache() {
        let root = std::env::temp_dir().join(format!(
            "h146-catalog-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        for (name, id) in [("first", "minecraft:stone"), ("second", "minecraft:apple")] {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("command-registries.json"),serde_json::json!({"entity_types":[id],"blocks":{"minecraft:grass_block":[["snowy",["true","false"]]]}}).to_string()).unwrap();
        }
        let first = command_export(&root.join("first"), true).unwrap();
        let second = command_export(&root.join("second"), false).unwrap();
        assert_eq!(first.entity_types, vec!["minecraft:stone"]);
        assert_eq!(second.entity_types, vec!["minecraft:apple"]);
        assert_eq!(
            first.blocks["minecraft:grass_block"][0],
            ("snowy".into(), vec!["true".into(), "false".into()])
        );
        std::fs::write(root.join("first/command-registries.json"), "{}").unwrap();
        std::fs::write(root.join("second/command-registries.json"), "{}").unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &command_export(&root.join("first"), true).unwrap()
        ));
        assert!(
            command_export(&root.join("second"), false)
                .unwrap()
                .entity_types
                .is_empty()
        );
        assert!(command_export(&root.join("missing"), false).is_none());
    }
}
