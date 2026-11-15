//! `/defaultgamemode` reports the players it switched, and only a server that
//! forces its mode switches any (`DefaultGameModeCommands.setMode`,
//! `MinecraftServer.enforceGameTypeForPlayers`, `IntegratedServer.getForcedGameType`);
//! `/gamemode` from a source without a player fails (`GameModeCommand.setMode`,
//! `CommandSourceStack.getPlayerOrException`).
use super::super::*;
use super::{Flags, LoadPhase, Serving};
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
};
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const SURVIVAL: i32 = 0;
const CREATIVE: i32 = 1;
const GUEST: &str = "QN48Guest";

/// A host on a stone floor with two guests in creative (conn 2, permission 0,
/// and conn 3, permission 2: a game master, whom an integrated server spares).
/// `force` is the world's `forceGameMode`; `published` makes this the world
/// open to the LAN.
fn game(integrated: bool, force: bool, hardcore: bool, published: bool) -> Game<'static> {
    let mut game = super::super::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    for x in 0..2 {
        game.world
            .chunks
            .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
    }
    assert!(game.run_command("fill 0 99 0 31 99 15 stone"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 6.5)).unwrap());
    game.level.hardcore = hardcore;
    let net = NetConfig {
        announce: false,
        ..NetConfig::lan(0, "q-n48-test".into(), 3)
    };
    game.lan_host(
        super::super::lan::LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: force,
        },
        net,
        HubConfig {
            default_game_mode: CREATIVE,
            guest_commands: false,
            force_game_mode: force,
            hardcore,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 3,
            integrated,
            levels: Default::default(),
        },
    )
    .unwrap();
    if published {
        // `IntegratedServer.publishServer`: `getPort` answers, so `isPublished`.
        let (_sender, tasks) = mpsc::channel();
        let flags = Arc::new(Flags::default());
        flags.published.store(25565, Ordering::Relaxed);
        game.serving = Some(Serving {
            flags,
            tasks,
            owner: 1,
            pending: None,
            load: LoadPhase::default(),
        });
    }
    for (conn, name, permission) in [(2, GUEST, 0), (3, "QN48Operator", 2)] {
        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: conn as u128,
                name: name.into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
                configured: Default::default(),
            },
            conn as i32 + 1,
            [22.5, 101.0, 9.5],
            1,
        );
        guest.placed = true;
        guest.loaded = true;
        guest.game_mode = CREATIVE;
        guest.permission = permission;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
    }
    game
}

/// The guests' game modes, in join order.
fn modes(game: &Game<'_>) -> Vec<i32> {
    game.lan
        .as_ref()
        .unwrap()
        .hub
        .guests
        .iter()
        .map(|g| g.game_mode)
        .collect()
}

fn default_mode(game: &Game<'_>) -> i32 {
    game.lan.as_ref().unwrap().hub.config.default_game_mode
}

/// `/defaultgamemode <mode>` from the server's own source, and how many
/// players it reports it switched.
fn switch(game: &mut Game<'_>, mode: &str) -> i32 {
    let server = game.server_source();
    game.execute_line(&format!("defaultgamemode {mode}"), server)
        .unwrap()
}

/// `DefaultGameModeCommands.setMode` returns `enforceGameTypeForPlayers`'s
/// count: a dedicated server that forces its mode switches every player.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn default_gamemode_reports_the_players_it_switched() {
    let mut game = game(false, true, false, false);
    assert_eq!(switch(&mut game, "survival"), 2);
    assert_eq!(default_mode(&game), SURVIVAL);
    assert_eq!(modes(&game), [SURVIVAL, SURVIVAL]);
    // Nobody left to switch: `ServerPlayer.setGameMode` answers false.
    assert_eq!(switch(&mut game, "survival"), 0);
}

/// `IntegratedServer.getForcedGameType` is null until the world is published:
/// the default mode changes, no player does, and the command reports 0.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn default_gamemode_leaves_players_alone_until_the_world_is_published() {
    let mut game = game(true, true, false, false);
    assert_eq!(switch(&mut game, "survival"), 0);
    assert_eq!(default_mode(&game), SURVIVAL);
    assert_eq!(modes(&game), [CREATIVE, CREATIVE]);
}

/// Published, a forcing integrated server takes the players who may not change
/// their own mode (`IntegratedServer.updateGameModeBasedOnPermission`), and
/// reports only them.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn default_gamemode_forces_the_guests_of_a_published_world() {
    let mut game = game(true, true, false, true);
    assert_eq!(switch(&mut game, "survival"), 1);
    assert_eq!(modes(&game), [SURVIVAL, CREATIVE]);
}

/// `IntegratedServer.getForcedGameType` is null in a hardcore world, even
/// published: the mode changes and nobody is switched.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn default_gamemode_does_not_force_a_hardcore_world() {
    let mut game = game(true, true, true, true);
    assert_eq!(switch(&mut game, "survival"), 0);
    assert_eq!(modes(&game), [CREATIVE, CREATIVE]);
}

/// `GameModeCommand.setMode` with no target asks for the source's player
/// (`getPlayerOrException`): a source without one fails, and an explicit
/// target still works from it.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn gamemode_needs_a_player_source() {
    let mut game = game(false, false, false, false);
    let server = game.server_source();
    let refused = game.execute_line("gamemode survival", server).unwrap_err();
    assert!(
        format!("{refused:?}").contains("permissions.requires.player"),
        "{refused:?}"
    );
    assert_eq!(modes(&game), [CREATIVE, CREATIVE]);
    let target = format!("gamemode survival {GUEST}");
    assert_eq!(game.execute_line(&target, server).unwrap(), 1);
    assert_eq!(modes(&game), [SURVIVAL, CREATIVE]);
}

/// The player who runs `/defaultgamemode` is one of the players it forces
/// (`MinecraftServer.enforceGameTypeForPlayers` walks every player): when
/// their command's turn ends they keep the new mode.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn default_gamemode_forces_the_player_who_ran_it() {
    let mut game = game(false, true, false, false);
    let (ok, _) = game
        .run_guest_command_told(super::super::lan_actions::GuestCommand {
            conn: 3,
            command: "defaultgamemode survival".into(),
            permission: 2,
            integrated: false,
            quiet: false,
        })
        .expect("the operator is online");
    assert!(ok);
    assert_eq!(modes(&game), [SURVIVAL, SURVIVAL]);
}
