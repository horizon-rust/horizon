//! `cargo run -p horizon-bootstrap --example fetch -- DIR [--assets]`: the Mojang download
//! without the app, to check it from a computer.
use std::sync::atomic::AtomicBool;

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    // `fetch --list VERSION.json`: what a version file would download, without downloading.
    if args.next_if(|a| a == "--list").is_some() {
        let file = args.next().expect("usage: fetch --list VERSION.json");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        let items =
            horizon_bootstrap::download::game_items(&v, std::path::Path::new("/mc"), "26.3")
                .unwrap();
        for i in &items {
            println!("{} {} {}", i.size, i.sha1, i.dest.display());
        }
        println!(
            "{} files, {} MB",
            items.len(),
            items.iter().map(|i| i.size).sum::<u64>() >> 20
        );
        return;
    }
    let dir = args.next().expect("usage: fetch DIR [--assets]");
    let assets = args.any(|a| a == "--assets");
    let cancel = AtomicBool::new(false);
    let result = horizon_bootstrap::download::download(
        std::path::Path::new(&dir),
        horizon_bootstrap::download::VERSION,
        assets,
        &cancel,
        &|p| {
            eprintln!(
                "{}: {}/{} files, {}/{} MB",
                p.stage,
                p.files_done,
                p.files_total,
                p.bytes_done >> 20,
                p.bytes_total >> 20
            )
        },
    );
    println!("{result:?}");
}
