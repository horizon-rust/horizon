//! What Smooth fluids (`mesher/fluid_surface.rs`) cost against vanilla's fluid
//! cells, on a saved world: loads the columns around a point, then meshes, on
//! this one thread, every section of the inner columns that holds a fluid face,
//! in both modes, alternating rounds so the machine's load hits both alike.
//!
//!     cargo run --profile fast -p horizon-client --example fluid_mesh_bench -- \
//!         .local/horizon-client/26.3 saves/WORLD [--center CX CZ] [--radius 6]
//!         [--rounds 5]
//!
//! Times are this thread's CPU time in `Mesher::mesh` (best of `--rounds`);
//! quads are those of the five fluid sprites. The vanilla checksum is the one
//! `chunk_load_bench`-style A/B pairs compare.
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::HEIGHT,
    mesher::{MeshTables, Mesher, Region, SectionMesh, borrow},
    models,
    save::{self, SaveDir},
    spans::thread_cpu,
};
use horizon_core::biome::BiomeZoom;
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

type Error = Box<dyn std::error::Error>;

fn flag<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1)?.parse().ok())
        .unwrap_or(default)
}

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().collect();
    let usage = "usage: fluid_mesh_bench <resources> <world>";
    let root = PathBuf::from(args.get(1).ok_or(usage)?);
    let world = PathBuf::from(args.get(2).ok_or(usage)?);
    let radius: i32 = flag(&args, "--radius", 6);
    let rounds: usize = flag(&args, "--rounds", 5);
    let center = match args.iter().position(|a| a == "--center") {
        Some(i) => (
            args.get(i + 1).ok_or("--center CX CZ")?.parse()?,
            args.get(i + 2).ok_or("--center CX CZ")?.parse()?,
        ),
        None => (4, 3),
    };

    let assets = Assets::load(&root)?;
    let render = assets.reload(&root)?;
    let facts = BlockFacts::load(&root, assets.registry)?;
    let visibility = models::load_visibility(&root, assets.registry)?;
    let biomes = Biomes::load(&root)?;
    let tables = MeshTables::new(&assets, &render, &facts, visibility)?;
    let fluid_layers: Vec<u16> = ["water_still", "water_flow", "water_overlay", "lava_still"]
        .into_iter()
        .chain(["lava_flow"])
        .filter_map(|n| render.sprite_layers.get(&format!("minecraft:block/{n}")))
        .map(|&l| l as u16)
        .collect();

    let save = SaveDir::new(save::dimension_dir(&world, "minecraft:overworld"));
    let mut loaded = std::collections::HashMap::new();
    for x in center.0 - radius..=center.0 + radius {
        for z in center.1 - radius..=center.1 + radius {
            if let Some(chunk) = save.load(x, z, assets.registry, &biomes) {
                loaded.insert((x, z), Arc::new(chunk));
            }
        }
    }
    let inner = radius - 1;
    let regions: Vec<(Region, i32)> = (center.0 - inner..=center.0 + inner)
        .flat_map(|x| (center.1 - inner..=center.1 + inner).map(move |z| (x, z)))
        .filter_map(|(x, z)| {
            let region: Region = std::array::from_fn(|dz| {
                std::array::from_fn(|dx| {
                    loaded.get(&(x + dx as i32 - 1, z + dz as i32 - 1)).cloned()
                })
            });
            region
                .iter()
                .flatten()
                .all(Option::is_some)
                .then_some(region)
        })
        .flat_map(|region| (0..HEIGHT / 16).map(move |sy| (region.clone(), sy)))
        .collect();

    let fluid_quads = |mesh: &SectionMesh| {
        [&mesh.solid, &mesh.translucent]
            .into_iter()
            .flat_map(|v| v.as_chunks::<4>().0)
            .filter(|q| fluid_layers.contains(&q[0].layer))
            .count()
    };
    let mesher = |smooth_fluids: bool| Mesher {
        nether_shading: false,
        zoom: BiomeZoom::new(0),
        blend_radius: 2,
        smooth_lighting: true,
        cutout_leaves: true,
        smooth_fluids,
        rt_lighting: false,
        assets: &assets,
        render: &render,
        tables: &tables,
        biomes: &biomes,
        visibility: tables.visibility.clone(),
    };
    // Sections with fluid faces, found in vanilla mode.
    let mut vanilla = mesher(false);
    let wet: Vec<&(Region, i32)> = regions
        .iter()
        .filter(|(region, sy)| fluid_quads(&vanilla.mesh(&borrow(region), *sy)) > 0)
        .collect();
    println!(
        "{} columns loaded, {} sections, {} with fluid faces",
        loaded.len(),
        regions.len(),
        wet.len()
    );
    let mut best = [Duration::MAX; 2];
    let mut stats = [(0usize, 0usize, 0u64); 2];
    for _ in 0..rounds.max(1) {
        for (mode, smooth) in [false, true].into_iter().enumerate() {
            let mut m = mesher(smooth);
            let (mut quads, mut fluid, mut hash) = (0, 0, DefaultHasher::new());
            let mut spent = Duration::ZERO;
            for (region, sy) in &wet {
                let started = thread_cpu().ok_or("no thread CPU clock here")?;
                let mesh = m.mesh(&borrow(region), *sy);
                spent += thread_cpu().ok_or("no thread CPU clock here")? - started;
                quads += (mesh.solid.len() + mesh.translucent.len()) / 4;
                fluid += fluid_quads(&mesh);
                bytemuck::cast_slice::<_, u8>(&mesh.solid).hash(&mut hash);
                bytemuck::cast_slice::<_, u8>(&mesh.translucent).hash(&mut hash);
            }
            best[mode] = best[mode].min(spent);
            stats[mode] = (quads, fluid, hash.finish());
        }
    }
    let n = wet.len().max(1) as f64;
    for (mode, name) in ["vanilla", "smooth "].into_iter().enumerate() {
        let (quads, fluid, checksum) = stats[mode];
        println!(
            "{name}: {:.1} ms CPU ({:.1} us per section), {quads} quads ({} triangles), {fluid} fluid quads, checksum {checksum:016x}",
            best[mode].as_secs_f64() * 1e3,
            best[mode].as_secs_f64() * 1e6 / n,
            quads * 2,
        );
    }
    println!(
        "smooth fluids: {:+.1}% meshing CPU on these sections",
        (best[1].as_secs_f64() / best[0].as_secs_f64() - 1.0) * 100.0
    );
    Ok(())
}
