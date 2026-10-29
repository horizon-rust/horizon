//! Loads real mods built with the SDK: the coords-hud and predator-missile
//! examples and a guest
//! that misbehaves on command. Needs `rustup target add wasm32-unknown-unknown`.
use horizon_mods::{
    BlockPos, Clock, Corner, GameMode, HeldItem, Hit, Mods, Output, Package, Player, Runs,
    ServerPlayer, Shape, Snapshot, Status,
    manifest::componentize,
    testing::{Script, TestGame},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    time::Instant,
};

const GUEST_MANIFEST: &str = r#"
id = "test_guest"
name = "Test Guest"
version = "0.1.0"
api = "0.1"
capabilities = ["chat", "storage"]
"#;
const UI_MANIFEST: &str = r#"
id = "test_ui_guest"
name = "Test UI Guest"
version = "0.1.0"
api = "0.1"
capabilities = ["game-read", "chat", "command", "ui", "render", "input", "recipes"]
experimental = ["ui", "render", "input", "recipes"]
"#;

/// Builds the mods once and writes their `.hmod` files; returns the folder.
fn mods_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("horizon-guests-{}", std::process::id()))
            })
            .join("mods-test");
        let status = Command::new(env!("CARGO"))
            .current_dir(&root)
            .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
            .args([
                "-p",
                "coords-hud",
                "-p",
                "predator-missile",
                "-p",
                "welcome",
                "-p",
                "beacons",
                "-p",
                "horizon-mods-test-guest",
                "-p",
                "horizon-mods-test-ui-guest",
                "--target-dir",
            ])
            .arg(&target)
            .status()
            .expect("cargo runs");
        assert!(
            status.success(),
            "building the test mods needs `rustup target add wasm32-unknown-unknown`"
        );
        let built = target.join("wasm32-unknown-unknown/release");
        let dir = target.join("mods");
        std::fs::create_dir_all(&dir).unwrap();
        let example = |name: &str| {
            let path = format!("crates/horizon-mod-sdk/examples/{name}/horizon-mod.toml");
            std::fs::read_to_string(root.join(path)).unwrap()
        };
        let mods = [
            ("coords_hud", example("coords-hud")),
            ("predator_missile", example("predator-missile")),
            ("welcome", example("welcome")),
            ("beacons", example("beacons")),
            ("horizon_mods_test_guest", GUEST_MANIFEST.to_string()),
            ("horizon_mods_test_ui_guest", UI_MANIFEST.to_string()),
        ];
        for (name, manifest) in mods {
            let core = std::fs::read(built.join(format!("{name}.wasm"))).unwrap();
            let hmod = Package::write(&manifest, &componentize(&core).unwrap()).unwrap();
            std::fs::write(dir.join(format!("{name}.hmod")), hmod).unwrap();
        }
        dir
    })
}

fn load(name: &str) -> Mods {
    Mods::load(&[mods_dir().join(name)], |_| true, |c| c == "give")
}

fn player() -> Snapshot {
    Snapshot {
        player: Some(Player {
            x: 12.5,
            y: 64.0,
            z: -3.25,
            yaw: 180.0,
            pitch: 0.0,
            health: 20.0,
            food: 20,
            dimension: "minecraft:overworld".into(),
        }),
        clock: Some(Clock {
            game_time: 100,
            day_time: 24000 + 6000,
        }),
        gui_size: (427.0, 240.0),
        ..Snapshot::default()
    }
}

/// Capability `command`: `/missile` queues vanilla commands, and the chat
/// line its impact check prints sets off the warhead.
#[test]
fn predator_missile_runs_commands() {
    let mut mods = load("predator_missile.hmod");
    assert_eq!(mods.entries()[0].status, Status::Running);
    mods.set_snapshot(player());
    let commands = |mods: &mut Mods| -> Vec<String> {
        mods.drain()
            .into_iter()
            .filter_map(|o| match o {
                Output::Command { line } => Some(line),
                _ => None,
            })
            .collect()
    };
    assert!(mods.command("missile"));
    let launch = commands(&mut mods);
    assert_eq!(launch.len(), 6);
    assert!(launch.contains(&"gamemode spectator".to_string()));
    mods.tick(1);
    mods.tick(2);
    let flight = commands(&mut mods);
    assert!(flight.iter().any(|l| l.ends_with("run tp @s ^ ^ ^1.5")));
    mods.chat("Predator: impact");
    let blast = commands(&mut mods);
    assert_eq!(blast.len(), 4);
    assert!(blast.iter().all(|l| l.contains("summon tnt")));
}

#[test]
fn coords_hud_draws_and_answers() {
    let mut mods = load("coords_hud.hmod");
    assert_eq!(mods.entries()[0].status, Status::Running);
    assert_eq!(mods.commands().collect::<Vec<_>>(), ["where", "coords"]);
    mods.set_snapshot(player());
    mods.tick(1);
    let panel = mods.panels().next().expect("a panel after a tick");
    assert_eq!(panel.corner, Corner::TopLeft);
    let lines: Vec<&str> = panel.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        lines,
        [
            "XYZ 12.5 / 64.0 / -3.2",
            "Facing North (-Z)",
            "Day 2, 12:00"
        ]
    );

    assert!(mods.command("where"));
    let chat = Output::Chat {
        name: "Coordinates HUD".into(),
        text: "12, 64, -4 in minecraft:overworld".into(),
    };
    assert_eq!(mods.drain(), [chat]);
    assert!(mods.command("coords"));
    mods.tick(2);
    assert_eq!(mods.panels().count(), 0);
    let hidden = Output::Feedback {
        text: "Coordinates panel hidden".into(),
        error: false,
    };
    assert_eq!(mods.drain(), [hidden]);
    assert!(!mods.command("gamemode creative"));
}

#[test]
fn off_mods_cost_nothing() {
    let mods = Mods::load(&[mods_dir().join("coords_hud.hmod")], |_| false, |_| false);
    assert_eq!(mods.entries()[0].status, Status::Off);
    assert!(!mods.is_running());
    assert_eq!(mods.commands().count(), 0);
}

#[test]
fn undeclared_capabilities_are_refused() {
    let dir = mods_dir().join("undeclared");
    std::fs::create_dir_all(&dir).unwrap();
    let wasm = Package::read(&mods_dir().join("horizon_mods_test_guest.hmod"))
        .unwrap()
        .wasm
        .expect("the guest has code");
    let manifest = GUEST_MANIFEST.replace("\"chat\", ", "");
    std::fs::write(dir.join("horizon-mod.toml"), manifest).unwrap();
    std::fs::write(dir.join("mod.wasm"), wasm).unwrap();
    let mods = Mods::load(&[dir], |_| true, |_| false);
    let refused = Status::Failed("uses capability chat, missing from horizon-mod.toml".into());
    assert_eq!(mods.entries()[0].status, refused);
}

/// A mod calling a function this Horizon doesn't have is refused with
/// "update Horizon", not a linking error; the test mods' levels are read
/// from the functions they import.
#[test]
fn a_mod_for_a_newer_horizon_is_refused() {
    use horizon_mods::api_level::{imports, needed};
    use wit_parser::{ManglingAndAbi, Resolve};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wit = std::fs::read_to_string(root.join("crates/horizon-mod-sdk/wit/horizon.wit"))
        .unwrap()
        .replace(
            "    set-minimap: func(minimap: option<minimap>);",
            "    set-minimap: func(minimap: option<minimap>);\n    set-sky: func(on: bool);",
        );
    let mut resolve = Resolve::default();
    let package = resolve.push_str("horizon.wit", &wit).unwrap();
    let world = resolve
        .select_world(&[package], Some("horizon-ui-mod"))
        .unwrap();
    let mut module = wit_component::dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    wit_component::embed_component_metadata(
        &mut module,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .unwrap();
    let wasm = wit_component::ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .encode()
        .unwrap();
    let functions = imports(&wasm).unwrap();
    assert!(functions.iter().any(|f| f == "render.set-sky"));
    let dir = mods_dir().join("too_new");
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = r#"
id = "too_new"
name = "Too New"
version = "0.1.0"
api = "0.2"
capabilities = ["game-read", "chat", "hud", "storage", "command", "ui", "render", "input", "recipes", "resources"]
experimental = ["ui", "render", "input", "recipes", "resources"]
"#;
    std::fs::write(dir.join("horizon-mod.toml"), manifest).unwrap();
    std::fs::write(dir.join("mod.wasm"), wasm).unwrap();
    let mods = Mods::load(&[dir], |_| true, |_| false);
    let refused = "made for a newer Horizon (it uses render.set-sky): update Horizon to run it";
    assert_eq!(mods.entries()[0].status, Status::Failed(refused.into()));

    let level = |file: &str| {
        let wasm = Package::read(&mods_dir().join(file)).unwrap().wasm.unwrap();
        needed(imports(&wasm).unwrap().iter().map(String::as_str)).0
    };
    assert_eq!(level("coords_hud.hmod"), 0);
    // The UI guest draws far terrain (`render.set-far-terrain`).
    assert_eq!(level("horizon_mods_test_ui_guest.hmod"), 7);
}

#[test]
fn vanilla_commands_win() {
    let mods = Mods::load(
        &[mods_dir().join("coords_hud.hmod")],
        |_| true,
        |c| c == "where",
    );
    assert_eq!(mods.commands().collect::<Vec<_>>(), ["coords"]);
}

#[test]
fn a_misbehaving_mod_is_stopped_alone() {
    for (command, reason) in [
        ("spin", "it took too long (an endless loop?)"),
        ("panic", "it crashed (a panic)"),
        ("grow", "it crashed (a panic)"),
    ] {
        let paths = [
            mods_dir().join("coords_hud.hmod"),
            mods_dir().join("horizon_mods_test_guest.hmod"),
        ];
        let mut mods = Mods::load(&paths, |_| true, |_| false);
        let started = Instant::now();
        assert!(mods.command(command));
        assert!(started.elapsed().as_secs() < 5, "/{command} was cut short");
        let stopped = Output::Stopped {
            name: "Test Guest".into(),
            reason: reason.into(),
        };
        assert_eq!(mods.drain(), [stopped], "/{command}");
        assert_eq!(mods.entries()[1].status, Status::Failed(reason.into()));
        // The other mod runs on, and the stopped one's command is gone.
        assert!(mods.is_running());
        assert!(!mods.command("hello"));
        mods.set_snapshot(player());
        mods.tick(1);
        assert_eq!(mods.panels().count(), 1);
    }
}

#[test]
fn hot_reload_restarts_a_changed_mod() {
    let dir = mods_dir().join("reload");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("guest.hmod");
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &path).unwrap();
    let mut mods = Mods::load(std::slice::from_ref(&path), |_| true, |_| false);
    mods.tick(1);
    assert!(!mods.reload_changed(|_| false));
    // A rebuild: same bytes, newer file.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert!(mods.reload_changed(|_| false));
    assert!(mods.command("hello world"));
    let chat = Output::Chat {
        name: "Test Guest".into(),
        text: "hello world after 0 ticks".into(),
    };
    assert_eq!(mods.drain(), [chat]);
}

#[test]
fn storage_persists_across_reload() {
    let dir = mods_dir().join("storage_reload");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("guest.hmod");
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &path).unwrap();
    let mut mods = Mods::load(std::slice::from_ref(&path), |_| true, |_| false);
    assert!(mods.command("save remembered"));
    let saved = Output::Feedback {
        text: "saved".into(),
        error: false,
    };
    assert_eq!(mods.drain(), [saved]);

    // A rebuild (hot_reload_restarts_a_changed_mod's pattern): a fresh Store,
    // but the same on-disk data directory.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert!(mods.reload_changed(|_| false));

    assert!(mods.command("load"));
    let chat = Output::Chat {
        name: "Test Guest".into(),
        text: "remembered".into(),
    };
    assert_eq!(mods.drain(), [chat]);
}

#[test]
fn storage_write_budget_limits_one_call() {
    let dir = mods_dir().join("storage_budget");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("guest.hmod");
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &path).unwrap();
    let mut mods = Mods::load(std::slice::from_ref(&path), |_| true, |_| false);
    // Under the 4 MiB on-disk quota, but over the 1 MiB a single call may
    // write: a loop replacing one key can never bypass the quota by
    // writing a lot in one call, only across many.
    let over_budget = "a".repeat((1 << 20) + 1024);
    assert!(mods.command(&format!("save {over_budget}")));
    let refused = Output::Feedback {
        text: "too much storage traffic in one call".into(),
        error: true,
    };
    assert_eq!(mods.drain(), [refused]);
}

#[test]
fn storage_quota_is_enforced() {
    let dir = mods_dir().join("storage_quota");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("guest.hmod");
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &path).unwrap();
    let mut mods = Mods::load(std::slice::from_ref(&path), |_| true, |_| false);
    // Each call is under the per-call write budget; distinct keys so the
    // quota (not the fixed "note" key being overwritten) is what's tested.
    let chunk = "a".repeat(900 << 10);
    for i in 0..4 {
        assert!(mods.command(&format!("save_key k{i} {chunk}")));
        assert_eq!(
            mods.drain(),
            [Output::Feedback {
                text: "saved".into(),
                error: false,
            }]
        );
    }
    assert!(mods.command(&format!("save_key k4 {chunk}")));
    let refused = Output::Feedback {
        text: "would use over 4 MiB of storage".into(),
        error: true,
    };
    assert_eq!(mods.drain(), [refused]);
}

#[test]
fn storage_key_count_is_bounded() {
    let dir = mods_dir().join("storage_keys");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("guest.hmod");
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &path).unwrap();
    let mut mods = Mods::load(std::slice::from_ref(&path), |_| true, |_| false);
    for i in 0..64 {
        assert!(mods.command(&format!("save_key k{i} x")));
        assert_eq!(
            mods.drain(),
            [Output::Feedback {
                text: "saved".into(),
                error: false,
            }],
            "key {i}"
        );
    }
    assert!(mods.command("save_key k64 x"));
    let refused = Output::Feedback {
        text: "would hold over 64 keys".into(),
        error: true,
    };
    assert_eq!(mods.drain(), [refused]);
}

#[test]
fn duplicate_mod_ids_are_refused() {
    let dir = mods_dir().join("duplicate_ids");
    std::fs::create_dir_all(&dir).unwrap();
    let first = dir.join("first.hmod");
    let second = dir.join("second.hmod");
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &first).unwrap();
    std::fs::copy(mods_dir().join("horizon_mods_test_guest.hmod"), &second).unwrap();
    // Both declare id "test_guest" (the guest's own manifest): sharing a
    // `data_dir` would let the second read, overwrite or delete the
    // first's storage, so only the first-loaded one may start.
    let mods = Mods::load(&[first, second], |_| true, |_| false);
    assert_eq!(mods.entries()[0].status, Status::Running);
    let refused = Status::Failed("another running mod already uses id test_guest".into());
    assert_eq!(mods.entries()[1].status, refused);
}

/// `cargo test -p horizon-mods --release -- --ignored --nocapture`: the
/// cost of the per-tick and per-frame hooks (docs/mods.md, Performance).
#[test]
#[ignore]
fn hook_overhead() {
    let time = |mods: &mut Mods, frame: bool| {
        let n = 200_000;
        for i in 0..1000 {
            mods.tick(i);
        }
        let started = Instant::now();
        for i in 0..n {
            if frame {
                mods.frame(0.016);
            } else {
                mods.tick(i);
            }
        }
        started.elapsed().as_nanos() as f64 / n as f64
    };
    let mut guest = load("horizon_mods_test_guest.hmod");
    let mut hud = load("coords_hud.hmod");
    hud.set_snapshot(player());
    let mut none = Mods::default();
    println!("no mods, tick:        {:8.1} ns", time(&mut none, false));
    println!("empty tick hook:      {:8.1} ns", time(&mut guest, false));
    println!("empty frame hook:     {:8.1} ns", time(&mut guest, true));
    println!("coords-hud tick hook: {:8.1} ns", time(&mut hud, false));
    let started = Instant::now();
    let _ = load("coords_hud.hmod");
    println!(
        "load coords-hud:      {:8.1} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

#[test]
fn ui_screens_settings_and_events() {
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    assert_eq!(mods.entries()[0].status, Status::Running);
    assert!(!mods.entries()[0].has_ui_settings());
    // `/ui_screen` pushes the screen; the host clips nothing of this one.
    assert!(mods.command("ui_screen"));
    assert_eq!(mods.drain(), [Output::OpenScreen { owner: 0 }]);
    let screen = mods.screen(0).expect("a pushed screen");
    assert_eq!(screen.title, "Guest's screen");
    assert_eq!(screen.widgets.len(), 3);
    // What the player did comes back through `ui-events`.
    mods.ui_select(0, 7, 2);
    mods.ui_closed(0);
    assert_eq!(
        mods.drain(),
        [
            Output::Chat {
                name: "Test UI Guest".into(),
                text: "selected 7:2".into(),
            },
            Output::Chat {
                name: "Test UI Guest".into(),
                text: "closed".into(),
            },
        ]
    );
    // The settings page: empty until the mod fills it, refreshed on open.
    assert!(mods.command("ui_settings"));
    assert_eq!(mods.settings_widgets(0).len(), 3);
    assert!(mods.entries()[0].has_ui_settings());
    mods.ui_settings_opened(0);
    assert_eq!(mods.settings_widgets(0).len(), 2);
    // `/ui_close` closes the mod's own screen.
    assert!(mods.command("ui_close"));
    assert!(mods.drain().contains(&Output::CloseScreen { owner: 0 }));
    assert!(mods.screen(0).is_none());
}

#[test]
fn render_shapes_and_shakes() {
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    mods.set_snapshot(Snapshot {
        gui_size: (427.0, 240.0),
        ..Snapshot::default()
    });
    assert_eq!(mods.hud_shapes().count(), 0);
    assert!(mods.command("render_hud"));
    let shapes: Vec<_> = mods.hud_shapes().cloned().collect();
    assert_eq!(shapes.len(), 5);
    // The guest read `gui-size`; text is clipped to 256 characters.
    let Shape::Text(t) = &shapes[4] else {
        panic!("{:?}", shapes[4])
    };
    assert!(t.text.starts_with("gui 427x240 "), "{}", t.text);
    assert_eq!(t.text.chars().count(), 256);
    assert!(mods.command("render_clear"));
    assert_eq!(mods.hud_shapes().count(), 0);
    // One jolt, clamped; zero and NaN shake nothing.
    mods.drain();
    assert!(mods.command("render_shake"));
    assert_eq!(
        mods.drain(),
        [Output::Shake {
            strength: 1.0,
            seconds: 10.0
        }]
    );
}

#[test]
fn input_claims_keys_and_events() {
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    let chat = |mods: &mut Mods| -> Vec<String> {
        mods.drain()
            .into_iter()
            .filter_map(|o| match o {
                Output::Chat { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    };
    assert!(!mods.claims_any() && !mods.has_keys());
    assert!(mods.command("input_claim"));
    let said = chat(&mut mods);
    assert!(
        said[0].starts_with("claimed; \"R\" is not a key"),
        "{said:?}"
    );
    assert!(mods.claims("minecraft:stick") && !mods.claims("minecraft:stone"));
    let keys: Vec<_> = mods
        .mod_keys()
        .map(|(m, k)| (m.to_owned(), k.key.clone()))
        .collect();
    assert_eq!(
        keys,
        [("Test UI Guest".to_owned(), "key.keyboard.r".to_owned())]
    );
    // The game feeds the events; the mod reads them in order with `poll`.
    let stick = HeldItem {
        id: "minecraft:stick".into(),
        count: 3,
        slot: 2,
    };
    mods.use_start(&stick);
    mods.key("key.keyboard.r", true);
    mods.key("key.keyboard.t", true);
    mods.key("key.keyboard.r", false);
    mods.use_stop(&stick, 12);
    assert!(mods.command("input_poll"));
    assert_eq!(
        chat(&mut mods),
        [
            "use-start minecraft:stick 0",
            "key-down wheel",
            "key-up wheel",
            "use-stop minecraft:stick 12"
        ]
    );
    // `held` reads the snapshot.
    mods.set_snapshot(Snapshot {
        held: Some(stick),
        ..Snapshot::default()
    });
    assert!(mods.command("input_held"));
    assert_eq!(chat(&mut mods), ["holding minecraft:stick x3 in 2"]);
    // A screen opening lets held keys up, once; the release that follows
    // is not sent again.
    mods.key("key.keyboard.r", true);
    mods.release_keys();
    mods.release_keys();
    mods.key("key.keyboard.r", false);
    assert!(mods.command("input_poll"));
    assert_eq!(chat(&mut mods), ["key-down wheel", "key-up wheel"]);
}

#[test]
fn the_players_key_choice_wins() {
    let chat = |mods: &mut Mods| -> Vec<String> {
        mods.drain()
            .into_iter()
            .filter_map(|o| match o {
                Output::Chat { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    };
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    assert!(mods.command("input_claim"));
    // Its keys alone give it a page in Options > Mods, where the game lists them.
    assert!(mods.settings_widgets(0).is_empty() && mods.entries()[0].has_ui_settings());
    // Rebinding while held lets the old key go.
    mods.key("key.keyboard.r", true);
    assert!(mods.rebind(0, "wheel", "key.mouse.4"));
    assert_eq!(mods.keys_of(0)[0].key, "key.mouse.4");
    mods.key("key.keyboard.r", false);
    mods.key("key.mouse.4", true);
    assert!(!mods.rebind(0, "wheel", "R") && !mods.rebind(0, "nope", "key.keyboard.g"));
    chat(&mut mods);
    assert!(mods.command("input_poll"));
    assert_eq!(
        chat(&mut mods),
        ["key-down wheel", "key-up wheel", "key-down wheel"]
    );
    // A choice saved earlier replaces the default, registered before or after.
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    mods.set_bindings(|_| vec![("wheel", "key.keyboard.g"), ("bad", "G")]);
    assert!(mods.command("input_claim"));
    assert_eq!(mods.keys_of(0)[0].key, "key.keyboard.g");
    mods.set_bindings(|_| vec![("wheel", "key.keyboard.h")]);
    assert_eq!(mods.keys_of(0)[0].key, "key.keyboard.h");
}

#[test]
fn input_wheel_is_the_mods_while_claimed() {
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    let chat = |mods: &mut Mods| -> Vec<String> {
        mods.drain()
            .into_iter()
            .filter_map(|o| match o {
                Output::Chat { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    };
    // Unclaimed, the hotbar keeps the wheel and the mod hears nothing.
    assert!(!mods.wheel_claimed() && !mods.wheel(1.0));
    assert!(mods.command("input_wheel"));
    assert!(mods.wheel_claimed());
    assert!(mods.wheel(1.0) && mods.wheel(-2.0));
    assert!(mods.command("input_poll"));
    assert_eq!(chat(&mut mods), ["wheel 1", "wheel -2"]);
    assert!(mods.command("input_wheel off"));
    assert!(!mods.wheel(1.0));
}

#[test]
fn render_zoom_is_clamped_and_the_strongest_counts() {
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    let zoom = |mods: &mut Mods, args: &str| {
        assert!(mods.command(&format!("zoom {args}")));
        mods.zoom().map(|z| (z.factor, z.smooth, z.turn))
    };
    assert!(mods.zoom().is_none());
    assert_eq!(zoom(&mut mods, "4 1 0.25"), Some((4.0, true, 0.25)));
    // Over 10 and a turn under 0.05 are clamped.
    assert_eq!(zoom(&mut mods, "50 0 0"), Some((10.0, false, 0.05)));
    // A NaN turn keeps the player's sensitivity.
    assert_eq!(zoom(&mut mods, "2 0 x"), Some((2.0, false, 1.0)));
    // 1, under 1 and NaN end the zoom.
    assert_eq!(zoom(&mut mods, "1 0 1"), None);
    assert_eq!(zoom(&mut mods, "0.5 0 1"), None);
    assert_eq!(zoom(&mut mods, "x 0 1"), None);
}

#[test]
fn render_minimap_is_clamped_and_hidden() {
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    let map = |mods: &mut Mods, args: &str| {
        assert!(mods.command(&format!("minimap {args}")));
        mods.minimap()
            .map(|m| (m.corner, m.size, m.blocks_per_pixel, m.rotate, m.round))
    };
    assert!(mods.minimap().is_none());
    let corner = horizon_mods::RenderCorner::BottomLeft;
    assert_eq!(
        map(&mut mods, "96 2 1 0"),
        Some((corner, 96.0, 2.0, true, false))
    );
    // The side and the scale are clamped; NaN takes the smallest.
    assert_eq!(
        map(&mut mods, "999 0.1 0 1"),
        Some((corner, 256.0, 0.5, false, true))
    );
    assert_eq!(
        map(&mut mods, "x 9 0 0"),
        Some((corner, 16.0, 4.0, false, false))
    );
    assert_eq!(map(&mut mods, "off"), None);
}

#[test]
fn render_far_terrain_is_clamped_and_ended() {
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    let far = |mods: &mut Mods, args: &str| {
        assert!(mods.command(&format!("far_terrain {args}")));
        mods.far_terrain().map(|f| (f.distance, f.reduced))
    };
    assert!(mods.far_terrain().is_none());
    assert_eq!(far(&mut mods, "256 1"), Some((256, true)));
    // The distance is clamped to LOD Distance's 64 to 1024 chunks.
    assert_eq!(far(&mut mods, "8 0"), Some((64, false)));
    assert_eq!(far(&mut mods, "99999 0"), Some((1024, false)));
    assert_eq!(far(&mut mods, "off"), None);
}

/// `game.raycast`: the view's hit the game worked out, up to the distance
/// asked for; the game works it out only while a mod imports the call.
#[test]
fn game_raycast_sees_the_view_hit_within_its_distance() {
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    assert!(mods.wants_raycast());
    assert!(!load("coords_hud.hmod").wants_raycast());
    let said = |mods: &mut Mods, args: &str| -> Vec<String> {
        assert!(mods.command(&format!("raycast {args}")));
        mods.drain()
            .into_iter()
            .filter_map(|o| match o {
                Output::Chat { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    };
    assert_eq!(said(&mut mods, "100"), ["no hit"]);
    mods.set_snapshot(Snapshot {
        view_hit: Some((
            40.5,
            Hit {
                block_x: 12,
                block_y: 63,
                block_z: -44,
                x: 12.5,
                y: 64.0,
                z: -43.75,
                block: "minecraft:grass_block".into(),
            },
        )),
        ..player()
    });
    let hit = "hit minecraft:grass_block at 12 63 -44 (12.5 64.0 -43.8)";
    assert_eq!(said(&mut mods, "100"), [hit]);
    assert_eq!(said(&mut mods, "40"), ["no hit"]);
    // NaN asks for nothing; past 1024 is 1024.
    assert_eq!(said(&mut mods, "x"), ["no hit"]);
    assert_eq!(said(&mut mods, "1e9"), [hit]);
}

#[test]
fn quiet_commands_come_out_as_one_batch_capped() {
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    let quiet = |mods: &mut Mods, command: &str| -> Vec<(String, Vec<String>)> {
        assert!(mods.command(command));
        mods.drain()
            .into_iter()
            .filter_map(|o| match o {
                Output::QuietCommands { mod_id, lines } => Some((mod_id, lines)),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        quiet(&mut mods, "quiet setblock 0 0 0 stone;say hi"),
        [(
            "test_ui_guest".to_string(),
            vec!["setblock 0 0 0 stone".to_string(), "say hi".to_string()]
        )]
    );
    let batches = quiet(&mut mods, "quiet_many 300");
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].1.len(), 256);
    assert_eq!(batches[0].1[255], "setblock 255 300 0 stone");
}

#[test]
fn corner_panels_text_width_and_readings() {
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    // A font of 5-pixel glyphs, 3-pixel spaces.
    let mut widths = vec![5.0f32; 0x250];
    widths[' ' as usize] = 3.0;
    mods.set_snapshot(Snapshot {
        frame: (60, 41, 52),
        mappings: vec![("key.forward", "W".into(), true)],
        cps: (7, 2),
        font_widths: widths.into(),
        ..Snapshot::default()
    });
    assert!(mods.command("corner"));
    let panels: Vec<_> = mods.corner_panels().cloned().collect();
    assert_eq!(panels.len(), 1);
    // "60 fps": five glyphs and a space, plus the box's 4.
    assert_eq!(
        (panels[0].width, panels[0].height),
        (5.0 * 5.0 + 3.0 + 4.0, 12.0)
    );
    assert_eq!(panels[0].shapes.len(), 2);
    assert!(matches!(
        panels[0].corner,
        horizon_mods::RenderCorner::TopRight
    ));
    assert!(matches!(
        &mods.settings_widgets(0)[0],
        horizon_mods::UiWidget::Button(b) if b.id == 3 && b.label == "Corner: Top Right"
    ));
    mods.drain();
    assert!(mods.command("readings"));
    assert_eq!(
        mods.drain(),
        [Output::Chat {
            name: "Test UI Guest".into(),
            text: "60 41 52; forward W true; cps 7 2; other true".into(),
        }]
    );
}

#[test]
fn content_only_mods_load_without_an_engine() {
    let manifest = r#"
id = "ruby"
name = "Ruby"
version = "0.1.0"
api = "0.2"
side = "both"
capabilities = ["registries"]
"#;
    let block: &[u8] = br#"{"template": "minecraft:iron_block"}"#;
    let dir = mods_dir().join("content");
    std::fs::create_dir_all(&dir).unwrap();
    let ok = dir.join("ruby.hmod");
    let files: &[(&str, &[u8])] = &[("data/ruby/horizon/block/ruby_block.json", block)];
    std::fs::write(&ok, Package::write_files(manifest, None, files).unwrap()).unwrap();
    let bad = dir.join("bad.hmod");
    let files: &[(&str, &[u8])] = &[("data/minecraft/recipe/stone.json", b"{}")];
    let bad_manifest = manifest.replace("\"ruby\"", "\"bad\"");
    std::fs::write(
        &bad,
        Package::write_files(&bad_manifest, None, files).unwrap(),
    )
    .unwrap();
    let mods = Mods::load(&[ok, bad], |_| true, |_| false);
    assert_eq!(mods.entries()[0].status, Status::Running);
    assert!(matches!(&mods.entries()[1].status, Status::Failed(e) if e.contains("own namespace")));
    // Content has no hooks: the game's per-tick mod work stays skipped.
    assert!(!mods.is_running());
}

/// `cargo horizon-mod test` on the examples: their own `tests/*.toml` pass.
#[test]
fn the_examples_test_scripts_pass() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../horizon-mod-sdk/examples");
    let mut ran = 0;
    let examples_and_files = [
        ("coords-hud", "coords_hud"),
        ("welcome", "welcome"),
        ("beacons", "beacons"),
    ];
    for (example, hmod) in examples_and_files {
        for entry in std::fs::read_dir(examples.join(example).join("tests")).unwrap() {
            let path = entry.unwrap().path();
            let script = Script::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let mut game = TestGame::load(&mods_dir().join(format!("{hmod}.hmod"))).unwrap();
            script
                .run(&mut game)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            ran += 1;
        }
    }
    assert_eq!(ran, 5);
}

/// Capability `world` without a world lent (the registry's trial run):
/// `get-block` finds nothing, `set-blocks` checks its count and queues.
#[test]
fn a_server_mod_without_a_world_reads_nothing() {
    let mut game = TestGame::load(&mods_dir().join("beacons.hmod")).unwrap();
    game.join("Alex");
    let alex = game.players[0].clone();
    assert!(game.player_command(&alex, "ground"));
    let out = game.drain();
    assert!(
        out.iter()
            .any(|o| matches!(o, Output::Reply { text, error: true, .. }
            if text == "That block isn't loaded")),
        "{out:?}"
    );
}

#[test]
fn a_test_script_says_which_step_failed_and_why() {
    let script = Script::parse(
        r#"
        [[step]]
        ticks = 3
        command = "hello there"
        expect.chat = ["hello there after 2 ticks"]
        "#,
    )
    .unwrap();
    let mut game = TestGame::load(&mods_dir().join("horizon_mods_test_guest.hmod")).unwrap();
    assert_eq!(
        script.run(&mut game).unwrap_err(),
        "step 1 (3 ticks, /hello there): chat\n    expected: [\"hello there after 2 ticks\"]\n    \
         got:      [\"hello there after 3 ticks\"]"
    );
    assert!(
        Script::parse("[[step]]\ntick = 1").is_err(),
        "a typo is an error"
    );

    let mut game = TestGame::load(&mods_dir().join("welcome.hmod")).unwrap();
    let script = "[[step]]\njoin = \"Alex\"\n\n[[step]]\nas = \"Steve\"\ncommand = \"mined\"";
    assert_eq!(
        Script::parse(script).unwrap().run(&mut game).unwrap_err(),
        "step 2 (/mined): Steve is not on the server"
    );
}

#[test]
fn a_test_script_reloads_and_expects_a_stop() {
    let script = Script::parse(
        r#"
        [[step]]
        command = "save kept"
        expect.feedback = "saved"

        [[step]]
        reload = true
        command = "load"
        expect.chat = ["kept"]

        [[step]]
        command = "panic"
        expect.stopped = "panic"

        [[step]]
        reload = true
        command = "nope"
        "#,
    )
    .unwrap();
    let mut game = TestGame::load(&mods_dir().join("horizon_mods_test_guest.hmod")).unwrap();
    let dir = game.hmod().parent().unwrap().to_path_buf();
    assert_eq!(
        script.run(&mut game).unwrap_err(),
        "step 4 (reload, /nope): the mod owns no command /nope"
    );
    drop(game);
    assert!(
        !dir.exists(),
        "the scratch copy and its storage are removed"
    );
}

fn alex() -> ServerPlayer {
    ServerPlayer {
        id: "00000000-0000-4000-8000-000000000001".into(),
        name: "Alex".into(),
        x: 0.5,
        y: 64.0,
        z: 0.5,
        dimension: "minecraft:the_nether".into(),
        game_mode: GameMode::Survival,
        health: 20.0,
    }
}

#[test]
fn a_server_mod_runs_only_where_the_world_does() {
    // A fresh folder: what the mod stored in an earlier run is beside it.
    let dir = mods_dir().join("server");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("welcome.hmod");
    std::fs::copy(mods_dir().join("welcome.hmod"), &path).unwrap();
    let paths = std::slice::from_ref(&path);
    let window = Mods::load_for(Runs::Client, paths, |_| true, |_| false);
    assert_eq!(window.entries()[0].status, Status::Elsewhere);
    assert!(!window.is_running());

    let mut mods = Mods::load_for(Runs::Server, paths, |_| true, |c| c == "give");
    assert!(mods.runs_server());
    let commands: Vec<_> = mods.server_commands().map(|c| c.name.as_str()).collect();
    assert_eq!(commands, ["online", "mined"]);
    // `/mined [player]`: its argument travels with it, for the command tree.
    let mined = mods.server_commands().nth(1).unwrap();
    assert_eq!(mined.arguments[0].kind, horizon_mods::ArgumentKind::Player);
    assert!(mined.arguments[0].optional);
    assert_eq!(mods.commands().count(), 0, "not the window's chat commands");
    let alex = alex();
    mods.set_snapshot(Snapshot {
        players: vec![alex.clone()],
        ..Snapshot::default()
    });
    mods.player_joined(&alex);
    let greeted = [
        Output::Tell {
            name: "Welcome".into(),
            player: alex.id.clone(),
            text: "Welcome, Alex! 0 other player(s) online.".into(),
        },
        Output::Broadcast {
            name: "Welcome".into(),
            text: "Alex is here for the first time".into(),
        },
    ];
    assert_eq!(mods.drain(), greeted);
    let stone = BlockPos { x: 1, y: 63, z: 0 };
    mods.block_broken(&alex, stone, "minecraft:stone");
    assert!(mods.server_command(&alex, "mined"));
    assert!(mods.server_command(&alex, "online"));
    let reply = |text: &str| Output::Reply {
        player: alex.id.clone(),
        text: text.into(),
        error: false,
    };
    assert_eq!(
        mods.drain(),
        [reply("You broke 1 block(s)"), reply("Alex (the_nether)")]
    );
    assert!(!mods.server_command(&alex, "where"));
    assert!(
        !mods.command("online"),
        "a server command isn't the window's"
    );
}

/// Capability `recipes`: the guest reads tables as big as vanilla's (1,500
/// items, 1,600 recipes of nine five-item slots) within one command's fuel.
#[test]
fn recipes_reads_the_tables() {
    use horizon_mods::{Recipe, RecipeItem, RecipeKind, RecipeStack, Recipes};
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    let stack = |item| RecipeStack {
        item,
        count: 1,
        components: String::new(),
    };
    let items = (0..1500)
        .map(|i| RecipeItem {
            id: format!("minecraft:item_{i}"),
            name: format!("Item {i}"),
            tags: vec!["planks".into(), "logs".into()],
        })
        .collect();
    let mut recipes: Vec<Recipe> = (0..1600)
        .map(|i| Recipe {
            id: format!("minecraft:recipe_{i}"),
            kind: RecipeKind::CraftingShaped,
            category: "crafting_misc".into(),
            group: String::new(),
            width: 3,
            height: 3,
            slots: (0..9)
                .map(|k| (0..5).map(|j| stack((i + k + j) % 1500)).collect())
                .collect(),
            results: vec![RecipeStack {
                item: 7,
                count: 4,
                components: String::new(),
            }],
            cooking_time: 0,
            experience: 0.0,
            book_id: i as i32,
        })
        .collect();
    recipes[1].kind = RecipeKind::Brewing;
    recipes[1].results[0].components =
        "[minecraft:potion_contents={potion:\"minecraft:swiftness\"}]".into();
    let mods_recipes = Some(std::sync::Arc::new(Recipes { items, recipes }));
    mods.set_snapshot(Snapshot {
        recipes: mods_recipes.clone(),
        known_recipes: vec![1, 5, 9].into(),
        ..Snapshot::default()
    });
    let said = |mods: &mut Mods, command: &str| {
        assert!(mods.command(command));
        mods.drain()
            .into_iter()
            .filter_map(|o| match o {
                Output::Chat { text, .. } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        said(&mut mods, "recipes"),
        ["1500 items, 1600 recipes, 3 known, 1 brewing"]
    );
    assert_eq!(
        said(&mut mods, "recipes minecraft:recipe_1"),
        [
            "minecraft:recipe_1 Kind::Brewing 3x3 0t slots 9, the first 1 minecraft:item_1 -> \
          4 minecraft:item_7[minecraft:potion_contents={potion:\"minecraft:swiftness\"}] known true"
        ]
    );
    assert!(
        mods.entries()
            .iter()
            .all(|e| e.status == horizon_mods::Status::Running),
        "it ran out of fuel"
    );
    // Actions: refused without a crafting screen, for an unknown recipe,
    // and outside creative mode.
    assert_eq!(
        said(&mut mods, "recipe_place minecraft:recipe_5"),
        ["place minecraft:recipe_5: no screen that takes recipes is open"]
    );
    let mut snap = Snapshot {
        recipes: mods_recipes.clone(),
        known_recipes: vec![1, 5, 9].into(),
        placing: true,
        ..Snapshot::default()
    };
    mods.set_snapshot(snap.clone());
    assert_eq!(
        said(&mut mods, "recipe_place minecraft:recipe_6"),
        ["place minecraft:recipe_6: recipe 6 is not in the player's recipe book"]
    );
    assert!(mods.command("recipe_place minecraft:recipe_5 all"));
    assert_eq!(
        mods.drain()[0],
        Output::PlaceRecipe {
            book_id: 5,
            all: true
        }
    );
    assert_eq!(
        said(&mut mods, "recipe_give minecraft:item_7 3"),
        ["give minecraft:item_7: only in creative mode"]
    );
    snap.creative = true;
    mods.set_snapshot(snap);
    assert!(mods.command("recipe_give minecraft:item_7 3"));
    assert_eq!(
        mods.drain()[0],
        Output::Give {
            item: "minecraft:item_7".into(),
            count: 3,
            components: String::new()
        }
    );
    // Without the game's tables (no mod read them yet): empty, not a trap.
    mods.set_snapshot(Snapshot::default());
    assert_eq!(
        said(&mut mods, "recipes"),
        ["0 items, 0 recipes, 0 known, 0 brewing"]
    );
}

#[test]
fn browser_lists_items_and_hears_the_player() {
    use horizon_mods::{BrowserClick, BrowserEvent, BrowserStack};
    let mut mods = load("horizon_mods_test_ui_guest.hmod");
    let said = |mods: &mut Mods, command: &str| {
        assert!(mods.command(command));
        mods.drain()
            .into_iter()
            .filter_map(|o| match o {
                Output::Chat { text, .. } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert!(mods.browser_items().is_none());
    let potion = "[minecraft:potion_contents={potion:\"minecraft:swiftness\"}]";
    assert_eq!(
        said(
            &mut mods,
            &format!("browser minecraft:stone minecraft:potion{potion}")
        ),
        ["panel of 2"]
    );
    let (owner, version, items) = mods.browser_items().unwrap();
    assert_eq!(
        (items[0].item.as_str(), items[1].components.as_str()),
        ("minecraft:stone", potion)
    );
    mods.browser_event(
        owner,
        BrowserEvent::Clicked(BrowserClick {
            index: 1,
            right: true,
        }),
    );
    mods.browser_event(owner, BrowserEvent::Search("oak".into()));
    let log = BrowserStack {
        item: "minecraft:oak_log".into(),
        count: 3,
        components: String::new(),
    };
    // R is the guest's `recipe`; X is none of its keys.
    mods.browser_key("key.keyboard.r", &log);
    mods.browser_key("key.keyboard.x", &log);
    assert_eq!(
        said(&mut mods, "browser_poll"),
        [
            "3 events",
            "clicked 1 right",
            "search \"oak\"",
            "key recipe on minecraft:oak_log x3"
        ]
    );
    // Each `set-items` is a new version; an empty list hides the panel.
    said(&mut mods, "browser minecraft:dirt");
    assert_ne!(mods.browser_items().unwrap().1, version);
    assert_eq!(said(&mut mods, "browser"), ["panel of 0"]);
    assert!(mods.browser_items().is_none());
    // Recipes in the panel's place: the guest shows those making planks.
    use horizon_mods::{BrowserLayout, Recipe, RecipeItem, RecipeKind, RecipeStack, Recipes};
    let item = |id: &str| RecipeItem {
        id: id.into(),
        name: id.into(),
        tags: Vec::new(),
    };
    let stack = |item| RecipeStack {
        item,
        count: 1,
        components: String::new(),
    };
    mods.set_snapshot(Snapshot {
        recipes: Some(std::sync::Arc::new(Recipes {
            items: vec![item("minecraft:oak_log"), item("minecraft:oak_planks")],
            recipes: vec![Recipe {
                id: "minecraft:oak_planks".into(),
                kind: RecipeKind::CraftingShapeless,
                category: "building".into(),
                group: "planks".into(),
                width: 0,
                height: 0,
                slots: vec![vec![stack(0)]],
                results: vec![RecipeStack {
                    count: 4,
                    ..stack(1)
                }],
                cooking_time: 0,
                experience: 0.0,
                book_id: 1,
            }],
        })),
        ..Snapshot::default()
    });
    said(&mut mods, "browser minecraft:dirt");
    assert_eq!(
        said(&mut mods, "browser_recipes minecraft:oak_planks"),
        ["recipes shown: 1"]
    );
    let page = mods.browser_recipes(owner).unwrap();
    assert_eq!(page.title, "Recipes for minecraft:oak_planks");
    assert_eq!(
        (page.tabs[0].item.as_str(), page.tab),
        ("minecraft:crafting_table", 0)
    );
    let r = &page.recipes[0];
    assert_eq!(
        (r.layout, r.width, r.height, r.action),
        (BrowserLayout::Grid, 1, 1, Some(false))
    );
    assert_eq!(
        (r.slots[0][0].item.as_str(), r.results[0].count),
        ("minecraft:oak_log", 4)
    );
    assert_eq!(said(&mut mods, "browser_recipes"), ["recipes shown: 0"]);
    assert!(mods.browser_recipes(owner).is_none());
    // At most 256 events wait for a poll: the oldest go.
    for _ in 0..300 {
        mods.browser_event(owner, BrowserEvent::Page(1));
    }
    assert_eq!(said(&mut mods, "browser_poll")[0], "256 events");
}

const RESOURCE_JSON: &str = "data/resource_lab/species/sample.json";
const RESOURCE_BINARY: &str = "assets/resource_lab/models/sample.bin";
const RESOURCE_EMPTY: &str = "data/resource_lab/empty.bin";
const RESOURCE_EXTRA: &str = "assets/extras_lab/note.txt";
const RESOURCE_LARGE: &str = "data/resource_lab/large.bin";

fn resource_guests() -> &'static [Vec<u8>; 2] {
    static GUESTS: OnceLock<[Vec<u8>; 2]> = OnceLock::new();
    GUESTS.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("resource-build-{}", std::process::id()))
            });
        [false, true].map(|server| {
            let target = target.join(if server {
                "resource-server"
            } else {
                "resource-client"
            });
            let mut cmd = Command::new(env!("CARGO"));
            cmd.current_dir(&root)
                .args([
                    "build",
                    "--locked",
                    "--release",
                    "--target",
                    "wasm32-unknown-unknown",
                    "-p",
                    "horizon-mods-test-resource-guest",
                    "--target-dir",
                ])
                .arg(&target);
            if server {
                cmd.args(["--features", "server"]);
            }
            assert!(cmd.status().expect("cargo runs").success());
            let path =
                target.join("wasm32-unknown-unknown/release/horizon_mods_test_resource_guest.wasm");
            let core = std::fs::read(&path).unwrap();
            let component = componentize(&core).unwrap();
            std::fs::write(target.join("resource.component.wasm"), &component).unwrap();
            let functions = horizon_mods::api_level::imports(&component).unwrap();
            assert_eq!(
                horizon_mods::api_level::needed(functions.iter().map(String::as_str)),
                (8, Some("resources.read"))
            );
            assert!(
                horizon_mods::manifest::component_imports(&component)
                    .unwrap()
                    .iter()
                    .all(|name| name.starts_with("horizon:mods/"))
            );
            eprintln!(
                "resource guest compiled: server={server} core={} bytes={} component_bytes={}",
                path.display(),
                core.len(),
                component.len()
            );
            component
        })
    })
}

fn resource_manifest(id: &str, side: &str) -> String {
    let api = if side == "server" { "0.3" } else { "0.2" };
    let capabilities = if side == "server" {
        "\"resources\", \"players\""
    } else {
        "\"resources\""
    };
    let namespaces = if id == "resource_lab" {
        "\"extras_lab\""
    } else {
        "\"resource_lab\", \"extras_lab\""
    };
    format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\napi = \"{api}\"\nside = \"{side}\"\ncapabilities = [{capabilities}]\nexperimental = [\"resources\"]\nnamespaces = [{namespaces}]\n"
    )
}

struct ResourceFixture {
    root: PathBuf,
    dir: PathBuf,
    archive: PathBuf,
    summary: String,
    nonce: String,
}
impl ResourceFixture {
    fn new(server: bool, id: &str, command: &str) -> Self {
        let wasm = &resource_guests()[usize::from(server)];
        let nonce = format!(
            "{:x}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = target.join("resource-proof").join(&nonce);
        let dir = root.join("program");
        std::fs::create_dir_all(&dir).unwrap();
        let mut fixture = Self {
            archive: root.join("program.hmod"),
            root,
            dir,
            summary: String::new(),
            nonce,
        };
        fixture.put(
            "horizon-mod.toml",
            resource_manifest(id, if server { "server" } else { "client" }).as_bytes(),
        );
        fixture.put("mod.wasm", wasm);
        let json = format!(r#"{{"nonce":"{}"}}"#, fixture.nonce);
        let binary = [0, 255, fixture.nonce.as_bytes()[0], 3, 128];
        fixture.put(RESOURCE_JSON, json.as_bytes());
        fixture.put(RESOURCE_BINARY, &binary);
        fixture.put(RESOURCE_EMPTY, &[]);
        fixture.put(RESOURCE_EXTRA, command.as_bytes());
        let hex = binary
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        fixture.summary = format!(
            "json:{}:{json}|bin:{}:{hex}|empty:0|extra:{}:{command}",
            json.len(),
            binary.len(),
            command.len()
        );
        eprintln!(
            "resource input after compile: root={} server={server} nonce={} json={json} binary={hex} expected={}",
            fixture.root.display(),
            fixture.nonce,
            fixture.summary
        );
        fixture
    }
    fn put(&self, name: &str, bytes: &[u8]) {
        let path = self.dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    fn pack(&self) {
        let manifest = std::fs::read_to_string(self.dir.join("horizon-mod.toml")).unwrap();
        let m = horizon_mods::Manifest::parse(&manifest).unwrap();
        let content = horizon_mods::Content::read(&self.dir, &m).unwrap();
        let mut reader = content.reader().unwrap();
        let bytes: Vec<_> = content
            .files
            .iter()
            .map(|name| (name.as_str(), reader.read(name).unwrap()))
            .collect();
        let files: Vec<_> = bytes
            .iter()
            .map(|(name, bytes)| (*name, bytes.as_slice()))
            .collect();
        let wasm = std::fs::read(self.dir.join("mod.wasm")).unwrap();
        let archive = Package::write_files(&manifest, Some(&wasm), &files).unwrap();
        let pending = self.archive.with_extension("partial");
        std::fs::write(&pending, archive).unwrap();
        std::fs::rename(pending, &self.archive).unwrap();
    }
    fn touch(&self) {
        let path = self.dir.join("mod.wasm");
        let was = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(was + std::time::Duration::from_secs(2))
            .unwrap();
    }
}

fn resource_reply(mods: &mut Mods, server: bool, line: &str) -> (String, bool) {
    let called = if server {
        mods.server_command(&alex(), line)
    } else {
        mods.command(line)
    };
    assert!(
        called,
        "{line:?}: {:?}",
        mods.entries().iter().map(|e| &e.status).collect::<Vec<_>>()
    );
    let output = mods.drain();
    eprintln!("resource guest output: server={server} command={line:?} output={output:?}");
    assert_eq!(output.len(), 1);
    match &output[0] {
        Output::Feedback { text, error } if !server => (text.clone(), *error),
        Output::Reply {
            player,
            text,
            error,
        } if server => {
            assert_eq!(*player, alex().id);
            (text.clone(), *error)
        }
        other => panic!("unexpected resource output: {other:?}"),
    }
}

#[test]
fn resources_real_guests_read_directory_archive_and_test_game() {
    for server in [false, true] {
        let f = ResourceFixture::new(server, "resource_lab", "resource");
        f.pack();
        for path in [&f.dir, &f.archive] {
            let mut mods = Mods::load_for(
                if server { Runs::Server } else { Runs::Client },
                std::slice::from_ref(path),
                |_| true,
                |_| false,
            );
            assert_eq!(mods.entries()[0].status, Status::Running);
            assert_eq!(
                resource_reply(&mut mods, server, "resource"),
                (f.summary.clone(), false)
            );
            assert_eq!(
                resource_reply(
                    &mut mods,
                    server,
                    &format!("resource read {RESOURCE_EMPTY}")
                ),
                ("some:0:".into(), false)
            );
            assert_eq!(
                resource_reply(
                    &mut mods,
                    server,
                    "resource read data/resource_lab/absent.bin"
                ),
                ("none".into(), false)
            );
        }
        let mut game = TestGame::load(&f.dir).unwrap();
        if server {
            game.join("Alex");
        }
        let text = format!(
            "[[step]]\n{}command = \"resource\"\nexpect.feedback = '{}'\n",
            if server { "as = \"Alex\"\n" } else { "" },
            f.summary
        );
        Script::parse(&text).unwrap().run(&mut game).unwrap();
        let changed = format!(r#"{{"nonce":"{}-changed"}}"#, f.nonce);
        f.put(RESOURCE_JSON, changed.as_bytes());
        f.pack();
        let mut mods = Mods::load_for(
            Runs::Both,
            std::slice::from_ref(&f.archive),
            |_| true,
            |_| false,
        );
        let expected = f
            .summary
            .replace(
                &format!("json:{}:", changed.len() - 8),
                &format!("json:{}:", changed.len()),
            )
            .replace(&f.nonce, &format!("{}-changed", f.nonce));
        assert_eq!(
            resource_reply(&mut mods, server, "resource"),
            (expected, false)
        );
    }
}

#[test]
fn resources_paths_are_owned_literal_and_bounded() {
    let f = ResourceFixture::new(false, "resource_lab", "resource");
    let mut mods = Mods::load(std::slice::from_ref(&f.dir), |_| true, |_| false);
    for path in [
        "assets/resource_lab/../x",
        "assets/resource_lab/./x",
        "/assets/resource_lab/x",
        "assets\\resource_lab\\x",
        "assets/resource_lab/x:y",
        "assets/resource_lab//x",
        "assets/Resource_lab/x",
        "storage/note",
        "horizon-mod.toml",
        "mod.wasm",
        "assets/resource_lab/x\0",
        "assets/resource_lab",
    ] {
        assert_eq!(
            resource_reply(&mut mods, false, &format!("resource read {path}")),
            ("invalid resource path".into(), true),
            "{path:?}"
        );
    }
    let prefix = "data/resource_lab/";
    let mut path = prefix.to_string();
    while path.len() + 101 < 1024 {
        path.push_str(&format!("{}/", "a".repeat(100)));
    }
    path.push_str(&"a".repeat(1024 - path.len()));
    assert_eq!(path.len(), 1024);
    assert_eq!(
        resource_reply(&mut mods, false, &format!("resource read {path}")),
        ("none".into(), false)
    );
    path.push('a');
    assert_eq!(
        resource_reply(&mut mods, false, &format!("resource read {path}")),
        ("invalid resource path".into(), true)
    );
    for ns in ["other_lab", "minecraft", "c", "horizon"] {
        assert_eq!(
            resource_reply(
                &mut mods,
                false,
                &format!("resource read data/{ns}/sentinel.bin")
            ),
            ("resource outside owner".into(), true)
        );
    }
    f.put("data/resource_lab/new.bin", b"not indexed");
    assert_eq!(
        resource_reply(&mut mods, false, "resource read data/resource_lab/new.bin"),
        ("none".into(), false)
    );
    std::fs::remove_file(f.dir.join(RESOURCE_BINARY)).unwrap();
    assert_eq!(
        resource_reply(
            &mut mods,
            false,
            &format!("resource read {RESOURCE_BINARY}")
        ),
        ("resource unavailable".into(), true)
    );
}

#[test]
fn resources_equal_namespaces_keep_owners_separate() {
    let first = ResourceFixture::new(false, "resource_lab", "resource");
    let second = ResourceFixture::new(false, "other_lab", "healthy");
    second.put("data/other_lab/sentinel.bin", b"other owner sentinel");
    second.put("data/resource_lab/second.bin", b"second package only");
    first.pack();
    second.pack();
    for paths in [
        [first.dir.clone(), second.dir.clone()],
        [first.archive.clone(), second.archive.clone()],
    ] {
        let mut mods = Mods::load(&paths, |_| true, |_| false);
        assert!(mods.entries().iter().all(|e| e.status == Status::Running));
        assert_eq!(
            resource_reply(&mut mods, false, "resource"),
            (first.summary.clone(), false)
        );
        assert_eq!(
            resource_reply(&mut mods, false, "healthy"),
            (second.summary.clone(), false)
        );
        assert_eq!(
            resource_reply(
                &mut mods,
                false,
                "resource read data/other_lab/sentinel.bin"
            ),
            ("resource outside owner".into(), true)
        );
        assert_eq!(
            resource_reply(
                &mut mods,
                false,
                "resource read data/resource_lab/second.bin"
            ),
            ("none".into(), false)
        );
        assert_eq!(
            resource_reply(
                &mut mods,
                false,
                "healthy read data/resource_lab/second.bin"
            ),
            (
                "some:19:7365636f6e64207061636b616765206f6e6c79".into(),
                false
            )
        );
    }
}

#[cfg(unix)]
#[test]
fn resources_directory_links_cannot_reach_scratch_sentinels() {
    use std::os::unix::fs::symlink;
    let f = ResourceFixture::new(false, "resource_lab", "resource");
    let outside = f.root.join("outside");
    std::fs::create_dir_all(outside.join("resource_lab")).unwrap();
    std::fs::write(
        outside.join("resource_lab/sentinel.bin"),
        b"scratch outside sentinel",
    )
    .unwrap();
    symlink(&outside, f.dir.join("assets/resource_lab/link")).unwrap();
    symlink(
        outside.join("resource_lab/sentinel.bin"),
        f.dir.join("data/resource_lab/linked.bin"),
    )
    .unwrap();
    let mut mods = Mods::load(std::slice::from_ref(&f.dir), |_| true, |_| false);
    for path in [
        "assets/resource_lab/link/resource_lab/sentinel.bin",
        "data/resource_lab/linked.bin",
    ] {
        assert_eq!(
            resource_reply(&mut mods, false, &format!("resource read {path}")),
            ("none".into(), false)
        );
    }
    std::fs::remove_file(f.dir.join(RESOURCE_BINARY)).unwrap();
    symlink(
        outside.join("resource_lab/sentinel.bin"),
        f.dir.join(RESOURCE_BINARY),
    )
    .unwrap();
    assert_eq!(
        resource_reply(
            &mut mods,
            false,
            &format!("resource read {RESOURCE_BINARY}")
        ),
        ("resource unavailable".into(), true)
    );
    std::fs::remove_file(f.dir.join(RESOURCE_BINARY)).unwrap();
    std::fs::remove_file(f.dir.join("assets/resource_lab/link")).unwrap();
    std::fs::remove_dir(f.dir.join("assets/resource_lab/models")).unwrap();
    std::fs::write(
        outside.join("resource_lab/sample.bin"),
        b"scratch ancestor sentinel",
    )
    .unwrap();
    symlink(
        "../../../outside/resource_lab",
        f.dir.join("assets/resource_lab/models"),
    )
    .unwrap();
    assert_eq!(
        resource_reply(
            &mut mods,
            false,
            &format!("resource read {RESOURCE_BINARY}")
        ),
        ("resource unavailable".into(), true)
    );
    std::fs::remove_dir_all(f.dir.join("assets")).unwrap();
    symlink(&outside, f.dir.join("assets")).unwrap();
    let m = horizon_mods::Manifest::parse(
        &std::fs::read_to_string(f.dir.join("horizon-mod.toml")).unwrap(),
    )
    .unwrap();
    let content = horizon_mods::Content::read(&f.dir, &m).unwrap();
    assert!(!content.files.iter().any(|p| p.starts_with("assets/")));
    std::fs::remove_file(f.dir.join("assets")).unwrap();
    std::fs::remove_dir_all(f.dir.join("data")).unwrap();
    symlink(&outside, f.dir.join("data")).unwrap();
    assert!(
        horizon_mods::Content::read(&f.dir, &m)
            .unwrap()
            .files
            .is_empty()
    );
}

#[test]
fn resources_file_bounds_quota_failures_and_callback_reset() {
    use horizon_mods::content::MAX_FILE;
    for server in [false, true] {
        let f = ResourceFixture::new(server, "resource_lab", "resource");
        f.put(RESOURCE_LARGE, &vec![0x51; MAX_FILE as usize]);
        f.put("data/resource_lab/mid.bin", &vec![0x41; 12 << 20]);
        f.pack();
        for path in [&f.dir, &f.archive] {
            let mut mods =
                Mods::load_for(Runs::Both, std::slice::from_ref(path), |_| true, |_| false);
            assert_eq!(
                resource_reply(
                    &mut mods,
                    server,
                    &format!("resource read {RESOURCE_LARGE}")
                ),
                ("some:16777216:51515151".into(), false)
            );
            assert_eq!(
                resource_reply(
                    &mut mods,
                    server,
                    &format!("resource bytes {RESOURCE_LARGE}")
                ),
                (
                    "some:16777216|some:16777216|resource callback budget exhausted".into(),
                    false
                )
            );
            assert_eq!(
                resource_reply(
                    &mut mods,
                    server,
                    "resource bytes data/resource_lab/mid.bin"
                ),
                (
                    "some:12582912|some:12582912|resource callback budget exhausted".into(),
                    false
                )
            );
            for (path, result) in [
                ("data/resource_lab/missing.bin", "none"),
                ("invalid", "invalid resource path"),
                (RESOURCE_EMPTY, "some:0"),
            ] {
                assert_eq!(
                    resource_reply(&mut mods, server, &format!("resource requests {path}")),
                    (
                        format!("64:{result}|65:resource callback budget exhausted"),
                        false
                    )
                );
            }
            assert_eq!(
                resource_reply(
                    &mut mods,
                    server,
                    &format!("resource read {RESOURCE_LARGE}")
                ),
                ("some:16777216:51515151".into(), false)
            );
        }
        let mut mods = Mods::load_for(
            Runs::Both,
            std::slice::from_ref(&f.dir),
            |_| true,
            |_| false,
        );
        std::fs::File::options()
            .write(true)
            .open(f.dir.join(RESOURCE_LARGE))
            .unwrap()
            .set_len(MAX_FILE + 1)
            .unwrap();
        assert_eq!(
            resource_reply(
                &mut mods,
                server,
                &format!("resource read {RESOURCE_LARGE}")
            ),
            ("resource file too large".into(), true)
        );
        std::fs::remove_file(f.dir.join(RESOURCE_LARGE)).unwrap();
        assert_eq!(
            resource_reply(
                &mut mods,
                server,
                &format!("resource bytes {RESOURCE_LARGE}")
            ),
            (
                "resource unavailable|resource unavailable|resource callback budget exhausted"
                    .into(),
                false
            )
        );
        assert_eq!(
            resource_reply(&mut mods, server, "resource"),
            (f.summary.clone(), false)
        );
        f.put("data/resource_lab/init.txt", b"requests");
        let mut mods = Mods::load_for(
            Runs::Both,
            std::slice::from_ref(&f.dir),
            |_| true,
            |_| false,
        );
        assert_eq!(
            resource_reply(&mut mods, server, "resource"),
            (
                format!("{}|init:resource callback budget exhausted", f.summary),
                false
            )
        );
        f.put(RESOURCE_LARGE, &vec![0x51; MAX_FILE as usize]);
        f.put("data/resource_lab/init.txt", b"bytes");
        let mut mods = Mods::load_for(
            Runs::Both,
            std::slice::from_ref(&f.dir),
            |_| true,
            |_| false,
        );
        assert_eq!(
            resource_reply(&mut mods, server, "resource"),
            (
                format!(
                    "{}|init:some:16777216|resource callback budget exhausted|resource callback budget exhausted",
                    f.summary
                ),
                false
            )
        );
    }
}

#[test]
fn resources_advertised_oversize_is_refused_before_init() {
    use horizon_mods::content::MAX_FILE;
    let f = ResourceFixture::new(false, "resource_lab", "resource");
    let file = f.dir.join(RESOURCE_LARGE);
    std::fs::File::create(&file)
        .unwrap()
        .set_len(MAX_FILE + 1)
        .unwrap();
    let manifest = std::fs::read_to_string(f.dir.join("horizon-mod.toml")).unwrap();
    let large = vec![0x43; MAX_FILE as usize + 1];
    std::fs::write(
        &f.archive,
        Package::write_files(
            &manifest,
            Some(&resource_guests()[0]),
            &[(RESOURCE_LARGE, &large)],
        )
        .unwrap(),
    )
    .unwrap();
    for path in [&f.dir, &f.archive] {
        let mods = Mods::load(std::slice::from_ref(path), |_| true, |_| false);
        assert!(
            matches!(&mods.entries()[0].status, Status::Failed(e) if e == "data/resource_lab/large.bin is over 16 MiB")
        );
        assert!(!mods.is_running());
        assert_eq!(mods.commands().count(), 0);
    }
}

#[test]
fn resources_require_declared_permission_opt_in_and_code() {
    for server in [false, true] {
        let f = ResourceFixture::new(server, "resource_lab", "resource");
        let text = std::fs::read_to_string(f.dir.join("horizon-mod.toml")).unwrap();
        let undeclared = if server {
            std::fs::remove_dir_all(f.dir.join("data")).unwrap();
            std::fs::remove_dir_all(f.dir.join("assets")).unwrap();
            text.replace(
                "capabilities = [\"resources\", \"players\"]",
                "capabilities = [\"players\"]",
            )
            .replace("namespaces = [\"extras_lab\"]", "namespaces = []")
        } else {
            text.replace(
                "capabilities = [\"resources\"]",
                "capabilities = [\"registries\"]",
            )
        };
        f.put("horizon-mod.toml", undeclared.as_bytes());
        let mods = Mods::load(std::slice::from_ref(&f.dir), |_| true, |_| false);
        assert_eq!(
            mods.entries()[0].status,
            Status::Failed("uses capability resources, missing from horizon-mod.toml".into())
        );
        f.put(
            "horizon-mod.toml",
            text.replace("experimental = [\"resources\"]", "experimental = []")
                .as_bytes(),
        );
        let mods = Mods::load(std::slice::from_ref(&f.dir), |_| true, |_| false);
        assert!(
            matches!(&mods.entries()[0].status, Status::Failed(e) if e.contains("experimental = [\"resources\"]"))
        );
        f.put("horizon-mod.toml", text.as_bytes());
        std::fs::remove_file(f.dir.join("mod.wasm")).unwrap();
        assert!(
            Package::read(&f.dir)
                .err()
                .unwrap()
                .contains("only a content mod")
        );
    }
}

#[test]
fn resources_execution_contexts_preserve_both_and_off_behavior() {
    for side in ["client", "both", "server"] {
        let server = side == "server";
        let f = ResourceFixture::new(server, "resource_lab", "resource");
        f.put(
            "horizon-mod.toml",
            resource_manifest("resource_lab", side).as_bytes(),
        );
        for runs in [Runs::Client, Runs::Server, Runs::Both] {
            let expected = runs == Runs::Both || ((runs == Runs::Server) == server);
            let mut mods = Mods::load_for(runs, std::slice::from_ref(&f.dir), |_| true, |_| false);
            assert_eq!(
                mods.entries()[0].status,
                if expected {
                    Status::Running
                } else {
                    Status::Elsewhere
                }
            );
            assert_eq!(mods.is_running(), expected);
            if expected {
                assert_eq!(
                    resource_reply(&mut mods, server, "resource"),
                    (f.summary.clone(), false)
                );
            }
        }
        let mut off = Mods::load_for(
            Runs::Both,
            std::slice::from_ref(&f.dir),
            |_| false,
            |_| false,
        );
        assert_eq!(off.entries()[0].status, Status::Off);
        assert!(!off.is_running());
        off.tick(1);
        off.frame(0.1);
        off.server_tick(1);
        assert!(off.drain().is_empty());
        assert!(!off.command("resource"));
        f.put("mod.wasm", &resource_guests()[usize::from(!server)]);
        let wrong = Mods::load_for(
            Runs::Both,
            std::slice::from_ref(&f.dir),
            |_| true,
            |_| false,
        );
        assert!(
            matches!(wrong.entries()[0].status, Status::Failed(_)),
            "wrong exports must not create a new Side::Both server half"
        );
    }
}

#[test]
fn resources_reload_revalidates_inventory_side_and_failed_owners() {
    for server in [false, true] {
        let first = ResourceFixture::new(server, "resource_lab", "resource");
        let healthy = ResourceFixture::new(server, "other_lab", "healthy");
        let paths = [first.dir.clone(), healthy.dir.clone()];
        let runs = if server { Runs::Server } else { Runs::Client };
        let mut mods = Mods::load_for(runs, &paths, |_| true, |_| false);
        assert_eq!(
            resource_reply(&mut mods, server, "resource"),
            (first.summary.clone(), false)
        );
        first.put("data/resource_lab/new.bin", b"fresh inventory");
        let changed = format!(r#"{{"nonce":"{}-reload"}}"#, first.nonce);
        first.put(RESOURCE_JSON, changed.as_bytes());
        assert_eq!(
            resource_reply(&mut mods, server, "resource read data/resource_lab/new.bin"),
            ("none".into(), false)
        );
        assert_eq!(
            resource_reply(&mut mods, server, "resource"),
            (first.summary.clone(), false)
        );
        assert_eq!(
            resource_reply(&mut mods, server, &format!("resource read {RESOURCE_JSON}")),
            (
                format!(
                    "some:{}:{}",
                    changed.len(),
                    changed
                        .as_bytes()
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                ),
                false
            )
        );
        first.touch();
        assert!(mods.reload_changed(|_| false));
        let expected = first
            .summary
            .replace(
                &format!("json:{}:", changed.len() - 7),
                &format!("json:{}:", changed.len()),
            )
            .replace(&first.nonce, &format!("{}-reload", first.nonce));
        assert_eq!(
            resource_reply(&mut mods, server, "resource"),
            (expected, false)
        );
        assert_eq!(
            resource_reply(&mut mods, server, "resource read data/resource_lab/new.bin"),
            ("some:15:667265736820696e76656e746f7279".into(), false)
        );
        first.put(
            "horizon-mod.toml",
            resource_manifest("resource_lab", if server { "client" } else { "server" }).as_bytes(),
        );
        first.touch();
        assert!(mods.reload_changed(|_| false));
        assert_eq!(mods.entries()[0].status, Status::Elsewhere);
        assert_eq!(
            resource_reply(&mut mods, server, "healthy"),
            (healthy.summary.clone(), false)
        );
        assert!(!mods.command("resource"));
        assert!(!mods.server_command(&alex(), "resource"));
        first.put(
            "horizon-mod.toml",
            resource_manifest("resource_lab", if server { "server" } else { "client" }).as_bytes(),
        );
        first.touch();
        assert!(mods.reload_changed(|_| false));
        assert_eq!(mods.entries()[0].status, Status::Running);
        first.put("data/unowned/invalid.bin", b"invalid content");
        first.touch();
        assert!(mods.reload_changed(|_| false));
        assert!(matches!(mods.entries()[0].status, Status::Failed(_)));
        assert!(!mods.command("resource"));
        assert!(!mods.server_command(&alex(), "resource"));
        assert_eq!(
            resource_reply(&mut mods, server, "healthy"),
            (healthy.summary.clone(), false)
        );
        std::fs::remove_dir_all(first.dir.join("data/unowned")).unwrap();
        first.put("data/resource_lab/init.txt", b"fail");
        first.touch();
        assert!(mods.reload_changed(|_| false));
        assert_eq!(
            mods.entries()[0].status,
            Status::Failed("requested init failure".into())
        );
        assert!(!mods.command("resource"));
        assert!(!mods.server_command(&alex(), "resource"));
        assert_eq!(
            resource_reply(&mut mods, server, "healthy"),
            (healthy.summary.clone(), false)
        );
    }
}

#[test]
fn resources_archive_reader_is_replaced_only_on_reload() {
    let f = ResourceFixture::new(false, "resource_lab", "resource");
    f.pack();
    let mut mods = Mods::load(std::slice::from_ref(&f.archive), |_| true, |_| false);
    f.put(RESOURCE_EXTRA, b"updated");
    f.pack();
    assert_eq!(
        resource_reply(&mut mods, false, &format!("resource read {RESOURCE_EXTRA}")),
        ("some:8:7265736f75726365".into(), false)
    );
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&f.archive)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert!(mods.reload_changed(|_| false));
    assert!(!mods.has_command("resource"));
    assert_eq!(
        resource_reply(&mut mods, false, "updated"),
        (
            f.summary.replace("extra:8:resource", "extra:7:updated"),
            false
        )
    );
}

#[cfg(target_os = "linux")]
#[test]
fn resources_package_handles_close_on_trap_failure_off_and_drop() {
    let handles = |root: &Path| {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .flatten()
            .filter_map(|e| std::fs::read_link(e.path()).ok())
            .filter(|p| p.starts_with(root))
            .count()
    };
    let f = ResourceFixture::new(false, "resource_lab", "resource");
    f.pack();
    assert_eq!(handles(&f.root), 0);
    let mut mods = Mods::load(std::slice::from_ref(&f.archive), |_| true, |_| false);
    assert_eq!(handles(&f.root), 1, "one live ZIP index per store");
    assert!(mods.command("resource panic"));
    assert!(matches!(mods.entries()[0].status, Status::Failed(_)));
    assert_eq!(handles(&f.root), 0);
    let off = Mods::load(std::slice::from_ref(&f.dir), |_| false, |_| false);
    assert_eq!(handles(&f.root), 0);
    drop(off);
    f.put("data/resource_lab/init.txt", b"fail");
    let failed = Mods::load(std::slice::from_ref(&f.dir), |_| true, |_| false);
    assert_eq!(
        failed.entries()[0].status,
        Status::Failed("requested init failure".into())
    );
    assert_eq!(handles(&f.root), 0);
    drop(failed);
    std::fs::remove_file(f.dir.join("data/resource_lab/init.txt")).unwrap();
    let live = Mods::load(std::slice::from_ref(&f.dir), |_| true, |_| false);
    assert!(handles(&f.root) >= 1);
    drop(live);
    assert_eq!(handles(&f.root), 0);
}

fn entity_component() -> &'static [u8] {
    static COMPONENT: OnceLock<Vec<u8>> = OnceLock::new();
    COMPONENT.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("horizon-entity-tests"))
            .join("entity-guest");
        let status = Command::new(env!("CARGO"))
            .current_dir(&root)
            .args([
                "build",
                "--locked",
                "--release",
                "--target",
                "wasm32-unknown-unknown",
                "-p",
                "horizon-mods-test-entity-server-guest",
                "--target-dir",
            ])
            .arg(&target)
            .status()
            .unwrap();
        assert!(status.success(), "compile the actual Rust SDK server guest");
        let core = std::fs::read(
            target
                .join("wasm32-unknown-unknown/release/horizon_mods_test_entity_server_guest.wasm"),
        )
        .unwrap();
        let component = componentize(&core).unwrap();
        std::fs::write(target.join("entity-component.wasm"), &component).unwrap();
        if let Some(dir) = std::env::var_os("HORIZON_ENTITY_EVIDENCE_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(Path::new(&dir).join("entity-core.wasm"), &core).unwrap();
            std::fs::write(Path::new(&dir).join("entity-component.wasm"), &component).unwrap();
        }
        component
    })
}
fn entity_manifest(id: &str) -> String {
    format!(
        r#"id = "{id}"
name = "{id}"
version = "0.1.0"
api = "0.3"
side = "server"
capabilities = ["entities", "resources", "players"]
experimental = ["entities", "resources"]
namespaces = ["entity_lab"]
"#
    )
}
struct EntityFixture {
    root: PathBuf,
    id: String,
    command: String,
    nonce: Vec<u8>,
    update: Vec<u8>,
}
impl EntityFixture {
    fn new(id: &str, command: &str) -> Self {
        let component = entity_component();
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut nonce = now.to_le_bytes().to_vec();
        nonce.extend_from_slice(&serial.to_le_bytes());
        assert!(!component.windows(nonce.len()).any(|w| w == nonce));
        let root = std::env::temp_dir().join(format!("he-{}-{serial}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let fixture = Self {
            root,
            id: id.into(),
            command: command.into(),
            nonce,
            update: format!("updated-{now:x}").into_bytes(),
        };
        fixture.put("mod.wasm", component);
        fixture.put("horizon-mod.toml", entity_manifest(id).as_bytes());
        fixture.mode("");
        fixture.put("data/entity_lab/nonce.bin", &fixture.nonce);
        fixture.put("data/entity_lab/update.bin", &fixture.update);
        eprintln!(
            "entity input after compilation: {} nonce={:?} update={:?}",
            fixture.root.display(),
            fixture.nonce,
            fixture.update
        );
        fixture
    }
    fn put(&self, name: &str, bytes: &[u8]) {
        let path = self.root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    fn mode(&self, mode: &str) {
        self.put(
            "data/entity_lab/config.txt",
            format!("{}|{}|{mode}", self.id, self.command).as_bytes(),
        );
    }
    fn touch(&self) {
        let p = self.root.join("mod.wasm");
        let now = std::fs::metadata(&p).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(now + std::time::Duration::from_secs(2))
            .unwrap();
    }
    fn archive(&self) -> PathBuf {
        let manifest = std::fs::read_to_string(self.root.join("horizon-mod.toml")).unwrap();
        let files: Vec<_> = ["config.txt", "nonce.bin", "update.bin"]
            .into_iter()
            .map(|n| {
                let name = format!("data/entity_lab/{n}");
                let bytes = std::fs::read(self.root.join(&name)).unwrap();
                (name, bytes)
            })
            .collect();
        let refs: Vec<_> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let bytes = Package::write_files(&manifest, Some(entity_component()), &refs).unwrap();
        let path = self.root.join("fixture.hmod");
        std::fs::write(&path, bytes).unwrap();
        path
    }
    fn load(&self, path: &Path) -> Mods {
        let mods = Mods::load_for(Runs::Server, &[path.to_owned()], |_| true, |_| false);
        assert_eq!(mods.entries()[0].status, Status::Running);
        mods
    }
}
impl Drop for EntityFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
struct EntityHost {
    id: horizon_mods::entities::WorldId,
    next: u32,
    levels: Vec<(String, horizon_mods::entities::Level)>,
}
impl EntityHost {
    fn new() -> Self {
        Self {
            id: horizon_mods::entities::WorldId::new().unwrap(),
            next: 41,
            levels: vec![("minecraft:overworld".into(), Default::default())],
        }
    }
    fn lend(&mut self, mods: &mut Mods) {
        let levels = self
            .levels
            .iter_mut()
            .map(|(d, l)| l.loaded(d, -64, 320, [(0, 0)]))
            .collect();
        mods.lend_entities(horizon_mods::entities::World::new(
            self.id, self.next, levels,
        ));
    }
    fn returned(&mut self, mods: &mut Mods) {
        if let Some(w) = mods.take_entities() {
            self.next = w.last_id();
        }
        for (_, l) in &mut self.levels {
            l.prune();
        }
    }
    fn records(&self) -> Vec<horizon_mods::entities::Record> {
        self.levels.iter().flat_map(|(_, l)| l.records()).collect()
    }
    fn command(&mut self, mods: &mut Mods, line: &str) -> (String, bool) {
        self.in_dimension(mods, line, "minecraft:overworld")
    }
    fn in_dimension(&mut self, mods: &mut Mods, line: &str, dimension: &str) -> (String, bool) {
        let player = ServerPlayer {
            x: 0.5,
            y: 64.0,
            z: 0.5,
            dimension: dimension.into(),
            ..alex()
        };
        mods.set_snapshot(Snapshot {
            players: vec![player.clone()],
            ..Default::default()
        });
        mods.drain();
        self.lend(mods);
        assert!(mods.server_command(&player, line));
        self.returned(mods);
        let out = mods.drain();
        eprintln!(
            "entity actual output: {line}: {out:?}; applied={:?}",
            self.records().len()
        );
        out.into_iter()
            .find_map(|o| match o {
                Output::Reply { text, error, .. } => Some((text, error)),
                Output::Stopped { reason, .. } => Some((reason, true)),
                _ => None,
            })
            .expect("guest reply or trap")
    }
    fn close(&mut self) {
        for (_, l) in &mut self.levels {
            l.clear(horizon_mods::EntityReason::WorldUnloaded);
            assert!(!l.allocated());
        }
        self.id = horizon_mods::entities::WorldId::new().unwrap();
    }
}
#[test]
fn entities_real_guest_folder_archive_and_test_game_records() {
    let functions = horizon_mods::api_level::imports(entity_component()).unwrap();
    assert_eq!(
        horizon_mods::api_level::needed(functions.iter().map(String::as_str)).0,
        9
    );
    for f in [
        "register-types",
        "context",
        "spawn",
        "set-state",
        "despawn",
        "poll",
    ] {
        assert!(functions.contains(&format!("entities.{f}")));
    }
    for archive in [false, true] {
        let f = EntityFixture::new("entity_alpha", "entitya");
        let path = if archive { f.archive() } else { f.root.clone() };
        let mut mods = f.load(&path);
        let mut host = EntityHost::new();
        let (handle, error) = host.command(&mut mods, "entitya spawn");
        assert!(!error);
        let records = host.records();
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.handle, handle);
        assert_eq!(r.engine_id, 42);
        assert_eq!(r.kind.key, "entity_alpha:probe");
        assert_eq!(r.state.public_state, f.nonce);
        assert_eq!(r.state.transform.x, f64::from(f.nonce[0] % 8) + 0.5);
        assert_eq!(r.state.transform.y, 64.0 + f64::from(f.nonce[0] % 16));
        assert_eq!(
            r.state.transform.z,
            f64::from(f.nonce.last().unwrap() % 8) + 0.5
        );
        assert_eq!(
            host.command(&mut mods, "entitya update"),
            ("updated".into(), false)
        );
        let r = host.records().remove(0);
        assert_eq!(r.handle, handle);
        assert_eq!(r.revision, 2);
        assert_eq!(
            r.state.public_state,
            f.update.iter().rev().copied().collect::<Vec<_>>()
        );
        assert_eq!(
            host.command(&mut mods, "entitya despawn"),
            ("despawned".into(), false)
        );
        assert!(host.records().is_empty());
        assert!(!host.levels[0].1.allocated());
        assert!(host.command(&mut mods, "entitya stale").1);
        let (new, error) = host.command(&mut mods, "entitya spawn");
        assert!(!error);
        assert_ne!(new, handle);
        drop(mods);
        assert!(host.records().is_empty());
        host.levels[0].1.prune();
        assert!(!host.levels[0].1.allocated());
        let mut game = TestGame::load(&path).unwrap();
        let p = game.join("EntityFixture");
        assert!(game.player_command(&p, "entitya spawn"));
        assert_eq!(game.entity_records()[0].state.public_state, f.nonce);
        let script = Script::parse(&format!("[[step]]\ncommand = \"entitya update\"\n[step.expect]\nentities = 1\nentity-state = {:?}\n\n[[step]]\ncommand = \"entitya despawn\"\n[step.expect]\nentities = 0\n",f.update.iter().rev().copied().collect::<Vec<_>>())).unwrap();
        script.run(&mut game).unwrap();
    }
}
#[test]
fn entities_rejections_are_atomic_and_attempt_budget_resets() {
    let f = EntityFixture::new("entity_alpha", "entitya");
    let mut mods = f.load(&f.root);
    let mut host = EntityHost::new();
    host.command(&mut mods, "entitya spawn");
    let before = host.records().remove(0);
    let next = host.next;
    for (op, why) in [
        ("badfinite", "finite"),
        ("badchunk", "chunk"),
        ("baddim", "dimension"),
        ("badheight", "height"),
        ("badstate", "1024"),
        ("badrange", "range"),
        ("type-as-entity", "handle"),
        ("foreign e:0:0:0:0:0", "handle"),
        ("fail", "requested callback failure"),
        ("register-fail", "requested callback failure"),
    ] {
        let (text, error) = host.command(&mut mods, &format!("entitya {op}"));
        assert!(error && text.contains(why), "{op}: {text}");
        let after = host.records().remove(0);
        assert_eq!(after.state, before.state);
        assert_eq!(after.revision, before.revision);
        assert_eq!(host.next, next);
    }
    let (text, error) = host.command(&mut mods, "entitya attempts");
    assert!(!error && text.contains("attempt budget"));
    for bad in [
        "namespace",
        "renderer",
        "finite",
        "range",
        "version",
        "width",
    ] {
        assert!(host.command(&mut mods, &format!("entitya badtype {bad}")).1);
    }
    for axis in ["y", "z", "yaw", "pitch"] {
        assert!(
            host.command(&mut mods, &format!("entitya badfinite {axis}"))
                .0
                .contains("finite")
        );
    }
    assert_eq!(
        host.command(&mut mods, "entitya types 7"),
        ("types:7".into(), false)
    );
    assert!(
        host.command(&mut mods, "entitya types 1")
            .0
            .contains("eight")
    );
    let (kind, _) = host.command(&mut mods, "entitya kind");
    assert!(
        host.command(&mut mods, &format!("entitya foreign {kind}"))
            .1
    );
    assert!(
        host.command(&mut mods, &format!("entitya type-spawn {}", before.handle))
            .1
    );
    assert!(!host.command(&mut mods, "entitya spawn").1);
    f.put("data/entity_lab/update.bin", &vec![7; 1024]);
    assert!(!host.command(&mut mods, "entitya update").1);
    assert_eq!(
        host.records()
            .iter()
            .find(|r| r.revision == 2)
            .unwrap()
            .state
            .public_state
            .len(),
        1024
    );
    let maximum = EntityFixture::new("entity_maximum", "entitymax");
    let mut maximum_mods = maximum.load(&maximum.root);
    assert!(!host.command(&mut maximum_mods, "entitymax typebounds").1);
    host.next = i32::MAX as u32;
    assert!(
        host.command(&mut mods, "entitya spawn")
            .0
            .contains("ID exhausted")
    );
    assert_eq!(host.records().len(), 2);
    assert_eq!(host.next, i32::MAX as u32);
    host.next = u32::MAX;
    assert!(
        host.command(&mut mods, "entitya spawn")
            .0
            .contains("ID exhausted")
    );
    assert_eq!(host.next, u32::MAX);
}
#[test]
fn entities_two_healthy_owners_survive_foreign_calls_trap_and_disable() {
    let a = EntityFixture::new("entity_alpha", "entitya");
    let b = EntityFixture::new("entity_beta", "entityb");
    let c = EntityFixture::new("entity_gamma", "entityc");
    let mut mods = Mods::load_for(
        Runs::Server,
        &[a.root.clone(), b.root.clone(), c.root.clone()],
        |_| true,
        |_| false,
    );
    assert!(mods.entries().iter().all(|e| e.status == Status::Running));
    let mut host = EntityHost::new();
    let (ah, _) = host.command(&mut mods, "entitya spawn");
    host.command(&mut mods, "entityb spawn");
    host.command(&mut mods, "entityc spawn");
    let before: Vec<_> = host
        .records()
        .iter()
        .take(2)
        .map(|r| (r.handle.clone(), r.state.clone()))
        .collect();
    assert!(host.command(&mut mods, &format!("entityb foreign {ah}")).1);
    let (ak, _) = host.command(&mut mods, "entitya kind");
    assert!(
        host.command(&mut mods, &format!("entityb type-spawn {ak}"))
            .1
    );
    let next = host.next;
    assert!(host.command(&mut mods, "entityc trap").1);
    assert_eq!(host.next, next);
    assert_eq!(host.records().len(), 2);
    assert!(matches!(mods.entries()[2].status, Status::Failed(_)));
    for (h, s) in before {
        assert_eq!(
            host.records().iter().find(|r| r.handle == h).unwrap().state,
            s
        );
    }
    assert!(!host.command(&mut mods, "entitya update").1);
    assert!(!host.command(&mut mods, "entityb update").1);
    assert!(mods.disable("entity_alpha"));
    assert_eq!(host.records().len(), 1);
    assert_eq!(host.records()[0].kind.key, "entity_beta:probe");
    assert!(!host.command(&mut mods, "entityb update").1);
    mods.sync_enabled(|id| id == "entity_beta" || id == "entity_alpha", |_| false);
    assert_eq!(host.records().len(), 1);
    assert!(!host.command(&mut mods, "entitya spawn").1);
    assert_eq!(host.records().len(), 2);
}
#[test]
fn entities_owner_world_quotas_and_maximum_counts() {
    let fixtures: Vec<_> = (0..5)
        .map(|i| EntityFixture::new(&format!("entity_{i}"), &format!("entity{i}")))
        .collect();
    let paths: Vec<_> = fixtures.iter().map(|f| f.root.clone()).collect();
    let mut mods = Mods::load_for(Runs::Server, &paths, |_| true, |_| false);
    let mut host = EntityHost::new();
    for i in 0..4 {
        assert_eq!(
            host.command(&mut mods, &format!("entity{i} fill")),
            ("filled".into(), false)
        );
        assert_eq!(host.records().len(), (i + 1) * 64);
    }
    let next = host.next;
    assert!(host.command(&mut mods, "entity0 spawn").0.contains("quota"));
    assert_eq!(host.next, next);
    assert!(host.command(&mut mods, "entity4 spawn").0.contains("quota"));
    assert_eq!(host.next, next);
    assert_eq!(host.records().len(), 256);
    mods.disable("entity_2");
    assert_eq!(host.records().len(), 192);
    assert_eq!(
        host.command(&mut mods, "entity4 fill"),
        ("filled".into(), false)
    );
    assert_eq!(host.records().len(), 256);
    drop(mods);
    assert!(host.records().is_empty());
    host.levels[0].1.prune();
    assert!(!host.levels[0].1.allocated());
}
#[test]
fn entities_chunk_dimension_world_reopen_and_reload_failure_cleanup() {
    let a = EntityFixture::new("entity_alpha", "entitya");
    let b = EntityFixture::new("entity_beta", "entityb");
    let mut mods = Mods::load_for(
        Runs::Server,
        &[a.root.clone(), b.root.clone()],
        |_| true,
        |_| false,
    );
    let mut host = EntityHost::new();
    host.levels
        .push(("minecraft:the_nether".into(), Default::default()));
    assert_eq!(
        host.command(&mut mods, "entitya context").0,
        format!("{}:minecraft:overworld,minecraft:the_nether", host.id.get())
    );
    let (old, _) = host.command(&mut mods, "entitya spawn");
    assert!(
        !host
            .in_dimension(&mut mods, "entityb spawn", "minecraft:the_nether")
            .1
    );
    host.levels[0]
        .1
        .unload_chunks(&[(0, 0)].into_iter().collect());
    assert_eq!(host.records().len(), 1);
    assert!(
        host.command(&mut mods, "entitya poll")
            .0
            .contains("ChunkUnloaded")
    );
    assert!(host.command(&mut mods, &format!("entitya foreign {old}")).1);
    host.levels.pop();
    assert!(host.records().is_empty());
    assert!(
        host.command(&mut mods, "entityb poll")
            .0
            .contains("DimensionUnloaded")
    );
    for _ in 0..8 {
        let (old, _) = host.command(&mut mods, "entitya spawn");
        host.close();
        assert!(
            host.command(&mut mods, "entitya poll")
                .0
                .contains("WorldUnloaded")
        );
        assert!(host.command(&mut mods, &format!("entitya foreign {old}")).1);
        assert!(host.records().is_empty());
        assert!(!host.levels[0].1.allocated());
    }
    host.command(&mut mods, "entitya spawn");
    host.command(&mut mods, "entityb spawn");
    let healthy = host
        .records()
        .iter()
        .find(|r| r.kind.key == "entity_beta:probe")
        .unwrap()
        .handle
        .clone();
    a.mode("spawnfail");
    a.touch();
    host.lend(&mut mods);
    let next = host.next;
    assert!(mods.reload_changed(|_| false));
    host.returned(&mut mods);
    assert!(
        matches!(&mods.entries()[0].status,Status::Failed(e) if e.contains("failure after spawn"))
    );
    assert_eq!(host.records().len(), 1);
    assert_eq!(host.records()[0].handle, healthy);
    assert_eq!(host.next, next);
    assert_eq!(mods.server_commands().count(), 1);
    assert!(!host.command(&mut mods, "entityb update").1);
    a.mode("");
    a.touch();
    assert!(mods.reload_changed(|_| false));
    assert_eq!(mods.entries()[0].status, Status::Running);
    assert!(!host.command(&mut mods, "entitya spawn").1);
    a.put("mod.wasm", b"invalid");
    a.touch();
    assert!(mods.reload_changed(|_| false));
    assert_eq!(host.records().len(), 1);
    assert_eq!(host.records()[0].handle, healthy);
}
#[test]
fn entities_optin_import_permissions_off_side_and_init_failure() {
    let f = EntityFixture::new("entity_alpha", "entitya");
    let off = Mods::load_for(
        Runs::Server,
        std::slice::from_ref(&f.root),
        |_| false,
        |_| false,
    );
    assert!(!off.wants_entities());
    assert_eq!(off.entries()[0].status, Status::Off);
    let elsewhere = Mods::load_for(
        Runs::Client,
        std::slice::from_ref(&f.root),
        |_| true,
        |_| false,
    );
    assert!(!elsewhere.wants_entities());
    assert_eq!(elsewhere.entries()[0].status, Status::Elsewhere);
    let manifest = entity_manifest(&f.id);
    f.put(
        "horizon-mod.toml",
        manifest
            .replacen(
                "[\"entities\", \"resources\", \"players\"]",
                "[\"resources\", \"players\"]",
                1,
            )
            .as_bytes(),
    );
    let mods = Mods::load_for(
        Runs::Server,
        std::slice::from_ref(&f.root),
        |_| true,
        |_| false,
    );
    assert!(
        matches!(&mods.entries()[0].status,Status::Failed(e) if e.contains("uses capability entities"))
    );
    f.put(
        "horizon-mod.toml",
        manifest
            .replace(
                "experimental = [\"entities\", \"resources\"]",
                "experimental = [\"resources\"]",
            )
            .as_bytes(),
    );
    let mods = Mods::load_for(
        Runs::Server,
        std::slice::from_ref(&f.root),
        |_| true,
        |_| false,
    );
    assert!(
        matches!(&mods.entries()[0].status,Status::Failed(e) if e.contains("entities is experimental"))
    );
    assert!(
        horizon_mods::Manifest::parse(
            &manifest
                .replace("api = \"0.3\"", "api = \"0.2\"")
                .replace("side = \"server\"", "side = \"client\"")
        )
        .unwrap_err()
        .contains("for server mods")
    );
    f.put("horizon-mod.toml", manifest.as_bytes());
    for mode in ["initfail", "inittrap"] {
        f.mode(mode);
        let mut mods = Mods::load_for(
            Runs::Server,
            std::slice::from_ref(&f.root),
            |_| true,
            |_| false,
        );
        assert!(matches!(mods.entries()[0].status, Status::Failed(_)));
        assert!(!mods.wants_entities());
        assert_eq!(mods.server_commands().count(), 0);
        let mut host = EntityHost::new();
        host.lend(&mut mods);
        host.returned(&mut mods);
        assert!(host.records().is_empty());
        assert!(!host.levels[0].1.allocated());
    }
}

#[test]
fn entities_keep_a_real_pre_entity_sdk_guest_compatible() {
    let fixture = EntityFixture::new("entity_alpha", "entitya");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let old = fixture.root.join("old");
    let sdk = old.join("sdk");
    for file in ["Cargo.toml", "src/lib.rs", "wit/horizon.wit"] {
        let spec =
            format!("1feb28c0452cde2587375357f56ac606c5de7276:crates/horizon-mod-sdk/{file}");
        let out = Command::new("git")
            .current_dir(&root)
            .args(["show", &spec])
            .output()
            .unwrap();
        assert!(out.status.success());
        let mut bytes = out.stdout;
        if file == "Cargo.toml" {
            bytes = String::from_utf8(bytes)
                .unwrap()
                .replace("edition.workspace = true", "edition = \"2024\"")
                .replace("license.workspace = true", "license = \"MIT\"")
                .replace("[lints]\nworkspace = true", "")
                .into_bytes();
        }
        let path = sdk.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    std::fs::create_dir_all(old.join("src")).unwrap();
    std::fs::write(old.join("Cargo.toml"),"[workspace]\n[package]\nname=\"old-server-entity-check\"\nversion=\"0.1.0\"\nedition=\"2024\"\n[lib]\ncrate-type=[\"cdylib\"]\n[dependencies]\nhorizon-mod-sdk={path=\"sdk\"}\n").unwrap();
    std::fs::write(old.join("src/lib.rs"),r#"use horizon_mod_sdk::server::{Args, Player, Registration, ServerMod};
struct Old;
impl ServerMod for Old {
    fn init() -> Result<(Self,Registration),String> { Ok((Self,Registration::new().command("oldentity"))) }
    fn command(&mut self,_p:&Player,_n:&str,_a:&Args)->Result<Option<String>,String> { Ok(Some("old server still works".into())) }
}
horizon_mod_sdk::export_server_mod!(Old);
"#).unwrap();
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("old-entity-sdk");
    assert!(
        Command::new(env!("CARGO"))
            .current_dir(&old)
            .args([
                "build",
                "--release",
                "--target",
                "wasm32-unknown-unknown",
                "--target-dir"
            ])
            .arg(&target)
            .status()
            .unwrap()
            .success()
    );
    let core =
        std::fs::read(target.join("wasm32-unknown-unknown/release/old_server_entity_check.wasm"))
            .unwrap();
    let component = componentize(&core).unwrap();
    assert_eq!(
        horizon_mods::api_level::needed(
            horizon_mods::api_level::imports(&component)
                .unwrap()
                .iter()
                .map(String::as_str)
        )
        .0,
        0
    );
    let manifest = "id=\"old_entity\"\nname=\"Old entity compatibility\"\nversion=\"0.1.0\"\napi=\"0.3\"\nside=\"server\"\ncapabilities=[\"players\"]\n";
    let folder = old.join("package");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("horizon-mod.toml"), manifest).unwrap();
    std::fs::write(folder.join("mod.wasm"), &component).unwrap();
    let archive = old.join("old.hmod");
    std::fs::write(&archive, Package::write(manifest, &component).unwrap()).unwrap();
    for path in [folder, archive] {
        let mut mods = Mods::load_for(Runs::Server, &[path], |_| true, |_| false);
        assert_eq!(mods.entries()[0].status, Status::Running);
        assert!(!mods.wants_entities());
        assert!(mods.server_command(&alex(), "oldentity"));
        assert!(mods.drain().iter().any(
            |o| matches!(o,Output::Reply{text,error:false,..} if text=="old server still works")
        ));
    }
}
#[test]
fn entities_future_import_has_the_useful_update_error() {
    use wit_parser::{ManglingAndAbi, Resolve};
    let f = EntityFixture::new("entity_alpha", "entitya");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wit = std::fs::read_to_string(root.join("crates/horizon-mod-sdk/wit/horizon.wit"))
        .unwrap()
        .replace(
            "interface entities {",
            "interface entities {\n    future-spawn: func();",
        );
    let mut resolve = Resolve::default();
    let package = resolve.push_str("horizon.wit", &wit).unwrap();
    let world = resolve
        .select_world(&[package], Some("horizon-server-mod"))
        .unwrap();
    let mut module = wit_component::dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    wit_component::embed_component_metadata(
        &mut module,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .unwrap();
    let component = componentize(&module).unwrap();
    let manifest = entity_manifest(&f.id).replace(
        "[\"entities\", \"resources\", \"players\"]",
        "[\"entities\", \"resources\", \"players\", \"storage\", \"world\"]",
    );
    f.put("mod.wasm", &component);
    f.put("horizon-mod.toml", manifest.as_bytes());
    let mods = Mods::load_for(
        Runs::Server,
        std::slice::from_ref(&f.root),
        |_| true,
        |_| false,
    );
    assert_eq!(
        mods.entries()[0].status,
        Status::Failed(
            "made for a newer Horizon (it uses entities.future-spawn): update Horizon to run it"
                .into()
        )
    );
}

#[path = "../../horizon-client/src/game/mod_enable.rs"]
mod game_mod_enable;

#[test]
fn selective_enable_restores_saved_keys_and_preserves_healthy_entity_owner() {
    let healthy = EntityFixture::new("entity_keys_healthy", "entitykeyshealthy");
    let paths = [
        mods_dir().join("horizon_mods_test_ui_guest.hmod"),
        healthy.root.clone(),
    ];
    let mut mods = Mods::load_for(Runs::Both, &paths, |_| true, |_| false);
    assert!(mods.entries().iter().all(|e| e.status == Status::Running));
    assert!(mods.command("input_claim"));
    assert_eq!(mods.keys_of(0)[0].key, "key.keyboard.r");
    let saved = |id: &str| {
        if id == "test_ui_guest" {
            vec![("wheel", "key.keyboard.g"), ("recipe", "key.keyboard.h")]
        } else {
            Vec::new()
        }
    };
    // Startup choice only. Both toggle transitions use the game's helper.
    mods.set_bindings(&saved);
    assert_eq!(mods.keys_of(0)[0].key, "key.keyboard.g");
    let mut host = EntityHost::new();
    assert!(!host.command(&mut mods, "entitykeyshealthy spawn").1);
    let original = host.records().remove(0);
    let next = host.next;
    let mut revision = original.revision;

    for _ in 0..2 {
        game_mod_enable::sync_enabled(&mut mods, |id| id != "test_ui_guest", |_| false, &saved);
        assert_eq!(mods.entries()[0].status, Status::Off);
        assert!(mods.keys_of(0).is_empty());
        assert!(!mods.command("input_claim"));
        assert_eq!(mods.entries()[1].status, Status::Running);
        let records = host.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].handle, original.handle);
        assert_eq!(records[0].engine_id, original.engine_id);
        assert_eq!(records[0].revision, revision);

        game_mod_enable::sync_enabled(&mut mods, |_| true, |_| false, &saved);
        assert_eq!(mods.entries()[0].status, Status::Running);
        assert!(mods.keys_of(0).is_empty());
        assert!(mods.command("input_claim"));
        assert_eq!(
            mods.keys_of(0)[0].key,
            "key.keyboard.g",
            "selective enable must restore saved G before default R is registered"
        );
        assert!(mods.command("browser minecraft:stone"));
        let keys: Vec<_> = mods
            .keys_of(0)
            .iter()
            .map(|key| (key.name.as_str(), key.key.as_str()))
            .collect();
        assert_eq!(
            keys,
            [
                ("wheel", "key.keyboard.g"),
                ("recipe", "key.keyboard.h"),
                ("uses", "key.keyboard.u"),
            ]
        );
        mods.drain();
        mods.key("key.keyboard.r", true);
        mods.key("key.keyboard.r", false);
        assert!(mods.command("input_poll"));
        let events: Vec<_> = mods
            .drain()
            .into_iter()
            .filter_map(|output| match output {
                Output::Chat { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        assert!(events.is_empty(), "default R must not trigger saved G");
        mods.key("key.keyboard.g", true);
        mods.key("key.keyboard.g", false);
        assert!(mods.command("input_poll"));
        let events: Vec<_> = mods
            .drain()
            .into_iter()
            .filter_map(|output| match output {
                Output::Chat { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(events, ["key-down wheel", "key-up wheel"]);

        assert!(!host.command(&mut mods, "entitykeyshealthy update").1);
        revision += 1;
        let records = host.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].handle, original.handle);
        assert_eq!(records[0].engine_id, original.engine_id);
        assert_eq!(records[0].revision, revision);
        assert_eq!(
            records[0].state.public_state,
            healthy.update.iter().rev().copied().collect::<Vec<_>>()
        );
        assert_eq!(host.next, next);
    }
}
