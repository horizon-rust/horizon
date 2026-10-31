//! Fabric API surface for Horizon.
//!
//! This crate models the parts of Fabric API that a mod can use through the
//! bridge: an event bus, namespaced registries, and custom payload channels.
//!
//! It is deliberately self-contained. It does not pretend to be Fabric API's
//! Java classes; the Java side of Fabric API is loaded by the JVM, and these
//! Rust types are what the bridge forwards those calls into.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod events;
pub mod networking;
pub mod registry;

pub use events::{Event, EventBus, SubscriptionId};
pub use networking::{ChannelDirection, ChannelRegistry, CustomPayload};
pub use registry::{Identifier, Registry, RegistryError};

use std::sync::Arc;

use horizon_fabric_jvm::FabricJvm;
use horizon_fabric_loader::FabricLoader;
use thiserror::Error;

/// Errors from Fabric API operations.
#[derive(Debug, Error)]
pub enum FabricApiError {
    /// The JVM failed.
    #[error("JVM error: {0}")]
    Jvm(#[from] horizon_fabric_jvm::JvmError),
    /// The loader failed.
    #[error("Loader error: {0}")]
    Loader(#[from] horizon_fabric_loader::LoaderError),
    /// A registry operation failed.
    #[error("Registry error: {0}")]
    Registry(#[from] RegistryError),
    /// The API has not been initialized.
    #[error("Fabric API is not initialized")]
    NotInitialized,
    /// A feature is not available yet.
    #[error("not implemented: {0}")]
    NotImplemented(String),
}

/// The Fabric API context: the event bus, registries and channels that mods
/// see, plus a handle back to the JVM that is running them.
pub struct FabricApi {
    jvm: Arc<FabricJvm>,
    loader: Arc<parking_lot::RwLock<FabricLoader>>,
    events: EventBus,
    channels: ChannelRegistry,
    items: Registry<registry::ItemEntry>,
    blocks: Registry<registry::BlockEntry>,
    entity_types: Registry<registry::EntityTypeEntry>,
    initialized: bool,
}

impl FabricApi {
    /// Create a new Fabric API context.
    pub fn new(
        jvm: Arc<FabricJvm>,
        loader: Arc<parking_lot::RwLock<FabricLoader>>,
    ) -> Result<Self, FabricApiError> {
        Ok(Self {
            jvm,
            loader,
            events: EventBus::new(),
            channels: ChannelRegistry::new(),
            items: Registry::new("item"),
            blocks: Registry::new("block"),
            entity_types: Registry::new("entity_type"),
            initialized: false,
        })
    }

    /// Mark the API as initialized. Called once the JVM and loader are ready.
    pub fn initialize(&mut self) -> Result<(), FabricApiError> {
        if !self.jvm.is_started() {
            return Err(FabricApiError::NotInitialized);
        }
        self.initialized = true;
        Ok(())
    }

    /// Whether the API has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// The JVM running the mods.
    pub fn jvm(&self) -> &Arc<FabricJvm> {
        &self.jvm
    }

    /// The Fabric loader.
    pub fn loader(&self) -> &Arc<parking_lot::RwLock<FabricLoader>> {
        &self.loader
    }

    /// The event bus.
    pub fn events(&self) -> &EventBus {
        &self.events
    }

    /// The event bus, mutably.
    pub fn events_mut(&mut self) -> &mut EventBus {
        &mut self.events
    }

    /// The custom payload channel registry.
    pub fn channels(&self) -> &ChannelRegistry {
        &self.channels
    }

    /// The custom payload channel registry, mutably.
    pub fn channels_mut(&mut self) -> &mut ChannelRegistry {
        &mut self.channels
    }

    /// The item registry.
    pub fn items(&self) -> &Registry<registry::ItemEntry> {
        &self.items
    }

    /// The item registry, mutably.
    pub fn items_mut(&mut self) -> &mut Registry<registry::ItemEntry> {
        &mut self.items
    }

    /// The block registry.
    pub fn blocks(&self) -> &Registry<registry::BlockEntry> {
        &self.blocks
    }

    /// The block registry, mutably.
    pub fn blocks_mut(&mut self) -> &mut Registry<registry::BlockEntry> {
        &mut self.blocks
    }

    /// The entity type registry.
    pub fn entity_types(&self) -> &Registry<registry::EntityTypeEntry> {
        &self.entity_types
    }

    /// The entity type registry, mutably.
    pub fn entity_types_mut(&mut self) -> &mut Registry<registry::EntityTypeEntry> {
        &mut self.entity_types
    }
}
