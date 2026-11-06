//! Clean rounds against the serial walk, through the real mob tick: the same
//! scene runs on each path from the same start, and what the walk leaves is the
//! expected result.
use super::*;
use horizon_client::mobs::region_tick::{NotClean, RegionStats};

/// One path's run of a scene.
struct Run {
    /// What each walk finished with (`ServerCoordination::walk_log`).
    walks: Vec<String>,
    /// The mobs and the level random after each whole tick.
    ends: Vec<String>,
    stats: Vec<RegionStats>,
}
impl Run {
    fn clean_rounds(&self) -> usize {
        self.stats.iter().map(|s| s.clean_rounds).sum()
    }
    fn fallback_rounds(&self) -> usize {
        self.stats.iter().map(|s| s.fallback_rounds).sum()
    }
    fn fell_back_for(&self, reason: NotClean) -> bool {
        self.stats.iter().any(|s| s.fallback == Some(reason))
    }
    /// Whether any walk's output `name` holds anything.
    fn any(&self, name: &str) -> bool {
        let empty = [format!("\n{name} []\n"), format!("\n{name} {{}}\n")];
        self.walks.iter().any(|walk| {
            assert!(walk.contains(&format!("\n{name} ")), "no {name} in the log");
            !empty.iter().any(|empty| walk.contains(empty))
        })
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Path {
    Walk,
    Clean,
    /// The walk answers, with the clean path run over copies of every clean round.
    Verify,
}

struct Scene {
    name: &'static str,
    chunks: &'static [(i32, i32)],
    ticks: usize,
    /// The mobs in slot order, with whatever makes them interesting.
    mobs: Vec<(&'static str, [f64; 3], fn(&mut Mob))>,
    /// The mobs whose deaths the level already knows of.
    died: &'static [u32],
    /// Where a guest stands, if the level has one.
    guest_at: Option<[f64; 3]>,
}

fn stage(game: &mut Game<'_>, scene: &Scene, path: Path) {
    game.simulation_server.force_workers(3);
    game.simulation_server
        .force_commit(path != Path::Walk, path == Path::Verify);
    // Above the scene's ids: a duplicate id turns predictions off altogether.
    game.simulation_server.next_entity = 10_000;
    game.lan = None;
    let air = game.res.facts().air;
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    let mut world = horizon_client::world::World::new(air, 0);
    world.game_time = 1000;
    game.simulation = simulation::LevelSimulation::new(world, 0);
    for &chunk in scene.chunks {
        let empty = horizon_client::chunk::Chunk::empty(chunk.0, chunk.1);
        game.simulation
            .world
            .chunks
            .insert(chunk, std::sync::Arc::new(empty));
        for x in 0..16 {
            for z in 0..16 {
                let (x, z) = (chunk.0 * 16 + x, chunk.1 * 16 + z);
                game.simulation.world.set(x, 63, z, stone).unwrap();
            }
        }
    }
    game.simulation.died.extend(scene.died.iter().copied());
    for (i, &(kind, at, make)) in scene.mobs.iter().enumerate() {
        let kind = game.res.mob_types().get(kind).cloned().unwrap();
        let at = Vec3::new(at[0], at[1], at[2]);
        let mut mob = Mob::new(i as u32 + 1, kind, at, 0.0);
        mob.persistent = true;
        mob.previous = at;
        make(&mut mob);
        game.simulation.mobs.push(mob);
    }
    if let Some(at) = scene.guest_at {
        host_a_guest(game, at);
    }
}

/// Opens the level to a guest who has just arrived at `at`.
fn host_a_guest(game: &mut Game<'_>, at: [f64; 3]) {
    use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    let net = NetConfig {
        announce: false,
        ..NetConfig::lan(0, "clean commit".into(), 2)
    };
    let config = HubConfig {
        default_game_mode: 0,
        guest_commands: false,
        force_game_mode: false,
        hardcore: false,
        view_distance: 8,
        simulation_distance: 8,
        dimension: game.dim.id().to_string(),
        allow_flight: false,
        seed: 0,
        max_players: 2,
        integrated: false,
        levels: Default::default(),
    };
    let settings = lan::LanSettings {
        port: 0,
        guest_commands: false,
        force_game_mode: false,
    };
    game.lan_host(settings, net, config).unwrap();
    // Nothing listens to the guest; the sender stays open for the whole run.
    let (sender, receiver) = std::sync::mpsc::channel();
    std::mem::forget(receiver);
    let mut guest = Guest::new(
        Joined {
            conn: 1,
            uuid: 1,
            name: "Guest".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: "vanilla".into(),
            sender: PlayerSender::new(sender, std::sync::Arc::default()),
            latency: std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        900,
        at,
        8,
    );
    guest.placed = true;
    guest.loaded = true;
    guest.dimension = game.dim.id().to_string();
    let host = game.lan.as_mut().unwrap();
    host.hub.guests.push(guest);
}

fn run(game: &mut Game<'_>, scene: &Scene, path: Path) -> Run {
    stage(game, scene, path);
    let (mut ends, mut stats) = (Vec::new(), Vec::new());
    for _ in 0..scene.ticks {
        // Goals check their intervals against it (`MeleeAttack.can_use`).
        game.simulation.world.game_time += 1;
        game.tick_creature_entities().unwrap();
        stats.push(game.simulation_server.stats);
        ends.push(format!(
            "mobs {:?}\nrandom {:?}",
            *game.simulation.mobs, game.simulation.world.random
        ));
        game.simulation_server.retire_predictions();
    }
    Run {
        walks: game.simulation_server.walk_log.take().unwrap_or_default(),
        ends,
        stats,
    }
}

/// A log without the mobs' `JoinSeq`s, which a process-wide counter hands out:
/// each run of a scene starts where the last stopped.
fn without_joins(log: &str) -> String {
    const JOIN: &str = "join: JoinSeq(";
    let mut out = String::with_capacity(log.len());
    let mut rest = log;
    while let Some(at) = rest.find(JOIN) {
        out.push_str(&rest[..at]);
        rest = &rest[at + JOIN.len()..];
        rest = &rest[rest.find(')').unwrap() + 1..];
    }
    out.push_str(rest);
    out
}

/// The first place two logs differ, enough of it to see what differs.
fn assert_same(scene: &str, what: &str, tick: usize, actual: &str, expected: &str) {
    let (actual, expected) = (&*without_joins(actual), &*without_joins(expected));
    if actual != expected {
        let at = actual
            .bytes()
            .zip(expected.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(actual.len().min(expected.len()));
        let part = |text: &'_ str| {
            let bytes = text.as_bytes();
            let from = at.saturating_sub(160);
            String::from_utf8_lossy(&bytes[from..(at + 240).min(bytes.len())]).into_owned()
        };
        let mob = actual[..at]
            .rfind("Mob { , id: ")
            .map(|i| &actual[i + 6..i + 18]);
        panic!(
            "{scene}, tick {tick}: the {what} differs from the serial walk at {at}, in {mob:?}\nclean:  {}\nserial: {}",
            part(actual),
            part(expected)
        );
    }
}

/// Runs `scene` on every path and compares each with the walk: gives back the
/// run that applied its clean rounds, and the walk's.
fn both_paths(game: &mut Game<'_>, scene: &Scene) -> (Run, Run) {
    let walk = run(game, scene, Path::Walk);
    assert_eq!(
        walk.clean_rounds() + walk.fallback_rounds(),
        0,
        "{}",
        scene.name
    );
    assert_eq!(walk.walks.len(), scene.ticks, "{}", scene.name);
    for (name, path) in [("clean", Path::Clean), ("verify", Path::Verify)] {
        let other = run(game, scene, path);
        let what = format!("{name} path's walk");
        for (tick, (actual, expected)) in other.walks.iter().zip(&walk.walks).enumerate() {
            assert_same(scene.name, &what, tick, actual, expected);
        }
        let what = format!("{name} path's level");
        for (tick, (actual, expected)) in other.ends.iter().zip(&walk.ends).enumerate() {
            assert_same(scene.name, &what, tick, actual, expected);
        }
        for (tick, (actual, expected)) in other.stats.iter().zip(&walk.stats).enumerate() {
            assert_same(
                scene.name,
                &format!("{name} path's stats"),
                tick,
                &format!("{:?}", actual.outcome()),
                &format!("{:?}", expected.outcome()),
            );
        }
        assert_eq!(other.walks.len(), scene.ticks, "{}", scene.name);
    }
    (run(game, scene, Path::Clean), walk)
}

fn plain(_: &mut Mob) {}

/// Four zombies stacked on a spot, three spots in their own regions, the slots
/// interleaved as a level's are.
fn crowds() -> Vec<(&'static str, [f64; 3], fn(&mut Mob))> {
    let mut mobs: Vec<(&'static str, [f64; 3], fn(&mut Mob))> = Vec::new();
    for i in 0..4 {
        for region in 0..3 {
            let x = f64::from(region) * 128.0 + 8.0 + f64::from(i) * 0.1;
            mobs.push(("minecraft:zombie", [x, 64.0, 8.0], plain));
        }
    }
    mobs
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn clean_commit_matches_the_serial_walk() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    let game = &mut game;
    let three_regions: &[(i32, i32)] = &[(0, 0), (8, 0), (16, 0)];

    // A clean round: crowds that push each other, in regions that never meet.
    let scene = Scene {
        name: "clean",
        chunks: three_regions,
        ticks: 30,
        mobs: crowds(),
        died: &[],
        guest_at: None,
    };
    let (clean, walk) = both_paths(game, &scene);
    assert_eq!(clean.clean_rounds(), scene.ticks);
    assert_eq!(clean.fallback_rounds(), 0);
    assert!(
        walk.any("pushes"),
        "pushes should be left over at a mob already passed"
    );
    assert!(
        walk.stats
            .iter()
            .all(|s| s.committed == 12 && s.replayed == 0)
    );

    // A cross-region conflict: each goat's sensor box covers the other's
    // column over a gap of unloaded chunks.
    let scene = Scene {
        name: "cross-region read",
        chunks: &[(0, 0), (2, 0)],
        ticks: 30,
        mobs: vec![
            ("minecraft:goat", [15.5, 64.0, 8.0], plain),
            ("minecraft:goat", [32.1, 64.0, 8.0], plain),
        ],
        died: &[],
        guest_at: None,
    };
    let (clean, _) = both_paths(game, &scene);
    assert!(
        clean.fell_back_for(NotClean::ForeignRead),
        "{:?}",
        clean.stats.iter().map(|s| s.fallback).collect::<Vec<_>>()
    );

    // A brain write to another mob: two animals in love find each other, with
    // no other region whose writes could send the same rounds to the walk.
    fn in_love(m: &mut Mob) {
        m.extra.in_love = 600;
    }
    let scene = Scene {
        name: "brain write",
        chunks: three_regions,
        ticks: 100,
        mobs: vec![
            ("minecraft:cow", [8.0, 64.0, 8.0], in_love),
            ("minecraft:cow", [9.0, 64.0, 8.0], in_love),
        ],
        died: &[],
        guest_at: None,
    };
    let (clean, _) = both_paths(game, &scene);
    assert!(
        clean.fell_back_for(NotClean::BrainWrite),
        "{:?}",
        clean.stats.iter().map(|s| s.fallback).collect::<Vec<_>>()
    );
    assert!(
        clean.clean_rounds() > 0,
        "the rounds without a write stay clean"
    );

    // A death and removal, in clean rounds: a body at its last tick with
    // effects on it, one killed by its own tick, and one already removed.
    fn last_tick_of_death(m: &mut Mob) {
        m.health = 0.0;
        m.death_time = 19;
        m.extra.effects.push(("minecraft:speed", 200, 0));
    }
    fn dies_in_the_void(m: &mut Mob) {
        m.health = 0.5;
    }
    fn removed(m: &mut Mob) {
        m.removed = true;
    }
    let scene = Scene {
        name: "death and removal",
        chunks: three_regions,
        ticks: 30,
        mobs: vec![
            ("minecraft:zombie", [8.0, 64.0, 8.0], last_tick_of_death),
            ("minecraft:zombie", [136.0, -200.0, 8.0], dies_in_the_void),
            ("minecraft:zombie", [264.0, 64.0, 8.0], removed),
            ("minecraft:zombie", [10.0, 64.0, 8.0], plain),
        ],
        died: &[1],
        guest_at: None,
    };
    let (clean, walk) = both_paths(game, &scene);
    assert!(clean.clean_rounds() > 0);
    assert_eq!(clean.fallback_rounds(), 0);
    for output in ["corpses", "gone", "dead"] {
        assert!(
            walk.any(output),
            "the scene should leave something in {output}"
        );
    }

    // A guest hit: a zombie reaches the guest beside it.
    fn hunting(m: &mut Mob) {
        m.ai.target = Some(horizon_client::mobs::Who::Player);
    }
    let scene = Scene {
        name: "guest hit",
        chunks: three_regions,
        ticks: 60,
        mobs: vec![
            ("minecraft:zombie", [8.5, 64.0, 8.5], hunting),
            ("minecraft:zombie", [136.0, 64.0, 8.0], plain),
        ],
        died: &[],
        guest_at: Some([9.0, 64.0, 8.5]),
    };
    let (clean, walk) = both_paths(game, &scene);
    assert!(walk.any("guest_events"), "the zombie should hit the guest");
    assert!(
        walk.any("bindings"),
        "the zombie should be bound to the guest"
    );
    assert!(clean.clean_rounds() > 0);

    // A round that falls back: a mob outside every region has no prediction.
    let scene = Scene {
        name: "fallback",
        chunks: three_regions,
        ticks: 20,
        mobs: vec![
            ("minecraft:zombie", [8.0, 64.0, 8.0], plain),
            ("minecraft:zombie", [800.0, 64.0, 8.0], plain),
            ("minecraft:zombie", [136.0, 64.0, 8.0], plain),
        ],
        died: &[],
        guest_at: None,
    };
    let (clean, walk) = both_paths(game, &scene);
    assert_eq!(clean.clean_rounds(), 0);
    assert_eq!(clean.fallback_rounds(), scene.ticks);
    assert!(clean.fell_back_for(NotClean::Unpredicted));
    assert!(walk.stats.iter().all(|s| s.committed == 2));
}
