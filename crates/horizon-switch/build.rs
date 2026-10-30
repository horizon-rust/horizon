//! Links what a Horizon program needs, on `target_os = "horizon"` only: libnx and newlib from
//! devkitPro, and NVK, mesa-switch's Vulkan driver (`packaging/switch/build.sh nvk`), as
//! bevy-horizon's `build.rs` does. Link arguments of a library do not reach the binary, so
//! `horizon-client`'s own `build.rs` adds `-z notext` for it.
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=HORIZON_NVK_LIB");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("horizon") {
        return;
    }
    for dir in [
        "/opt/devkitpro/libnx/lib",
        "/opt/devkitpro/portlibs/switch/lib",
        "/opt/devkitpro/devkitA64/aarch64-none-elf/lib/pic",
        "/opt/devkitpro/devkitA64/lib/gcc/aarch64-none-elf/16.1.0/pic",
    ] {
        println!("cargo:rustc-link-search=native={dir}");
    }
    for lib in ["nx", "sysbase", "c", "m", "gcc"] {
        println!("cargo:rustc-link-lib={lib}");
    }

    // NVK as scripts/localize_nvk.sh leaves it: one object whose only globals are the vk* entry
    // points (and bevy-horizon's allocator hooks), so its copy of Rust's std cannot clash with ours.
    let dir = std::env::var_os("HORIZON_NVK_LIB")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../bevy-horizon/nvk/lib"));
    if !dir.join("libvulkan_bh.a").is_file() {
        panic!(
            "{} is missing: run packaging/switch/build.sh nvk, or set HORIZON_NVK_LIB to the folder that has libvulkan_bh.a",
            dir.join("libvulkan_bh.a").display()
        );
    }
    println!(
        "cargo:rerun-if-changed={}",
        dir.join("libvulkan_bh.a").display()
    );
    println!("cargo:rustc-link-search=native={}", dir.display());
    for lib in ["vulkan_bh", "expat", "zstd", "z", "stdc++"] {
        println!("cargo:rustc-link-lib={lib}");
    }
}
