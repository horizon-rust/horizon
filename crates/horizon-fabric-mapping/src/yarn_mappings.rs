//! Yarn mappings loader for Fabric.

use std::path::Path;
use std::collections::HashMap;
use crate::{TinyMappings, MappingError};

// Embedded Yarn mappings for Minecraft 26.3
// In production, these would be downloaded by horizon-prepare
const YARN_26_3_JSON: &str = include_str!("../data/yarn-26.3.tiny.json");

/// Yarn mappings for a specific Minecraft version.
pub struct YarnMappings {
    version: String,
    mappings: TinyMappings,
    /// Fast lookup: intermediary name -> class mapping
    class_index: HashMap<String, usize>,
}

impl YarnMappings {
    /// Load Yarn mappings for the given Minecraft version.
    pub fn load_for_version(version: &str) -> Result<Self, MappingError> {
        match version {
            "26.3" => Self::load_embedded(),
            _ => Err(MappingError::LoadFailed(format!("Unsupported version: {}", version))),
        }
    }
    
    /// Load the embedded 26.3 mappings.
    fn load_embedded() -> Result<Self, MappingError> {
        let mappings: TinyMappings = serde_json::from_str(YARN_26_3_JSON)
            .map_err(|e| MappingError::LoadFailed(format!("Failed to parse embedded mappings: {}", e)))?;
        
        let mut class_index = HashMap::new();
        for (i, class) in mappings.mappings.iter().enumerate() {
            class_index.insert(class.intermediary.clone(), i);
        }
        
        Ok(Self {
            version: "26.3".into(),
            mappings,
            class_index,
        })
    }
    
    /// Load Yarn mappings from a file (for development/custom versions).
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, MappingError> {
        let content = std::fs::read_to_string(path)?;
        let mappings: TinyMappings = serde_json::from_str(&content)
            .map_err(|e| MappingError::LoadFailed(format!("Failed to parse mappings file: {}", e)))?;
        
        let mut class_index = HashMap::new();
        for (i, class) in mappings.mappings.iter().enumerate() {
            class_index.insert(class.intermediary.clone(), i);
        }
        
        Ok(Self {
            version: mappings.minecraft_version.clone(),
            mappings,
            class_index,
        })
    }
    
    /// Get the Minecraft version.
    pub fn version(&self) -> &str {
        &self.version
    }
    
    /// Get the raw TinyMappings.
    pub fn mappings(&self) -> &TinyMappings {
        &self.mappings
    }
    
    /// Find a class mapping by intermediary name.
    pub fn get_class(&self, intermediary: &str) -> Option<&crate::ClassMapping> {
        self.class_index.get(intermediary).map(|&i| &self.mappings.mappings[i])
    }
    
    /// Find a class mapping by obfuscated name.
    pub fn get_class_obf(&self, obf: &str) -> Option<&crate::ClassMapping> {
        self.mappings.mappings.iter().find(|c| c.obf == obf)
    }
    
    /// Iterate over all class mappings.
    pub fn classes(&self) -> impl Iterator<Item = &crate::ClassMapping> {
        self.mappings.mappings.iter()
    }
    
    /// Get the number of mapped classes.
    pub fn class_count(&self) -> usize {
        self.mappings.mappings.len()
    }
}

impl std::ops::Deref for YarnMappings {
    type Target = TinyMappings;
    
    fn deref(&self) -> &Self::Target {
        &self.mappings
    }
}