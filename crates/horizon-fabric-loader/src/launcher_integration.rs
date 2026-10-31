//! Launcher integration for Fabric mod management.

use std::path::PathBuf;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::FabricMod;

/// Launcher integration for Fabric mods.
pub struct LauncherIntegration {
    /// Fabric profiles.
    profiles: HashMap<String, FabricProfile>,
    /// Active profile.
    active_profile: Option<String>,
}

impl LauncherIntegration {
    /// Create a new launcher integration.
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
            active_profile: None,
        }
    }
    
    /// Create a new Fabric profile.
    pub fn create_profile(&mut self, name: &str, minecraft_version: &str) -> FabricProfile {
        let profile = FabricProfile {
            name: name.to_string(),
            minecraft_version: minecraft_version.to_string(),
            fabric_loader_version: Self::get_fabric_loader_version(minecraft_version),
            fabric_api_version: Self::get_fabric_api_version(minecraft_version),
            mods: Vec::new(),
            jvm_args: Vec::new(),
            created_at: chrono::Utc::now(),
        };
        
        self.profiles.insert(name.to_string(), profile.clone());
        profile
    }
    
    /// Get a profile by name.
    pub fn get_profile(&self, name: &str) -> Option<&FabricProfile> {
        self.profiles.get(name)
    }
    
    /// Get mutable profile.
    pub fn get_profile_mut(&mut self, name: &str) -> Option<&mut FabricProfile> {
        self.profiles.get_mut(name)
    }
    
    /// Set active profile.
    pub fn set_active(&mut self, name: &str) -> bool {
        if self.profiles.contains_key(name) {
            self.active_profile = Some(name.to_string());
            true
        } else {
            false
        }
    }
    
    /// Get active profile.
    pub fn active_profile(&self) -> Option<&FabricProfile> {
        self.active_profile.as_ref().and_then(|n| self.profiles.get(n))
    }
    
    /// Add a mod to a profile.
    pub fn add_mod(&mut self, profile_name: &str, mod_info: FabricMod) -> bool {
        if let Some(profile) = self.profiles.get_mut(profile_name) {
            // Check for conflicts
            if !profile.mods.iter().any(|m| m.id == mod_info.id) {
                profile.mods.push(mod_info);
                return true;
            }
        }
        false
    }
    
    /// Remove a mod from a profile.
    pub fn remove_mod(&mut self, profile_name: &str, mod_id: &str) -> bool {
        if let Some(profile) = self.profiles.get_mut(profile_name) {
            let len = profile.mods.len();
            profile.mods.retain(|m| m.id != mod_id);
            profile.mods.len() != len
        } else {
            false
        }
    }
    
    /// Get all profiles.
    pub fn profiles(&self) -> &HashMap<String, FabricProfile> {
        &self.profiles
    }
    
    /// Get Fabric Loader version for Minecraft version.
    fn get_fabric_loader_version(minecraft_version: &str) -> String {
        // In production, this would query Fabric's meta API
        match minecraft_version {
            "26.3" => "0.16.0".into(),
            _ => "0.16.0".into(),
        }
    }
    
    /// Get Fabric API version for Minecraft version.
    fn get_fabric_api_version(minecraft_version: &str) -> String {
        match minecraft_version {
            "26.3" => "0.100.0".into(),
            _ => "0.100.0".into(),
        }
    }
}

/// A Fabric mod profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricProfile {
    pub name: String,
    pub minecraft_version: String,
    pub fabric_loader_version: String,
    pub fabric_api_version: String,
    pub mods: Vec<FabricMod>,
    pub jvm_args: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl FabricProfile {
    /// Get the profile directory name.
    pub fn dir_name(&self) -> String {
        format!("fabric-{}", self.name)
    }
    
    /// Get the mods directory for this profile.
    pub fn mods_dir(&self, base: &PathBuf) -> PathBuf {
        base.join("profiles").join(self.dir_name()).join("mods")
    }
    
    /// Get the Fabric Loader installation directory.
    pub fn loader_dir(&self, base: &PathBuf) -> PathBuf {
        base.join("fabric-loader").join(self.fabric_loader_version.clone())
    }
    
    /// Get the JVM arguments for this profile.
    pub fn jvm_args(&self) -> &[String] {
        &self.jvm_args
    }
    
    /// Add a JVM argument.
    pub fn add_jvm_arg(&mut self, arg: String) {
        self.jvm_args.push(arg);
    }
}

/// Fabric mod metadata for the launcher UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricModMetadata {
    /// Mod ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Mod version.
    pub version: String,
    /// Description.
    pub description: String,
    /// Authors.
    pub authors: Vec<String>,
    /// Icon path inside the jar.
    pub icon: Option<String>,
    /// Declared dependencies.
    pub dependencies: Vec<FabricDependency>,
    /// Whether this jar is Fabric API itself.
    pub is_fabric_api: bool,
}

impl FabricModMetadata {
    /// Build launcher metadata from a discovered mod.
    pub fn from_mod(mod_info: &crate::FabricMod) -> Self {
        let mut dependencies: Vec<FabricDependency> = mod_info
            .depends
            .iter()
            .map(|(mod_id, version_range)| FabricDependency {
                mod_id: mod_id.clone(),
                version_range: version_range.clone(),
                is_required: true,
                is_recommended: false,
            })
            .collect();
        dependencies.extend(
            mod_info
                .recommends
                .iter()
                .map(|(mod_id, version_range)| FabricDependency {
                    mod_id: mod_id.clone(),
                    version_range: version_range.clone(),
                    is_required: false,
                    is_recommended: true,
                }),
        );

        Self {
            id: mod_info.id.clone(),
            name: mod_info.name.clone(),
            version: mod_info.version.clone(),
            description: mod_info.description.clone(),
            authors: mod_info.authors.clone(),
            icon: mod_info.icon.clone(),
            dependencies,
            is_fabric_api: mod_info.id == "fabric-api",
        }
    }
}

/// A Fabric mod dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricDependency {
    /// The depended-on mod ID.
    pub mod_id: String,
    /// Accepted version range.
    pub version_range: String,
    /// Whether the mod cannot start without it.
    pub is_required: bool,
    /// Whether it is only recommended.
    pub is_recommended: bool,
}

impl Default for LauncherIntegration {
    fn default() -> Self {
        Self::new()
    }
}