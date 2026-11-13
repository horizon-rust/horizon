//! Shared data-pack metadata codec boundary. Callers borrow the registries
//! already owned by startup/reload; this layer performs no registry I/O.
use horizon_command::component::json::{self, Context, Error, value::Value};

/// Decode the fallback pack section through plain JsonOps semantics. An
/// unavailable context or unfinished value codec remains an explicit error;
/// callers must not reinterpret it as an ordinary invalid-pack negative.
/// Format compatibility and file discovery policy are handled by their owners.
pub fn validate(bytes: &[u8], context: &dyn Context) -> Result<(), Error> {
    let root = json::value::parse_first(bytes).map_err(|_| Error::Invalid)?;
    let description = root
        .as_object()
        .and_then(|m| m.get("pack"))
        .and_then(Value::as_object)
        .and_then(|m| m.get("description"))
        .ok_or(Error::Invalid)?;
    json::validate(description, context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_command::component::json::context::MetadataContext;
    #[test]
    fn fallback_pack_codec_keeps_partial_and_unavailable_results_distinct() {
        let registries = horizon_command::Registries::default();
        let context = MetadataContext {
            registries: &registries,
        };
        for (raw, expected) in [
            (r#"{"pack":{"description":"description"}}"#, Ok(())),
            (
                r#"{"pack":{"description":{"text":"description","bold":1}}}"#,
                Err(Error::Invalid),
            ),
            (r#"{"pack":{"description":7}}"#, Err(Error::Invalid)),
            (r#"{"pack":{}}"#, Err(Error::Invalid)),
            (
                r#"{"pack":{"description":{"text":"description","hover_event":{"action":"show_item","id":"minecraft:stone"}}}}"#,
                Err(Error::Unavailable),
            ),
            (
                r#"{"pack":{"description":{"text":"description","click_event":{"action":"change_page","page":9007199254740993}}}}"#,
                Ok(()),
            ),
            (
                r#"{"pack":{"description":{"text":"description","click_event":{"action":"change_page","page":0.99999999999999999999999}}}}"#,
                Err(Error::Invalid),
            ),
        ] {
            assert_eq!(validate(raw.as_bytes(), &context), expected, "{raw}");
        }
    }
}

/// Borrow a root's existing command catalog for one operation. Missing files
/// are explicit unavailable authority for nested registry-dependent values;
/// context-free inline profiles/dialogs still use their own codecs.
pub fn with_root<T>(
    resources: &std::path::Path,
    keep: bool,
    f: impl FnOnce(&dyn Context) -> T,
) -> T {
    use horizon_command::component::json::context::{MetadataContext, UnavailableContext};
    let catalog = horizon_client_core::registries::prepared::command_export(resources, keep);
    match catalog {
        Some(catalog) => f(&MetadataContext {
            registries: &catalog,
        }),
        None => f(&UnavailableContext),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct AdmissionError {
    pub pack: String,
    pub reason: Error,
}
/// Apply the metadata codec before a caller selects or loads pack content.
/// Invalid/throwing metadata excludes only its pack, as Pack.readPackMetadata does.
/// Missing authority and unfinished implementations are operational errors,
/// never fabricated valid-negative pack decisions. No active store is changed.
pub fn admit(
    packs: Vec<super::DataPack>,
    context: &dyn Context,
) -> Result<Vec<super::DataPack>, AdmissionError> {
    let mut admitted = Vec::with_capacity(packs.len());
    for pack in packs {
        let result = pack
            .read("pack.mcmeta")
            .ok_or(Error::Invalid)
            .and_then(|bytes| validate(&bytes, context));
        match result {
            Ok(()) => admitted.push(pack),
            Err(Error::Invalid | Error::DecodeException) => {}
            Err(reason @ (Error::Unavailable | Error::IncompleteCodec)) => {
                return Err(AdmissionError {
                    pack: pack.id,
                    reason,
                });
            }
        }
    }
    Ok(admitted)
}

#[cfg(test)]
mod caller_tests {
    use super::*;
    use crate::datapack::{self, DataStore};
    #[test]
    fn borrowed_catalog_admission_precedes_selection_and_store_loading() {
        let world = std::env::temp_dir().join(format!(
            "h146-metadata-caller-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let resources = world.join("resources");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(resources.join("command-registries.json"),serde_json::json!({"by_registry":{"minecraft:item":{"ids":["minecraft:stone"]}},"persistent_components":["minecraft:weapon"]}).to_string()).unwrap();
        for (id, description, body) in [
            (
                "a",
                serde_json::json!({"text":"first","hover_event":{"action":"show_item","id":"stone","components":{"weapon":{}}}}),
                "return 11",
            ),
            ("b", serde_json::json!({"text":"bad","bold":1}), "return 99"),
            (
                "c",
                serde_json::json!({"text":"last","click_event":{"action":"show_dialog","dialog":{"type":"notice","title":"Inline"}}}),
                "return 33",
            ),
        ] {
            let dir = world.join("datapacks").join(id);
            std::fs::create_dir_all(dir.join("data/test/function")).unwrap();
            std::fs::write(
                dir.join("pack.mcmeta"),
                serde_json::json!({"pack":{"description":description}}).to_string(),
            )
            .unwrap();
            std::fs::write(dir.join("data/test/function/value.mcfunction"), body).unwrap();
        }
        std::fs::write(
            world.join("datapacks.json"),
            r#"{"enabled":["file/c","file/b","file/a"]}"#,
        )
        .unwrap();
        let packs = with_root(&resources, false, |context| {
            datapack::discover(&world, context)
        })
        .unwrap();
        let enabled = datapack::enabled_ids(&world, &packs, &[]);
        assert_eq!(enabled, vec!["file/c", "file/a"]);
        let refs: Vec<_> = enabled
            .iter()
            .map(|id| packs.iter().find(|p| &p.id == id).unwrap())
            .collect();
        let active = DataStore::load(&refs);
        assert_eq!(
            active.functions["test:value"].instantiate(None).unwrap(),
            vec!["return 11"]
        );
        // A fresh read uses the new bytes; no admitted/invalid decision is cached.
        std::fs::write(
            world.join("datapacks/a/pack.mcmeta"),
            r#"{"pack":{"description":{"text":"x","bold":1}}}"#,
        )
        .unwrap();
        let next = with_root(&resources, false, |context| {
            datapack::discover(&world, context)
        })
        .unwrap();
        assert_eq!(
            next.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["file/c"]
        );
        assert_eq!(
            active.functions["test:value"].instantiate(None).unwrap(),
            vec!["return 11"]
        );
        // An unavailable authority is not a valid negative and does not publish
        // a replacement for the active snapshot.
        let missing = world.join("missing");
        std::fs::write(world.join("datapacks/a/pack.mcmeta"),r#"{"pack":{"description":{"text":"x","hover_event":{"action":"show_item","id":"stone"}}}}"#).unwrap();
        let error = with_root(&missing, false, |context| {
            datapack::discover(&world, context)
        })
        .err()
        .unwrap();
        assert_eq!(
            error,
            AdmissionError {
                pack: "file/a".into(),
                reason: Error::Unavailable
            }
        );
        assert_eq!(
            active.functions["test:value"].instantiate(None).unwrap(),
            vec!["return 11"]
        );
        let inline=br#"{"pack":{"description":{"text":"x","click_event":{"action":"show_dialog","dialog":{"type":"notice","title":"t"}}}}}"#;
        assert_eq!(
            with_root(&missing, false, |context| validate(inline, context)),
            Ok(())
        );
    }
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "data pack {} metadata codec unavailable: {:?}",
            self.pack, self.reason
        )
    }
}
impl std::error::Error for AdmissionError {}

/// Report an operational repository error while the caller retains its current
/// screen snapshot. `None` cancels the operation; it is not an empty repository.
pub fn report<T>(result: Result<T, AdmissionError>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            log::error!("Horizon: data pack repository: {error}");
            None
        }
    }
}
