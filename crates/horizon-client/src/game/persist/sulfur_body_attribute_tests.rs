use super::*;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::motion::PlayerMotion;

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
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
    options.game_dir = Some(
        std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join(format!("h192-{:?}", std::thread::current().id())),
    );
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
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
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("difficulty normal"));
    assert!(game.run_command("scoreboard objectives add h192 dummy"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
    game.creative = true;
    test(&mut game);
}
/// `/attribute <cube> <attribute> get 1000`, read back through a score.
fn read(game: &mut Game<'_>, n: i32, attribute: &str) -> i32 {
    let sel = format!("@e[type=minecraft:sulfur_cube,nbt={{UUID:[I;192,0,0,{n}]}},limit=1]");
    assert!(game.run_command(&format!(
        "execute store result score #v h192 run attribute {sel} minecraft:{attribute} get 1000"
    )));
    (-3000..=3000)
        .find(|v| game.run_command_quietly(&format!("execute if score #v h192 matches {v}")))
        .unwrap_or(i32::MIN)
}

/// Expected values from the official 26.3 server: a sulfur cube reads its
/// defaults until a tick adopts its body (collectEquipmentChanges); then the
/// body's archetype modifiers count (oak planks: bouncy).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
fn a_sulfur_cube_reads_its_body_archetype_modifiers_after_a_tick() {
    with_game(|game| {
        for (n, body) in [
            (1, ",equipment:{body:{id:\"minecraft:oak_planks\"}}"),
            (2, ""),
        ] {
            assert!(game.run_command(&format!(
                    "summon minecraft:sulfur_cube {}.5 200 4.5 {{UUID:[I;192,0,0,{n}],NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,Size:1{body}}}",
                    3 * n
                )));
        }
        let attrs = [
            "knockback_resistance",
            "explosion_knockback_resistance",
            "bounciness",
            "friction_modifier",
            "air_drag_modifier",
        ];
        let defaults = [0, 0, 0, 1000, 1000];
        let mut failures = Vec::new();
        for (n, label) in [(1, "body"), (2, "none")] {
            for (a, d) in attrs.iter().zip(defaults) {
                let got = read(game, n, a);
                if got != d {
                    failures.push(format!("{label} {a} before a tick: {got}, expected {d}"));
                }
            }
        }
        game.tick_world(true).unwrap();
        game.tick_world(true).unwrap();
        // Bouncy: -2, -2 clamped to 0, +0.9, x0.3, x0.01.
        let bouncy = [-2000, 0, 899, 300, 9];
        for (n, label, want) in [(1, "body", bouncy), (2, "none", defaults)] {
            for (a, d) in attrs.iter().zip(want) {
                let got = read(game, n, a);
                if got != d {
                    failures.push(format!("{label} {a} after two ticks: {got}, expected {d}"));
                }
            }
        }
        assert!(failures.is_empty(), "sulfur body attributes: {failures:#?}");
    });
}

/// Added before production (routing compatibility): a cube with no body
/// keeps the existing effect of a command-set knockback base and permanent
/// modifier on the cached resistance its physics reads
/// (Mob::knockback_resistance falls back to attrs.knockback_resistance).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
fn a_bodiless_cube_keeps_its_command_knockback_resistance_effect() {
    with_game(|game| {
        assert!(game.run_command(
                "summon minecraft:sulfur_cube 4.5 200 8.5 {UUID:[I;192,0,0,9],NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,Size:1}"
            ));
        let sel = "@e[type=minecraft:sulfur_cube,nbt={UUID:[I;192,0,0,9]},limit=1]";
        let cached = |game: &Game<'_>| {
            let m = game
                .mobs
                .iter()
                .find(|m| game.persist.uuid_of(m.id) == Some([192, 0, 0, 9]))
                .unwrap();
            (m.attrs.knockback_resistance, m.knockback_resistance())
        };
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let mut failures = Vec::new();
        for (command, want) in [
            (
                format!("attribute {sel} minecraft:knockback_resistance base set 0.5"),
                0.5,
            ),
            (
                format!(
                    "attribute {sel} minecraft:knockback_resistance modifier add h192:extra 0.25 add_value"
                ),
                0.75,
            ),
            (
                format!(
                    "attribute {sel} minecraft:knockback_resistance modifier remove h192:extra"
                ),
                0.5,
            ),
        ] {
            assert!(game.run_command(&command), "{command}");
            let (c, e) = cached(game);
            if !close(c, want) || !close(e, want) {
                failures.push(format!(
                    "{command}: cached {c}, effective {e}, expected {want}"
                ));
            }
            if read(game, 9, "knockback_resistance") != (want * 1000.0) as i32 {
                failures.push(format!(
                    "{command}: /attribute get {}",
                    read(game, 9, "knockback_resistance")
                ));
            }
        }
        game.tick_world(true).unwrap();
        let (c, _) = cached(game);
        if !close(c, 0.5) {
            failures.push(format!("after a tick: cached {c}"));
        }
        assert!(
            failures.is_empty(),
            "bodiless cube knockback: {failures:#?}"
        );
    });
}
/// Added control (H192 routing mirror): base reset and modifier add/remove
/// on a bodiless and a bodied cube reach the cached resistance, which never
/// takes the body archetype's transient modifiers (bouncy: -2).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
fn command_knockback_changes_reach_the_cache_without_archetype_values() {
    with_game(|game| {
        for (n, body) in [
            (10, ""),
            (11, ",equipment:{body:{id:\"minecraft:oak_planks\"}}"),
        ] {
            assert!(game.run_command(&format!(
                    "summon minecraft:sulfur_cube {}.5 200 8.5 {{UUID:[I;192,0,0,{n}],NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,Size:1{body}}}",
                    n - 8
                )));
        }
        game.tick_world(true).unwrap();
        let cached = |game: &Game<'_>, n: i32| {
            game.mobs
                .iter()
                .find(|m| game.persist.uuid_of(m.id) == Some([192, 0, 0, n]))
                .unwrap()
                .attrs
                .knockback_resistance
        };
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let mut failures = Vec::new();
        // (command, cached, /attribute get for the bodied cube)
        for (command, want, bodied) in [
            ("base set 0.5", 0.5, -1500),
            ("modifier add h192:extra 0.25 add_value", 0.75, -1250),
            ("modifier remove h192:extra", 0.5, -1500),
            ("base reset", 0.0, -2000),
            ("modifier add h192:extra 0.25 add_value", 0.25, -1750),
            ("modifier remove h192:extra", 0.0, -2000),
        ] {
            for n in [10, 11] {
                let sel =
                    format!("@e[type=minecraft:sulfur_cube,nbt={{UUID:[I;192,0,0,{n}]}},limit=1]");
                let full = format!("attribute {sel} minecraft:knockback_resistance {command}");
                assert!(game.run_command(&full), "{full}");
                let c = cached(game, n);
                if !close(c, want) {
                    failures.push(format!("{n} {command}: cached {c}, expected {want}"));
                }
            }
            let got = read(game, 10, "knockback_resistance");
            if got != (want * 1000.0) as i32 {
                failures.push(format!("10 {command}: /attribute get {got}"));
            }
            let got = read(game, 11, "knockback_resistance");
            if got != bodied {
                failures.push(format!(
                    "11 {command}: /attribute get {got}, expected {bodied}"
                ));
            }
        }
        assert!(failures.is_empty(), "knockback mirror: {failures:#?}");
    });
}
