//! `player.send_packets` against real LAN guests, with a literal test for each refusal.

use super::native_entity_refs::tests::with_client_game;
use super::native_guest_turn_tests::with_guests;
use horizon_protocol::packets::{Empty, play::Clientbound as C};
use mod_api::{
    ClientboundPlayPacket as Packet, EntityRef, Identifier, PayloadId, PlayerRef,
    SendPacketsRefusal as Refusal, WorldEpoch,
};

/// Routes a guest's packets to a channel the test reads.
fn listen(
    game: &mut super::Game<'_>,
    conn: u32,
) -> std::sync::mpsc::Receiver<horizon_client::lan::Out> {
    let (tx, rx) = std::sync::mpsc::channel();
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_connection(conn, |g| {
            g.sender = horizon_client::lan::PlayerSender::new(tx, Default::default());
        });
    rx
}

fn received(rx: &std::sync::mpsc::Receiver<horizon_client::lan::Out>) -> Vec<C> {
    let mut out = Vec::new();
    while let Ok(horizon_client::lan::Out::Packet(body)) = rx.try_recv() {
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        out.extend(C::decode(id, r.rest()));
    }
    out
}

/// The envelope of a packet, as a mod would build it from its own encoding.
fn envelope(name: &str, packet: C) -> Packet {
    let bytes = packet.encode();
    let mut r = horizon_protocol::buf::Reader::new(&bytes);
    r.varint().unwrap();
    Packet::new(Identifier::parse(name).unwrap(), r.rest().to_vec())
}

fn time(ticks: i64) -> Packet {
    envelope(
        "minecraft:set_time",
        C::SetTime(horizon_protocol::packets::play::SetTime {
            game_time: ticks,
            clocks: Vec::new(),
        }),
    )
}

fn time_of(packet: &C) -> i64 {
    match packet {
        C::SetTime(t) => t.game_time,
        other => panic!("{other:?}"),
    }
}

/// The packets reach that player alone, in order, and `bundle` puts them between delimiters
/// (a single packet, or a list sent without it, needs none).
#[test]
#[ignore = "daily"]
fn packets_go_to_one_player_in_order_and_bundled_on_request() {
    with_guests(|game| {
        let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
        let (rx_a, rx_b) = (listen(game, 12), listen(game, 13));
        assert_eq!(b.send_packets(game, &[time(1), time(2)], false), Ok(()));
        assert_eq!(b.send_packets(game, &[time(3), time(4)], true), Ok(()));
        assert_eq!(b.send_packets(game, &[time(5)], true), Ok(()));
        assert_eq!(b.send_packets(game, &[], true), Ok(()));
        assert!(received(&rx_a).is_empty(), "the other player hears nothing");
        let sent = received(&rx_b);
        let delimiter = |p: &C| matches!(p, C::BundleDelimiter(Empty));
        let shape: Vec<_> = sent
            .iter()
            .map(|p| (!delimiter(p)).then(|| time_of(p)))
            .collect();
        assert_eq!(
            shape,
            [Some(1), Some(2), None, Some(3), Some(4), None, Some(5)]
        );
        let _ = a;
    });
}

/// Each refusal is its own name and sends none of the list, the good packets before it included.
#[test]
#[ignore = "daily"]
fn each_send_refusal_has_its_own_name() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        let rx = listen(game, 13);
        let unknown = Packet::new(
            Identifier::parse("minecraft:no_such_packet").unwrap(),
            vec![],
        );
        assert_eq!(
            b.send_packets(game, &[time(1), unknown.clone()], false),
            Err(Refusal::UnknownPacket {
                index: 1,
                id: unknown.id.clone()
            })
        );
        let foreign = Packet::new(Identifier::parse("other:set_time").unwrap(), vec![]);
        assert!(matches!(
            b.send_packets(game, &[foreign], false),
            Err(Refusal::UnknownPacket { index: 0, .. })
        ));
        // The name exists, but Horizon has no codec for its body. Opaque
        // `Clientbound::Unknown` is not a successfully validated packet.
        let unmodeled = Packet::new(
            Identifier::parse("minecraft:command_suggestions").unwrap(),
            vec![],
        );
        assert_eq!(
            b.send_packets(game, &[time(1), unmodeled.clone()], true),
            Err(Refusal::UnknownPacket {
                index: 1,
                id: unmodeled.id,
            })
        );
        let mut cut = time(1);
        cut.body.pop();
        assert_eq!(
            b.send_packets(game, &[cut], false),
            Err(Refusal::InvalidBody { index: 0 })
        );
        let mut long = time(1);
        long.body.push(0);
        assert_eq!(
            b.send_packets(game, &[time(1), long], false),
            Err(Refusal::InvalidBody { index: 1 })
        );
        let delimiter = envelope("minecraft:bundle_delimiter", C::BundleDelimiter(Empty));
        assert_eq!(
            b.send_packets(game, &[delimiter], false),
            Err(Refusal::ServerOnly { index: 0 })
        );
        let configuration = Packet::new(
            Identifier::parse("minecraft:start_configuration").unwrap(),
            vec![],
        );
        assert_eq!(
            b.send_packets(game, &[time(1), configuration], true),
            Err(Refusal::ServerOnly { index: 1 })
        );
        let mut payload = envelope(
            "minecraft:custom_payload",
            C::CustomPayload(horizon_protocol::packets::custom_payload::CustomPayload {
                channel: "horizon:levels".into(),
                data: vec![1],
            }),
        );
        assert!(matches!(
            b.send_packets(game, &[payload.clone()], false),
            Err(Refusal::ProtectedPayload {
                index: 0,
                id: PayloadId(_)
            })
        ));
        payload.body.clear();
        assert_eq!(
            b.send_packets(game, &[payload], false),
            Err(Refusal::InvalidBody { index: 0 })
        );
        let many = vec![time(0); 4097];
        assert_eq!(
            b.send_packets(game, &many, true),
            Err(Refusal::BundleTooLong {
                len: 4097,
                limit: 4096
            })
        );
        assert_eq!(b.send_packets(game, &many[..4096], true), Ok(()));
        assert_eq!(received(&rx).len(), 4098, "only the full bundle went out");
        let entity = b.entity();
        let stale: PlayerRef = mod_api::__private::game::player_ref(
            EntityRef::new(WorldEpoch::new(entity.epoch().get() + 1), entity.id()),
            b.uuid(),
        );
        assert_eq!(
            stale.send_packets(game, &[time(1)], false),
            Err(Refusal::Stale)
        );
        game.next_entity = 100;
        let level = game.level(game.dim.id()).unwrap();
        let machine = game.fake_player(level, 1234, "machine").unwrap();
        assert_eq!(
            machine.send_packets(game, &[time(1)], false),
            Err(Refusal::NoConnection)
        );
    });
}

/// On a client the call answers `Client`.
#[test]
#[ignore = "daily"]
fn a_client_cannot_send_packets_to_a_player() {
    with_client_game(|game| {
        let ghost = mod_api::__private::game::player_ref(EntityRef::new(WorldEpoch::new(0), 1), 1);
        assert_eq!(
            ghost.send_packets(game, &[time(1)], false),
            Err(Refusal::Client)
        );
    });
}
