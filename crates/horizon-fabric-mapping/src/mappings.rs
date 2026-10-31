//! Tiny mappings format parser and mapping layer.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Tiny mappings format (v2) - used by Fabric for Yarn mappings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TinyMappings {
    /// Format version (always 2 for Tiny v2).
    pub version: u32,
    /// Minecraft version (e.g., "26.3").
    pub minecraft_version: String,
    /// Mapping data.
    pub mappings: Vec<ClassMapping>,
}

/// A class mapping entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassMapping {
    /// Obfuscated name (e.g., "a.b.c").
    pub obf: String,
    /// Intermediary name (Yarn, e.g., "net.minecraft.class.ClassName").
    pub intermediary: String,
    /// Official Mojang name (if available).
    pub named: Option<String>,
    /// Field mappings.
    pub fields: Vec<FieldMapping>,
    /// Method mappings.
    pub methods: Vec<MethodMapping>,
}

/// A field mapping entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldMapping {
    /// Obfuscated name.
    pub obf: String,
    /// Intermediary name (Yarn).
    pub intermediary: String,
    /// Official name (if available).
    pub named: Option<String>,
    /// JVM descriptor (e.g., "I", "Lnet/minecraft/block/Block;").
    pub desc: String,
}

/// A method mapping entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodMapping {
    /// Obfuscated name.
    pub obf: String,
    /// Intermediary name (Yarn).
    pub intermediary: String,
    /// Official name (if available).
    pub named: Option<String>,
    /// JVM descriptor (e.g., "(III)V", "(Lnet/minecraft/world/World;)Z").
    pub desc: String,
}

/// Horizon internal name for a class, field, or method.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HorizonName {
    /// The Rust module path (e.g., "horizon_client::blocks::BlockFacts").
    pub rust_path: String,
    /// The Rust item name (e.g., "BlockFacts").
    pub rust_name: String,
    /// The kind of item.
    pub kind: HorizonNameKind,
}

/// Kind of Horizon name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HorizonNameKind {
    /// A Java class.
    Class,
    /// A Rust struct.
    Struct,
    /// A Rust enum.
    Enum,
    /// A field.
    Field,
    /// An instance method.
    Method,
    /// A constant.
    Const,
    /// A static method.
    StaticMethod,
}

/// The mapping layer that translates between Yarn and Horizon names.
#[derive(Debug)]
pub struct MappingLayer {
    /// Yarn intermediary -> Horizon name
    yarn_to_horizon: HashMap<String, HorizonName>,
    /// Horizon name -> Yarn intermediary
    horizon_to_yarn: HashMap<String, String>,
    /// Obfuscated -> Yarn intermediary
    obf_to_yarn: HashMap<String, String>,
    /// Class mappings by intermediary name
    class_by_intermediary: HashMap<String, ClassMapping>,
    /// Class mappings by obfuscated name
    class_by_obf: HashMap<String, ClassMapping>,
    /// Method signatures for verification
    method_signatures: HashMap<String, String>, // key: "ClassName.methodName", value: descriptor
}

impl MappingLayer {
    /// Create a new mapping layer from Yarn and Horizon mappings.
    pub fn new(yarn: crate::YarnMappings, horizon: crate::HorizonMappings) -> Self {
        let mut yarn_to_horizon = HashMap::new();
        let mut horizon_to_yarn = HashMap::new();
        let mut obf_to_yarn = HashMap::new();
        let mut class_by_intermediary = HashMap::new();
        let mut class_by_obf = HashMap::new();
        let mut method_signatures = HashMap::new();
        
        // Process class mappings
        for class in &yarn.mappings {
            class_by_intermediary.insert(class.intermediary.clone(), class.clone());
            class_by_obf.insert(class.obf.clone(), class.clone());
            obf_to_yarn.insert(class.obf.clone(), class.intermediary.clone());
            
            // Map class itself
            let horizon_class = horizon.find_class(&class.intermediary);
            if let Some(h) = horizon_class {
                let key = class.intermediary.clone();
                yarn_to_horizon.insert(key.clone(), h.clone());
                horizon_to_yarn.insert(format!("{}::{}", h.rust_path, h.rust_name), key);
            }
            
            // Process fields
            for field in &class.fields {
                let yarn_key = format!("{}/{}", class.intermediary, field.intermediary);
                obf_to_yarn.insert(format!("{}/{}", class.obf, field.obf), yarn_key.clone());
                
                let horizon_field = horizon.find_field(&class.intermediary, &field.intermediary);
                if let Some(h) = horizon_field {
                    yarn_to_horizon.insert(yarn_key.clone(), h.clone());
                    horizon_to_yarn.insert(format!("{}::{}", h.rust_path, h.rust_name), yarn_key);
                }
                
                // Store signature
                method_signatures.insert(
                    format!("{}.{}", class.intermediary, field.intermediary),
                    field.desc.clone(),
                );
            }
            
            // Process methods
            for method in &class.methods {
                let yarn_key = format!("{}/{}", class.intermediary, method.intermediary);
                obf_to_yarn.insert(format!("{}/{}", class.obf, method.obf), yarn_key.clone());
                
                let horizon_method = horizon.find_method(&class.intermediary, &method.intermediary, &method.desc);
                if let Some(h) = horizon_method {
                    yarn_to_horizon.insert(yarn_key.clone(), h.clone());
                    horizon_to_yarn.insert(format!("{}::{}", h.rust_path, h.rust_name), yarn_key);
                }
                
                // Store signature
                method_signatures.insert(
                    format!("{}.{}", class.intermediary, method.intermediary),
                    method.desc.clone(),
                );
            }
        }
        
        Self {
            yarn_to_horizon,
            horizon_to_yarn,
            obf_to_yarn,
            class_by_intermediary,
            class_by_obf,
            method_signatures,
        }
    }
    
    /// Resolve a Yarn class name to a Horizon name.
    pub fn resolve_class(&self, yarn_class: &str) -> Option<&HorizonName> {
        self.yarn_to_horizon.get(yarn_class)
    }

    /// The reverse lookup: Horizon `rust_path::rust_name` back to its Yarn key.
    pub fn horizon_to_yarn(&self, rust_path: &str, rust_name: &str) -> Option<&str> {
        self.horizon_to_yarn
            .get(&format!("{rust_path}::{rust_name}"))
            .map(String::as_str)
    }
    
    /// Resolve a Yarn field name to a Horizon name.
    pub fn resolve_field(&self, yarn_class: &str, yarn_field: &str) -> Option<&HorizonName> {
        let key = format!("{}/{}", yarn_class, yarn_field);
        self.yarn_to_horizon.get(&key)
    }
    
    /// Resolve a Yarn method name to a Horizon name.
    pub fn resolve_method(&self, yarn_class: &str, yarn_method: &str, yarn_desc: &str) -> Option<&HorizonName> {
        // First try exact match with descriptor
        let key_with_desc = format!("{}/{}#{}", yarn_class, yarn_method, yarn_desc);
        if let Some(h) = self.yarn_to_horizon.get(&key_with_desc) {
            return Some(h);
        }
        
        // Try without descriptor
        let key = format!("{}/{}", yarn_class, yarn_method);
        self.yarn_to_horizon.get(&key)
    }
    
    /// Resolve an obfuscated name to Yarn.
    pub fn obf_to_yarn(&self, obf_class: &str, obf_member: &str) -> Option<String> {
        let key = format!("{}/{}", obf_class, obf_member);
        self.obf_to_yarn.get(&key).cloned()
    }
    
    /// Get the JVM descriptor for a method.
    pub fn get_method_descriptor(&self, yarn_class: &str, yarn_method: &str) -> Option<&str> {
        self.method_signatures.get(&format!("{}.{}", yarn_class, yarn_method))
            .map(|s| s.as_str())
    }
    
    /// Get the JVM descriptor for a field.
    pub fn get_field_descriptor(&self, yarn_class: &str, yarn_field: &str) -> Option<&str> {
        self.method_signatures.get(&format!("{}.{}", yarn_class, yarn_field))
            .map(|s| s.as_str())
    }
    
    /// Get a class mapping by intermediary name.
    pub fn get_class(&self, intermediary: &str) -> Option<&ClassMapping> {
        self.class_by_intermediary.get(intermediary)
    }
    
    /// Get a class mapping by obfuscated name.
    pub fn get_class_obf(&self, obf: &str) -> Option<&ClassMapping> {
        self.class_by_obf.get(obf)
    }
    
    /// Iterate over all class mappings.
    pub fn classes(&self) -> impl Iterator<Item = &ClassMapping> {
        self.class_by_intermediary.values()
    }
    
    /// Get the number of mapped classes.
    pub fn class_count(&self) -> usize {
        self.class_by_intermediary.len()
    }
    
    /// Get the number of mapped fields.
    pub fn field_count(&self) -> usize {
        self.yarn_to_horizon.values().filter(|h| h.kind == HorizonNameKind::Field).count()
    }
    
    /// Get the number of mapped methods.
    pub fn method_count(&self) -> usize {
        self.yarn_to_horizon.values().filter(|h| matches!(h.kind, HorizonNameKind::Method | HorizonNameKind::StaticMethod)).count()
    }
}

/// A resolved mapping for a class member.
#[derive(Debug, Clone)]
pub struct ResolvedMapping {
    /// The Horizon name.
    pub horizon: HorizonName,
    /// The Yarn intermediary name.
    pub yarn: String,
    /// The JVM descriptor.
    pub descriptor: String,
    /// The obfuscated name (if available).
    pub obfuscated: Option<String>,
}

impl MappingLayer {
    /// Resolve a class member with full details.
    pub fn resolve_member(&self, yarn_class: &str, yarn_member: &str, descriptor: Option<&str>) -> Option<ResolvedMapping> {
        let yarn_key = format!("{}/{}", yarn_class, yarn_member);
        let horizon = self.yarn_to_horizon.get(&yarn_key)?;
        let obfuscated = self.obf_to_yarn.get(&yarn_key).cloned();
        let descriptor = descriptor.map(|s| s.to_string())
            .or_else(|| self.method_signatures.get(&format!("{}.{}", yarn_class, yarn_member)).cloned())
            .unwrap_or_default();
        
        Some(ResolvedMapping {
            horizon: horizon.clone(),
            yarn: yarn_key,
            descriptor,
            obfuscated,
        })
    }
}