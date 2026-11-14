//! A message command's text is what `MessageArgument.resolveComponent` makes of it: with
//! `COMMANDS_ENTITY_SELECTORS` (level 2 and up) its selectors become the names of the players
//! around the *sender*, and a plain player keeps them literal.
use super::*;
use horizon_client::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
use horizon_protocol::packets::{configuration::ClientInformation, play::Clientbound};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const ALICE: u32 = 2;
const BOB: u32 = 7;

/// A server with an operator at the origin and a plain player eight blocks away.
fn server() -> (Game<'static>, mpsc::Receiver<Out>, mpsc::Receiver<Out>) {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "test-lan-chat".into(), 4)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 4,
            integrated: false,
            levels: Default::default(),
        },
    )
    .unwrap();
    let mut receivers = Vec::new();
    for (conn, name, permission, position) in [
        (ALICE, "Alice", 2, [0.5, 100.0, 0.5]),
        (BOB, "Bob", 0, [0.5, 100.0, 8.5]),
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
            (1_000 + conn) as i32,
            position,
            2,
        );
        guest.placed = true;
        guest.loaded = true;
        guest.permission = permission;
        guest.dimension = "minecraft:overworld".into();
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        receivers.push(rx);
    }
    let bob = receivers.pop().unwrap();
    let alice = receivers.pop().unwrap();
    (game, alice, bob)
}

/// `conn`'s command packet, then the commands it left for the hub.
fn run(game: &mut Game<'_>, conn: u32, command: &str) {
    let mut host = game.lan.take().expect("a hub");
    game.guest_chat_command(&mut host, conn, command);
    game.lan = Some(host);
    game.run_guest_commands();
    for guest in &game.lan.as_ref().expect("the hub is back").hub.guests {
        guest.sender.flush();
    }
}

/// The chat components a client was shown (an unsigned command packet's
/// chat: PlayerChatMessage.system).
fn shown_json(rx: &mpsc::Receiver<Out>) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for sent in rx.try_iter() {
        let bodies = match sent {
            Out::Packet(p) => vec![p],
            Out::Packets(p) => p,
            Out::Disconnect(_) => continue,
        };
        for body in bodies {
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let Ok(id) = reader.varint() else {
                continue;
            };
            let Ok(packet) = Clientbound::decode(id, reader.rest()) else {
                continue;
            };
            let Clientbound::DisguisedChat(p) = packet else {
                continue;
            };
            out.push(horizon_client::scoreboard::tag_to_json(&p.message));
        }
    }
    out
}

/// The chat lines a client was shown, as the plain text it reads.
fn shown(rx: &mpsc::Receiver<Out>) -> Vec<String> {
    shown_json(rx)
        .iter()
        .map(|json| horizon_client::scoreboard::comp_text(json).plain())
        .collect()
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_message_command_resolves_selectors_only_for_a_source_that_may_use_them() {
    let (mut game, alice, bob) = server();
    // An operator's `/say`, `/me` and `/msg`: `@p` is the sender himself.
    run(&mut game, ALICE, "say nearest is @p");
    let heard = shown_json(&alice);
    assert_eq!(
        horizon_client::scoreboard::comp_text(&heard[0]).plain(),
        "nearest is Alice"
    );
    // MessageArgument.toComponent: the literal before the selector, then the
    // selector's component (not an empty literal before it).
    assert_eq!(heard[0]["text"].as_str(), Some("nearest is "));
    assert_eq!(heard[0]["extra"][0]["text"].as_str(), Some("Alice"));
    assert_eq!(shown(&bob), ["nearest is Alice"], "everyone hears it");
    run(&mut game, ALICE, "me waves at @p");
    assert_eq!(shown(&alice), ["waves at Alice"]);
    assert_eq!(shown(&bob), ["waves at Alice"]);
    run(&mut game, ALICE, "msg Bob hi @p");
    assert_eq!(shown(&alice), ["hi Alice"], "the outgoing line");
    assert_eq!(shown(&bob), ["hi Alice"], "the incoming line");
    // Every player is in reach, in join order (`EntitySelector.joinNames`).
    run(&mut game, ALICE, "say all @a");
    assert_eq!(shown(&alice), ["all Alice, Bob"]);
    // The selectors are read from where the sender stands: Bob is eight blocks
    // away, and only Alice is within four of Alice.
    run(&mut game, ALICE, "say close @a[distance=..4]");
    assert_eq!(shown(&alice), ["close Alice"]);
    // A plain player has no COMMANDS_ENTITY_SELECTORS: the text stays literal.
    run(&mut game, BOB, "me waves at @p");
    assert_eq!(shown(&alice), ["waves at @p"]);
}
