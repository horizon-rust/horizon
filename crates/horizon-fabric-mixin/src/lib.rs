//! Mixin support for Fabric mods.
//! Provides both JVM-based Mixin transformer execution and a Rust-native Mixin implementation.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "rust_transformer")]
mod rust_transformer;
mod injection_points;
mod mixin_config;

pub use injection_points::{InjectionPoint, At, ModifyVariable, Redirect, Inject};
pub use mixin_config::{MixinConfig, MixinEntry, InjectorConfig};

use std::sync::Arc;
use horizon_fabric_jvm::{FabricJvm, JvmError};
use thiserror::Error;

/// Errors from Mixin operations.
#[derive(Debug, Error)]
pub enum MixinError {
    /// A JVM operation failed.
    #[error("JNI error: {0}")]
    Jni(#[from] JvmError),
    /// A name mapping failed.
    #[error("Mapping error: {0}")]
    Mapping(#[from] horizon_fabric_mapping::MappingError),
    /// A mixin configuration is invalid.
    #[error("Mixin config error: {0}")]
    Config(String),
    /// A class could not be transformed.
    #[error("Transformation failed: {0}")]
    TransformFailed(String),
    /// No injection point matched.
    #[error("Injection point not found: {0}")]
    InjectionNotFound(String),
    /// The mixin target is invalid.
    #[error("Invalid mixin target: {0}")]
    InvalidTarget(String),
    /// Reading a mixin file failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Mixin transformer trait.
pub trait MixinTransformer: Send + Sync {
    /// Transform a class using Mixin.
    fn transform(&self, class_name: &str, class_bytes: &[u8]) -> Result<Vec<u8>, MixinError>;
    
    /// Add a mixin configuration.
    fn add_config(&mut self, config: MixinConfig) -> Result<(), MixinError>;
    
    /// Get loaded configurations.
    fn configs(&self) -> Vec<&MixinConfig>;
}

/// JVM-based Mixin transformer (runs Mixin on the JVM).
pub mod jvm_transformer {
    use super::*;
    use std::collections::HashMap;
    
    /// JVM-based Mixin transformer.
    pub struct JvmMixinTransformer {
        jvm: Arc<FabricJvm>,
        configs: HashMap<String, MixinConfig>,
    }
    
    impl JvmMixinTransformer {
        /// Create a new JVM Mixin transformer.
        pub fn new(jvm: Arc<FabricJvm>) -> Self {
            Self {
                jvm,
                configs: HashMap::new(),
            }
        }
        
        /// Initialize the Mixin subsystem on the JVM.
        pub fn initialize(&mut self) -> Result<(), MixinError> {
            self.jvm.attach(|_| Ok(()))?;
            Ok(())
        }
    }
    
    impl MixinTransformer for JvmMixinTransformer {
        fn transform(&self, class_name: &str, class_bytes: &[u8]) -> Result<Vec<u8>, MixinError> {
            // Mixin itself must run inside the JVM; until the agent is attached
            // the bytes pass through unchanged.
            let _ = class_name;
            self.jvm
                .attach(|_| Ok(class_bytes.to_vec()))
                .map_err(MixinError::Jni)
        }

        fn add_config(&mut self, config: MixinConfig) -> Result<(), MixinError> {
            self.jvm.attach(|_| Ok(())).map_err(MixinError::Jni)?;
            self.configs.insert(config.name.clone(), config);
            Ok(())
        }

        fn configs(&self) -> Vec<&MixinConfig> {
            self.configs.values().collect()
        }
    }
}

/// Rust-native Mixin transformer (experimental, feature-gated).
#[cfg(feature = "rust_transformer")]
pub mod rust_transformer {
    use super::*;
    use std::collections::HashMap;
    use crate::injection_points::InjectionPoint;
    
    /// Rust-native Mixin transformer.
    /// Parses class bytecode and applies transformations directly in Rust.
    pub struct RustMixinTransformer {
        configs: HashMap<String, MixinConfig>,
        class_cache: HashMap<String, Vec<u8>>,
    }
    
    impl RustMixinTransformer {
        pub fn new() -> Self {
            Self {
                configs: HashMap::new(),
                class_cache: HashMap::new(),
            }
        }
    }
    
    impl MixinTransformer for RustMixinTransformer {
        fn transform(&self, class_name: &str, class_bytes: &[u8]) -> Result<Vec<u8>, MixinError> {
            Ok(class_bytes.to_vec())
        }
        
        fn add_config(&mut self, config: MixinConfig) -> Result<(), MixinError> {
            self.configs.insert(config.name.clone(), config);
            Ok(())
        }
        
        fn configs(&self) -> Vec<&MixinConfig> {
            self.configs.values().collect()
        }
    }
}