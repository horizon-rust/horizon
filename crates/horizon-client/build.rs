//! Generates the native mod API. The launcher's native-mod game package (`horizon-native-game`,
//! which owns the `horizon-client` executable of a build with native mods) runs this build script
//! too: there it also gives the Windows executable Horizon's icon and its stack
//! (`packaging/windows/exe.rs`). horizon-client itself has no executable (horizon-game's).
#[path = "../../packaging/windows/exe.rs"]
mod windows_exe;

fn main() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    horizon_mod_api_gen::generate(&root, &out_dir, horizon_mod_api_gen::Target::Client)
        .expect("generating native mod API");
    // A workspace horizon-mixins changed holds native mods, whichever tool ran it (the launcher,
    // or a mod's own script with no `native-mods.json` beside its game): `HORIZON_MIXIN_MODS`
    // names them for `game::mods::built_with_native_mods`. Watched only once it exists: a
    // bundle applied in place also edits `game/`, which `generate` watches.
    let stage = root.join(horizon_mod_api_gen::MIXINS_STAGE);
    if stage.is_file() {
        println!("cargo:rerun-if-changed={}", stage.display());
    }
    let staged = horizon_mod_api_gen::staged_mods(&root);
    if !staged.is_empty() {
        println!("cargo:rustc-env=HORIZON_MIXIN_MODS={}", staged.join(","));
    }
    // Whether a native mod can run in a mob's turn (`game/creatures/mob_islands.rs`). The
    // launcher adds trait packages to the workspace's members.
    println!(
        "cargo:rerun-if-changed={}",
        root.join("Cargo.toml").display()
    );
    for dir in ["native-api", "native-crates"] {
        if root.join(dir).is_dir() {
            println!("cargo:rerun-if-changed={}", root.join(dir).display());
        }
    }
    if let Some(found) = horizon_mod_api_gen::mob_turn(&root) {
        println!("cargo:rustc-env=HORIZON_MOB_TURN={found}");
    }
    if std::env::var("CARGO_PKG_NAME").as_deref() != Ok("horizon-client") {
        windows_exe::resources("horizon-client");
    }
}
