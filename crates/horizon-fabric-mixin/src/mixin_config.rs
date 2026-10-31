//! Mixin configuration parsing.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// A Mixin configuration (mixins.modname.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixinConfig {
    /// Required Mixin version.
    #[serde(default = "default_required")]
    pub required: bool,
    
    /// Package for generated classes.
    #[serde(default)]
    pub package: String,
    
    /// Compatibility level.
    #[serde(default = "default_compatibility")]
    pub compatibility: String,
    
    /// Minimum version.
    #[serde(default)]
    pub min_version: String,
    
    /// Mixin classes.
    pub mixins: Vec<MixinEntry>,
    
    /// Injectors configuration.
    #[serde(default)]
    pub injectors: HashMap<String, InjectorConfig>,
    
    /// Client-only mixins.
    #[serde(default)]
    pub client: Vec<String>,
    
    /// Server-only mixins.
    #[serde(default)]
    pub server: Vec<String>,
    
    /// Mixin priority.
    #[serde(default)]
    pub priority: i32,
    
    /// Config name (derived from filename).
    #[serde(skip)]
    pub name: String,
}

fn default_required() -> bool {
    true
}

fn default_compatibility() -> String {
    "DEFAULT".into()
}

/// A single mixin entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixinEntry {
    /// Target class (intermediary name).
    pub target: String,
    
    /// Mixin class (intermediary name).
    pub mixin: String,
    
    /// Priority.
    #[serde(default)]
    pub priority: i32,
    
    /// Whether this mixin is required.
    #[serde(default = "default_required")]
    pub required: bool,
    
    /// Injectors specific to this mixin.
    #[serde(default)]
    pub injectors: HashMap<String, InjectorConfig>,
    
    /// Accessors.
    #[serde(default)]
    pub accessors: HashMap<String, AccessorConfig>,
    
    /// Shadow fields/methods.
    #[serde(default)]
    pub shadows: HashMap<String, ShadowConfig>,
}

/// Accessor configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessorConfig {
    /// Target field/method.
    pub target: String,
    /// Accessor type (field, method, static).
    #[serde(rename = "type")]
    pub accessor_type: AccessorType,
    /// Expected type.
    pub expected_type: Option<String>,
}

/// Accessor type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessorType {
    Field,
    Method,
    Static,
}

/// Shadow configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowConfig {
    /// Target field/method.
    pub target: String,
    /// Shadow type (field, method).
    #[serde(rename = "type")]
    pub shadow_type: ShadowType,
    /// Expected type.
    pub expected_type: Option<String>,
}

/// Shadow type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShadowType {
    Field,
    Method,
    New,
}

/// Injector configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectorConfig {
    /// Default injector options.
    #[serde(default)]
    pub defaults: HashMap<String, serde_json::Value>,
    
    /// Injector priority.
    #[serde(default)]
    pub priority: i32,
}

impl Default for InjectorConfig {
    fn default() -> Self {
        Self {
            defaults: HashMap::new(),
            priority: 1000,
        }
    }
}

impl MixinConfig {
    /// Load a MixinConfig from a JSON file.
    pub fn load_from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self, crate::MixinError> {
        let path_ref = path.as_ref();
        let content = std::fs::read_to_string(path_ref)?;
        let mut config: Self = serde_json::from_str(&content)
            .map_err(|e| crate::MixinError::Config(format!("Failed to parse mixin config: {}", e)))?;
        
        // Extract name from filename
        config.name = path_ref
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .trim_start_matches("mixins.")
            .to_string();
        
        Ok(config)
    }
    
    /// Get all mixin classes.
    pub fn mixin_classes(&self) -> Vec<String> {
        self.mixins.iter().map(|m| m.mixin.clone()).collect()
    }
    
    /// Get all target classes.
    pub fn target_classes(&self) -> Vec<String> {
        self.mixins.iter().map(|m| m.target.clone()).collect()
    }
    
    /// Check if this config applies to the given environment.
    pub fn applies_to(&self, client: bool, server: bool) -> bool {
        if !self.client.is_empty() && !client {
            return false;
        }
        if !self.server.is_empty() && !server {
            return false;
        }
        true
    }
}