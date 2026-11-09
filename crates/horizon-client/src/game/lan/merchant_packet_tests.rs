//! Vanilla notifyTrade changes the live offer while its frozen owner retains
//! the last outbound offer packet. No sockets, server loop or world generation.

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn merchant_take_keeps_packet_snapshot_until_price_refresh() {
    use crate::game::{Audio, Options, Progress, Settings, load_resources};
    use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest};
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

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
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let res = Box::leak(Box::new(loaded.resources));
    let mut game = super::Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    game.world_root =
        std::env::temp_dir().join(format!("horizon-merchant-packet-{}", std::process::id()));
    game.build_registries();
    game.set_allow_commands(true);

    let mut sync = horizon_client::lan::data::SyncData::empty();
    for (registry, names) in [
        ("minecraft:menu", vec!["minecraft:merchant".to_owned()]),
        (
            "minecraft:item",
            vec![
                "minecraft:air".to_owned(),
                "minecraft:emerald".to_owned(),
                "minecraft:glass".to_owned(),
            ],
        ),
    ] {
        sync.builtin.registries.insert(
            registry.to_owned(),
            horizon_protocol::registry::Ids::from_names(names),
        );
    }
    let sync = Arc::new(sync);
    let net = NetConfig {
        listen: false,
        announce: false,
        ..NetConfig::loopback(0, String::new(), 2)
    };
    let lan = horizon_client::lan::Lan::start_with(sync.clone(), net).unwrap();
    let encoder = horizon_client::lan::chunks::ChunkEncoder::new(
        [] as [(Arc<str>, bool); 0],
        &horizon_protocol::registry::Builtin::default(),
        &[],
        &[],
    );
    let config = horizon_client::lan::hub::HubConfig {
        default_game_mode: 0,
        guest_commands: false,
        force_game_mode: false,
        hardcore: false,
        view_distance: 10,
        simulation_distance: 10,
        dimension: "minecraft:overworld".into(),
        allow_flight: false,
        seed: 0,
        max_players: 20,
        integrated: true,
        levels: Default::default(),
    };
    let mut hub = horizon_client::lan::hub::Hub::new(lan, sync, encoder, config);
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 1,
            uuid: 1,
            name: "Test".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: String::new(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:1".parse().unwrap(),
            expected: false,
            configured: Default::default(),
        },
        1,
        [0.5, 4.0, 0.5],
        8,
    );
    guest.placed = true;
    guest.dimension = "minecraft:overworld".into();
    guest.inventory[0] = Some(horizon_client::items::ItemStack::new(
        "minecraft:emerald",
        8,
    ));
    hub.guests.push(guest);
    let mut host = super::LanHost {
        hub,
        players_told_left: false,
        settings: super::LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        },
        play: Default::default(),
        broken: Default::default(),
        mob_state: Default::default(),
        poses: Default::default(),
        mob_players: Default::default(),
        owner: None,
        waypoints: Default::default(),
        waypoint_dead: Default::default(),
        border_sent: None,
        commands: Default::default(),
        quiet: Default::default(),
        far: None,
        native_commands: None,
        native_usages: Vec::new(),
        sleep_ignored: Default::default(),
        states: Default::default(),
        states_next_entity: None,
        idle_states: Default::default(),
    };

    use crate::game::trading::MerchantState;
    use horizon_client::{
        items::ItemStack,
        merchant::{ItemCost, MerchantOffer, Session},
    };
    let offer = MerchantOffer {
        base_cost_a: ItemCost {
            stack: ItemStack::new("minecraft:emerald", 4),
        },
        cost_b: None,
        result: ItemStack::new("minecraft:glass", 3),
        uses: 0,
        max_uses: 100,
        reward_exp: false,
        special_price_diff: 0,
        demand: 0,
        price_multiplier: 0.0,
        xp: 1,
    };
    let mut session = Session::new(vec![offer], true);
    session.xp = 20;
    session.level = 2;
    let kind = game
        .res
        .mob_types()
        .get("minecraft:villager")
        .unwrap()
        .clone();
    let mut mob = horizon_client::mobs::Mob::new(
        77,
        kind,
        horizon_core::geometry::Vec3::new(0.5, 4.0, 2.5),
        0.0,
    );
    mob.extra.trade = Some(Box::new(horizon_client::merchant::Trader {
        xp: 20,
        offers: session.offers.clone(),
        ..Default::default()
    }));
    game.mobs.push(mob);
    game.lan_open_merchant(
        &mut host,
        1,
        MerchantState {
            session: Box::new(session),
            scroll: 0,
            dragging: false,
            focused: None,
            mob: Some(77),
            name: Some("Packet timing".into()),
        },
    );
    fn packets(
        rx: &std::sync::mpsc::Receiver<horizon_client::lan::Out>,
    ) -> (
        Vec<horizon_protocol::packets::play::MerchantOffers>,
        Vec<horizon_protocol::packets::play::ContainerSetContent>,
    ) {
        let mut offers = Vec::new();
        let mut contents = Vec::new();
        for out in rx.try_iter() {
            let horizon_client::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            match horizon_protocol::packets::play::Clientbound::decode(id, reader.rest()).unwrap() {
                horizon_protocol::packets::play::Clientbound::MerchantOffers(packet) => {
                    offers.push(packet)
                }
                horizon_protocol::packets::play::Clientbound::ContainerSetContent(packet) => {
                    contents.push(packet)
                }
                _ => {}
            }
        }
        (offers, contents)
    }
    let initial = packets(&rx).0;
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].offers[0].uses, 0);
    game.lan_select_trade(&mut host, 1, 0);
    assert!(
        packets(&rx).0.is_empty(),
        "selecting does not resend offers"
    );
    let window = initial[0].container;
    // Real ContainerClick on the merchant result slot, through the same
    // native path as the parity harness. Vanilla notifyTrade increments
    // uses/XP but sends no offers until the later reputation AI phase.
    let click = horizon_protocol::packets::play::ContainerClick {
        container: window,
        state: 0,
        slot: 2,
        button: 0,
        input: 0,
        changed: Vec::new(),
        carried: None,
    };
    assert!(game.lan_merchant_click(&mut host, 1, &click));
    let state = &host.play[&1].merchant.as_ref().unwrap().merchant.session;
    assert_eq!(state.offers[0].uses, 1);
    assert_eq!(state.xp, 21);
    let (offers, contents) = packets(&rx);
    assert!(
        offers.is_empty(),
        "a take leaves the last offer packet untouched"
    );
    let content = contents.last().expect("take sends real inventory contents");
    assert_eq!(content.items[0].as_ref().unwrap().count, 4);
    assert_eq!(content.items[2].as_ref().unwrap().count, 3);
    assert_eq!(content.carried.as_ref().unwrap().count, 3);
    let second = horizon_protocol::packets::play::ContainerClick {
        state: content.state,
        ..click
    };
    assert!(game.lan_merchant_click(&mut host, 1, &second));
    let (offers, contents) = packets(&rx);
    assert!(offers.is_empty(), "the second take also keeps the snapshot");
    let content = contents.last().expect("second take sends contents");
    assert!(content.items[0].is_none());
    assert!(content.items[2].is_none());
    assert_eq!(content.carried.as_ref().unwrap().count, 6);
    let state = &host.play[&1].merchant.as_ref().unwrap().merchant.session;
    assert_eq!(state.offers[0].uses, 2);
    assert_eq!(state.xp, 22);
    // An actual price refresh sends the snapshot even if only uses/XP
    // changed, and even a repeated refresh must reach the client.
    game.lan_merchant_reputation(&mut host, 0);
    assert!(
        packets(&rx).0.is_empty(),
        "unrelated merchants do not refresh this menu"
    );
    game.lan_merchant_reputation(&mut host, 77);
    let refreshed = packets(&rx).0;
    assert_eq!(refreshed.len(), 1);
    assert_eq!(refreshed[0].offers[0].uses, 2);
    assert_eq!(refreshed[0].xp, 22);
    game.lan_merchant_reputation(&mut host, 77);
    assert_eq!(
        packets(&rx).0.len(),
        1,
        "price refresh sends even an unchanged offer"
    );
    let live = game.mobs[0].extra.trade.as_mut().unwrap();
    live.offers[0].uses = 0;
    live.offers[0].demand = 12;
    game.lan_merchant_refresh(&mut host, 77, None);
    let restocked = packets(&rx).0;
    assert_eq!(restocked.len(), 1);
    assert_eq!(restocked[0].offers[0].uses, 0);
    assert_eq!(restocked[0].offers[0].demand, 12);
    assert!(game.lan_merchant_closed(&mut host, 1, Some(window)));
    assert!(packets(&rx).0.is_empty(), "closing does not resend offers");
}
