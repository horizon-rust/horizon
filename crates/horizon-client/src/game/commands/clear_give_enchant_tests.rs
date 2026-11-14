//! `/give` splitting by the stack's `max_stack_size` component, `/enchant` on
//! a mob's main hand, and the menu `/clear` broadcasts
//! (`GiveCommand.giveItem`, `EnchantCommand.enchant`,
//! `ClearInventoryCommands.clearInventory`).
use super::*;
use crate::game::lan::LanSettings;
use crate::game::lan_actions::GuestCommand;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use crate::mobs::{finalize, gear};
use horizon_client::chunk::Chunk;
use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
use horizon_client::motion::PlayerMotion;
use horizon_core::{geometry::Vec3, position::ChunkPos};
use horizon_protocol::packets::{configuration::ClientInformation, play};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

/// A game over the prepared resources; None when this machine has none (CI),
/// and the test does nothing.
fn game() -> Option<Game<'static>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("entity-types.json").exists() {
        eprintln!("Prepared resources unavailable; the command coverage did not run");
        return None;
    }
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
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 100.0, 8.5)).unwrap());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    Some(game)
}

/// Whether the command ran, with what it said when it did not.
fn ran(game: &mut Game<'_>, command: &str) -> bool {
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.chat_capture.take().unwrap_or_default();
    if !ok {
        eprintln!(
            "{command}: {}",
            said.iter().map(Text::plain).collect::<Vec<_>>().join(" | ")
        );
    }
    ok
}

/// The hub's packets for the guest `conn`.
fn packets(rx: &mpsc::Receiver<crate::lan::Out>) -> Vec<C> {
    let mut out = Vec::new();
    while let Ok(crate::lan::Out::Packet(body)) = rx.try_recv() {
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        out.push(C::decode(id, r.rest()).unwrap());
    }
    out
}

/// Host the guest on conn 2; its packets are read from the returned receiver.
fn hosted(game: &mut Game<'static>) -> mpsc::Receiver<crate::lan::Out> {
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "command-tests".into(), 2)
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
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 2,
            uuid: 234,
            name: "CommandGuest".into(),
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
    game.lan.as_mut().unwrap().hub.guests.push(guest);
    rx
}

#[test]
fn give_splits_by_the_max_stack_size_component() {
    let Some(mut game) = game() else { return };
    // `ItemInput.createItemStack(1)`'s component: 16 a stack, so 40 is 16+16+8.
    assert!(game.run_command("give @s minecraft:stone[minecraft:max_stack_size=16] 40"));
    let counts: Vec<u32> = game.inventory.iter().flatten().map(|s| s.count).collect();
    assert_eq!(counts, [16, 16, 8]);
    // 100 stacks of 16 is the most one command gives, and 16 more is refused.
    assert!(game.run_command("give @s minecraft:stone[minecraft:max_stack_size=16] 16"));
    assert!(!game.run_command("give @s minecraft:stone[minecraft:max_stack_size=16] 1601"));
    // A plain item still stacks by the registry's 64.
    assert!(game.run_command("give @s minecraft:stone 100"));
    let counts: Vec<u32> = game.inventory.iter().flatten().map(|s| s.count).collect();
    assert_eq!(counts, [16, 16, 16, 8, 64, 36]);
}

#[test]
fn enchant_reaches_a_mobs_main_hand() {
    let Some(mut game) = game() else { return };
    assert!(game.run_command("difficulty normal"));
    assert!(ran(
        &mut game,
        "summon minecraft:zombie 0 100 0 {equipment:{mainhand:{id:\"minecraft:iron_sword\",count:1}}}"
    ));
    assert!(ran(
        &mut game,
        "enchant @e[type=minecraft:zombie,limit=1] minecraft:sharpness 3"
    ));
    // `getMainHandItem().enchant`: the stack the mob holds and shows.
    let shown = game.mobs[0].equipment_stacks()[0].clone().unwrap();
    assert_eq!(shown.item.as_ref(), "minecraft:iron_sword");
    assert_eq!(shown.enchantment_level("minecraft:sharpness"), 3);
    // The slot's enchantments, which its attacks read.
    assert_eq!(
        gear::level(&game.mobs[0], gear::MAIN, "minecraft:sharpness"),
        3
    );
    assert_eq!(
        finalize::slot_enchantments(&game.mobs[0], finalize::MAINHAND),
        vec![(Arc::from("minecraft:sharpness"), 3u32)]
    );
    // An enchantment the sword cannot take is still refused, and changes nothing.
    assert!(!ran(
        &mut game,
        "enchant @e[type=minecraft:zombie,limit=1] minecraft:protection 1"
    ));
    assert_eq!(shown.enchantments().len(), 1);
    // A mob the game equipped from a loot table keeps only its slot's
    // enchantments (`getMainHandItem` builds the stack from them): an exclusive
    // one is still refused, and a compatible one joins them.
    {
        let m = &mut game.mobs[0];
        m.extra
            .set_main_hand(Some(Arc::from("minecraft:iron_sword")));
        gear::set(m, gear::MAIN, vec![(Arc::from("minecraft:smite"), 2)]);
    }
    assert!(!ran(
        &mut game,
        "enchant @e[type=minecraft:zombie,limit=1] minecraft:sharpness 3"
    ));
    assert_eq!(gear::level(&game.mobs[0], gear::MAIN, "minecraft:smite"), 2);
    assert!(ran(
        &mut game,
        "enchant @e[type=minecraft:zombie,limit=1] minecraft:looting 1"
    ));
    assert_eq!(gear::level(&game.mobs[0], gear::MAIN, "minecraft:smite"), 2);
    assert_eq!(
        gear::level(&game.mobs[0], gear::MAIN, "minecraft:looting"),
        1
    );
}

#[test]
fn clear_empties_the_menu_grid_and_tells_the_player() {
    let Some(mut game) = game() else { return };
    let rx = hosted(&mut game);
    let log = ItemStack::new("minecraft:oak_log", 1);
    let planks = ItemStack::new("minecraft:oak_planks", 4);
    {
        let g = game.lan.as_mut().unwrap().hub.guest_mut(2).unwrap();
        g.craft_grid[0] = Some(log.clone());
        g.craft_grid[1] = Some(log);
        g.craft_result = Some(planks.clone());
        g.carried = Some(planks);
        g.inventory[0] = Some(ItemStack::new("minecraft:oak_log", 3));
    }
    assert!(game.run_guest_command(GuestCommand {
        conn: 2,
        command: "clear @s".into(),
        permission: 2,
        integrated: false,
        quiet: false,
    }));
    let g = game.lan.as_ref().unwrap().hub.guest(2).unwrap();
    assert_eq!(g.inventory[0], None);
    assert_eq!(g.craft_grid, [None, None, None, None]);
    assert_eq!(g.carried, None);
    // `inventoryMenu.slotsChanged`: the result of the now-empty grid.
    assert_eq!(g.craft_result, None);
    // `broadcastChanges`: the cursor the player no longer holds.
    assert!(
        packets(&rx)
            .iter()
            .any(|p| matches!(p, C::SetCursorItem(play::SetCursorItem { item: None }))),
        "the emptied cursor goes to the player"
    );
    // The menu it is shown: slot 0 is the empty result, the cursor empty.
    let host = game.lan.as_mut().unwrap();
    host.hub.send_inventory(0);
    assert!(
        packets(&rx).iter().any(|p| matches!(
            p,
            C::ContainerSetContent(play::ContainerSetContent { container: 0, items, carried, .. })
                if items[0].is_none() && carried.is_none()
        )),
        "the emptied result slot and cursor are what the menu shows"
    );
}
