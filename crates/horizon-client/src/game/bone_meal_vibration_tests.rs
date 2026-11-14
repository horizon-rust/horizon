//! `BoneMealItem.useOn`'s vibration: `growCrop` shrinks the stack first and
//! then `ItemStack.causeUseVibration` posts `ITEM_INTERACT_FINISH` from the
//! player whenever the stack's `use_effects` component asks for it (the
//! default) — a stack of one that the click empties included, since the
//! component survives a count of 0.
use super::*;
use crate::game::{Hit, native_machines::tests::server_game};
use horizon_client::{items::ItemStack, mobs::Who, motion::PlayerMotion};
use horizon_core::geometry::Vec3;

const POS: [i32; 3] = [8, 100, 8];
const PLAYER: [f64; 3] = [8.5, 100.0, 10.5];

/// A server game with the player over a fresh wheat block at `POS`, the click
/// forced onto the wheat's top face.
fn game() -> Game<'static> {
    let mut game = server_game();
    game.options.dedicated = None;
    game.player =
        Some(PlayerMotion::new(game.motion, Vec3::new(PLAYER[0], PLAYER[1], PLAYER[2])).unwrap());
    game.forced_hit = Some(Hit {
        pos: POS,
        face: [0, 1, 0],
        point: [
            f64::from(POS[0]) + 0.5,
            f64::from(POS[1]) + 1.0,
            f64::from(POS[2]) + 0.5,
        ],
    });
    game
}

/// Puts an age-0 wheat back at `POS` and `stack` in the main hand.
fn hold(game: &mut Game<'_>, stack: ItemStack) {
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:wheat")
        .unwrap();
    game.world.set(POS[0], POS[1], POS[2], state).unwrap();
    *game.held_mut() = Some(stack);
}

/// The `item_interact_finish` game events a right click on the wheat posts.
fn click(game: &mut Game<'_>) -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    let hit = game.forced_hit.unwrap();
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
    assert!(game.use_hand(Some(&hit)).unwrap(), "the bone meal is used");
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| e.borrow_mut().take().unwrap_or_default())
}

/// The bone meal that empties the stack still vibrates, and so does every
/// earlier one; in creative the stack is not consumed at all, and a stack
/// whose use effects turn interaction vibrations off is silent.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn the_bone_meal_that_empties_the_stack_still_vibrates() {
    if std::env::var_os("HORIZON_TEST_RESOURCES").is_none() {
        return;
    }
    let finish = [("minecraft:item_interact_finish", PLAYER, Some(Who::Player))];
    let mut game = game();
    // A stack of one: the click empties it and vanilla still posts the event.
    game.creative = false;
    hold(&mut game, ItemStack::new("minecraft:bone_meal", 1));
    assert_eq!(click(&mut game), finish);
    assert!(game.held().is_none(), "the click emptied the hand");
    // Two clicks of a stack of two vibrate twice, as in the repro.
    hold(&mut game, ItemStack::new("minecraft:bone_meal", 2));
    assert_eq!(click(&mut game), finish);
    assert_eq!(click(&mut game), finish);
    assert!(game.held().is_none(), "both meals were used");
    // Creative leaves the stack alone and still vibrates.
    game.creative = true;
    hold(&mut game, ItemStack::new("minecraft:bone_meal", 1));
    assert_eq!(click(&mut game), finish);
    assert_eq!(game.held().unwrap().count, 1, "creative kept the meal");
    // A stack whose use effects turn interaction vibrations off is silent
    // (`ItemStack.causeUseVibration`), the component `use_vibration` reads.
    game.creative = false;
    let mut silent = ItemStack::new("minecraft:bone_meal", 1);
    silent.data_mut().set_loot_component(
        "minecraft:use_effects",
        &serde_json::json!({"interact_vibrations": false}),
    );
    hold(&mut game, silent);
    assert!(click(&mut game).is_empty());
}
