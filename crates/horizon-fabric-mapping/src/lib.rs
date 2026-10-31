//! Yarn ↔ Horizon mapping layer for Fabric mod compatibility.
//! Provides runtime translation between Fabric's Yarn mappings and Horizon's internal names.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod mappings;
mod yarn_mappings;
mod horizon_mappings;

pub use mappings::{TinyMappings, ClassMapping, FieldMapping, MethodMapping, MappingLayer, HorizonName, HorizonNameKind};
pub use yarn_mappings::YarnMappings;
pub use horizon_mappings::HorizonMappings;

use std::sync::Arc;
use once_cell::sync::Lazy;
use parking_lot::RwLock;

/// Global mapping layer instance.
static MAPPING_LAYER: Lazy<RwLock<Option<Arc<MappingLayer>>>> = Lazy::new(|| RwLock::new(None));

/// Initialize the global mapping layer with Yarn mappings for the given version.
pub fn initialize_global(version: &str) -> Result<(), MappingError> {
    let yarn = YarnMappings::load_for_version(version)?;
    let horizon = HorizonMappings::load()?;
    let layer = MappingLayer::new(yarn, horizon);
    
    let mut global = MAPPING_LAYER.write();
    *global = Some(Arc::new(layer));
    Ok(())
}

/// Get the global mapping layer.
pub fn global() -> Result<Arc<MappingLayer>, MappingError> {
    MAPPING_LAYER.read()
        .as_ref()
        .cloned()
        .ok_or(MappingError::NotInitialized)
}

/// Errors from mapping operations.
#[derive(Debug, thiserror::Error)]
pub enum MappingError {
    /// The global mapping layer has not been initialized.
    #[error("Mappings not initialized")]
    NotInitialized,
    /// The mapping files could not be read or parsed.
    #[error("Failed to load mappings: {0}")]
    LoadFailed(String),
    /// No mapping exists for the requested name.
    #[error("Mapping not found: {0}")]
    NotFound(String),
    /// Several mappings match the requested name.
    #[error("Ambiguous mapping: {0}")]
    Ambiguous(String),
    /// Reading a mapping file failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// Parsing a mapping file failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}