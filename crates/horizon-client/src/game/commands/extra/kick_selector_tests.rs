//! `/kick` kicks the players the selector it was given finds, and no one else
//! (`KickCommand.kickPlayers`, which takes `EntityArgument.getPlayers`): its
//! predicates, its limit and its selector kind all decide the set, an empty
//! set is `argument.entity.notfound.player`, and the owner is never kicked.
use super::*;
use crate::game::{Dim, lan_actions::GuestCommand, native_machines::tests::server_game};
use horizon_client::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

/// A server with Bob (conn 2, beside the spawn) and Carol (conn 7, a hundred
/// blocks away), each on its own connection.
fn server_with_two_guests() -> (Game<'static>, [mpsc::Receiver<Out>; 2]) {
    let mut game = server_game();
    game.build_registries();
    game.set_allow_commands(true);
    game.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "kick-test".into(), 4)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 4,
            integrated: false,
            levels: Default::default(),
        },
    )
    .unwrap();
    let mut receivers = Vec::new();
    for (conn, name, position) in [
        (2, "Bob", [2.5, 64.0, 2.5]),
        (7, "Carol", [102.5, 64.0, 2.5]),
    ] {
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn),
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
            conn as i32,
            position,
            0,
        );
        guest.placed = true;
        guest.loaded = true;
        guest.permission = 4;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        receivers.push(rx);
    }
    let receivers: [mpsc::Receiver<Out>; 2] = receivers.try_into().expect("two receivers");
    (game, receivers)
}

/// What a guest's connection was sent as a kick: `ClientboundDisconnectPacket`'s
/// reason, in order.
fn disconnects(receiver: &mpsc::Receiver<Out>) -> Vec<Tag> {
    receiver
        .try_iter()
        .filter_map(|out| match out {
            Out::Disconnect(reason) => Some(reason),
            _ => None,
        })
        .collect()
}

/// The reason a player is told: the given one, or vanilla's default.
fn kicked_reason(given: Option<&str>) -> Tag {
    match given {
        Some(reason) => Tag::String(reason.into()),
        None => Tag::compound([(
            "translate",
            Tag::String("multiplayer.disconnect.kicked".into()),
        )]),
    }
}

/// Runs `command` as the server's console (no player, at the world spawn) and
/// returns whether it ran and the lines the source was told.
fn run(game: &mut Game<'_>, command: &str) -> (bool, Vec<Text>) {
    // `Game::as_console_in_overworld`'s source: the server's, heard.
    let console = crate::game::commands::execute::Source {
        silent: false,
        ..game.server_source()
    };
    let outer = game.command_source.replace(console);
    // The console's source always takes its success feedback
    // (`CommandSource.acceptsSuccess`), whatever `send_command_feedback` says.
    let inform = game.console_broadcast.replace(false);
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    game.console_broadcast = inform;
    game.command_source = outer;
    (ok, game.chat_capture.take().unwrap_or_default())
}

/// The translation key of a failure line (`CommandSourceStack.sendFailure`
/// wraps the message in a red sibling).
fn failure_key(line: &Text) -> Option<String> {
    line.component()["extra"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|first| first["translate"].as_str())
        .map(str::to_owned)
}

/// `commands.kick.success`'s name: who the issuer was told was kicked. None
/// for any other line (a failure).
fn kicked_name(line: &Text) -> Option<String> {
    if line.translation_key() != Some("commands.kick.success") {
        return None;
    }
    line.component()["with"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|first| first["text"].as_str())
        .map(str::to_owned)
}

/// One `/kick` and what came of it.
struct Kicked {
    ok: bool,
    /// The names `commands.kick.success` named, in order.
    names: Vec<String>,
    /// Who was disconnected, by name.
    who: Vec<&'static str>,
    /// What the source was told, as the English it reads as.
    said: Vec<String>,
    /// The first line's translation key, for a failure.
    key: Option<String>,
    /// The reasons Bob's and Carol's clients were sent.
    to_bob: Vec<Tag>,
    to_carol: Vec<Tag>,
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn kick_kicks_the_selectors_players_only() {
    let (mut game, [bob, carol]) = server_with_two_guests();
    assert!(game.run_command("team add red"));
    assert!(game.run_command("team join red Bob"));
    // Every case starts with nobody kicked: the hub only takes the kicks on
    // its next tick, so the reasons stay readable.
    let case = |game: &mut Game<'_>, command: &str| {
        let _ = disconnects(&bob);
        let _ = disconnects(&carol);
        let (ok, said) = run(game, command);
        let names: Vec<String> = said.iter().filter_map(kicked_name).collect();
        let (to_bob, to_carol) = (disconnects(&bob), disconnects(&carol));
        let who: Vec<&'static str> = [("Bob", !to_bob.is_empty()), ("Carol", !to_carol.is_empty())]
            .iter()
            .filter_map(|(name, kicked)| kicked.then_some(*name))
            .collect();
        Kicked {
            ok,
            names,
            who,
            said: said.iter().map(Text::plain).collect(),
            key: said.first().and_then(failure_key),
            to_bob,
            to_carol,
        }
    };

    // A predicate decides who is kicked, not the selector's kind.
    let k = case(&mut game, "kick @a[distance=..5]");
    assert!(k.ok, "{:?}", k.said);
    assert_eq!(k.who, ["Bob"], "@a[distance=..5]: {:?}", k.said);
    assert_eq!(k.names, ["Bob"], "{:?}", k.said);
    // The limit too.
    let k = case(&mut game, "kick @a[limit=1]");
    assert!(k.ok, "{:?}", k.said);
    assert_eq!(k.who, ["Bob"], "@a[limit=1]: {:?}", k.said);
    // And `team=`.
    let k = case(&mut game, "kick @a[team=red]");
    assert!(k.ok, "{:?}", k.said);
    assert_eq!(k.who, ["Bob"], "@a[team=red]: {:?}", k.said);
    // `@p` is the nearest player to the source, `@r` one at random.
    let k = case(&mut game, "kick @p");
    assert!(k.ok, "{:?}", k.said);
    assert_eq!(k.who, ["Bob"], "@p: {:?}", k.said);
    let k = case(&mut game, "kick @r");
    assert!(k.ok, "{:?}", k.said);
    assert_eq!(k.who.len(), 1, "@r kicks one player: {:?}", k.said);
    assert_eq!(k.names, [k.who[0]], "{:?}", k.said);
    // `@a` is both, and each is told of it.
    let k = case(&mut game, "kick @a");
    assert!(k.ok, "{:?}", k.said);
    assert_eq!(k.who, ["Bob", "Carol"], "@a: {:?}", k.said);
    assert_eq!(k.names, ["Bob", "Carol"]);
    // A selector that finds nobody kicks nobody: vanilla's not-found error,
    // before the published check.
    for command in ["kick @a[team=blue]", "kick @s"] {
        let k = case(&mut game, command);
        assert!(!k.ok, "{command}: {:?}", k.said);
        assert!(k.names.is_empty(), "{command}: {:?}", k.said);
        assert!(k.who.is_empty(), "{command}: {:?}", k.said);
        assert!(k.to_bob.is_empty() && k.to_carol.is_empty(), "{command}");
        assert_eq!(
            k.key.as_deref(),
            Some("argument.entity.notfound.player"),
            "{command}: {:?}",
            k.said
        );
    }
    // A reason reaches the kicked players' clients as it was given.
    let k = case(&mut game, "kick @a[distance=..5] late again");
    assert_eq!(k.to_bob, [kicked_reason(Some("late again"))]);
    assert_eq!(k.to_carol, []);
    // No reason: vanilla's own.
    let k = case(&mut game, "kick @a[distance=..5]");
    assert_eq!(k.to_bob, [kicked_reason(None)]);
    assert_eq!(k.to_carol, []);
    // A name is a selector of one.
    let k = case(&mut game, "kick Carol");
    assert_eq!(k.to_bob, []);
    assert_eq!(k.to_carol, [kicked_reason(None)]);

    // The singleplayer owner is never kicked (`KickCommand.kickPlayers`'
    // `isSingleplayerOwner`), and a selector that found only them fails as if
    // it had found nobody.
    game.lan.as_mut().unwrap().hub.singleplayer_owner = Some(2u128); // Bob's world
    let k = case(&mut game, "kick @a");
    assert!(k.ok, "{:?}", k.said);
    assert_eq!(k.who, ["Carol"], "@a skips the owner: {:?}", k.said);
    assert_eq!(k.names, ["Carol"], "{:?}", k.said);
    let k = case(&mut game, "kick @a[team=red]");
    assert!(!k.ok, "{:?}", k.said);
    assert!(k.who.is_empty(), "only the owner matches: {:?}", k.said);
    assert_eq!(
        k.key.as_deref(),
        Some("commands.kick.owner.failed"),
        "{:?}",
        k.said
    );
    // The owner's own command runs as the guest whose turn it is.
    let _ = (disconnects(&bob), disconnects(&carol));
    let (ok, said) = guest_command(&mut game, 2, "kick @a");
    assert!(ok, "{said:?}");
    let names: Vec<String> = said.iter().filter_map(kicked_name).collect();
    assert_eq!(names, ["Carol"], "{said:?}");
    assert!(disconnects(&bob).is_empty(), "the owner was kicked");
    assert_eq!(disconnects(&carol), [kicked_reason(None)]);
}

/// A command typed by guest `conn`, as `LanHub` queues it: whether it ran and
/// the lines it was told.
fn guest_command(game: &mut Game<'_>, conn: u32, command: &str) -> (bool, Vec<Text>) {
    game.run_guest_command_told(GuestCommand {
        conn,
        command: command.into(),
        permission: 4,
        integrated: false,
        quiet: false,
    })
    .expect("the guest is online")
}
