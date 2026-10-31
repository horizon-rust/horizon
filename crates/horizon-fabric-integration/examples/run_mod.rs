//! End-to-end smoke test: start the JVM, load a real compiled Fabric mod jar,
//! and run its entrypoints.
//!
//! Build the fixtures first (see `docs/fabric-bridge.md`), then:
//! `cargo run -p horizon-fabric-integration --example run_mod`

use std::path::PathBuf;

use horizon_fabric_integration::FabricIntegration;
use horizon_fabric_jvm::JvmConfig;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".local")
        .join("fabric-smoke");

    let api_jar = fixture_root.join("out").join("fabric-api-stub.jar");
    let mods_dir = fixture_root.join("out");

    if !api_jar.exists() {
        eprintln!("missing fixture {}; build it with the javac/jar steps first", api_jar.display());
        return Err("fixture not built".into());
    }

    let mut config = JvmConfig::for_system_jvm()?;
    config.heap_max = "256m".into();
    config.classpath = api_jar.clone();

    println!("JVM: {}", config.jvm_path.display());
    println!("API jar: {}", api_jar.display());

    let mut fabric = FabricIntegration::with_config(config)?;
    println!("JVM started: {}", fabric.jvm().is_started());

    fabric.initialize(&mods_dir)?;

    println!("mods discovered: {}", fabric.mod_count());
    println!("load order: {:?}", fabric.load_order());
    println!("fabric api initialized: {}", fabric.api().is_initialized());
    println!("mixin initialized: {}", fabric.mixin().is_initialized());
    println!(
        "mixin configs parsed: {:?}",
        fabric
            .loader()
            .read()
            .mixin_bootstrap()
            .parsed_configs()
            .iter()
            .map(|config| config.mixin_classes())
            .collect::<Vec<_>>()
    );

    // One tick through the Rust-side event bus.
    println!("tick listeners fired: {}", fabric.tick()?);

    fabric.shutdown()?;
    Ok(())
}
