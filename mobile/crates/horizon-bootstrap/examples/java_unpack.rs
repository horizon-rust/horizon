//! `cargo run --release -p horizon-bootstrap --example java_unpack -- DEB PREFIX DEST`: unpack a
//! Termux .deb the way the app does, to check it from a computer.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let started = std::time::Instant::now();
    let n = horizon_bootstrap::java::extract_deb(
        std::path::Path::new(&a[0]),
        &a[1],
        std::path::Path::new(&a[2]),
        &|_| true,
    );
    println!("{n:?} in {:?}", started.elapsed());
}
