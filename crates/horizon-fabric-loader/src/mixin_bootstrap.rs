//! Mixin bootstrap for Fabric mods.
//! Initializes the Mixin transformer and applies mixin configurations.

use horizon_fabric_jvm::{FabricJvm, JvmError};
use thiserror::Error;

/// Errors from Mixin bootstrap.
#[derive(Debug, Error)]
pub enum MixinError {
    /// A JVM operation failed.
    #[error("JNI error: {0}")]
    Jni(#[from] JvmError),

    /// A Mixin class could not be loaded.
    #[error("Mixin class not found: {0}")]
    ClassNotFound(String),

    /// The Mixin subsystem could not start.
    #[error("Mixin initialization failed: {0}")]
    InitializationFailed(String),

    /// A Mixin configuration file was missing.
    #[error("Mixin config not found: {0}")]
    ConfigNotFound(String),

    /// Applying a Mixin failed.
    #[error("Mixin apply failed: {0}")]
    ApplyFailed(String),
}

/// Mixin bootstrap - initializes Mixin subsystem.
pub struct MixinBootstrap {
    /// Whether Mixin has been initialized.
    initialized: bool,
    /// Loaded mixin configuration names.
    configs: Vec<String>,
    /// Parsed configurations.
    parsed: Vec<config::MixinConfig>,
}

impl MixinBootstrap {
    /// Create a new Mixin bootstrap.
    pub fn new() -> Self {
        Self {
            initialized: false,
            configs: Vec::new(),
            parsed: Vec::new(),
        }
    }
    
    /// Initialize the Mixin subsystem.
    pub fn initialize(&mut self, jvm: &FabricJvm) -> Result<(), MixinError> {
        if self.initialized {
            return Ok(());
        }
        
        jvm.attach(|_| {
            // In a real implementation, this would do JNI operations
            // For now, just simulate success
            Ok(())
        })?;
        
        self.initialized = true;
        Ok(())
    }
    
    /// Add a mixin configuration.
    pub fn add_config(&mut self, config_name: &str) {
        if !self.configs.contains(&config_name.to_string()) {
            self.configs.push(config_name.to_string());
        }
    }

    /// Read a mixin configuration out of a mod jar and register it.
    ///
    /// `config_name` is the file name listed under `mixins` in
    /// `fabric.mod.json` (for example `mymod.mixins.json`).
    pub fn load_config_from_jar(
        &mut self,
        jar_path: &std::path::Path,
        config_name: &str,
    ) -> Result<config::MixinConfig, MixinError> {
        use std::io::Read;

        let file = std::fs::File::open(jar_path)
            .map_err(|_| MixinError::ConfigNotFound(config_name.to_string()))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| MixinError::InitializationFailed(e.to_string()))?;
        let mut entry = archive
            .by_name(config_name)
            .map_err(|_| MixinError::ConfigNotFound(config_name.to_string()))?;

        let mut text = String::new();
        entry
            .read_to_string(&mut text)
            .map_err(|e| MixinError::InitializationFailed(e.to_string()))?;

        let parsed: config::MixinConfig = serde_json::from_str(&text)
            .map_err(|e| MixinError::InitializationFailed(format!("{config_name}: {e}")))?;

        self.add_config(config_name);
        self.parsed.push(parsed.clone());
        Ok(parsed)
    }

    /// Every configuration parsed so far.
    pub fn parsed_configs(&self) -> &[config::MixinConfig] {
        &self.parsed
    }
    
    /// Get loaded configurations.
    pub fn configs(&self) -> &[String] {
        &self.configs
    }
    
    /// Check if initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
    
    /// Transform a class using Mixin.
    /// This is called by the class loader when loading mod classes.
    pub fn transform_class(
        &self,
        jvm: &FabricJvm,
        _class_name: &str,
        class_bytes: &[u8],
    ) -> Result<Vec<u8>, MixinError> {
        if !self.initialized {
            return Ok(class_bytes.to_vec());
        }
        
        jvm.attach(|_| Ok(class_bytes.to_vec())).map_err(MixinError::Jni)
    }
}

impl Default for MixinBootstrap {
    fn default() -> Self {
        Self::new()
    }
}

/// Mixin configuration parser.
///
/// This mirrors the real `mixins.<mod>.json` schema Fabric mods ship, so a
/// config can be read straight out of a mod jar.
pub mod config {
    use std::collections::HashMap;
    use serde::{Deserialize, Serialize};

    /// Mixin configuration (`mixins.modname.json`).
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct MixinConfig {
        /// Whether the mod requires Mixin to load.
        #[serde(default = "default_required")]
        pub required: bool,

        /// Package the mixin class names are relative to.
        #[serde(default)]
        pub package: String,

        /// Compatibility level (`JAVA_21`, `JAVA_17`, ...).
        #[serde(default, rename = "compatibilityLevel", alias = "compatibility")]
        pub compatibility: String,

        /// Mixin classes applied on both sides.
        #[serde(default)]
        pub mixins: Vec<String>,

        /// Injector defaults.
        #[serde(default)]
        pub injectors: HashMap<String, serde_json::Value>,

        /// Client-only mixins.
        #[serde(default)]
        pub client: Vec<String>,

        /// Server-only mixins.
        #[serde(default)]
        pub server: Vec<String>,

        /// Mixins that must be applied before others.
        #[serde(default)]
        pub priority: i32,
    }

    impl MixinConfig {
        /// Every mixin class this config declares, fully qualified.
        pub fn mixin_classes(&self) -> Vec<String> {
            self.mixins
                .iter()
                .chain(self.client.iter())
                .chain(self.server.iter())
                .map(|name| {
                    if self.package.is_empty() {
                        name.clone()
                    } else {
                        format!("{}.{}", self.package, name)
                    }
                })
                .collect()
        }
    }

    fn default_required() -> bool {
        true
    }
}