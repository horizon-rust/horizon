//! A guest's take from a crafting result teaches the recipe (known, to be
//! highlighted, the add packet) and counts only the amount taken.
use super::*;
use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
use horizon_protocol::packets::{configuration::ClientInformation, play::ContainerClick};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

/// A server game with one placed guest, or None without prepared resources.
fn guest_server() -> Option<(Game<'static>, LanHost)> {
    std::env::var_os("HORIZON_TEST_RESOURCES")?;
    let mut game = crate::game::native_machines::tests::server_game();
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
            ..NetConfig::loopback(0, "crafting-teach-test".into(), 2)
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
    let (tx, _rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 2,
            uuid: 2,
            name: "CraftGuest".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
            configured: Default::default(),
        },
        2,
        [8.5, 200.0, 8.5],
        2,
    );
    guest.dimension = "minecraft:overworld".into();
    guest.placed = true;
    guest.loaded = true;
    guest.permission = 2;
    let mut host = game.lan.take().unwrap();
    host.hub.guests.push(guest);
    Some((game, host))
}

fn click(game: &mut Game<'_>, host: &mut LanHost, button: i8, input: i32) -> bool {
    game.lan_inventory_click(
        host,
        2,
        &ContainerClick {
            container: 0,
            state: 0,
            slot: 0,
            button,
            input,
            changed: Vec::new(),
            carried: None,
        },
    )
}

fn prepare(host: &mut LanHost, logs: u32) {
    let g = &mut host.hub.guests[0];
    g.craft_grid = [
        Some(ItemStack::new("minecraft:oak_log", logs)),
        None,
        None,
        None,
    ];
    g.craft_result = None;
    g.carried = None;
    g.inventory = vec![None; 36];
}

fn crafted(host: &LanHost) -> i32 {
    host.play
        .get(&2)
        .map_or(0, |p| p.stats.get("crafted", "minecraft:oak_planks"))
}

/// The first take teaches the recipe: it is known, waits to be highlighted
/// and is queued for `ClientboundRecipeBookAddPacket`.
#[test]
fn a_guest_take_teaches_the_recipe_and_counts_the_amount_taken() {
    let Some((mut game, mut host)) = guest_server() else {
        return;
    };
    prepare(&mut host, 1);
    assert!(!host.hub.guests[0].recipe_book.knows("minecraft:oak_planks"));
    assert!(click(&mut game, &mut host, 0, 0));
    let g = &host.hub.guests[0];
    assert_eq!(g.carried.as_ref().map(|s| s.count), Some(4));
    assert!(g.recipe_book.knows("minecraft:oak_planks"));
    assert!(
        g.recipe_book
            .to_be_displayed
            .contains("minecraft:oak_planks")
    );
    assert_eq!(g.recipes_unlocked, ["minecraft:oak_planks"]);
    assert_eq!(crafted(&host), 4);

    // A right click takes all four planks but counts half of them.
    prepare(&mut host, 1);
    assert!(click(&mut game, &mut host, 1, 0));
    assert_eq!(
        host.hub.guests[0].carried.as_ref().map(|s| s.count),
        Some(4)
    );
    assert_eq!(crafted(&host), 6);

    // A shift click moves the four planks and counts all four.
    prepare(&mut host, 1);
    assert!(click(&mut game, &mut host, 0, 1));
    let planks: u32 = host.hub.guests[0]
        .inventory
        .iter()
        .flatten()
        .filter(|s| &*s.item == "minecraft:oak_planks")
        .map(|s| s.count)
        .sum();
    assert_eq!(planks, 4);
    assert_eq!(crafted(&host), 10);
}
