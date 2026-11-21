use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
use crate::game::execution_role;
use horizon_assets::resource_source::{ResourceRoot, ResourceSource};
use horizon_client::chunk::Chunk;
use horizon_client::motion::PlayerMotion;
use horizon_core::{geometry::Vec3, position::ChunkPos};
use std::{path::PathBuf, sync::Arc};

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.game_dir = Some(
        PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join(format!(
            "falling-passengers-{:?}",
            std::thread::current().id()
        )),
    );
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&ResourceSource::directory(root.clone())),
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
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
    game.creative = true;
    test(&mut game);
}

#[test]
#[ignore = "daily"]
fn summon_falling_block_loads_non_mob_passengers() {
    with_game(|game| {
        assert!(game.run_command(
            r#"summon minecraft:falling_block 4.5 200 4.5 {BlockState:{id:stone},Passengers:[{id:armor_stand,Health:0f,Passengers:[{id:falling_block,BlockState:{id:stone}}]}]}"#
        ));
        assert_eq!(game.falling.len(), 2, "both falling blocks should load");
        assert_eq!(game.armor_stands.len(), 1, "the armor stand should load");
        let root = game
            .falling
            .iter()
            .find(|falling| !game.persist.falling_vehicles.contains_key(&falling.id))
            .expect("the summoned falling block is the vehicle");
        let stand = &game.armor_stands[0];
        assert_eq!(
            stand.vehicle,
            Some(root.id),
            "the stand should ride the root"
        );
        let child = game
            .falling
            .iter()
            .find(|falling| game.persist.falling_vehicles.get(&falling.id) == Some(&stand.id))
            .expect("the nested falling block is the stand's passenger");
        assert_eq!(
            game.persist.falling_vehicles.get(&child.id),
            Some(&stand.id)
        );
    });
}

#[test]
#[ignore = "daily"]
fn mounted_falling_block_ticks_its_lifetime_and_expires() {
    with_game(|game| {
        assert!(game.run_command(
            r#"summon minecraft:falling_block 4.5 200 4.5 {BlockState:{id:stone},Passengers:[{id:falling_block,Time:600,BlockState:{id:gravel}}]}"#
        ));
        let (&child_id, _) = game
            .persist
            .falling_vehicles
            .iter()
            .find(|(child, _)| game.falling.iter().any(|falling| falling.id == **child))
            .expect("the nested falling block should be mounted");
        let child_index = game
            .falling
            .iter()
            .position(|falling| falling.id == child_id)
            .unwrap();
        game.tick_one_falling(child_index, &[]).unwrap();
        assert!(
            game.falling.iter().all(|falling| falling.id != child_id),
            "a mounted falling block at Time:600 should expire on its next tick"
        );
    });
}

#[test]
#[ignore = "daily"]
fn falling_block_passenger_stays_attached_to_any_loaded_vehicle() {
    with_game(|game| {
        assert!(game.run_command(
            r#"summon minecraft:tnt 4.5 200 4.5 {Fuse:100,Passengers:[{id:falling_block,BlockState:{id:gravel}}]}"#
        ));
        let (&child_id, &vehicle_id) = game
            .persist
            .falling_vehicles
            .iter()
            .find(|(child, vehicle)| {
                game.falling.iter().any(|falling| falling.id == **child)
                    && game.falling.iter().all(|falling| falling.id != **vehicle)
            })
            .expect("the falling block should retain its non-falling vehicle");
        let target = game.level_entity(vehicle_id as i32).unwrap();
        let at = game.target_position(target).unwrap();
        let bounds = game.local_passenger_vehicle_bounds(vehicle_id).unwrap();
        let expected = Vec3::new(at[0], bounds.max_position().y, at[2]);
        let child_index = game
            .falling
            .iter()
            .position(|falling| falling.id == child_id)
            .unwrap();
        let age = game.falling[child_index].age;
        game.tick_one_falling(child_index, &[]).unwrap();
        let child = game
            .falling
            .iter()
            .find(|falling| falling.id == child_id)
            .expect("the mounted child should remain present");
        assert_eq!(child.position, expected);
        assert_eq!(child.age, age + 1, "rideTick still advances the block age");
        assert_eq!(
            game.persist.falling_vehicles.get(&child_id),
            Some(&vehicle_id),
            "the passenger should remain linked to its TNT vehicle"
        );

        let moved_vehicle = Vec3::new(8.5, 210.0, 8.5);
        game.tnts
            .iter_mut()
            .find(|tnt| tnt.id == vehicle_id)
            .expect("the TNT vehicle should remain present")
            .position = moved_vehicle;
        let target = game.level_entity(vehicle_id as i32).unwrap();
        let at = game.target_position(target).unwrap();
        let bounds = game.local_passenger_vehicle_bounds(vehicle_id).unwrap();
        let moved_expected = Vec3::new(at[0], bounds.max_position().y, at[2]);

        game.tick_one_falling(child_index, &[]).unwrap();
        let child = game
            .falling
            .iter()
            .find(|falling| falling.id == child_id)
            .expect("the mounted child should stay present after its vehicle moves");
        assert_eq!(child.position, moved_expected);
        assert_eq!(child.age, age + 2, "each mounted tick advances the block age");
        assert_eq!(
            game.persist.falling_vehicles.get(&child_id),
            Some(&vehicle_id),
            "the moving vehicle should keep its passenger link"
        );
    });
}
