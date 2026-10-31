//! JNI bridge between Horizon and the embedded Fabric JVM.
//!
//! This is a thin, honest wrapper over [`jni::JNIEnv`]: it owns no JVM of its
//! own, it just attaches the calling thread and offers the handful of
//! operations the Fabric loader needs (class lookup, construction, method
//! calls, exception reporting).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;

use jni::objects::{GlobalRef, JObject, JString, JValue};
use jni::JNIEnv;
use thiserror::Error;

use horizon_fabric_jvm::{FabricClassLoader, FabricJvm, JvmError};
use horizon_fabric_mapping::{MappingError, MappingLayer};

/// Errors from JNI bridge operations.
#[derive(Debug, Error)]
pub enum JniError {
    /// The JVM failed.
    #[error("JVM error: {0}")]
    Jvm(#[from] JvmError),
    /// A JNI call failed.
    #[error("JNI error: {0}")]
    Jni(#[from] jni::errors::Error),
    /// A mapping lookup failed.
    #[error("mapping error: {0}")]
    Mapping(#[from] MappingError),
    /// The class could not be found.
    #[error("class not found: {0}")]
    ClassNotFound(String),
    /// The method could not be found.
    #[error("method not found: {class}.{method}{sig}")]
    MethodNotFound {
        /// Declaring class.
        class: String,
        /// Method name.
        method: String,
        /// JVM descriptor.
        sig: String,
    },
    /// A class loader operation failed.
    #[error("class loader error: {0}")]
    ClassLoader(#[from] horizon_fabric_jvm::ClassLoaderError),
    /// The object does not implement the expected interface.
    #[error("{class} does not implement {interface}")]
    NotAnInstance {
        /// Concrete class name.
        class: String,
        /// Expected interface.
        interface: String,
    },
    /// Java threw an exception.
    #[error("Java exception: {0}")]
    JavaException(String),
    /// Anything else.
    #[error("{0}")]
    Other(String),
}

/// Result alias for bridge operations.
pub type JniResult<T> = Result<T, JniError>;

/// The JNI bridge: a handle to the JVM plus the Fabric class loader.
#[derive(Clone)]
pub struct JniBridge {
    jvm: Arc<FabricJvm>,
    class_loader: Arc<FabricClassLoader>,
    mapping: Arc<MappingLayer>,
}

impl std::fmt::Debug for JniBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JniBridge")
            .field("jvm", &self.jvm)
            .field("mapping_classes", &self.mapping.class_count())
            .finish_non_exhaustive()
    }
}

impl JniBridge {
    /// Create a bridge over an already-created JVM and class loader.
    pub fn new(
        jvm: Arc<FabricJvm>,
        class_loader: Arc<FabricClassLoader>,
        mapping: Arc<MappingLayer>,
    ) -> Self {
        Self {
            jvm,
            class_loader,
            mapping,
        }
    }

    /// Create a bridge, using the JVM's own class loader and the global mappings.
    pub fn for_jvm(jvm: Arc<FabricJvm>) -> JniResult<Self> {
        let mapping = match horizon_fabric_mapping::global() {
            Ok(mapping) => mapping,
            Err(_) => {
                let yarn = horizon_fabric_mapping::YarnMappings::load_for_version(
                    horizon_core::VANILLA_VERSION,
                )?;
                let horizon = horizon_fabric_mapping::HorizonMappings::load()?;
                Arc::new(MappingLayer::new(yarn, horizon))
            }
        };
        let class_loader = jvm.class_loader_arc();
        Ok(Self {
            jvm,
            class_loader,
            mapping,
        })
    }

    /// The underlying JVM.
    pub fn jvm(&self) -> &Arc<FabricJvm> {
        &self.jvm
    }

    /// The Fabric class loader.
    pub fn class_loader(&self) -> &Arc<FabricClassLoader> {
        &self.class_loader
    }

    /// The mapping layer.
    pub fn mapping(&self) -> &Arc<MappingLayer> {
        &self.mapping
    }

    /// Run a closure with the calling thread attached to the JVM.
    pub fn with_env<F, R>(&self, f: F) -> JniResult<R>
    where
        F: FnOnce(&mut JNIEnv<'_>) -> JniResult<R>,
        R: 'static,
    {
        let mut slot: Option<JniResult<R>> = None;
        self.jvm.with_env(|env| {
            slot = Some(f(env));
            Ok(())
        })?;
        slot.unwrap_or_else(|| Err(JniError::Other("JNI closure did not run".into())))
    }

    /// Find a class by JVM name (`java/lang/String` or `java.lang.String`).
    pub fn find_class(&self, name: &str) -> JniResult<GlobalRef> {
        let java_name = name.replace('.', "/");
        self.with_env(|env| {
            let class = env
                .find_class(&java_name)
                .map_err(|_| JniError::ClassNotFound(name.to_string()))?;
            Ok(env.new_global_ref(class)?)
        })
    }

    /// Load a class through the Fabric class loader (so mod classes resolve).
    pub fn load_class(&self, name: &str) -> JniResult<GlobalRef> {
        let vm = self.jvm.vm().ok_or(JvmError::NotInitialized)?;
        Ok(self.class_loader.load_class(&vm, name)?)
    }

    /// Load a class through the Fabric loader, falling back to the JVM's own
    /// class path for JDK and Fabric API classes.
    pub fn resolve_class(&self, name: &str) -> JniResult<GlobalRef> {
        match self.load_class(name) {
            Ok(class) => Ok(class),
            Err(_) => self.find_class(name),
        }
    }

    /// Instantiate a class with its no-argument constructor.
    pub fn instantiate(&self, class: &GlobalRef) -> JniResult<GlobalRef> {
        self.with_env(|env| {
            let object = env.new_object(class, "()V", &[])?;
            Ok(env.new_global_ref(object)?)
        })
    }

    /// Check whether `class` implements (or extends) `interface`.
    pub fn implements(&self, class: &GlobalRef, interface: &str) -> JniResult<bool> {
        // Resolve the interface through the Fabric loader too: it lives in the
        // Fabric API jar, which is not on the JVM's system class path.
        let interface_class = self.resolve_class(interface)?;
        self.with_env(|env| Ok(env.is_assignable_from(class, &interface_class)?))
    }

    /// Call a no-argument `void` method on an object.
    pub fn call_void(&self, object: &GlobalRef, method: &str) -> JniResult<()> {
        self.with_env(|env| {
            env.call_method(object.as_obj(), method, "()V", &[])?;
            Self::take_exception(env)
        })
    }

    /// Call a static `void` method.
    pub fn call_static_void(&self, class: &GlobalRef, method: &str) -> JniResult<()> {
        self.with_env(|env| {
            env.call_static_method(class, method, "()V", &[])?;
            Self::take_exception(env)
        })
    }

    /// Invoke a Fabric entrypoint: instantiate the class and call its method.
    pub fn invoke_entrypoint(
        &self,
        class_name: &str,
        interface: &str,
        method: &str,
    ) -> JniResult<()> {
        let class = self.resolve_class(class_name)?;

        if self.implements(&class, interface)? {
            let object = self.instantiate(&class)?;
            self.call_void(&object, method)?;
            return Ok(());
        }

        // Some mods list a `static` entrypoint holder; try a static call before
        // giving up.
        if self.call_static_void(&class, method).is_ok() {
            return Ok(());
        }

        Err(JniError::NotAnInstance {
            class: class_name.to_string(),
            interface: interface.to_string(),
        })
    }

    /// Convert a thrown Java exception into an [`JniError::JavaException`].
    fn take_exception(env: &mut JNIEnv<'_>) -> JniResult<()> {
        if !env.exception_check()? {
            return Ok(());
        }

        let throwable = env.exception_occurred()?;
        env.exception_clear()?;

        let message = env
            .call_method(&throwable, "toString", "()Ljava/lang/String;", &[])
            .ok()
            .and_then(|value| value.l().ok())
            .and_then(|string| {
                let jstring = JString::from(string);
                env.get_string(&jstring)
                    .ok()
                    .map(|chars| chars.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "unknown Java exception".to_string());

        Err(JniError::JavaException(message))
    }
}

/// Convenience: build a `JValue` array from string arguments.
pub fn string_args<'a>(env: &mut JNIEnv<'a>, values: &[&str]) -> JniResult<Vec<JObject<'a>>> {
    values
        .iter()
        .map(|value| env.new_string(value).map(JObject::from).map_err(JniError::from))
        .collect()
}

/// Convenience: call `String.valueOf`-style `toString` on a Java object.
pub fn java_to_string(env: &mut JNIEnv<'_>, object: &JObject<'_>) -> JniResult<String> {
    let value = env.call_method(object, "toString", "()Ljava/lang/String;", &[])?;
    let string = JString::from(value.l()?);
    let chars = env.get_string(&string)?;
    Ok(chars.to_string_lossy().into_owned())
}

/// Build a `JClass` from a global reference.
///
/// `&GlobalRef` already implements `Desc<JClass>`, so this is only a marker for
/// call sites that want to be explicit.
pub fn class_of(object: &GlobalRef) -> &GlobalRef {
    object
}

/// Helper for building a `JClass` from a [`GlobalRef`].
pub fn as_class(object: &GlobalRef) -> &GlobalRef {
    object
}

/// Helper for building a `JObject` from a [`GlobalRef`].
pub fn as_object(object: &GlobalRef) -> &JObject<'_> {
    object.as_obj()
}

/// Build a `JValue` for an object reference.
pub fn object_value(object: &GlobalRef) -> JValue<'static, '_> {
    JValue::Object(object.as_obj())
}
