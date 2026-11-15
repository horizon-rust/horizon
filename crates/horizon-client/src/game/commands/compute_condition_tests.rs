//! `/compute`'s number providers read a `random_chance` chance through
//! `ContextFloatProvider.getFloat` and `binomial`'s probability through
//! `getFloatOrThrow` (`LootItemRandomChanceCondition.test`,
//! `BinomialDistributionGenerator.getIntUnsafe`): an inline provider chance is
//! evaluated (1.0 always holds, 0.0 never, a non-finite value reads as 0), and
//! a non-finite probability fails the command instead of counting every draw.
use crate::game::*;
use horizon_client::motion::PlayerMotion;
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
        .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 100.0, 0.5)).unwrap());
    game
}

/// Runs `command`: whether it succeeded and what the issuer was told.
fn run(game: &mut Game<'_>, command: &str) -> (bool, Vec<String>) {
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.chat_capture.take().unwrap_or_default();
    (
        ok,
        said.iter().map(horizon_client::text::Text::plain).collect(),
    )
}

/// The one line a command told its issuer.
fn only(said: &[String]) -> &str {
    assert_eq!(said.len(), 1, "{said:?}");
    &said[0]
}

/// `ConditionalValue` picks `on_true` or `on_false` from its condition, whose
/// `random_chance` reads the provider. A non-finite chance is 0, so it never
/// holds (`ContextFloatProvider.getFloat`).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_conditional_reads_its_random_chance_provider() {
    let mut game = game();
    let conditional = |chance: &str| {
        format!(
            r#"compute default integer {{type:"minecraft:conditional",condition:{{type:"minecraft:random_chance",chance:{chance}}},on_true:5,on_false:7}}"#
        )
    };
    for _ in 0..3 {
        let (ok, said) = run(
            &mut game,
            &conditional(r#"{type:"minecraft:constant",value:1.0}"#),
        );
        assert!(ok, "{said:?}");
        assert_eq!(only(&said), "Number provider returned value 5");
        let (ok, said) = run(
            &mut game,
            &conditional(r#"{type:"minecraft:constant",value:0.0}"#),
        );
        assert!(ok, "{said:?}");
        assert_eq!(only(&said), "Number provider returned value 7");
        let (ok, said) = run(
            &mut game,
            &conditional(r#"{type:"minecraft:div",left:1,right:0}"#),
        );
        assert!(ok, "{said:?}");
        assert_eq!(only(&said), "Number provider returned value 7");
    }
}

/// `BinomialDistributionGenerator.getIntUnsafe` asks for the probability with
/// `getFloatOrThrow`: a non-finite one is an `ArithmeticException`, which
/// `ComputeCommand.computeAsInt` reports, not a full count.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_non_finite_binomial_probability_fails_the_command() {
    let mut game = game();
    let binomial =
        |p: &str| format!(r#"compute default integer {{type:"minecraft:binomial",n:3,p:{p}}}"#);
    let (ok, said) = run(
        &mut game,
        &binomial(r#"{type:"minecraft:constant",value:1.0}"#),
    );
    assert!(ok, "{said:?}");
    assert_eq!(only(&said), "Number provider returned value 3");
    let (ok, said) = run(
        &mut game,
        &binomial(r#"{type:"minecraft:constant",value:0.0}"#),
    );
    assert!(ok, "{said:?}");
    assert_eq!(only(&said), "Number provider returned value 0");
    let (ok, said) = run(
        &mut game,
        &binomial(r#"{type:"minecraft:div",left:1,right:0}"#),
    );
    assert!(!ok, "vanilla throws ArithmeticException: {said:?}");
    assert_eq!(
        only(&said),
        "Number provider returned invalid value (Invalid value: Infinity)"
    );
}
