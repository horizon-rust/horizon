//! Embeds Horizon's icon (`packaging/icons/horizon.ico`) in the Windows executables, as icon
//! resource 1: Explorer and the task bar show it, and the window takes it as its icon.
//! The optional Segment Heap manifest is embedded in the same resource compilation.
/// The vanilla exporters' Java sources (`mods/horizon/src/test/java/dev/horizonlab/parity`), packed
/// into `$OUT_DIR/exporters.tar.gz` for the Android build, which compiles and runs them on the
/// device to prepare the game files (game/mobile_launcher.rs). Other builds get an empty file.
fn pack_exporters(dir: &str) {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default())
        .join("exporters.tar.gz");
    let source = format!("{dir}/../../mods/horizon/src/test/java/dev/horizonlab/parity");
    println!("cargo:rerun-if-changed={source}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
        std::fs::write(&out, []).expect("writing the empty exporter archive");
        return;
    }
    let file = std::fs::File::create(&out).expect("creating the exporter archive");
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        file,
        flate2::Compression::best(),
    ));
    let mut names: Vec<_> = std::fs::read_dir(&source)
        .unwrap_or_else(|e| panic!("reading {source}: {e}"))
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "java"))
        .map(|e| e.path())
        .collect();
    names.sort();
    for path in names {
        let name = path.file_name().expect("a file name").to_owned();
        tar.append_path_with_name(&path, &name)
            .expect("adding an exporter source");
    }
    tar.into_inner()
        .and_then(|gz| gz.finish())
        .expect("finishing the exporter archive");
}

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    pack_exporters(&dir);
    let ico = format!("{dir}/../../packaging/icons/horizon.ico");
    println!("cargo:rerun-if-changed={ico}");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(&ico);
        if std::env::var_os("CARGO_FEATURE_WINDOWS_SEGMENT_HEAP").is_some() {
            let manifest = format!("{dir}/windows-segment-heap.manifest");
            println!("cargo:rerun-if-changed={manifest}");
            resource.set_manifest_file(&manifest);
        }
        if let Err(e) = resource.compile() {
            panic!("embedding Windows resources: {e}");
        }
    }
}
