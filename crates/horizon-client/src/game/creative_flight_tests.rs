//! A rider's double jump toggles creative flight, a refused toggle re-sends the
//! server's own flying speed, and Ctrl-pick with `include_data` prints an
//! avatar's profile.
use crate::game::*;
use crate::mod_api::{self, Hooks, Host, PlayerToggleFlightEvent};
use horizon_client::{
    items::ItemStack,
    lan::{
        Joined, NetConfig, Out, PlayerSender,
        guest::{CREATIVE, Guest},
        hub::HubConfig,
    },
    mobs::Mob,
    motion::PlayerMotion,
    sprint::Keys,
};
use horizon_core::geometry::Vec3;
use horizon_protocol::{
    buf::Reader,
    connection::Pipe,
    packets::{
        configuration::ClientInformation,
        play::{self, Clientbound as C, Serverbound as S},
    },
};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

fn output(rx: &mpsc::Receiver<Out>) -> Vec<C> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(reason) => panic!("unexpected disconnect: {reason:?}"),
        })
        .map(|body| {
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            C::decode(id, reader.rest()).unwrap()
        })
        .collect()
}

fn guest(
    conn: u32,
    uuid: u128,
    name: &str,
    entity_id: i32,
    at: [f64; 3],
) -> (Guest, mpsc::Receiver<Out>) {
    let (tx, rx) = mpsc::channel();
    let mut g = Guest::new(
        Joined {
            conn,
            uuid,
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
        entity_id,
        at,
        2,
    );
    g.placed = true;
    g.loaded = true;
    (g, rx)
}

/// A server with a creative game-master picker (conn 1) and another player's
/// avatar (conn 2) beside it, both in the Overworld.
fn lan_game(name: &str) -> (Game<'static>, mpsc::Receiver<Out>) {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::loopback(0, name.into(), 2)
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
            max_players: 2,
            integrated: false,
            levels: Default::default(),
        },
    )
    .unwrap();
    let (mut picker, rx) = guest(1, 1, "Picker", 21, [0.5, 100.0, 0.5]);
    picker.game_mode = CREATIVE;
    picker.permission = 2;
    picker.flying_speed = 2.5;
    let (mut target, _) = guest(2, 2, "Target", 22, [1.5, 100.0, 0.5]);
    target.properties = vec![horizon_protocol::packets::login::Property {
        name: "textures".into(),
        value: "skin".into(),
        signature: None,
    }];
    let hub = &mut game.lan.as_mut().unwrap().hub;
    hub.guests.push(picker);
    hub.guests.push(target);
    (game, rx)
}

fn pass(game: &mut Game<'_>, conn: u32, packet: S) {
    let mut host = game.lan.take().unwrap();
    game.lan_guest_packet(&mut host, conn, packet);
    game.lan = Some(host);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_riding_player_double_jump_toggles_creative_flight() {
    let mut game = crate::game::native_machines::tests::server_game();
    let (client, _server) = Pipe::pair();
    game.connect_local(client, "FlightTest").unwrap();
    game.remote.as_mut().unwrap().joined = true;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());

    let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
    let mut horse = Mob::new(7, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    horse.extra.tame = true;
    game.mobs.clear();
    game.mobs.push(horse);
    game.riding = Some(7);
    game.remote.as_mut().unwrap().first_passenger = true;
    let player = game.player.as_mut().unwrap();
    player.may_fly = true;
    player.flying = false;

    let press = |game: &mut Game<'_>, jump: bool| {
        game.remote_ride(
            Keys {
                jump,
                ..Default::default()
            },
            0,
        )
        .unwrap()
    };
    // A saddled horse is a `jumpableVehicle`: a double tap toggles flight.
    game.mobs[0].set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
    assert!(press(&mut game, true));
    assert!(
        !game.player.as_ref().unwrap().flying,
        "the first press only opens the double-tap window"
    );
    assert!(press(&mut game, false));
    assert!(!game.player.as_ref().unwrap().flying);
    assert!(press(&mut game, true));
    assert!(
        game.player.as_ref().unwrap().flying,
        "the second press toggles creative flight while riding"
    );

    // An unsaddled horse cannot jump, so it is no `jumpableVehicle` and the
    // toggle stays away (`getVehicle() == null || jumpableVehicle() != null`).
    game.mobs[0].set_saddle(None);
    assert!(press(&mut game, false));
    assert!(press(&mut game, true));
    assert!(press(&mut game, false));
    assert!(press(&mut game, true));
    assert!(
        game.player.as_ref().unwrap().flying,
        "no toggle on a mount that cannot jump"
    );
}

/// A mod that refuses every flight toggle.
struct RefuseFlight;
impl Hooks for Arc<RefuseFlight> {
    fn player_toggle_flight(
        &self,
        _: &mut dyn Host,
        _: &mut PlayerToggleFlightEvent<'_>,
    ) -> mod_api::Flow {
        mod_api::Flow::Cancel
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_refused_flight_toggle_keeps_the_servers_flying_speed() {
    crate::mod_api::in_own_process(
        "game::remote_vehicles::creative_flight_tests::a_refused_flight_toggle_keeps_the_servers_flying_speed",
        a_refused_flight_toggle_keeps_the_servers_flying_speed_in_own_process,
    );
}

fn a_refused_flight_toggle_keeps_the_servers_flying_speed_in_own_process() {
    assert!(
        mod_api::MODS
            .set(Arc::from(vec![
                Box::new(Arc::new(RefuseFlight)) as Box<dyn Hooks>
            ]))
            .is_ok()
    );
    let (mut game, rx) = lan_game("flight-abilities-test");
    let mut host = game.lan.take().unwrap();
    game.lan_flight_toggled(&mut host, 1, true);
    game.lan = Some(host);
    let abilities = output(&rx)
        .into_iter()
        .find_map(|packet| match packet {
            C::PlayerAbilities(abilities) => Some(abilities),
            _ => None,
        })
        .expect("a refused toggle re-sends the guest's abilities");
    assert_eq!(
        abilities.flying_speed, 2.5,
        "the server's own flying speed, not the default 0.05"
    );
    assert_eq!(abilities.flags & 2, 0, "the refused flight stays off");
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn ctrl_pick_with_include_data_prints_another_players_profile() {
    let (mut game, rx) = lan_game("flight-profile-test");
    pass(
        &mut game,
        1,
        S::PickItemFromEntity(play::PickItemFromEntity {
            id: 22,
            include_data: false,
        }),
    );
    assert!(
        !output(&rx)
            .iter()
            .any(|packet| matches!(packet, C::SystemChat(_))),
        "no profile without include_data"
    );

    pass(
        &mut game,
        1,
        S::PickItemFromEntity(play::PickItemFromEntity {
            id: 22,
            include_data: true,
        }),
    );
    let chats: Vec<_> = output(&rx)
        .into_iter()
        .filter_map(|packet| match packet {
            C::SystemChat(chat) => Some(chat.content),
            _ => None,
        })
        .collect();
    assert_eq!(chats.len(), 1, "one profile report");
    let text = horizon_client::snbt::to_snbt(&chats[0]);
    assert!(
        text.contains("commands.fetchprofile.entity.success"),
        "{text}"
    );
    assert!(text.contains("Target"), "{text}");
    assert!(text.contains("textures"), "{text}");
}
