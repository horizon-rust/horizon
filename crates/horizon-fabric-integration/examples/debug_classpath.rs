//! Debug helper: prove the Fabric class loader can load a mod class from a jar.

use std::path::PathBuf;

use horizon_fabric_jvm::{FabricJvm, JvmConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".local")
        .join("fabric-smoke");
    let api = root.join("out").join("fabric-api-stub.jar");
    let modjar = root.join("out").join("horizon-smoketest.jar");

    let mut config = JvmConfig::for_system_jvm()?;
    config.heap_max = "256m".into();
    config.classpath = api.clone();

    let jvm = FabricJvm::new(config)?;
    jvm.start()?;
    println!("classpath before: {:?}", jvm.class_loader().classpath());

    jvm.add_classpath(std::slice::from_ref(&modjar))?;
    println!("classpath after: {:?}", jvm.class_loader().classpath());

    let vm = jvm.vm().ok_or("no vm")?;
    match jvm.class_loader().load_class(&vm, "dev.horizonlab.smoketest.SmokeMod") {
        Ok(_) => println!("loaded mod class: OK"),
        Err(e) => println!("loaded mod class: FAILED ({e})"),
    }
    match jvm.class_loader().load_class(&vm, "net.fabricmc.api.ModInitializer") {
        Ok(_) => println!("loaded api class: OK"),
        Err(e) => println!("loaded api class: FAILED ({e})"),
    }

    jvm.shutdown()?;
    Ok(())
}
