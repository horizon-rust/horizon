//! `/tp`'s motion and wake-up: `TeleportCommand.getRelatives` names the axes a
//! relative destination is relative on, so the motion along them survives the
//! move (`PositionMoveRotation.calculateAbsolute`), and
//! `ServerPlayer.teleportTo` wakes a sleeper before the move.
use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;

fn integrated() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
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
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
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
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
    );
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 100.0, 0.5)).unwrap());
    game
}

/// A motion on every axis, so a kept one shows.
fn moving() -> Vec3 {
    Vec3::new(0.3, 0.6, -0.2)
}

/// Sets the player's motion, runs `command`, and answers the motion it left.
fn after(game: &mut Game<'_>, command: &str) -> Vec3 {
    if let Some(p) = game.player.as_mut() {
        p.velocity = moving();
    }
    assert!(game.run_command(command), "{command}");
    game.player.as_ref().unwrap().velocity
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_relative_teleport_keeps_the_motion_of_the_axes_it_names() {
    let mut game = integrated();
    // `~ ~1 ~` is relative on every axis, so `getRelatives` names
    // `Relative.DELTA_X/Y/Z` and the move carries the player's own motion.
    assert_eq!(after(&mut game, "tp @s ~ ~1 ~"), moving());
    // An absolute x is not named: its motion goes, the others stay.
    assert_eq!(after(&mut game, "tp @s 0.5 ~ ~"), Vec3::new(0.0, 0.6, -0.2));
    // `LocalCoordinates` is relative on every axis (`isXRelative`).
    assert_eq!(after(&mut game, "tp @s ^ ^ ^2"), moving());
    // An absolute destination carries no motion at all: the player stops dead.
    assert_eq!(after(&mut game, "tp @s 0.5 100.0 0.5"), Vec3::ZERO);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_relative_teleport_of_a_mob_keeps_the_motion_it_names() {
    let mut game = integrated();
    let kind = game
        .res
        .mob_types()
        .get("minecraft:zombie")
        .unwrap()
        .clone();
    game.mobs.push(horizon_client::mobs::Mob::new(
        42,
        kind,
        Vec3::new(0.5, 100.0, 0.5),
        0.0,
    ));
    let left = |game: &mut Game<'_>, command: &str| {
        game.mobs[0].velocity = moving();
        assert!(game.run_command(command), "{command}");
        game.mobs[0].velocity
    };
    // A mob is the server's own, so `performTeleport` zeroes its vertical
    // motion after the move and keeps the relative axes' horizontal motion.
    assert_eq!(
        left(&mut game, "tp @e[type=minecraft:zombie,limit=1] ~ ~ ~2"),
        Vec3::new(0.3, 0.0, -0.2)
    );
    assert_eq!(
        left(
            &mut game,
            "tp @e[type=minecraft:zombie,limit=1] 0.5 100.0 0.5"
        ),
        Vec3::ZERO
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_teleport_wakes_a_sleeping_player_before_it_moves_them() {
    let mut game = integrated();
    // A bed (foot at 4,100,4 facing south, head at 4,100,5) on a stone floor.
    assert!(game.run_command("fill 0 99 0 8 99 8 minecraft:stone"));
    assert!(game.run_command("setblock 4 100 4 minecraft:red_bed[part=foot,facing=south]"));
    assert!(game.run_command("setblock 4 100 5 minecraft:red_bed[part=head,facing=south]"));
    if let Some(p) = game.player.as_mut() {
        p.position = Vec3::new(4.5, 100.5625, 4.5);
    }
    game.sleeping = Some([4, 100, 4]);
    game.sleep_timer = 100;
    // `ServerPlayer.teleportTo` calls `stopSleepInBed(true, true)` first: the
    // player gets up (no wake-up fade) and the move takes them from where the
    // command read their position, not from beside the bed.
    assert!(game.run_command("tp @s ~ ~ ~2"));
    assert!(game.sleeping.is_none(), "the sleeper is out of bed");
    // The client's own counter, which fades the overlay out from 100
    // (`ClientboundAnimatePacket`'s 0 -> `stopSleepInBed(false, false)`).
    assert_eq!(game.sleep_timer, 100, "the fade-out starts, as on any wake");
    // `AbstractBedBlock.findStandUpPosition` puts them west of the bed, at
    // (3.5, 100.0, 4.5), and `teleportSetPosition` reads their position after
    // the wake: the relative axes move from there, so z is the bed's + 2.
    assert_eq!(
        game.player.as_ref().unwrap().position,
        Vec3::new(3.5, 100.0, 6.5)
    );
}
