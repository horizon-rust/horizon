//! Horizon ↔ Fabric integration.
//!
//! This is the entry point a game or launcher uses to run Fabric mods:
//!
//! 1. start the embedded JVM,
//! 2. point the Fabric class loader at the mod jars,
//! 3. discover and dependency-sort the mods,
//! 4. run their entrypoints phase by phase,
//! 5. forward tick and event traffic into the Rust-side Fabric API surface.
//!
//! The honest limit: mods that call `net.minecraft.*` need Minecraft's Java
//! classes on the classpath. Horizon does not ship them, so only mods that stay
//! inside their own classes and Fabric API can run end to end.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::Path;
use std::sync::Arc;

use horizon_fabric_api::{FabricApi, FabricApiError};
use horizon_fabric_jvm::{FabricJvm, JvmConfig, JvmError};
use horizon_fabric_loader::{FabricLoader, LoadPhase, LoaderError, MixinBootstrap};
use thiserror::Error;

/// Errors from Fabric integration.
#[derive(Debug, Error)]
pub enum IntegrationError {
    /// The JVM failed.
    #[error("JVM error: {0}")]
    Jvm(#[from] JvmError),
    /// The loader failed.
    #[error("Loader error: {0}")]
    Loader(#[from] LoaderError),
    /// The Fabric API failed.
    #[error("Fabric API error: {0}")]
    Api(#[from] FabricApiError),
    /// The Mixin subsystem failed.
    #[error("Mixin error: {0}")]
    Mixin(#[from] horizon_fabric_loader::MixinError),
    /// The integration has not been initialized.
    #[error("Fabric integration is not initialized")]
    NotInitialized,
    /// I/O failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// The Fabric integration: JVM, loader and API wired together.
pub struct FabricIntegration {
    jvm: Arc<FabricJvm>,
    loader: Arc<parking_lot::RwLock<FabricLoader>>,
    api: FabricApi,
    mixin: MixinBootstrap,
    initialized: bool,
    tick_counter: u64,
}

impl FabricIntegration {
    /// Create the integration and start the JVM.
    pub fn new() -> Result<Self, IntegrationError> {
        let config = JvmConfig::for_system_jvm()
            .or_else(|_| JvmConfig::default_for_version(horizon_core::VANILLA_VERSION))?;
        Self::with_config(config)
    }

    /// Create the integration with an explicit JVM configuration.
    pub fn with_config(config: JvmConfig) -> Result<Self, IntegrationError> {
        let jvm = FabricJvm::new(config)?;
        jvm.start()?;

        let loader = Arc::new(parking_lot::RwLock::new(FabricLoader::new(jvm.clone())?));
        let api = FabricApi::new(jvm.clone(), loader.clone())?;

        Ok(Self {
            jvm,
            loader,
            api,
            mixin: MixinBootstrap::new(),
            initialized: false,
            tick_counter: 0,
        })
    }

    /// Discover mods in `mods_dir`, resolve them, and run every client phase.
    pub fn initialize(&mut self, mods_dir: &Path) -> Result<(), IntegrationError> {
        {
            let mut loader = self.loader.write();
            loader.discover_mods(mods_dir)?;
            loader.resolve()?;
        }

        self.mixin.initialize(&self.jvm)?;

        {
            let mut loader = self.loader.write();
            loader.initialize(LoadPhase::PreLaunch)?;
        }

        self.api.initialize()?;

        {
            let mut loader = self.loader.write();
            loader.initialize(LoadPhase::Launch)?;
            loader.initialize(LoadPhase::ClientLaunch)?;
        }

        self.initialized = true;
        Ok(())
    }

    /// Run the dedicated-server entrypoints.
    pub fn initialize_server(&mut self) -> Result<(), IntegrationError> {
        let mut loader = self.loader.write();
        loader.initialize(LoadPhase::ServerLaunch)?;
        Ok(())
    }

    /// Advance the Fabric side by one game tick.
    ///
    /// Fires [`horizon_fabric_api::events::TickEvent`] and returns the number of
    /// listeners that ran.
    pub fn tick(&mut self) -> Result<usize, IntegrationError> {
        if !self.initialized {
            return Err(IntegrationError::NotInitialized);
        }

        self.tick_counter = self.tick_counter.wrapping_add(1);
        let event = horizon_fabric_api::events::TickEvent {
            tick: self.tick_counter,
        };
        Ok(self.api.events().fire(&event))
    }

    /// The JVM.
    pub fn jvm(&self) -> &Arc<FabricJvm> {
        &self.jvm
    }

    /// The loader.
    pub fn loader(&self) -> &Arc<parking_lot::RwLock<FabricLoader>> {
        &self.loader
    }

    /// The Fabric API surface.
    pub fn api(&self) -> &FabricApi {
        &self.api
    }

    /// The Fabric API surface, mutably.
    pub fn api_mut(&mut self) -> &mut FabricApi {
        &mut self.api
    }

    /// The Mixin bootstrap.
    pub fn mixin(&self) -> &MixinBootstrap {
        &self.mixin
    }

    /// Whether [`FabricIntegration::initialize`] has run.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Number of discovered mods.
    pub fn mod_count(&self) -> usize {
        self.loader.read().mods().len()
    }

    /// IDs in load order.
    pub fn load_order(&self) -> Vec<String> {
        self.loader.read().load_order().to_vec()
    }

    /// Shut the JVM down.
    pub fn shutdown(&mut self) -> Result<(), IntegrationError> {
        self.jvm.shutdown()?;
        self.initialized = false;
        Ok(())
    }
}

impl Drop for FabricIntegration {
    fn drop(&mut self) {
        let _ = self.jvm.shutdown();
    }
}
