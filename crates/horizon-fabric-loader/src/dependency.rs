//! Dependency resolution for Fabric mods.

use std::collections::{HashMap, VecDeque};
use semver::{Version, VersionReq};
use crate::{mod_json::ModJsonError, FabricMod};
use thiserror::Error;

/// A mod with resolved dependency information.
#[derive(Debug, Clone)]
pub struct ResolvedMod {
    /// The mod itself.
    pub mod_info: FabricMod,
    /// Mod IDs this mod depends on.
    pub dependencies: Vec<String>,
    /// Mod IDs that depend on this mod.
    pub dependents: Vec<String>,
    /// Dependencies that could not be satisfied.
    pub missing_dependencies: Vec<String>,
    /// Version conflicts as `(dep_id, required, actual)`.
    pub version_conflicts: Vec<(String, String, String)>,
}

/// Errors from dependency resolution.
#[derive(Debug, Error)]
pub enum ResolutionError {
    /// A required dependency is not present.
    #[error("Missing dependency: {mod_id} requires {dep_id}")]
    MissingDependency {
        /// The mod that declares the dependency.
        mod_id: String,
        /// The dependency that is missing.
        dep_id: String,
    },

    /// A dependency is present at the wrong version.
    #[error("Version conflict: {mod_id} requires {dep_id} {required}, but {actual} is loaded")]
    VersionConflict {
        /// The mod that declares the dependency.
        mod_id: String,
        /// The dependency.
        dep_id: String,
        /// The version requirement.
        required: String,
        /// The loaded version.
        actual: String,
    },

    /// The dependency graph has a cycle.
    #[error("Circular dependency detected: {0}")]
    CircularDependency(String),

    /// A version requirement could not be parsed.
    #[error("Invalid version requirement: {0}")]
    InvalidVersionReq(String),

    /// A `fabric.mod.json` could not be read.
    #[error("Mod JSON error: {0}")]
    ModJson(#[from] ModJsonError),
}

/// Dependency resolver for Fabric mods.
pub struct DependencyResolver {
    /// Whether to allow missing optional dependencies.
    allow_missing_optional: bool,
}

impl DependencyResolver {
    /// Create a new dependency resolver.
    pub fn new() -> Self {
        Self {
            allow_missing_optional: true,
        }
    }
    
    /// Set whether to allow missing optional dependencies.
    pub fn with_missing_optional(mut self, allow: bool) -> Self {
        self.allow_missing_optional = allow;
        self
    }
    
    /// Resolve dependencies for all mods.
    pub fn resolve(&self, mods: &HashMap<String, FabricMod>) -> Result<Vec<ResolvedMod>, ResolutionError> {
        let mut resolved = Vec::new();
        let mod_ids: Vec<_> = mods.keys().cloned().collect();
        
        // Add implicit dependencies
        for mod_info in mods.values() {
            for dep_id in mod_info.depends.keys() {
                if dep_id != "minecraft" && dep_id != "fabricloader" && dep_id != "fabric-api" {
                    if !mods.contains_key(dep_id) {
                        // Check if it's optional (in recommends/suggests)
                        let is_optional = mod_info.recommends.contains_key(dep_id) 
                            || mod_info.suggests.contains_key(dep_id);
                        
                        if !is_optional || !self.allow_missing_optional {
                            return Err(ResolutionError::MissingDependency {
                                mod_id: mod_info.id.clone(),
                                dep_id: dep_id.clone(),
                            });
                        }
                    }
                }
            }
        }
        
        // Build dependency graph
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        let mut reverse_graph: HashMap<String, Vec<String>> = HashMap::new();
        
        for mod_info in mods.values() {
            let mut deps = Vec::new();
            
            // Required dependencies
            for dep_id in mod_info.depends.keys() {
                if dep_id != "minecraft" && dep_id != "fabricloader" && dep_id != "fabric-api" {
                    deps.push(dep_id.clone());
                    reverse_graph.entry(dep_id.clone()).or_default().push(mod_info.id.clone());
                }
            }
            
            // Recommended dependencies (if present)
            for dep_id in mod_info.recommends.keys() {
                if mods.contains_key(dep_id) && !deps.contains(dep_id) {
                    deps.push(dep_id.clone());
                    reverse_graph.entry(dep_id.clone()).or_default().push(mod_info.id.clone());
                }
            }
            
            graph.insert(mod_info.id.clone(), deps);
        }
        
        // Topological sort (Kahn's algorithm) on the REVERSE graph:
        // if A depends on B, edge B -> A means B loads before A
        let load_order = self.topological_sort(&reverse_graph, &mod_ids)?;
        
        // Build resolved mods in load order
        let mut loaded_versions: HashMap<String, String> = HashMap::new();
        
        for mod_id in &load_order {
            let mod_info = mods.get(mod_id).unwrap();
            
            let mut dependencies = Vec::new();
            let mut missing = Vec::new();
            let mut conflicts = Vec::new();
            
            // Check required dependencies
            for (dep_id, req_str) in &mod_info.depends {
                if dep_id == "minecraft" || dep_id == "fabricloader" || dep_id == "fabric-api" {
                    continue;
                }
                
                if let Some(loaded_version) = loaded_versions.get(dep_id) {
                    let req = VersionReq::parse(req_str)
                        .map_err(|e| ResolutionError::InvalidVersionReq(format!("{}: {}", dep_id, e)))?;
                    
                    if !req.matches(&Version::parse(loaded_version).unwrap()) {
                        conflicts.push((dep_id.clone(), req_str.clone(), loaded_version.clone()));
                    }
                    dependencies.push(dep_id.clone());
                } else if mods.contains_key(dep_id) {
                    // Dependency not loaded yet - will be checked after sorting
                    missing.push(dep_id.clone());
                } else if !self.allow_missing_optional || !mod_info.recommends.contains_key(dep_id) {
                    return Err(ResolutionError::MissingDependency {
                        mod_id: mod_info.id.clone(),
                        dep_id: dep_id.clone(),
                    });
                }
            }
            
            // Check recommended dependencies
            for (dep_id, req_str) in &mod_info.recommends {
                if let Some(loaded_version) = loaded_versions.get(dep_id) {
                    let req = VersionReq::parse(req_str)
                        .map_err(|e| ResolutionError::InvalidVersionReq(format!("{}: {}", dep_id, e)))?;
                    
                    if !req.matches(&Version::parse(loaded_version).unwrap()) {
                        conflicts.push((dep_id.clone(), req_str.clone(), loaded_version.clone()));
                    }
                    if !dependencies.contains(dep_id) {
                        dependencies.push(dep_id.clone());
                    }
                }
            }
            
            let dependents = reverse_graph.get(mod_id).cloned().unwrap_or_default();
            
            resolved.push(ResolvedMod {
                mod_info: mod_info.clone(),
                dependencies,
                dependents,
                missing_dependencies: missing,
                version_conflicts: conflicts,
            });
            
            loaded_versions.insert(mod_info.id.clone(), mod_info.version.clone());
        }
        
        // Verify no missing dependencies remain
        for resolved_mod in &resolved {
            for missing in &resolved_mod.missing_dependencies {
                if !mods.contains_key(missing) {
                    return Err(ResolutionError::MissingDependency {
                        mod_id: resolved_mod.mod_info.id.clone(),
                        dep_id: missing.clone(),
                    });
                }
            }
        }
        
        Ok(resolved)
    }
    
    /// Perform topological sort on the dependency graph.
    fn topological_sort(
        &self,
        graph: &HashMap<String, Vec<String>>,
        mod_ids: &[String],
    ) -> Result<Vec<String>, ResolutionError> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        
        // Initialize
        for id in mod_ids {
            in_degree.insert(id.clone(), 0);
            adj.insert(id.clone(), Vec::new());
        }
        
        // Build adjacency and in-degree
        for (from, tos) in graph {
            for to in tos {
                if in_degree.contains_key(to) {
                    adj.entry(from.clone()).or_default().push(to.clone());
                    *in_degree.get_mut(to).unwrap() += 1;
                }
            }
        }
        
        // Kahn's algorithm
        let mut queue: VecDeque<String> = in_degree.iter()
            .filter(|&(_, &deg)| deg == 0)
            .map(|(id, _)| id.clone())
            .collect();
        
        let mut sorted = Vec::new();
        
        while let Some(id) = queue.pop_front() {
            sorted.push(id.clone());
            
            for neighbor in adj.get(&id).unwrap_or(&Vec::new()) {
                let deg = in_degree.get_mut(neighbor).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(neighbor.clone());
                }
            }
        }
        
        if sorted.len() != mod_ids.len() {
            // Circular dependency
            let remaining: Vec<_> = in_degree.iter()
                .filter(|(_, deg)| **deg > 0)
                .map(|(id, _)| id.clone())
                .collect();
            
            return Err(ResolutionError::CircularDependency(
                format!("Circular dependency involving: {}", remaining.join(", "))
            ));
        }
        
        Ok(sorted)
    }
}

impl Default for DependencyResolver {
    fn default() -> Self {
        Self::new()
    }
}