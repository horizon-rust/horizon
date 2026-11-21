//! `TitleCommand.showTitle` resolves the component for each target
//! (`ResolutionContext.withEntityOverride`), and `SelectorContents.resolve`
//! joins the matched entities with the object's own `separator`, else
//! `ComponentUtils.DEFAULT_SEPARATOR` (", " in gray).
use super::Game;
use crate::game::chat::PLAYER_NAME;
use horizon_client::text::{GRAY, Text};
use horizon_protocol::packets::play::Clientbound;

/// A server's game with the local player standing at the origin.
fn game() -> Game<'static> {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(
            game.motion,
            horizon_core::geometry::Vec3::new(0.5, 200.0, 0.5),
        )
        .unwrap(),
    );
    game
}

/// The actionbar and chat texts sent to a guest, as plain text.
fn guest_texts(
    receiver: &std::sync::mpsc::Receiver<horizon_client::lan::Out>,
) -> (Vec<String>, Vec<String>) {
    let plain = |tag: &horizon_world::nbt::Tag| {
        let json = horizon_client::scoreboard::tag_to_json(tag);
        horizon_client::component::to_text(&json, &mut |s| Text::literal(s)).plain()
    };
    let (mut actionbars, mut chats) = (Vec::new(), Vec::new());
    for sent in receiver.try_iter() {
        let bodies = match sent {
            horizon_client::lan::Out::Packet(p) => vec![p],
            horizon_client::lan::Out::Packets(p) => p,
            horizon_client::lan::Out::Disconnect(_) => continue,
        };
        for body in bodies {
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let Ok(id) = reader.varint() else { continue };
            match Clientbound::decode(id, reader.rest()) {
                Ok(Clientbound::SetActionBarText(p)) => actionbars.push(plain(&p.text)),
                Ok(Clientbound::SystemChat(p)) => chats.push(plain(&p.content)),
                _ => {}
            }
        }
    }
    (actionbars, chats)
}

#[test]
#[ignore = "daily"]
fn a_selector_joins_with_its_own_separator_and_a_gray_comma_space_by_default() {
    let mut game = game();
    for x in [0.5, 2.5, 4.5] {
        assert!(
            game.run_command(&format!("summon minecraft:pig {x} 200.0 0.5 {{NoAI:1b}}")),
            "summon at {x}"
        );
    }
    assert!(game.run_command(
        r#"title @s actionbar {"text":"","extra":[{"selector":"@e[type=minecraft:pig]","separator":" | "}]}"#
    ));
    let overlay = game.titles.overlay.clone().expect("the actionbar");
    assert_eq!(overlay.plain(), "Pig | Pig | Pig");
    assert!(
        overlay.spans.iter().any(|s| s.text == " | "),
        "{:?}",
        overlay.spans
    );
    // ComponentUtils.DEFAULT_SEPARATOR.
    assert!(game.run_command(r#"title @s actionbar {"selector":"@e[type=minecraft:pig]"}"#));
    let overlay = game.titles.overlay.clone().expect("the actionbar");
    assert_eq!(overlay.plain(), "Pig, Pig, Pig");
    let separator = overlay
        .spans
        .iter()
        .find(|s| s.text == ", ")
        .expect("a separator span");
    assert_eq!(separator.style.color, Some(GRAY));
    // ComponentSerialization's FuzzyCodec: a `text` beside a `selector` is the
    // component's contents, the selector is ignored.
    assert!(
        game.run_command(r#"title @s actionbar {"text":"x","selector":"@e[type=minecraft:pig]"}"#)
    );
    assert_eq!(
        game.titles.overlay.as_ref().map(Text::plain).as_deref(),
        Some("x")
    );
    // ComponentSerialization.bootstrap puts `score` before `selector`.
    for line in [
        "scoreboard objectives add pts dummy",
        "scoreboard players set Holder pts 5",
        r#"title @s actionbar {"score":{"name":"Holder","objective":"pts"},"selector":"@e[type=minecraft:pig]"}"#,
    ] {
        assert!(game.run_command(line), "{line}");
    }
    assert_eq!(
        game.titles.overlay.as_ref().map(Text::plain).as_deref(),
        Some("5")
    );
}

#[test]
#[ignore = "daily"]
fn a_title_score_wildcard_is_the_target_it_is_sent_to() {
    use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use horizon_protocol::packets::configuration::ClientInformation;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    let mut game = game();
    game.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "title-score".into(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: crate::game::Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
        },
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 2,
            uuid: 114,
            name: "TitleGuest".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
            configured: Default::default(),
        },
        1002,
        [2.5, 200.0, 0.5],
        2,
    );
    guest.placed = true;
    guest.loaded = true;
    guest.permission = 2;
    game.lan.as_mut().unwrap().hub.guests.push(guest);
    for line in [
        "scoreboard objectives add pts dummy",
        &format!("scoreboard players set {PLAYER_NAME} pts 3"),
        "scoreboard players set TitleGuest pts 7",
    ] {
        assert!(game.run_command(line), "{line}");
    }
    let score = r#"{"text":"","extra":[{"score":{"name":"*","objective":"pts"}}]}"#;
    assert!(game.run_command(&format!("title @a actionbar {score}")));
    assert!(game.run_command(&format!("tellraw @a {score}")));
    for g in &game.lan.as_ref().unwrap().hub.guests {
        g.sender.flush();
    }
    assert_eq!(
        game.titles.overlay.as_ref().map(Text::plain).as_deref(),
        Some("3"),
        "the local player's own score"
    );
    let (actionbars, chats) = guest_texts(&rx);
    assert_eq!(actionbars, vec!["7".to_owned()]);
    assert_eq!(chats, vec!["7".to_owned()]);
}
