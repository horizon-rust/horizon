use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
use horizon_client::chunk::Chunk;
use horizon_client::motion::PlayerMotion;
use horizon_core::{geometry::Vec3, position::ChunkPos};
use std::{path::PathBuf, sync::Arc};

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
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
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
    game.creative = true;
    test(&mut game);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
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
