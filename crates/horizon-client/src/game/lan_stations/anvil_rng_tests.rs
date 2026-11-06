use super::*;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::{chunk::Chunk, lan::{Joined, PlayerSender, NetConfig, guest::Guest, hub::HubConfig}, motion::PlayerMotion};
use horizon_core::{geometry::Vec3, random::LegacyRandom};
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn anvil_wear_draws_only_from_the_acting_player() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(root.clone(), None, &root, &settings, std::time::Instant::now(), false, true, &Progress::new(), &mut |_, _| unreachable!()).unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.headless = true;
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(&loaded.resources, loaded.pack, loaded.gui.unwrap(), options, settings, Audio::silent(&root), None).unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    // No listener, announcements, sockets, or server loop: only the guest swap.
    game.lan_host(crate::game::lan::LanSettings { port: 0, guest_commands: false, force_game_mode: false },
        NetConfig { listen: false, announce: false, ..NetConfig::loopback(0, "anvil-test".into(), 2) },
        HubConfig { default_game_mode: 0, guest_commands: false, force_game_mode: false, hardcore: false, view_distance: 2, simulation_distance: 2, dimension: "minecraft:overworld".into(), allow_flight: false, seed: 0, max_players: 2, integrated: false, levels: Default::default() }).unwrap();
    let mut host = game.lan.take().unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut guest = Guest::new(Joined { conn: 2, uuid: 330, name: "AnvilGuest".into(), properties: Vec::new(), info: ClientInformation::new(2), brand: "test".into(), sender: PlayerSender::new(tx, Arc::default()), latency: Arc::new(AtomicI32::new(0)), address: "127.0.0.1:25565".parse().unwrap(), expected: false }, 2, [0.5, 64.0, 0.5], 2);
    guest.dimension = "minecraft:overworld".into();
    guest.placed = true;
    guest.loaded = true;
    host.hub.guests.push(guest);
    game.as_guest(&mut host, 2, |_| ()).unwrap();
    assert_ne!(game.player.as_ref().unwrap().random.clone().next_int(), host.hub.guests[0].motion.as_ref().unwrap().random.clone().next_int(), "co-located players own distinct initialized streams");
    let mut trace = Vec::new();
    for guest_turn in [false, true] {
        for creative in [false, true] {
            for name in ["minecraft:anvil", "minecraft:chipped_anvil", "minecraft:damaged_anvil", "minecraft:stone"] {
                for seed in [0_i64, 4096, 99999] {
                    let pos = [3, 64, 3];
                    let mut chunk = Chunk::empty(0, 0);
                    chunk.set(pos[0], pos[1], pos[2], game.pack.assets.registry.default_state(name).unwrap().0);
                    game.world.chunks.insert((0, 0), Arc::new(chunk));
                    game.world.random = LegacyRandom::new(98765);
                    game.player.as_mut().unwrap().random = LegacyRandom::new(seed);
                    host.hub.guests[0].motion.as_mut().unwrap().random = LegacyRandom::new(seed + 7);
                    if guest_turn {
                        let mut menu = Box::new(Menu::new(Kind::Anvil));
                        menu.combined = true;
                        menu.creative = creative;
                        let mut station = GuestStation { window: 1, state: 0, kind: StationKind::Anvil, pos, menu, sent_data: Vec::new(), last_sound: 0 };
                        game.lan_station_combined(&mut host, 2, &mut station, None);
                    } else {
                        game.anvil_used(pos, creative);
                    }
                    let result = game.res.facts().get(game.world.block(pos[0], pos[1], pos[2])).name.clone();
                    trace.push(format!("{} {} {name} {seed} {result} {} {} {}", u8::from(guest_turn), u8::from(creative), game.player.as_mut().unwrap().random.next_int(), host.hub.guests[0].motion.as_mut().unwrap().random.next_int(), game.world.random.next_int()));
                }
            }
        }
    }
    if let Some(path) = std::env::var_os("HORIZON_ANVIL_RNG_ORACLE") {
        assert_eq!(trace.join("\n"), std::fs::read_to_string(path).unwrap().trim());
    } else {
        panic!("run with tools/test_anvil_rng_parity.py to compare native outcomes");
    }
}
