//! Direct production handlers over an empty in-memory world; no server or renderer.
use super::*;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::chunk::Chunk;
use std::sync::Arc;

#[test]
#[ignore = "needs explicit HORIZON_TEST_RESOURCES and HORIZON_PORTAL_REFERENCE"]
fn live_and_saved_portal_counters_follow_runtime_and_data_updates() {
    let reference: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var_os("HORIZON_PORTAL_REFERENCE").unwrap()).unwrap(),
    )
    .unwrap();
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
    options.headless = true;
    options.game_dir = Some(std::env::current_dir().unwrap());
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    game.build_registries();
    game.options.dedicated = Some(Vec::new());
    game.world = horizon_client::world::World::new(game.res.facts().air, 1);
    for z in -2..=2 {
        for x in -2..=2 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
    }
    let kinds = [
        "minecraft:oak_boat",
        "minecraft:minecart",
        "minecraft:arrow",
        "minecraft:pig",
    ];
    let mut ids = Vec::new();
    for (index, kind) in kinds.iter().enumerate() {
        let uuid = [11318, 10, 0, index as i32];
        let tag = Tag::compound([
            ("id", Tag::String((*kind).into())),
            ("UUID", Tag::IntArray(uuid.to_vec())),
            ("Pos", doubles([4.5, 210.0, 4.5])),
            ("NoAI", Tag::Byte(1)),
            ("NoGravity", Tag::Byte(1)),
            ("PortalCooldown", Tag::Int(5)),
        ]);
        game.load_listed(kind, Some(uuid), None, tag).unwrap();
        ids.push(game.next_entity);
    }
    assert_eq!(game.boats.len(), 1);
    assert_eq!(game.carts.len(), 1);
    assert_eq!(game.creatures.projectiles.len(), 1);
    assert_eq!(game.mobs.len(), 1);
    game.tickets.update(Some(crate::ticket_levels::Sources {
        forced: vec![(0, 0)],
        ..Default::default()
    }));
    assert!(game.entity_ticking_area()(4, 4));
    game.handle_vehicle_portals();
    game.handle_entity_portals();
    for id in &ids {
        assert_eq!(game.portals.entities.get(id).unwrap().cooldown, 4);
        let target = game.level_entity(*id as i32).unwrap();
        assert_eq!(counter(&game.entity_nbt(target).unwrap()), 4);
    }
    let mut nbt = Vec::new();
    let mut saved = Vec::new();
    for row in reference["contact"].as_array().unwrap() {
        for (&id, value) in ids.iter().zip(row.as_array().unwrap()) {
            game.restore_portal_cooldown(
                id,
                &Tag::compound([("PortalCooldown", Tag::Int(value.as_i64().unwrap() as i32))]),
            );
        }
        nbt.push(
            ids.iter()
                .map(|id| {
                    let target = game.level_entity(*id as i32).unwrap();
                    counter(&game.entity_nbt(target).unwrap())
                })
                .collect::<Vec<_>>(),
        );
        let chunks = game.entity_chunks(&std::collections::HashSet::from([(0, 0)]), false);
        saved.push(
            kinds
                .iter()
                .map(|kind| {
                    counter(
                        chunks
                            .iter()
                            .flat_map(|(_, tags)| tags)
                            .find(|tag| {
                                horizon_client::save::text(tag.get("id")).as_deref() == Some(*kind)
                            })
                            .unwrap(),
                    )
                })
                .collect::<Vec<_>>(),
        );
    }
    let id = ids[0];
    let mut gateway = horizon_client::end_gateway::Traveller::default();
    gateway.set_cooldown(7);
    game.end.gateways.insert(id, gateway);
    let target = game.level_entity(id as i32).unwrap();
    assert_eq!(counter(&game.entity_nbt(target).unwrap()), 7);
    game.restore_portal_cooldown(id, &Tag::compound([("PortalCooldown", Tag::Int(3))]));
    assert_eq!(game.end.gateways[&id].cooldown(), 3);
    game.end.gateways.remove(&id);
    // /data merge reads the new zero counter into the live entity, not just its raw tag.
    for id in ids {
        let target = game.level_entity(id as i32).unwrap();
        let mut tag = game.entity_nbt(target).unwrap();
        if let Tag::Compound(fields) = &mut tag {
            fields.insert("PortalCooldown".into(), Tag::Int(2));
        }
        game.apply_entity_nbt(target, tag).unwrap();
        assert_eq!(game.portals.entities.get(&id).unwrap().cooldown, 2);
        game.portals.entities.get_mut(&id).unwrap().cooldown = 5;
        let target = game.level_entity(id as i32).unwrap();
        let mut tag = game.entity_nbt(target).unwrap();
        if let Tag::Compound(fields) = &mut tag {
            fields.insert("PortalCooldown".into(), Tag::Int(0));
        }
        game.apply_entity_nbt(target, tag).unwrap();
        assert!(!game.portals.entities.contains_key(&id));
    }
    let id = game.next_entity;
    let state = game.portals.entities.entry(id).or_default();
    state.set_inside(BlockPos::new(4, 210, 4), 300);
    game.restore_portal_cooldown(id, &Tag::compound([("PortalCooldown", Tag::Int(2))]));
    assert!(game.portals.entities[&id].process.is_some());
    assert_eq!(game.portals.entities[&id].cooldown, 2);
    game.restore_portal_cooldown(id, &Tag::compound([("PortalCooldown", Tag::Int(0))]));
    assert!(game.portals.entities[&id].process.is_some());
    assert_eq!(game.portals.entities[&id].cooldown, 0);
    let report = serde_json::json!({"nbt":nbt,"saved":saved});
    for field in ["nbt", "saved"] {
        assert_eq!(report[field], reference["contact"], "{field}");
    }
    if let Some(path) = std::env::var_os("HORIZON_PORTAL_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
fn counter(tag: &Tag) -> i32 {
    horizon_client::save::number(tag.get("PortalCooldown")).unwrap() as i32
}
