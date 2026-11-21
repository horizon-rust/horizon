//! `/difficulty` on a hardcore world: `MinecraftServer.setDifficulty` stores
//! `HARD` whatever difficulty it is given when the world is hardcore, and
//! `DifficultyCommand` goes through it (with the lock ignored), so the command
//! says it changed the difficulty and the world stays hard.
use super::*;
use crate::game::native_machines::tests::server_game;

/// Run `command` as the server console: whether it succeeded, and what it said.
fn say(game: &mut Game<'_>, command: &str) -> (bool, Vec<Text>) {
    let ok = game.as_console(|g| {
        g.chat_capture = Some(Vec::new());
        g.run_command(command)
    });
    (ok, game.chat_capture.take().unwrap_or_default())
}

fn said(lines: &[Text]) -> Vec<String> {
    lines.iter().map(Text::plain).collect()
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn difficulty_stays_hard_on_a_hardcore_world() {
    let mut game = server_game();
    game.build_registries();
    game.set_allow_commands(true);
    // A hardcore world is hard, and level.dat says so.
    game.level.hardcore = true;
    game.level.difficulty = "hard".into();
    game.ui.difficulty = 3;
    game.sync_rules();
    let (ok, lines) = say(&mut game, "difficulty");
    assert!(ok);
    assert_eq!(said(&lines), ["The difficulty is Hard"]);

    // DifficultyCommand: peaceful is not the world's difficulty, so it goes
    // through setDifficulty, which reports the success it was asked for...
    let (ok, lines) = say(&mut game, "difficulty peaceful");
    assert!(ok);
    assert_eq!(said(&lines), ["The difficulty has been set to Peaceful"]);
    // ...and leaves the world hard: the level data (what level.dat gets), the
    // difficulty the mobs read, and what the next query says.
    assert_eq!(game.ui.difficulty, 3);
    assert_eq!(game.level.difficulty_id(), 3);
    assert_eq!(
        game.world.health_rules.difficulty,
        horizon_world::food::Difficulty::Hard
    );
    let (ok, lines) = say(&mut game, "difficulty");
    assert!(ok);
    assert_eq!(said(&lines), ["The difficulty is Hard"]);

    // The world is already hard, so asking for hard is the command's own failure.
    let (ok, lines) = say(&mut game, "difficulty hard");
    assert!(!ok);
    assert_eq!(
        said(&lines),
        ["The difficulty did not change; it is already set to Hard"]
    );

    // The clamp is hardcore's alone: the same command lowers any other world.
    game.level.hardcore = false;
    let (ok, lines) = say(&mut game, "difficulty peaceful");
    assert!(ok, "{:?}", said(&lines));
    assert_eq!(game.ui.difficulty, 0);
    assert_eq!(game.level.difficulty_id(), 0);
    assert_eq!(
        game.world.health_rules.difficulty,
        horizon_world::food::Difficulty::Peaceful
    );
}
