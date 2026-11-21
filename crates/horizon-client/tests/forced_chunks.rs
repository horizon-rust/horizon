//! A `/forceload`ed chunk reaches vanilla's rings (#2785): vanilla holds a
//! FORCED ticket at level 31 (`ChunkMap.FORCED_TICKET_LEVEL`), the level
//! rises one per ring (`ChunkTracker.computeLevelFromNeighbor`) and picks the
//! column's status (`ChunkLevel.generationStatus`). The chunk itself and the
//! two rings around it are level chunks with their light, the third ring has
//! its features and the fourth only its terrain.
extern crate horizon_client_lib as horizon_client;
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::Chunk,
    level_loading::Status,
    light::LightTables,
    mesher::MeshTables,
    models,
    section_mask::SectionMask,
    streaming::{ChunkGenerator, Proto, Streaming, WorkerContext},
    world::World,
};
use horizon_world::registry::BlockStateId;
use std::{
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn resources() -> Option<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    root.join("items.json").exists().then_some(root)
}

/// A column under generation, and whether it has its features.
struct Column {
    x: i32,
    z: i32,
    decorated: bool,
}

/// A generator that makes every column: a proto, decorated on request and
/// finished into a level chunk. One column can fail its generation, as a
/// generator reports one that threw (`ChunkGenerator::generated`), and one can
/// come from the save.
struct Plain {
    failed: Option<(i32, i32)>,
    saved: Option<(i32, i32)>,
}

impl ChunkGenerator for Plain {
    type Worker = ();
    fn worker(&self) {}
    /// The one column the save already holds: the barrier must load it even
    /// when a failed generation sits one chunk away.
    fn load(&self, x: i32, z: i32) -> Option<Chunk> {
        if self.saved == Some((x, z)) {
            Some(Chunk::empty(x, z))
        } else {
            None
        }
    }
    fn proto(&self, _: &mut (), x: i32, z: i32) -> Proto {
        Box::new(Column {
            x,
            z,
            decorated: false,
        })
    }
    fn generated(&self, proto: &Proto) -> bool {
        proto
            .downcast_ref::<Column>()
            .is_some_and(|c| Some((c.x, c.z)) != self.failed)
    }
    fn decorates(&self) -> bool {
        true
    }
    /// The centre gets its features, as vanilla's FEATURES step does; the
    /// neighbours are only written to.
    fn decorate(&self, _: &mut (), region: &mut [Proto; 9]) {
        region[4].downcast_mut::<Column>().unwrap().decorated = true;
    }
    fn is_decorated(&self, proto: &Proto) -> bool {
        proto.downcast_ref::<Column>().is_some_and(|c| c.decorated)
    }
    fn finish(&self, proto: Proto) -> Chunk {
        let column = *proto.downcast::<Column>().unwrap();
        Chunk::empty(column.x, column.z)
    }
    fn light_view(&self, proto: &Proto) -> Option<Chunk> {
        let column = proto.downcast_ref::<Column>()?;
        Some(Chunk::empty(column.x, column.z))
    }
    fn non_air_sections(&self, proto: &Proto) -> Option<SectionMask> {
        proto.downcast_ref::<Column>()?;
        Some(SectionMask::single(4))
    }
}

fn worker_context(root: &Path) -> Arc<WorkerContext> {
    let assets = Arc::new(Assets::load(root).unwrap());
    let render = Arc::new(
        assets
            .reload(&horizon_assets::resource_source::ResourceRoot::directory(
                root.to_path_buf(),
            ))
            .unwrap(),
    );
    let facts = BlockFacts::load(root, assets.registry).unwrap();
    let visibility = models::load_visibility(root, assets.registry).unwrap();
    Arc::new(WorkerContext {
        tables: Arc::new(MeshTables::new(&assets, &render, &facts, visibility, None).unwrap()),
        light: Arc::new(LightTables::load(root, assets.registry, &facts).unwrap()),
        biomes: Arc::new(
            Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
                root.to_path_buf(),
            ))
            .unwrap(),
        ),
        assets,
        render: render.clone(),
        hidden_states: Arc::from([]),
        part_tables: Default::default(),
    })
}

/// One forced load barrier in a thread of its own: without the failure exit
/// it never returns, and the test has to fail rather than hang.
fn forced_load(
    mut streaming: Streaming,
    mut world: World,
    (x, z): (i32, i32),
    never: &str,
) -> (World, Streaming) {
    let (tx, rx) = std::sync::mpsc::channel();
    let barrier = thread::spawn(move || {
        let changed = streaming.set_forced_loaded(&mut world, x, z, true);
        let _ = tx.send((changed, world, streaming));
    });
    let (_changed, world, streaming) = rx.recv_timeout(Duration::from_secs(20)).expect(never);
    barrier.join().unwrap();
    (world, streaming)
}

#[test]
fn forced_chunks_reach_vanillas_rings() {
    let Some(root) = resources() else {
        return;
    };
    let context = worker_context(&root);
    let mut streaming = Streaming::new(
        Arc::new(Plain {
            failed: None,
            saved: None,
        }),
        context,
        2,
    );
    // The player stands far away: only the forced chunk's own rings are
    // generated.
    streaming.center = (0, 0);
    streaming.radius = 2;
    streaming.forced.insert((40, -40));
    let mut world = World::new(BlockStateId(0), 0);
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut quiet = 0;
    while quiet < 20 {
        assert!(Instant::now() < deadline, "streaming never settled");
        let before = world.chunks.len();
        let _ = streaming.update(&mut world);
        thread::sleep(Duration::from_millis(5));
        let idle = !streaming.busy() && before == world.chunks.len();
        quiet = if idle { quiet + 1 } else { 0 };
    }
    // The chunk itself and the two rings around it are level chunks with
    // their light.
    for (dx, dz) in (-2..=2).flat_map(|dx| (-2..=2).map(move |dz| (dx, dz))) {
        let (x, z) = (40 + dx, -40 + dz);
        assert!(
            world.chunks.contains_key(&(x, z)),
            "({x}, {z}) is not a level chunk"
        );
        assert!(streaming.is_lit(x, z), "({x}, {z}) has no light");
    }
    // The third ring has its features, the fourth only its terrain.
    for (dx, dz) in (-4i32..=4).flat_map(|dx| (-4i32..=4).map(move |dz| (dx, dz))) {
        let (x, z) = (40 + dx, -40 + dz);
        let ring = dx.abs().max(dz.abs());
        let status = streaming.latest_status(&world, x, z);
        // A level chunk's light layers count a neighbour at FEATURES as
        // vanilla's do one at INITIALIZE_LIGHT (#4294); not one at TERRAIN.
        let proto = streaming.proto_sections(x, z);
        if ring == 3 {
            assert_eq!(status, Some(Status::Features), "({x}, {z})");
            assert_eq!(proto, Some(SectionMask::single(4)), "({x}, {z})");
        } else if ring == 4 {
            assert_eq!(status, Some(Status::Terrain), "({x}, {z})");
            assert_eq!(proto, None, "({x}, {z})");
        } else {
            assert_eq!(proto, None, "({x}, {z})");
        }
    }
}

/// The load barrier of a forced chunk ends when the generation it needs
/// failed: the mark never becomes a column, and the barrier runs no sweep, the
/// only place a mark is dropped, so waiting again would never end (#18403).
/// It pumps the queue first, so a column the save holds still loads beside a
/// failure.
#[test]
fn a_forced_load_ends_when_its_generation_failed() {
    let Some(root) = resources() else {
        return;
    };
    let context = worker_context(&root);
    let generator = Arc::new(Plain {
        failed: Some((40, -40)),
        saved: Some((40, -41)),
    });
    let mut streaming = Streaming::new(generator, context, 2);
    streaming.center = (0, 0);
    streaming.radius = 2;
    let world = World::new(BlockStateId(0), 0);
    // The failed generation is not finished into an empty column.
    let (world, streaming) = forced_load(
        streaming,
        world,
        (40, -40),
        "the forced load barrier never returned",
    );
    assert!(
        !world.chunks.contains_key(&(40, -40)),
        "a failed generation became a column"
    );
    // A column beside it needs the failed one's decoration, so its own
    // barrier has to end too.
    let (world, streaming) = forced_load(
        streaming,
        world,
        (41, -40),
        "the forced load beside the failed column never returned",
    );
    // The barrier loads a column the save holds before it gives up: a saved
    // column needs no decoration, so a failed neighbour must not stop it.
    let (world, _streaming) = forced_load(
        streaming,
        world,
        (40, -41),
        "the forced load of a saved column never returned",
    );
    assert!(
        world.chunks.contains_key(&(40, -41)),
        "a saved column was dropped because a neighbour's generation failed"
    );
}
