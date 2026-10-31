//! Tests for `fabric.mod.json` parsing and dependency resolution.

use std::collections::HashMap;
use std::path::PathBuf;

use horizon_fabric_loader::dependency::{DependencyResolver, ResolutionError};
use horizon_fabric_loader::{Entrypoints, FabricMod, FabricModJson};

const SAMPLE: &str = r#"
{
  "schemaVersion": 1,
  "id": "examplemod",
  "version": "1.2.3",
  "name": "Example Mod",
  "description": "A mod",
  "authors": ["someone"],
  "license": "MIT",
  "entrypoints": {
    "main": ["com.example.ExampleMod"],
    "client": [{ "value": "com.example.ExampleClient", "adapter": "kotlin" }]
  },
  "mixins": ["examplemod.mixins.json"],
  "depends": { "minecraft": "26.3", "fabricloader": ">=0.16.0" }
}
"#;

#[test]
fn parses_entrypoints_and_mixins() {
    let parsed: FabricModJson = serde_json::from_str(SAMPLE).unwrap();

    assert_eq!(parsed.id, "examplemod");
    assert_eq!(parsed.entrypoints.main, vec!["com.example.ExampleMod"]);
    // The object form keeps only its class name.
    assert_eq!(parsed.entrypoints.client, vec!["com.example.ExampleClient"]);
    assert_eq!(parsed.mixins, vec!["examplemod.mixins.json"]);
    assert!(parsed.depends.contains_key("fabricloader"));
    parsed.validate().unwrap();
}

#[test]
fn rejects_invalid_mod_ids() {
    let mut parsed: FabricModJson = serde_json::from_str(SAMPLE).unwrap();
    parsed.id = "ExampleMod".into();
    assert!(parsed.validate().is_err());
}

fn mod_with(id: &str, depends: &[(&str, &str)]) -> FabricMod {
    FabricMod {
        id: id.to_string(),
        name: id.to_string(),
        version: "1.0.0".into(),
        description: String::new(),
        authors: Vec::new(),
        contact: HashMap::new(),
        license: "MIT".into(),
        icon: None,
        entrypoints: Entrypoints::default(),
        mixins: Vec::new(),
        depends: depends
            .iter()
            .map(|(id, range)| (id.to_string(), range.to_string()))
            .collect(),
        recommends: HashMap::new(),
        suggests: HashMap::new(),
        conflicts: HashMap::new(),
        breaks: Vec::new(),
        custom: serde_json::Value::Null,
        jar_path: PathBuf::from(format!("{id}.jar")),
    }
}

#[test]
fn resolves_dependencies_in_order() {
    let mut mods = HashMap::new();
    mods.insert("a".to_string(), mod_with("a", &[("b", ">=1.0.0")]));
    mods.insert("b".to_string(), mod_with("b", &[]));

    let resolved = DependencyResolver::new().resolve(&mods).unwrap();
    let order: Vec<&str> = resolved.iter().map(|m| m.mod_info.id.as_str()).collect();

    let b = order.iter().position(|id| *id == "b").unwrap();
    let a = order.iter().position(|id| *id == "a").unwrap();
    assert!(b < a, "dependency must load before its dependent: {order:?}");
}

#[test]
fn reports_missing_required_dependency() {
    let mut mods = HashMap::new();
    mods.insert("a".to_string(), mod_with("a", &[("missing", ">=1.0.0")]));

    let error = DependencyResolver::new().resolve(&mods).unwrap_err();
    assert!(matches!(error, ResolutionError::MissingDependency { .. }));
}

#[test]
fn detects_dependency_cycles() {
    let mut mods = HashMap::new();
    mods.insert("a".to_string(), mod_with("a", &[("b", ">=1.0.0")]));
    mods.insert("b".to_string(), mod_with("b", &[("a", ">=1.0.0")]));

    let error = DependencyResolver::new().resolve(&mods).unwrap_err();
    assert!(matches!(error, ResolutionError::CircularDependency(_)));
}
