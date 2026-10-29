//! Real guest callbacks applied to the game's per-level entity adapter.
#![cfg(feature = "mods")]
#[path = "../src/game/mod_entities.rs"]
mod mod_entities;
use horizon_client::{chunk::Chunk, chunk_map::ChunkMap};
use horizon_mods::{
    EntityReason, GameMode, Mods, Output, Package, Runs, ServerPlayer, Status,
    entities::{World, WorldId},
};
use mod_entities::ModEntities;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

fn component(root: &Path, scratch: &Path) -> Vec<u8> {
    if let Some(path) = std::env::var_os("HORIZON_ENTITY_TEST_COMPONENT") {
        return std::fs::read(path).unwrap();
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("client-entity-guest");
    let status = Command::new(env!("CARGO"))
        .current_dir(root)
        .args([
            "build",
            "--locked",
            "-p",
            "cargo-horizon-mod",
            "--target-dir",
        ])
        .arg(target.join("cli"))
        .status()
        .unwrap();
    assert!(status.success(), "build the native component packer");
    std::fs::create_dir_all(scratch.join("src")).unwrap();
    let sdk = root.join("crates/horizon-mod-sdk");
    std::fs::write(scratch.join("Cargo.toml"),format!("[workspace]\n[package]\nname=\"entity-host-fixture\"\nversion=\"0.1.0\"\nedition=\"2024\"\n[lib]\ncrate-type=[\"cdylib\"]\n[dependencies]\nhorizon-mod-sdk={{path={:?}}}\n",sdk.to_str().unwrap())).unwrap();
    std::fs::copy(
        root.join("crates/horizon-mods/tests/entity_server_guest/src/lib.rs"),
        scratch.join("src/lib.rs"),
    )
    .unwrap();
    let binary = target
        .join("cli/debug")
        .join(format!("cargo-horizon-mod{}", std::env::consts::EXE_SUFFIX));
    assert!(
        Command::new(binary)
            .arg("build")
            .current_dir(scratch)
            .env("CARGO_TARGET_DIR", target.join("wasm"))
            .status()
            .unwrap()
            .success()
    );
    Package::read(&target.join("wasm/horizon-mod/entity_host.hmod"))
        .unwrap()
        .wasm
        .unwrap()
}
fn manifest() -> &'static str {
    "id=\"entity_host\"\nname=\"Entity host fixture\"\nversion=\"0.1.0\"\napi=\"0.3\"\nside=\"server\"\ncapabilities=[\"entities\",\"resources\",\"players\"]\nexperimental=[\"entities\",\"resources\"]\nnamespaces=[\"entity_lab\"]\n"
}
fn run(
    mods: &mut Mods,
    id: WorldId,
    next: &mut u32,
    levels: &mut [(&str, &mut ModEntities, &ChunkMap)],
    command: &str,
    dimension: &str,
) -> bool {
    let loaded = levels
        .iter_mut()
        .map(|(d, l, c)| l.loaded(d, -64, 320, c))
        .collect();
    mods.lend_entities(World::new(id, *next, loaded));
    let player = ServerPlayer {
        id: "00000000-0000-4000-8000-000000000001".into(),
        name: "EntityFixture".into(),
        x: 0.5,
        y: 64.0,
        z: 0.5,
        dimension: dimension.into(),
        game_mode: GameMode::Creative,
        health: 20.0,
    };
    assert!(mods.server_command(&player, command));
    *next = mods.take_entities().unwrap().last_id();
    for (_, l, _) in levels {
        l.prune();
    }
    !mods.drain().iter().any(|o| {
        matches!(
            o,
            Output::Reply { error: true, .. } | Output::Stopped { .. }
        )
    })
}
#[test]
fn real_guest_records_follow_parked_level_ownership_and_release() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let scratch = std::env::temp_dir().join(format!("he-client-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("horizon-mod.toml"), manifest()).unwrap();
    let wasm = component(&root, &scratch);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let nonce = now.to_le_bytes();
    assert!(!wasm.windows(nonce.len()).any(|w| w == nonce));
    let files = [
        (
            "data/entity_lab/config.txt",
            b"entity_host|entityhost|".as_slice(),
        ),
        ("data/entity_lab/nonce.bin", nonce.as_slice()),
        ("data/entity_lab/update.bin", b"new-state".as_slice()),
    ];
    let hmod = Package::write_files(manifest(), Some(&wasm), &files).unwrap();
    let path = scratch.join("entity.hmod");
    std::fs::write(&path, hmod).unwrap();
    let mut chunks = ChunkMap::default();
    chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let empty = ChunkMap::default();
    let mut current = ModEntities::default();
    let mut parked = ModEntities::default();
    assert!(!current.allocated() && !parked.allocated());
    let mut mods = Mods::load_for(Runs::Server, &[path], |_| true, |_| false);
    assert_eq!(mods.entries()[0].status, Status::Running);
    let mut id = WorldId::new().unwrap();
    let mut next = 400;
    assert!(run(
        &mut mods,
        id,
        &mut next,
        &mut [
            ("minecraft:overworld", &mut current, &chunks),
            ("minecraft:the_nether", &mut parked, &chunks)
        ],
        "entityhost spawn",
        "minecraft:the_nether"
    ));
    assert!(current.records().is_empty());
    assert_eq!(parked.records()[0].state.public_state, nonce);
    let handle = parked.records()[0].handle.clone();
    assert_eq!(next, 401);
    std::mem::swap(&mut current, &mut parked);
    assert!(run(
        &mut mods,
        id,
        &mut next,
        &mut [
            ("minecraft:the_nether", &mut current, &chunks),
            ("minecraft:overworld", &mut parked, &chunks)
        ],
        "entityhost update",
        "minecraft:the_nether"
    ));
    assert_eq!(current.records()[0].handle, handle);
    assert_eq!(current.records()[0].state.public_state, b"etats-wen");
    assert_eq!(current.records()[0].revision, 2);
    current.unload(&[(0, 0)].into_iter().collect());
    assert!(!current.allocated());
    assert!(!run(
        &mut mods,
        id,
        &mut next,
        &mut [("minecraft:the_nether", &mut current, &empty)],
        "entityhost spawn",
        "minecraft:the_nether"
    ));
    assert!(!current.allocated());
    for _ in 0..8 {
        id = WorldId::new().unwrap();
        assert!(run(
            &mut mods,
            id,
            &mut next,
            &mut [("minecraft:overworld", &mut current, &chunks)],
            "entityhost spawn",
            "minecraft:overworld"
        ));
        assert_eq!(current.records().len(), 1);
        current.clear(EntityReason::WorldUnloaded);
        assert!(current.records().is_empty());
        assert!(!current.allocated());
        assert!(!run(
            &mut mods,
            WorldId::new().unwrap(),
            &mut next,
            &mut [("minecraft:overworld", &mut current, &chunks)],
            &format!("entityhost foreign {handle}"),
            "minecraft:overworld"
        ));
    }
    assert!(run(
        &mut mods,
        id,
        &mut next,
        &mut [("minecraft:overworld", &mut current, &chunks)],
        "entityhost spawn",
        "minecraft:overworld"
    ));
    assert!(mods.disable("entity_host"));
    assert!(current.records().is_empty());
    current.prune();
    assert!(!current.allocated());
    mod_entities::trace(
        None,
        next,
        &[
            ("minecraft:overworld", &current),
            ("minecraft:the_nether", &parked),
        ],
    );
    std::fs::remove_dir_all(scratch).unwrap();
}
