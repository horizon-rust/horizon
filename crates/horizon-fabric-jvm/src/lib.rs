//! Embedded JVM for Fabric mod execution.
//!
//! This crate owns the process-wide JVM. It starts a real HotSpot JVM through
//! the JNI invocation API and hands out attached [`jni::JNIEnv`] references to
//! the rest of the Fabric stack. Only one JVM can exist per process, so the
//! instance is kept in a global cell.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod class_loader;
mod jvm_args;

pub use class_loader::{ClassLoaderConfig, ClassLoaderError, ClassLoaderParent, FabricClassLoader, ModClassLoader};
pub use jvm_args::{GcConfig, JvmConfig};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use horizon_core::VANILLA_VERSION;

/// Errors that can occur during JVM operations.
#[derive(Debug, Error)]
pub enum JvmError {
    /// The JVM shared library was not found.
    #[error("JVM library not found at {0}")]
    LibraryNotFound(PathBuf),
    /// Loading the JVM shared library failed.
    #[error("Failed to load JVM: {0}")]
    LoadFailed(String),
    /// Creating the JVM failed.
    #[error("Failed to create JVM: {0}")]
    CreationFailed(String),
    /// A JNI call failed.
    #[error("JNI error: {0}")]
    Jni(String),
    /// A class loader operation failed.
    #[error("Class loader error: {0}")]
    ClassLoader(#[from] crate::class_loader::ClassLoaderError),
    /// The JVM was already started.
    #[error("JVM already initialized")]
    AlreadyInitialized,
    /// The JVM has not been started yet.
    #[error("JVM not initialized")]
    NotInitialized,
}

impl From<jni::errors::Error> for JvmError {
    fn from(value: jni::errors::Error) -> Self {
        JvmError::Jni(value.to_string())
    }
}

/// Global JVM instance. Only one can exist per process.
static JVM_INSTANCE: once_cell::sync::OnceCell<Arc<FabricJvm>> = once_cell::sync::OnceCell::new();

/// The embedded JVM running Fabric mods.
pub struct FabricJvm {
    config: JvmConfig,
    class_loader: Arc<FabricClassLoader>,
    /// The live JVM, present once [`FabricJvm::start`] has succeeded.
    vm: parking_lot::RwLock<Option<Arc<jni::JavaVM>>>,
    /// Whether the JVM has been started.
    started: parking_lot::Mutex<bool>,
}

impl std::fmt::Debug for FabricJvm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FabricJvm")
            .field("config", &self.config)
            .field("started", &*self.started.lock())
            .finish_non_exhaustive()
    }
}

impl FabricJvm {
    /// Create a new JVM instance with the given configuration.
    ///
    /// The JVM itself is not started until [`FabricJvm::start`] is called.
    pub fn new(config: JvmConfig) -> Result<Arc<Self>, JvmError> {
        let class_loader = Arc::new(FabricClassLoader::new(ClassLoaderConfig {
            extra_urls: config.classpath_entries(),
            ..ClassLoaderConfig::default()
        })?);

        Ok(Arc::new(Self {
            config,
            class_loader,
            vm: parking_lot::RwLock::new(None),
            started: parking_lot::Mutex::new(false),
        }))
    }

    /// Start the JVM.
    ///
    /// This loads the platform JVM library (or the one configured in
    /// [`JvmConfig::jvm_path`]) and creates the VM with the configured heap and
    /// GC options.
    pub fn start(self: &Arc<Self>) -> Result<(), JvmError> {
        let mut started = self.started.lock();
        if *started {
            return Err(JvmError::AlreadyInitialized);
        }

        let mut builder = jni::InitArgsBuilder::new().version(jni::JNIVersion::V8);
        for option in self.config.jvm_options() {
            builder = builder.option(option);
        }
        let args = builder
            .build()
            .map_err(|e| JvmError::CreationFailed(e.to_string()))?;

        let vm = match Self::find_jvm_library(&self.config.jvm_path) {
            Ok(library) => jni::JavaVM::with_libjvm(args, || {
                Ok::<_, jni::errors::StartJvmError>(library.as_os_str().to_owned())
            }),
            Err(_) => jni::JavaVM::new(args),
        }
        .map_err(|e| JvmError::CreationFailed(e.to_string()))?;

        self.class_loader.initialize(&vm)?;
        *self.vm.write() = Some(Arc::new(vm));
        *started = true;
        Ok(())
    }

    /// Execute a closure with a JNI environment attached to the current thread.
    ///
    /// The thread stays attached for the duration of the call.
    pub fn with_env<F, R>(&self, f: F) -> Result<R, JvmError>
    where
        F: FnOnce(&mut jni::JNIEnv<'_>) -> Result<R, JvmError>,
    {
        let vm = self.vm.read();
        let vm = vm.as_ref().ok_or(JvmError::NotInitialized)?;
        let mut env = vm.attach_current_thread_permanently().map_err(JvmError::from)?;
        f(&mut env)
    }

    /// Execute a closure with a JNI environment, using raw JNI results.
    pub fn attach<F, R>(&self, f: F) -> Result<R, JvmError>
    where
        F: FnOnce(&mut jni::JNIEnv<'_>) -> jni::errors::Result<R>,
    {
        self.with_env(|env| f(env).map_err(JvmError::from))
    }

    /// Get the JVM configuration.
    pub fn config(&self) -> &JvmConfig {
        &self.config
    }

    /// Get the Fabric class loader.
    pub fn class_loader(&self) -> &FabricClassLoader {
        &self.class_loader
    }

    /// Get the Fabric class loader as a shared handle.
    pub fn class_loader_arc(&self) -> Arc<FabricClassLoader> {
        self.class_loader.clone()
    }

    /// Add jars or directories to the Fabric class path.
    ///
    /// Mod jars are only known after discovery, so this runs between discovery
    /// and entrypoint invocation. Classes already loaded are unaffected.
    pub fn add_classpath(&self, paths: &[PathBuf]) -> Result<(), JvmError> {
        let vm = self.vm().ok_or(JvmError::NotInitialized)?;
        self.class_loader.add_classpath(&vm, paths)?;
        Ok(())
    }

    /// Whether the JVM is currently running.
    pub fn is_started(&self) -> bool {
        *self.started.lock()
    }

    /// A handle to the live JVM, if it has been started.
    pub fn vm(&self) -> Option<Arc<jni::JavaVM>> {
        self.vm.read().as_ref().cloned()
    }

    /// Find the JVM shared library in the given directory.
    pub fn find_jvm_library(jvm_path: &Path) -> Result<PathBuf, JvmError> {
        let lib_name = if cfg!(windows) {
            "jvm.dll"
        } else if cfg!(target_os = "macos") {
            "libjvm.dylib"
        } else {
            "libjvm.so"
        };

        let candidates = [
            jvm_path.join("bin").join("server").join(lib_name),
            jvm_path.join("lib").join("server").join(lib_name),
            jvm_path.join("jre").join("lib").join("server").join(lib_name),
            jvm_path.join("jre").join("bin").join("server").join(lib_name),
            jvm_path.join(lib_name),
        ];

        for candidate in &candidates {
            if candidate.exists() {
                return Ok(candidate.clone());
            }
        }

        Err(JvmError::LibraryNotFound(jvm_path.to_path_buf()))
    }

    /// Get the global JVM instance, creating it (but not starting it) if needed.
    pub fn global() -> Result<Arc<Self>, JvmError> {
        JVM_INSTANCE
            .get_or_try_init(|| {
                let config = JvmConfig::default_for_version(VANILLA_VERSION)?;
                Self::new(config)
            })
            .cloned()
    }

    /// Set the global JVM instance.
    pub fn set_global(jvm: Arc<Self>) -> Result<(), JvmError> {
        JVM_INSTANCE
            .set(jvm)
            .map_err(|_| JvmError::AlreadyInitialized)
    }

    /// Shut the JVM down.
    ///
    /// JNI cannot destroy and recreate a VM in one process, so this only drops
    /// our handle; the process is expected to exit afterwards.
    pub fn shutdown(&self) -> Result<(), JvmError> {
        let mut started = self.started.lock();
        if !*started {
            return Ok(());
        }
        *self.vm.write() = None;
        *started = false;
        Ok(())
    }
}

impl Drop for FabricJvm {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// Initialize the global JVM with default settings for the current Minecraft version.
pub fn initialize_global() -> Result<Arc<FabricJvm>, JvmError> {
    FabricJvm::global()
}

/// Initialize the global JVM with a custom configuration and start it.
pub fn initialize_global_with(config: JvmConfig) -> Result<Arc<FabricJvm>, JvmError> {
    let jvm = FabricJvm::new(config)?;
    jvm.start()?;
    FabricJvm::set_global(jvm.clone())?;
    Ok(jvm)
}
