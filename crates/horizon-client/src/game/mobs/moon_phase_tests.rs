//! The Nether and the End have no `minecraft:moon` timeline (`TimelineTagsProvider`:
//! `#minecraft:in_nether` and `#minecraft:in_end` hold only `#minecraft:universal`), so
//! `visual/moon_phase` keeps its `FULL_MOON` default there. The local difficulty, the
//! cat variant's moon check, the `moon_phase` item property and the surface slime
//! spawn chance therefore do not follow the Overworld clock outside the Overworld.
use crate::game::{Dim, Game, native_machines::tests::server_game};
use crate::mobs::spawn_rules::SpawnLevel;
use horizon_world::food::Difficulty;

/// `DifficultyInstance.calculateDifficulty` at normal difficulty with no inhabited
/// time: `total` overworld-clock ticks and the moon brightness.
fn effective(total: i64, moon: f32) -> f32 {
    let global = ((total as f32 - 72_000.0) / 1_440_000.0).clamp(0.0, 1.0) * 0.25;
    2.0 * (0.75 + global + (moon * 0.25).clamp(0.0, global))
}

/// `gameplay/surface_slime_spawn_chance` as natural spawning reads it.
fn slime_chance(game: &Game<'_>) -> f32 {
    game.mob_world_base(&[]).surface_slime_chance(0, 60, 0)
}

#[test]
fn the_nether_and_the_end_keep_a_full_moon_whatever_the_overworld_clock_says() {
    if std::env::var_os("HORIZON_TEST_RESOURCES").is_none() {
        return;
    }
    let mut game = server_game();
    game.build_registries();
    game.world.health_rules.difficulty = Difficulty::Normal;
    let at = [0.5, 100.0, 0.5];
    // The loaded chunk the difficulty and the biome facts read.
    assert_eq!(game.dim, Dim::Overworld);
    game.world.day_time = 0;
    assert!(
        game.world.chunks.contains_key(&(0, 0)),
        "getCurrentDifficultyAt reads the moon only with the chunk loaded"
    );
    assert!(game.biome_facts(0, 100, 0).full_moon);

    // The Overworld follows its clock: a new moon at 102000 (phase 4), a third quarter
    // at 48000 (phase 2, half the light). No full-moon cat, no moon's share at all.
    for (clock, phase, moon) in [(102_000, 4, 0.0), (48_000, 2, 0.5)] {
        game.world.day_time = clock;
        assert!(
            !game.biome_facts(0, 100, 0).full_moon,
            "the Overworld at {clock}"
        );
        assert_eq!(game.item_level().moon_phase, phase);
        assert_eq!(
            game.local_difficulty(at).effective,
            effective(clock, moon),
            "the Overworld at {clock}"
        );
        assert_eq!(slime_chance(&game), 0.5 * moon, "the Overworld at {clock}");
    }

    // The same clocks in the Nether and the End: a full moon all the time, so the
    // moon's share of the local difficulty is there whatever the clock says. Every
    // consumer is reported, not just the first to disagree.
    let mut wrong = Vec::new();
    for dim in [Dim::Nether, Dim::End] {
        game.dim = dim;
        for clock in [102_000, 48_000] {
            game.world.day_time = clock;
            if !game.biome_facts(0, 100, 0).full_moon {
                wrong.push(format!("{dim:?} at {clock}: no full-moon cat"));
            }
            let phase = game.item_level().moon_phase;
            if phase != 0 {
                wrong.push(format!("{dim:?} at {clock}: moon_phase {phase}, want 0"));
            }
            let (got, want) = (game.local_difficulty(at).effective, effective(clock, 1.0));
            if got != want {
                wrong.push(format!(
                    "{dim:?} at {clock}: local difficulty {got}, want {want}"
                ));
            }
            let chance = slime_chance(&game);
            if chance != 0.0 {
                wrong.push(format!(
                    "{dim:?} at {clock}: surface slime chance {chance}, want 0"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "the Nether and the End have no moon timeline:\n{wrong:#?}"
    );
}
