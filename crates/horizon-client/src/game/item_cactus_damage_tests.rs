//! `CactusBlock.entityInside` for a dropped item (`Entity.applyEffectsFromBlocks`
//! in the item's own tick): a point of health a tick, and `ItemEntity.hurtServer`
//! posts `entity_damage` at the item's position for every hit it takes
//! (`ItemEntity.java:300-306`), so a cactus killing an item clicks a sculk
//! sensor.
use super::super::native_machines::tests::server_game;
use horizon_client::items::ItemStack;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;

type Posted = (&'static str, [f64; 3], Option<horizon_client::mobs::Who>);

fn capture() {
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
}

fn take_events() -> Vec<Posted> {
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| e.borrow_mut().take().unwrap_or_default())
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_cactus_hurts_an_item_a_point_a_tick_and_posts_entity_damage_for_each_hit() {
    let mut game = server_game();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 70.0, 8.5)).unwrap());
    game.update_chunk_tickets();
    assert!(
        game.entity_ticking_area()(10, 8),
        "chunk (0, 0) ticks entities"
    );
    let cactus = game
        .pack
        .assets
        .registry
        .default_state("minecraft:cactus")
        .unwrap();
    game.world.set(10, 70, 8, cactus).unwrap();
    // The item's own box sits inside the cactus block, as `/summon` drops one
    // there: it has no physics and slides out over the next few ticks.
    game.spawn_item(
        ItemStack::new("minecraft:stone", 1),
        [10.5, 70.0, 8.5],
        [0.0, 0.0, 0.0],
        0,
    );
    game.ticked_entities_through = u32::MAX;
    capture();
    // One tick inside the cactus: a point off its 5 health, and the hit is
    // heard at the item's own position, with no source entity (a cactus is
    // not one: `damageSources().cactus()` names none).
    game.tick_items();
    let item = &game.dropped[0];
    let at = [item.position.x, item.position.y, item.position.z];
    assert_eq!(
        item.health, 4,
        "CactusBlock.entityInside takes a point a tick"
    );
    assert_eq!(take_events(), vec![("minecraft:entity_damage", at, None)]);
    // The hit that kills it is heard too, before the item leaves the level.
    game.dropped[0].health = 1;
    capture();
    game.tick_items();
    assert!(game.dropped.is_empty(), "1 health: the cactus kills it");
    let events = take_events();
    assert_eq!(events.len(), 1, "{events:?}");
    let (event, pos, source) = events[0];
    assert_eq!(event, "minecraft:entity_damage");
    assert_eq!(source, None, "a cactus is no source entity");
    assert!(
        (0..3).all(|i| (pos[i] - at[i]).abs() < 0.5),
        "{pos:?} is not where the item was ({at:?})"
    );
}
