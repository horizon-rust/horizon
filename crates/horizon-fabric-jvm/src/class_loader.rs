//! Fabric mod class loader isolation.
//!
//! Fabric runs every mod in a single `URLClassLoader` whose parent is the
//! platform class loader, so mods can see each other and Fabric API but not the
//! (nonexistent here) application class path. This module builds that loader
//! and exposes class lookup through it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Configuration for the Fabric class loader.
#[derive(Debug, Clone)]
pub struct ClassLoaderConfig {
    /// Parent class loader (usually the platform loader).
    pub parent: Option<ClassLoaderParent>,
    /// Whether to enable class transformation (Mixin).
    pub enable_transformer: bool,
    /// Additional URLs to add to the classpath.
    pub extra_urls: Vec<PathBuf>,
}

impl Default for ClassLoaderConfig {
    fn default() -> Self {
        Self {
            parent: Some(ClassLoaderParent::Platform),
            enable_transformer: true,
            extra_urls: Vec::new(),
        }
    }
}

/// Parent class loader options.
#[derive(Debug, Clone, Copy)]
pub enum ClassLoaderParent {
    /// Bootstrap class loader.
    Bootstrap,
    /// Platform class loader (Java 9+).
    Platform,
    /// System class loader.
    System,
}

/// Errors from class loader operations.
#[derive(Debug, Error)]
pub enum ClassLoaderError {
    /// Creating the loader failed.
    #[error("Failed to create class loader: {0}")]
    CreationFailed(String),
    /// Adding a URL failed.
    #[error("Failed to add URL: {0}")]
    AddUrlFailed(String),
    /// A class could not be found.
    #[error("Class not found: {0}")]
    ClassNotFound(String),
    /// A JNI call failed.
    #[error("JNI error: {0}")]
    Jni(String),
}

impl From<jni::errors::Error> for ClassLoaderError {
    fn from(value: jni::errors::Error) -> Self {
        ClassLoaderError::Jni(value.to_string())
    }
}

/// Fabric mod class loader.
pub struct FabricClassLoader {
    config: ClassLoaderConfig,
    /// Every URL currently on the loader's class path.
    urls: parking_lot::RwLock<Vec<PathBuf>>,
    /// The Java `ClassLoader` instance, once initialized.
    loader: parking_lot::RwLock<Option<jni::objects::GlobalRef>>,
    /// Per-mod class loaders (mod ID -> ClassLoader).
    mod_loaders: parking_lot::RwLock<HashMap<String, jni::objects::GlobalRef>>,
}

impl std::fmt::Debug for FabricClassLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FabricClassLoader")
            .field("config", &self.config)
            .field("initialized", &self.loader.read().is_some())
            .finish_non_exhaustive()
    }
}

impl FabricClassLoader {
    /// Create a new Fabric class loader with the given configuration.
    pub fn new(config: ClassLoaderConfig) -> Result<Self, ClassLoaderError> {
        let urls = config.extra_urls.clone();
        Ok(Self {
            config,
            urls: parking_lot::RwLock::new(urls),
            loader: parking_lot::RwLock::new(None),
            mod_loaders: parking_lot::RwLock::new(HashMap::new()),
        })
    }

    /// Initialize the class loader inside the running JVM.
    pub fn initialize(&self, vm: &jni::JavaVM) -> Result<(), ClassLoaderError> {
        let mut env = vm.attach_current_thread_permanently()?;
        let urls = self.urls.read().clone();
        let loader = create_class_loader(&mut env, &self.config, &urls)?;
        *self.loader.write() = Some(loader);
        Ok(())
    }

    /// Add jars or directories to the class path, rebuilding the loader.
    ///
    /// `URLClassLoader`'s own `addURL` is protected and inaccessible from
    /// outside `java.base`, so the loader is recreated with the full path. Any
    /// class already loaded keeps the old loader, which is why this must run
    /// before mod classes are touched — exactly what the loader does.
    pub fn add_classpath(
        &self,
        vm: &jni::JavaVM,
        paths: &[PathBuf],
    ) -> Result<(), ClassLoaderError> {
        {
            let mut urls = self.urls.write();
            for path in paths {
                if !urls.contains(path) {
                    urls.push(path.clone());
                }
            }
        }

        if self.is_initialized() {
            self.initialize(vm)?;
        }
        Ok(())
    }

    /// The URLs currently on the class path.
    pub fn classpath(&self) -> Vec<PathBuf> {
        self.urls.read().clone()
    }

    /// Whether the loader has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.loader.read().is_some()
    }

    /// Create a per-mod class loader that delegates to the base Fabric loader.
    pub fn create_mod_loader(
        &self,
        mod_id: &str,
        mod_jar: &Path,
        vm: &jni::JavaVM,
    ) -> Result<(), ClassLoaderError> {
        let parent = self
            .loader
            .read()
            .clone()
            .ok_or_else(|| ClassLoaderError::CreationFailed("class loader not initialized".into()))?;

        let mut env = vm.attach_current_thread_permanently()?;
        let url = file_url(mod_jar);
        let urls = make_url_array(&mut env, std::slice::from_ref(&url))?;
        let loader_class = env.find_class("java/net/URLClassLoader")?;
        let loader = env.new_object(
            loader_class,
            "([Ljava/net/URL;Ljava/lang/ClassLoader;)V",
            &[
                jni::objects::JValue::Object(&urls),
                jni::objects::JValue::Object(parent.as_obj()),
            ],
        )?;
        let global = env.new_global_ref(loader)?;
        self.mod_loaders.write().insert(mod_id.to_string(), global);
        Ok(())
    }

    /// Load a class through the Fabric class loader.
    pub fn load_class(
        &self,
        vm: &jni::JavaVM,
        name: &str,
    ) -> Result<jni::objects::GlobalRef, ClassLoaderError> {
        let loader = self
            .loader
            .read()
            .clone()
            .ok_or_else(|| ClassLoaderError::CreationFailed("class loader not initialized".into()))?;

        let mut env = vm.attach_current_thread_permanently()?;
        // `ClassLoader#loadClass` takes a binary name (`a.b.C`), not a JVM
        // internal name (`a/b/C`).
        let binary_name = name.replace('/', ".");
        let java_name = env.new_string(&binary_name)?;
        let result = env.call_method(
            loader.as_obj(),
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[jni::objects::JValue::Object(&*java_name)],
        );

        let class = match result {
            Ok(value) => match value.l() {
                Ok(class) => class,
                Err(e) => {
                    clear_pending_exception(&mut env);
                    return Err(ClassLoaderError::Jni(e.to_string()));
                }
            },
            Err(_) => {
                // Leave no pending exception behind: every later JNI call on
                // this thread would fail otherwise.
                clear_pending_exception(&mut env);
                return Err(ClassLoaderError::ClassNotFound(name.to_string()));
            }
        };
        Ok(env.new_global_ref(class)?)
    }

    /// Add a URL to the class loader's classpath.
    ///
    /// `URLClassLoader` only accepts new URLs through a protected method, so a
    /// loader created by this crate must be given its full class path up front.
    pub fn add_url(&self, _vm: &jni::JavaVM, url: &str) -> Result<(), ClassLoaderError> {
        Err(ClassLoaderError::AddUrlFailed(format!(
            "class path is fixed at creation; add `{url}` to ClassLoaderConfig::extra_urls"
        )))
    }
}

/// Clear a pending Java exception, if any.
///
/// A failed `loadClass` leaves a `ClassNotFoundException` pending; every later
/// JNI call on the same thread would fail until it is cleared.
fn clear_pending_exception(env: &mut jni::JNIEnv<'_>) {
    if let Ok(true) = env.exception_check() {
        let _ = env.exception_clear();
    }
}

/// Convert a filesystem path into a `file://` URL.
pub fn file_url(path: &Path) -> String {    let path = path.to_string_lossy().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

fn make_url_array<'a>(
    env: &mut jni::JNIEnv<'a>,
    urls: &[String],
) -> Result<jni::objects::JObjectArray<'a>, ClassLoaderError> {
    let url_class = env.find_class("java/net/URL")?;
    let array = env.new_object_array(
        urls.len() as i32,
        url_class,
        jni::objects::JObject::null(),
    )?;

    for (index, url) in urls.iter().enumerate() {
        let string = env.new_string(url)?;
        let url_class = env.find_class("java/net/URL")?;
        let url_obj = env.new_object(
            url_class,
            "(Ljava/lang/String;)V",
            &[jni::objects::JValue::Object(&*string)],
        )?;
        env.set_object_array_element(&array, index as i32, url_obj)?;
    }

    Ok(array)
}

fn create_class_loader(
    env: &mut jni::JNIEnv<'_>,
    config: &ClassLoaderConfig,
    urls: &[PathBuf],
) -> Result<jni::objects::GlobalRef, ClassLoaderError> {
    let urls: Vec<String> = urls.iter().map(|p| file_url(p)).collect();
    let url_array = make_url_array(env, &urls)?;

    let parent = match config.parent {
        Some(ClassLoaderParent::Bootstrap) | None => jni::objects::JObject::null(),
        Some(ClassLoaderParent::Platform) => {
            let class_loader = env.find_class("java/lang/ClassLoader")?;
            env.call_static_method(
                class_loader,
                "getPlatformClassLoader",
                "()Ljava/lang/ClassLoader;",
                &[],
            )?
            .l()?
        }
        Some(ClassLoaderParent::System) => {
            let class_loader = env.find_class("java/lang/ClassLoader")?;
            env.call_static_method(
                class_loader,
                "getSystemClassLoader",
                "()Ljava/lang/ClassLoader;",
                &[],
            )?
            .l()?
        }
    };

    let loader_class = env.find_class("java/net/URLClassLoader")?;
    let loader = env.new_object(
        loader_class,
        "([Ljava/net/URL;Ljava/lang/ClassLoader;)V",
        &[
            jni::objects::JValue::Object(&url_array),
            jni::objects::JValue::Object(&parent),
        ],
    )?;

    Ok(env.new_global_ref(loader)?)
}

/// A class loader handle for a specific mod.
pub struct ModClassLoader {
    mod_id: String,
    loaded_classes: parking_lot::RwLock<std::collections::HashSet<String>>,
}

impl ModClassLoader {
    /// Create a new mod class loader handle.
    pub fn new(mod_id: String) -> Self {
        Self {
            mod_id,
            loaded_classes: parking_lot::RwLock::new(std::collections::HashSet::new()),
        }
    }

    /// Get the mod ID.
    pub fn mod_id(&self) -> &str {
        &self.mod_id
    }

    /// Record that a class was loaded through this mod.
    pub fn record_loaded(&self, name: impl Into<String>) {
        self.loaded_classes.write().insert(name.into());
    }

    /// Get all classes loaded by this mod loader.
    pub fn loaded_classes(&self) -> Vec<String> {
        self.loaded_classes.read().iter().cloned().collect()
    }
}
