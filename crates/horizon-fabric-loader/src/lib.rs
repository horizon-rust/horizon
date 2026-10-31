//! Fabric Loader reimplementation in Rust.
//! Handles mod discovery, dependency resolution, and entrypoint invocation.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod mod_json;
pub mod dependency;
mod entrypoint;
mod mixin_bootstrap;
mod launcher_integration;

pub use mod_json::{FabricMod, FabricModJson, Entrypoints, EntrypointType, DependencyMap, ModContact, ModLicense};
pub use dependency::{DependencyResolver, ResolutionError};
pub use entrypoint::{ClientModInitializer, DedicatedServerModInitializer, EntrypointInvoker, LoadPhase, ModInitializer};
pub use mixin_bootstrap::{MixinBootstrap, MixinError};
pub use launcher_integration::{FabricDependency, FabricModMetadata, LauncherIntegration};

use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use horizon_fabric_jvm::{FabricJvm, JvmError};
use horizon_fabric_jni::JniBridge;
use crate::dependency::ResolvedMod;
use thiserror::Error;

/// Errors from the Fabric loader.
#[derive(Debug, Error)]
pub enum LoaderError {
    /// The embedded JVM failed.
    #[error("JVM error: {0}")]
    Jvm(#[from] JvmError),
    /// A `fabric.mod.json` was invalid.
    #[error("Mod JSON error: {0}")]
    ModJson(#[from] crate::mod_json::ModJsonError),
    /// Dependency resolution failed.
    #[error("Dependency resolution error: {0}")]
    Dependency(#[from] ResolutionError),
    /// An entrypoint threw or could not be invoked.
    #[error("Entrypoint invocation error: {0}")]
    Entrypoint(#[from] crate::entrypoint::EntrypointError),
    /// A JNI call failed.
    #[error("JNI bridge error: {0}")]
    Jni(#[from] horizon_fabric_jni::JniError),
    /// The Mixin subsystem failed.
    #[error("Mixin bootstrap error: {0}")]
    Mixin(#[from] crate::mixin_bootstrap::MixinError),
    /// Reading a mod jar failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// A mod jar is not a readable zip archive.
    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// A mod ID was requested that was not discovered.
    #[error("Mod not found: {0}")]
    ModNotFound(String),
    /// Two jars declare the same mod ID.
    #[error("Duplicate mod ID: {0}")]
    DuplicateModId(String),
    /// A jar is not a Fabric mod.
    #[error("Invalid mod JAR: {0}")]
    InvalidJar(String),
}

/// The Fabric Loader.
pub struct FabricLoader {
    /// The JVM instance.
    jvm: Arc<FabricJvm>,
    /// The JNI bridge for Fabric mod interaction.
    jni_bridge: JniBridge,
    /// Discovered mods, indexed by mod ID.
    mods: HashMap<String, FabricMod>,
    /// Load order (topologically sorted by dependencies).
    load_order: Vec<String>,
    /// Resolved mods with dependency information.
    resolved: Vec<ResolvedMod>,
    /// Whether the loader has been initialized.
    initialized: bool,
    /// Mixin bootstrap.
    mixin_bootstrap: MixinBootstrap,
    /// Launcher integration.
    launcher: LauncherIntegration,
}

impl FabricLoader {
    /// Create a new Fabric Loader.
    pub fn new(jvm: Arc<FabricJvm>) -> Result<Self, LoaderError> {
        let jni_bridge = JniBridge::for_jvm(jvm.clone())?;

        Ok(Self {
            jvm,
            jni_bridge,
            mods: HashMap::new(),
            load_order: Vec::new(),
            resolved: Vec::new(),
            initialized: false,
            mixin_bootstrap: MixinBootstrap::new(),
            launcher: LauncherIntegration::new(),
        })
    }
    
    /// Discover mods in the given directory.
    pub fn discover_mods(&mut self, mods_dir: &Path) -> Result<usize, LoaderError> {
        let mut count = 0;
        
        if !mods_dir.exists() {
            return Ok(0);
        }
        
        for entry in std::fs::read_dir(mods_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension().map_or(false, |e| e == "jar") {
                match self.read_mod_jar(&path) {
                    Ok(mod_info) => {
                        let mod_id = mod_info.id.clone();
                        if self.mods.contains_key(&mod_id) {
                            return Err(LoaderError::DuplicateModId(mod_id));
                        }
                        self.mods.insert(mod_id, mod_info);
                        count += 1;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to read mod JAR {}: {}", path.display(), e);
                    }
                }
            }
        }
        
        Ok(count)
    }
    
    /// Read a mod JAR and extract its fabric.mod.json.
    fn read_mod_jar(&self, jar_path: &Path) -> Result<FabricMod, LoaderError> {
        use zip::ZipArchive;
        use std::io::Read;
        
        let file = std::fs::File::open(jar_path)?;
        let mut archive = ZipArchive::new(file)?;
        
        // Find fabric.mod.json
        let mut json_data = String::new();
        let mut found = false;
        
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            if file.name() == "fabric.mod.json" {
                file.read_to_string(&mut json_data)?;
                found = true;
                break;
            }
        }
        
        if !found {
            return Err(LoaderError::InvalidJar("No fabric.mod.json found".into()));
        }
        
        let mod_json: FabricModJson = serde_json::from_str(&json_data)
            .map_err(|e| LoaderError::ModJson(crate::mod_json::ModJsonError::Parse(e)))?;
        
        Ok(FabricMod {
            id: mod_json.id.clone(),
            name: mod_json.name,
            version: mod_json.version,
            description: mod_json.description,
            authors: mod_json.authors,
            contact: mod_json.contact,
            license: mod_json.license,
            icon: mod_json.icon,
            entrypoints: mod_json.entrypoints,
            mixins: mod_json.mixins,
            depends: mod_json.depends,
            recommends: mod_json.recommends,
            suggests: mod_json.suggests,
            conflicts: mod_json.conflicts,
            breaks: mod_json.breaks,
            custom: mod_json.custom,
            jar_path: jar_path.to_path_buf(),
        })
    }
    
    /// Resolve dependencies and compute load order.
    pub fn resolve(&mut self) -> Result<(), LoaderError> {
        let resolver = DependencyResolver::new();
        self.resolved = resolver.resolve(&self.mods)?;

        // Extract load order
        self.load_order = self.resolved.iter().map(|m| m.mod_info.id.clone()).collect();

        // Mod jars are only known now: put them on the Fabric class path before
        // any entrypoint class is touched.
        let jars: Vec<PathBuf> = self
            .load_order
            .iter()
            .filter_map(|id| self.mods.get(id))
            .map(|mod_info| mod_info.jar_path.clone())
            .collect();
        self.jvm.add_classpath(&jars)?;

        Ok(())
    }
    
    /// Initialize all mods for the given phase.
    pub fn initialize(&mut self, phase: LoadPhase) -> Result<(), LoaderError> {
        if self.load_order.is_empty() {
            self.resolve()?;
        }
        
        // Initialize Mixin bootstrap first
        if phase == LoadPhase::PreLaunch {
            self.mixin_bootstrap.initialize(&self.jvm)?;
            self.load_mixin_configs()?;
        }
        
        // Initialize mods in load order
        for mod_id in &self.load_order {
            if let Some(mod_info) = self.mods.get(mod_id) {
                self.initialize_mod(mod_info, phase)?;
            }
        }
        
        self.initialized = true;
        Ok(())
    }
    
    /// Read every mixin configuration declared by the discovered mods.
    fn load_mixin_configs(&mut self) -> Result<(), LoaderError> {
        let mods: Vec<(PathBuf, Vec<String>)> = self
            .mods
            .values()
            .map(|mod_info| {
                (
                    mod_info.jar_path.clone(),
                    mod_info.mixins.clone(),
                )
            })
            .collect();

        for (jar_path, configs) in mods {
            for config_name in configs {
                self.mixin_bootstrap
                    .load_config_from_jar(&jar_path, &config_name)?;
            }
        }

        Ok(())
    }

    /// Initialize a single mod for the given phase.
    ///
    /// `Launch` runs the `main` entrypoints (both sides). `ClientLaunch` runs
    /// only the `client` ones and `ServerLaunch` only the `server` ones, so a
    /// client boot is `Launch` followed by `ClientLaunch` and nothing runs
    /// twice.
    fn initialize_mod(&self, mod_info: &FabricMod, phase: LoadPhase) -> Result<(), LoaderError> {
        let invoker = EntrypointInvoker::new(self.jni_bridge.clone());

        match phase {
            LoadPhase::PreLaunch => {}
            LoadPhase::Launch => {
                for entrypoint in &mod_info.entrypoints.main {
                    invoker.invoke_main(entrypoint)?;
                }
            }
            LoadPhase::ClientLaunch => {
                for entrypoint in &mod_info.entrypoints.client {
                    invoker.invoke_client(entrypoint)?;
                }
            }
            LoadPhase::ServerLaunch => {
                for entrypoint in &mod_info.entrypoints.server {
                    invoker.invoke_server(entrypoint)?;
                }
            }
        }

        Ok(())
    }

    /// Get a mod by ID.
    pub fn get_mod(&self, mod_id: &str) -> Option<&FabricMod> {
        self.mods.get(mod_id)
    }
    
    /// Get all loaded mods.
    pub fn mods(&self) -> &HashMap<String, FabricMod> {
        &self.mods
    }
    
    /// Get the load order.
    pub fn load_order(&self) -> &[String] {
        &self.load_order
    }
    
    /// Get the JVM instance.
    pub fn jvm(&self) -> &Arc<FabricJvm> {
        &self.jvm
    }
    
    /// Get the Mixin bootstrap.
    pub fn mixin_bootstrap(&self) -> &MixinBootstrap {
        &self.mixin_bootstrap
    }
    
    /// Get the launcher integration.
    pub fn launcher(&self) -> &LauncherIntegration {
        &self.launcher
    }
    
    /// Check if a mod is loaded.
    pub fn is_mod_loaded(&self, mod_id: &str) -> bool {
        self.mods.contains_key(mod_id)
    }
    
    /// Get mod metadata.
    pub fn get_mod_metadata(&self, mod_id: &str) -> Option<&FabricMod> {
        self.mods.get(mod_id)
    }
}

/// Global Fabric Loader instance.
static LOADER: OnceLock<Arc<parking_lot::RwLock<FabricLoader>>> = OnceLock::new();

/// Get the global Fabric Loader, initializing if needed.
pub fn global() -> Result<Arc<parking_lot::RwLock<FabricLoader>>, LoaderError> {
    let loader = LOADER.get_or_init(|| {
        let jvm = FabricJvm::global().expect("Failed to create JVM");
        Arc::new(parking_lot::RwLock::new(FabricLoader::new(jvm).expect("Failed to create loader")))
    });
    Ok(loader.clone())
}

/// Initialize the global Fabric Loader with a JVM.
pub fn initialize_global(jvm: Arc<FabricJvm>) -> Result<Arc<parking_lot::RwLock<FabricLoader>>, LoaderError> {
    let loader = Arc::new(parking_lot::RwLock::new(FabricLoader::new(jvm)?));
    LOADER.set(loader.clone()).map_err(|_| LoaderError::ModNotFound("Loader already initialized".into()))?;
    Ok(loader)
}