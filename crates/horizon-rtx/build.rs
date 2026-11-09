//! Links NVIDIA's NGX static library for DLSS Ray Reconstruction (`dlss-rr` feature,
//! docs/rt-dlss-rr.md). Without the feature this does nothing and the crate builds as it did.
//!
//! With the feature, `DLSS_SDK_DIR` names a local copy of NVIDIA's DLSS SDK (github.com/NVIDIA/DLSS,
//! never committed here) and the build links `nvsdk_ngx` from its `lib/` folder. A missing or
//! wrong `DLSS_SDK_DIR` fails the build with a message saying so, except with
//! `HORIZON_DLSS_RR_NO_LINK=1`: then nothing is linked, which is enough for `cargo check` and
//! `cargo clippy` on a machine without the SDK (a binary that calls NGX then fails to link).

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_DLSS_RR").is_none() {
        return;
    }
    println!("cargo:rerun-if-env-changed=DLSS_SDK_DIR");
    println!("cargo:rerun-if-env-changed=HORIZON_DLSS_RR_NO_LINK");
    let no_link = std::env::var("HORIZON_DLSS_RR_NO_LINK").is_ok_and(|v| v == "1");
    let Some(sdk) = std::env::var_os("DLSS_SDK_DIR").map(PathBuf::from) else {
        if no_link {
            println!(
                "cargo:warning=horizon-rtx: the dlss-rr feature without DLSS_SDK_DIR \
                 (HORIZON_DLSS_RR_NO_LINK=1): NGX is not linked, type-checking only"
            );
            return;
        }
        panic!(
            "horizon-rtx: the `dlss-rr` feature needs NVIDIA's DLSS SDK: set DLSS_SDK_DIR to a \
             local copy of github.com/NVIDIA/DLSS (the folder holding `include/` and `lib/`), \
             see docs/rt-dlss-rr.md. To only type-check without it, set HORIZON_DLSS_RR_NO_LINK=1."
        );
    };
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let crt_static = std::env::var("CARGO_CFG_TARGET_FEATURE")
        .is_ok_and(|f| f.split(',').any(|f| f == "crt-static"));
    // The SDK's layout (as of its 310.x releases): `lib/Windows_x86_64/x64/nvsdk_ngx_{d,s}.lib`
    // (`_d` for the dynamic C runtime Rust's MSVC targets use by default, `_s` for crt-static),
    // `lib/Linux_x86_64/libnvsdk_ngx.a`, which is C++ and needs libstdc++ and libdl.
    let (dir, file, name) = match os.as_str() {
        "windows" if crt_static => ("lib/Windows_x86_64/x64", "nvsdk_ngx_s.lib", "nvsdk_ngx_s"),
        "windows" => ("lib/Windows_x86_64/x64", "nvsdk_ngx_d.lib", "nvsdk_ngx_d"),
        "linux" => ("lib/Linux_x86_64", "libnvsdk_ngx.a", "nvsdk_ngx"),
        other => panic!("horizon-rtx: DLSS Ray Reconstruction has no NGX library for {other}"),
    };
    let dir = sdk.join(dir);
    if !dir.join(file).is_file() {
        if no_link {
            println!(
                "cargo:warning=horizon-rtx: no {} (HORIZON_DLSS_RR_NO_LINK=1): NGX is not linked",
                dir.join(file).display()
            );
            return;
        }
        panic!(
            "horizon-rtx: DLSS_SDK_DIR={} holds no {}: point it at the DLSS SDK's root folder",
            sdk.display(),
            dir.join(file).display()
        );
    }
    if no_link {
        return;
    }
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static={name}");
    if os == "linux" {
        println!("cargo:rustc-link-lib=dylib=stdc++");
        println!("cargo:rustc-link-lib=dylib=dl");
    }
}
