//! Smoke test: start a real JVM and call into it.
//!
//! Run with: `cargo run -p horizon-fabric-jvm --example smoke`

use horizon_fabric_jvm::{FabricJvm, JvmConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = JvmConfig::for_system_jvm()?;
    config.heap_max = "256m".into();
    config.gc = horizon_fabric_jvm::GcConfig::G1 {
        max_pause_ms: Some(50),
    };

    println!("JVM home: {}", config.jvm_path.display());

    let jvm = FabricJvm::new(config)?;
    jvm.start()?;
    println!("JVM started: {}", jvm.is_started());

    let version = jvm.with_env(|env| {
        let system = env.find_class("java/lang/System").map_err(|e| {
            horizon_fabric_jvm::JvmError::Jni(e.to_string())
        })?;
        let key = env
            .new_string("java.version")
            .map_err(|e| horizon_fabric_jvm::JvmError::Jni(e.to_string()))?;
        let value = env
            .call_static_method(
                system,
                "getProperty",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[jni::objects::JValue::Object(&key)],
            )
            .map_err(|e| horizon_fabric_jvm::JvmError::Jni(e.to_string()))?;
        let value = value
            .l()
            .map_err(|e| horizon_fabric_jvm::JvmError::Jni(e.to_string()))?;
        let string = jni::objects::JString::from(value);
        let chars = env
            .get_string(&string)
            .map_err(|e| horizon_fabric_jvm::JvmError::Jni(e.to_string()))?;
        Ok(chars.to_string_lossy().into_owned())
    })?;

    println!("java.version = {version}");
    println!("class loader initialized: {}", jvm.class_loader().is_initialized());

    jvm.shutdown()?;
    Ok(())
}
