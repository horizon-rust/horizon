//! The chat a command sends beyond its source's own feedback: a team message
//! to the team (`TeamMsgCommand.sendMessage`), a whisper to its target
//! (`MsgCommand.sendMessage`) and an emote from a source that is not a player
//! to every player (`EmoteCommands` through `PlayerList.broadcastChatMessage`).
use crate::game::Game;
use crate::game::lan_actions::GuestCommand;
use crate::game::test_pos_guest_tests::{GUEST, game};
use horizon_client::lan::{Joined, PlayerSender, guest::Guest};
use horizon_protocol::packets::configuration::ClientInformation;
use horizon_protocol::packets::play::Clientbound as C;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

/// The name the window's own player is on the scoreboard as (`PLAYER_NAME`).
const HOST: &str = "Player";

/// The chat type a name has in the sync the guest was sent.
fn chat_type(game: &Game<'_>, name: &str) -> i32 {
    game.lan
        .as_ref()
        .unwrap()
        .hub
        .registry_id("minecraft:chat_type", &format!("minecraft:{name}"))
        .expect("the chat types are in the sync")
}

/// Every chat packet the guest has been sent since the last call, as (chat
/// type, the sender's name, the chat type's target name, the message).
fn chats(
    rx: &mpsc::Receiver<horizon_client::lan::Out>,
) -> Vec<(i32, String, Option<String>, String)> {
    let language = horizon_client::lang::current().unwrap_or_default();
    let lang = language.map();
    let plain = |tag: &horizon_world::nbt::Tag| horizon_protocol::text::plain(tag, lang);
    let mut bodies = Vec::new();
    while let Ok(out) = rx.try_recv() {
        match out {
            horizon_client::lan::Out::Packet(body) => bodies.push(body),
            horizon_client::lan::Out::Packets(batch) => bodies.extend(batch),
            horizon_client::lan::Out::Disconnect(_) => {}
        }
    }
    bodies
        .into_iter()
        .filter_map(|body| {
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            match C::decode(id, reader.rest()).unwrap() {
                C::DisguisedChat(p) => Some((
                    p.chat_type.chat_type,
                    plain(&p.chat_type.name),
                    p.chat_type.target.as_ref().map(plain),
                    plain(&p.message),
                )),
                _ => None,
            }
        })
        .collect()
}

/// What a command said to the window's own player.
fn said(game: &mut Game<'_>, command: &str) -> Vec<String> {
    game.chat_capture = Some(Vec::new());
    assert!(game.run_command(command), "{command}");
    game.chat_capture
        .take()
        .unwrap_or_default()
        .iter()
        .map(horizon_client::text::Text::plain)
        .collect()
}

/// A second placed guest (conn 7, "Second") on the host of `game()`, and what
/// it is sent.
fn second_guest(game: &mut Game<'_>) -> mpsc::Receiver<horizon_client::lan::Out> {
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 7,
            uuid: 7,
            name: "Second".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
            configured: Default::default(),
        },
        7,
        [26.5, 101.0, 9.5],
        1,
    );
    guest.placed = true;
    guest.loaded = true;
    game.lan.as_mut().unwrap().hub.guests.push(guest);
    rx
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_team_message_from_a_guest_reaches_the_guest_and_the_team() {
    let (mut game, rx) = game();
    let rx2 = second_guest(&mut game);
    // The window's own player is a guest of the integrated server's hub, which
    // has no player of its own.
    game.player = None;
    let incoming = chat_type(&game, "team_msg_command_incoming");
    assert!(game.run_command("scoreboard objectives add count dummy"));
    assert!(game.run_command("team add red"));
    assert!(game.run_command(&format!("team join red {GUEST}")));
    assert!(game.run_command("team join red Second"));
    let _ = chats(&rx);
    let _ = chats(&rx2);
    // The sender's own line is its turn's output, the team's is its chat, and
    // vanilla's result is the receiver count.
    let (ok, lines) = game
        .run_guest_command_told(GuestCommand {
            conn: 2,
            command: "execute store result score #count count run teammsg hello team".into(),
            permission: 4,
            integrated: false,
            quiet: false,
        })
        .unwrap();
    assert!(ok);
    assert_eq!(
        lines
            .iter()
            .map(horizon_client::text::Text::plain)
            .collect::<Vec<_>>(),
        [format!("-> [red] <{GUEST}> hello team")]
    );
    assert_eq!(
        chats(&rx2),
        [(
            incoming,
            GUEST.to_owned(),
            Some("[red]".into()),
            "hello team".into()
        )]
    );
    assert_eq!(game.score.board.score("#count", "count").unwrap().value, 2);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_team_message_reaches_every_member_and_the_sender() {
    let (mut game, rx) = game();
    let incoming = chat_type(&game, "team_msg_command_incoming");
    assert!(game.run_command("team add red"));
    assert!(game.run_command(&format!("team join red {HOST}")));
    assert!(game.run_command(&format!("team join red {GUEST}")));
    let _ = chats(&rx);
    // The sender's own line, then the team's.
    assert_eq!(
        said(&mut game, "teammsg hello team"),
        ["-> [red] <Player> hello team"]
    );
    // The guest is on the team, so the incoming line is theirs, bound to the
    // team's display name as the chat type's target.
    assert_eq!(
        chats(&rx),
        [(
            incoming,
            HOST.to_owned(),
            Some("[red]".into()),
            "hello team".into()
        )]
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_whisper_names_its_target_on_the_sender_and_reaches_the_target() {
    let (mut game, rx) = game();
    let incoming = chat_type(&game, "msg_command_incoming");
    let _ = chats(&rx);
    // The outgoing line names the target, and the incoming one goes to it.
    assert_eq!(
        said(&mut game, &format!("msg {GUEST} hi")),
        [format!("You whisper to {GUEST}: hi")]
    );
    assert_eq!(
        chats(&rx),
        [(incoming, HOST.to_owned(), None, "hi".to_owned())]
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_emote_from_a_source_that_is_not_a_player_is_broadcast() {
    let (mut game, rx) = game();
    let emote = chat_type(&game, "emote_command");
    assert!(game.run_command("summon pig 4 100 6 {NoAI:1b}"));
    let _ = chats(&rx);
    // `execute as` makes the pig the command's source, as a command block
    // would be: the line still reaches the players, and the host's chat.
    assert!(game.run_command("execute as @e[type=minecraft:pig,limit=1] run me squeak"));
    assert_eq!(game.ui.chat.recent_lines(1), ["* Pig squeak"]);
    assert_eq!(
        chats(&rx),
        [(emote, "Pig".to_owned(), None, "squeak".to_owned())]
    );
}
