//! `PlaceOnWaterBlockItem.use`: the lily pad aims at the block above the block
//! `BlockGetter.clip` meets with `ClipContext.Block.OUTLINE` and
//! `ClipContext.Fluid.SOURCE_ONLY`, whichever shape is nearer. In a kelp cell
//! that is the plant's outline (`GrowingPlantBlock.getShape`, a 9/16-high
//! column); in a seagrass cell, whose outline is empty, it is the fluid box
//! (`FlowingFluid.getShape`: the whole cell under the same fluid, 8/9 high with
//! air above). Either way the hit's block is the plant's cell, so the pad lands
//! in the air above it.
use crate::game::native_machines::tests::server_game;
use crate::game::{Game, Hit};
use horizon_client::command::args::GameMode;
use horizon_client::items::ItemStack;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;

/// `stack` in the main hand, clicked at `hit` as `Minecraft.startUseItem` does.
fn click(game: &mut Game<'_>, hit: Hit, stack: ItemStack) {
    game.using = None;
    game.cooldowns = Default::default();
    *game.held_mut() = Some(stack);
    game.forced_hit = Some(hit);
    game.with_player_actor(|g| g.use_hand(Some(&hit))).unwrap();
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_lily_pad_places_on_the_water_above_a_kelp_plant() {
    let mut game = server_game();
    game.set_mode(GameMode::Survival);
    let registry = game.pack.assets.registry;
    let sand = registry.default_state("minecraft:sand").unwrap();
    let kelp = registry.default_state("minecraft:kelp").unwrap();
    let kelp_plant = registry.default_state("minecraft:kelp_plant").unwrap();
    let pad = registry.default_state("minecraft:lily_pad").unwrap();
    // A kelp plant whose head is the top water cell, the player's eyes 3.62
    // above it looking straight down.
    game.world.set(0, 60, 0, sand).unwrap();
    game.world.set(0, 61, 0, kelp_plant).unwrap();
    game.world.set(0, 62, 0, kelp_plant).unwrap();
    game.world.set(0, 63, 0, kelp).unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 66.0, 0.5)).unwrap());
    game.yaw = 0.0;
    game.pitch = 90.0;
    // The client's crosshair hit is the kelp cell itself (`Minecraft.pick`:
    // `Block.OUTLINE` and `Fluid.NONE`, the kelp's 9/16-high outline). The
    // item's `use` throws it away and casts its own SOURCE_ONLY ray, which must
    // meet that cell too and aim the pad at the air above it.
    let hit = Hit {
        pos: [0, 63, 0],
        face: [0, 1, 0],
        point: [0.5, 63.5625, 0.5],
        inside: false,
    };
    click(&mut game, hit, ItemStack::new("minecraft:lily_pad", 2));
    assert_eq!(
        game.world.block(0, 64, 0),
        pad,
        "no pad on the air cell above the kelp head"
    );
    assert_eq!(game.world.block(0, 63, 0), kelp, "the kelp head is gone");
    assert_eq!(
        game.held().map(|s| (s.item.to_string(), s.count)),
        Some(("minecraft:lily_pad".into(), 1)),
        "one pad was used"
    );
    // Nothing landed on the sea floor, which is where the ray used to stop.
    for y in 60..64 {
        assert_ne!(game.world.block(0, y, 0), pad, "a pad was placed at y={y}");
    }
}
