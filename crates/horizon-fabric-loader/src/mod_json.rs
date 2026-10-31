//! Fabric mod JSON parsing and types.

use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The parsed fabric.mod.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricModJson {
    /// Schema version (always 1).
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    
    /// Unique mod ID (lowercase, alphanumeric, hyphens, underscores).
    pub id: String,
    
    /// Human-readable mod name.
    pub name: String,
    
    /// Mod version (semver).
    pub version: String,
    
    /// Description.
    #[serde(default)]
    pub description: String,
    
    /// Authors.
    #[serde(default)]
    pub authors: Vec<String>,
    
    /// Contact information.
    #[serde(default)]
    pub contact: HashMap<String, String>,
    
    /// License identifier (SPDX).
    #[serde(default)]
    pub license: String,
    
    /// Icon file (relative to JAR root).
    #[serde(default)]
    pub icon: Option<String>,
    
    /// Entrypoints.
    #[serde(default)]
    pub entrypoints: Entrypoints,

    /// Mixin configuration file names, relative to the jar root.
    #[serde(default)]
    pub mixins: Vec<String>,
    
    /// Dependencies.
    #[serde(default)]
    pub depends: DependencyMap,
    
    /// Recommended dependencies.
    #[serde(default)]
    pub recommends: DependencyMap,
    
    /// Suggested dependencies.
    #[serde(default)]
    pub suggests: DependencyMap,
    
    /// Conflicting mods.
    #[serde(default)]
    pub conflicts: DependencyMap,
    
    /// Mods that this mod breaks (replaces).
    #[serde(default, rename = "break")]
    pub breaks: Vec<String>,
    
    /// Custom data.
    #[serde(default)]
    pub custom: serde_json::Value,
}

fn default_schema_version() -> u32 {
    1
}

/// A mod with resolved file path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricMod {
    /// Unique mod ID.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Mod version.
    pub version: String,
    /// Description.
    pub description: String,
    /// Authors.
    pub authors: Vec<String>,
    /// Contact links, keyed by kind (`homepage`, `issues`, ...).
    pub contact: HashMap<String, String>,
    /// SPDX license identifier.
    pub license: String,
    /// Icon path inside the jar.
    pub icon: Option<String>,
    /// Entrypoints per phase.
    pub entrypoints: Entrypoints,
    /// Mixin configuration file names, relative to the jar root.
    pub mixins: Vec<String>,
    /// Required dependencies.
    pub depends: DependencyMap,
    /// Recommended dependencies.
    pub recommends: DependencyMap,
    /// Suggested dependencies.
    pub suggests: DependencyMap,
    /// Mods this mod cannot run with.
    pub conflicts: DependencyMap,
    /// Mods this mod breaks (replaces).
    pub breaks: Vec<String>,
    /// Unrecognised custom data.
    pub custom: serde_json::Value,
    /// Path to the mod jar on disk.
    pub jar_path: PathBuf,
}

/// Entrypoints for different load phases.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Entrypoints {
    /// Main entrypoints (ModInitializer) - run on both client and server.
    #[serde(default, deserialize_with = "deserialize_entrypoint_list")]
    pub main: Vec<String>,
    
    /// Client entrypoints (ClientModInitializer) - run on client only.
    #[serde(default, deserialize_with = "deserialize_entrypoint_list")]
    pub client: Vec<String>,
    
    /// Server entrypoints (DedicatedServerModInitializer) - run on dedicated server only.
    #[serde(default, deserialize_with = "deserialize_entrypoint_list")]
    pub server: Vec<String>,
}

/// Fabric allows an entrypoint to be either a bare class name or an object with
/// a `value` and an optional `adapter`. Normalize both to the class name.
fn deserialize_entrypoint_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    let raw = Vec::<serde_json::Value>::deserialize(deserializer)?;
    raw.into_iter()
        .map(|value| match value {
            serde_json::Value::String(name) => Ok(name),
            serde_json::Value::Object(object) => object
                .get("value")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .ok_or_else(|| D::Error::custom("entrypoint object is missing `value`")),
            other => Err(D::Error::custom(format!(
                "invalid entrypoint: {other}"
            ))),
        })
        .collect()
}

impl Entrypoints {
    /// Get entrypoints for a specific load phase.
    pub fn for_phase(&self, phase: crate::entrypoint::LoadPhase) -> Vec<String> {
        match phase {
            crate::entrypoint::LoadPhase::PreLaunch => Vec::new(),
            crate::entrypoint::LoadPhase::Launch => self.main.clone(),
            crate::entrypoint::LoadPhase::ClientLaunch => self.client.clone(),
            crate::entrypoint::LoadPhase::ServerLaunch => self.server.clone(),
        }
    }
}

/// Dependency map: mod ID -> version requirement.
pub type DependencyMap = HashMap<String, String>;

/// Mod contact information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModContact {
    /// Project homepage.
    pub homepage: Option<String>,
    /// Issue tracker.
    pub issues: Option<String>,
    /// Source repository.
    pub sources: Option<String>,
    /// Wiki.
    pub wiki: Option<String>,
    /// Discord invite.
    pub discord: Option<String>,
    /// Donation link.
    pub donate: Option<String>,
}

/// Mod license.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModLicense {
    /// SPDX identifier.
    pub identifier: String,
    /// Link to the full license text.
    pub url: Option<String>,
}

/// Entrypoint type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntrypointType {
    /// `main` entrypoints, run on both sides.
    Main,
    /// `client` entrypoints.
    Client,
    /// `server` entrypoints.
    Server,
    /// `mixins` configurations.
    Mixin,
}

/// Errors from mod JSON parsing.
#[derive(Debug, Error)]
pub enum ModJsonError {
    /// The JSON could not be parsed.
    #[error("JSON parse error: {0}")]
    Parse(#[from] serde_json::Error),

    /// A required field is absent.
    #[error("Missing required field: {0}")]
    MissingField(String),

    /// The mod ID is not a valid Fabric identifier.
    #[error("Invalid mod ID: {0} (must be lowercase alphanumeric with hyphens/underscores)")]
    InvalidModId(String),

    /// The version is not valid semver.
    #[error("Invalid version: {0} (must be valid semver)")]
    InvalidVersion(String),

    /// A dependency entry is malformed.
    #[error("Invalid dependency: {0}")]
    InvalidDependency(String),
}

impl FabricModJson {
    /// Validate the mod JSON.
    pub fn validate(&self) -> Result<(), ModJsonError> {
        // Validate mod ID
        if !is_valid_mod_id(&self.id) {
            return Err(ModJsonError::InvalidModId(self.id.clone()));
        }
        
        // Validate version
        if semver::Version::parse(&self.version).is_err() {
            return Err(ModJsonError::InvalidVersion(self.version.clone()));
        }
        
        // Validate dependencies
        for (dep_id, _dep_version) in &self.depends {
            if !is_valid_mod_id(dep_id) && dep_id != "minecraft" && dep_id != "fabricloader" && dep_id != "fabric-api" {
                return Err(ModJsonError::InvalidDependency(format!("Invalid dependency ID: {}", dep_id)));
            }
            // Version requirement validation would go here
        }
        
        Ok(())
    }
}

/// Check if a string is a valid mod ID.
fn is_valid_mod_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 64 {
        return false;
    }
    
    id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

impl Default for FabricModJson {
    fn default() -> Self {
        Self {
            schema_version: 1,
            id: String::new(),
            name: String::new(),
            version: String::new(),
            description: String::new(),
            authors: Vec::new(),
            contact: HashMap::new(),
            license: String::new(),
            icon: None,
            entrypoints: Entrypoints::default(),
            mixins: Vec::new(),
            depends: DependencyMap::new(),
            recommends: DependencyMap::new(),
            suggests: DependencyMap::new(),
            conflicts: DependencyMap::new(),
            breaks: Vec::new(),
            custom: serde_json::Value::Null,
        }
    }
}