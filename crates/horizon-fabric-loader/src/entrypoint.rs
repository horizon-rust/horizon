//! Entrypoint invocation for Fabric mods.
//!
//! Fabric's loader instantiates each entrypoint class and calls a single
//! no-argument method on it. This module does exactly that through the JNI
//! bridge, checking the interface first so a wrong entrypoint fails loudly.

use horizon_fabric_jni::{JniBridge, JniError};
use thiserror::Error;

/// Fabric's `ModInitializer` interface.
pub const MOD_INITIALIZER: &str = "net/fabricmc/api/ModInitializer";
/// Fabric's `ClientModInitializer` interface.
pub const CLIENT_MOD_INITIALIZER: &str = "net/fabricmc/api/ClientModInitializer";
/// Fabric's `DedicatedServerModInitializer` interface.
pub const SERVER_MOD_INITIALIZER: &str = "net/fabricmc/api/DedicatedServerModInitializer";

/// Load phases for mod initialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadPhase {
    /// Pre-launch: Mixin bootstrap, early initialization.
    PreLaunch,
    /// Launch: Main mod initializers (`ModInitializer`).
    Launch,
    /// Client launch: Client mod initializers (`ClientModInitializer`).
    ClientLaunch,
    /// Server launch: Server mod initializers (`DedicatedServerModInitializer`).
    ServerLaunch,
}

/// Errors from entrypoint invocation.
#[derive(Debug, Error)]
pub enum EntrypointError {
    /// The JNI bridge failed.
    #[error("JNI error: {0}")]
    Jni(#[from] JniError),
    /// The class could not be loaded.
    #[error("class not found: {0}")]
    ClassNotFound(String),
    /// The class does not implement the required interface.
    #[error("{class} does not implement {interface}")]
    InvalidType {
        /// The entrypoint class.
        class: String,
        /// The required interface.
        interface: String,
    },
    /// The entrypoint method threw.
    #[error("entrypoint {class}.{method} failed: {source}")]
    InvocationFailed {
        /// The entrypoint class.
        class: String,
        /// The method name.
        method: String,
        /// The underlying error.
        source: JniError,
    },
}

/// Mirrors Fabric's `ModInitializer` for Rust-side implementations.
pub trait ModInitializer {
    /// Called during the Launch phase.
    fn on_initialize(&self) -> Result<(), EntrypointError>;
}

/// Mirrors Fabric's `ClientModInitializer`.
pub trait ClientModInitializer {
    /// Called during the ClientLaunch phase.
    fn on_initialize_client(&self) -> Result<(), EntrypointError>;
}

/// Mirrors Fabric's `DedicatedServerModInitializer`.
pub trait DedicatedServerModInitializer {
    /// Called during the ServerLaunch phase.
    fn on_initialize_server(&self) -> Result<(), EntrypointError>;
}

/// Invokes mod entrypoints through the JNI bridge.
#[derive(Clone)]
pub struct EntrypointInvoker {
    bridge: JniBridge,
}

impl EntrypointInvoker {
    /// Create a new entrypoint invoker.
    pub fn new(bridge: JniBridge) -> Self {
        Self { bridge }
    }

    /// Invoke a `ModInitializer` entrypoint.
    pub fn invoke_main(&self, class_name: &str) -> Result<(), EntrypointError> {
        self.invoke_typed(class_name, MOD_INITIALIZER, "onInitialize")
    }

    /// Invoke a `ClientModInitializer` entrypoint.
    pub fn invoke_client(&self, class_name: &str) -> Result<(), EntrypointError> {
        self.invoke_typed(class_name, CLIENT_MOD_INITIALIZER, "onInitializeClient")
    }

    /// Invoke a `DedicatedServerModInitializer` entrypoint.
    pub fn invoke_server(&self, class_name: &str) -> Result<(), EntrypointError> {
        self.invoke_typed(class_name, SERVER_MOD_INITIALIZER, "onInitializeServer")
    }

    /// Invoke an entrypoint of the given phase.
    pub fn invoke(&self, class_name: &str, phase: LoadPhase) -> Result<(), EntrypointError> {
        match phase {
            LoadPhase::PreLaunch => Ok(()),
            LoadPhase::Launch => self.invoke_main(class_name),
            LoadPhase::ClientLaunch => self.invoke_client(class_name),
            LoadPhase::ServerLaunch => self.invoke_server(class_name),
        }
    }

    fn invoke_typed(
        &self,
        class_name: &str,
        interface: &str,
        method: &str,
    ) -> Result<(), EntrypointError> {
        let class = self
            .bridge
            .resolve_class(class_name)
            .map_err(|_| EntrypointError::ClassNotFound(class_name.to_string()))?;

        if !self
            .bridge
            .implements(&class, interface)
            .map_err(EntrypointError::Jni)?
        {
            return Err(EntrypointError::InvalidType {
                class: class_name.to_string(),
                interface: interface.to_string(),
            });
        }

        let object = self.bridge.instantiate(&class).map_err(EntrypointError::Jni)?;
        self.bridge
            .call_void(&object, method)
            .map_err(|source| EntrypointError::InvocationFailed {
                class: class_name.to_string(),
                method: method.to_string(),
                source,
            })
    }
}
