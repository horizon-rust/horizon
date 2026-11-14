//! `/damage` on the decorations `BlockAttachedEntity.hurtServer` covers (a
//! frame, a painting, a cushion), the `#bypasses_invulnerability` hit that
//! reaches a creative or spectator player (`Player.hurtServer`), and the
//! damage event a boat hit posts (`VehicleEntity.hurtServer`). Vanilla 26.3
//! is the oracle.
use crate::game::*;
use horizon_client::chunk::Chunk;
use horizon_client::mobs::Who;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

fn game() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
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
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
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
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    for command in [
        "forceload add 0 0",
        "fill 0 99 0 15 99 15 stone",
        "fill 4 100 0 4 102 15 stone",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game
}

fn with_player(game: &mut Game<'_>) {
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(
            game.motion,
            horizon_core::geometry::Vec3::new(14.5, 100.0, 14.5),
        )
        .unwrap(),
    );
}

fn health(game: &Game<'_>) -> f32 {
    game.player.as_ref().unwrap().health.health()
}

/// The game events posted from here on, in order: their name, position and
/// causing entity.
fn capture() {
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
}

fn take_events() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| e.borrow_mut().take().unwrap_or_default())
}

fn dropped(game: &Game<'_>, item: &str) -> usize {
    game.dropped
        .iter()
        .filter(|d| &*d.stack.item == item)
        .count()
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn damage_reaches_frames_paintings_and_cushions() {
    let mut game = game();
    // A filled frame and a painting on the wall's west face, a cushion on the floor.
    assert!(game.run_command(
        "summon minecraft:item_frame 3.5 100.5 6.5 {Facing:4b,Item:{id:\"minecraft:diamond\",count:1}}"
    ));
    assert!(game.run_command(
        "summon minecraft:painting 5.5 100 9.5 {facing:3b,variant:\"minecraft:kebab\",block_pos:[I;5,100,9]}"
    ));
    assert!(game.run_command("summon minecraft:cushion 8.5 100 12.5"));
    assert_eq!(
        (
            game.item_frames.len(),
            game.paintings.len(),
            game.cushions.len()
        ),
        (1, 1, 1)
    );
    // `ItemFrame.hurtServer`: a filled frame gives its item up and stays.
    assert!(game.run_command("damage @e[type=minecraft:item_frame,limit=1] 1 minecraft:generic"));
    assert_eq!(game.item_frames.len(), 1, "the frame stays");
    assert!(game.item_frames[0].item.is_none(), "its item came out");
    assert_eq!(dropped(&game, "minecraft:diamond"), 1);
    // The empty frame breaks whole.
    assert!(game.run_command("damage @e[type=minecraft:item_frame,limit=1] 1 minecraft:generic"));
    assert!(game.item_frames.is_empty(), "the empty frame breaks");
    assert_eq!(dropped(&game, "minecraft:item_frame"), 1);
    // `BlockAttachedEntity.hurtServer`: a painting and a cushion break and drop.
    assert!(game.run_command("damage @e[type=minecraft:painting,limit=1] 1 minecraft:generic"));
    assert!(game.paintings.is_empty());
    assert_eq!(dropped(&game, "minecraft:painting"), 1);
    assert!(game.run_command("damage @e[type=minecraft:cushion,limit=1] 1 minecraft:generic"));
    assert!(game.cushions.is_empty());
    assert_eq!(dropped(&game, "minecraft:white_cushion"), 1);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_fixed_frame_only_gives_way_to_a_bypassing_hit() {
    let mut game = game();
    assert!(game.run_command(
        "summon minecraft:item_frame 3.5 100.5 3.5 {Facing:4b,Fixed:1b,Item:{id:\"minecraft:diamond\",count:1}}"
    ));
    // `ItemFrame.canHurtWhenFixed` refuses an ordinary source.
    assert!(!game.run_command("damage @e[type=minecraft:item_frame,limit=1] 1 minecraft:generic"));
    assert_eq!(game.item_frames.len(), 1, "the fixed frame stays");
    // `#bypasses_invulnerability` breaks it; `ItemFrame.dropItem` does nothing
    // for a fixed frame, so neither it nor its item drops.
    assert!(
        game.run_command("damage @e[type=minecraft:item_frame,limit=1] 1 minecraft:generic_kill")
    );
    assert!(game.item_frames.is_empty());
    assert_eq!(dropped(&game, "minecraft:item_frame"), 0);
    assert_eq!(dropped(&game, "minecraft:diamond"), 0);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_bypassing_type_hurts_a_creative_and_a_spectator_player() {
    for mode in ["creative", "spectator"] {
        let mut game = game();
        with_player(&mut game);
        assert!(game.run_command(&format!("gamemode {mode} @p")));
        // `Player.hurtServer`'s `abilities.invulnerable` refuses an ordinary source.
        assert!(!game.run_command("damage @s 1 minecraft:generic"), "{mode}");
        assert_eq!(health(&game), 20.0, "{mode}");
        // `#bypasses_invulnerability` passes it: 1 damage lands.
        assert!(
            game.run_command("damage @s 1 minecraft:generic_kill"),
            "{mode}"
        );
        assert_eq!(health(&game), 19.0, "{mode}");
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_boat_hit_posts_the_damage_event() {
    let mut game = game();
    with_player(&mut game);
    assert!(game.run_command("summon minecraft:oak_boat 8.5 100 8.5"));
    assert_eq!(game.boats.len(), 1);
    let at = game.boats[0].position;
    capture();
    assert!(game.run_command("damage @e[type=minecraft:oak_boat,limit=1] 1"));
    assert_eq!(
        take_events(),
        vec![("minecraft:entity_damage", [at.x, at.y, at.z], None)],
        "VehicleEntity.hurtServer posts the damage event"
    );
    // Attributed to the causing player, as `source.getEntity()`.
    capture();
    assert!(
        game.run_command("damage @e[type=minecraft:oak_boat,limit=1] 1 minecraft:generic by @s")
    );
    assert_eq!(
        take_events(),
        vec![(
            "minecraft:entity_damage",
            [at.x, at.y, at.z],
            Some(Who::Player)
        )]
    );
    // A mob's hit is the mob's, not the player's.
    assert!(game.run_command("summon minecraft:pig 12.5 100 12.5"));
    let mob = game.mobs[0].id;
    capture();
    assert!(game.run_command(
        "damage @e[type=minecraft:oak_boat,limit=1] 1 minecraft:generic \
         by @e[type=minecraft:pig,limit=1]"
    ));
    assert_eq!(
        take_events(),
        vec![(
            "minecraft:entity_damage",
            [at.x, at.y, at.z],
            Some(Who::Mob(mob))
        )]
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_mob_caused_hit_is_credited_to_the_mob() {
    let mut game = game();
    assert!(game.run_command("summon minecraft:item_frame 3.5 100.5 6.5 {Facing:4b}"));
    assert!(game.run_command("summon minecraft:pig 12.5 100 12.5"));
    let mob = game.mobs[0].id;
    capture();
    assert!(game.run_command(
        "damage @e[type=minecraft:item_frame,limit=1] 1 minecraft:generic \
         by @e[type=minecraft:pig,limit=1]"
    ));
    // `BlockAttachedEntity.kill(level, source.getEntity())`, then
    // `ItemFrame.dropItem(level, causedBy)`: both events are the mob's.
    let events = take_events();
    let sources = |name: &str| {
        events
            .iter()
            .filter(|e| e.0 == name)
            .map(|e| e.2)
            .collect::<Vec<_>>()
    };
    assert_eq!(sources("minecraft:entity_die"), vec![Some(Who::Mob(mob))]);
    assert_eq!(sources("minecraft:block_change"), vec![Some(Who::Mob(mob))]);
}
