//! LAN event, metadata and synchronization unit tests.
use super::{EventTimings, OFFHAND_SLOT, equipment_slot_name, living_flags, mob_freeze_meta};

#[test]
fn a_burst_of_short_events_still_has_a_slow_tick_note() {
    use horizon_client::spans;
    // Thread-local recording avoids changing instrumentation on other test threads.
    spans::record_here(true);
    spans::take_notes();
    let mut timings = EventTimings::new(spans::NOTE_MS);
    for _ in 0..200 {
        spans::note(5.0, || "UseItemOn of fixture player".into());
        timings.add("UseItemOn", "fixture player", None::<()>, 5.0);
    }
    assert!(
        spans::take_notes().is_empty(),
        "every individual event is below NOTE_MS"
    );
    for burst in timings.bursts() {
        spans::note(burst.ms, || format!("{} x {}", burst.count, burst.kind));
    }
    let notes = spans::take_notes();
    spans::record_here(false);
    assert_eq!(notes, vec![(1000.0, "200 x UseItemOn".into())]);
}
use horizon_client::mobs::Mob;
use horizon_protocol::entity_data::{Value, serializer};

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn guest_climbable_glide() {
    use crate::game::{Audio, Options, Progress, Settings, load_resources};
    use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest};
    use horizon_client::motion::MotionWorld;
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
    let render = game.pack.assets.reload(&root).unwrap();
    game.world = horizon_client::world::World::test_scene(&game.pack.assets, &render, 2).unwrap();

    let sync = Arc::new(horizon_client::lan::data::SyncData::empty());
    let net = NetConfig {
        listen: false,
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
    let (tx, _rx) = mpsc::channel();
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
    for (block, expected) in [
        ("minecraft:ladder", false),
        ("minecraft:scaffolding", false),
        ("minecraft:vine", true),
    ] {
        let state = game.pack.assets.registry.default_state(block).unwrap();
        game.world.set(0, 4, 0, state).unwrap();
        host.hub.guests[0].fall_flying = true;
        game.as_guest(&mut host, 1, |game| {
            let player = game.player.as_mut().unwrap();
            MotionWorld {
                entities: &[],
                player,
                scene: &mut game.world,
                assets: &game.pack.assets,
                collision_binding: game.collision,
                resources: game.motion,
                health: &game.health_resources,
            }
            .server_travel()
            .unwrap();
        })
        .unwrap();
        assert_eq!(host.hub.guests[0].fall_flying, expected, "{block}");
    }
}

#[test]
#[ignore = "requires prepared vanilla resources"]
fn datapack_damage_world_open_reuses_the_no_pack_preload() {
    use super::{SyncCache, SyncData, cached_sync_data, initialize_world_sync_data};
    use horizon_world::loot::PackOverlay;
    use std::{path::Path, sync::Arc};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    let preloaded = Arc::new(SyncData::load(&root, &PackOverlay::NONE).unwrap());
    let mut cache = SyncCache {
        data: Some((false, Ok(preloaded.clone()))),
        ..Default::default()
    };
    let damage = horizon_world::damage_source::Catalog::load(&root, &PackOverlay::NONE).unwrap();
    // A missing root makes an accidental second disk read fail, rather
    // than merely comparing equal snapshots loaded twice.
    let absent = root.join("no-such-resources");
    initialize_world_sync_data(&mut cache, &absent, &PackOverlay::NONE, &damage).unwrap();
    let opened = cached_sync_data(&mut cache, &absent, &PackOverlay::NONE).unwrap();
    assert!(Arc::ptr_eq(&preloaded, &opened));
    assert!(Arc::ptr_eq(
        &opened,
        &cached_sync_data(&mut cache, &absent, &PackOverlay::NONE).unwrap()
    ));
}

#[test]
#[ignore = "requires prepared vanilla resources"]
fn datapack_damage_world_open_replaces_preload_atomically() {
    use super::{SyncCache, SyncData, cached_sync_data, initialize_world_sync_data};
    use horizon_world::loot::PackOverlay;
    use std::{
        path::Path,
        sync::{Arc, Barrier, Mutex},
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    let preloaded = Arc::new(SyncData::load(&root, &PackOverlay::NONE).unwrap());
    let cache = Arc::new(Mutex::new(SyncCache {
        data: Some((false, Ok(preloaded.clone()))),
        ..Default::default()
    }));
    let barrier = Arc::new(Barrier::new(2));
    let worker = {
        let cache = cache.clone();
        let barrier = barrier.clone();
        let root = root.clone();
        std::thread::spawn(move || {
            barrier.wait();
            for _ in 0..32 {
                cached_sync_data(&mut cache.lock().unwrap(), &root, &PackOverlay::NONE).unwrap();
            }
        })
    };
    let files = vec![(
        "damage_type/demo:world_heat".into(),
        vec![br#"{"message_id":"world_heat","scaling":"never","exhaustion":0.25}"#.to_vec()],
    )];
    barrier.wait();
    let overlay = PackOverlay { files: &files };
    let damage = horizon_world::damage_source::Catalog::load(&root, &overlay).unwrap();
    initialize_world_sync_data(&mut cache.lock().unwrap(), &root, &overlay, &damage).unwrap();
    let opened = cached_sync_data(&mut cache.lock().unwrap(), &root, &overlay).unwrap();
    worker.join().unwrap();
    assert!(!Arc::ptr_eq(&preloaded, &opened));
    let id = opened
        .id("minecraft:damage_type", "demo:world_heat")
        .unwrap();
    let later = cached_sync_data(&mut cache.lock().unwrap(), &root, &PackOverlay::NONE).unwrap();
    assert!(
        Arc::ptr_eq(&opened, &later),
        "a late helper or disabled pack cannot replace a frozen world"
    );
    let packet = later.registry_packets(true, &horizon_client::chunk::LevelShapes::VANILLA);
    let packet = packet
        .iter()
        .find(|p| p.registry == "minecraft:damage_type")
        .unwrap();
    assert_eq!(packet.entries[id as usize].id, "demo:world_heat");
    assert!(packet.entries[id as usize].data.is_some());
}

#[test]
#[ignore = "requires prepared vanilla resources"]
fn datapack_damage_unrelated_packs_do_not_load_sync_data_at_open() {
    use super::{SyncCache, SyncData, initialize_world_sync_data};
    use horizon_world::{damage_source::Catalog, loot::PackOverlay};
    use std::{path::Path, sync::Arc};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    let damage = Catalog::load(&root, &PackOverlay::NONE).unwrap();
    let preloaded = Arc::new(SyncData::load(&root, &PackOverlay::NONE).unwrap());
    // A recipe-only pack and a damage-tag-only pack must retain main's
    // lazy wire-data path. An absent root proves no full load happens.
    for key in ["recipe/demo:test", "tags/minecraft:damage_type/is_fire"] {
        let files = vec![(key.into(), vec![br#"{"values":[]}"#.to_vec()])];
        let mut cache = SyncCache {
            data: Some((false, Ok(preloaded.clone()))),
            ..Default::default()
        };
        initialize_world_sync_data(
            &mut cache,
            &root.join("absent"),
            &PackOverlay { files: &files },
            &damage,
        )
        .unwrap();
        assert!(cache.data.is_none(), "{key} must not eagerly load SyncData");
        assert!(cache.damage.is_some());
    }
    let mut empty = SyncCache::default();
    initialize_world_sync_data(
        &mut empty,
        &root.join("absent"),
        &PackOverlay::NONE,
        &damage,
    )
    .unwrap();
    assert!(
        empty.data.is_none(),
        "no-pack worlds need no synchronous fallback load either"
    );
}

#[test]
#[ignore = "requires prepared vanilla resources"]
fn datapack_damage_lazy_wire_snapshot_survives_reload_before_first_hub() {
    use super::{SyncCache, cached_sync_data, initialize_world_sync_data, prepare_sync_reload};
    use horizon_world::{damage_source::Catalog, loot::PackOverlay};
    use std::path::Path;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    let damage = Catalog::load(&root, &PackOverlay::NONE).unwrap();
    let initial = vec![(
        "dialog/demo:before".into(),
        vec![br#"{"type":"minecraft:notice"}"#.to_vec()],
    )];
    let mut cache = SyncCache::default();
    initialize_world_sync_data(&mut cache, &root, &PackOverlay { files: &initial }, &damage)
        .unwrap();
    assert!(cache.data.is_none());
    // A late vanilla helper cannot install its overlay over this world.
    cached_sync_data(&mut cache, &root, &PackOverlay::NONE).unwrap();
    assert!(cache.data.is_none());
    let changed = vec![
        (
            "damage_type/demo:too_late".into(),
            vec![br#"{"message_id":"too_late","scaling":"never","exhaustion":0.5}"#.to_vec()],
        ),
        (
            "damage_type/minecraft:fall".into(),
            vec![br#"{"message_id":"changed","scaling":"never","exhaustion":0.9}"#.to_vec()],
        ),
        (
            "dialog/demo:after".into(),
            vec![br#"{"type":"minecraft:notice"}"#.to_vec()],
        ),
        (
            "tags/demo:damage_type/reloaded".into(),
            vec![
                br#"{"values":["minecraft:fall",{"id":"demo:too_late","required":false}]}"#
                    .to_vec(),
            ],
        ),
    ];
    let overlay = PackOverlay { files: &changed };
    assert!(prepare_sync_reload(&mut cache, &overlay).is_none());
    let health = damage.reload_tags(&root, &overlay).unwrap();
    let wire = cached_sync_data(&mut cache, &root, &overlay).unwrap();
    assert!(wire.id("minecraft:damage_type", "demo:too_late").is_none());
    assert!(health.id("demo:too_late").is_none());
    assert!(wire.id("minecraft:dialog", "demo:before").is_none());
    assert!(
        wire.id("minecraft:dialog", "demo:after").is_some(),
        "unrelated registries stay lazy"
    );
    let fall = wire.id("minecraft:damage_type", "minecraft:fall").unwrap();
    let packets = wire.registry_packets(false, &horizon_client::chunk::LevelShapes::VANILLA);
    let packet = packets
        .iter()
        .find(|p| p.registry == "minecraft:damage_type")
        .unwrap();
    let definition = health
        .definition(health.id("minecraft:fall").unwrap())
        .unwrap();
    assert_eq!(
        packet.entries[fall as usize].data.as_ref(),
        Some(&definition.to_nbt())
    );
    let tags = wire
        .tags
        .registries
        .iter()
        .find(|r| r.registry == "minecraft:damage_type")
        .unwrap();
    assert_eq!(
        tags.tags
            .iter()
            .find(|t| t.name == "demo:reloaded")
            .unwrap()
            .ids,
        vec![fall]
    );
    assert_eq!(
        health.tag_members("demo:reloaded").collect::<Vec<_>>(),
        vec!["minecraft:fall"]
    );
    // The next world's initial load can register the previously-late type.
    let next = Catalog::load(&root, &overlay).unwrap();
    let mut next_cache = SyncCache::default();
    initialize_world_sync_data(&mut next_cache, &root, &overlay, &next).unwrap();
    assert!(
        cached_sync_data(&mut next_cache, &root, &overlay)
            .unwrap()
            .id("minecraft:damage_type", "demo:too_late")
            .is_some()
    );
}

#[test]
#[ignore = "requires prepared vanilla resources"]
fn datapack_damage_lazy_wire_load_caches_after_all_packs_are_removed() {
    use super::{SyncCache, cached_sync_data, initialize_world_sync_data, prepare_sync_reload};
    use horizon_world::{damage_source::Catalog, loot::PackOverlay};
    use std::{path::Path, sync::Arc};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    let damage = Catalog::load(&root, &PackOverlay::NONE).unwrap();
    let files = vec![(
        "recipe/demo:unrelated".into(),
        vec![br#"{"type":"minecraft:crafting_special_mapcloning"}"#.to_vec()],
    )];
    let mut cache = SyncCache::default();
    initialize_world_sync_data(&mut cache, &root, &PackOverlay { files: &files }, &damage).unwrap();
    assert!(cache.data.is_none());
    assert!(prepare_sync_reload(&mut cache, &PackOverlay::NONE).is_none());
    let opened = cached_sync_data(&mut cache, &root, &PackOverlay::NONE).unwrap();
    // An accidental uncached load would fail on the next request rather
    // than merely reconstructing an equal registry from the same files.
    let reused = cached_sync_data(
        &mut cache,
        &root.join("no-such-resources"),
        &PackOverlay::NONE,
    )
    .unwrap();
    assert!(Arc::ptr_eq(&opened, &reused));
}

fn mob_of(kind: &str) -> Option<Mob> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    let types = horizon_client::mobs::MobTypes::load(&root).ok()?;
    let t = std::sync::Arc::new(types.types.get(kind)?.clone());
    Some(Mob::new(
        1,
        t,
        horizon_core::geometry::Vec3::new(0.0, 64.0, 0.0),
        0.0,
    ))
}

fn freeze_meta(m: &Mob) -> (Vec<(u8, i32, Value)>, Vec<(u8, i32, Value)>) {
    let (mut meta, mut defaults) = (Vec::new(), Vec::new());
    mob_freeze_meta(m, &mut meta, &mut defaults);
    (meta, defaults)
}

/// Entity.setTicksFrozen can go negative; index 7 must carry it. Index 1
/// (air supply) goes out for every mob, with the species' own default.
#[test]
fn negative_frozen_ticks_and_axolotl_air_are_synced() {
    let Some(mut cow) = mob_of("minecraft:cow") else {
        return;
    };
    cow.ticks_frozen = -3;
    let (meta, _) = freeze_meta(&cow);
    assert!(meta.contains(&(7, serializer::INT, Value::Int(-3))));
    cow.air = 100;
    assert!(
        freeze_meta(&cow)
            .0
            .contains(&(1, serializer::INT, Value::Int(100)))
    );

    let Some(mut axolotl) = mob_of("minecraft:axolotl") else {
        return;
    };
    axolotl.air = 5000;
    let (meta, defaults) = freeze_meta(&axolotl);
    assert!(meta.contains(&(1, serializer::INT, Value::Int(5000))));
    assert!(defaults.contains(&(1, serializer::INT, Value::Int(6000))));
    axolotl.air = 6000;
    assert!(freeze_meta(&axolotl).0.iter().all(|e| e.0 != 1));
}

/// Raider.IS_CELEBRATING goes out through the tracker's own metadata
/// (`mob_meta`), not only through the helper: false is listed as a
/// default so the end of a celebration is sent, and true is sent while
/// the raider celebrates.
#[test]
fn a_celebrating_raider_is_sent_by_mob_meta() {
    let Some(mut pillager) = mob_of("minecraft:pillager") else {
        return;
    };
    let hub = meta_hub("RaiderMetaTest");
    let meta_of = |m: &Mob| {
        let (mut meta, mut defaults) = (Vec::new(), Vec::new());
        super::mob_meta(m, &hub, None, 0, &mut meta, &mut defaults);
        (meta, defaults)
    };

    let (meta, defaults) = meta_of(&pillager);
    assert!(defaults.contains(&(16, serializer::BOOLEAN, Value::Bool(false))));
    assert!(meta.iter().all(|e| e.0 != 16), "{meta:?}");
    horizon_client::mobs::raider::state_mut(&mut pillager).celebrating = true;
    let (meta, _) = meta_of(&pillager);
    assert!(
        meta.contains(&(16, serializer::BOOLEAN, Value::Bool(true))),
        "{meta:?}"
    );
}

/// A content mod's living type sends the entity data of its template's
/// class (`Zombie.DATA_BABY_ID`, the villager's unhappy counter), entry
/// for entry what its template sends.
#[test]
fn a_mod_living_type_sends_its_templates_entity_data() {
    let types = super::super::mods::living_mod_types();
    let hub = meta_hub("ModMobMetaTest");
    let meta_of = |id: &str| {
        let at = horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5);
        let mut m = Mob::new(1, types[id].clone(), at, 0.0);
        m.extra.age = -24000;
        // The same health: the ghoul's own maximum is 40, the zombie's 20.
        m.health = 10.0;
        let (mut meta, mut defaults) = (Vec::new(), Vec::new());
        super::mob_meta(&m, &hub, None, 0, &mut meta, &mut defaults);
        (meta, defaults)
    };
    for (template, id) in [
        ("minecraft:zombie", "ruby:ghoul"),
        ("minecraft:villager", "ruby:settler"),
        ("minecraft:wandering_trader", "ruby:merchant"),
    ] {
        assert_eq!(
            meta_of(id),
            meta_of(template),
            "{id} sends what {template} sends"
        );
    }
}

/// A hub with no players and empty registries, for `mob_meta`.
fn meta_hub(name: &str) -> horizon_client::lan::hub::Hub {
    use horizon_client::lan::{Lan, NetConfig, chunks::ChunkEncoder, data, hub};
    let sync = std::sync::Arc::new(data::SyncData::empty());
    let config = NetConfig {
        announce: false,
        ..NetConfig::loopback(0, name.into(), 2)
    };
    let lan = Lan::start_with(sync.clone(), config).unwrap();
    let encoder = ChunkEncoder::new(
        [] as [(std::sync::Arc<str>, bool); 0],
        &horizon_protocol::registry::Builtin::default(),
        &[],
        &[],
    );
    let hub_config = hub::HubConfig {
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
        integrated: false,
        levels: Default::default(),
    };
    hub::Hub::new(lan, sync, encoder, hub_config)
}

/// `LivingEntity.startUsingItem` sets flag 1 for the use and flag 2 for
/// the off hand, so an off-hand use is 3, not 2. A reader takes bit 0 as
/// `isUsingItem` and bit 1 as `getUsedItemHand`.
#[test]
fn an_off_hand_use_flags_both_the_use_and_the_hand() {
    assert_eq!(living_flags(None), 0);
    assert_eq!(living_flags(Some((0, 12))), 1);
    assert_eq!(living_flags(Some((OFFHAND_SLOT, 12))), 3);
}

/// `ComposterBlock.useItemOn` calls `levelEvent(1500, pos, data)`, the
/// null-source overload, so the player who filled it still receives
/// `handleFill`. `Block.playerWillDestroy`'s 2001 names that player.
#[test]
fn the_composter_fill_reaches_the_player_who_filled_it() {
    let actor = Some(4);
    assert_eq!(super::level_event_except(1500, actor, false), None);
    assert_eq!(super::level_event_except(2001, actor, false), actor);
    assert_eq!(super::level_event_except(2001, actor, true), None);
}

/// The names a mod matches on for a broken item's slot (a contract).
#[test]
fn a_broken_items_slot_is_named_for_mods() {
    use horizon_world::player_inventory::EquipmentSlot::*;
    let named: Vec<_> = [MainHand, OffHand, Head, Chest, Legs, Feet, Body, Saddle]
        .map(equipment_slot_name)
        .to_vec();
    assert_eq!(
        named,
        [
            "mainhand", "offhand", "head", "chest", "legs", "feet", "body", "saddle"
        ]
    );
}
